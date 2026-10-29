//! Owned snapshot rendering for the GPU-bearing icon/drag/cache consumers:
//! the same native-plus-output compositor `ViewCapture` runs for
//! effect views, pointed at an owned Metal texture, with one final RGBA8
//! readback at the end.
//!
//! A capture through here is a deliberate render destination of the same
//! producer — never a CPU screenshot path, a duplicated producer, or an
//! onscreen presentation. External surfaces inside the subtree take part
//! through their `CapturableSurface` contract, so a GPU surface or a
//! nested filter contributes real pixels rather than a blank.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

#[cfg(all(target_os = "macos", feature = "draggable"))]
use cocoa_ui::objc2::AnyThread;
use cocoa_ui::objc2::MainThreadOnly;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::ProtocolObject;
#[cfg(all(target_os = "macos", feature = "draggable"))]
use cocoa_ui::objc2_core_foundation::CGSize;
use cocoa_ui::objc2_metal::{
    MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use waterui_backend_core::Environment;

/// The platform's native image object — `NSImage` on macOS, `UIImage` on
/// iOS — the type icon and drag endpoints publish.
#[cfg(target_os = "macos")]
type PlatformImage = cocoa_ui::objc2_app_kit::NSImage;
/// See the macOS alias.
#[cfg(target_os = "ios")]
type PlatformImage = cocoa_ui::objc2_ui_kit::UIImage;

/// One owned RGBA8 premultiplied raster — the single readback the API
/// contract allows.
#[derive(Debug)]
pub struct CapturedRgba {
    /// Row-major RGBA8 premultiplied texels, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
    /// Raster width in pixels.
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
    /// Pixels per point the raster was rendered at — the backing scale of
    /// the window the capture ran in.
    pub scale: f64,
}

/// Why [`capture_rgba`] produced no raster.
#[derive(Debug)]
pub enum CaptureFailure {
    /// A GPU surface inside the captured subtree reported a terminal
    /// failure — no recapture on its context generation can produce the
    /// frame. Carries the surface's own `Arc`'d cause.
    GpuSurfaceFailed(Arc<dyn std::error::Error + Send + Sync>),
    /// The native-view composition command buffer ended in a status other
    /// than `Completed`, so the composited frame never landed.
    CompositionFailed {
        /// The status the command buffer ended with.
        status: cocoa_ui::objc2_metal::MTLCommandBufferStatus,
        /// Metal's description of the command buffer's error, when it
        /// attached one.
        error: Option<String>,
    },
    /// No connected window scene can host a detached view's capture window.
    #[cfg(target_os = "ios")]
    NoWindowScene,
    /// The Metal device refused the capture's readback texture.
    TargetAllocation {
        /// The requested target width in pixels.
        width: u32,
        /// The requested target height in pixels.
        height: u32,
    },
}

impl core::fmt::Display for CaptureFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::GpuSurfaceFailed(_) => {
                f.write_str("a GPU surface in the captured subtree failed")
            }
            Self::CompositionFailed { status, error } => {
                write!(
                    f,
                    "the native view composition command buffer ended with status {status:?}"
                )?;
                if let Some(error) = error {
                    write!(f, ": {error}")?;
                }
                Ok(())
            }
            #[cfg(target_os = "ios")]
            Self::NoWindowScene => {
                f.write_str("no UIWindowScene is connected to host the capture window")
            }
            Self::TargetAllocation { width, height } => write!(
                f,
                "the device could not allocate a {width}x{height} capture target"
            ),
        }
    }
}

impl std::error::Error for CaptureFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::GpuSurfaceFailed(cause) => Some(&**cause),
            Self::CompositionFailed { .. } => None,
            #[cfg(target_os = "ios")]
            Self::NoWindowScene => None,
            Self::TargetAllocation { .. } => None,
        }
    }
}

/// A one-shot main-thread signal a completion or redraw callback fires
/// into — the wake half of the capture's async contract.
#[derive(Default)]
struct Signal {
    /// Whether `fire` has run since the last `wait` poll consumed it.
    flag: bool,
    /// The pending waiter, if the future polled before the callback fired.
    waker: Option<Waker>,
}

