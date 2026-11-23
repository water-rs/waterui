//! Pixel capture helpers: RGBA8 bitmap contexts, `CGImage` construction, and
//! the offscreen window an offscreen render attaches its tree to.
//!
//! # Safety
//!
//! `CGBitmapContextCreate` has no `objc2` binding, so the one `extern`
//! declaration lives here, discharged next to the call. Everything else is
//! safe `objc2` API on the caller's own objects, all main-thread.

use filtrate_core::space::{SRGB_TO_P3, from_srgb, srgb_decode, srgb_encode, transform};
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyClass;
use objc2::runtime::NSObjectProtocol;
#[cfg(target_os = "macos")]
use objc2::{
    AllocAnyThread, MainThreadMarker, MainThreadOnly, Message as _, define_class, msg_send,
};
#[cfg(target_os = "ios")]
use objc2::{AllocAnyThread, MainThreadOnly};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
#[cfg(target_os = "macos")]
use objc2_core_foundation::CGRect;
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapContextCreateImage, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider,
    CGImage, CGImageAlphaInfo,
};
use objc2_quartz_core::CALayer;
#[cfg(target_os = "macos")]
use objc2_quartz_core::CATransaction;

#[cfg(target_os = "ios")]
use objc2_ui_kit::UIWindow;
use waterui_contract::contract_violation;

use crate::ColorScheme;
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

/// One owned top-down extended linear Display P3 `f32` RGBA raster.
///
/// Channels hold unclamped linear light in Display P3 primaries where `1.0`
/// corresponds to SDR reference white and values above `1.0` represent EDR
/// (HDR) headroom.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrRaster {
    /// Row-major premultiplied linear Display P3 `f32` RGBA pixels
    /// (`width * height * 4` floats).
    pub pixels: Vec<f32>,
    /// Raster width in pixels.
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
}

/// Source encoding of premultiplied extended-range RGBA `f32` pixels before
/// normalization into [`HdrRaster`]'s extended linear Display P3 working space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedColorSpace {
    /// Already premultiplied extended linear Display P3.
    ExtendedLinearDisplayP3,
    /// Premultiplied extended linear sRGB (Rec. 709 primaries, D65).
    ExtendedLinearSrgb,
    /// Premultiplied extended gamma-encoded Display P3.
    ExtendedDisplayP3,
    /// Premultiplied extended gamma-encoded sRGB.
    ExtendedSrgb,
}

impl HdrRaster {
    /// Normalizes premultiplied extended-range RGBA `f32` pixels from `space`
    /// into an [`HdrRaster`] in extended linear Display P3, reusing
    /// `filtrate_core::space`'s canonical transfer and chromaticity transforms.
    #[must_use]
    pub fn from_extended_rgba(
        mut pixels: Vec<[f32; 4]>,
        width: u32,
        height: u32,
        space: ExtendedColorSpace,
    ) -> Self {
        for pixel in &mut pixels {
            let alpha = if pixel[3].is_finite() {
                pixel[3].clamp(0.0, 1.0)
            } else {
                0.0
            };
            if alpha <= 0.0 {
                *pixel = [0.0; 4];
            } else {
                for channel in &mut pixel[..3] {
                    if !channel.is_finite() {
                        *channel = 0.0;
                    }
                }
                pixel[3] = alpha;
            }
        }
        match space {
            ExtendedColorSpace::ExtendedLinearDisplayP3 => {}
            ExtendedColorSpace::ExtendedSrgb => from_srgb(&mut pixels),
            ExtendedColorSpace::ExtendedLinearSrgb => {
                for pixel in &mut pixels {
                    let alpha = pixel[3];
                    if alpha > 0.0 {
                        let [r, g, b] = transform(
                            &SRGB_TO_P3,
                            [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha],
                        );
                        *pixel = [r * alpha, g * alpha, b * alpha, alpha];
                    }
                }
            }
            ExtendedColorSpace::ExtendedDisplayP3 => {
                for pixel in &mut pixels {
                    let alpha = pixel[3];
                    if alpha > 0.0 {
                        let [r, g, b] =
                            [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha].map(srgb_decode);
                        *pixel = [r * alpha, g * alpha, b * alpha, alpha];
                    }
                }
            }
        }
        let mut flat = Vec::with_capacity(pixels.len() * 4);
        for pixel in pixels {
            flat.extend_from_slice(&pixel);
        }
        Self {
            pixels: flat,
            width,
            height,
        }
    }

    /// Maximum finite RGB channel value across all pixels (`0.0` when empty).
    #[must_use]
    pub fn max_channel(&self) -> f32 {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [px[0], px[1], px[2]])
            .filter(|v| v.is_finite())
            .fold(0.0_f32, f32::max)
    }

    /// Peak EDR headroom observed in this raster relative to SDR white
    /// (`>= 1.0`).
    #[must_use]
    pub fn max_edr_headroom(&self) -> f32 {
        self.max_channel().max(1.0)
    }

    /// Encodes this premultiplied extended linear Display P3 raster into
    /// premultiplied Display P3 RGBA8 bytes (`width * height * 4`), clamping
    /// extended channels to the SDR Display P3 gamut `[0.0, 1.0]` before
    /// applying the Display P3 transfer curve.
    #[must_use]
    pub fn to_display_p3_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len());
        for pixel in self.pixels.as_chunks::<4>().0 {
            out.extend_from_slice(&linear_p3_to_display_p3_u8(*pixel));
        }
        out
    }
}

