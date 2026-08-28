//! Material Design 3 elevation composed from `WaterUI` primitives.

use waterui::style::{FloatingStyle, Shadow as ViewShadow, Vector};
use waterui::{Environment, View, ViewExt as _};

use crate::color::Shadow;

const KEY_OPACITY: f32 = 0.19;
const AMBIENT_OPACITY: f32 = 0.039;

const KEY_SHADOWS: [ElevationShadow; 6] = [
    ElevationShadow::new(0.0, 0.0),
    ElevationShadow::new(0.5, 1.5),
    ElevationShadow::new(0.85, 3.0),
    ElevationShadow::new(1.25, 5.0),
    ElevationShadow::new(1.85, 6.25),
    ElevationShadow::new(2.75, 9.0),
];

const AMBIENT_SHADOWS: [ElevationShadow; 6] = [
    ElevationShadow::new(0.0, 0.0),
    ElevationShadow::new(0.0, 1.0),
    ElevationShadow::new(0.25, 1.0),
    ElevationShadow::new(0.3333, 1.5),
    ElevationShadow::new(0.5, 1.75),
    ElevationShadow::new(0.25, 3.0),
];

/// A Material Design 3 elevation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialElevationLevel(u8);

impl MaterialElevationLevel {
    /// No elevation.
    pub const LEVEL0: Self = Self(0);
    /// Level 1 elevation.
    pub const LEVEL1: Self = Self(1);
    /// Level 2 elevation.
    pub const LEVEL2: Self = Self(2);
    /// Level 3 elevation.
    pub const LEVEL3: Self = Self(3);
    /// Level 4 elevation.
    pub const LEVEL4: Self = Self(4);
    /// Level 5 elevation.
    pub const LEVEL5: Self = Self(5);

    /// Creates a Material elevation level.
    ///
    /// # Panics
    ///
    /// Panics when `level` is outside Material's 0..=5 elevation range.
    #[must_use]
    pub const fn new(level: u8) -> Self {
        assert!(level <= 5, "Material elevation level must be in 0..=5");
        Self(level)
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

impl Default for MaterialElevationLevel {
    fn default() -> Self {
        Self::LEVEL0
    }
}

#[derive(Debug)]
/// A Material Design 3 elevated surface wrapper.
pub struct MaterialElevation<Content> {
    level: MaterialElevationLevel,
    content: Content,
}

impl<Content> MaterialElevation<Content> {
    /// Creates a Material elevation wrapper.
    #[must_use]
    pub const fn new(level: MaterialElevationLevel, content: Content) -> Self {
        Self { level, content }
    }
}

impl<Content> View for MaterialElevation<Content>
where
    Content: View + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let tokens = ElevationTokens::for_level(self.level);
        self.content
            .shadow(tokens.ambient_shadow())
            .shadow(tokens.key_shadow())
    }
}

