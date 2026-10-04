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

#[cfg(target_os = "macos")]
use cocoa_ui::objc2::AnyThread;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::ProtocolObject;
#[cfg(target_os = "macos")]
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
type PlatformImage = cocoa_ui::objc2_app_kit::NSImage;
/// See the macOS alias.
#[cfg(target_os = "ios")]
type PlatformImage = cocoa_ui::objc2_ui_kit::UIImage;

/// One owned RGBA8 premultiplied raster — the single readback the API
/// contract allows.
pub struct CapturedRgba {
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
    #[allow(clippy::future_not_send)]
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
    /// The temporary window — kept alive for the capture's duration; the
    /// struct's drop hides it and restores the view before the window dies.
    #[cfg(target_os = "macos")]
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
    /// render their chrome.
    fn attach_if_windowless(view: &Retained<cocoa_ui::PlatformView>) -> Option<Self> {
        if cocoa_ui::view::window(view).is_some() {
            return None;
        }
        let mtm = cocoa_ui::MainThreadMarker::new().expect("capture runs on the main thread");
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
            let window = cocoa_ui::bitmap::make_offscreen_window(mtm, size);
            let content = window.contentView().expect("offscreen window content");
            cocoa_ui::view::add_subview(&content, view);
            cocoa_ui::bitmap::show_capture_window(&window);
            // Text fields stay out of a capture unless marked dirty — the
            // existing bitmap path's preparation.
            cocoa_ui::bitmap::force_text_fields_display(view);
            window
        };
        #[cfg(target_os = "ios")]
        let window = {
            let scene = cocoa_ui::bitmap::any_window_scene()
                .expect("a detached capture needs a connected UIWindowScene");
            let window = cocoa_ui::bitmap::make_offscreen_window(mtm, &scene, size);
            // The `UIViewController` containment + unhide + layout is the
            // only correct UIKit mount — a bare `addSubview` leaves the
            // window without a root controller.
            cocoa_ui::bitmap::show_capture_window(&window, view, size);
            window
        };
        Some(Self {
            window,
            superview,
            #[cfg(target_os = "macos")]
            next_sibling,
            #[cfg(target_os = "ios")]
            index,
            frame,
            view: view.clone(),
        })
    }
}

impl Drop for WindowHost {
    /// Restores the view to its original parent's exact child position and
    /// frame — appending would silently reorder the host's z-order, and a
    /// resized frame would linger.
    fn drop(&mut self) {
        // The capture window hides first — a re-hosted view must never be
        // left visible inside it — then the view returns to its parent.
        #[cfg(target_os = "macos")]
        cocoa_ui::bitmap::close_capture_window(&self.window);
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
/// The compositor's native raster pass (`CALayer.renderInContext` into
/// shared-buffer memory) and output pass (`CapturableSurface` external
/// rendering) write the texture; one
/// `getBytes` readback follows. A deferred capture — a GPU surface that
/// had no current content, or a context lost mid-capture — is honored by
/// waiting on the surfaces' own redraw wake and recapturing; the future
/// only completes once a frame really landed, so no bogus-ready blank
/// ships.
#[allow(clippy::future_not_send)]
#[expect(
    clippy::too_many_lines,
    reason = "the deferred-retry loop, generation parking and the single readback stay in one pass"
)]
pub async fn capture_rgba(
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

    let registry = CaptureRegistry::get(env);
    let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(
        mtm,
        view.clone(),
        registry.resolver(),
    ));
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
        let target = rgba_target(&device, width, height);

        let completion = Rc::new(RefCell::new(Signal::default()));
        let landed = Rc::new(RefCell::new(false));
        // The completion is `Send` — it parks on Metal's own queue before
        // hopping to the main queue — so the slots ride `MainThreadBound`.
        let slot = dispatch2::MainThreadBound::new((completion.clone(), landed.clone()), mtm);
        guard
            .capture()
            .capture(&target, context.generation(), move |ok| {
                let mtm = cocoa_ui::MainThreadMarker::new()
                    .expect("the capture completion runs on the main thread");
                let (completion, landed) = slot.get(mtm);
                *landed.borrow_mut() = ok;
                Signal::fire(completion);
            });
        Signal::wait(&completion).await;
        // A `true` completion from a context that lost or was superseded
        // mid-flight settles stale pixels — never the current target.
        if *landed.borrow()
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
        Signal::wait(&redraw).await;
    };
    drop(guard);
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
/// Reuses [`capture_rgba`]; the `CGImage` conversion and `max_side` shrink
/// conventions match `cocoa_ui::bitmap::view_template_image`.
#[allow(clippy::future_not_send)]
pub async fn template_image(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
    max_side: f64,
) -> Retained<PlatformImage> {
    let bounds = cocoa_ui::view::bounds(view);
    let scale = cocoa_ui::view::backing_scale_factor(view);
    let captured = capture_rgba(view, env, scale.max(1.0)).await;
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
        cocoa_ui::bitmap::template_image(&image, scale / shrink)
            .expect("UIImage template conversion failed")
    }
}

/// `view` rasterized as a non-template `NSImage` at its exact logical
/// bounds — the drag path's payload.
///
/// Reuses [`capture_rgba`] at the window's backing scale; the `CGImage`
/// wraps the top-down raster unflipped, preserving the capture
/// orientation.
#[cfg(target_os = "macos")]
#[allow(clippy::future_not_send)]
pub async fn drag_image(
    view: &cocoa_ui::PlatformView,
    env: &Environment,
) -> Retained<cocoa_ui::objc2_app_kit::NSImage> {
    let bounds = cocoa_ui::view::bounds(view);
    let scale = cocoa_ui::view::backing_scale_factor(view);
    let captured = capture_rgba(view, env, scale.max(1.0)).await;
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
