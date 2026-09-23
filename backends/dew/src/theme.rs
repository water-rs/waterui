//! Dew's built-in widget palette: named colors shared by every handler.
//!
//! Dew targets panels without an OS theme service, so it ships one
//! deliberately neutral light palette as compile-time constants. Widget
//! handlers must reference these names instead of inlining color literals,
//! keeping the palette swappable in one place.
//!
//! Environment-driven theming (reading installed `WaterUI` theme tokens
//! such as `Foreground` / `Accent` from the [`waterui_core::Environment`],
//! the way hydrolysis does) is the documented follow-up; these constants
//! are the slot defaults it will fall back to.

use peniko::Color;

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
