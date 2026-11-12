//! Hydrolysis's theme contract: what a style package implements, and how the
//! runtime assembles the environment's theme tokens.
//!
//! A style package (Material 3, a glass theme, a test theme) is a
//! [`Style`](crate::Style): a [`WidgetTheme`] plus a token installer. The
//! contract is everything a style package names from Hydrolysis:
//!
//! - [`WidgetTheme`], the widget chrome the renderer lays out and draws from:
//!   motion policies, per-widget metrics and the `draw_*` methods;
//! - the state the renderer hands a theme on every draw:
//!   [`WidgetInteractionState`] with its [`PressWaves`];
//! - the motion and metric token tables a theme returns, such as
//!   [`InteractionMotion`] and [`ListMetrics`];
//! - the looks a theme returns for the widgets ported to the declarative
//!   contract (water-rs/waterui#1434), such as [`ButtonLook`] for a
//!   [`ButtonSpec`], [`SwitchLook`] for a [`ToggleSpec`], [`StepperLook`]
//!   for a [`StepperSpec`], [`SegmentedLook`] for a [`SegmentedSpec`],
//!   [`MenuLook`] for a [`MenuSpec`], [`MenuPickerLook`] and
//!   [`RadioGroupLook`] for a [`PickerSpec`] and [`LinearProgressLook`] for a
//!   [`ProgressSpec`], [`NavigationBarLook`] for a [`NavigationSpec`],
//!   [`TabsLook`] for a [`TabsSpec`] and [`GlassLook`] for a
//!   `.background(Glass…)`, built from the look vocabulary: [`Driven`] values over
//!   the widget's [`Inputs`] with their [`Motion`], [`Custom`] parts,
//!   [`Label`] styling and [`HitRegion`]s;
//! - the engine-provided chrome primitives a theme describes interactive
//!   chrome with instead of drawing it — [`Surface`] with its [`Fill`] or
//!   [`Backdrop`], [`CornerShape`], [`Elevation`], [`StateLayer`],
//!   [`PressFeedback`] and [`FocusIndicator`], bundled per widget part as
//!   [`Chrome`] — which Hydrolysis records once and animates through live
//!   operands;
//! - the presentation plugins a style package installs on composed controls:
//!   [`Chrome`], [`InteractionFocusBinding`] and [`ModalInteraction`].
//!
//! Drawing goes through the render-target-neutral recording vocabulary of
//! `cherenkov-record` ([`Recorder`](cherenkov_record::Recorder),
//! [`Paint`](cherenkov_record::Paint),
//! [`MaterialRegistry`](cherenkov_record::MaterialRegistry)), never through a
//! render engine, so a theme renders on every target Hydrolysis draws
//! through. A style package depends on Hydrolysis, the framework crates whose
//! configuration types the contract names, and `cherenkov-record`.
//!
//! # Token assembly
//!
//! `install_theme_tokens` installs the runtime's theme tokens in precedence
//! order: framework defaults < [`Style::install_tokens`](crate::Style) < the
//! application's own environment. Every runner calls it — the rendered
//! runners with their style, the semantic runtime with `None` — so the same
//! precedence holds whether or not a style package is present.
//!
//! The framework defaults are *not* a theme: they carry no widget theme and
//! no presentation decisions beyond the framework's baseline palette and
//! type scale. A `Style` may overwrite any of them, and the application
//! overwrites both.

mod chrome;
mod glass;
#[cfg(hydrolysis_runtime)]
pub(crate) mod live;
mod look;
mod menu;
mod navigation;
mod picker;
mod progress;
mod segmented;
mod stepper;
mod tabs;
mod toggle;
mod widget;