/// Encodes one premultiplied extended linear Display P3 `f32` pixel into
/// premultiplied Display P3 RGBA8, clamping straight channels to `[0.0, 1.0]`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels are clamped to [0.0, 1.0] before scaling by 255.0 and rounding"
)]
fn linear_p3_to_display_p3_u8(pixel: [f32; 4]) -> [u8; 4] {
    let alpha = if pixel[3].is_finite() {
        pixel[3].clamp(0.0, 1.0)
    } else {
        0.0
    };
    if alpha == 0.0 {
        return [0, 0, 0, 0];
    }
    let encode = |channel: f32| -> u8 {
        let straight = if channel.is_finite() {
            (channel / alpha).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let encoded = srgb_encode(straight) * alpha;
        (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    [
        encode(pixel[0]),
        encode(pixel[1]),
        encode(pixel[2]),
        (alpha * 255.0).round() as u8,
    ]
}

/// A `CGContext` drawing premultiplied RGBA8 into `pixels`' storage.
///
/// The context holds a raw pointer into `pixels`' buffer, so callers must not
/// reallocate the buffer until the context is dropped.
///
/// `None` when the system cannot make the context, and also when `pixels` is
/// not exactly `width*height*4` bytes, which breaks the contract and is
/// reported.
#[must_use]
pub fn bitmap_context(
    pixels: &mut [u8],
    width: usize,
    height: usize,
) -> Option<CFRetained<CGContext>> {
    if pixels.len() != width * height * BYTES_PER_PIXEL {
        contract_violation!(
            "a bitmap context needs exactly width*height*4 bytes: {width}x{height} over {} bytes; no context is made",
            pixels.len()
        );
        return None;
    }
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
/// `None` when the system cannot make the image, and also when `pixels` is
/// not exactly `width*height*4` bytes, which breaks the contract and is
/// reported.
#[must_use]
pub fn image_from_rgba(pixels: &[u8], width: usize, height: usize) -> Option<CFRetained<CGImage>> {
    if pixels.len() != width * height * BYTES_PER_PIXEL {
        contract_violation!(
            "an RGBA8 image needs exactly width*height*4 bytes: {width}x{height} over {} bytes; no image is made",
            pixels.len()
        );
        return None;
    }
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
pub fn render_layer(layer: &CALayer, context: &CGContext) {
    #[cfg(target_os = "macos")]
    without_sdf_layers(layer, || {
        layer.renderInContext(context);
    });
    #[cfg(target_os = "ios")]
    layer.renderInContext(context);
}

/// Temporarily hides visible `CASDFLayer` descendants under `layer` for the
/// duration of `body`, restoring their `isHidden` state before returning.
///
/// On macOS 27, `AppKit`'s `NSBezelStyle::Glass` and tinted `NSGlassEffectView`
/// mount `CASDFLayer` sublayers that only serialize to the out-of-process
/// `WindowServer` compositor (`_copyRenderLayer:layerFlags:commitFlags:`) and
/// produce zero alpha in in-process `CALayer.renderInContext:` or
/// `cacheDisplayInRect:toBitmapImageRep:`. When a glass control carries a
/// tint/bezel color inside an active window, `AppKit` pairs its tint
/// `CASDFLayer` with a sibling `CASDFLayer` whose `compositingFilter` is
/// `"destIn"` (`kCGBlendModeDestinationIn`) on a `0x0`-bounds parent
/// `CALayer`; in `renderInContext:`, that empty `destIn` layer multiplies
/// the entire destination `CGContext` (including any already-drawn ancestor
/// background) by `0`. Hiding `CASDFLayer` sublayers while the in-process
/// raster runs prevents the unimplemented `WindowServer` mask from clobbering
/// the destination buffer.
#[cfg(target_os = "macos")]
pub(crate) fn without_sdf_layers<R>(layer: &CALayer, body: impl FnOnce() -> R) -> R {
    struct Restore(Vec<Retained<CALayer>>);

    impl Drop for Restore {
        fn drop(&mut self) {
            for layer in &self.0 {
                layer.setHidden(false);
            }
            CATransaction::commit();
        }
    }

    CATransaction::begin();
    CATransaction::setDisableActions(true);
    let mut restore = Restore(Vec::new());
    if let Some(sdf_class) = AnyClass::get(c"CASDFLayer") {
        collect_and_hide_sdf_layers(layer, sdf_class, &mut restore.0);
    }
    body()
}

#[cfg(target_os = "macos")]
fn collect_and_hide_sdf_layers(
    layer: &CALayer,
    sdf_class: &AnyClass,
    hidden: &mut Vec<Retained<CALayer>>,
) {
    if layer.isHidden() {
        return;
    }
    if layer.isKindOfClass(sdf_class) {
        layer.setHidden(true);
        hidden.push(layer.retain());
        return;
    }
    // SAFETY: `layer` is a live `CALayer` on the main thread.
    let Some(sublayers) = (unsafe { layer.sublayers() }) else {
        return;
    };
    for sublayer in sublayers {
        collect_and_hide_sdf_layers(&sublayer, sdf_class, hidden);
    }
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

/// A `UIImage` showing `image` at `size` points, tagged `alwaysTemplate` —
/// its scale is the raster's pixels per point, so the image keeps every
/// captured pixel.
#[cfg(target_os = "ios")]
#[must_use]
pub fn template_image(image: &CGImage, size: Size) -> Retained<objc2_ui_kit::UIImage> {
    use objc2_ui_kit::{UIImage, UIImageOrientation, UIImageRenderingMode};
    #[expect(
        clippy::cast_precision_loss,
        reason = "a raster's pixel width is far below f64's exact integer range"
    )]
    let scale = CGImage::width(Some(image)) as f64 / size.width;
    let ui = UIImage::initWithCGImage_scale_orientation(
        UIImage::alloc(),
        image,
        scale,
        UIImageOrientation::Up,
    );
    ui.imageWithRenderingMode(UIImageRenderingMode::AlwaysTemplate)
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

/// A borderless `NSWindow` that is never ordered in, for offscreen capture.
///
/// It has no on-screen position to hide, while attaching a view to it drives
/// `AppKit`'s window-dependent rendering paths with key-window appearance so
/// controls draw their active accent colors rather than inactive gray.
#[cfg(target_os = "macos")]
#[must_use]
/// # Panics
///
/// On a failure to allocate the window.
pub fn capture_window(mtm: MainThreadMarker, size: Size) -> Retained<NSWindow> {
    use crate::dynamic_range::{DynamicRange, apply_to_view};
    // SAFETY: `initWithContentRect:styleMask:backing:defer:` is `NSWindow`'s
    // designated initializer; `setReleasedWhenClosed:false` keeps the object
    // owned by the `Retained` the caller drops.
    unsafe {
        let rect: CGRect = Rect::new(0.0, 0.0, size.width, size.height).into();
        let window: Retained<CaptureWindow> = msg_send![
            CaptureWindow::alloc(mtm),
            initWithContentRect: rect,
            styleMask: NSWindowStyleMask::Borderless,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ];
        let window: Retained<NSWindow> = Retained::into_super(window);
        window.setReleasedWhenClosed(false);
        // The offscreen render target is an RGBA8 bitmap, so the host
        // declares `DynamicRange::Standard` on its content view: a leaf
        // resolving its inherited mode inside this window answers from
        // the tag rather than the missing display. `apply_to_view`
        // preserves any local tag content inside the capture carries.
        if let Some(content) = window.contentView() {
            apply_to_view(DynamicRange::Standard, &content);
        }
        window
    }
}

#[cfg(target_os = "macos")]
define_class!(
    // SAFETY: `NSWindow`'s designated initializer is
    // `initWithContentRect:styleMask:backing:defer:`, which `capture_window`
    // calls, and the class holds no ivars and implements no `Drop`.
    #[unsafe(super(NSWindow))]
    #[name = "CocoaUiCaptureWindow"]
    #[thread_kind = MainThreadOnly]
    /// The never-ordered `NSWindow` a view is rehosted in for offscreen capture.
    ///
    /// Because a capture window is never ordered on screen or activated, a plain
    /// `NSWindow` reports `isKeyWindow == false` and `hasKeyAppearance == false`,
    /// causing `AppKit` controls (`NSButton`, `NSSwitch`, `NSSlider`,
    /// `NSProgressIndicator`, `NSSegmentedControl`) and key-window-aware views
    /// to render in their inactive gray state. Overriding the key/main state
    /// and appearance queries gives captured views their active appearance
    /// without ordering a window or changing application activation policy.
    struct CaptureWindow;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSWindow` subclass.
    unsafe impl NSObjectProtocol for CaptureWindow {}

    impl CaptureWindow {
        #[unsafe(method(isKeyWindow))]
        fn is_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(isMainWindow))]
        fn is_main_window(&self) -> bool {
            true
        }

        #[unsafe(method(hasKeyAppearance))]
        fn has_key_appearance(&self) -> bool {
            true
        }

        #[unsafe(method(hasMainAppearance))]
        fn has_main_appearance(&self) -> bool {
            true
        }
    }
);

/// An offscreen `UIWindow` in `scene`, attached so `UIView` captures see a
/// real window.
///
/// The window has no device regions: the layout region reader treats the
/// capture root as the window (`is_capture_window`), whatever safe area
/// `UIKit` propagates into it.
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

/// A scene-less `CaptureWindow` for a Mac Catalyst process that runs
/// without `UIApplicationMain`.
///
/// The window attaches to the screen's unassociated window scene, so a
/// hierarchy inside it gets the trait environment and layout pass native
/// controls build their parts in, while `UIKit` creates no `NSWindow` for
/// it — nothing is registered with the window server. `application` is
/// the proof the `AppKit` application that scene requires exists.
#[cfg(target_abi = "macabi")]
#[must_use]
pub fn scene_less_capture_window(
    mtm: objc2::MainThreadMarker,
    _application: &crate::uikit::catalyst::BackgroundApplication,
    size: Size,
) -> Retained<CaptureWindow> {
    // SAFETY: `initWithFrame:` is `UIView`'s designated initializer, which
    // `UIWindow` inherits for a window that names no scene.
    unsafe {
        objc2::msg_send![
            CaptureWindow::alloc(mtm),
            initWithFrame: objc2_core_foundation::CGRect::from(Rect::new(0.0, 0.0, size.width, size.height))
        ]
    }
}

#[cfg(target_os = "ios")]
objc2::define_class!(
    // SAFETY: `UIWindow` asks a subclass to initialize through a designated
    // initializer, which `make_offscreen_window` and
    // `scene_less_capture_window` do, and the class holds no ivars and
    // implements no `Drop`.
    #[unsafe(super(objc2_ui_kit::UIWindow))]
    #[name = "CocoaUiCaptureWindow"]
    #[thread_kind = MainThreadOnly]
    /// The window a view is rehosted in for capture. It never appears on a
    /// display, so no device region applies: the layout region reader
    /// treats the capture root (the root controller's view) as the window,
    /// with zero container and keyboard depths, and measures chrome bands
    /// against the capture root's own `safeAreaInsets`. `UIKit` still
    /// propagates the scene's device bands into the window's descendants;
    /// the reader, not the window's `safeAreaInsets`, is what ignores them.
    pub struct CaptureWindow;

    unsafe impl NSObjectProtocol for CaptureWindow {}
);

/// Whether `window` is a [`CaptureWindow`] — the context whose layout
/// regions follow the capture contract instead of the device's geometry.
#[cfg(target_os = "ios")]
#[must_use]
pub fn is_capture_window(window: &objc2_ui_kit::UIWindow) -> bool {
    use objc2::ClassType;
    window.isKindOfClass(CaptureWindow::class())
}

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

/// Mounts `view` in `window` through a plain `UIViewController`, unhides the
/// window and lays everything out — the `UIKit` analogue of
/// `orderFrontRegardless`.
///
/// Off the main thread this breaks the contract: it is reported and the
/// window is not shown.
#[cfg(target_os = "ios")]
pub fn show_capture_window(window: &CaptureWindow, view: &crate::PlatformView, size: Size) {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        contract_violation!("show_capture_window runs on the main thread; the window is not shown");
        return;
    };
    let controller = objc2_ui_kit::UIViewController::new(mtm);
    window.setRootViewController(Some(&controller));
    if let Some(content) = controller.view() {
        content.setFrame(Rect::new(0.0, 0.0, size.width, size.height).into());
        // The offscreen render target is an RGBA8 bitmap, so the host
        // view declares `DynamicRange::Standard` for the subtree it
        // hosts — tagged before `view` attaches so window-dependent
        // dynamic-range resolution answers SDR instead of panicking on
        // the missing display. `apply_to_view` preserves local tags.
        crate::dynamic_range::apply_to_view(crate::dynamic_range::DynamicRange::Standard, &content);
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
    if let Some(layer) = view.layer() {
        without_sdf_layers(&layer, || {
            view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
        });
    } else {
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    }
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
    Some(template_image(
        &image,
        Size::new(bounds.size.width * shrink, bounds.size.height * shrink),
    ))
}

/// Any connected `UIWindowScene`.
///
/// Capture windows must attach to a real scene, and any connected one works.
/// `None` also in a process without a `UIApplication`, which connects no
/// scene.
///
/// Off the main thread this breaks the contract: it is reported and no scene
/// is answered.
#[cfg(target_os = "ios")]
#[must_use]
pub fn any_window_scene() -> Option<Retained<objc2_ui_kit::UIWindowScene>> {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        contract_violation!("any_window_scene runs on the main thread; no scene is answered");
        return None;
    };
    let application = crate::uikit::shared_application(mtm)?;
    let scenes = application.connectedScenes();
    let iterator = scenes.iter();
    for scene in iterator {
        if let Ok(window_scene) = scene.downcast::<objc2_ui_kit::UIWindowScene>() {
            return Some(window_scene);
        }
    }
    None
}

/// Applies `scheme` to `window` so offscreen controls and dynamic system colors
/// resolve against the requested light or dark appearance.
#[cfg(target_os = "macos")]
pub fn set_window_color_scheme(window: &NSWindow, scheme: ColorScheme) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization as _, NSAppearanceNameAqua,
        NSAppearanceNameDarkAqua,
    };

    // SAFETY: `NSAppearanceNameAqua` and `NSAppearanceNameDarkAqua` are
    // immutable `AppKit` string constants.
    let name = unsafe {
        match scheme {
            ColorScheme::Light => NSAppearanceNameAqua,
            ColorScheme::Dark => NSAppearanceNameDarkAqua,
        }
    };
    if let Some(appearance) = NSAppearance::appearanceNamed(name) {
        window.setAppearance(Some(&appearance));
    }
}

/// Applies `scheme` to `window` through `overrideUserInterfaceStyle` so
/// offscreen `UIKit` controls and materials resolve against the requested
/// light or dark interface style.
#[cfg(target_os = "ios")]
pub fn set_window_color_scheme(window: &UIWindow, scheme: ColorScheme) {
    use objc2_ui_kit::UIUserInterfaceStyle;

    let style = match scheme {
        ColorScheme::Light => UIUserInterfaceStyle::Light,
        ColorScheme::Dark => UIUserInterfaceStyle::Dark,
    };
    window.setOverrideUserInterfaceStyle(style);
}

/// Opt-in in-process `Core Animation` render-server capture.
///
/// Strictly compiled under `feature = "render-server-capture"` (forwarded only
/// by `cocoa-ui/native-test` and `waterui-apple/preview`) on `macOS` and Mac
/// Catalyst so production builds link and reference zero private `QuartzCore`
/// symbols.
#[cfg(all(
    feature = "render-server-capture",
    any(target_os = "macos", all(target_os = "ios", target_abi = "macabi"))
))]
mod render_server {
    use core::ptr::{from_ref, null, null_mut};
    use core::slice::from_raw_parts;

    use filtrate_core::space::from_srgb;
    use half::f16;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{Message as _, msg_send, sel};
    use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString};
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
        CGImageAlphaInfo, kCGColorSpaceDisplayP3, kCGColorSpaceExtendedLinearDisplayP3,
        kCGColorSpaceExtendedSRGB,
    };
    use objc2_foundation::{NSDictionary, NSObjectProtocol as _, NSString};
    use objc2_io_surface::{
        IOSurfaceLockOptions, IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceColorSpace,
        kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth,
    };
    use objc2_quartz_core::{
        CADynamicRange, CADynamicRangeHigh, CALayer, CATransaction, CATransform3D,
    };

    use super::{BITS_PER_COMPONENT, BITS_PER_PIXEL, BYTES_PER_PIXEL, linear_p3_to_display_p3_u8};

    /// Scale and translation mapping layer coordinates into the render-server
    /// surface.
    #[derive(Clone, Copy, Debug)]
    pub struct RenderServerTransform {
        /// Horizontal point-to-pixel scale.
        pub scale_x: f64,
        /// Vertical point-to-pixel scale.
        pub scale_y: f64,
        /// Horizontal pixel translation.
        pub translate_x: f64,
        /// Vertical pixel translation.
        pub translate_y: f64,
    }

    /// `kCVPixelFormatType_64RGBAHalf` four-character code (`'RGhA'`), four
    /// IEEE 754 binary16 channels per pixel (8 bytes/element).
    ///
    /// `CARenderServer` composites into sRGB primaries with the sRGB transfer
    /// curve regardless of `kIOSurfaceColorSpace` or `CAContext.colorSpace`:
    /// an 8-bit `BGRA8` surface clips to `[0, 1]` sRGB, whereas `'RGhA'`
    /// writes premultiplied `kCGColorSpaceExtendedSRGB` half-floats that
    /// preserve both wide-gamut Display P3 (via negative and `> 1.0` channels)
    /// and HDR headroom.
    const SURFACE_PIXEL_FORMAT_RGBA16F: i64 = u32::from_be_bytes(*b"RGhA") as i64;
    const SURFACE_BYTES_PER_ELEMENT_RGBA16F: i64 = 8;
    /// EDR headroom requested on the temporary local `CAContext` during HDR
    /// render-server captures — matches Apple's maximum 16x (4-stop) EDR range
    /// so Liquid Glass and HDR highlights render unclamped without hitting an
    /// SDR ceiling.
    const HDR_DESIRED_DYNAMIC_RANGE: f32 = 16.0;
    /// `QuartzCore` render-server flag (`0x80`) that leaves `CA::Render::Update`
    /// in macOS's native bottom-up coordinate space so `AppKit` `CABackingStore`
    /// bitmaps (`NSViewBackingLayerContents`, `CGDrawingLayer`) share the same
    /// vertical orientation as `CAShapeLayer` and layer geometry, while
    /// post-multiplying `transform` by `Translate(0, surface_height) * Scale(1, -1)`.
    #[cfg(target_os = "macos")]
    const RENDER_SERVER_FLAG_UNFLIPPED_COORDINATES: u32 = 0x80;

    #[link(name = "QuartzCore", kind = "framework")]
    unsafe extern "C" {
        fn CARenderServerStart() -> bool;
        fn CARenderServerGetPort() -> u32;
        #[cfg(target_os = "macos")]
        fn CARenderServerRenderDisplayLayerWithTransformTimeOffsetAndFlags(
            server_port: u32,
            display_name: *const CFString,
            client_id: u32,
            layer: *const CALayer,
            surface: *const IOSurfaceRef,
            ox: i32,
            oy: i32,
            transform: *const CATransform3D,
            time_offset: f64,
            flags: u32,
        ) -> bool;
        #[cfg(not(target_os = "macos"))]
        fn CARenderServerRenderLayerWithTransform(
            server_port: u32,
            client_id: u32,
            layer: *const CALayer,
            surface: *const IOSurfaceRef,
            ox: i32,
            oy: i32,
            transform: *const CATransform3D,
        ) -> bool;
    }

    struct LocalContextGuard {
        context: Retained<AnyObject>,
        previous_context: Option<Retained<AnyObject>>,
        previous_root: Retained<CALayer>,
        previous_dynamic_ranges: Vec<(Retained<CALayer>, Retained<CADynamicRange>)>,
        #[cfg(target_os = "macos")]
        unwrapped_layer: Option<Retained<CALayer>>,
    }

    impl Drop for LocalContextGuard {
        fn drop(&mut self) {
            if !self.previous_dynamic_ranges.is_empty() {
                CATransaction::begin();
                CATransaction::setDisableActions(true);
                for (layer, previous) in self.previous_dynamic_ranges.drain(..) {
                    layer.setPreferredDynamicRange(&previous);
                }
                CATransaction::commit();
            }
            // SAFETY: `CAContext` exposes `-setLayer:` (accepting `nil`) and
            // `-invalidate` on the main thread.
            unsafe {
                let _: () = msg_send![&*self.context, setLayer: Option::<&CALayer>::None];
                let _: () = msg_send![&*self.context, invalidate];
            }
            #[cfg(target_os = "macos")]
            if let Some(layer) = &self.unwrapped_layer {
                CATransaction::begin();
                CATransaction::setDisableActions(true);
                layer.removeFromSuperlayer();
                CATransaction::commit();
            }
            if let Some(previous_context) = &self.previous_context {
                // SAFETY: `previous_context` was `previous_root`'s own
                // `CAContext` before the temporary local context took over.
                unsafe {
                    let _: () = msg_send![&**previous_context, setLayer: &*self.previous_root];
                }
                CATransaction::flush();
            }
        }
    }

    fn allocate_iosurface(
        width: usize,
        height: usize,
        bytes_per_element: i64,
        pixel_format: i64,
        color_space_name: &CFString,
    ) -> Option<CFRetained<IOSurfaceRef>> {
        let width = i64::try_from(width).ok()?;
        let height = i64::try_from(height).ok()?;
        // SAFETY: these statics are `IOSurface` framework `CFString` constants.
        let keys: [&CFString; 4] = unsafe {
            [
                kIOSurfaceWidth,
                kIOSurfaceHeight,
                kIOSurfaceBytesPerElement,
                kIOSurfacePixelFormat,
            ]
        };
        let values = [
            CFNumber::new_i64(width),
            CFNumber::new_i64(height),
            CFNumber::new_i64(bytes_per_element),
            CFNumber::new_i64(pixel_format),
        ];
        let value_refs: [&CFNumber; 4] = values.each_ref().map(|value| &**value);
        let properties = CFDictionary::from_slices(&keys, &value_refs);
        // SAFETY: every key in `properties` is a valid `IOSurface` property
        // paired with a `CFNumber`.
        let surface = unsafe { IOSurfaceRef::new(properties.as_opaque()) }?;
        if let Some(space) = CGColorSpace::with_name(Some(color_space_name))
            && let Some(plist) = space.property_list()
        {
            // SAFETY: `kIOSurfaceColorSpace` accepts a color-space property list.
            unsafe { surface.set_value(kIOSurfaceColorSpace, &plist) };
        }
        Some(surface)
    }

    fn allocate_rgba16f_surface(width: usize, height: usize) -> Option<CFRetained<IOSurfaceRef>> {
        // SAFETY: `kCGColorSpaceExtendedSRGB` is a `Core Graphics` string
        // constant matching the premultiplied extended sRGB encoding
        // `CARenderServer` writes into `'RGhA'` surfaces.
        let color_space = unsafe { kCGColorSpaceExtendedSRGB };
        allocate_iosurface(
            width,
            height,
            SURFACE_BYTES_PER_ELEMENT_RGBA16F,
            SURFACE_PIXEL_FORMAT_RGBA16F,
            color_space,
        )
    }

    /// Reads premultiplied `kCGColorSpaceExtendedSRGB` half-float pixels from
    /// `surface` and converts them to premultiplied Display P3 RGBA8.
    fn read_surface_rgba16f_display_p3_u8(
        surface: &IOSurfaceRef,
        pixel_width: usize,
        pixel_height: usize,
    ) -> Option<Vec<u8>> {
        let row_bytes = pixel_width.checked_mul(8)?;
        // SAFETY: read-only lock on a live `IOSurface` owned by the caller.
        let lock_status = unsafe { surface.lock(IOSurfaceLockOptions::ReadOnly, null_mut()) };
        if lock_status != 0 {
            return None;
        }
        let bytes_per_row = surface.bytes_per_row();
        let base = surface.base_address().as_ptr().cast::<u8>();
        let pixels = if !base.is_null() && bytes_per_row >= row_bytes {
            let mut pixels = vec![0u8; pixel_width * pixel_height * 4];
            let mut row_buf = vec![[0.0_f32; 4]; pixel_width];
            // SAFETY: `surface` is locked read-only and has `pixel_height` rows
            // of at least `bytes_per_row` bytes each.
            let raw = unsafe { from_raw_parts(base, bytes_per_row * pixel_height) };
            for y in 0..pixel_height {
                let src_row = &raw[y * bytes_per_row..y * bytes_per_row + row_bytes];
                for (src_px, dst_px) in src_row.as_chunks::<8>().0.iter().zip(row_buf.iter_mut()) {
                    *dst_px = [
                        f16::from_le_bytes([src_px[0], src_px[1]]).to_f32(),
                        f16::from_le_bytes([src_px[2], src_px[3]]).to_f32(),
                        f16::from_le_bytes([src_px[4], src_px[5]]).to_f32(),
                        f16::from_le_bytes([src_px[6], src_px[7]]).to_f32(),
                    ];
                }
                from_srgb(&mut row_buf);
                let dst_row = &mut pixels[y * pixel_width * 4..(y + 1) * pixel_width * 4];
                for (linear_p3, dst_px) in row_buf.iter().zip(dst_row.as_chunks_mut::<4>().0) {
                    *dst_px = linear_p3_to_display_p3_u8(*linear_p3);
                }
            }
            Some(pixels)
        } else {
            None
        };
        // SAFETY: pairs the successful read-only lock above.
        unsafe { surface.unlock(IOSurfaceLockOptions::ReadOnly, null_mut()) };
        pixels
    }

    /// Reads premultiplied `kCGColorSpaceExtendedSRGB` half-float pixels from
    /// `surface` and converts them to premultiplied extended linear Display P3
    /// `f32` RGBA pixels.
    fn read_surface_rgba16f_linear_p3(
        surface: &IOSurfaceRef,
        pixel_width: usize,
        pixel_height: usize,
    ) -> Option<Vec<f32>> {
        let row_bytes = pixel_width.checked_mul(8)?;
        // SAFETY: read-only lock on a live `IOSurface` owned by the caller.
        let lock_status = unsafe { surface.lock(IOSurfaceLockOptions::ReadOnly, null_mut()) };
        if lock_status != 0 {
            return None;
        }
        let bytes_per_row = surface.bytes_per_row();
        let base = surface.base_address().as_ptr().cast::<u8>();
        let pixels = if !base.is_null() && bytes_per_row >= row_bytes {
            let mut pixels = vec![0.0_f32; pixel_width * pixel_height * 4];
            // SAFETY: `surface` is locked read-only and has `pixel_height` rows
            // of at least `bytes_per_row` bytes each.
            let raw = unsafe { from_raw_parts(base, bytes_per_row * pixel_height) };
            for y in 0..pixel_height {
                let src_row = &raw[y * bytes_per_row..y * bytes_per_row + row_bytes];
                let dst_row = &mut pixels[y * pixel_width * 4..(y + 1) * pixel_width * 4];
                for (src_ch, dst_ch) in src_row.as_chunks::<2>().0.iter().zip(dst_row.iter_mut()) {
                    *dst_ch = f16::from_le_bytes(*src_ch).to_f32();
                }
                from_srgb(dst_row.as_chunks_mut::<4>().0);
            }
            Some(pixels)
        } else {
            None
        };
        // SAFETY: pairs the successful read-only lock above.
        unsafe { surface.unlock(IOSurfaceLockOptions::ReadOnly, null_mut()) };
        pixels
    }

    fn read_surface_rgba16f_into_buffer(
        surface: &IOSurfaceRef,
        pixel_width: usize,
        pixel_height: usize,
        dst: &mut [u8],
        dst_row_bytes: usize,
    ) -> bool {
        let Some(row_bytes) = pixel_width.checked_mul(8) else {
            return false;
        };
        if dst_row_bytes < row_bytes
            || dst_row_bytes
                .checked_mul(pixel_height)
                .is_none_or(|needed| dst.len() < needed)
        {
            return false;
        }
        // SAFETY: read-only lock on a live `IOSurface` owned by the caller.
        let lock_status = unsafe { surface.lock(IOSurfaceLockOptions::ReadOnly, null_mut()) };
        if lock_status != 0 {
            return false;
        }
        let src_row_bytes = surface.bytes_per_row();
        let base = surface.base_address().as_ptr().cast::<u8>();
        let ok = if !base.is_null() && src_row_bytes >= row_bytes {
            // SAFETY: `surface` is locked read-only and has `pixel_height` rows
            // of at least `src_row_bytes` bytes each.
            let raw = unsafe { from_raw_parts(base, src_row_bytes * pixel_height) };
            for y in 0..pixel_height {
                let src_row = &raw[y * src_row_bytes..y * src_row_bytes + row_bytes];
                let dst_row = &mut dst[y * dst_row_bytes..y * dst_row_bytes + row_bytes];
                dst_row.copy_from_slice(src_row);
            }
            true
        } else {
            false
        };
        // SAFETY: pairs the successful read-only lock above.
        unsafe { surface.unlock(IOSurfaceLockOptions::ReadOnly, null_mut()) };
        ok
    }

    /// Wraps premultiplied Display P3 RGBA8 pixels into a `CGImage` tagged with
    /// `kCGColorSpaceDisplayP3` so drawing into a `CGContext` converts cleanly
    /// without clamping wide-gamut colors to sRGB.
    pub fn image_from_display_p3_rgba(
        pixels: &[u8],
        width: usize,
        height: usize,
    ) -> Option<CFRetained<CGImage>> {
        if pixels.len() != width * height * BYTES_PER_PIXEL {
            return None;
        }
        let data = CFData::from_bytes(pixels);
        let provider = CGDataProvider::with_cf_data(Some(&data))?;
        // SAFETY: `kCGColorSpaceDisplayP3` is a `Core Graphics` string constant.
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceDisplayP3 }))?;
        // SAFETY: `decode` is null; every other argument matches `pixels`'
        // premultiplied RGBA8 layout.
        unsafe {
            CGImage::new(
                width,
                height,
                BITS_PER_COMPONENT,
                BITS_PER_PIXEL,
                width * BYTES_PER_PIXEL,
                Some(&space),
                CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
                Some(&provider),
                null(),
                true,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
    }

    /// Renders `layer`'s committed subtree through an in-process
    /// `Core Animation` render server into premultiplied top-down Display P3
    /// RGBA8 pixels (`pixel_width * pixel_height * 4` bytes).
    ///
    /// Unlike `CALayer.renderInContext:`, the in-process render server runs
    /// `Core Animation`'s real compositor passes — evaluating `CABackdropLayer`
    /// and `CASDFLayer` glass materials (`UIGlassEffect`, `UIVisualEffectView`,
    /// `UIButtonConfiguration.{glass,prominentGlass}`, `NSGlassEffectView`,
    /// `NSBezelStyle::Glass`) without ordering any window on screen or crossing
    /// IPC to `WindowServer`.
    #[must_use]
    pub fn render_layer_via_render_server(
        layer: &CALayer,
        scale: f64,
        pixel_width: usize,
        pixel_height: usize,
    ) -> Option<Vec<u8>> {
        let bounds = layer.bounds();
        render_layer_via_render_server_transformed(
            layer,
            RenderServerTransform {
                scale_x: scale,
                scale_y: scale,
                translate_x: -bounds.origin.x * scale,
                translate_y: -bounds.origin.y * scale,
            },
            pixel_width,
            pixel_height,
        )
    }

    /// Renders `layer`'s committed subtree into unclamped extended linear
    /// Display P3 `f32` RGBA pixels (`pixel_width * pixel_height * 4` floats).
    ///
    /// Uses an in-process `Core Animation` render server targeting a
    /// `kCVPixelFormatType_64RGBAHalf` (`'RGhA'`) surface and converts its
    /// premultiplied `kCGColorSpaceExtendedSRGB` output into extended linear
    /// Display P3 so both wide-gamut colors and HDR (`> 1.0`) highlights from
    /// Liquid Glass and extended-range layers are preserved without clipping.
    #[must_use]
    pub fn render_layer_via_render_server_hdr(
        layer: &CALayer,
        scale: f64,
        pixel_width: usize,
        pixel_height: usize,
    ) -> Option<Vec<f32>> {
        let bounds = layer.bounds();
        render_layer_via_render_server_hdr_transformed(
            layer,
            RenderServerTransform {
                scale_x: scale,
                scale_y: scale,
                translate_x: -bounds.origin.x * scale,
                translate_y: -bounds.origin.y * scale,
            },
            pixel_width,
            pixel_height,
        )
    }

    struct RenderTarget {
        layer: Retained<CALayer>,
        root: Retained<CALayer>,
        #[cfg(target_os = "macos")]
        unwrapped_layer: Option<Retained<CALayer>>,
        offset_x: f64,
        offset_y: f64,
    }

    fn resolve_render_target(
        layer: &CALayer,
        top_root: Retained<CALayer>,
        transform: RenderServerTransform,
    ) -> RenderTarget {
        // On AppKit, `CARenderServer` skips the target layer's own
        // `geometryFlipped` transform on direct sublayer positions while still
        // honoring `contentsAreFlipped` on child backing stores. Rendering
        // through an unflipped ancestor (the window's root layer, or a
        // temporary unflipped wrapper when `layer` is detached) lets Core
        // Animation traverse the `unflipped -> flipped` edge so sublayer
        // positions, glyphs, controls, and glass all share one orientation.
        #[cfg(target_os = "macos")]
        if layer.isGeometryFlipped() {
            if let Some(parent) = layer.superlayer()
                && !parent.isGeometryFlipped()
            {
                let frame = layer.frame();
                return RenderTarget {
                    layer: parent,
                    root: top_root,
                    unwrapped_layer: None,
                    offset_x: -frame.origin.x * transform.scale_x,
                    offset_y: -frame.origin.y * transform.scale_y,
                };
            }
            if layer.superlayer().is_none() {
                let wrapper = CALayer::new();
                CATransaction::begin();
                CATransaction::setDisableActions(true);
                wrapper.setBounds(layer.bounds());
                wrapper.addSublayer(layer);
                CATransaction::commit();
                return RenderTarget {
                    layer: wrapper.clone(),
                    root: wrapper,
                    unwrapped_layer: Some(layer.retain()),
                    offset_x: 0.0,
                    offset_y: 0.0,
                };
            }
        }
        let _ = transform;
        RenderTarget {
            layer: layer.retain(),
            root: top_root,
            #[cfg(target_os = "macos")]
            unwrapped_layer: None,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    fn build_ca_transform(
        transform: RenderServerTransform,
        offset_x: f64,
        offset_y: f64,
        pixel_height: usize,
    ) -> CATransform3D {
        #[cfg(target_os = "macos")]
        let m42 = {
            // `RENDER_SERVER_FLAG_UNFLIPPED_COORDINATES` post-multiplies
            // `ca_transform` by `Translate(0, surface_height) * Scale(1, -1)`,
            // so `y_out = scale_y * (H - y) + m42`. Setting
            // `m42 = H - scale_y * H - (translate_y + offset_y)` yields
            // `y_out = H - (scale_y * y + translate_y + offset_y)`, mapping
            // bottom-up AppKit layer coordinates directly into top-down
            // `IOSurface` rows.
            #[expect(
                clippy::cast_precision_loss,
                reason = "capture pixel height fits well within f64's exact integer range"
            )]
            let surface_height = pixel_height as f64;
            transform.scale_y.mul_add(-surface_height, surface_height)
                - (transform.translate_y + offset_y)
        };
        #[cfg(not(target_os = "macos"))]
        let m42 = {
            let _ = pixel_height;
            transform.translate_y + offset_y
        };
        CATransform3D {
            m11: transform.scale_x,
            m12: 0.0,
            m13: 0.0,
            m14: 0.0,
            m21: 0.0,
            m22: transform.scale_y,
            m23: 0.0,
            m24: 0.0,
            m31: 0.0,
            m32: 0.0,
            m33: 1.0,
            m34: 0.0,
            m41: transform.translate_x + offset_x,
            m42,
            m43: 0.0,
            m44: 1.0,
        }
    }

    fn configure_local_context(context: &AnyObject, hdr: bool) {
        let context_space_name = if hdr {
            // SAFETY: `kCGColorSpaceExtendedLinearDisplayP3` is a system constant.
            unsafe { kCGColorSpaceExtendedLinearDisplayP3 }
        } else {
            // SAFETY: `kCGColorSpaceDisplayP3` is a system constant.
            unsafe { kCGColorSpaceDisplayP3 }
        };
        // SAFETY: `CAContext` inherits `NSObject`'s `-respondsToSelector:`.
        let supports_color_space: bool =
            unsafe { msg_send![context, respondsToSelector: sel!(setColorSpace:)] };
        if supports_color_space
            && let Some(space) = CGColorSpace::with_name(Some(context_space_name))
        {
            // SAFETY: `-[CAContext setColorSpace:]` accepts a `CGColorSpaceRef`.
            unsafe {
                let _: () = msg_send![context, setColorSpace: from_ref(&*space)];
            }
        }
        // SAFETY: `CAContext` inherits `NSObject`'s `-respondsToSelector:`.
        let supports_dynamic_range: bool =
            unsafe { msg_send![context, respondsToSelector: sel!(setDesiredDynamicRange:)] };
        if hdr && supports_dynamic_range {
            // SAFETY: `-[CAContext setDesiredDynamicRange:]` accepts an `f32`
            // EDR headroom multiplier on modern QuartzCore.
            unsafe {
                let _: () = msg_send![context, setDesiredDynamicRange: HDR_DESIRED_DYNAMIC_RANGE];
            }
        }
    }

    fn apply_hdr_dynamic_range(
        root: &CALayer,
        target: &CALayer,
    ) -> Vec<(Retained<CALayer>, Retained<CADynamicRange>)> {
        let mut previous = Vec::with_capacity(2);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for candidate in [root, target] {
            let prior = candidate.preferredDynamicRange();
            // SAFETY: `CADynamicRangeHigh` is an immutable framework constant.
            candidate.setPreferredDynamicRange(unsafe { CADynamicRangeHigh });
            previous.push((candidate.retain(), prior));
        }
        CATransaction::commit();
        previous
    }

    fn render_layer_into_surface(
        layer: &CALayer,
        transform: RenderServerTransform,
        pixel_width: usize,
        pixel_height: usize,
        surface: &IOSurfaceRef,
        hdr: bool,
    ) -> bool {
        if pixel_width == 0
            || pixel_height == 0
            || !transform.scale_x.is_finite()
            || transform.scale_x <= 0.0
            || !transform.scale_y.is_finite()
            || transform.scale_y <= 0.0
        {
            return false;
        }
        // SAFETY: `CARenderServerStart` is idempotent (guarded internally by
        // `CA::Render::Server::_mutex`) and must precede `CARenderServerGetPort`,
        // which otherwise lazily allocates an unserviced Mach port.
        let server_port = unsafe {
            if !CARenderServerStart() {
                return false;
            }
            CARenderServerGetPort()
        };
        if server_port == 0 {
            return false;
        }

        let Some(context_class) = AnyClass::get(c"CAContext") else {
            return false;
        };
        let options = NSDictionary::<NSString, AnyObject>::new();
        // SAFETY: `+[CAContext localContextWithOptions:]` returns an
        // autoreleased local `CAContext` (or nil).
        let context: Option<Retained<AnyObject>> =
            unsafe { msg_send![context_class, localContextWithOptions: &*options] };
        let Some(context) = context else {
            return false;
        };
        configure_local_context(&context, hdr);

        // SAFETY: `-[CAContext contextId]` returns `u32`.
        let context_id: u32 = unsafe { msg_send![&*context, contextId] };

        // Walk up to the root-most layer so binding `context.layer` never
        // detaches `layer` from its `superlayer`.
        let mut top_root = layer.retain();
        while let Some(parent) = top_root.superlayer() {
            top_root = parent;
        }
        let previous_root = top_root.clone();
        let previous_context: Option<Retained<AnyObject>> =
            if top_root.respondsToSelector(sel!(context)) {
                // SAFETY: `-[CALayer context]` returns the owning `CAContext`
                // (or nil) on the main thread when the selector is available.
                unsafe { msg_send![&*top_root, context] }
            } else {
                None
            };

        // Flush any pending layout/display transaction before switching the
        // layer tree onto the local render-server context.
        CATransaction::flush();

        let target = resolve_render_target(layer, top_root, transform);
        let previous_dynamic_ranges = if hdr {
            apply_hdr_dynamic_range(&target.root, &target.layer)
        } else {
            Vec::new()
        };

        // SAFETY: `-[CAContext setLayer:]` binds the root layer on the main thread.
        unsafe {
            let _: () = msg_send![&*context, setLayer: &*target.root];
        }
        let guard = LocalContextGuard {
            context,
            previous_context,
            previous_root,
            previous_dynamic_ranges,
            #[cfg(target_os = "macos")]
            unwrapped_layer: target.unwrapped_layer,
        };
        CATransaction::flush();

        let ca_transform =
            build_ca_transform(transform, target.offset_x, target.offset_y, pixel_height);
        #[cfg(target_os = "macos")]
        let rendered = {
            let display = CFString::from_str("defaultDisplay");
            // SAFETY: `server_port`, `display`, `context_id`, `target.layer`,
            // `surface`, and `ca_transform` are live on the main thread for the
            // duration of the call.
            unsafe {
                CARenderServerRenderDisplayLayerWithTransformTimeOffsetAndFlags(
                    server_port,
                    from_ref(&*display),
                    context_id,
                    from_ref(&*target.layer),
                    from_ref(surface),
                    0,
                    0,
                    &raw const ca_transform,
                    0.0,
                    RENDER_SERVER_FLAG_UNFLIPPED_COORDINATES,
                )
            }
        };
        #[cfg(not(target_os = "macos"))]
        // SAFETY: `server_port`, `context_id`, `target.layer`, `surface`, and
        // `ca_transform` are live on the main thread for the duration of the call.
        let rendered = unsafe {
            CARenderServerRenderLayerWithTransform(
                server_port,
                context_id,
                from_ref(&*target.layer),
                from_ref(surface),
                0,
                0,
                &raw const ca_transform,
            )
        };
        drop(guard);
        rendered
    }

    /// Renders `layer`'s committed subtree through an in-process
    /// `Core Animation` render server with an explicit scale and pixel
    /// translation, returning premultiplied top-down Display P3 RGBA8 pixels.
    ///
    /// Renders into an `RGBA16F` (`'RGhA'`) surface so `CARenderServer`'s
    /// extended sRGB output keeps the full Display P3 gamut unclipped before
    /// color-matching into Display P3 RGBA8. On `macOS`,
    /// `CARenderServerRenderDisplayLayerWithTransformTimeOffsetAndFlags` is
    /// invoked with `RENDER_SERVER_FLAG_UNFLIPPED_COORDINATES` (`0x80`) so
    /// `AppKit` `CABackingStore` bitmaps and `CAShapeLayer` vectors render with
    /// matching orientation directly into top-down `IOSurface` rows. On Mac
    /// Catalyst (`UIKit`), `CARenderServerRenderLayerWithTransform` already
    /// renders top-down directly.
    #[must_use]
    pub fn render_layer_via_render_server_transformed(
        layer: &CALayer,
        transform: RenderServerTransform,
        pixel_width: usize,
        pixel_height: usize,
    ) -> Option<Vec<u8>> {
        let surface = allocate_rgba16f_surface(pixel_width, pixel_height)?;
        if !render_layer_into_surface(layer, transform, pixel_width, pixel_height, &surface, false)
        {
            return None;
        }
        read_surface_rgba16f_display_p3_u8(&surface, pixel_width, pixel_height)
    }

    /// Renders `layer`'s committed subtree with an explicit transform into
    /// unclamped extended linear Display P3 `f32` RGBA pixels.
    ///
    /// Allocates an `RGBA16F` (`kCVPixelFormatType_64RGBAHalf`) `IOSurface`,
    /// decodes `CARenderServer`'s premultiplied `kCGColorSpaceExtendedSRGB`
    /// half-float pixels, and transforms them into extended linear Display P3.
    #[must_use]
    pub fn render_layer_via_render_server_hdr_transformed(
        layer: &CALayer,
        transform: RenderServerTransform,
        pixel_width: usize,
        pixel_height: usize,
    ) -> Option<Vec<f32>> {
        let surface = allocate_rgba16f_surface(pixel_width, pixel_height)?;
        if !render_layer_into_surface(layer, transform, pixel_width, pixel_height, &surface, true) {
            return None;
        }
        read_surface_rgba16f_linear_p3(&surface, pixel_width, pixel_height)
    }

    /// Renders `layer`'s committed subtree into `dst` as premultiplied
    /// `kCGColorSpaceExtendedSRGB` `RGBA16F` rows (`BLEND_COLOR`) using
    /// `dst_row_bytes` stride for direct consumption by `ViewCapture`'s Metal
    /// compositor.
    pub fn render_layer_via_render_server_extended_srgb_into_buffer(
        layer: &CALayer,
        transform: RenderServerTransform,
        pixel_width: usize,
        pixel_height: usize,
        dst: &mut [u8],
        dst_row_bytes: usize,
        hdr: bool,
    ) -> bool {
        let Some(surface) = allocate_rgba16f_surface(pixel_width, pixel_height) else {
            return false;
        };
        if !render_layer_into_surface(layer, transform, pixel_width, pixel_height, &surface, hdr) {
            return false;
        }
        read_surface_rgba16f_into_buffer(&surface, pixel_width, pixel_height, dst, dst_row_bytes)
    }
}

#[cfg(all(
    feature = "render-server-capture",
    any(target_os = "macos", all(target_os = "ios", target_abi = "macabi"))
))]
pub use render_server::{
    RenderServerTransform, render_layer_via_render_server, render_layer_via_render_server_hdr,
    render_layer_via_render_server_hdr_transformed, render_layer_via_render_server_transformed,
};
#[cfg(all(
    feature = "render-server-capture",
    any(target_os = "macos", all(target_os = "ios", target_abi = "macabi"))
))]
pub(crate) use render_server::{
    image_from_display_p3_rgba, render_layer_via_render_server_extended_srgb_into_buffer,
};
