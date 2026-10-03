//! The platform theme in the environment.
//!
//! The activity's `Configuration` and `Resources.Theme` are the source of
//! truth: the scheme `uiMode` reports, resolved per semantic slot into
//! concrete colors, and the default type scale as concrete fonts. Every
//! slot is a `Binding` behind a `Computed`, so a configuration change —
//! the user toggling dark mode — lands as a precise per-slot update,
//! exactly the way the Apple backend's `ThemeSignals` fed its signals.

use alloc::boxed::Box;
use alloc::vec::Vec;

use jni::Env;
use waterui::graphics::color::{self as colors, WorkingColor, srgb_to_linear, working};
use waterui::reactive::{Binding, SignalExt};
use waterui::text::font::{self, FontSlot};
use waterui::theme::{self, color};
use waterui_backend_core::Environment;

use crate::jvm::globals;

/// `android.R.attr.colorBackground` — the window's themed background.
const ATTR_COLOR_BACKGROUND: i32 = 0x0101_0031;
/// `android.R.attr.textColorPrimary` — the themed primary text.
const ATTR_TEXT_PRIMARY: i32 = 0x0101_0036;
/// `android.R.attr.textColorSecondary` — the themed secondary text.
const ATTR_TEXT_SECONDARY: i32 = 0x0101_0038;
/// `android.R.attr.textColorPrimaryInverse` — text that reads on the dark
/// material, the on-accent/on-error answer.
const ATTR_TEXT_PRIMARY_INVERSE: i32 = 0x0101_0287;
/// `android.R.attr.colorPrimary` — the themed accent.
const ATTR_COLOR_PRIMARY: i32 = 0x0101_0433;
/// `android.R.attr.colorPrimaryDark` — the accent's container pair.
const ATTR_COLOR_PRIMARY_DARK: i32 = 0x0101_0434;
/// `android.R.attr.colorControlNormal` — the themed divider/icon tint.
const ATTR_CONTROL_NORMAL: i32 = 0x0101_0436;
/// `android.R.attr.colorControlHighlight` — the themed pressed/selected
/// fill, the surface-variant answer.
const ATTR_CONTROL_HIGHLIGHT: i32 = 0x0101_0437;
/// `android.R.attr.colorSecondary` — the themed complementary accent.
const ATTR_COLOR_SECONDARY: i32 = 0x0101_0530;
/// `android.R.attr.colorError` — the themed destructive emphasis.
const ATTR_COLOR_ERROR: i32 = 0x0101_0543;

/// `Configuration.UI_MODE_NIGHT_MASK` and `UI_MODE_NIGHT_YES`.
const UI_MODE_NIGHT_MASK: i32 = 0x30;
const UI_MODE_NIGHT_YES: i32 = 0x20;

/// The signals the platform theme is fed through. Keep the returned value
/// alive; every slot's `Computed` also holds its `Binding`, so nothing but
/// [`refresh`] needs to reach it.
pub(crate) struct ThemeSignals {
    scheme: Binding<colors::ColorScheme>,
    colors: Vec<Box<dyn Fn(&mut Env)>>,
}