pub use chrome::{
    Backdrop, Border, Chrome, CornerShape, Corners, Elevation, Fill, FocusIndicator, PressFeedback,
    Ripple, StateLayer, Surface, Wash,
};
pub use glass::GlassLook;
pub use look::{
    ButtonLook, ButtonSpec, Custom, DragMapping, Driven, DrivenSet, HitRegion, Inputs, Knob, Label,
    Motion, SliderLayout, SliderLook, SliderSpec, SpecTint, Transition, ValueIndicator,
    ValueMotion,
};
#[cfg(hydrolysis_runtime)]
pub(crate) use look::{Change, CustomPart, LayerRotation, LiveDriven, RealizedCustom, Retarget};
pub use menu::{MenuLayout, MenuLook, MenuSpec};
pub use navigation::{
    BarSurface, NavigationBarLook, NavigationMetrics, NavigationMotion, NavigationSpec,
};
pub use picker::{MenuPickerLayout, MenuPickerLook, PickerSpec, RadioGroupLook, RadioLayout};
pub use progress::{
    IndicatorLook, LinearLayout, LinearProgressLook, ProgressControl, ProgressSpec,
};
pub use segmented::{
    Segment, SegmentLabel, SegmentedDrag, SegmentedLayout, SegmentedLook, SegmentedSpec,
    SegmentedValue,
};
pub use stepper::{StepperLook, StepperSpec};
pub use tabs::{TabItem, TabItemGeometry, TabItemLayout, TabsLook, TabsMetrics, TabsSpec};
pub use toggle::{CheckboxLook, SwitchDrag, SwitchLook, ToggleBackdrop, ToggleControl, ToggleSpec};
pub use widget::{
    BadgeMetrics, DividerMetrics, InputFieldMetrics, InteractionFocusBinding, InteractionMotion,
    ListDividerMetrics, ListMetrics, ListRowMetrics, ListSectionMetrics,
    ListTrailingControlMetrics, MAX_PRESS_WAVES, ModalInteraction, PressWave, PressWaves,
    TableMetrics, TextCaretMotion, WidgetInteractionState, WidgetTheme,
};

#[cfg(hydrolysis_runtime)]
use waterui::{
    Environment, Plugin,
    color::{Srgb, WorkingColor},
    reactive::signal::IntoComputed,
    theme::{
        ColorDerivation, ColorScheme, ColorSettings, FontSettings, Theme, color as token,
        install_color_derivation, installed_color_scheme,
    },
};

/// The framework's baseline palette for one colour scheme, as encoded sRGB.
///
/// Not a theme: the opaque colour slots a bare application resolves when
/// neither a style package nor the application installs its own. The scrim
/// is the same in both schemes and is not part of it, nor is the disabled
/// foreground, which derives from the scheme's foreground.
#[cfg(hydrolysis_runtime)]
struct Palette {
    background: u32,
    surface: u32,
    surface_variant: u32,
    border: u32,
    foreground: u32,
    muted_foreground: u32,
    tertiary_foreground: u32,
    accent: u32,
    accent_container: u32,
    accent_foreground: u32,
    tertiary: u32,
    tertiary_container: u32,
    selection_container: u32,
    selection_foreground: u32,
    unemphasized_selection: u32,
    error: u32,
    error_foreground: u32,
}

/// The disabled foreground is the scheme's foreground at this alpha, the
/// contrast Material gives disabled content (on-surface at 38%).
#[cfg(hydrolysis_runtime)]
const DISABLED_FOREGROUND_ALPHA: f32 = 0.38;

/// Dark content on light surfaces.
#[cfg(hydrolysis_runtime)]
const LIGHT: Palette = Palette {
    background: 0xFF_FF_FF,
    surface: 0xFF_FF_FF,
    surface_variant: 0xF3_F4_F6,
    border: 0xD1_D5_DB,
    foreground: 0x11_18_27,
    muted_foreground: 0x4B_55_63,
    tertiary_foreground: 0x6B_72_80,
    accent: 0x25_63_EB,
    accent_container: 0xDB_EA_FE,
    accent_foreground: 0xFF_FF_FF,
    tertiary: 0x7C_3A_ED,
    tertiary_container: 0xED_E9_FE,
    selection_container: 0x25_63_EB,
    selection_foreground: 0xFF_FF_FF,
    unemphasized_selection: 0xD1_D5_DB,
    error: 0xDC_26_26,
    error_foreground: 0xFF_FF_FF,
};

/// Light content on dark surfaces, from the same neutral, blue, violet and
/// red ramps as [`LIGHT`]: the fills lighten so they keep their contrast
/// against the dark background, and the content drawn on them darkens.
#[cfg(hydrolysis_runtime)]
const DARK: Palette = Palette {
    background: 0x11_18_27,
    surface: 0x1F_29_37,
    surface_variant: 0x37_41_51,
    border: 0x4B_55_63,
    foreground: 0xF9_FA_FB,
    muted_foreground: 0x9C_A3_AF,
    tertiary_foreground: 0x6B_72_80,
    accent: 0x60_A5_FA,
    accent_container: 0x1E_3A_8A,
    accent_foreground: 0x17_25_54,
    tertiary: 0xA7_8B_FA,
    tertiary_container: 0x4C_1D_95,
    selection_container: 0x60_A5_FA,
    selection_foreground: 0x17_25_54,
    unemphasized_selection: 0x4B_55_63,
    error: 0xF8_71_71,
    error_foreground: 0x45_0A_0A,
};

