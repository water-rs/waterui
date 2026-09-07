//! Dew's reactive widget palette.

use nami::{Computed, Signal};
use peniko::Color;
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_core::{Environment, env::Store};
use waterui_graphics::color::{
    AccentColor, AccentForegroundColor, BackgroundColor, BorderColor, ForegroundColor,
    MutedForegroundColor, ResolvedColor, Srgb, SurfaceColor, SurfaceVariantColor,
};
use waterui_text::font::{
    Body, Caption, FontWeight, Footnote, Headline, ResolvedFont, Subheadline, Title,
};

use crate::dispatch::WatchedSignal;

/// Primary content color: body text, icons, and stepper glyphs.
pub const FOREGROUND: Color = Color::from_rgb8(28, 28, 30);

/// Secondary content color: placeholders and de-emphasized text.
pub const MUTED_FOREGROUND: Color = Color::from_rgb8(142, 142, 147);

/// Window background behind all content.
pub const BACKGROUND: Color = Color::WHITE;

/// Raised control surface: text-field boxes and stepper buttons.
pub const SURFACE: Color = Color::from_rgb8(242, 242, 247);

/// Hairlines: dividers, control outlines, and thumb borders.
pub const BORDER: Color = Color::from_rgb8(198, 198, 208);

/// Brand color for active control states: toggle-on tracks, slider and
/// progress fills.
pub const ACCENT: Color = Color::from_rgb8(0, 122, 255);

/// Content drawn on top of [`ACCENT`] fills.
pub const ACCENT_FOREGROUND: Color = Color::WHITE;

/// Inactive track color: toggle-off tracks, slider and progress remainders.
pub const TRACK: Color = Color::from_rgb8(229, 229, 234);

/// Movable control knobs: toggle and slider thumbs.
pub const THUMB: Color = Color::WHITE;

/// Body text: the size and weight `text("…")` shapes at.
pub const BODY_FONT: ResolvedFont = ResolvedFont::new(16.0, FontWeight::Normal);

/// Screen and section titles.
pub const TITLE_FONT: ResolvedFont = ResolvedFont::new(22.0, FontWeight::Normal);

/// The most prominent line on a screen.
pub const HEADLINE_FONT: ResolvedFont = ResolvedFont::new(24.0, FontWeight::Normal);

/// Body-sized text carrying a heading's emphasis.
pub const SUBHEADLINE_FONT: ResolvedFont = ResolvedFont::new(16.0, FontWeight::Medium);

/// Secondary annotations beside content.
pub const CAPTION_FONT: ResolvedFont = ResolvedFont::new(12.0, FontWeight::Normal);

/// The smallest supporting text.
pub const FOOTNOTE_FONT: ResolvedFont = ResolvedFont::new(11.0, FontWeight::Medium);

/// Installs dew's built-in type scale for every font slot the application's
/// theme left unset.
///
/// Font slots are the one part of the palette dew cannot resolve at draw time:
/// a colour an application never chose falls back to this module's constants
/// where it is drawn, while a font is resolved inside `waterui-text`, which
/// requires the token to be in the environment and panics when it is not. Supplying a default appearance is
/// the backend's job rather than the view code's, so [`crate::DewRuntime`]
/// applies this to the environment it renders under — a device app that
/// installs no theme still renders `text("…")`, exactly as it renders a
/// foreground colour it never chose.
///
/// The scale matches the one every other `WaterUI` backend defaults to, so the
/// same view has the same proportions on a panel and on a desktop window. No
/// family is named: firmware shapes with the faces its board bundles, and a
/// desktop simulator with the system collection.
pub fn install_default_fonts(env: &mut Environment) {
    install_default::<Body>(env, BODY_FONT);
    install_default::<Title>(env, TITLE_FONT);
    install_default::<Headline>(env, HEADLINE_FONT);
    install_default::<Subheadline>(env, SUBHEADLINE_FONT);
    install_default::<Caption>(env, CAPTION_FONT);
    install_default::<Footnote>(env, FOOTNOTE_FONT);
}

fn install_default<T: 'static>(env: &mut Environment, font: ResolvedFont) {
    if env.query::<T, Computed<ResolvedFont>>().is_none() {
        env.insert(Store::<T, Computed<ResolvedFont>>::new(Computed::constant(
            font,
        )));
    }
}

