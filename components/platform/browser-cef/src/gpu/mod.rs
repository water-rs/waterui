#[cfg(target_os = "windows")]
use num_traits::ToPrimitive as _;
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use waterui_graphics::gpu::ExternalFrameView;
#[cfg(target_os = "windows")]
use waterui_graphics::gpu::GpuContentView;
#[cfg(target_os = "windows")]
use waterui_graphics::gpu::{Context, Frame, GpuContent};

#[cfg(target_os = "windows")]
use crate::AcceleratedFrameSink;
use crate::CefPageHandle;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use crate::input::CefSurfaceInput;
#[cfg(target_os = "windows")]
use presenter::{GpuHandles, OwnedFrameMailbox, TexturePresenter};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
pub mod presenter;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod sink;
#[cfg(target_os = "windows")]
mod windows;

/// The physical size and device-pixel ratio of the last rendered frame,
/// published to the UI side so it can keep Chromium's logical viewport in
/// step. Nothing renders at `(0, 0)`, so it also marks "no frame yet".
#[cfg(target_os = "windows")]
type BrowserViewport = (u32, u32, f32);

/// The state the UI hook and the render-side [`CefGpuContent`] share.
#[cfg(target_os = "windows")]
pub struct CefShared {
    pub mailbox: Arc<OwnedFrameMailbox>,
    pub viewport: Arc<Mutex<BrowserViewport>>,
}

/// GPU content that composites one CEF page's shared textures into the
/// engine's layer.
///
/// It owns only `Send` state: `CefPageHandle` is confined to the UI thread, so
/// every page call lives in [`CefUiBridge`] and the two sides exchange data
/// through [`OwnedFrameMailbox`] and the shared viewport cell.
#[cfg(target_os = "windows")]
pub struct CefGpuContent {
    mailbox: Arc<OwnedFrameMailbox>,
    viewport: Arc<Mutex<BrowserViewport>>,
    presenter: Option<TexturePresenter>,
}

#[cfg(target_os = "windows")]
impl CefGpuContent {
    fn new() -> (Self, CefShared) {
        let shared = CefShared {
            mailbox: Arc::new(OwnedFrameMailbox::new()),
            viewport: Arc::new(Mutex::new((0, 0, 1.0))),
        };
        (
            Self {
                mailbox: Arc::clone(&shared.mailbox),
                viewport: Arc::clone(&shared.viewport),
                presenter: None,
            },
            shared,
        )
    }
}

#[cfg(target_os = "windows")]
impl GpuContent for CefGpuContent {
    fn setup(&mut self, context: &Context<'_>) {
        self.mailbox.set_gpu_handles(GpuHandles {
            adapter: context.adapter.clone(),
            device: context.device.clone(),
            queue: context.queue.clone(),
        });
        let redraw = context.redraw.clone();
        self.mailbox.set_waker(move || redraw.request_redraw());
        self.presenter = Some(TexturePresenter::new(context));
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        *self.viewport.lock().expect("CEF browser viewport poisoned") =
            (frame.width, frame.height, frame.scale);
        // On Windows the host paces the compositor itself; asking for the
        // next frame here keeps `page.request_frame()` pumping.
        frame.request_redraw();
        let presenter = self
            .presenter
            .as_mut()
            .expect("CEF GPU content rendered before setup");
        if let Some(texture) = self.mailbox.take_view() {
            presenter.set_source(texture);
        }
        if let Some(texture) = self.mailbox.take_popup() {
            presenter.set_popup_source(texture);
        }
        presenter.set_popup_rect(self.mailbox.popup_rect());
        presenter.render(frame, f64::from(frame.scale));
    }
}

/// The UI-thread half of a CEF GPU view: it installs the page's frame sink
/// once the render thread publishes its device handles, keeps Chromium's
/// logical viewport in step with the last rendered size, and asks Chromium
/// for the next compositor frame.
#[cfg(target_os = "windows")]
pub struct CefUiBridge<S: AcceleratedFrameSink> {
    page: CefPageHandle,
    shared: CefShared,
    sink_installed: bool,
    make_sink: fn(GpuHandles, Arc<OwnedFrameMailbox>) -> S,
}