#[cfg(hydrolysis_runtime)]
const fn palette(scheme: ColorScheme) -> &'static Palette {
    match scheme {
        ColorScheme::Light => &LIGHT,
        ColorScheme::Dark => &DARK,
    }
}

#[cfg(hydrolysis_runtime)]
fn color(rgb: u32) -> WorkingColor {
    Srgb::from_u32(rgb).resolve()
}

/// Installs every palette slot as a derivation from the colour scheme: a
/// scheme change re-resolves exactly the views that read a slot, and a scheme
/// installed on a subtree re-derives the slots that subtree resolves.
#[cfg(hydrolysis_runtime)]
fn install_default_colors(env: &mut Environment) {
    macro_rules! slots {
        ($( $slot:ident => $field:ident ),* $(,)?) => {
            $(
                install_color_derivation::<token::$slot>(
                    env,
                    ColorDerivation::new(|scheme| color(palette(scheme).$field)),
                );
            )*
        };
    }
    slots! {
        Background => background,
        Surface => surface,
        SurfaceVariant => surface_variant,
        Border => border,
        Foreground => foreground,
        MutedForeground => muted_foreground,
        TertiaryForeground => tertiary_foreground,
        Accent => accent,
        AccentContainer => accent_container,
        AccentForeground => accent_foreground,
        Tertiary => tertiary,
        TertiaryContainer => tertiary_container,
        SelectionContainer => selection_container,
        SelectionForeground => selection_foreground,
        UnemphasizedSelection => unemphasized_selection,
        Error => error,
        ErrorForeground => error_foreground,
    }
    install_color_derivation::<token::DisabledForeground>(
        env,
        ColorDerivation::new(|scheme| {
            color(palette(scheme).foreground).with_alpha(DISABLED_FOREGROUND_ALPHA)
        }),
    );
}

/// Writes `resolved` into `scheme`, only when it moved.
///
/// The host's appearance feeds reach the scheme binding through this seam —
/// winit's theme reports on Windows, the portal's `color-scheme` on Linux —
/// and a report repeating the held scheme writes nothing, so the token
/// slots never re-resolve for a report that moved nothing.
#[cfg(any(
    test,
    all(hydrolysis_winit, any(target_os = "windows", target_os = "linux"))
))]
pub(crate) fn sync_color_scheme(scheme: &nami::Binding<ColorScheme>, resolved: ColorScheme) {
    use nami::Signal as _;
    if scheme.snapshot() != resolved {
        scheme.set(resolved);
    }
}

/// Assembles the runtime environment's theme tokens in precedence order:
/// framework defaults < `Style::install_tokens` < the application's own
/// environment (water-rs/hydrolysis#203).
///
/// `appearance` is the host platform's light or dark appearance, a signal
/// the host keeps current — the system appearance on macOS — or a fixed
/// scheme where the host has none. The installed colour scheme is the
/// application's own when it installed one and `appearance` otherwise, and
/// the framework default colours are derivations from that scheme, so a
/// plain view is drawn in the right scheme, an appearance change updates the
/// bound tokens in place, never rebuilding the tree, and a scheme installed
/// on a subtree re-derives the tokens of that subtree.
///
/// The style installs into an environment that already carries the
/// application's entries layered over the framework defaults, so a style
/// that reads the environment while installing — e.g. Material3's dynamic
/// colours binding `installed_color_scheme` — sees the application's
/// installed values. The application's entries are then layered back on top,
/// so a `Theme` it installed is never replaced.
#[cfg(hydrolysis_runtime)]
pub(crate) fn install_theme_tokens(
    env: &mut Environment,
    style: Option<&dyn crate::Style>,
    appearance: impl IntoComputed<ColorScheme>,
) {
    let scheme = installed_color_scheme(env).unwrap_or_else(|| appearance.into_computed());
    let mut defaults = Environment::new();
    Theme::new()
        .color_scheme(scheme)
        // Scrim: black at 32% over the content beneath a modal layer, the
        // same in both schemes.
        .colors(ColorSettings::new().scrim(Srgb::BLACK.resolve().with_alpha(0.32)))
        .fonts(FontSettings::default_scale())
        .install(&mut defaults);
    install_default_colors(&mut defaults);
    let mut styled = env.layered_on(&defaults);
    if let Some(style) = style {
        style.install_tokens(&mut styled);
    }
    *env = env.layered_on(&styled);
}

