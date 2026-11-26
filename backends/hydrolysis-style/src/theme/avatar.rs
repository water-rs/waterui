//! Theme contract for avatars.

use waterui_core::Environment;
use waterui_graphics::color::Color;
use waterui_text::font::Font;

use waterui_graphics::color::token::{MutedForeground, SurfaceVariant};

/// Default avatar side length in logical points when callers do not specify one.
const DEFAULT_AVATAR_SIDE: f32 = 40.0;

/// Default monogram font-size ratio relative to the avatar's inner side length.
///
/// Two capitals at 40% of a circle's diameter leave clean optical margin inside
/// the clip silhouette.
const DEFAULT_INITIALS_SCALE: f32 = 0.4;

/// Sizing and typography proportions for an avatar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarMetrics {
    default_side: f32,
    initials_scale: f32,
}

impl AvatarMetrics {
    /// Creates avatar metrics with explicit `default_side` and `initials_scale`.
    #[must_use]
    pub const fn new(default_side: f32, initials_scale: f32) -> Self {
        Self {
            default_side,
            initials_scale,
        }
    }

    /// Returns the default avatar side length in logical points.
    #[must_use]
    pub const fn default_side(&self) -> f32 {
        self.default_side
    }

    /// Returns the monogram font-size scale factor relative to the inner side.
    #[must_use]
    pub const fn initials_scale(&self) -> f32 {
        self.initials_scale
    }
}

impl Default for AvatarMetrics {
    fn default() -> Self {
        Self::new(DEFAULT_AVATAR_SIDE, DEFAULT_INITIALS_SCALE)
    }
}

/// Visual look returned by [`WidgetTheme::avatar`](super::WidgetTheme::avatar).
#[derive(Debug, Clone)]
pub struct AvatarLook {
    metrics: AvatarMetrics,
    background_color: Color,
    monogram_color: Color,
    monogram_font: Font,
}

impl AvatarLook {
    /// Creates an avatar look from metrics, fill color, and monogram styling.
    #[must_use]
    pub const fn new(
        metrics: AvatarMetrics,
        background_color: Color,
        monogram_color: Color,
        monogram_font: Font,
    ) -> Self {
        Self {
            metrics,
            background_color,
            monogram_color,
            monogram_font,
        }
    }

    /// Returns the avatar sizing and monogram scale metrics.
    #[must_use]
    pub const fn metrics(&self) -> AvatarMetrics {
        self.metrics
    }

    /// Returns the backdrop fill color drawn behind the fallback or image.
    #[must_use]
    pub const fn background_color(&self) -> &Color {
        &self.background_color
    }

    /// Returns the text color applied to derived monogram initials.
    #[must_use]
    pub const fn monogram_color(&self) -> &Color {
        &self.monogram_color
    }

    /// Returns the base font applied to derived monogram initials before sizing.
    #[must_use]
    pub const fn monogram_font(&self) -> &Font {
        &self.monogram_font
    }
}

/// Framework baseline avatar look derived from semantic theme tokens.
#[must_use]
pub fn default_avatar_look(_env: &Environment) -> AvatarLook {
    AvatarLook::new(
        AvatarMetrics::default(),
        Color::new(SurfaceVariant),
        Color::new(MutedForeground),
        Font::default(),
    )
}