#[cfg(target_os = "windows")]
impl<S: AcceleratedFrameSink> CefUiBridge<S> {
    pub fn new(
        page: CefPageHandle,
        shared: CefShared,
        make_sink: fn(GpuHandles, Arc<OwnedFrameMailbox>) -> S,
    ) -> Self {
        Self {
            page,
            shared,
            sink_installed: false,
            make_sink,
        }
    }

    /// Runs one UI-side browser frame; call once per presented frame.
    ///
    /// # Panics
    ///
    /// Panics when the logical viewport does not fit a `u32`.
    pub fn frame(&mut self) {
        if !self.sink_installed
            && let Some(handles) = self.shared.mailbox.take_gpu_handles()
        {
            self.page
                .set_frame_sink((self.make_sink)(handles, Arc::clone(&self.shared.mailbox)));
            self.sink_installed = true;
        }
        let (width, height, scale) = *self
            .shared
            .viewport
            .lock()
            .expect("CEF browser viewport poisoned");
        if width > 0 && height > 0 {
            let logical_width = (f64::from(width) / f64::from(scale))
                .round()
                .max(1.0)
                .to_u32()
                .expect("CEF logical width exceeds u32");
            let logical_height = (f64::from(height) / f64::from(scale))
                .round()
                .max(1.0)
                .to_u32()
                .expect("CEF logical height exceeds u32");
            self.page.set_viewport(logical_width, logical_height, scale);
        }
        self.page.request_frame();
    }
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
///
/// The page's shared frames import on the layer's own device and present
/// through [`FrameOutput`](waterui_graphics::gpu::FrameOutput) — the view
/// is an [`ExternalFrameView`].
#[cfg(target_os = "linux")]
#[must_use]
pub fn gpu_view(page: CefPageHandle) -> ExternalFrameView {
    linux::gpu_view(page)
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
///
/// The page's shared `IOSurface` frames import on the layer's own device
/// and present through
/// [`FrameOutput`](waterui_graphics::gpu::FrameOutput) — the view is an
/// [`ExternalFrameView`].
#[cfg(target_os = "macos")]
#[must_use]
pub fn gpu_view(page: CefPageHandle) -> ExternalFrameView {
    macos::gpu_view(page)
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
#[cfg(target_os = "windows")]
#[must_use]
pub fn gpu_view(page: CefPageHandle) -> GpuContentView {
    windows::gpu_view(page)
}

/// Creates the presenter for one visible CEF page, wired to take its own
/// input.
///
/// The view answers `wants_input_events`, so a backend that routes surface
/// input to GPU views needs nothing CEF-specific: the pointer, keyboard,
/// scroll and composition events landing on this layer reach Chromium
/// through [`CefSurfaceInput`]. A backend whose input arrives somewhere
/// else entirely — GTK delivers it to the `GtkGLArea`'s event controllers —
/// uses [`gpu_view`] and owns a [`CefSurfaceInput`] beside it instead.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[must_use]
pub fn gpu_view_with_input(page: CefPageHandle) -> ExternalFrameView {
    let input = std::cell::RefCell::new(CefSurfaceInput::new(page.clone()));
    gpu_view(page).on_input(move |event| input.borrow_mut().handle(event))
}

/// Creates the presenter for one visible CEF page, wired to take its own
/// input.
///
/// The view answers
/// [`wants_input_events`](GpuContentView::wants_input_events), so a backend
/// that routes surface input to GPU views needs nothing CEF-specific: the
/// pointer, keyboard, scroll and composition events landing on this layer
/// reach Chromium through [`CefSurfaceInput`]. A backend whose input arrives
/// somewhere else entirely — GTK delivers it to the `GtkGLArea`'s event
/// controllers — uses [`gpu_view`] and owns a [`CefSurfaceInput`] beside it
/// instead.
#[cfg(target_os = "windows")]
#[must_use]
pub fn gpu_view_with_input(page: CefPageHandle) -> GpuContentView {
    let input = std::cell::RefCell::new(CefSurfaceInput::new(page.clone()));
    gpu_view(page).on_input(move |event| input.borrow_mut().handle(event))
}