/// Installs the framework default tokens underneath whatever `env` already
/// carries.
///
/// This is the `None`-style arm of `install_theme_tokens`. External test
/// harnesses (`waterui-testing`) call this; the runners go through
/// `install_theme_tokens`.
#[cfg(hydrolysis_runtime)]
pub fn install_default_tokens(env: &mut Environment) {
    install_theme_tokens(env, None, ColorScheme::Light);
}

#[cfg(test)]
mod tests {
    use waterui::reactive::binding;
    use waterui::theme::color::{Background, Foreground};
    use waterui::theme::{ColorScheme, Theme, installed_color_scheme, installed_color_signal};
    use waterui::{Environment, Signal as _};

    use super::{DARK, LIGHT, color, install_theme_tokens};

    fn bits(color: waterui::color::WorkingColor) -> [u32; 4] {
        color.components.map(f32::to_bits)
    }

    fn resolved<S: 'static>(env: &Environment) -> [u32; 4] {
        bits(
            installed_color_signal::<S>(env)
                .expect("the framework defaults install every colour slot")
                .snapshot(),
        )
    }

    /// The host's appearance is the installed scheme of an application that
    /// installs none, and the default tokens follow it in place.
    #[test]
    fn host_appearance_drives_the_default_tokens() {
        let appearance = binding(ColorScheme::Light);
        let mut env = Environment::new();
        install_theme_tokens(&mut env, None, appearance.clone());
        let scheme = installed_color_scheme(&env).expect("the scheme installs");
        assert_eq!(scheme.snapshot(), ColorScheme::Light);
        assert_eq!(resolved::<Background>(&env), bits(color(LIGHT.background)));
        assert_eq!(resolved::<Foreground>(&env), bits(color(LIGHT.foreground)));

        appearance.set(ColorScheme::Dark);
        assert_eq!(scheme.snapshot(), ColorScheme::Dark);
        assert_eq!(resolved::<Background>(&env), bits(color(DARK.background)));
        assert_eq!(resolved::<Foreground>(&env), bits(color(DARK.foreground)));
    }

    /// The tertiary, disabled and unemphasized-selection tokens have
    /// framework defaults that follow the scheme: the disabled foreground is
    /// the scheme's foreground at the disabled alpha.
    #[test]
    fn default_tokens_carry_the_tier_colors() {
        use waterui::theme::color::{
            DisabledForeground, TertiaryForeground, UnemphasizedSelection,
        };

        let appearance = binding(ColorScheme::Light);
        let mut env = Environment::new();
        install_theme_tokens(&mut env, None, appearance.clone());
        assert_eq!(
            resolved::<TertiaryForeground>(&env),
            bits(color(LIGHT.tertiary_foreground))
        );
        assert_eq!(
            resolved::<UnemphasizedSelection>(&env),
            bits(color(LIGHT.unemphasized_selection))
        );
        assert_eq!(
            resolved::<DisabledForeground>(&env),
            bits(color(LIGHT.foreground).with_alpha(super::DISABLED_FOREGROUND_ALPHA))
        );

        appearance.set(ColorScheme::Dark);
        assert_eq!(
            resolved::<TertiaryForeground>(&env),
            bits(color(DARK.tertiary_foreground))
        );
        assert_eq!(
            resolved::<UnemphasizedSelection>(&env),
            bits(color(DARK.unemphasized_selection))
        );
        assert_eq!(
            resolved::<DisabledForeground>(&env),
            bits(color(DARK.foreground).with_alpha(super::DISABLED_FOREGROUND_ALPHA))
        );
    }

    /// An application that installs its own scheme overrides the host's
    /// appearance, and the default tokens resolve in the application's
    /// scheme rather than the platform's.
    #[test]
    fn application_scheme_outranks_host_appearance() {
        let appearance = binding(ColorScheme::Light);
        let mut env = Environment::new();
        env.install(Theme::new().color_scheme(ColorScheme::Dark));
        install_theme_tokens(&mut env, None, appearance.clone());
        assert_eq!(resolved::<Background>(&env), bits(color(DARK.background)));

        appearance.set(ColorScheme::Light);
        assert_eq!(
            installed_color_scheme(&env).map(|scheme| scheme.snapshot()),
            Some(ColorScheme::Dark)
        );
        assert_eq!(resolved::<Background>(&env), bits(color(DARK.background)));
    }
}
