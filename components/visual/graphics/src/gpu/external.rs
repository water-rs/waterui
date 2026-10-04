//! Externally produced frames as the content of a view's own engine layer.
//!
//! A video decoder, a camera or a compositor-backed web view already holds
//! its pixels in GPU memory — a `CVPixelBuffer`'s `IOSurface`, an
//! `AHardwareBuffer`, a dmabuf. [`ExternalFrameView`] presents those frames
//! without copying them: the producer imports each plane onto the host's
//! device and hands the engine a [`cherenkov_gpu::interop::ExternalFrame`],
//! which becomes the content of the view's layer. The engine samples the
//! planes where they are, converts `Y'CbCr` to its working space from the
//! frame's [`FrameColor`](cherenkov_gpu::interop::FrameColor), and orders
//! its reads behind the frame's [`FrameSync`](cherenkov_gpu::interop::FrameSync).
//! The layer is an ordinary layer — default blend, no backdrop — so an
//! engine that can promote it to a system-compositor plane may do so.
//!
//! Frames flow through a single-slot mailbox. The producer publishes from
//! any thread with [`FrameOutput::present`]; the newest frame replaces an
//! older one that was never shown, which releases the older planes back to
//! the producer's pool. The host drains the mailbox once per engine pass,
//! so a frame never rebuilds the view.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use cherenkov::kurbo;
use cherenkov_gpu::interop::{ExternalFrame, SharedDevice};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_core::{Environment, Native, NativeView, View};
use wgpu::{Device, Queue};

use super::{CaretQuery, FrameHook, InputHandler, RedrawHandle, Shared, measure_by_intrinsic_size};
use crate::input::SurfaceInputEvent;

/// A producer of [`ExternalFrame`]s: a video decoder, a camera, a web view.
///
/// The source lives on the UI thread with its view; the work that produces
/// frames usually does not. [`start`](Self::start) hands the source a
/// [`FrameOutput`] — cloneable and, on native targets, `Send` — that the
/// producing thread keeps and publishes through.
pub trait ExternalFrameSource: 'static {
    /// Starts producing frames onto `output`'s device.
    ///
    /// The host calls this on the UI thread once it has built the layer the
    /// frames compose into: when the view is first presented, and again on
    /// the replacement device after a device loss. Planes must be imported on
    /// [`FrameOutput::device`]; an output whose host is gone refuses frames
    /// with [`RetiredOutput`], which is the signal to stop producing for it.
    fn start(&mut self, output: FrameOutput);

    /// Runs one UI-thread tick once per presented frame, before the host's
    /// engine pass.
    ///
    /// Producers confined to the UI thread — a browser engine whose objects
    /// are not `Send` — pump their work here: they read
    /// [`FrameOutput::presented_size`] for the pixels and scale the layer
    /// presents at, drive their engine, and publish what it produced. It runs
    /// before any [`start`](Self::start) output exists too; a source with no
    /// live output does nothing.
    fn frame(&mut self) {}

    /// Whether every pixel of every frame is opaque.
    fn is_opaque(&self) -> bool {
        false
    }

    /// The size the frames are shown at, in logical points; `None` takes
    /// whatever the layout gives.
    ///
    /// A stream whose size is only known once it starts decoding leaves this
    /// `None` and answers [`measure`](Self::measure) instead.
    fn intrinsic_size(&self) -> Option<Size> {
        None
    }

    /// Measures the view against a layout proposal.
    ///
    /// The default answers [`intrinsic_size`](Self::intrinsic_size) when it
    /// knows one and fills the proposal otherwise. The frames are stretched
    /// to the measured bounds, so a source that must keep its aspect ratio
    /// answers a size with that ratio. This is layout-only and must not touch
    /// GPU state.
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure_by_intrinsic_size(self.intrinsic_size(), proposal)
    }

    /// Which dynamic range the source prefers its presentation target to
    /// carry: `Some(true)` for HDR, `Some(false)` for SDR, `None` to follow
    /// the host's policy. The view's own
    /// [`prefer_hdr_surface`](ExternalFrameView::prefer_hdr_surface) wins.
    fn preferred_surface_hdr(&self) -> Option<bool> {
        None
    }
}

/// Why [`FrameOutput::present`] refused a frame.
///
/// The host that owned this output is gone: its layer was torn down, or
/// rebuilt on a new device and given a new output through
/// [`ExternalFrameSource::start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the external-frame output was retired by its host")]
pub struct RetiredOutput;

/// Where an [`ExternalFrameSource`] publishes frames for one host layer.
///
/// Clone it into the producing thread. Every clone publishes into the same
/// single-slot mailbox.
#[derive(Clone)]
pub struct FrameOutput {
    device: SharedDevice,
    mailbox: Shared<Mailbox>,
}

impl fmt::Debug for FrameOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameOutput")
            .field("retired", &self.is_retired())
            .finish_non_exhaustive()
    }
}

impl FrameOutput {
    /// The device the host's engine renders with; frame planes must be
    /// imported onto it.
    #[must_use]
    pub const fn device(&self) -> &Device {
        &self.device.device
    }

