//! Drawing a laid-out formula through the engine-independent `Scene2D`
//! contract.
//!
//! Nothing here knows which engine is underneath. That is the point: the same
//! commands render on the classic compute pipeline, on the CPU/GPU split engine
//! that adapters without compute shaders fall to, and on dew's CPU scene.

use alloc::vec::Vec;

use kurbo::{Affine, Rect, Shape};
use peniko::{Brush, Fill, FontData, StyleRef};
use waterui_graphics::{Glyph, GlyphRun, Scene2D};

use crate::layout::{MathLayout, Placed};

/// Draws `layout` into `scene`, with its baseline origin at `(x, y)`.
///
/// Items are emitted in layout order rather than grouped by kind, because a
/// rule drawn out of order would paint over a glyph that should sit on top of
/// it. Consecutive glyphs at the same size are still batched into one run,
/// which is what keeps a formula from becoming one draw call per character.
pub fn draw(
    layout: &MathLayout,
    scene: &mut dyn Scene2D,
    font: &FontData,
    brush: &Brush,
    x: f32,
    y: f32,
) {
    let origin = Affine::translate((f64::from(x), f64::from(y)));
    let mut pending: Vec<Glyph> = Vec::new();
    let mut pending_size = 0.0_f32;

    for item in &layout.items {
        match item {
            Placed::Glyph {
                glyph,
                x,
                baseline,
                size,
            } => {
                // A run carries one em size, so a size change ends the run.
                if !pending.is_empty() && (*size - pending_size).abs() > f32::EPSILON {
                    flush(scene, font, brush, origin, pending_size, &mut pending);
                }
                pending_size = *size;
                pending.push(Glyph {
                    id: u32::from(glyph.id.0),
                    x: *x,
                    y: *baseline,
                });
            }
            Placed::Outline(outline) => {
                flush(scene, font, brush, origin, pending_size, &mut pending);
                scene.fill(Fill::NonZero, origin, brush, None, outline);
            }
            Placed::Rule {
                x,
                y,
                width,
                height,
            } => {
                flush(scene, font, brush, origin, pending_size, &mut pending);
                let rect = Rect::new(
                    f64::from(*x),
                    f64::from(*y),
                    f64::from(*x + *width),
                    f64::from(*y + *height),
                );
                scene.fill(Fill::NonZero, origin, brush, None, &rect.to_path(0.1));
            }
        }
    }

    flush(scene, font, brush, origin, pending_size, &mut pending);
}

fn flush(
    scene: &mut dyn Scene2D,
    font: &FontData,
    brush: &Brush,
    transform: Affine,
    size: f32,
    glyphs: &mut Vec<Glyph>,
) {
    if glyphs.is_empty() {
        return;
    }
    scene.draw_glyph_run(&GlyphRun {
        font,
        font_size: size,
        normalized_coords: &[],
        transform,
        brush,
        brush_alpha: 1.0,
        style: StyleRef::Fill(Fill::NonZero),
        glyphs,
    });
    glyphs.clear();
}