impl Signal {
    /// Fires the signal — wakes the waiter if one is armed.
    fn fire(self_shared: &Rc<RefCell<Self>>) {
        let waker = {
            let mut this = self_shared.borrow_mut();
            this.flag = true;
            this.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Consumes one signal edge; pends until `fire` runs.
    #[expect(
        clippy::future_not_send,
        reason = "the signal is a main-thread `Rc` the capture's callbacks share"
    )]
    async fn wait(self_shared: &Rc<RefCell<Self>>) {
        core::future::poll_fn(|cx: &mut Context<'_>| {
            let mut this = self_shared.borrow_mut();
            if this.flag {
                this.flag = false;
                Poll::Ready(())
            } else {
                this.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await;
    }
}

/// Hosts a detached view in a private offscreen window while a capture
/// needs window-bound control rendering; restores the original parent on
/// drop. `None` while the view already has a window.
pub struct WindowHost {
    /// The temporary window — kept alive for the capture's duration and
    /// never ordered in; the struct's drop restores the view before the
    /// window dies. Never read directly: its only job is to live.
    #[cfg(target_os = "macos")]
    #[expect(
        dead_code,
        reason = "the never-ordered capture window's role is to stay alive for the capture's duration"
    )]
    window: Retained<cocoa_ui::objc2_app_kit::NSWindow>,
    /// See the macOS field.
    #[cfg(target_os = "ios")]
    window: Retained<cocoa_ui::bitmap::CaptureWindow>,
    /// The view's parent before the re-host, restored on drop.
    superview: Option<Retained<cocoa_ui::PlatformView>>,
    /// The sibling the view preceded in `superview` — `AppKit` has no
    /// indexed insert, so restoration is relative to it (`None` = last).
    #[cfg(target_os = "macos")]
    next_sibling: Option<Retained<cocoa_ui::PlatformView>>,
    /// The view's index in `superview`'s subviews — iOS restores by index.
    #[cfg(target_os = "ios")]
    index: usize,
    /// The view's frame before the re-host — the offscreen layout may have
    /// resized it; restored on drop.
    frame: cocoa_ui::Rect,
    /// The view being hosted.
    view: Retained<cocoa_ui::PlatformView>,
}

impl WindowHost {
    /// Re-hosts `view` iff it has no window — native controls need one to
    /// render their chrome. `Ok(None)` while the view already has one.
    ///
    /// # Errors
    ///
    /// [`CaptureFailure::NoWindowScene`] on iOS when no connected
    /// `UIWindowScene` can host the offscreen window.
    #[cfg_attr(
        target_os = "macos",
        expect(
            clippy::unnecessary_wraps,
            reason = "only UIKit's window-scene lookup can fail; AppKit's offscreen window always attaches"
        )
    )]
    pub fn attach_if_windowless(
        view: &Retained<cocoa_ui::PlatformView>,
        mtm: cocoa_ui::MainThreadMarker,
    ) -> Result<Option<Self>, CaptureFailure> {
        if cocoa_ui::view::window(view).is_some() {
            return Ok(None);
        }
        let bounds = cocoa_ui::view::bounds(view);
        let size = cocoa_ui::Size::new(bounds.size.width.max(1.0), bounds.size.height.max(1.0));
        let frame = cocoa_ui::view::frame(view);
        let superview = cocoa_ui::view::superview(view);
        #[cfg(target_os = "macos")]
        let next_sibling = superview.as_ref().and_then(|parent| {
            let subviews = cocoa_ui::view::subviews(parent);
            subviews
                .iter()
                .position(|sibling| std::ptr::eq(&raw const **sibling, &raw const **view))
                .and_then(|index| subviews.get(index + 1).cloned())
        });
        #[cfg(target_os = "ios")]
        let index = superview
            .as_ref()
            .and_then(|parent| {
                cocoa_ui::view::subviews(parent)
                    .iter()
                    .position(|sibling| std::ptr::eq(&raw const **sibling, &raw const **view))
            })
            .unwrap_or(0);
        #[cfg(target_os = "macos")]
        let window = {
            // The windowless capture contract: the host is never ordered
            // in — a view attached to it still drives AppKit's
            // window-dependent rendering paths for `renderInContext`.
            let window = cocoa_ui::bitmap::capture_window(mtm, size);
            let content = window.contentView().expect("offscreen window content");
            cocoa_ui::view::add_subview(&content, view);
            // Text fields stay out of a capture unless marked dirty — the
            // existing bitmap path's preparation.
            cocoa_ui::bitmap::force_text_fields_display(view);
            window
        };
        #[cfg(target_os = "ios")]
        let window = {
            let scene =
                cocoa_ui::bitmap::any_window_scene().ok_or(CaptureFailure::NoWindowScene)?;
            let window = cocoa_ui::bitmap::make_offscreen_window(mtm, &scene, size);
            // The `UIViewController` containment + unhide + layout is the
            // only correct UIKit mount — a bare `addSubview` leaves the
            // window without a root controller.
            cocoa_ui::bitmap::show_capture_window(&window, view, size);
            window
        };
        Ok(Some(Self {
            window,
            superview,
            #[cfg(target_os = "macos")]
            next_sibling,
            #[cfg(target_os = "ios")]
            index,
            frame,
            view: view.clone(),
        }))
    }
}

