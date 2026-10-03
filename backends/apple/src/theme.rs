//! The platform theme in the environment.
//!
//! The AppKit/UIKit appearance is the source of truth: the scheme the
//! application currently draws in, resolved per semantic slot into concrete
//! colors, and the platform's preferred text styles as concrete fonts.
//! Every slot is a `Binding` behind a `Computed`, so a platform change —
//! the user toggling dark mode, or an override the app applies — lands as a
//! precise per-slot update, exactly the way the Swift `ThemeBridge` fed its
//! signals.

use alloc::boxed::Box;
use alloc::vec::Vec;

use cocoa_ui::ColorScheme;
use waterui::graphics::color::{
    WorkingColor, srgb_to_linear,
    working::{self},
};
use waterui::reactive::{Binding, SignalExt};
use waterui::text::font::{self, FontWeight, ResolvedFont};
use waterui::theme::{self, color};
use waterui_backend_core::Environment;

/// The signals the platform theme is fed through. Keep the returned value
/// alive; every slot's `Computed` also holds its `Binding`, so nothing but
/// [`refresh`] needs to reach it.
pub struct ThemeSignals {
    scheme: Binding<waterui::graphics::color::ColorScheme>,
    colors: Vec<Box<dyn Fn(ColorScheme)>>,
    fonts: Vec<Box<dyn Fn()>>,
}

/// Installs and observes the theme owned by a mounted `UIKit` controller.
#[cfg(target_os = "ios")]
pub fn install_controller(
    env: &mut Environment,
    controller: &cocoa_ui::uikit::ViewController,
    keepalive: &mut crate::contract::KeepAlive,
) {
    let theme = alloc::rc::Rc::new(install(env, controller.color_scheme()));
    let observed = theme.clone();
    keepalive.keep(controller.observe_color_scheme(move |scheme| refresh(&observed, scheme)));
    keepalive.keep(theme);
}

/// Installs the color-scheme signal and every color and font slot, reading
/// the platform's current appearance. The app feeds [`refresh`] each time
/// the platform appearance changes.
pub fn install(env: &mut Environment, scheme: ColorScheme) -> ThemeSignals {
    let scheme_binding = waterui::reactive::binding(color_scheme(scheme));
    theme::install_color_scheme(env, scheme_binding.computed());

    let mut signals = ThemeSignals {
        scheme: scheme_binding,
        colors: Vec::new(),
        fonts: Vec::new(),
    };

    // The slot table the Swift ThemeBridge published: each semantic slot
    // reads a concrete platform color in the current scheme.
    macro_rules! colors {
        ($( $slot:ident => $resolver:expr ),* $(,)?) => {
            $( signals.install_color::<color::$slot>(env, $resolver); )*
        };
    }
    #[cfg(target_os = "macos")]
    {
        use cocoa_ui::appkit::colors::{self as colors, AppColor as P};
        colors! {
            Background => |s| colors::resolve(P::WindowBackground, s),
            Surface => |s| colors::resolve(P::ControlBackground, s),
            SurfaceVariant => |s| colors::resolve(P::TertiarySystemFill, s),
            Border => |s| colors::resolve(P::Separator, s),
            Foreground => |s| colors::resolve(P::Label, s),
            MutedForeground => |s| colors::resolve(P::SecondaryLabel, s),
            Accent => |s| colors::resolve(P::ControlAccent, s),
            // The semantic "text on accent" color: flips to black under accent
            // colors and contrast settings where a constant white would fail.
            AccentForeground => |s| colors::resolve(P::AlternateSelectedControlText, s),
            AccentContainer => |s| colors::resolve(P::ControlAccent, s)
                .map(|c| c.with_alpha(0.16)),
            Tertiary => |s| colors::resolve(P::SystemPurple, s),
            TertiaryContainer => |s| colors::resolve(P::SystemPurple, s)
                .map(|c| c.with_alpha(0.16)),
            // AppKit's own emphasized selection fill, the color a focused table
            // paints behind a selected row.
            SelectionContainer => |s| colors::resolve(P::SelectedContentBackground, s),
            SelectionForeground => |s| colors::resolve(P::AlternateSelectedControlText, s),
            // White on systemRed is what AppKit draws for destructive fills.
            Error => |s| colors::resolve(P::SystemRed, s),
            ErrorForeground => |s| colors::resolve(P::White, s),
        }
    }
    #[cfg(target_os = "ios")]
    {
        use cocoa_ui::uikit::colors::{self as colors, UiColor as P};
        colors! {
            Background => |s| Some(colors::resolve(P::SystemBackground, s)),
            Surface => |s| Some(colors::resolve(P::SecondarySystemBackground, s)),
            // `tertiarySystemBackground` is pure white in light mode —
            // identical to the Background slot — so an "alternate surface"
            // filled with it is invisible. The system fill is the color
            // intended for input fields and shape fills.
            SurfaceVariant => |s| Some(colors::resolve(P::TertiarySystemFill, s)),
            Border => |s| Some(colors::resolve(P::Separator, s)),
            Foreground => |s| Some(colors::resolve(P::Label, s)),
            MutedForeground => |s| Some(colors::resolve(P::SecondaryLabel, s)),
            Accent => |s| Some(colors::resolve(P::Accent, s)),
            AccentForeground => |s| Some(colors::resolve(P::White, s)),
            AccentContainer => |s| Some(colors::resolve(P::Accent, s).with_alpha(0.16)),
            Tertiary => |s| Some(colors::resolve(P::SystemPurple, s)),
            TertiaryContainer => |s| Some(colors::resolve(P::SystemPurple, s)
                .with_alpha(0.16)),
            // The selection fill is the app's accent, and its content is drawn
            // in the same on-accent color the accent pair uses.
            SelectionContainer => |s| Some(colors::resolve(P::Accent, s)),
            SelectionForeground => |s| Some(colors::resolve(P::White, s)),
            // White on systemRed is what UIKit draws for destructive fills.
            Error => |s| Some(colors::resolve(P::SystemRed, s)),
            ErrorForeground => |s| Some(colors::resolve(P::White, s)),
        }
    }

    macro_rules! fonts {
        ($( $slot:ident => $style:ident ),* $(,)?) => {
            $( signals.install_font::<font::$slot>(
                env,
                cocoa_ui::system_font::TextStyle::$style,
            ); )*
        };
    }
    fonts! {
        Body => Body,
        Title => Title1,
        Headline => Headline,
        Subheadline => Subheadline,
        Caption => Caption1,
        Footnote => Footnote,
    }

    refresh(&signals, scheme);
    signals
}

