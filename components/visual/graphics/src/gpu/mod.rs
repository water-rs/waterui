//! User GPU work inside the scene: [`GpuContent`] and the view that hosts it.
//!
//! The engine owns the device. Content that draws with `wgpu` directly — a
//! particle system, a 3D viewport, a video decoder's output — implements
//! [`GpuContent`] and the backend installs it on a layer of an engine whose
//! backend hosts GPU content, where it renders into a texture the engine
//! composites like any other layer. The content runs on the render thread:
//! it is `Send`, and whatever it shares with the UI (a pointer position, a
//! simulation parameter) crosses through its own synchronised state.

extern crate alloc;

pub mod runtime;
pub use runtime::{GpuRuntime, GpuRuntimeError, preferred_surface_format};

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use core::fmt;
use core::time::Duration;

use waterui_core::layout::{Size, StretchAxis};
use waterui_core::{Environment, Native, NativeView, View};
use wgpu::{Adapter, Device, Queue, Texture, TextureFormat, TextureView};

use cherenkov::kurbo;

use crate::input::SurfaceInputEvent;

/// Wakes the host for another frame from anywhere: a decoder thread, a
/// browser's compositor callback, a network task.
///
/// The backend supplies it in [`Context`]; requesting a redraw while a frame
/// is already scheduled is a no-op.
#[derive(Clone)]
pub struct RedrawHandle(Arc<dyn Fn() + Send + Sync>);

impl fmt::Debug for RedrawHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RedrawHandle")
    }
}

impl RedrawHandle {
    /// A handle that runs `wake` on every request.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }

    /// Asks the host for another frame.
    pub fn request_redraw(&self) {
        (self.0)();
    }
}

/// The engine's device, handed to [`GpuContent::setup`].
#[derive(Debug)]
pub struct Context<'a> {
    /// The adapter the device was created from.
    pub adapter: &'a Adapter,
    /// The device the engine renders with.
    pub device: &'a Device,
    /// Its queue.
    pub queue: &'a Queue,
    /// The format of the texture the content renders into.
    pub format: TextureFormat,
    /// Wakes the host for another frame from any thread.
    pub redraw: RedrawHandle,
}

/// One frame of a [`GpuContent`]: the texture to draw into and the clock.
#[derive(Debug)]
pub struct Frame<'a> {
    /// The device the engine renders with.
    pub device: &'a Device,
    /// Its queue.
    pub queue: &'a Queue,
    /// The texture the content draws into.
    pub texture: &'a Texture,
    /// A view of the whole texture.
    pub view: &'a TextureView,
    /// The texture's format.
    pub format: TextureFormat,
    /// The texture's width in pixels.
    pub width: u32,
    /// The texture's height in pixels.
    pub height: u32,
    /// Pixels per logical point.
    pub scale: f32,
    /// Time since the content was set up.
    pub elapsed: Duration,
    /// Time since the previous frame.
    pub delta: Duration,
    redraw: bool,
}

impl<'a> Frame<'a> {
    /// A frame that has not yet requested a redraw.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        device: &'a Device,
        queue: &'a Queue,
        texture: &'a Texture,
        view: &'a TextureView,
        format: TextureFormat,
        (width, height): (u32, u32),
        scale: f32,
        (elapsed, delta): (Duration, Duration),
    ) -> Self {
        Self {
            device,
            queue,
            texture,
            view,
            format,
            width,
            height,
            scale,
            elapsed,
            delta,
            redraw: false,
        }
    }

    /// Asks for another frame after this one.
    pub const fn request_redraw(&mut self) {
        self.redraw = true;
    }

    /// Whether the content asked for another frame.
    #[must_use]
    pub const fn redraw_requested(&self) -> bool {
        self.redraw
    }
}