impl Drop for WindowHost {
    /// Restores the view to its original parent's exact child position and
    /// frame — appending would silently reorder the host's z-order, and a
    /// resized frame would linger.
    fn drop(&mut self) {
        // The macOS capture window was never ordered in — nothing to hide;
        // the iOS one hides first — a re-hosted view must never be left
        // visible inside it — then the view returns to its parent.
        #[cfg(target_os = "ios")]
        cocoa_ui::bitmap::close_capture_window(&self.window);
        cocoa_ui::view::remove_from_superview(&self.view);
        if let Some(superview) = &self.superview {
            #[cfg(target_os = "ios")]
            {
                #[expect(
                    clippy::cast_possible_wrap,
                    reason = "a view hierarchy never reaches `NSInteger::MAX` subviews"
                )]
                superview.insertSubview_atIndex(&self.view, self.index as isize);
            }
            #[cfg(target_os = "macos")]
            match &self.next_sibling {
                Some(sibling) => superview.addSubview_positioned_relativeTo(
                    &self.view,
                    cocoa_ui::objc2_app_kit::NSWindowOrderingMode::Below,
                    Some(&**sibling),
                ),
                None => superview.addSubview(&self.view),
            }
        }
        // The offscreen layout may have resized the view — restore its frame
        // whether or not it had a parent to return to.
        cocoa_ui::view::set_frame(&self.view, self.frame);
    }
}

/// Ends a live capture's external-rendering scopes exactly once — the
/// capture's own `shutdown` must run even when the awaiting task is
/// dropped mid-flight (an icon or drag mount cancelled); `ViewCapture`'s
/// Drop asserts the scopes are gone, so the guard owns that contract.
struct CaptureGuard {
    /// The live capture while `shutdown` is owed.
    capture: Option<Rc<cocoa_ui::capture::ViewCapture>>,
}

impl CaptureGuard {
    /// The capture under guard.
    const fn capture(&self) -> &Rc<cocoa_ui::capture::ViewCapture> {
        self.capture.as_ref().expect("capture guard is armed")
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        if let Some(capture) = self.capture.take() {
            capture.shutdown();
        }
    }
}

/// The shared `RGBA8Unorm` readback target — `.shared` storage so the one
/// `getBytes` the API permits can run without a blit.
///
/// # Errors
///
/// [`CaptureFailure::TargetAllocation`] when the device refuses the texture.
fn rgba_target(
    device: &ProtocolObject<dyn MTLDevice>,
    width: u32,
    height: u32,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, CaptureFailure> {
    // SAFETY: a 2D descriptor is always valid to construct.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::RGBA8Unorm,
            width as usize,
            height as usize,
            false,
        )
    };
    descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
    descriptor.setStorageMode(MTLStorageMode::Shared);
    device
        .newTextureWithDescriptor(&descriptor)
        .ok_or(CaptureFailure::TargetAllocation { width, height })
}