    /// The device's queue.
    #[must_use]
    pub const fn queue(&self) -> &Queue {
        &self.device.queue
    }

    /// All four handles of the device chain, for a producer whose import
    /// needs the shared device — `cherenkov_gpu`'s native-plane import does.
    #[must_use]
    pub const fn shared_device(&self) -> &SharedDevice {
        &self.device
    }

    /// Wakes the host for another frame without publishing one.
    ///
    /// An engine that produces between presents — a browser's frame
    /// callback arriving on the UI thread — wakes the host through this so
    /// its next [`frame`](ExternalFrameSource::frame) tick runs; publishing
    /// the frame itself already wakes it.
    pub fn request_redraw(&self) {
        self.mailbox.redraw.request_redraw();
    }

    /// The physical pixel extent and display scale this output's layer
    /// presents at, reported by the host through
    /// [`FrameReceiver::set_presented_size`].
    ///
    /// `(0, 0, _)` until the host's first pass: nothing presents at zero
    /// size, so it also marks "no pass yet".
    #[must_use]
    pub fn presented_size(&self) -> (u32, u32, f32) {
        *self.mailbox.presented()
    }

    /// Publishes `frame` as the layer's next content and wakes the host.
    ///
    /// A frame the host has not drained yet is replaced, and dropping it
    /// releases its planes. The frame carries its own colour description
    /// and GPU-side sync; the engine reads the planes in place.
    ///
    /// # Errors
    /// [`RetiredOutput`] when the host that owned this output is gone; the
    /// frame is dropped.
    pub fn present(&self, frame: ExternalFrame) -> Result<(), RetiredOutput> {
        if self.is_retired() {
            return Err(RetiredOutput);
        }
        let superseded = self.mailbox.slot().replace(frame);
        // The superseded planes are released here, outside the lock.
        drop(superseded);
        self.mailbox.redraw.request_redraw();
        Ok(())
    }

    /// Whether the host that owned this output is gone.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.mailbox.retired.load(Ordering::Acquire)
    }
}

/// The single-slot handoff between a producer and its host.
///
/// The slot is a mutex because a newer frame must displace an undrained one
/// — a bounded channel would keep the oldest — and the lock guards only the
/// swap of one `Option`.
struct Mailbox {
    frame: Mutex<Option<ExternalFrame>>,
    retired: AtomicBool,
    /// The extent and scale the host presents at, mirrored into every
    /// `FrameOutput` clone for the producing thread to read.
    presented: Mutex<(u32, u32, f32)>,
    redraw: RedrawHandle,
}

impl Mailbox {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<ExternalFrame>> {
        self.frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn presented(&self) -> std::sync::MutexGuard<'_, (u32, u32, f32)> {
        self.presented
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The host's end of one [`FrameOutput`]: drains the frames its producer
/// publishes.
///
/// Dropping the receiver retires the output, so a producer publishing into
/// a torn-down or rebuilt layer learns it from [`RetiredOutput`].
pub struct FrameReceiver {
    mailbox: Shared<Mailbox>,
}

impl fmt::Debug for FrameReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameReceiver").finish_non_exhaustive()
    }
}

impl FrameReceiver {
    /// The newest frame published since the last call, if any.
    #[must_use]
    pub fn take(&self) -> Option<ExternalFrame> {
        self.mailbox.slot().take()
    }

    /// Reports the physical pixel extent and display scale the layer is
    /// presented at this pass; the producing side reads it back through
    /// [`FrameOutput::presented_size`].
    ///
    /// The report travels the mailbox like a frame does: the newest value
    /// replaces an unread one, and a source that was never resized sees the
    /// initial `(0, 0, _)`.
    pub fn set_presented_size(&self, size: (u32, u32), scale: f32) {
        *self.mailbox.presented() = (size.0, size.1, scale);
    }
}

impl Drop for FrameReceiver {
    fn drop(&mut self) {
        self.mailbox.retired.store(true, Ordering::Release);
        let pending = self.mailbox.slot().take();
        drop(pending);
    }
}

/// A host's handle to an [`ExternalFrameView`]'s source.
///
/// The view keeps a clone, so layout measurement reaches the live source,
/// and a host that rebuilds its layer after device loss starts the same
/// source again on the new device.
#[derive(Clone)]
pub struct ExternalFrameStream(Rc<RefCell<Box<dyn ExternalFrameSource>>>);

impl fmt::Debug for ExternalFrameStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalFrameStream")
            .finish_non_exhaustive()
    }
}

impl ExternalFrameStream {
    /// Starts the source producing onto `device` for a host layer.
    ///
    /// `redraw` wakes the host when a frame is published; the host drains
    /// the returned receiver on its next engine pass. The previous
    /// receiver, if the host still holds one, keeps its own output until it
    /// is dropped.
    #[must_use]
    pub fn start(&self, device: &SharedDevice, redraw: RedrawHandle) -> FrameReceiver {
        let mailbox = Shared::new(Mailbox {
            frame: Mutex::new(None),
            retired: AtomicBool::new(false),
            presented: Mutex::new((0, 0, 1.0)),
            redraw,
        });
        self.0.borrow_mut().start(FrameOutput {
            device: device.clone(),
            mailbox: Shared::clone(&mailbox),
        });
        FrameReceiver { mailbox }
    }

