//! The `image` leaf: `Native<SystemIcon>` rendered through the kit's image
//! view.
//!
//! Mirrors `WuiSystemIcon`: the `SystemIcon`'s `name` resolves an `SF
//! Symbol` once (a missing name is a fatal authoring error, as in Swift),
//! the theme `Foreground` slot drives the tint, and the theme `Body` font
//! slot re-renders the symbol at the themed type size — `preferredSymbol-
//! Configuration` on `UIKit`, a fresh configured `NSImage` on `AppKit`.
//! Every update is an imperative kit call inside a watcher; no signal type
//! crosses into `cocoa-ui`.

use cocoa_ui::Retained;
use waterui::graphics::color::WorkingColor;
use waterui::icon::SystemIcon;
use waterui::reactive::Signal;
use waterui::resolve::Resolvable;
use waterui::text::font::{Body, FontWeight, ResolvedFont};
use waterui::theme::color::Foreground;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::ImageView;
    pub(super) use cocoa_ui::appkit::colors;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::ImageView;
    pub(super) use cocoa_ui::uikit::colors;
}

use cocoa_ui::PlatformView;
use platform::ImageView;

/// The image view as its platform view, for the leaf and layout helpers.
fn as_view(image: &ImageView) -> &PlatformView {
    image
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_ui_kit::UIColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object, with HDR
/// headroom applied as a content-headroom multiplier — the `AppKit` variant.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_app_kit::NSColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The platform weight of a `FontWeight` on the `UIFont`/`NSFont` scale —
/// the same table `WuiFontWeight.toNSFontWeight()` applies.
const fn platform_weight(weight: FontWeight) -> f64 {
    use cocoa_ui::font::weight;
    match weight {
        FontWeight::Thin => weight::THIN,
        FontWeight::UltraLight => weight::ULTRA_LIGHT,
        FontWeight::Light => weight::LIGHT,
        FontWeight::Normal => weight::REGULAR,
        FontWeight::Medium => weight::MEDIUM,
        FontWeight::SemiBold => weight::SEMI_BOLD,
        FontWeight::Bold => weight::BOLD,
        FontWeight::UltraBold => weight::HEAVY,
        FontWeight::Black => weight::BLACK,
    }
}

/// Pushes the themed font onto the symbol: `preferredSymbolConfiguration`
/// with a font-derived configuration on `UIKit`.
#[cfg(target_os = "ios")]
fn apply_symbol_font(mtm: cocoa_ui::MainThreadMarker, view: &ImageView, font: &ResolvedFont) {
    let font = cocoa_ui::font::system(mtm, f64::from(font.size), platform_weight(font.weight));
    assert!(
        view.set_symbol_font(&font),
        "SF Symbol rejected the themed font configuration"
    );
}

/// Pushes the themed font onto the symbol: a point-size/weight symbol
/// configuration rebuilt from the symbol's name on `AppKit`.
#[cfg(target_os = "macos")]
fn apply_symbol_font(_mtm: cocoa_ui::MainThreadMarker, view: &ImageView, font: &ResolvedFont) {
    assert!(
        view.set_symbol_configuration(f64::from(font.size), platform_weight(font.weight)),
        "SF Symbol rejected the themed font configuration"
    );
}

/// The image view's layout face: intrinsic, non-stretching.
struct ImageSubView {
    /// The image view whose current image the measure reports.
    view: Retained<ImageView>,
}

impl core::fmt::Debug for ImageSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageSubView").finish_non_exhaustive()
    }
}

impl SubView for ImageSubView {
    // The platform measures in f64; `ViewDimensions` speaks f32 — the
    // narrowing is the layout contract.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        let size = self
            .view
            .image_size()
            .expect("SF Symbol has no intrinsic size");
        ViewDimensions::new(Size::new(size.width as f32, size.height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `image` handler on the dispatcher: `Native<SystemIcon>` maps
/// to a kit image view drawing the named `SF Symbol`, tinted by the theme
/// foreground and sized by the theme body font.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<SystemIcon>(|config, ctx| {
        let mtm = ctx.mtm();
        let view = ImageView::new(mtm);
        view.set_scale_mode(cocoa_ui::ScaleMode::Fit);

        let name = config.name.as_str();
        assert!(
            view.set_system_symbol(name),
            "Unknown SF Symbol name: {name}"
        );

        let mut leaf = NativeLeaf::new(as_view(&view), ImageSubView { view: view.clone() });

        // The theme foreground tints the symbol; the theme body font
        // re-renders it at the themed type size — both push imperatively,
        // exactly as the Swift leaf's two observations did. Watchers only
        // fire on change, so the current values are applied eagerly.
        let foreground = Foreground.resolve(ctx.env());
        view.set_tint_color(Some(&platform_color(&foreground.snapshot())));
        leaf.bind(&foreground, {
            let view = view.clone();
            move |color| view.set_tint_color(Some(&platform_color(&color)))
        });
        let body = Body.resolve(ctx.env());
        apply_symbol_font(mtm, &view, &body.snapshot());
        leaf.bind(&body, move |font| apply_symbol_font(mtm, &view, &font));
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_weight_maps_every_font_weight() {
        use cocoa_ui::font::weight;
        let table: [(FontWeight, f64); 9] = [
            (FontWeight::Thin, weight::THIN),
            (FontWeight::UltraLight, weight::ULTRA_LIGHT),
            (FontWeight::Light, weight::LIGHT),
            (FontWeight::Normal, weight::REGULAR),
            (FontWeight::Medium, weight::MEDIUM),
            (FontWeight::SemiBold, weight::SEMI_BOLD),
            (FontWeight::Bold, weight::BOLD),
            (FontWeight::UltraBold, weight::HEAVY),
            (FontWeight::Black, weight::BLACK),
        ];
        for (weight, expected) in table {
            assert_eq!(platform_weight(weight).to_bits(), expected.to_bits());
        }
    }
}