/// Installs the color-scheme signal and every color and font slot, reading
/// the platform's current appearance. The runtime feeds [`refresh`] each
/// time `onConfigurationChanged` fires.
pub(crate) fn install(env: &mut Environment) -> ThemeSignals {
    let scheme = waterui::reactive::binding(colors::ColorScheme::Light);
    theme::install_color_scheme(env, scheme.computed());

    let mut signals = ThemeSignals {
        scheme,
        colors: Vec::new(),
    };

    // The slot table the framework attrs answer: each semantic slot reads a
    // concrete theme attribute in the current configuration.
    macro_rules! colors {
        ($( $slot:ident => $attr:expr ),* $(,)?) => {
            $( signals.install_color::<color::$slot>(env, $attr); )*
        };
    }
    colors! {
        Background => ATTR_COLOR_BACKGROUND,
        // Framework attrs name no surface family; the window background is
        // the closest canonical answer.
        Surface => ATTR_COLOR_BACKGROUND,
        SurfaceVariant => ATTR_CONTROL_HIGHLIGHT,
        Border => ATTR_CONTROL_NORMAL,
        Foreground => ATTR_TEXT_PRIMARY,
        MutedForeground => ATTR_TEXT_SECONDARY,
        Accent => ATTR_COLOR_PRIMARY,
        AccentContainer => ATTR_COLOR_PRIMARY_DARK,
        AccentForeground => ATTR_TEXT_PRIMARY_INVERSE,
        Tertiary => ATTR_COLOR_SECONDARY,
        TertiaryContainer => ATTR_COLOR_SECONDARY,
        SelectionContainer => ATTR_CONTROL_HIGHLIGHT,
        SelectionForeground => ATTR_TEXT_PRIMARY,
        Error => ATTR_COLOR_ERROR,
        ErrorForeground => ATTR_TEXT_PRIMARY_INVERSE,
    }

    macro_rules! fonts {
        ($( $slot:ident ),* $(,)?) => {
            $( theme::install_font_signal::<font::$slot>(
                env,
                waterui::reactive::Computed::constant(font::$slot::DEFAULT),
            ); )*
        };
    }
    // Framework attrs name no text-appearance metrics; the default type
    // scale is the install, and `setTextSize` already applies the user's
    // font scale on the platform side.
    fonts! {
        Body, Title, Headline, Subheadline, Caption, Footnote,
    }

    refresh(&signals);
    signals
}

/// Re-reads the platform and pushes every changed slot value. Called from
/// `nativeOnConfigurationChanged`.
pub(crate) fn refresh(signals: &ThemeSignals) {
    crate::jvm::with_env(|env| {
        let night = globals()
            .bindings()
            .ui_mode(env)
            .map(|ui_mode| ui_mode & UI_MODE_NIGHT_MASK == UI_MODE_NIGHT_YES)
            .unwrap_or(false);
        signals.scheme.set(if night {
            colors::ColorScheme::Dark
        } else {
            colors::ColorScheme::Light
        });
        for set in &signals.colors {
            set(env);
        }
        crate::jvm::refresh_metrics(env).expect("DisplayMetrics are always readable");
    });
}

impl ThemeSignals {
    fn install_color<S: 'static>(
        &mut self,
        env: &mut Environment,
        attr: i32,
    ) {
        let binding = waterui::reactive::binding(WorkingColor::BLACK);
        theme::install_color_signal::<S>(env, binding.computed());
        self.colors.push(Box::new(move |env| {
            if let Ok(Some(argb)) = globals().bindings().theme_color(env, attr) {
                binding.set(argb_to_working(argb));
            }
        }));
    }
}

/// A packed ARGB `jint` becomes the wire `WorkingColor`: the platform's
/// sRGB channels are decoded and converted into the working space, with
/// alpha carried straight — the same conversion Apple's `into_working` ran
/// on `Rgba`.
fn argb_to_working(argb: i32) -> WorkingColor {
    let channel = |shift| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the channel is masked to its low byte"
        )]
        let byte = ((argb >> shift) & 0xff) as u8;
        srgb_to_linear(f32::from(byte) / 255.0)
    };
    let alpha = {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the alpha byte is masked to its low byte"
        )]
        let a = ((argb >> 24) & 0xff) as u8;
        f32::from(a) / 255.0
    };
    working::from_linear_srgb([channel(16), channel(8), channel(0)], alpha)
}

/// A `WorkingColor` back to a packed ARGB `jint`, the direction
/// `setBackgroundColor`-style setters take: working space → sRGB bytes →
/// the platform's `0xAARRGGBB` word.
pub(crate) fn working_to_argb(color: WorkingColor) -> i32 {
    let [red, green, blue] = working::to_linear_srgb(color);
    let byte = |c: f32| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to a byte range before the cast"
        )]
        let byte = (colors::linear_to_srgb(c).clamp(0.0, 1.0) * 255.0).round() as u8;
        i32::from(byte)
    };
    let alpha = {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to a byte range before the cast"
        )]
        let a = (color.components[3].clamp(0.0, 1.0) * 255.0).round() as u8;
        i32::from(a)
    };
    (alpha << 24) | (byte(red) << 16) | (byte(green) << 8) | byte(blue)
}
