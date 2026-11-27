//! Apple-platform SF Symbol realization for Hydrolysis.
//!
//! Queries the host operating system's SF Symbol catalog at runtime (`NSImage`
//! on macOS, `UIImage` on iOS), rasterizes the configured symbol into an 8-bit
//! CoreGraphics alpha mask at the active display scale factor, tints it with
//! the live `Foreground` [`WorkingColor`] in Display P3, and draws it through
//! the Cherenkov scene recorder.

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;

use cherenkov_record::{ImageColorSpace, ImageData, Rgba8, Sampling, WorkingColor};
use kurbo::Rect;
use lru::LruCache;
use nami::{Computed, SignalExt as _};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGContext, CGImageAlphaInfo};
use objc2_foundation::NSString;
use waterui_backend_core::SfSymbol;
use waterui_controls::theme::color;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::resolve::Resolvable as _;
use waterui_icon::IconSize;
use waterui_text::font::{Body, FontWeight, ResolvedFont};

use crate::num_cast::{f32_as_u8, f64_as_f32, f64_as_u32};
use crate::renderer::{HydroState, WidgetRenderContext};

unsafe extern "C" {
    /// `CGBitmapContextCreate` — `objc2-core-graphics` does not expose a binding.
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

const SIZE_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(256).unwrap();
const MASK_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(128).unwrap();
const TINTED_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(128).unwrap();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MeasureKey {
    name: &'static str,
    size_bits: u32,
    weight: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MaskKey {
    name: &'static str,
    size_bits: u32,
    weight: u8,
    scale_bits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TintedKey {
    mask: MaskKey,
    rgba: [u8; 4],
}

#[derive(Debug, Clone)]
struct CachedMask {
    point_size: LayoutSize,
    width: u32,
    height: u32,
    alpha: Arc<[u8]>,
}

/// Per-renderer cache for measured SF Symbol sizes, rasterized alpha masks,
/// and Display P3 tinted `ImageData` uploads.
pub struct SfSymbolCache {
    sizes: LruCache<MeasureKey, LayoutSize>,
    masks: LruCache<MaskKey, CachedMask>,
    tinted: LruCache<TintedKey, (u64, LayoutSize, ImageData<Rgba8>)>,
    next_image_id: u64,
}

impl SfSymbolCache {
    pub fn new() -> Self {
        Self {
            sizes: LruCache::new(SIZE_CACHE_CAPACITY),
            masks: LruCache::new(MASK_CACHE_CAPACITY),
            tinted: LruCache::new(TINTED_CACHE_CAPACITY),
            next_image_id: 1,
        }
    }

    fn measure(&mut self, name: &'static str, point_size: f32, weight: FontWeight) -> LayoutSize {
        let size = normalized_point_size(point_size);
        let key = MeasureKey {
            name,
            size_bits: size.to_bits(),
            weight: weight_discriminant(weight),
        };
        if let Some(&cached) = self.sizes.get(&key) {
            return cached;
        }
        let measured = platform_measure_symbol(name, size, weight).unwrap_or_else(|| {
            waterui_core::contract_violation!(
                "Unknown SF Symbol name: {name}; the view measures zero"
            );
            LayoutSize::zero()
        });
        self.sizes.put(key, measured);
        measured
    }

    fn tinted_image(
        &mut self,
        name: &'static str,
        point_size: f32,
        weight: FontWeight,
        scale: f64,
        foreground: WorkingColor,
    ) -> Option<(u64, LayoutSize, ImageData<Rgba8>)> {
        let size = normalized_point_size(point_size);
        let scale_f32 = f64_as_f32(scale.clamp(1.0, 4.0));
        let mask_key = MaskKey {
            name,
            size_bits: size.to_bits(),
            weight: weight_discriminant(weight),
            scale_bits: scale_f32.to_bits(),
        };
        let rgba = working_color_to_display_p3_rgba8(foreground);
        let tinted_key = TintedKey {
            mask: mask_key,
            rgba,
        };
        if let Some(entry) = self.tinted.get(&tinted_key) {
            return Some(entry.clone());
        }

        let mask = if let Some(cached) = self.masks.get(&mask_key) {
            cached.clone()
        } else {
            let Some(rasterized) =
                platform_rasterize_symbol(name, size, weight, f64::from(scale_f32))
            else {
                waterui_core::contract_violation!(
                    "Unknown SF Symbol name: {name}; the view draws no symbol"
                );
                return None;
            };
            self.sizes.put(
                MeasureKey {
                    name,
                    size_bits: size.to_bits(),
                    weight: weight_discriminant(weight),
                },
                rasterized.point_size,
            );
            self.masks.put(mask_key, rasterized.clone());
            rasterized
        };

        let mut pixels = Vec::with_capacity(mask.alpha.len() * 4);
        let [r, g, b, fg_a] = rgba;
        for &cov in mask.alpha.iter() {
            // Rounded 8-bit coverage product: (cov * fg_a + 127) / 255 is in 0..=255.
            let out_a = u8::try_from((u16::from(cov) * u16::from(fg_a) + 127) / 255)
                .expect("rounded 8-bit alpha product fits in u8");
            if out_a == 0 {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                pixels.extend_from_slice(&[r, g, b, out_a]);
            }
        }
        let image = ImageData::<Rgba8>::new(mask.width, mask.height, Arc::<[u8]>::from(pixels))
            .expect("SF Symbol alpha mask length matches width * height * 4")
            .color_space(ImageColorSpace::DisplayP3);
        let image_id = self.next_image_id;
        self.next_image_id = self.next_image_id.wrapping_add(1).max(1);
        let entry = (image_id, mask.point_size, image);
        self.tinted.put(tinted_key, entry.clone());
        Some(entry)
    }
}

fn normalized_point_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 {
        size
    } else {
        f64_as_f32(platform_system_font_size())
    }
}

const fn weight_discriminant(weight: FontWeight) -> u8 {
    match weight {
        FontWeight::Thin => 0,
        FontWeight::UltraLight => 1,
        FontWeight::Light => 2,
        FontWeight::Normal => 3,
        FontWeight::Medium => 4,
        FontWeight::SemiBold => 5,
        FontWeight::Bold => 6,
        FontWeight::UltraBold => 7,
        FontWeight::Black => 8,
    }
}

/// Platform `NSFontWeight` / `UIFontWeight` scale matching `cocoa-ui`.
const fn apple_font_weight(weight: FontWeight) -> f64 {
    match weight {
        FontWeight::Thin => -0.6,
        FontWeight::UltraLight => -0.8,
        FontWeight::Light => -0.4,
        FontWeight::Normal => 0.0,
        FontWeight::Medium => 0.23,
        FontWeight::SemiBold => 0.3,
        FontWeight::Bold => 0.4,
        FontWeight::UltraBold => 0.56,
        FontWeight::Black => 0.62,
    }
}

/// Resolves the effective weight of `font`, matching `waterui-apple`'s
/// `face_weight` rule for bold text styles.
const fn effective_font_weight(resolved: &ResolvedFont) -> FontWeight {
    let style_bold = if cfg!(target_os = "macos") && resolved.unstyled() {
        None
    } else {
        resolved.bold_weight()
    };
    match (resolved.bold(), style_bold) {
        (true, Some(declared)) => declared,
        (true, None) => FontWeight::Bold,
        (false, _) => resolved.weight(),
    }
}

/// Resolves the `Body` font slot overridden by `IconSize` when installed,
/// matching `waterui-apple`'s `realize_sf_symbol`.
fn symbol_font_signal(env: &Environment) -> Computed<ResolvedFont> {
    let body = Body.resolve(env);
    env.get::<IconSize>().map_or_else(
        || body.computed(),
        |icon_size| {
            body.zip(icon_size.size())
                .map(|(mut font, size)| {
                    font.set_size(size);
                    font
                })
                .computed()
        },
    )
}

/// IEC 61966-2-1 sRGB / Display P3 opto-electronic transfer function.
fn linear_to_transfer_u8(linear: f32) -> u8 {
    let clamped = if linear.is_finite() {
        linear.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let encoded = if clamped <= 0.003_130_8 {
        12.92 * clamped
    } else {
        1.055_f32.mul_add(clamped.powf(1.0 / 2.4), -0.055)
    };
    f32_as_u8((encoded.clamp(0.0, 1.0) * 255.0).round())
}

/// Encodes linear Display P3 [`WorkingColor`] into straight-alpha Display P3
/// RGBA8 for `ImageColorSpace::DisplayP3`.
fn working_color_to_display_p3_rgba8(color: WorkingColor) -> [u8; 4] {
    let [r, g, b, a] = color.components;
    let alpha = if a.is_finite() {
        a.clamp(0.0, 1.0)
    } else {
        0.0
    };
    [
        linear_to_transfer_u8(r),
        linear_to_transfer_u8(g),
        linear_to_transfer_u8(b),
        f32_as_u8((alpha * 255.0).round()),
    ]
}

/// Uniformly scales `(width, height)` to fit inside `bounds` and centers it.
fn aspect_fit_centered(bounds: Rect, width: f64, height: f64) -> Rect {
    if width <= 0.0 || height <= 0.0 || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Rect::ZERO;
    }
    let fit = (bounds.width() / width).min(bounds.height() / height);
    let draw_w = width * fit;
    let draw_h = height * fit;
    let x0 = (bounds.width() - draw_w).mul_add(0.5, bounds.x0);
    let y0 = (bounds.height() - draw_h).mul_add(0.5, bounds.y0);
    Rect::new(x0, y0, x0 + draw_w, y0 + draw_h)
}

fn rgba_bitmap_context(
    pixels: &mut [u8],
    width: usize,
    height: usize,
) -> Option<CFRetained<CGContext>> {
    let bytes_per_row = width.checked_mul(4)?;
    if width == 0 || height == 0 || pixels.len() != bytes_per_row.checked_mul(height)? {
        return None;
    }
    let space = CGColorSpace::new_device_rgb()?;
    // SAFETY: `pixels` stays alive for the lifetime of the returned context and
    // holds `width * height * 4` premultiplied RGBA8 bytes matching `space`.
    let ptr = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            bytes_per_row,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    };
    // SAFETY: CoreGraphics returns a +1 retained context or NULL.
    Some(unsafe { CFRetained::from_raw(core::ptr::NonNull::new(ptr)?) })
}

fn extract_alpha_channel(rgba: &[u8]) -> Arc<[u8]> {
    Arc::from(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .map(|px| px[3])
            .collect::<Vec<u8>>(),
    )
}

#[cfg(target_os = "macos")]
fn platform_system_font_size() -> f64 {
    objc2_app_kit::NSFont::systemFontSize()
}

#[cfg(target_os = "ios")]
fn platform_system_font_size() -> f64 {
    objc2_ui_kit::UIFont::systemFontSize()
}

#[cfg(target_os = "macos")]
fn with_macos_symbol<R>(
    name: &str,
    point_size: f32,
    weight: FontWeight,
    f: impl FnOnce(&objc2_app_kit::NSImage) -> Option<R>,
) -> Option<R> {
    use objc2_app_kit::{NSImage, NSImageSymbolConfiguration};

    let symbol_name = NSString::from_str(name);
    let base = NSImage::imageWithSystemSymbolName_accessibilityDescription(&symbol_name, None)?;
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(
        f64::from(point_size),
        apple_font_weight(weight),
    );
    let configured = base.imageWithSymbolConfiguration(&config);
    f(configured.as_deref().unwrap_or(&base))
}

#[cfg(target_os = "macos")]
fn platform_measure_symbol(name: &str, point_size: f32, weight: FontWeight) -> Option<LayoutSize> {
    with_macos_symbol(name, point_size, weight, |image| {
        let size = image.size();
        Some(LayoutSize::new(
            f64_as_f32(size.width.max(0.0)),
            f64_as_f32(size.height.max(0.0)),
        ))
    })
}

#[cfg(target_os = "macos")]
fn platform_rasterize_symbol(
    name: &str,
    point_size: f32,
    weight: FontWeight,
    scale: f64,
) -> Option<CachedMask> {
    use objc2_app_kit::{NSCompositingOperation, NSGraphicsContext};

    with_macos_symbol(name, point_size, weight, |image| {
        let pt_size = image.size();
        if pt_size.width <= 0.0 || pt_size.height <= 0.0 {
            return None;
        }
        let width_u32 = f64_as_u32((pt_size.width * scale).ceil().max(1.0));
        let height_u32 = f64_as_u32((pt_size.height * scale).ceil().max(1.0));
        let width = usize::try_from(width_u32).ok()?;
        let height = usize::try_from(height_u32).ok()?;
        let mut rgba = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
        {
            let cg_ctx = rgba_bitmap_context(&mut rgba, width, height)?;
            let scale_x = f64::from(width_u32) / pt_size.width;
            let scale_y = f64::from(height_u32) / pt_size.height;
            CGContext::scale_ctm(Some(&cg_ctx), scale_x, scale_y);

            let ns_ctx = NSGraphicsContext::graphicsContextWithCGContext_flipped(&cg_ctx, false);
            NSGraphicsContext::saveGraphicsState_class();
            NSGraphicsContext::setCurrentContext(Some(&ns_ctx));
            image.drawInRect_fromRect_operation_fraction(
                CGRect::new(CGPoint::ZERO, CGSize::new(pt_size.width, pt_size.height)),
                CGRect::ZERO,
                NSCompositingOperation::SourceOver,
                1.0,
            );
            NSGraphicsContext::restoreGraphicsState_class();
        }
        Some(CachedMask {
            point_size: LayoutSize::new(f64_as_f32(pt_size.width), f64_as_f32(pt_size.height)),
            width: width_u32,
            height: height_u32,
            alpha: extract_alpha_channel(&rgba),
        })
    })
}

#[cfg(target_os = "ios")]
fn with_ios_symbol<R>(
    name: &str,
    point_size: f32,
    weight: FontWeight,
    f: impl FnOnce(&objc2_ui_kit::UIImage) -> Option<R>,
) -> Option<R> {
    use objc2_ui_kit::{UIFont, UIImage, UIImageSymbolConfiguration};

    let font = UIFont::systemFontOfSize_weight(f64::from(point_size), apple_font_weight(weight));
    let config = UIImageSymbolConfiguration::configurationWithFont(&font);
    let symbol_name = NSString::from_str(name);
    let image = UIImage::systemImageNamed_withConfiguration(&symbol_name, Some(&config))?;
    f(&image)
}

#[cfg(target_os = "ios")]
fn platform_measure_symbol(name: &str, point_size: f32, weight: FontWeight) -> Option<LayoutSize> {
    with_ios_symbol(name, point_size, weight, |image| {
        let size = image.size();
        Some(LayoutSize::new(
            f64_as_f32(size.width.max(0.0)),
            f64_as_f32(size.height.max(0.0)),
        ))
    })
}

#[cfg(target_os = "ios")]
fn platform_rasterize_symbol(
    name: &str,
    point_size: f32,
    weight: FontWeight,
    scale: f64,
) -> Option<CachedMask> {
    use objc2_ui_kit::{UIGraphicsPopContext, UIGraphicsPushContext};

    with_ios_symbol(name, point_size, weight, |image| {
        let pt_size = image.size();
        if pt_size.width <= 0.0 || pt_size.height <= 0.0 {
            return None;
        }
        let width_u32 = f64_as_u32((pt_size.width * scale).ceil().max(1.0));
        let height_u32 = f64_as_u32((pt_size.height * scale).ceil().max(1.0));
        let width = usize::try_from(width_u32).ok()?;
        let height = usize::try_from(height_u32).ok()?;
        let mut rgba = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
        {
            let cg_ctx = rgba_bitmap_context(&mut rgba, width, height)?;
            let scale_x = f64::from(width_u32) / pt_size.width;
            let scale_y = f64::from(height_u32) / pt_size.height;
            // UIImage::drawInRect expects top-down UIKit coordinates, whereas a
            // fresh CGBitmapContext uses bottom-up CoreGraphics coordinates.
            CGContext::translate_ctm(Some(&cg_ctx), 0.0, f64::from(height_u32));
            CGContext::scale_ctm(Some(&cg_ctx), scale_x, -scale_y);

            UIGraphicsPushContext(&cg_ctx);
            image.drawInRect(CGRect::new(
                CGPoint::ZERO,
                CGSize::new(pt_size.width, pt_size.height),
            ));
            UIGraphicsPopContext();
        }
        Some(CachedMask {
            point_size: LayoutSize::new(f64_as_f32(pt_size.width), f64_as_f32(pt_size.height)),
            width: width_u32,
            height: height_u32,
            alpha: extract_alpha_channel(&rgba),
        })
    })
}

/// Measures an [`SfSymbol`] leaf from the host catalog at the active `Body`
/// font / [`IconSize`] and weight.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn measure_sf_symbol_node<E: crate::text::TextEngine>(
    symbol: &SfSymbol,
    _proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    _theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let font_signal = symbol_font_signal(env);
    let font = state.measure_signal(&font_signal);
    let size = state
        .sf_symbols
        .measure(symbol.name(), font.size(), effective_font_weight(&font));
    ViewDimensions::new(size)
}

