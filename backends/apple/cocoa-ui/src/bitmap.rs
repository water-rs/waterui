//! Pixel capture helpers: RGBA8 bitmap contexts, `CGImage` construction, and
//! the offscreen window an offscreen render attaches its tree to.
//!
//! # Safety
//!
//! `CGBitmapContextCreate` has no `objc2` binding, so the one `extern`
//! declaration lives here, discharged next to the call. Everything else is
//! safe `objc2` API on the caller's own objects, all main-thread.

use objc2::rc::Retained;
#[cfg(target_os = "ios")]
use objc2::runtime::NSObjectProtocol;
use objc2::{AllocAnyThread, MainThreadOnly};
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapContextCreateImage, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider,
    CGImage, CGImageAlphaInfo,
};

use crate::geometry::{Rect, Size};

unsafe extern "C" {
    /// `CGBitmapContextCreate` — `objc2-core-graphics` has no binding for it.
    fn CGBitmapContextCreate(
        data: *mut core::ffi::c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: Option<&CGColorSpace>,
        bitmap_info: u32,
    ) -> *mut CGContext;
}

const BITS_PER_COMPONENT: usize = 8;
const BITS_PER_PIXEL: usize = 32;
const BYTES_PER_PIXEL: usize = 4;

/// A `CGContext` drawing premultiplied RGBA8 into `pixels`' storage.
///
/// The context holds a raw pointer into `pixels`' buffer, so callers must not
/// reallocate the buffer until the context is dropped.
///
/// # Panics
///
/// When `pixels` is not exactly `width*height*4` bytes.
#[must_use]
pub fn bitmap_context(
    pixels: &mut [u8],
    width: usize,
    height: usize,
) -> Option<CFRetained<CGContext>> {
    assert_eq!(
        pixels.len(),
        width * height * BYTES_PER_PIXEL,
        "a bitmap context needs exactly width*height*4 bytes"
    );
    // SAFETY: `pixels` is a live `width*height*4` buffer kept alive by the
    // caller for the context's use; the RGB colorspace is a fresh system
    // object.
    let ptr = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            BITS_PER_COMPONENT,
            width * BYTES_PER_PIXEL,
            CGColorSpace::new_device_rgb().as_deref(),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    };
    // SAFETY: CoreGraphics returns a +1 context or null.
    Some(unsafe { CFRetained::from_raw(core::ptr::NonNull::new(ptr)?) })
}

