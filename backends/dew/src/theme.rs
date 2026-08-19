//! Dew's reactive widget palette.

use nami::{Computed, Signal};
use peniko::Color;
use waterui::theme::{
    color::{
        Accent, AccentForeground, Border, Foreground, MutedForeground, Surface, SurfaceVariant,
    },
    installed_color_signal,
};
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_core::Environment;
use waterui_graphics::color::{ResolvedColor, Srgb};

use crate::dispatch::WatchedSignal;

/// Primary content color: body text, icons, and stepper glyphs.
pub const FOREGROUND: Color = Color::from_rgb8(28, 28, 30);

/// Secondary content color: placeholders and de-emphasized text.
pub const MUTED_FOREGROUND: Color = Color::from_rgb8(142, 142, 147);

/// Window background behind all content.
///
/// Dew draws no implicit background; apps fill it (typically with a
/// [`waterui_graphics::color::Color`] view) and widgets draw on top.
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

/// Theme signals retained for the renderer lifetime. Every slot requests a
/// frame when it changes, so controls repaint without rebuilding the tree.
pub(crate) struct ThemePalette {
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
            foreground: watch::<Foreground>(env, signals.clone(), FOREGROUND),
            muted_foreground: watch::<MutedForeground>(env, signals.clone(), MUTED_FOREGROUND),
            surface: watch::<Surface>(env, signals.clone(), SURFACE),
            border: watch::<Border>(env, signals.clone(), BORDER),
            accent: watch::<Accent>(env, signals.clone(), ACCENT),
            accent_foreground: watch::<AccentForeground>(env, signals.clone(), ACCENT_FOREGROUND),
            track: watch::<SurfaceVariant>(env, signals, TRACK),
        }
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

fn watch<T: 'static>(
    env: &Environment,
    signals: FrameSignals,
    default: Color,
) -> WatchedSignal<Computed<ResolvedColor>> {
    let signal =
        installed_color_signal::<T>(env).unwrap_or_else(|| Computed::constant(resolved(default)));
    WatchedSignal::new(signal, signals)
}

pub(crate) fn foreground(env: &Environment) -> Color {
    installed_color_signal::<Foreground>(env).map_or(FOREGROUND, |signal| color(signal.get()))
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