/// `view`'s subtree rendered into an owned `RGBA8` raster at the backing
/// scale of the window it is captured in — `ceil(bounds * scale)` physical
/// pixels, the scale reported on the result.
///
/// The scale is resolved here, after a windowless view has been rehosted:
/// a detached view has no screen and answers `1.0`, while the capture
/// window sits on the screen the image will be shown on.
///
/// The compositor's native raster pass (`CALayer.renderInContext` into
/// shared-buffer memory) and output pass (`CapturableSurface` external
/// rendering) write the texture; one
/// `getBytes` readback follows. A deferred capture — a GPU surface that
/// had no current content, or a context lost mid-capture — is honored by
/// waiting on the surfaces' own redraw wake and recapturing; the future
/// only completes once a frame really landed, so no bogus-ready blank
/// ships. A terminal surface failure ends the loop instead: no redraw
/// can produce the frame on the failed context, so waiting would never
/// return — the typed error propagates to the caller.
///
/// # Errors
///
/// [`CaptureFailure::GpuSurfaceFailed`] with the `Arc`'d cause a
/// surface's terminal `CaptureError::Failed` reported,
/// [`CaptureFailure::CompositionFailed`] when the native-view composition
/// command buffer did not complete,
/// [`CaptureFailure::NoWindowScene`] when a detached view has no window
/// scene to host it, and [`CaptureFailure::TargetAllocation`] when the
/// readback target cannot be allocated. `render_to_rgba` maps them onto
/// its `RenderError`; [`template_image`] and [`drag_image`] fail fast
/// through [`terminal_capture_failure`].
#[expect(
    clippy::future_not_send,
    reason = "capture runs on the main thread; the future holds the main-thread capture and window host"
)]
pub async fn capture_rgba(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
    mtm: cocoa_ui::MainThreadMarker,
) -> Result<CapturedRgba, CaptureFailure> {
    let view = cocoa_ui::view::retain_base(view);
    let host = WindowHost::attach_if_windowless(&view, mtm)?;
    if host.is_some() {
        // A re-hosted view lays out against the offscreen window before the
        // capture reads its actual bounds.
        cocoa_ui::view::layout_immediately(&view);
    }
    let scale = cocoa_ui::view::backing_scale_factor(&view).max(1.0);

    let bounds = cocoa_ui::view::bounds(&view);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Ok(CapturedRgba {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            scale,
        });
    }
    let (width, height) = pixel_extent(bounds.size, scale);

    let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(mtm, view.clone()));
    // `shutdown` is owed even if the awaiting task is cancelled.
    let guard = CaptureGuard {
        capture: Some(capture),
    };

    // The redraw contract: an external surface inside the subtree asks the
    // capture for a new frame through this hook — deferred captures retry
    // when it fires.
    let redraw = Rc::new(RefCell::new(Signal::default()));
    guard.capture().set_on_redraw({
        let redraw = redraw.clone();
        move || Signal::fire(&redraw)
    });

    // Context, device and destination resolve per attempt: a context
    // replacement mid-wait makes the old-generation target obsolete, and
    // the readback must come from the successfully fenced current one.
    let runtime = crate::gpu_runtime::runtime(env);
    let target = capture_attempts(&runtime, guard.capture(), &redraw, width, height, mtm).await?;
    drop(guard);
    drop(host);

    Ok(read_target(&target, width, height, scale))
}

/// The capture pass's pixel extent for a point `size` at `scale` — `ceil`
/// so a positive sub-pixel bound still covers a real pixel row.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a view's pixel dimensions fit u32 and are positive"
)]
fn pixel_extent(size: cocoa_ui::Size, scale: f64) -> (u32, u32) {
    (
        (size.width * scale).ceil() as u32,
        (size.height * scale).ceil() as u32,
    )
}

