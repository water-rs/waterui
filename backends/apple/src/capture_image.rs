//! Owned snapshot rendering for the GPU-bearing icon/drag/cache consumers
//! (#1683): the same native-plus-output compositor `ViewCapture` runs for
//! effect views, pointed at an owned Metal texture, with one final RGBA8
//! readback at the end.
//!
//! A capture through here is a deliberate render destination of the same
//! producer — never a CPU screenshot path, a duplicated producer, or an
//! onscreen presentation. External surfaces inside the subtree take part
//! through their `CapturableSurface` contract, so a GPU surface or a
//! nested filter contributes real pixels rather than a blank.

use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use cocoa_ui::objc2::AnyThread;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::ProtocolObject;
use cocoa_ui::objc2_core_foundation::CGSize;
use cocoa_ui::objc2_metal::{
    MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use waterui_backend_core::Environment;

use crate::capture_registry::CaptureRegistry;

/// The platform's native image object — `NSImage` on macOS, `UIImage` on
/// iOS — the type icon and drag endpoints publish.
#[cfg(target_os = "macos")]
pub(crate) type PlatformImage = cocoa_ui::objc2_app_kit::NSImage;
/// See the macOS alias.
#[cfg(target_os = "ios")]
pub(crate) type PlatformImage = cocoa_ui::objc2_ui_kit::UIImage;

/// One owned RGBA8 premultiplied raster — the single readback the API
/// contract allows.
pub(crate) struct CapturedRgba {
    /// Row-major RGBA8 premultiplied texels, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
    /// Raster width in pixels.
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
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
struct WindowHost {
    /// The temporary window — kept alive for the capture's duration.
    #[cfg(target_os = "macos")]
    window: Retained<cocoa_ui::objc2_app_kit::NSWindow>,
    /// See the macOS field.
    #[cfg(target_os = "ios")]
    window: Retained<cocoa_ui::bitmap::CaptureWindow>,
    /// The view's parent before the re-host, restored on drop.
    superview: Option<Retained<cocoa_ui::PlatformView>>,
    /// The view being hosted.
    view: Retained<cocoa_ui::PlatformView>,
}

impl WindowHost {
    /// Re-hosts `view` iff it has no window — native controls need one to
    /// render their chrome.
    fn attach_if_windowless(view: &Retained<cocoa_ui::PlatformView>) -> Option<Self> {
        if cocoa_ui::view::window(view).is_some() {
            return None;
        }
        let mtm = cocoa_ui::MainThreadMarker::new().expect("capture runs on the main thread");
        let bounds = cocoa_ui::view::bounds(view);
        let size = cocoa_ui::Size::new(bounds.size.width.max(1.0), bounds.size.height.max(1.0));
        let superview = cocoa_ui::view::superview(view);
        #[cfg(target_os = "macos")]
        let window = {
            let window = cocoa_ui::bitmap::make_offscreen_window(mtm, size);
            let content = window.contentView().expect("offscreen window content");
            cocoa_ui::view::add_subview(&content, view);
            window
        };
        #[cfg(target_os = "ios")]
        let window = {
            let scene = cocoa_ui::bitmap::any_window_scene()
                .expect("a detached capture needs a connected UIWindowScene");
            let window = cocoa_ui::bitmap::make_offscreen_window(mtm, &scene, size);
            cocoa_ui::view::add_subview(&window, view);
            window
        };
        Some(Self {
            window,
            superview,
            view: view.clone(),
        })
    }
}

impl Drop for WindowHost {
    fn drop(&mut self) {
        cocoa_ui::view::remove_from_superview(&self.view);
        if let Some(superview) = &self.superview {
            cocoa_ui::view::add_subview(superview, &self.view);
        }
    }
}

/// The shared `RGBA8Unorm` readback target — `.shared` storage so the one
/// `getBytes` the API permits can run without a blit.
fn rgba_target(
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
    descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
    descriptor.setStorageMode(MTLStorageMode::Shared);
    device
        .newTextureWithDescriptor(&descriptor)
        .expect("capture target texture")
}

/// `view`'s subtree rendered into an owned `RGBA8` raster at `scale`
/// pixels per point — `ceil(bounds * scale)` physical pixels.
///
/// The compositor's native pass (`CARenderer`) and output pass
/// (`CapturableSurface` external rendering) write the texture; one
/// `getBytes` readback follows. A deferred capture — a GPU surface that
/// had no current content, or a context lost mid-capture — is honored by
/// waiting on the surfaces' own redraw wake and recapturing; the future
/// only completes once a frame really landed, so no bogus-ready blank
/// ships.
#[allow(clippy::future_not_send)]
pub(crate) async fn capture_rgba(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
    scale: f64,
) -> CapturedRgba {
    let mtm = cocoa_ui::MainThreadMarker::new().expect("capture runs on the main thread");
    let view = cocoa_ui::view::retain_base(view);
    let host = WindowHost::attach_if_windowless(&view);
    if host.is_some() {
        // A re-hosted view lays out against the offscreen window before the
        // capture reads its actual bounds.
        cocoa_ui::view::layout_immediately(&view);
    }

    let bounds = cocoa_ui::view::bounds(&view);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return CapturedRgba {
            pixels: Vec::new(),
            width: 0,
            height: 0,
        };
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a view's pixel dimensions fit u32 and are positive"
    )]
    let (width, height) = (
        (bounds.size.width * scale).ceil() as u32,
        (bounds.size.height * scale).ceil() as u32,
    );

    let context = crate::gpu_runtime::runtime(env).context();
    let device = crate::gpu_runtime::raw_metal_device(&context);
    let target = rgba_target(&device, width, height);

    let registry = CaptureRegistry::get(env);
    let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(
        mtm,
        view.clone(),
        registry.resolver(),
    ));

    // The redraw contract: an external surface inside the subtree asks the
    // capture for a new frame through this hook — deferred captures retry
    // when it fires.
    let redraw = Rc::new(RefCell::new(Signal::default()));
    capture.set_on_redraw({
        let redraw = redraw.clone();
        move || Signal::fire(&redraw)
    });

    loop {
        let completion = Rc::new(RefCell::new(Signal::default()));
        let landed = Rc::new(RefCell::new(false));
        // The completion is `Send` — it parks on Metal's own queue before
        // hopping to the main queue — so the slots ride `MainThreadBound`.
        let slot = dispatch2::MainThreadBound::new((completion.clone(), landed.clone()), mtm);
        capture.capture(&target, move |ok| {
            let mtm = cocoa_ui::MainThreadMarker::new()
                .expect("the capture completion runs on the main thread");
            let (completion, landed) = slot.get(mtm);
            *landed.borrow_mut() = ok;
            Signal::fire(completion);
        });
        Signal::wait(&completion).await;
        if *landed.borrow() {
            break;
        }
        // Deferred: wait for the surfaces' redraw wake, then recapture.
        Signal::wait(&redraw).await;
    }
    capture.shutdown();
    drop(host);

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
    }
}

