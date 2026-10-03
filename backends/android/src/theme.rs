//! The platform theme in the environment.
//!
//! The activity's `Configuration` and `Resources.Theme` are the source of
//! truth: the scheme `uiMode` reports, resolved per semantic slot into
//! concrete colors, and the default type scale as concrete fonts. Every
//! slot is a `Binding` behind a `Computed`, so a configuration change —
//! the user toggling dark mode — lands as a precise per-slot update,
//! exactly the way the Apple backend's `ThemeSignals` fed its signals.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

use jni::strings::JNIStr;
use jni::{Env, jni_str};
use waterui::graphics::color::{self as colors, WorkingColor, srgb_to_linear, working};
use waterui::reactive::{Binding, SignalExt};
use waterui::text::font::{self, FontSlot};
use waterui::theme::{self, color};
use waterui_backend_core::Environment;

use crate::jvm::Platform;

/// A color slot's platform push: resolves its theme attribute's color
/// and writes the binding it was installed on. The attribute's runtime
/// id and `android.R.attr` name are captured in the closure.
type SlotPush = Box<dyn Fn(&mut Env) -> jni::errors::Result<()>>;

/// The signals the platform theme is fed through. Keep the returned value
/// alive; every slot's `Computed` also holds its `Binding`, so nothing but
/// [`refresh`] needs to reach it. `platform` is the runtime's JNI surface,
/// held by `Rc` — [`refresh`] and every slot push read the theme through it.
pub struct ThemeSignals {
    scheme: Binding<colors::ColorScheme>,
    colors: Vec<SlotPush>,
    platform: Rc<Platform>,
}

impl core::fmt::Debug for ThemeSignals {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ThemeSignals")
            .field("slots", &self.colors.len())
            .finish_non_exhaustive()
    }
}

/// Installs the color-scheme signal and every color and font slot, reading
/// the platform's current appearance. The runtime feeds [`refresh`] each
/// time `onConfigurationChanged` fires.
///
/// # Errors
///
/// A pending `Resources.NotFoundException` naming the `android.R.attr`
/// field when a theme attribute the table reads does not exist or does
/// not resolve to a color — a slot that cannot be answered is a
/// startup failure, never a silent skip.
pub fn install(
    env: &mut Environment,
    jni_env: &mut Env,
    platform: &Rc<Platform>,
) -> jni::errors::Result<ThemeSignals> {
    let scheme = waterui::reactive::binding(colors::ColorScheme::Light);
    theme::install_color_scheme(env, scheme.computed());

    let mut signals = ThemeSignals {
        scheme,
        colors: Vec::new(),
        platform: platform.clone(),
    };

    // The slot table the framework attrs answer: each semantic slot reads
    // a concrete theme attribute in the current configuration, and the
    // attribute's id is resolved on `android.R.attr` by name — the ids
    // are resource ids assigned per release, never stable constants.
    macro_rules! colors {
        ($( $slot:ident => $attr:expr ),* $(,)?) => {
            $( signals.install_color::<color::$slot>(env, platform.framework_attr(jni_env, $attr)?, $attr); )*
        };
    }
    colors! {
        Background => jni_str!("colorBackground"),
        // Framework attrs name no surface family; the window background is
        // the closest canonical answer.
        Surface => jni_str!("colorBackground"),
        SurfaceVariant => jni_str!("colorControlHighlight"),
        Border => jni_str!("colorControlNormal"),
        Foreground => jni_str!("textColorPrimary"),
        MutedForeground => jni_str!("textColorSecondary"),
        Accent => jni_str!("colorAccent"),
        AccentContainer => jni_str!("colorPrimaryDark"),
        AccentForeground => jni_str!("textColorPrimaryInverse"),
        Tertiary => jni_str!("colorSecondary"),
        TertiaryContainer => jni_str!("colorSecondary"),
        SelectionContainer => jni_str!("colorControlHighlight"),
        SelectionForeground => jni_str!("textColorPrimary"),
        Error => jni_str!("colorError"),
        ErrorForeground => jni_str!("textColorPrimaryInverse"),
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

    refresh(jni_env, &signals)?;
    Ok(signals)
}

/// Re-reads the platform and pushes every changed slot value. Called
/// from `nativeOnConfigurationChanged` inside its `with_env` frame.
///
/// # Errors
///
/// Propagates the same `NotFoundException` install reports: a theme
/// attribute that stops resolving mid-run is still a named failure.
pub fn refresh(env: &mut Env, signals: &ThemeSignals) -> jni::errors::Result<()> {
    let bindings = signals.platform.bindings();
    let night = signals.platform.ui_mode(env)? & bindings.ui_mode_night_mask()
        == bindings.ui_mode_night_yes();
    signals.scheme.set(if night {
        colors::ColorScheme::Dark
    } else {
        colors::ColorScheme::Light
    });
    for set in &signals.colors {
        set(env)?;
    }
    signals.platform.refresh_metrics(env)
}

impl ThemeSignals {
    fn install_color<S: 'static>(
        &mut self,
        env: &mut Environment,
        attr: i32,
        attr_name: &'static JNIStr,
    ) {
        let binding = waterui::reactive::binding(WorkingColor::BLACK);
        theme::install_color_signal::<S>(env, binding.computed());
        let platform = self.platform.clone();
        self.colors.push(Box::new(move |env| {
            let argb = platform.theme_color(env, attr, attr_name)?;
            binding.set(argb_to_working(argb));
            Ok(())
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
            clippy::cast_sign_loss,
            reason = "the channel is masked to its low byte"
        )]
        let byte = ((argb >> shift) & 0xff) as u8;
        srgb_to_linear(f32::from(byte) / 255.0)
    };
    let alpha = {
        #[expect(
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
pub fn working_to_argb(color: WorkingColor) -> i32 {
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