/// The one permitted readback: `width` × `height` premultiplied RGBA8
/// texels out of the `.shared` capture target.
fn read_target(
    target: &ProtocolObject<dyn MTLTexture>,
    width: u32,
    height: u32,
    scale: f64,
) -> CapturedRgba {
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: `pixels` holds `width*4` bytes per row for `height` rows and
    // `target` is `.shared` — the region read stays in bounds.
    unsafe {
        target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
            std::ptr::NonNull::new(pixels.as_mut_ptr().cast())
                .expect("the texel buffer is non-null"),
            width as usize * 4,
            MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: width as usize,
                    height: height as usize,
                    depth: 1,
                },
            },
            0,
        );
    }
    CapturedRgba {
        pixels,
        width,
        height,
        scale,
    }
}

/// The frame drive a windowless capture host runs while it waits for owed
/// content: one [`ViewCapture`](cocoa_ui::capture::ViewCapture) kept open
/// from mount to the terminal capture.
///
/// A GPU surface whose window cannot present never attaches, so its redraw
/// requests would die on a display link that never runs — and a surface
/// with no frame measures zero, so the capture's ordinary collection gives
/// it no spec. `ViewCapture` joins every resolved surface that cannot
/// present for redraw routing — see
/// [`CapturableSurface::can_present`](cocoa_ui::capture::CapturableSurface::can_present)
/// — and this pump answers each request with one capture pass into a
/// scratch texture with no readback: the pass lays the root out, records
/// every scene's content, and settles the registrations the awaited
/// content — a decoded photo — resolves on.
///
/// [`capture_rgba`](Self::capture_rgba) is the terminal half: surface
/// redraws move off the pump onto its readback retry loop, which renders
/// the same pass into the `.shared` target the one permitted readback
/// takes. The move happens inside the drive's one redraw hook, never by
/// replacing it: a surface binds the hook when it joins, so a replacement
/// would leave every surface the pump already joined waking the pump
/// while the retry loop waits on a redraw that never comes.
pub struct CaptureDrive {
    /// The open capture the pump passes and the terminal capture share.
    capture: Rc<cocoa_ui::capture::ViewCapture>,
    /// The mounted root every pass lays out and captures.
    root: Retained<cocoa_ui::PlatformView>,
    /// The environment's GPU runtime — the context resolves per pass.
    runtime: waterui_graphics::gpu::GpuRuntime,
    /// The capture window's backing scale — the scratch and readback
    /// targets' point-to-pixel factor.
    scale: f64,
    /// The main-thread proof the queued pump reuses.
    mtm: cocoa_ui::MainThreadMarker,
    /// Whether a pump pass is queued or in flight — redraw requests
    /// coalesce onto it.
    queued: Cell<bool>,
    /// The terminal capture's redraw signal once
    /// [`capture_rgba`](Self::capture_rgba) runs — the hook fires it
    /// instead of queueing a pump pass from then on.
    terminal: RefCell<Option<Rc<RefCell<Signal>>>>,
}

impl CaptureDrive {
    /// Opens the capture over `root` — already inside its windowless
    /// capture host — and starts the pump: the first pass runs on the
    /// next main-queue turn, joining every surface that cannot present,
    /// and each redraw they issue afterwards runs another.
    #[must_use]
    pub fn begin(
        root: &cocoa_ui::PlatformView,
        env: &Environment,
        mtm: cocoa_ui::MainThreadMarker,
    ) -> Rc<Self> {
        let root = cocoa_ui::view::retain_base(root);
        let scale = cocoa_ui::view::backing_scale_factor(root.as_ref());
        let drive = Rc::new(Self {
            capture: Rc::new(cocoa_ui::capture::ViewCapture::new(mtm, root.clone())),
            root,
            runtime: crate::gpu_runtime::runtime(env),
            scale,
            mtm,
            queued: Cell::new(false),
            terminal: RefCell::new(None),
        });
        // Surface redraws route here through the external-rendering
        // scopes the capture opens on every resolved surface that cannot
        // present: each request queues one coalesced pump pass until the
        // terminal capture starts, then wakes its retry loop instead.
        drive.capture.set_on_redraw({
            let drive = Rc::downgrade(&drive);
            move || {
                if let Some(drive) = drive.upgrade() {
                    drive.redraw();
                }
            }
        });
        drive.request_pump();
        drive
    }

