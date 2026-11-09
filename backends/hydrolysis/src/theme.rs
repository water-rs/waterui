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
//!   [`WidgetInteractionState`] with its [`PressWaves`], and
//!   [`RadioIndicatorState`];
//! - the motion and metric token tables a theme returns, such as
//!   [`InteractionMotion`] and [`ProgressMotion`];
//! - the looks a theme returns for the widgets ported to the declarative
//!   contract (water-rs/waterui#1434), such as [`ButtonLook`] for a
//!   [`ButtonSpec`], built from the look vocabulary: [`Driven`] values over
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
pub(crate) mod live;
mod look;
mod widget;

pub use chrome::{
    Backdrop, Border, Chrome, CornerShape, Corners, Elevation, Fill, FocusIndicator, PressFeedback,
    Ripple, StateLayer, Surface, Wash,
};
pub use look::{
    ButtonLook, ButtonSpec, Custom, Driven, DrivenSet, HitRegion, Inputs, Label, Motion,
    Transition, ValueMotion,
};
pub(crate) use look::{Change, RealizedCustom};
pub use widget::{
    BadgeMetrics, DividerMetrics, InputFieldMetrics, InteractionFocusBinding, InteractionMotion,
    ListDividerMetrics, ListMetrics, ListRowMetrics, ListSectionMetrics,
    ListTrailingControlMetrics, MAX_PRESS_WAVES, ModalInteraction, NavigationMetrics,
    NavigationMotion, PickerMetrics, PressWave, PressWaves, ProgressIndicatorStyle,
    ProgressMetrics, ProgressMotion, RadioIndicatorState, RadioSelectionMotion, SliderMetrics,
    SliderValueIndicatorMetrics, StepperEnd, StepperMetrics, TabItemGeometry, TabItemLayout,
    TableMetrics, TabsMetrics, TextCaretMotion, TextContextMenuMetrics, ToggleMetrics,
    WidgetInteractionState, WidgetTheme,
};

#[cfg(hydrolysis_runtime)]
use waterui::{
    Environment, Plugin,
    color::{Srgb, WorkingColor},
    reactive::{Computed, SignalExt as _, signal::IntoComputed},
    theme::{ColorScheme, ColorSettings, FontSettings, Theme, installed_color_scheme},
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

/// One palette slot as a signal over `scheme`: a scheme change re-resolves
/// exactly the views that read the slot.
#[cfg(hydrolysis_runtime)]
fn slot(scheme: &Computed<ColorScheme>, pick: fn(&Palette) -> u32) -> Computed<WorkingColor> {
    scheme
        .map(move |scheme| color(pick(palette(scheme))))
        .computed()
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
/// the framework default colours are signals over that scheme, so a plain
/// view is drawn in the right scheme and an appearance change updates the
/// bound tokens in place, never rebuilding the tree.
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
        .color_scheme(scheme.clone())
        .colors(
            ColorSettings::new()
                .background(slot(&scheme, |p| p.background))
                .surface(slot(&scheme, |p| p.surface))
                .surface_variant(slot(&scheme, |p| p.surface_variant))
                .border(slot(&scheme, |p| p.border))
                .foreground(slot(&scheme, |p| p.foreground))
                .muted_foreground(slot(&scheme, |p| p.muted_foreground))
                .tertiary_foreground(slot(&scheme, |p| p.tertiary_foreground))
                .disabled_foreground(
                    scheme
                        .map(|scheme| {
                            color(palette(scheme).foreground).with_alpha(DISABLED_FOREGROUND_ALPHA)
                        })
                        .computed(),
                )
                .accent(slot(&scheme, |p| p.accent))
                .accent_container(slot(&scheme, |p| p.accent_container))
                .accent_foreground(slot(&scheme, |p| p.accent_foreground))
                .tertiary(slot(&scheme, |p| p.tertiary))
                .tertiary_container(slot(&scheme, |p| p.tertiary_container))
                .selection_container(slot(&scheme, |p| p.selection_container))
                .selection_foreground(slot(&scheme, |p| p.selection_foreground))
                .unemphasized_selection(slot(&scheme, |p| p.unemphasized_selection))
                .error(slot(&scheme, |p| p.error))
                .error_foreground(slot(&scheme, |p| p.error_foreground))
                // Scrim: black at 32% over the content beneath a modal layer.
                .scrim(Srgb::BLACK.resolve().with_alpha(0.32)),
        )
        .fonts(FontSettings::default_scale())
        .install(&mut defaults);
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
