//! Material Design 3 elevation composed from `WaterUI` primitives.

use waterui::style::{Shadow as ViewShadow, Vector};
use waterui::{Environment, View, ViewExt as _};

use crate::color::Shadow;

const KEY_OPACITY: f32 = 0.30;
const AMBIENT_OPACITY: f32 = 0.15;

const KEY_SHADOWS: [ElevationShadow; 6] = [
    ElevationShadow::new(0.0, 0.0),
    ElevationShadow::new(1.0, 2.0),
    ElevationShadow::new(1.0, 2.0),
    ElevationShadow::new(1.0, 3.0),
    ElevationShadow::new(2.0, 3.0),
    ElevationShadow::new(4.0, 4.0),
];

const AMBIENT_SHADOWS: [ElevationAmbientShadow; 6] = [
    ElevationAmbientShadow::new(0.0, 0.0, 0.0),
    ElevationAmbientShadow::new(1.0, 3.0, 1.0),
    ElevationAmbientShadow::new(2.0, 6.0, 2.0),
    ElevationAmbientShadow::new(4.0, 8.0, 3.0),
    ElevationAmbientShadow::new(6.0, 10.0, 4.0),
    ElevationAmbientShadow::new(8.0, 12.0, 6.0),
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
    ambient: ElevationAmbientShadow,
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
            Shadow.with_opacity(KEY_OPACITY).into(),
            Vector::new(0.0, self.key.y),
            self.key.blur,
        )
    }

    fn ambient_shadow(self) -> ViewShadow {
        ViewShadow::new(
            Shadow.with_opacity(AMBIENT_OPACITY).into(),
            Vector::new(0.0, self.ambient.y),
            self.ambient.blur,
        )
    }
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ElevationAmbientShadow {
    y: f32,
    blur: f32,
    spread: f32,
}

impl ElevationAmbientShadow {
    const fn new(y: f32, blur: f32, spread: f32) -> Self {
        Self { y, blur, spread }
    }
}

#[cfg(test)]
mod tests {
    use super::{AMBIENT_SHADOWS, KEY_SHADOWS, MaterialElevationLevel};

    #[test]
    fn elevation_levels_match_material_web_v0_192() {
        assert_eq!(KEY_SHADOWS[0].y, 0.0);
        assert_eq!(KEY_SHADOWS[1].y, 1.0);
        assert_eq!(KEY_SHADOWS[2].blur, 2.0);
        assert_eq!(KEY_SHADOWS[3].blur, 3.0);
        assert_eq!(KEY_SHADOWS[4].y, 2.0);
        assert_eq!(KEY_SHADOWS[5].y, 4.0);

        assert_eq!(AMBIENT_SHADOWS[0].spread, 0.0);
        assert_eq!(AMBIENT_SHADOWS[1].blur, 3.0);
        assert_eq!(AMBIENT_SHADOWS[2].spread, 2.0);
        assert_eq!(AMBIENT_SHADOWS[3].y, 4.0);
        assert_eq!(AMBIENT_SHADOWS[4].blur, 10.0);
        assert_eq!(AMBIENT_SHADOWS[5].spread, 6.0);
    }

    #[test]
    #[should_panic(expected = "Material elevation level must be in 0..=5")]
    fn elevation_level_rejects_out_of_range_values() {
        let _ = MaterialElevationLevel::new(6);
    }
}
