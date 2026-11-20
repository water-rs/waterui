//! Offscreen view capture to RGBA pixel data.
//!
//! Backends implement [`OffscreenRenderer`] and supply it from
//! `NativeBackend::offscreen`. Portable application code attaches a
//! [`CaptureHandle`] to its view tree (for example with `view.capture(&handle)`)
//! so views of type `V` render through the active backend's offscreen renderer
//! without erasing `V` to [`AnyView`].

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::Size;
use waterui_core::metadata::MetadataKey;
use waterui_core::plugin::Plugin;
use waterui_core::{AnyView, Environment, Metadata, View};

use crate::content_ledger::{ContentError, DisplayList, PendingContent};

/// What an [`OffscreenRenderer`] capture renders: the size it proposes to the
/// view, and how long it waits for content the view owes after mount.
///
/// The renderer installs a fresh [`ContentLedger`](crate::content_ledger::ContentLedger)
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

/// Why an [`OffscreenRenderer`] produced no image.
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

/// Offscreen view-to-RGBA capture contract implemented by every backend.
///
/// Backends implement this trait and return it from
/// `NativeBackend::offscreen`. Keeping `render_to_rgba` generic over `V: View`
/// preserves static dispatch all the way to the backend's realization with no
/// `AnyView` or boxed renderer in the environment; callers holding an erased
/// view can still call `render_to_rgba::<AnyView>(...)`.
pub trait OffscreenRenderer: 'static {
    /// Renders `view` under `env` to RGBA pixels.
    ///
    /// Implementations:
    /// 1. Create or reuse an offscreen rendering context at the requested size
    /// 2. Mount the view hierarchy under a fresh
    ///    [`ContentLedger`](crate::content_ledger::ContentLedger)
    /// 3. Await [`ContentLedger::wait`](crate::content_ledger::ContentLedger::wait)
    ///    bounded by the request's content deadline, draining the backend
    ///    between passes
    /// 4. Capture the final composited result to RGBA pixels
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when the capture cannot produce the view's
    /// pixels — including [`RenderError::ContentPending`] when owed content
    /// misses the deadline and [`RenderError::ContentFailed`] when it fails to
    /// load.
    fn render_to_rgba<V: View>(
        &self,
        view: V,
        env: &Environment,
        request: RenderRequest,
    ) -> impl Future<Output = Result<RenderResult, RenderError>>;
}

/// Boxed future returned by [`CaptureHandle::render_to_rgba`] and
/// [`CaptureHandle::render_to_rgba_in`].
pub type RenderFuture = Pin<Box<dyn Future<Output = Result<RenderResult, RenderError>>>>;

/// Error returned when [`CaptureHandle::render_to_rgba`] is called before the
/// handle has been wired through a dispatched view tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("capture handle is not wired to a backend")]
pub struct UnwiredCaptureError;

type CaptureFn<V> = Rc<dyn Fn(V, &Environment, RenderRequest) -> RenderFuture>;

struct WiredCapture<V> {
    default_env: Environment,
    render: CaptureFn<V>,
}

/// Typed offscreen capture handle for views of type `V`.
///
/// Create a handle with [`CaptureHandle::new`], attach it to a view in the
/// rendered tree via [`CaptureHandle::attach`] (or `view.capture(&handle)`),
/// and call [`Self::render_to_rgba`] from event handlers or async tasks. The
/// handle is wired when the view dispatches to a backend (`View::dispatch`), so
/// `V` stays concrete without erasing to [`AnyView`].
pub struct CaptureHandle<V: View>(Rc<RefCell<Option<WiredCapture<V>>>>);

impl<V: View> Clone for CaptureHandle<V> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<V: View> Default for CaptureHandle<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: View> Plugin for CaptureHandle<V> {}

impl<V: View> fmt::Debug for CaptureHandle<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureHandle")
            .field("wired", &self.is_wired())
            .finish_non_exhaustive()
    }
}

