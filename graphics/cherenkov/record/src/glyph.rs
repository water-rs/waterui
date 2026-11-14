//! Glyph runs. Shaping happens outside the engine; a run is positioned glyphs.

use kurbo::{Affine, Stroke};
use std::sync::Arc;

/// A font registered with the render target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FontId(u64);

impl FontId {
    /// Creates an identifier from a backend-assigned raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// One positioned glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Glyph {
    /// Glyph index in the font.
    pub id: u32,
    /// Horizontal position of the glyph origin.
    pub x: f32,
    /// Vertical position of the glyph origin.
    pub y: f32,
    /// Per-glyph transform about its origin, for example an upright glyph in
    /// vertical CJK text.
    pub transform: Option<Affine>,
}

/// How glyphs are drawn.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GlyphStyle {
    /// Filled outlines.
    #[default]
    Fill,
    /// Stroked outlines.
    Stroke(Stroke),
}

/// A run of glyphs sharing a font, size and variation.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GlyphRun {
    /// The font.
    pub font: FontId,
    /// Size in the drawing's units per em.
    pub size: f32,
    /// Normalized variation coordinates, in `F2Dot14`.
    pub coords: Arc<[i16]>,
    /// The glyphs.
    pub glyphs: Arc<[Glyph]>,
    /// Fill or stroke.
    pub style: GlyphStyle,
}

#[cfg(test)]
mod tests {
    use super::{FontId, Glyph, GlyphRun, GlyphStyle};

    fn run() -> GlyphRun {
        GlyphRun {
            font: FontId::new(7),
            size: 18.0,
            coords: vec![-123, 456].into(),
            glyphs: vec![Glyph {
                id: 36,
                x: 1.5,
                y: 2.5,
                transform: None,
            }]
            .into(),
            style: GlyphStyle::Fill,
        }
    }

    #[test]
    fn clones_share_glyph_and_coordinate_storage() {
        let run = run();
        let cloned = run.clone();
        assert_eq!(run.coords.as_ptr(), cloned.coords.as_ptr());
        assert_eq!(run.glyphs.as_ptr(), cloned.glyphs.as_ptr());
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_round_trips_shared_slices() {
        let run = run();
        let encoded = serde_json::to_string(&run).expect("serialize glyph run");
        let decoded: GlyphRun = serde_json::from_str(&encoded).expect("deserialize glyph run");
        assert_eq!(decoded, run);
    }
}

nami_core::impl_constant!(GlyphRun, GlyphStyle);