    /// The capture's one redraw hook: a pump pass while the drive waits
    /// for content, the terminal retry loop's wake once it captures.
    fn redraw(self: &Rc<Self>) {
        let terminal = self.terminal.borrow().clone();
        match terminal {
            Some(signal) => Signal::fire(&signal),
            None => self.request_pump(),
        }
    }

    /// Queues one capture pass — the redraw answer for surfaces that
    /// cannot present. Requests coalesce: at most one pass is queued or
    /// in flight, and the pass itself lays the root out, so a request
    /// that lands while one runs schedules the next.
    fn request_pump(self: &Rc<Self>) {
        if self.queued.replace(true) {
            return;
        }
        let drive = Rc::downgrade(self);
        cocoa_ui::main_queue::enqueue_local(self.mtm, move |_mtm| {
            if let Some(drive) = drive.upgrade() {
                drive.pump();
            }
        });
    }

    /// One capture pass into a scratch texture with no readback — the
    /// frame a presentable surface would have drawn on its link's next
    /// update. The pass's own prepare lays the root out first, so bounds
    /// owed content changed are the bounds the collection sees.
    fn pump(self: &Rc<Self>) {
        self.queued.set(false);
        let context = self.runtime.context();
        if context.device_lost_reason().is_some() {
            // A dead context produces no frame: the pass re-requests on
            // the rebuilt context's publication — the same wake the
            // parked surfaces take.
            let generation = context.generation();
            let runtime = self.runtime.clone();
            let drive = Rc::downgrade(self);
            executor_core::spawn_local(async move {
                runtime.context_after(generation).await;
                if let Some(drive) = drive.upgrade() {
                    drive.request_pump();
                }
            })
            .detach();
            return;
        }
        let bounds = cocoa_ui::view::bounds(&self.root);
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return;
        }
        let (width, height) = pixel_extent(bounds.size, self.scale);
        let device = crate::gpu_runtime::raw_metal_device(&context);
        let target = scratch_target(&device, width, height);
        self.capture
            .capture(&target, context.generation(), |outcome| {
                if let Err(cocoa_ui::capture::CaptureError::Failed(error)) = &outcome {
                    tracing::debug!("windowless capture drive pass failed: {error}");
                }
            });
    }

    /// The terminal capture `capture_rgba` performs, over the drive's
    /// already-open `ViewCapture`: surface redraws move off the pump onto
    /// the readback's retry loop — `capture_attempts` then renders the
    /// same pass into the `.shared` target the readback takes.
    ///
    /// # Errors
    ///
    /// [`CaptureFailure::GpuSurfaceFailed`] with the `Arc`'d cause a
    /// surface's terminal `CaptureError::Failed` reported, or
    /// [`CaptureFailure::CompositionFailed`] when the native-view
    /// composition command buffer did not complete.
    #[expect(
        clippy::future_not_send,
        reason = "the terminal capture awaits main-thread signals shared through `Rc`"
    )]
    pub async fn capture_rgba(self: &Rc<Self>) -> Result<CapturedRgba, CaptureFailure> {
        let bounds = cocoa_ui::view::bounds(&self.root);
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return Ok(CapturedRgba {
                pixels: Vec::new(),
                width: 0,
                height: 0,
                scale: self.scale,
            });
        }
        let (width, height) = pixel_extent(bounds.size, self.scale);
        // The redraw contract the retry loop waits on — surface requests
        // now wake a recapture instead of queueing a pump pass.
        let redraw = Rc::new(RefCell::new(Signal::default()));
        let previous = self.terminal.borrow_mut().replace(redraw.clone());
        assert!(
            previous.is_none(),
            "a capture drive performs one terminal capture"
        );
        let target = capture_attempts(
            &self.runtime,
            &self.capture,
            &redraw,
            width,
            height,
            self.mtm,
        )
        .await?;
        Ok(read_target(&target, width, height, self.scale))
    }
}

impl Drop for CaptureDrive {
    /// The capture's own `shutdown` ends every external-rendering scope —
    /// the same contract `CaptureGuard` keeps for `capture_rgba`'s own
    /// capture.
    fn drop(&mut self) {
        self.capture.shutdown();
    }
}