/// Renders an [`SfSymbol`] leaf into the Cherenkov scene tinted by the live
/// `Foreground` color and emits image accessibility when labeled.
pub fn render_sf_symbol_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    symbol: &Rc<RefCell<SfSymbol>>,
    env: &Environment,
) {
    let render_ctx = ctx.render_context();
    crate::renderer::graphics_image_accessibility(
        ctx.renderer_mut(),
        Some(render_ctx),
        env,
        None,
        None,
    );

    let symbol = *symbol.borrow();
    let font_signal = symbol_font_signal(env);
    let font = ctx.renderer_mut().read_signal(&font_signal);
    let foreground_signal = color::Foreground.resolve(env).computed();
    let foreground = ctx.renderer_mut().read_signal(&foreground_signal);
    let scale = ctx.renderer_mut().display_scale();

    let Some((image_id, point_size, image)) = ctx.state_mut().sf_symbols.tinted_image(
        symbol.name(),
        font.size(),
        effective_font_weight(&font),
        scale,
        foreground,
    ) else {
        return;
    };

    let dest = aspect_fit_centered(
        ctx.bounds,
        f64::from(point_size.width),
        f64::from(point_size.height),
    );
    if dest.width() <= 0.0 || dest.height() <= 0.0 {
        return;
    }

    let local = ctx.local;
    ctx.renderer_mut()
        .scene_mut()
        .draw_image(local, dest, image_id, image, Sampling::Linear);
}

