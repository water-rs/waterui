//! The colour a window's content lays along its top edge, read from the
//! committed layers rather than from pixels: a host whose own chrome sits
//! against that edge — a browser filling the band behind its status bar —
//! matches it to what the app paints there.

use kurbo::{Affine, Rect};
use waterui_graphics::draw::WorkingColor;

use super::HydrolysisRenderer;
use super::mount::layers::{LayerVisitor, visit};
use super::recording::{SolidRect, axis_aligned};

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// The colour the committed frame shows along the top edge of a window
    /// `width` points wide whose surface clears to `base`: every
    /// single-colour fill that covers the whole edge, composited in paint
    /// order over `base`.
    ///
    /// Only flat colour fields are composed. Content that is not one colour
    /// over the edge — an image, a gradient, a backdrop material, text, a
    /// filtered subtree, anything under a non-rectangular clip — leaves the
    /// colour beneath it standing, so the result is the topmost flat
    /// surface the app spans the edge with, the window background when it
    /// spans none.
    #[must_use]
    pub fn top_edge_color(&self, width: f64, base: WorkingColor) -> WorkingColor {
        let mut sample = TopEdge {
            edge: Rect::new(0.0, 0.0, width, EDGE_DEPTH),
            clips: Vec::new(),
            filters: 0,
            color: base,
        };
        visit(&self.mount_roots(), &mut sample);
        sample.color
    }
}

/// How deep the sampled band reaches below the edge, in points: one point,
/// so a fill counts when it is what the very top row shows.
const EDGE_DEPTH: f64 = 1.0;

struct TopEdge {
    /// The band a fill must cover, in window points.
    edge: Rect,
    /// The open layer clips: each one's window-space rectangle, `None` for
    /// a clip that is not an axis-aligned rectangle — nothing under it can
    /// be shown to cover the edge.
    clips: Vec<Option<Rect>>,
    /// Open filtered layers: their content reaches the target transformed.
    filters: u32,
    color: WorkingColor,
}

impl TopEdge {
    fn edge_unclipped(&self) -> bool {
        self.clips
            .iter()
            .all(|clip| clip.is_some_and(|clip| contains(clip, self.edge)))
    }
}

impl<E: crate::text::TextEngine> LayerVisitor<E> for TopEdge {
    fn solid_rects(&mut self, world: Affine, alphas: &[f32], rects: &[SolidRect]) {
        if self.filters > 0 || !axis_aligned(world) || !self.edge_unclipped() {
            return;
        }
        let alpha: f32 = alphas.iter().product();
        for solid in rects {
            let (rect, color) = solid.current();
            if contains(world.transform_rect_bbox(rect), self.edge) {
                self.color = over(color, alpha, self.color);
            }
        }
    }

    fn open_clip(&mut self, world: Affine, clip: &cherenkov_record::ShapeData) {
        self.clips.push(match clip {
            cherenkov_record::ShapeData::Rect(rect) if axis_aligned(world) => {
                Some(world.transform_rect_bbox(*rect))
            }
            _ => None,
        });
    }

    fn close_clip(&mut self) {
        self.clips.pop();
    }

    fn open_filter(&mut self) {
        self.filters += 1;
    }

    fn close_filter(&mut self) {
        self.filters -= 1;
    }
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
}

/// `source` at `alpha` composited over `dest`: source-over in the linear
/// working space, both straight alpha.
fn over(source: WorkingColor, alpha: f32, dest: WorkingColor) -> WorkingColor {
    let [sr, sg, sb, sa] = source.components;
    let [dr, dg, db, da] = dest.components;
    let sa = sa * alpha;
    let below = da * (1.0 - sa);
    let out = sa + below;
    if out <= 0.0 {
        return WorkingColor::TRANSPARENT;
    }
    let mix = |s: f32, d: f32| d.mul_add(below, s * sa) / out;
    WorkingColor::new([mix(sr, dr), mix(sg, dg), mix(sb, db), out])
}