/// Re-reads the platform for `scheme` and pushes every changed slot value.
/// Call it from each surface whose platform reports an appearance change.
pub fn refresh(signals: &ThemeSignals, scheme: ColorScheme) {
    signals.scheme.set(color_scheme(scheme));
    for set in &signals.colors {
        set(scheme);
    }
    for set in &signals.fonts {
        set();
    }
}

impl ThemeSignals {
    fn install_color<S: 'static>(
        &mut self,
        env: &mut Environment,
        resolver: impl Fn(ColorScheme) -> Option<cocoa_ui::Rgba> + 'static,
    ) {
        let binding = waterui::reactive::binding(working::from_linear_srgb([0.0, 0.0, 0.0], 1.0));
        theme::install_color_signal::<S>(env, binding.computed());
        self.colors.push(Box::new(move |scheme| {
            if let Some(rgba) = resolver(scheme) {
                binding.set(into_working(rgba));
            }
        }));
    }

    fn install_font<F: 'static>(
        &mut self,
        env: &mut Environment,
        style: cocoa_ui::system_font::TextStyle,
    ) {
        let binding = waterui::reactive::binding(ResolvedFont::new(0.0, FontWeight::Normal));
        theme::install_font_signal::<F>(env, binding.computed());
        self.fonts.push(Box::new(move || {
            let metrics = cocoa_ui::system_font::preferred_font(style);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "platform metrics are f64; ResolvedFont is f32"
            )]
            let font = ResolvedFont::new(metrics.size as f32, font_weight(metrics.weight))
                .with_typography_metrics(metrics.line_height as f32, 0.0);
            binding.set(font);
        }));
    }
}

/// The waterui color scheme a platform scheme projects to.
const fn color_scheme(scheme: ColorScheme) -> waterui::graphics::color::ColorScheme {
    match scheme {
        ColorScheme::Light => waterui::graphics::color::ColorScheme::Light,
        ColorScheme::Dark => waterui::graphics::color::ColorScheme::Dark,
    }
}

/// A resolved platform color becomes the wire `WorkingColor`: the platform's
/// sRGB channels are decoded and converted into linear Display-P3, with alpha
/// carried straight.
#[expect(
    clippy::cast_possible_truncation,
    reason = "platform color components are f64; WorkingColor is f32"
)]
fn into_working(rgba: cocoa_ui::Rgba) -> WorkingColor {
    working::from_linear_srgb(
        [
            srgb_to_linear(rgba.red as f32),
            srgb_to_linear(rgba.green as f32),
            srgb_to_linear(rgba.blue as f32),
        ],
        rgba.alpha as f32,
    )
}