/// A `CGImage` holding a copy of `pixels`' premultiplied RGBA8 content.
///
/// # Panics
///
/// When `pixels` is not exactly `width*height*4` bytes.
#[must_use]
pub fn image_from_rgba(pixels: &[u8], width: usize, height: usize) -> Option<CFRetained<CGImage>> {
    assert_eq!(
        pixels.len(),
        width * height * BYTES_PER_PIXEL,
        "an RGBA8 image needs exactly width*height*4 bytes"
    );
    let data = CFData::from_bytes(pixels);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    // SAFETY: `decode` is null; every other argument describes the
    // premultiplied RGBA8 layout `pixels` carries.
    unsafe {
        CGImage::new(
            width,
            height,
            BITS_PER_COMPONENT,
            BITS_PER_PIXEL,
            width * BYTES_PER_PIXEL,
            CGColorSpace::new_device_rgb().as_deref(),
            objc2_core_graphics::CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
            Some(&provider),
            core::ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

/// Renders `layer`'s committed tree into `context`.
pub fn render_layer(layer: &objc2_quartz_core::CALayer, context: &CGContext) {
    layer.renderInContext(context);
}

/// Draws `image` covering `rect` inside `context`.
pub fn draw_image(context: &CGContext, image: &CGImage, rect: Rect) {
    CGContext::draw_image(Some(context), rect.into(), Some(image));
}

/// The image `context` captured — `CGBitmapContextCreateImage`.
#[must_use]
pub fn context_image(context: &CGContext) -> Option<CFRetained<CGImage>> {
    CGBitmapContextCreateImage(Some(context))
}

/// Scales `context` so one user-space unit is `scale` device pixels.
pub fn scale_to_pixels(context: &CGContext, scale: f64) {
    CGContext::scale_ctm(Some(context), scale, scale);
}

/// Pushes `context`'s flipped layer-render transform — `UIKit`'s layer tree
/// renders top-down into a bottom-up CG coordinate space.
#[cfg(target_os = "ios")]
pub fn begin_layer_flip(context: &CGContext, height: f64) {
    CGContext::save_g_state(Some(context));
    CGContext::translate_ctm(Some(context), 0.0, height);
    CGContext::scale_ctm(Some(context), 1.0, -1.0);
}

/// Pops the transform [`begin_layer_flip`] pushed.
#[cfg(target_os = "ios")]
pub fn end_layer_flip(context: &CGContext) {
    CGContext::restore_g_state(Some(context));
}

/// Fills `context`'s whole `rect` with `color`.
pub fn fill(context: &CGContext, color: &objc2_core_graphics::CGColor, rect: Rect) {
    CGContext::set_fill_color_with_color(Some(context), Some(color));
    CGContext::fill_rect(Some(context), rect.into());
}

/// Pushes `context` as `UIKit`'s current graphics context for the layer
/// render, then pops it.
#[cfg(target_os = "ios")]
pub fn with_uikit_context(context: &CGContext, body: impl FnOnce()) {
    // SAFETY: pushes/pops are balanced around `body`.
    unsafe { objc2_ui_kit::UIGraphicsPushContext(context) };
    body();
    // SAFETY: pops the context pushed above.
    unsafe { objc2_ui_kit::UIGraphicsPopContext() };
}

/// A `UIImage` rendering `image` at `scale`, tagged `alwaysTemplate`.
#[cfg(target_os = "ios")]
#[must_use]
pub fn template_image(image: &CGImage, scale: f64) -> Option<Retained<objc2_ui_kit::UIImage>> {
    use objc2_ui_kit::{UIImage, UIImageOrientation, UIImageRenderingMode};
    let ui = UIImage::initWithCGImage_scale_orientation(
        UIImage::alloc(),
        image,
        scale,
        UIImageOrientation::Up,
    );
    Some(ui.imageWithRenderingMode(UIImageRenderingMode::AlwaysTemplate))
}

/// An `NSImage` wrapping `image` at `size` points, tagged as a template.
#[cfg(target_os = "macos")]
#[must_use]
pub fn template_image(image: &CGImage, size: Size) -> Retained<objc2_app_kit::NSImage> {
    let ns = objc2_app_kit::NSImage::initWithCGImage_size(
        objc2_app_kit::NSImage::alloc(),
        image,
        size.into(),
    );
    ns.setTemplate(true);
    ns
}

/// A borderless offscreen `NSWindow` at `(-10_000, -10_000)` for headless
/// capture — it never appears, but attaching a view to it drives `AppKit`'s
/// window-dependent rendering paths.
#[cfg(target_os = "macos")]
#[must_use]
/// # Panics
///
/// On a failure to allocate the window.
pub fn make_offscreen_window(
    mtm: objc2::MainThreadMarker,
    size: Size,
) -> Retained<objc2_app_kit::NSWindow> {
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    // SAFETY: `initWithContentRect:styleMask:backing:defer:` is `NSWindow`'s
    // designated initializer; `setReleasedWhenClosed:false` keeps the object
    // owned by the `Retained` the caller drops.
    unsafe {
        let window: Retained<NSWindow> = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            Rect::new(-10_000.0, -10_000.0, size.width, size.height).into(),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window
    }
}

/// An offscreen `UIWindow` in `scene` — hidden off-display but attached so
/// `UIView` captures see a real window. Its safe-area insets are zeroed:
/// capture windows never sit under a device notch.
#[cfg(target_os = "ios")]
#[must_use]
/// # Panics
///
/// On a failure to allocate the window.
pub fn make_offscreen_window(
    mtm: objc2::MainThreadMarker,
    scene: &objc2_ui_kit::UIWindowScene,
    size: Size,
) -> Retained<CaptureWindow> {
    let window: Retained<CaptureWindow> =
        // SAFETY: `initWithWindowScene:` is `UIWindow`'s designated
        // initializer for scene-based windows.
        unsafe { objc2::msg_send![CaptureWindow::alloc(mtm), initWithWindowScene: scene] };
    window.setFrame(Rect::new(-10_000.0, -10_000.0, size.width, size.height).into());
    window
}

#[cfg(target_os = "ios")]
objc2::define_class!(
    // SAFETY: `UIWindow` asks a subclass to initialize through a designated
    // initializer, which `make_offscreen_window` does, and the class holds no
    // ivars and implements no `Drop`.
    #[unsafe(super(objc2_ui_kit::UIWindow))]
    #[name = "CocoaUiCaptureWindow"]
    #[thread_kind = MainThreadOnly]
    /// A capture window reporting zero safe-area insets: it never appears on
    /// a display, so no device notch or home-indicator inset applies.
    pub struct CaptureWindow;

    unsafe impl NSObjectProtocol for CaptureWindow {}

    impl CaptureWindow {
        /// Zero insets — the window is never on a device.
        #[unsafe(method(safeAreaInsets))]
        fn safe_area_insets(&self) -> objc2_ui_kit::UIEdgeInsets {
            objc2_ui_kit::UIEdgeInsets {
                top: 0.0,
                left: 0.0,
                bottom: 0.0,
                right: 0.0,
            }
        }
    }
);

/// Recursively forces every `NSTextField` inside `view` to draw — `AppKit`
/// leaves text-field contents out of `cacheDisplay` unless they are marked
/// dirty first.
#[cfg(target_os = "macos")]
pub fn force_text_fields_display(view: &crate::PlatformView) {
    use objc2_app_kit::NSTextField;
    if let Ok(text) = objc2::rc::Retained::downcast::<NSTextField>(crate::view::retain_base(view)) {
        text.setNeedsDisplayInRect(text.bounds());
        text.displayIfNeeded();
    }
    for subview in crate::view::subviews(view) {
        force_text_fields_display(&subview);
    }
}

/// Sends a macOS capture window front without activating the app — the
/// ordering `orderFrontRegardless` implies.
#[cfg(target_os = "macos")]
pub fn show_capture_window(window: &objc2_app_kit::NSWindow) {
    window.orderFrontRegardless();
    window.display();
}

/// Closes a capture window after the bitmap lands.
#[cfg(target_os = "macos")]
pub fn close_capture_window(window: &objc2_app_kit::NSWindow) {
    window.orderOut(None);
}

/// Mounts `view` in `window` through a plain `UIViewController`, unhides the
/// window and lays everything out — the `UIKit` analogue of
/// `orderFrontRegardless`.
#[cfg(target_os = "ios")]
/// # Panics
///
/// On a failure to confirm the main thread.
pub fn show_capture_window(window: &CaptureWindow, view: &crate::PlatformView, size: Size) {
    let mtm = objc2::MainThreadMarker::new().expect("main thread");
    let controller = objc2_ui_kit::UIViewController::new(mtm);
    window.setRootViewController(Some(&controller));
    if let Some(content) = controller.view() {
        content.setFrame(Rect::new(0.0, 0.0, size.width, size.height).into());
        crate::view::add_subview(&content, view);
    }
    window.setHidden(false);
    window.layoutIfNeeded();
    view.layoutIfNeeded();
}

/// Hides a `UIKit` capture window after the bitmap lands.
#[cfg(target_os = "ios")]
pub fn close_capture_window(window: &CaptureWindow) {
    window.setHidden(true);
}

/// `view` rasterized as a template `NSImage`, capped at `max_side` points —
/// the toolbar-icon shape of chrome-presented views.
///
/// The alpha channel is the image, so the toolbar tints it like its own items
/// instead of showing the view's accent-tinted pixels. `cacheDisplay` handles
/// views without a backing layer; GPU-surface content (Metal layers) does
/// not render through it.
#[cfg(target_os = "macos")]
#[must_use]
pub fn view_template_image(
    view: &crate::PlatformView,
    max_side: f64,
) -> Option<Retained<objc2_app_kit::NSImage>> {
    let bounds = view.bounds();
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    let rep = view.bitmapImageRepForCachingDisplayInRect(bounds)?;
    force_text_fields_display(view);
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let image = rep.CGImage()?;
    let shrink = (max_side / f64::max(bounds.size.width, bounds.size.height)).min(1.0);
    Some(template_image(
        &image,
        Size::new(bounds.size.width * shrink, bounds.size.height * shrink),
    ))
}

/// `view` rasterized as a template `UIImage`, capped at `max_side` points —
/// the `UIKit` half of the macOS helper.
///
/// Bar items draw a declared icon as an image inside the bar's chrome rather
/// than hosting the accent-tinted view. GPU-surface content does not render
/// through `renderInContext:`.
#[cfg(target_os = "ios")]
#[must_use]
pub fn view_template_image(
    view: &crate::PlatformView,
    max_side: f64,
) -> Option<Retained<objc2_ui_kit::UIImage>> {
    let bounds = view.bounds();
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    let scale = {
        use objc2_ui_kit::UITraitEnvironment;
        // SAFETY: an ordinary main-thread trait-environment read.
        let scale = unsafe { view.traitCollection().displayScale() };
        if scale > 0.0 { scale } else { 2.0 }
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a view's pixel dimensions fit usize and are positive"
    )]
    let width = (bounds.size.width * scale).ceil() as usize;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a view's pixel dimensions fit usize and are positive"
    )]
    let height = (bounds.size.height * scale).ceil() as usize;
    let mut pixels = vec![0u8; width * height * BYTES_PER_PIXEL];
    let context = bitmap_context(&mut pixels, width, height)?;
    scale_to_pixels(&context, scale);
    with_uikit_context(&context, || {
        begin_layer_flip(&context, bounds.size.height);
        render_layer(&view.layer(), &context);
        end_layer_flip(&context);
    });
    let image = context_image(&context)?;
    let shrink = (max_side / f64::max(bounds.size.width, bounds.size.height)).min(1.0);
    template_image(&image, scale / shrink)
}

/// Any connected `UIWindowScene`:
/// capture windows must attach to a real scene, and any connected one works.
#[cfg(target_os = "ios")]
#[must_use]
/// # Panics
///
/// On a failure to enumerate the application's connected scenes.
pub fn any_window_scene() -> Option<Retained<objc2_ui_kit::UIWindowScene>> {
    use objc2_ui_kit::UIApplication;
    let mtm = objc2::MainThreadMarker::new().expect("main thread");
    let application = UIApplication::sharedApplication(mtm);
    let scenes = application.connectedScenes();
    let iterator = scenes.iter();
    for scene in iterator {
        if let Ok(window_scene) = scene.downcast::<objc2_ui_kit::UIWindowScene>() {
            return Some(window_scene);
        }
    }
    None
}