    /// Runs the source's per-presented-frame tick — the UI thread's turn
    /// before the host's engine pass.
    pub fn frame(&self) {
        self.0.borrow_mut().frame();
    }

    /// Measures the source against a layout proposal.
    #[must_use]
    pub fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.0.borrow().measure(proposal)
    }

    /// The source's HDR preference for its presentation target.
    #[must_use]
    pub fn preferred_surface_hdr(&self) -> Option<bool> {
        self.0.borrow().preferred_surface_hdr()
    }
}

/// A view whose pixels are the frames an [`ExternalFrameSource`] publishes,
/// composed as the content of the view's own engine layer.
///
/// Each frame is stretched to the view's bounds.
///
/// # Layout Behavior
///
/// Stretches on both axes unless the source reports an intrinsic size, in
/// which case it is that size and does not stretch.
pub struct ExternalFrameView {
    stream: ExternalFrameStream,
    intrinsic_size: Option<Size>,
    opaque: bool,
    surface_prefers_hdr: Option<bool>,
    input: Option<InputHandler>,
    frame: Option<FrameHook>,
    caret: Option<CaretQuery>,
    label: Option<String>,
    value: Option<String>,
}

impl fmt::Debug for ExternalFrameView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalFrameView")
            .field("intrinsic_size", &self.intrinsic_size)
            .field("opaque", &self.opaque)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl ExternalFrameView {
    /// A view presenting the frames `source` publishes.
    #[must_use]
    pub fn new(source: impl ExternalFrameSource) -> Self {
        Self {
            intrinsic_size: source.intrinsic_size(),
            opaque: source.is_opaque(),
            stream: ExternalFrameStream(Rc::new(RefCell::new(Box::new(source)))),
            surface_prefers_hdr: None,
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

    /// Runs `hook` on the UI thread once per presented frame, after the
    /// source's own [`frame`](ExternalFrameSource::frame) tick.
    #[must_use]
    pub fn on_frame(mut self, hook: impl Fn() + 'static) -> Self {
        self.frame = Some(Rc::new(hook));
        self
    }

    /// Answers where the frames' text caret is, for input-method panels.
    #[must_use]
    pub fn on_ime_caret(mut self, query: impl Fn() -> Option<kurbo::Rect> + 'static) -> Self {
        self.caret = Some(Rc::new(query));
        self
    }

    /// The frames' text caret, if they have one right now.
    #[must_use]
    pub fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.caret.as_ref().and_then(|query| query())
    }

    /// Runs the per-frame UI turn: the source's
    /// [`frame`](ExternalFrameSource::frame) tick, then the installed hook.
    pub fn frame(&self) {
        self.stream.frame();
        if let Some(hook) = &self.frame {
            hook();
        }
    }

    /// Whether this view takes input events.
    #[must_use]
    pub const fn wants_input_events(&self) -> bool {
        self.input.is_some()
    }

    /// Routes an input event to the view's handler.
    pub fn input(&self, event: &SurfaceInputEvent) {
        if let Some(handler) = &self.input {
            handler(event);
        }
    }

    /// The accessibility name of the frames.
    #[must_use]
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The accessibility value of the frames.
    #[must_use]
    pub fn described(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Prefer an HDR presentation target for this view even when the
    /// surrounding platform style is SDR.
    #[must_use]
    pub const fn prefer_hdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(true);
        self
    }

    /// Prefer an SDR presentation target for this view even when HDR is
    /// available.
    #[must_use]
    pub const fn prefer_sdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(false);
        self
    }

    /// The natural size of the frames, if the source reported one.
    #[must_use]
    pub const fn intrinsic_size(&self) -> Option<Size> {
        self.intrinsic_size
    }

    /// Whether the frames are opaque.
    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        self.opaque
    }

    /// Measures the view against a layout proposal, asking the live source.
    #[must_use]
    pub fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.stream.measure(proposal)
    }

    /// Resolves this view's explicit or source-provided HDR preference.
    ///
    /// `None` means follow the surrounding platform style.
    #[must_use]
    pub fn resolved_hdr_preference(&self) -> Option<bool> {
        self.surface_prefers_hdr
            .or_else(|| self.stream.preferred_surface_hdr())
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

    /// The host's handle to the source.
    #[must_use]
    pub fn stream(&self) -> ExternalFrameStream {
        self.stream.clone()
    }
}

impl NativeView for ExternalFrameView {
    fn stretch_axis(&self) -> StretchAxis {
        if self.intrinsic_size.is_some() {
            StretchAxis::None
        } else {
            StretchAxis::Both
        }
    }
}

impl View for ExternalFrameView {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(self)
    }
}