/// Creates a Material Design 3 elevated surface wrapper.
#[must_use]
pub const fn material_elevation<Content>(
    level: MaterialElevationLevel,
    content: Content,
) -> MaterialElevation<Content> {
    MaterialElevation::new(level, content)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ElevationTokens {
    key: ElevationShadow,
    ambient: ElevationShadow,
}

impl ElevationTokens {
    const fn for_level(level: MaterialElevationLevel) -> Self {
        Self {
            key: KEY_SHADOWS[level.index()],
            ambient: AMBIENT_SHADOWS[level.index()],
        }
    }

    fn key_shadow(self) -> ViewShadow {
        ViewShadow::new(
            Shadow.with_opacity(self.key.opacity(KEY_OPACITY)).into(),
            Vector::new(0.0, self.key.y),
            self.key.blur,
        )
    }

    fn ambient_shadow(self) -> ViewShadow {
        ViewShadow::new(
            Shadow
                .with_opacity(self.ambient.opacity(AMBIENT_OPACITY))
                .into(),
            Vector::new(0.0, self.ambient.y),
            self.ambient.blur,
        )
    }
}

/// One of the two shadows Material casts at an elevation level, resolved into
/// the colour, offset and blur a theme hands to the renderer.
#[derive(Debug, Clone)]
pub(crate) struct LevelShadow {
    /// Shadow colour, already carrying the level's opacity.
    pub color: waterui::color::Color,
    /// Vertical offset.
    pub offset_y: f32,
    /// Blur radius.
    pub radius: f32,
}

/// The key and ambient shadows for `level`.
///
/// Components name their Compose elevation token — `SnackbarTokens
/// .ContainerElevation` is `Level3`, `ElevatedCardTokens` is `Level1` — and take
/// the numbers from here, so the elevation scale exists in exactly one place.
pub(crate) fn shadows_for_level(level: MaterialElevationLevel) -> (LevelShadow, LevelShadow) {
    let tokens = ElevationTokens::for_level(level);
    (
        LevelShadow {
            color: Shadow.with_opacity(tokens.key.opacity(KEY_OPACITY)).into(),
            offset_y: tokens.key.y,
            radius: tokens.key.blur,
        },
        LevelShadow {
            color: Shadow
                .with_opacity(tokens.ambient.opacity(AMBIENT_OPACITY))
                .into(),
            offset_y: tokens.ambient.y,
            radius: tokens.ambient.blur,
        },
    )
}

pub(crate) fn apply_to_floating_style(style: &mut FloatingStyle, level: MaterialElevationLevel) {
    let tokens = ElevationTokens::for_level(level);
    style.key_shadow_color = Shadow.with_opacity(tokens.key.opacity(KEY_OPACITY)).into();
    style.key_shadow_offset_y = tokens.key.y;
    style.key_shadow_radius = tokens.key.blur;
    style.ambient_shadow_color = Shadow
        .with_opacity(tokens.ambient.opacity(AMBIENT_OPACITY))
        .into();
    style.ambient_shadow_offset_y = tokens.ambient.y;
    style.ambient_shadow_radius = tokens.ambient.blur;
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ElevationShadow {
    y: f32,
    blur: f32,
}

impl ElevationShadow {
    const fn new(y: f32, blur: f32) -> Self {
        Self { y, blur }
    }

    const fn opacity(self, opacity: f32) -> f32 {
        if self.y == 0.0 && self.blur == 0.0 {
            0.0
        } else {
            opacity
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AMBIENT_OPACITY, AMBIENT_SHADOWS, KEY_OPACITY, KEY_SHADOWS, MaterialElevationLevel,
    };

    #[test]
    fn elevation_levels_match_compose_elevation_tokens() {
        assert_eq!(KEY_OPACITY, 0.19);
        assert_eq!(AMBIENT_OPACITY, 0.039);
        assert_eq!(KEY_SHADOWS[0].opacity(KEY_OPACITY), 0.0);
        assert_eq!(AMBIENT_SHADOWS[0].opacity(AMBIENT_OPACITY), 0.0);
        assert_eq!(KEY_SHADOWS[0].y, 0.0);
        assert_eq!(KEY_SHADOWS[1].y, 0.5);
        assert_eq!(KEY_SHADOWS[2].blur, 3.0);
        assert_eq!(KEY_SHADOWS[3].blur, 5.0);
        assert_eq!(KEY_SHADOWS[4].y, 1.85);
        assert_eq!(KEY_SHADOWS[5].y, 2.75);

        assert_eq!(AMBIENT_SHADOWS[0].blur, 0.0);
        assert_eq!(AMBIENT_SHADOWS[1].blur, 1.0);
        assert_eq!(AMBIENT_SHADOWS[2].y, 0.25);
        assert_eq!(AMBIENT_SHADOWS[3].y, 0.3333);
        assert_eq!(AMBIENT_SHADOWS[4].blur, 1.75);
        assert_eq!(AMBIENT_SHADOWS[5].blur, 3.0);
    }

    #[test]
    #[should_panic(expected = "Material elevation level must be in 0..=5")]
    fn elevation_level_rejects_out_of_range_values() {
        let _ = MaterialElevationLevel::new(6);
    }
}