/// Emits an [`SfSymbol`] leaf's image accessibility node for the semantic walk.
#[cfg(feature = "accessibility")]
pub fn emit_sf_symbol_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    _symbol: &Rc<RefCell<SfSymbol>>,
    env: &Environment,
) {
    crate::renderer::graphics_image_accessibility(renderer, None, env, None, None);
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::renderer::tests::test_environment;
    use crate::runner::HeadlessRuntime;
    use crate::testing::TestStyle;
    use nami::binding;
    use waterui::ViewExt as _;
    use waterui_controls::label::label;
    use waterui_core::AnyView;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_core::id::SelfId;
    use waterui_core::views::ForEach;
    use waterui_graphics::color::Color;
    use waterui_navigation::tab::{Tab, Tabs};
    use waterui_text::text;

    #[test]
    fn sf_symbol_measures_and_rasterizes_right_side_up() {
        let mut cache = SfSymbolCache::new();
        for symbol in [SfSymbol::HOUSE_FILL, SfSymbol::MAGNIFYINGGLASS] {
            let size = cache.measure(symbol.name(), 17.0, FontWeight::Normal);
            assert!(
                size.width > 8.0 && size.height > 8.0,
                "{} must measure non-zero logical dimensions, got {size:?}",
                symbol.name()
            );
        }

        // Verify orientation using an asymmetric upward vs downward arrow:
        // `arrow.up`'s head sits in the top half, `arrow.down`'s head in the bottom half.
        let up = platform_rasterize_symbol(SfSymbol::ARROW_UP.name(), 24.0, FontWeight::Bold, 2.0)
            .expect("arrow.up rasterizes");
        let down =
            platform_rasterize_symbol(SfSymbol::ARROW_DOWN.name(), 24.0, FontWeight::Bold, 2.0)
                .expect("arrow.down rasterizes");

        let half_mass = |mask: &CachedMask| -> (u64, u64) {
            let w = mask.width as usize;
            let mid = (mask.height as usize) / 2;
            let top: u64 = mask.alpha[..mid * w].iter().map(|&v| u64::from(v)).sum();
            let bottom: u64 = mask.alpha[mid * w..].iter().map(|&v| u64::from(v)).sum();
            (top, bottom)
        };
        let (up_top, up_bottom) = half_mass(&up);
        let (down_top, down_bottom) = half_mass(&down);
        assert!(
            up_top > up_bottom,
            "arrow.up must have more ink in top half than bottom half ({up_top} vs {up_bottom})"
        );
        assert!(
            down_bottom > down_top,
            "arrow.down must have more ink in bottom half than top half ({down_bottom} vs {down_top})"
        );
    }

    #[test]
    fn sf_symbol_renders_tinted_pixels_and_tracks_tab_foreground() {
        let env = test_environment();
        let mut runtime = HeadlessRuntime::new_for_tests(
            env,
            AnyViewBuilder::<AnyView>::new(|| {
                AnyView::new(SfSymbol::HOUSE_FILL.foreground(Color::srgb(220, 30, 30)))
            }),
            64,
            64,
            TestStyle::default(),
        )
        .with_scale_factor(2.0);
        let snap = runtime
            .pump_snapshot()
            .into_snapshot()
            .expect("tinted SF Symbol snapshot");
        let red_pixels = snap
            .rgba8()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] > 180 && px[1] < 80 && px[2] < 80)
            .count();
        assert!(
            red_pixels > 40,
            "tinted SfSymbol::HOUSE_FILL must draw non-blank red pixels, got {red_pixels}"
        );

        let selection = binding(0_u8);
        let selection_for_view = selection.clone();
        let env = test_environment();
        let mut tab_runtime = HeadlessRuntime::new_for_tests(
            env,
            AnyViewBuilder::<AnyView>::new(move || {
                let items = vec![SelfId::new(0_u8), SelfId::new(1_u8)];
                AnyView::new(Tabs::new(
                    &selection_for_view,
                    ForEach::new(items, |id: SelfId<u8>| {
                        let (title, symbol) = if id.into_inner() == 0 {
                            ("Home", SfSymbol::HOUSE_FILL)
                        } else {
                            ("Search", SfSymbol::MAGNIFYINGGLASS)
                        };
                        Tab::new(label(title).icon(symbol), move || text(title))
                    }),
                ))
            }),
            320,
            240,
            TestStyle::default(),
        )
        .with_scale_factor(2.0);

        let before = tab_runtime
            .pump_snapshot()
            .into_snapshot()
            .expect("initial tab snapshot");
        selection.set(1);
        let after = tab_runtime
            .pump_snapshot()
            .into_snapshot()
            .expect("switched tab snapshot");
        assert_ne!(
            before.rgba8(),
            after.rgba8(),
            "switching selected tab must repaint SF Symbol tab tinting"
        );
    }

    #[test]
    fn sf_symbol_accessibility_is_decorative_by_default_and_exposes_label_when_set() {
        let env = test_environment();
        let mut hidden_rt = HeadlessRuntime::new_for_tests(
            env,
            AnyViewBuilder::<AnyView>::new(|| AnyView::new(SfSymbol::STAR_FILL)),
            64,
            64,
            TestStyle::default(),
        );
        let hidden_update = hidden_rt
            .pump_snapshot()
            .into_tree_update()
            .expect("accessibility tree update");
        let unlabeled_images = hidden_update
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == accesskit::Role::Image)
            .count();
        assert_eq!(
            unlabeled_images, 0,
            "unlabeled SfSymbol must stay decorative in accessibility tree"
        );

        let env = test_environment();
        let mut labeled_rt = HeadlessRuntime::new_for_tests(
            env,
            AnyViewBuilder::<AnyView>::new(|| {
                AnyView::new(SfSymbol::STAR_FILL.a11y_label("Favourite"))
            }),
            64,
            64,
            TestStyle::default(),
        );
        let labeled_update = labeled_rt
            .pump_snapshot()
            .into_tree_update()
            .expect("accessibility tree update");
        let has_favourite = labeled_update.nodes.iter().any(|(_, node)| {
            node.role() == accesskit::Role::Image && node.label() == Some("Favourite")
        });
        assert!(
            has_favourite,
            "labeled SfSymbol must emit an Image node with its accessibility label"
        );
    }
}