impl<V: View> CaptureHandle<V> {
    /// Creates an unwired capture handle.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(None)))
    }

    /// Returns whether this handle has been wired to a backend.
    #[must_use]
    pub fn is_wired(&self) -> bool {
        self.0.borrow().is_some()
    }

    /// Attaches this handle to `content` so dispatching `content` wires the
    /// active backend's [`OffscreenRenderer`] onto this handle.
    pub fn attach<Content: View>(&self, content: Content) -> CaptureScope<V, Content> {
        CaptureScope {
            handle: self.clone(),
            content,
        }
    }

    fn wire(
        &self,
        default_env: Environment,
        render: impl Fn(V, &Environment, RenderRequest) -> RenderFuture + 'static,
    ) {
        *self.0.borrow_mut() = Some(WiredCapture {
            default_env,
            render: Rc::new(render),
        });
    }

    /// Renders `view` to RGBA pixels in the environment where this handle was
    /// wired.
    pub fn render_to_rgba(&self, view: V, request: RenderRequest) -> RenderFuture {
        let wired = {
            let borrowed = self.0.borrow();
            borrowed
                .as_ref()
                .map(|w| (w.default_env.clone(), Rc::clone(&w.render)))
        };
        match wired {
            Some((env, render)) => render(view, &env, request),
            None => Box::pin(async { Err(RenderError::Capture(Box::new(UnwiredCaptureError))) }),
        }
    }

    /// Renders `view` to RGBA pixels under an explicit `env`.
    pub fn render_to_rgba_in(
        &self,
        view: V,
        env: &Environment,
        request: RenderRequest,
    ) -> RenderFuture {
        let render = self.0.borrow().as_ref().map(|w| Rc::clone(&w.render));
        match render {
            Some(render) => render(view, env, request),
            None => Box::pin(async { Err(RenderError::Capture(Box::new(UnwiredCaptureError))) }),
        }
    }
}

/// View wrapper that wires a [`CaptureHandle`] during [`View::dispatch`] while
/// rendering `content` transparently.
#[must_use]
pub struct CaptureScope<V: View, Content: View> {
    handle: CaptureHandle<V>,
    content: Content,
}

impl<V: View, Content: View + fmt::Debug> fmt::Debug for CaptureScope<V, Content> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureScope")
            .field("handle", &self.handle)
            .field("content", &self.content)
            .finish()
    }
}

impl<V: View, Content: View> CaptureScope<V, Content> {
    fn binder<S: ChildSlot>(&self, env: &Environment) -> CaptureBinder<S> {
        let handle = self.handle.clone();
        let default_env = env.clone();
        CaptureBinder::new(move |render_slot: SlotRenderFn<S>| {
            handle.wire(
                default_env,
                move |view: V, render_env: &Environment, request| {
                    render_slot(S::from_view(view), render_env, request)
                },
            );
        })
    }
}

impl<V: View, Content: View> View for CaptureScope<V, Content> {
    fn body(self, env: &Environment) -> impl View {
        let binder = self.binder::<AnyView>(env);
        Metadata::new(self.content, binder)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let binder = self.binder::<B::Slot>(env);
        backend.metadata(
            Metadata::new(B::Slot::from_view(self.content), binder),
            env,
            cx,
        )
    }
}

/// Type-erased callback over a backend's child slot `S`, keeping `V` un-erased
/// via `S::from_view::<V>`.
#[doc(hidden)]
pub type SlotRenderFn<S> = Rc<dyn Fn(S, &Environment, RenderRequest) -> RenderFuture>;

/// Metadata key carrying the wiring callback from [`CaptureScope`] to
/// `NativeBackend` without exposing `NativeBackend` in `waterui-core`.
#[doc(hidden)]
pub struct CaptureBinder<S> {
    wire: Box<dyn FnOnce(SlotRenderFn<S>)>,
}

impl<S> fmt::Debug for CaptureBinder<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureBinder").finish_non_exhaustive()
    }
}

impl<S: 'static> CaptureBinder<S> {
    fn new(wire: impl FnOnce(SlotRenderFn<S>) + 'static) -> Self {
        Self {
            wire: Box::new(wire),
        }
    }

    /// Connects the capture handle to `render_slot`.
    pub fn bind(self, render_slot: SlotRenderFn<S>) {
        (self.wire)(render_slot);
    }
}

impl<S: 'static> MetadataKey for CaptureBinder<S> {}

impl<S: 'static> MapSlots<S> for CaptureBinder<S> {
    type With<T: 'static> = CaptureBinder<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> CaptureBinder<T> {
        let wire = self.wire;
        CaptureBinder::new(move |render_t: SlotRenderFn<T>| {
            let render_s: SlotRenderFn<S> = Rc::new(
                move |slot_s: S, env: &Environment, request: RenderRequest| {
                    render_t(f(slot_s), env, request)
                },
            );
            wire(render_s);
        })
    }
}

#[cfg(test)]
mod tests {
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};
    use core::time::Duration;

    use waterui_core::layout::Size;

    use super::{CaptureHandle, RenderError, RenderRequest, UnwiredCaptureError};

    fn block_on_ready<F: core::future::Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("expected future to resolve immediately"),
        }
    }

    #[test]
    fn unwired_capture_handle_returns_capture_error() {
        let handle = CaptureHandle::<()>::new();
        assert!(!handle.is_wired());
        let future = handle.render_to_rgba(
            (),
            RenderRequest::new(Size::new(16.0, 16.0), Duration::ZERO),
        );
        let err = block_on_ready(future).expect_err("unwired handle must fail");
        match err {
            RenderError::Capture(source) => {
                assert!(source.downcast_ref::<UnwiredCaptureError>().is_some());
            }
            other => panic!("unexpected render error: {other:?}"),
        }
    }
}
