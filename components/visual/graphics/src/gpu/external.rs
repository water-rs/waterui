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
//!
//! An interactive source — a web view — takes input, answers a caret and
//! pumps its UI-thread work through the same hooks a
//! [`GpuContentView`](super::GpuContentView) carries, and keeps its own
//! viewport at the layer's [`PresentedSize`].

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use cherenkov_gpu::interop::{ExternalFrame, SharedDevice};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_core::{Environment, Native, NativeView, View};
use wgpu::{Device, Queue};

use super::{RedrawHandle, Shared, SurfaceHooks, measure_by_intrinsic_size};
use crate::draw::kurbo;
use crate::input::SurfaceInputEvent;
use crate::offscreen::OffscreenSize;

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
    /// A source that reuses its planes learns when the host is done with a
    /// frame through `ExternalFrame::on_release`, and one without a natural
    /// size renders at [`FrameOutput::presented_size`].
    fn start(&mut self, output: FrameOutput);

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

/// The size a host layer presents its frames at: the physical pixel extent
/// the layer covers on screen and the display scale, in pixels per logical
/// point.
///
/// A source whose frames have no natural size — a web view — renders at
/// exactly this extent so the engine never resamples its pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentedSize {
    /// The layer's extent in physical pixels.
    pub pixels: OffscreenSize,
    /// Physical pixels per logical point on the display the layer shows on.
    pub scale: f64,
}

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

    /// The host engine's whole device-creation chain — instance, adapter,
    /// device and queue — for importers that open their own view of the
    /// device, such as `cherenkov_gpu::interop::vulkan::Device::new`.
    #[must_use]
    pub const fn shared_device(&self) -> &SharedDevice {
        &self.device
    }

    /// Wakes the host for another pass without publishing a frame.
    ///
    /// A source whose producer must be ticked from the UI thread — a
    /// browser engine driven by external begin frames — requests the next
    /// pass here so its [`ExternalFrameView::on_frame`] hook keeps running.
    pub fn request_redraw(&self) {
        self.mailbox.redraw.request_redraw();
    }

    /// The size the host layer presents frames at, as of its latest pass;
    /// `None` until the host has run its first pass.
    #[must_use]
    pub fn presented_size(&self) -> Option<PresentedSize> {
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
    /// The host's latest presented size, written every pass.
    presented: Mutex<Option<PresentedSize>>,
    retired: AtomicBool,
    redraw: RedrawHandle,
}

impl Mailbox {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<ExternalFrame>> {
        self.frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn presented(&self) -> std::sync::MutexGuard<'_, Option<PresentedSize>> {
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

    /// Records the size the host layer presents frames at this pass, which
    /// the source reads back through [`FrameOutput::presented_size`]. The
    /// host reports it on every pass, from wherever it sizes the layer.
    pub fn report_presented(&self, size: PresentedSize) {
        *self.mailbox.presented() = Some(size);
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
    /// Starts the source producing onto `device` — the host engine's
    /// device-creation chain — for a host layer.
    ///
    /// `redraw` wakes the host when a frame is published or the source asks
    /// for a pass; the host drains the returned receiver on its next engine
    /// pass and reports its presented size there. The previous receiver, if
    /// the host still holds one, keeps its own output until it is dropped.
    #[must_use]
    pub fn start(&self, device: &SharedDevice, redraw: RedrawHandle) -> FrameReceiver {
        let mailbox = Shared::new(Mailbox {
            frame: Mutex::new(None),
            presented: Mutex::new(None),
            retired: AtomicBool::new(false),
            redraw,
        });
        self.0.borrow_mut().start(FrameOutput {
            device: device.clone(),
            mailbox: Shared::clone(&mailbox),
        });
        FrameReceiver { mailbox }
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
    hooks: SurfaceHooks,
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
            hooks: SurfaceHooks::default(),
            label: None,
            value: None,
        }
    }

    /// Receives pointer, keyboard and gesture events the backend routes to
    /// this view.
    #[must_use]
    pub fn on_input(mut self, handler: impl Fn(&SurfaceInputEvent) + 'static) -> Self {
        self.hooks.input = Some(Rc::new(handler));
        self
    }

    /// Runs `hook` on the UI thread once per host pass, before the host
    /// drains the newest published frame.
    #[must_use]
    pub fn on_frame(mut self, hook: impl Fn() + 'static) -> Self {
        self.hooks.frame = Some(Rc::new(hook));
        self
    }

    /// Answers where the source's text caret is, for input-method panels.
    #[must_use]
    pub fn on_ime_caret(mut self, query: impl Fn() -> Option<kurbo::Rect> + 'static) -> Self {
        self.hooks.caret = Some(Rc::new(query));
        self
    }

    /// Whether this view takes input events.
    #[must_use]
    pub const fn wants_input_events(&self) -> bool {
        self.hooks.wants_input_events()
    }

    /// Routes an input event to the view's handler.
    pub fn input(&self, event: &SurfaceInputEvent) {
        self.hooks.input(event);
    }

    /// The source's text caret, if it has one right now.
    #[must_use]
    pub fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.hooks.ime_caret()
    }

    /// Runs the per-frame UI hook, if any.
    pub fn frame(&self) {
        self.hooks.frame();
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
