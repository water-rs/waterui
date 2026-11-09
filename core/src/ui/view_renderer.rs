//! View renderer for capturing views to pixel data.
//!
//! This module provides the `ViewRenderer` capability that native backends
//! install via FFI. It allows capturing `WaterUI` views as RGBA pixel data.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

use crate::AnyView;
use crate::layout::Size;
use crate::ui::content_ledger::{ContentError, DisplayList, PendingContent};

/// What a [`ViewRenderer`] capture renders: the size it proposes to the
/// view, and how long it waits for content the view owes after mount.
///
/// The renderer installs a fresh [`ContentLedger`](crate::ui::content_ledger::ContentLedger)
/// for the captured view and rasterizes only once nothing in it is owed, or
/// fails with [`RenderError::ContentPending`] when `content_deadline` passes
/// first.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RenderRequest {
    /// The size proposed to the view, in points.
    size: Size,
    /// How long the capture waits for owed content.
    content_deadline: Duration,
}

impl RenderRequest {
    /// The size proposed to the view, in points.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// How long the capture waits for owed content.
    #[must_use]
    pub const fn content_deadline(&self) -> Duration {
        self.content_deadline
    }
}

impl RenderRequest {
    /// A request proposing `size` that waits up to `content_deadline` for
    /// owed content.
    #[must_use]
    pub const fn new(size: Size, content_deadline: Duration) -> Self {
        Self {
            size,
            content_deadline,
        }
    }
}

/// Result of rendering a view to RGBA pixels.
#[derive(Debug)]
#[non_exhaustive]
pub struct RenderResult {
    /// RGBA pixel data (4 bytes per pixel, row-major order).
    rgba_data: Vec<u8>,
    /// Actual rendered width in pixels.
    width: u32,
    /// Actual rendered height in pixels.
    height: u32,
}

impl RenderResult {
    /// RGBA pixel data (4 bytes per pixel, row-major order).
    #[must_use]
    pub const fn rgba_data(&self) -> &[u8] {
        self.rgba_data.as_slice()
    }

    /// Actual rendered width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Actual rendered height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
}

impl RenderResult {
    /// `rgba_data` of a `width` by `height` pixel render, four bytes per
    /// pixel in row-major order.
    #[must_use]
    pub const fn new(rgba_data: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            rgba_data,
            width,
            height,
        }
    }

    /// Takes the result apart into its RGBA pixel data, width and height.
    #[must_use]
    pub fn into_parts(self) -> (Vec<u8>, u32, u32) {
        (self.rgba_data, self.width, self.height)
    }
}

/// Why a [`CustomViewRenderer`] produced no image.
///
/// A failed capture is always an error, never an empty or partial
/// [`RenderResult`]: callers read a successful result as the view's actual
/// pixels. The boxed sources carry the backend's own typed error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// The GPU work behind the capture failed: a render pass, a lost device,
    /// or the pixel readback.
    #[error("GPU rendering failed")]
    Gpu(#[source] Box<dyn core::error::Error + Send + Sync>),
    /// The platform did not provide a resource the capture needs, such as a
    /// bitmap context or a window to host the view.
    #[error("view capture failed")]
    Capture(#[source] Box<dyn core::error::Error + Send + Sync>),
    /// A native host completed the render with this failure message.
    #[error("native host failed to render the view: {0}")]
    Host(String),
    /// The content deadline passed while views of the captured subtree still
    /// owed content.
    #[error("{} still owe{} content after {deadline:?}", DisplayList(outstanding), if outstanding.len() == 1 { "s" } else { "" })]
    ContentPending {
        /// Every entry still owed when the deadline passed.
        outstanding: Vec<PendingContent>,
        /// The bound the capture waited for.
        deadline: Duration,
    },
    /// A view of the captured subtree reported that its content failed to
    /// load.
    #[error("{view} ({detail}) failed to load its content")]
    ContentFailed {
        /// The owing view's type name, without its path.
        view: &'static str,
        /// What the view was waiting for.
        detail: Cow<'static, str>,
        /// Why the load failed.
        #[source]
        source: Box<dyn core::error::Error + Send + Sync>,
    },
}

impl From<ContentError> for RenderError {
    fn from(error: ContentError) -> Self {
        match error {
            ContentError::Pending {
                outstanding,
                deadline,
            } => Self::ContentPending {
                outstanding,
                deadline,
            },
            ContentError::Failed(failure) => {
                let (view, detail, source) = failure.into_parts();
                Self::ContentFailed {
                    view,
                    detail,
                    source,
                }
            }
        }
    }
}

/// Trait for custom view renderers.
///
/// Native backends implement this to provide view-to-RGBA capture. The method
/// returns `impl Future`, so implementations write a plain `async fn` — the
/// object-safe boxing needed to store the renderer in the [`Environment`](crate::Environment) is an
/// internal detail of [`ViewRenderer`], never part of this contract.
pub trait CustomViewRenderer: 'static {
    /// Render a view to RGBA bytes.
    ///
    /// The implementation should:
    /// 1. Create an offscreen rendering context at the requested size
    /// 2. Mount the view hierarchy (native widgets + GPU surfaces) under a
    ///    fresh [`ContentLedger`](crate::ui::content_ledger::ContentLedger)
    /// 3. Await [`ContentLedger::wait`](crate::ui::content_ledger::ContentLedger::wait)
    ///    bounded by the request's content deadline, draining the backend
    ///    between passes
    /// 4. Capture the final composited result to RGBA pixels
    /// 5. Return the pixel data
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when the capture cannot produce the view's
    /// pixels — including [`RenderError::ContentPending`] when owed content
    /// misses the deadline and [`RenderError::ContentFailed`] when it fails to
    /// load.
    fn render_to_rgba(
        &self,
        view: AnyView,
        request: RenderRequest,
    ) -> impl Future<Output = Result<RenderResult, RenderError>>;
}

/// Object-safe shim over [`CustomViewRenderer`] so [`ViewRenderer`] can box the
/// renderer: the public trait keeps its friendly `impl Future` signature and
/// the blanket impl pins the future here, behind the erasure boundary.
trait CustomViewRendererImpl: 'static {
    fn render_to_rgba<'a>(
        &'a self,
        view: AnyView,
        request: RenderRequest,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<RenderResult, RenderError>>>>;
}

impl<T: CustomViewRenderer> CustomViewRendererImpl for T {
    fn render_to_rgba<'a>(
        &'a self,
        view: AnyView,
        request: RenderRequest,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<RenderResult, RenderError>>>> {
        Box::pin(CustomViewRenderer::render_to_rgba(self, view, request))
    }
}

/// Type-erased view renderer stored in Environment.
///
/// This wrapper allows storing the renderer in the Environment without
/// exposing the concrete implementation type.
pub struct ViewRenderer(Box<dyn CustomViewRendererImpl>);

impl ViewRenderer {
    /// Create a new view renderer from a custom implementation.
    pub fn new<T: CustomViewRenderer>(renderer: T) -> Self {
        Self(Box::new(renderer))
    }

    /// Render a view to RGBA pixel data.
    ///
    /// # Errors
    ///
    /// Returns the [`RenderError`] the installed renderer reports when the
    /// capture fails.
    #[expect(
        clippy::future_not_send,
        reason = "the erased `CustomViewRenderer` future is not Send; `render` runs on the caller's thread"
    )]
    pub async fn render(
        &self,
        view: AnyView,
        request: RenderRequest,
    ) -> Result<RenderResult, RenderError> {
        self.0.render_to_rgba(view, request).await
    }
}

impl fmt::Debug for ViewRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewRenderer").finish_non_exhaustive()
    }
}