/// GPU work the engine composites as a layer.
pub trait GpuContent: Send + 'static {
    /// Creates pipelines and resources against the engine's device.
    ///
    /// Called once, on the render thread, before the first [`render`](Self::render).
    fn setup(&mut self, gpu: &Context<'_>);

    /// Draws one frame into `frame`'s texture.
    ///
    /// Call [`Frame::request_redraw`] when the content animates and needs
    /// the next frame too.
    fn render(&mut self, frame: &mut Frame<'_>);

    /// Whether every pixel the content draws is opaque, letting the engine
    /// skip blending underneath it.
    fn is_opaque(&self) -> bool {
        false
    }

    /// The size the content is naturally, in logical points; `None` takes
    /// whatever the layout gives.
    fn intrinsic_size(&self) -> Option<Size> {
        None
    }
}

/// A UI-side handler for input the backend routes to a [`GpuContentView`].
pub type InputHandler = Rc<dyn Fn(&SurfaceInputEvent)>;

/// A UI-side hook the backend runs once per frame, before the content renders.
///
/// Producers confined to the UI thread — a browser engine whose objects are
/// not `Send` — pump their work here and hand the results to the content
/// through their own channels.
pub type FrameHook = Rc<dyn Fn()>;

/// A UI-side query for where the content's text caret is, in logical
/// view-local coordinates, so a host can place its input-method panel.
pub type CaretQuery = Rc<dyn Fn() -> Option<kurbo::Rect>>;

/// A view whose pixels come from [`GpuContent`].
///
/// # Layout Behavior
///
/// Stretches on both axes unless the content reports an intrinsic size, in
/// which case it is that size and does not stretch.
pub struct GpuContentView {
    content: Option<Box<dyn GpuContent>>,
    intrinsic_size: Option<Size>,
    opaque: bool,
    input: Option<InputHandler>,
    frame: Option<FrameHook>,
    caret: Option<CaretQuery>,
    label: Option<String>,
    value: Option<String>,
}

impl fmt::Debug for GpuContentView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContentView")
            .field("intrinsic_size", &self.intrinsic_size)
            .field("opaque", &self.opaque)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl GpuContentView {
    /// A view drawing `content`.
    #[must_use]
    pub fn new(content: impl GpuContent) -> Self {
        Self {
            intrinsic_size: content.intrinsic_size(),
            opaque: content.is_opaque(),
            content: Some(Box::new(content)),
            input: None,
            frame: None,
            caret: None,
            label: None,
            value: None,
        }
    }

    /// Receives pointer, keyboard and gesture events the backend routes to
    /// this view.
    #[must_use]
    pub fn on_input(mut self, handler: impl Fn(&SurfaceInputEvent) + 'static) -> Self {
        self.input = Some(Rc::new(handler));
        self
    }

    /// Runs `hook` on the UI thread once per frame before the content renders.
    #[must_use]
    pub fn on_frame(mut self, hook: impl Fn() + 'static) -> Self {
        self.frame = Some(Rc::new(hook));
        self
    }

    /// Answers where the content's text caret is, for input-method panels.
    #[must_use]
    pub fn on_ime_caret(mut self, query: impl Fn() -> Option<kurbo::Rect> + 'static) -> Self {
        self.caret = Some(Rc::new(query));
        self
    }

    /// The content's text caret, if it has one right now.
    #[must_use]
    pub fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.caret.as_ref().and_then(|query| query())
    }

    /// Runs the per-frame UI hook, if any.
    pub fn frame(&self) {
        if let Some(hook) = &self.frame {
            hook();
        }
    }

    /// The accessibility name of the content.
    #[must_use]
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The accessibility value of the content.
    #[must_use]
    pub fn described(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Whether this view takes input events.
    #[must_use]
    pub const fn wants_input_events(&self) -> bool {
        self.input.is_some()
    }

    /// Routes an input event to the content's handler.
    pub fn input(&self, event: &SurfaceInputEvent) {
        if let Some(handler) = &self.input {
            handler(event);
        }
    }

    /// The natural size of the content, if it has one.
    #[must_use]
    pub const fn intrinsic_size(&self) -> Option<Size> {
        self.intrinsic_size
    }

    /// Whether the content is opaque.
    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        self.opaque
    }

    /// The accessibility name.
    #[must_use]
    pub fn accessibility_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// The accessibility value.
    #[must_use]
    pub fn accessibility_value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Transfers this producer to a Cherenkov GPU layer.
    ///
    /// `wake` schedules the host's display link or event loop when asynchronous
    /// producer work finishes. The UI hooks and input handlers stay in this view.
    ///
    /// # Panics
    /// Panics if the content has already been transferred.
    #[must_use]
    pub fn take_engine_content(
        &mut self,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> cherenkov_gpu::interop::GpuContentBox {
        cherenkov_gpu::interop::GpuContentBox::new(EngineContent(self.take_content()), wake)
    }

    /// Takes the content out for installation on a layer.
    ///
    /// # Panics
    /// When the content was already taken: a view is installed once.
    #[must_use]
    pub fn take_content(&mut self) -> Box<dyn GpuContent> {
        self.content
            .take()
            .expect("GpuContentView content installed twice")
    }
}

impl NativeView for GpuContentView {
    fn stretch_axis(&self) -> StretchAxis {
        if self.intrinsic_size.is_some() {
            StretchAxis::None
        } else {
            StretchAxis::Both
        }
    }
}

impl View for GpuContentView {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(self)
    }
}

/// Projects WaterUI's producer contract onto the engine's render context.
struct EngineContent(Box<dyn GpuContent>);

impl cherenkov_gpu::interop::GpuContent for EngineContent {
    fn setup(
        &mut self,
        gpu: &cherenkov_gpu::interop::wgpu::Context<'_>,
    ) -> impl core::future::Future<Output = ()> {
        let redraw = gpu.redraw.clone();
        self.0.setup(&Context {
            adapter: gpu.adapter,
            device: gpu.device,
            queue: gpu.queue,
            format: gpu.format,
            redraw: RedrawHandle::new(move || redraw.request_redraw()),
        });
        core::future::ready(())
    }

    fn render(&mut self, gpu: &mut cherenkov_gpu::interop::wgpu::Frame<'_>) {
        let mut frame = Frame::new(
            gpu.device,
            gpu.queue,
            gpu.texture,
            gpu.view,
            gpu.format,
            (gpu.width, gpu.height),
            gpu.scale,
            (gpu.elapsed, gpu.delta),
        );
        self.0.render(&mut frame);
        if frame.redraw_requested() {
            gpu.request_redraw();
        }
    }
}
