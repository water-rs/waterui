//! Controls text: trivial one-pass shaping — cmap for glyph ids,
//! horizontal advances for positions — over the font the engine
//! registered.

use cherenkov::{FontId, Glyph, GlyphRun, GlyphStyle};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider};

/// Shapes `text` in `font` at `size` units per em, baseline at the
/// origin, advance left to right.
#[must_use]
pub fn run(font: FontId, data: &[u8], size: f32, text: &str) -> GlyphRun {
    let font_ref = FontRef::from_index(data, 0).expect("the registered font parses");
    let charmap = font_ref.charmap();
    let metrics = font_ref.glyph_metrics(Size::new(size), LocationRef::default());
    let mut x = 0.0f32;
    let mut glyphs = Vec::with_capacity(text.len());
    for c in text.chars() {
        let id = charmap.map(c).map_or(0, GlyphId::to_u32);
        glyphs.push(Glyph {
            id,
            x,
            y: 0.0,
            transform: None,
        });
        if let Some(advance) = metrics.advance_width(GlyphId::new(id)) {
            x += advance;
        }
    }
    GlyphRun {
        font,
        size,
        coords: Vec::new().into(),
        glyphs: glyphs.into(),
        style: GlyphStyle::Fill,
    }
}