/// Theme signals retained for the renderer lifetime. Every slot requests a
/// frame when it changes, so controls repaint without rebuilding the tree.
pub(crate) struct ThemePalette {
    background: WatchedSignal<Computed<ResolvedColor>>,
    foreground: WatchedSignal<Computed<ResolvedColor>>,
    muted_foreground: WatchedSignal<Computed<ResolvedColor>>,
    surface: WatchedSignal<Computed<ResolvedColor>>,
    border: WatchedSignal<Computed<ResolvedColor>>,
    accent: WatchedSignal<Computed<ResolvedColor>>,
    accent_foreground: WatchedSignal<Computed<ResolvedColor>>,
    track: WatchedSignal<Computed<ResolvedColor>>,
}

impl ThemePalette {
    pub(crate) fn new(env: &Environment, signals: FrameSignals) -> Self {
        Self {
            background: watch::<BackgroundColor>(env, signals.clone(), BACKGROUND),
            foreground: watch::<ForegroundColor>(env, signals.clone(), FOREGROUND),
            muted_foreground: watch::<MutedForegroundColor>(env, signals.clone(), MUTED_FOREGROUND),
            surface: watch::<SurfaceColor>(env, signals.clone(), SURFACE),
            border: watch::<BorderColor>(env, signals.clone(), BORDER),
            accent: watch::<AccentColor>(env, signals.clone(), ACCENT),
            accent_foreground: watch::<AccentForegroundColor>(
                env,
                signals.clone(),
                ACCENT_FOREGROUND,
            ),
            track: watch::<SurfaceVariantColor>(env, signals, TRACK),
        }
    }

    pub(crate) fn background(&self) -> Color {
        color(self.background.get())
    }
    pub(crate) fn foreground(&self) -> Color {
        color(self.foreground.get())
    }
    pub(crate) fn muted_foreground(&self) -> Color {
        color(self.muted_foreground.get())
    }
    pub(crate) fn surface(&self) -> Color {
        color(self.surface.get())
    }
    pub(crate) fn border(&self) -> Color {
        color(self.border.get())
    }
    pub(crate) fn accent(&self) -> Color {
        color(self.accent.get())
    }
    pub(crate) fn accent_foreground(&self) -> Color {
        color(self.accent_foreground.get())
    }
    pub(crate) fn track(&self) -> Color {
        color(self.track.get())
    }
    pub(crate) fn thumb(&self) -> Color {
        self.accent_foreground()
    }
}

/// The signal installed for color slot `T`, if a theme installed one.
///
/// Theme installation mirrors every slot into `waterui-graphics`' own slot
/// keys, so reading them here is equivalent to asking the `waterui` facade —
/// and it keeps the facade, with the visual component stack behind it, out of
/// the lean firmware graph (`default-features = false`).
fn installed<T: 'static>(env: &Environment) -> Option<Computed<ResolvedColor>> {
    env.query::<T, Computed<ResolvedColor>>().cloned()
}

/// The signal for colour slot `T`, falling back to dew's built-in default.
///
/// Widgets that hand a colour to a *subtree* need the signal itself rather
/// than a sampled value: a tab bar tints its selected item by installing one
/// of these into the item's environment, so the tint follows the theme without
/// anything rebuilding.
pub(crate) fn slot<T: 'static>(env: &Environment, default: Color) -> Computed<ResolvedColor> {
    installed::<T>(env).unwrap_or_else(|| Computed::constant(resolved(default)))
}

fn watch<T: 'static>(
    env: &Environment,
    signals: FrameSignals,
    default: Color,
) -> WatchedSignal<Computed<ResolvedColor>> {
    WatchedSignal::new(slot::<T>(env, default), signals)
}

pub(crate) fn foreground(env: &Environment) -> Color {
    installed::<ForegroundColor>(env).map_or(FOREGROUND, |signal| color(signal.get()))
}

fn resolved(color: Color) -> ResolvedColor {
    let [red, green, blue, alpha] = color.components;
    let mut resolved = ResolvedColor::from_srgb(Srgb::new(red, green, blue));
    resolved.opacity = alpha;
    resolved
}

fn color(resolved: ResolvedColor) -> Color {
    let srgb = resolved.to_srgb_with_headroom();
    Color::new([srgb.red, srgb.green, srgb.blue, resolved.opacity])
}
