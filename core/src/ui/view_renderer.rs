//! View renderer for capturing views to pixel data.
//!
//! This module provides the `ViewRenderer` capability that native backends
//! install via FFI. It allows capturing `WaterUI` views as RGBA pixel data.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use core::pin::Pin;

use crate::AnyView;

/// Size for view rendering.
#[derive(Debug, Clone, Copy)]
pub struct RenderSize {
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
}

impl RenderSize {
    /// Create a new render size.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Result of rendering a view to RGBA pixels.
#[derive(Debug)]
pub struct RenderResult {
    /// RGBA pixel data (4 bytes per pixel, row-major order).
    pub rgba_data: Vec<u8>,
    /// Actual rendered width in pixels.
    pub width: u32,
    /// Actual rendered height in pixels.
    pub height: u32,
}

/// Why a [`CustomViewRenderer`] produced no image.
///
/// A failed capture is always an error, never an empty or partial
/// [`RenderResult`]: callers read a successful result as the view's actual
/// pixels. The boxed sources carry the backend's own typed error.
#[derive(Debug, thiserror::Error)]
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
    /// 1. Create an offscreen rendering context at the given size
    /// 2. Render the view hierarchy (native widgets + GPU surfaces)
    /// 3. Capture the final composited result to RGBA pixels
    /// 4. Return the pixel data
    ///
    /// A capture that cannot produce the view's pixels returns
    /// [`RenderError`].
    fn render_to_rgba(
        &self,
        view: AnyView,
        size: RenderSize,
    ) -> impl Future<Output = Result<RenderResult, RenderError>>;
}

/// Object-safe shim over [`CustomViewRenderer`] so [`ViewRenderer`] can box the
/// renderer: the public trait keeps its friendly `impl Future` signature and
/// the blanket impl pins the future here, behind the erasure boundary.
trait CustomViewRendererImpl: 'static {
    fn render_to_rgba<'a>(
        &'a self,
        view: AnyView,
        size: RenderSize,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<RenderResult, RenderError>>>>;
}

impl<T: CustomViewRenderer> CustomViewRendererImpl for T {
    fn render_to_rgba<'a>(
        &'a self,
        view: AnyView,
        size: RenderSize,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<RenderResult, RenderError>>>> {
        Box::pin(CustomViewRenderer::render_to_rgba(self, view, size))
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
    #[allow(clippy::future_not_send)]
    pub async fn render(
        &self,
        view: AnyView,
        size: RenderSize,
    ) -> Result<RenderResult, RenderError> {
        self.0.render_to_rgba(view, size).await
    }
}

impl fmt::Debug for ViewRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewRenderer").finish_non_exhaustive()
    }
}
