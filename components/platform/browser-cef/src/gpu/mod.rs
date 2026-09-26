use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use num_traits::ToPrimitive as _;
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame, GpuContent, GpuContentView};

use crate::CefPageHandle;
use crate::input::CefSurfaceInput;
use presenter::{OwnedFrameMailbox, TexturePresenter};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod presenter;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// The logical browser viewport, derived from the frame the content renders into.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    width: u32,
    height: u32,
    scale: f32,
}

impl Viewport {
    fn of(frame: &GpuFrame<'_>) -> Self {
        let scale = f64::from(frame.scale);
        let logical = |pixels: u32| {
            (f64::from(pixels) / scale)
                .round()
                .max(1.0)
                .to_u32()
                .expect("CEF logical size exceeds u32")
        };
        Self {
            width: logical(frame.width),
            height: logical(frame.height),
            scale: frame.scale,
        }
    }
}

/// The UI-thread half of a CEF presenter.
///
/// Chromium's objects live on the UI thread, so everything that touches the
/// page — installing the accelerated frame sink once the engine's device is
/// known, sizing the off-screen viewport, asking for the next frame — happens
/// here, once per host frame. No message pumping: Chromium's loop belongs to
/// `CefRuntime::start_message_pump`, which Chromium itself paces.
pub struct CefFeed {
    page: CefPageHandle,
    sinks: Receiver<platform::SinkParts>,
    viewports: Receiver<Viewport>,
    viewport: RefCell<Option<Viewport>>,
}

impl core::fmt::Debug for CefFeed {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("CefFeed").finish_non_exhaustive()
    }
}

impl CefFeed {
    /// Applies what the content learned since the last frame and requests
    /// the browser's next one. Call once per host frame from the UI thread.
    pub fn pump(&self) {
        if let Some(parts) = self.sinks.try_iter().last() {
            self.page.set_frame_sink(platform::frame_sink(parts));
            tracing::debug!("Installed the accelerated CEF frame sink");
        }
        if let Some(viewport) = self.viewports.try_iter().last()
            && self.viewport.replace(Some(viewport)) != Some(viewport)
        {
            self.page
                .set_viewport(viewport.width, viewport.height, viewport.scale);
        }
        self.page.request_frame();
    }
}

/// GPU content presenting the newest accelerated frame a CEF page published.
pub struct CefContent {
    mailbox: Arc<OwnedFrameMailbox>,
    presenter: Option<TexturePresenter>,
    sinks: Sender<platform::SinkParts>,
    viewports: Sender<Viewport>,
    viewport: Option<Viewport>,
}

impl core::fmt::Debug for CefContent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("CefContent").finish_non_exhaustive()
    }
}

impl GpuContent for CefContent {
    fn setup(&mut self, context: &GpuContext<'_>) {
        platform::check_backend(context.adapter.get_info().backend);
        self.mailbox.set_waker(context.redraw.clone());
        // A feed dropped before the first frame has no page to install into.
        let _ = self
            .sinks
            .send(platform::sink_parts(context, Arc::clone(&self.mailbox)));
        self.presenter = Some(TexturePresenter::new(context));
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        let viewport = Viewport::of(frame);
        if self.viewport.replace(viewport) != Some(viewport) {
            let _ = self.viewports.send(viewport);
        }
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
        // Chromium paints on its own schedule: keep asking for frames so the
        // UI-side feed can request the next one.
        #[cfg(not(target_os = "macos"))]
        frame.request_redraw();
    }
}

/// Splits `page` into the UI-thread feed and the render-thread content.
///
/// The feed must be pumped once per host frame — see
/// [`GpuContentView::on_frame`] — and stays alive as long as the content is
/// installed.
#[must_use]
pub fn cef_presenter(page: CefPageHandle) -> (Rc<CefFeed>, CefContent) {
    let (sink_tx, sinks) = channel();
    let (viewport_tx, viewports) = channel();
    let feed = Rc::new(CefFeed {
        page,
        sinks,
        viewports,
        viewport: RefCell::new(None),
    });
    let content = CefContent {
        mailbox: Arc::new(OwnedFrameMailbox::new()),
        presenter: None,
        sinks: sink_tx,
        viewports: viewport_tx,
        viewport: None,
    };
    (feed, content)
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
///
/// A backend whose input arrives somewhere else entirely — GTK delivers it to
/// the `GtkGLArea`'s event controllers — uses this and owns a
/// [`CefSurfaceInput`] beside it.
#[must_use]
pub fn gpu_view(page: CefPageHandle) -> GpuContentView {
    let (feed, content) = cef_presenter(page);
    GpuContentView::new(content).on_frame(move || feed.pump())
}

/// Creates the presenter for one visible CEF page, wired to take its own input.
///
/// The pointer, keyboard, scroll and composition events landing on this layer
/// reach Chromium through [`CefSurfaceInput`], so a backend that routes surface
/// input to GPU content needs nothing CEF-specific.
#[must_use]
pub fn gpu_view_with_input(page: CefPageHandle) -> GpuContentView {
    let input = RefCell::new(CefSurfaceInput::new(page.clone()));
    gpu_view(page).on_input(move |event| input.borrow_mut().handle(event))
}