/// `view` rasterized as a template image, capped at `max_side` points —
/// the icon path's shape: the alpha channel is the image, chrome tints it.
///
/// Reuses [`capture_rgba`]; the CGImage conversion and `max_side` shrink
/// conventions match `cocoa_ui::bitmap::view_template_image`.
#[allow(clippy::future_not_send)]
pub(crate) async fn template_image(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
    max_side: f64,
) -> Retained<PlatformImage> {
    let bounds = cocoa_ui::view::bounds(view);
    let scale = cocoa_ui::view::backing_scale_factor(view);
    let captured = capture_rgba(view, env, scale.max(1.0)).await;
    let shrink = (max_side / f64::max(bounds.size.width, bounds.size.height)).min(1.0);

    #[cfg(target_os = "macos")]
    {
        let Some(image) = cocoa_ui::bitmap::image_from_rgba(
            &captured.pixels,
            captured.width as usize,
            captured.height as usize,
        ) else {
            return PlatformImage::new();
        };
        cocoa_ui::bitmap::template_image(
            &image,
            cocoa_ui::Size::new(bounds.size.width * shrink, bounds.size.height * shrink),
        )
    }
    #[cfg(target_os = "ios")]
    {
        let Some(image) = cocoa_ui::bitmap::image_from_rgba(
            &captured.pixels,
            captured.width as usize,
            captured.height as usize,
        ) else {
            return PlatformImage::new();
        };
        cocoa_ui::bitmap::template_image(&image, scale / shrink).unwrap_or_else(PlatformImage::new)
    }
}

/// `view` rasterized as a non-template `NSImage` at its exact logical
/// bounds — the drag path's payload.
///
/// Reuses [`capture_rgba`] at the window's backing scale; the CGImage
/// wraps the top-down raster unflipped, preserving the capture
/// orientation.
#[cfg(target_os = "macos")]
#[allow(clippy::future_not_send)]
pub(crate) async fn drag_image(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
) -> Retained<cocoa_ui::objc2_app_kit::NSImage> {
    let bounds = cocoa_ui::view::bounds(view);
    let scale = cocoa_ui::view::backing_scale_factor(view);
    let captured = capture_rgba(view, env, scale.max(1.0)).await;
    let Some(image) = cocoa_ui::bitmap::image_from_rgba(
        &captured.pixels,
        captured.width as usize,
        captured.height as usize,
    ) else {
        return cocoa_ui::objc2_app_kit::NSImage::new();
    };
    cocoa_ui::objc2_app_kit::NSImage::initWithCGImage_size(
        cocoa_ui::objc2_app_kit::NSImage::alloc(),
        &image,
        CGSize::new(bounds.size.width, bounds.size.height),
    )
}