/// The semantic weight a platform font's numeric weight expresses.
///
/// Platform weight constants are float32-rounded — semibold reads as
/// `0.30000001192092896` — so closed-interval bucketing on the decimal points
/// misclassifies every named weight that lands an epsilon above its boundary.
/// Snap to the nearest canonical weight instead.
fn font_weight(weight: f64) -> FontWeight {
    const CANONICAL: &[(f64, FontWeight)] = &[
        (-0.800_000_011_920_929, FontWeight::UltraLight),
        (-0.600_000_023_841_858, FontWeight::Thin),
        (-0.400_000_005_960_464_5, FontWeight::Light),
        (0.0, FontWeight::Normal),
        (0.230_000_004_172_325_1, FontWeight::Medium),
        (0.300_000_011_920_929, FontWeight::SemiBold),
        (0.400_000_005_960_464_5, FontWeight::Bold),
        (0.560_000_002_384_185_8, FontWeight::UltraBold),
        (0.620_000_004_768_371_6, FontWeight::Black),
    ];
    CANONICAL
        .iter()
        .min_by(|(a, _), (b, _)| {
            (a - weight)
                .abs()
                .partial_cmp(&(b - weight).abs())
                .unwrap_or(core::cmp::Ordering::Equal)
        })
        .map_or(FontWeight::Normal, |(_, weight)| *weight)
}

#[cfg(test)]
// The resolved values are exact constants the conversion table produces.
#[allow(clippy::float_cmp)]
mod tests {
    use cocoa_ui::Rgba;
    use waterui::graphics::color::ColorScheme as WuiColorScheme;

    use super::{color_scheme, font_weight, into_working};
    use waterui::text::font::FontWeight;

    /// Every canonical platform weight snaps to its named weight, including
    /// the float32-rounded constants `AppKit`/`UIKit` publish.
    #[test]
    fn font_weight_snaps_the_canonical_platform_weights() {
        let table: [(f64, FontWeight); 9] = [
            (-0.800_000_011_920_929, FontWeight::UltraLight),
            (-0.600_000_023_841_858, FontWeight::Thin),
            (-0.400_000_005_960_464_5, FontWeight::Light),
            (0.0, FontWeight::Normal),
            (0.230_000_004_172_325_1, FontWeight::Medium),
            (0.300_000_011_920_929, FontWeight::SemiBold),
            (0.400_000_005_960_464_5, FontWeight::Bold),
            (0.560_000_002_384_185_8, FontWeight::UltraBold),
            (0.620_000_004_768_371_6, FontWeight::Black),
        ];
        for (weight, expected) in table {
            assert_eq!(font_weight(weight), expected, "weight {weight}");
        }
    }

    /// Between two canonical weights the nearer one wins; the midpoint is a
    /// coin toss and a float64 system font reports a plain value like 0.28.
    #[test]
    fn font_weight_picks_the_nearer_canonical_neighbour() {
        assert_eq!(font_weight(0.20), FontWeight::Medium);
        assert_eq!(font_weight(0.28), FontWeight::SemiBold);
        assert_eq!(font_weight(0.37), FontWeight::Bold);
        // In-betweens must still answer a weight, never panic.
        for weight in [-1.0, -0.5, -0.2, 0.1, 0.45, 0.6, 1.0] {
            let _ = font_weight(weight);
        }
    }

    /// Out-of-range values clamp to the table's extremes, not `Normal`.
    #[test]
    fn font_weight_clamps_outside_the_table() {
        assert_eq!(font_weight(-2.0), FontWeight::UltraLight);
        assert_eq!(font_weight(5.0), FontWeight::Black);
    }

    #[test]
    fn color_scheme_maps_light_and_dark() {
        assert_eq!(
            color_scheme(cocoa_ui::ColorScheme::Light),
            WuiColorScheme::Light
        );
        assert_eq!(
            color_scheme(cocoa_ui::ColorScheme::Dark),
            WuiColorScheme::Dark
        );
    }

    /// The wire `WorkingColor` keeps the alpha straight; components are
    /// the platform's sRGB channels through the transfer function,
    /// narrowed to f32.
    #[test]
    fn into_working_carries_alpha_straight() {
        let working = into_working(Rgba::new(0.25, 0.5, 0.75, 0.4));
        assert_eq!(working.components[3], 0.4_f32);
        assert!(working.components[3] < 1.0);
        let opaque = into_working(Rgba::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(opaque.components[3], 1.0);
    }
}