/// A pump pass's render destination: `RenderTarget` usage in private
/// storage — the pass exists to record scenes and settle registrations,
/// so its pixels are never sampled or read back.
fn scratch_target(
    device: &ProtocolObject<dyn MTLDevice>,
    width: u32,
    height: u32,
) -> Retained<ProtocolObject<dyn MTLTexture>> {
    // SAFETY: a 2D descriptor is always valid to construct.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::RGBA8Unorm,
            width as usize,
            height as usize,
            false,
        )
    };
    descriptor.setUsage(MTLTextureUsage::RenderTarget);
    descriptor.setStorageMode(MTLStorageMode::Private);
    device
        .newTextureWithDescriptor(&descriptor)
        .expect("the device allocates the capture drive's scratch target")
}

/// The recapture loop behind [`capture_rgba`]: context, device and
/// destination resolve per attempt — a context replacement mid-wait
/// makes the old-generation target obsolete, and the readback must come
/// from the successfully fenced current one. A deferred capture waits on
/// the surfaces' redraw wake or the next context publication, whichever
/// lands first; a terminal surface failure stops the loop, because no
/// retry on the failed generation can produce the frame.
///
/// # Errors
///
/// [`CaptureFailure::GpuSurfaceFailed`] with the `Arc`'d cause a
/// surface's terminal `CaptureError::Failed` reported,
/// [`CaptureFailure::CompositionFailed`] when the native-view composition
/// command buffer did not complete, or
/// [`CaptureFailure::TargetAllocation`] when the attempt's readback
/// target cannot be allocated — [`capture_rgba`] propagates all three.
#[expect(
    clippy::future_not_send,
    reason = "the recapture loop awaits main-thread signals shared through `Rc`"
)]
async fn capture_attempts(
    runtime: &waterui_graphics::gpu::GpuRuntime,
    capture: &Rc<cocoa_ui::capture::ViewCapture>,
    redraw: &Rc<RefCell<Signal>>,
    width: u32,
    height: u32,
    mtm: cocoa_ui::MainThreadMarker,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, CaptureFailure> {
    let target = loop {
        let context = runtime.context();
        // A lost context cannot produce pixels: park on the next
        // publication before allocating anything against it — the
        // destination must never be allocated on a dead device.
        if context.device_lost_reason().is_some() {
            runtime.context_after(context.generation()).await;
            continue;
        }
        let device = crate::gpu_runtime::raw_metal_device(&context);
        let target = rgba_target(&device, width, height)?;

        let completion = Rc::new(RefCell::new(Signal::default()));
        let outcome = Rc::new(RefCell::new(None));
        // The completion is `Send` — it parks on Metal's own queue before
        // hopping to the main queue — so the slots ride `MainThreadBound`.
        let slot = dispatch2::MainThreadBound::new((completion.clone(), outcome.clone()), mtm);
        capture.capture(&target, context.generation(), move |result| {
            let mtm = cocoa_ui::MainThreadMarker::new()
                .expect("the capture completion runs on the main thread");
            let (completion, outcome) = slot.get(mtm);
            *outcome.borrow_mut() = Some(result);
            Signal::fire(completion);
        });
        Signal::wait(&completion).await;
        let result = outcome.borrow_mut().take();
        // A terminal surface failure settles the capture — no retry on
        // this generation can produce the frame, and no redraw wake ever
        // lands: the loop exits here.
        if let Some(Err(cocoa_ui::capture::CaptureError::Failed(error))) = &result {
            return Err(error
                .downcast_ref::<cocoa_ui::capture::CompositionFailed>()
                .map_or_else(
                    || CaptureFailure::GpuSurfaceFailed(error.clone()),
                    |composition| CaptureFailure::CompositionFailed {
                        status: composition.status,
                        error: composition.error.clone(),
                    },
                ));
        }
        // A `true` completion from a context that lost or was superseded
        // mid-flight settles stale pixels — never the current target.
        if matches!(result, Some(Ok(())))
            && context.device_lost_reason().is_none()
            && runtime.context().generation() == context.generation()
        {
            break target;
        }
        // Deferred. If the context was replaced mid-attempt the loop's top
        // already re-resolves it; otherwise park on the next real signal —
        // a surfaces' redraw wake or the rebuilt context's publication,
        // whichever lands first (both forward into `redraw`).
        if runtime.context().generation() != context.generation() {
            continue;
        }
        let generation = context.generation();
        let forward = redraw.clone();
        let forward_runtime = runtime.clone();
        let _watch = executor_core::spawn_local(async move {
            forward_runtime.context_after(generation).await;
            Signal::fire(&forward);
        });
        Signal::wait(redraw).await;
    };
    Ok(target)
}

/// Where a capture failure meets a caller with no error channel.
///
/// `render_to_rgba` returns its typed [`RenderError`](waterui_core::view_renderer::RenderError)
/// instead; [`template_image`] and [`drag_image`] remain here because
/// `AppKit` and `UIKit` give their icon/drag paths no error channel at all —
/// so they fail fast with the cause rather than ship an empty image.
/// Never produces an empty raster: the typed failure aborts here, its
/// whole `source()` chain in the message.
#[cold]
pub fn terminal_capture_failure(error: &dyn std::error::Error) -> ! {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    panic!("view capture failed: {message}")
}

/// `view` rasterized as a template image, capped at `max_side` points —
/// the icon path's shape: the alpha channel is the image, chrome tints it.
///
/// Reuses [`capture_rgba`]; the `CGImage` conversion and `max_side` shrink
/// conventions match `cocoa_ui::bitmap::view_template_image`.
#[expect(
    clippy::future_not_send,
    reason = "the icon capture awaits the main-thread capture of a main-thread view"
)]
pub async fn template_image(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
    max_side: f64,
) -> Retained<PlatformImage> {
    let bounds = cocoa_ui::view::bounds(view);
    let captured = capture_rgba(view, env, view.mtm())
        .await
        .unwrap_or_else(|error| terminal_capture_failure(&error));
    if captured.width == 0 || captured.height == 0 {
        // A genuinely empty view bounds is the only empty-image contract —
        // never a masked conversion failure.
        return PlatformImage::new();
    }
    let image = cocoa_ui::bitmap::image_from_rgba(
        &captured.pixels,
        captured.width as usize,
        captured.height as usize,
    )
    .expect("CGImage creation from captured pixels failed");
    let shrink = (max_side / f64::max(bounds.size.width, bounds.size.height)).min(1.0);

    #[cfg(target_os = "macos")]
    {
        cocoa_ui::bitmap::template_image(
            &image,
            cocoa_ui::Size::new(bounds.size.width * shrink, bounds.size.height * shrink),
        )
    }
    #[cfg(target_os = "ios")]
    {
        cocoa_ui::bitmap::template_image(&image, captured.scale / shrink)
            .expect("UIImage template conversion failed")
    }
}

/// `view` rasterized as a non-template `NSImage` at its exact logical
/// bounds — the drag path's payload.
///
/// Reuses [`capture_rgba`] at the window's backing scale; the `CGImage`
/// wraps the top-down raster unflipped, preserving the capture
/// orientation.
#[cfg(all(target_os = "macos", feature = "draggable"))]
#[expect(
    clippy::future_not_send,
    reason = "the drag capture awaits the main-thread capture of a main-thread view"
)]
pub async fn drag_image(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
) -> Retained<cocoa_ui::objc2_app_kit::NSImage> {
    let bounds = cocoa_ui::view::bounds(view);
    let captured = capture_rgba(view, env, view.mtm())
        .await
        .unwrap_or_else(|error| terminal_capture_failure(&error));
    if captured.width == 0 || captured.height == 0 {
        return cocoa_ui::objc2_app_kit::NSImage::new();
    }
    let image = cocoa_ui::bitmap::image_from_rgba(
        &captured.pixels,
        captured.width as usize,
        captured.height as usize,
    )
    .expect("CGImage creation from captured pixels failed");
    cocoa_ui::objc2_app_kit::NSImage::initWithCGImage_size(
        cocoa_ui::objc2_app_kit::NSImage::alloc(),
        &image,
        CGSize::new(bounds.size.width, bounds.size.height),
    )
}
