//! The retained-tree hit query for native views: `platform_view_at` walks
//! the mirrored layer tree in paint order — ancestor opacity gates, the
//! parent's transform maps the point down, clips cut it, scroll shifts it —
//! and answers which platform view (or self-drawn content, which is a
//! `None` to the caller) owns the point. It mutates nothing.

use kurbo::{Point, Shape as _};

use super::{Encoder, HwuiError};
use crate::hwui::geometry::unproject_point;
use cherenkov_record::LayerId;
use waterui_graphics::draw::ShapeData;

/// What `platform_view_at` resolves — self-drawn content or the view's id.
enum Hit {
    Content,
    Platform(u64),
}

impl Encoder {
    /// The platform view under `point` (host physical pixels — the root
    /// transform carries the density), or
    /// `None` when self-drawn content or nothing covers it. A view reached
    /// through a clip-free ancestor answers its own bounds; everything else
    /// — alpha, transforms, clips, scroll — composes exactly as drawn.
    ///
    /// # Errors
    ///
    /// A [`HwuiError`] when a layer's projective pose is degenerate.
    pub fn platform_view_at(&self, point: Point) -> Result<Option<u64>, HwuiError> {
        match self.hit_layer(self.tree.root(), point, 1.0)? {
            Some(Hit::Platform(view)) => Ok(Some(view)),
            Some(Hit::Content) | None => Ok(None),
        }
    }

    /// One layer's subtree, in reverse child order (paint topmost first).
    fn hit_layer(
        &self,
        id: LayerId,
        parent_point: Point,
        alpha: f32,
    ) -> Result<Option<Hit>, HwuiError> {
        let node = self.tree.layer(id);
        let state = &self.layers[&id.raw()];
        let alpha = alpha * node.opacity;
        if alpha <= crate::renderer::HIT_TEST_ALPHA_THRESHOLD {
            return Ok(None);
        }
        let point = match self.tree.projective_pose(id) {
            Some(Ok(pose)) => {
                let Some(point) = unproject_point(pose.plane_homography(), parent_point) else {
                    return Ok(None);
                };
                point
            }
            Some(Err(source)) => {
                return Err(HwuiError::Projective {
                    layer: id.raw(),
                    source,
                });
            }
            None => {
                let point = node.transform.inverse() * parent_point;
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Ok(None);
                }
                point
            }
        };
        if let Some(clip) = &state.clip
            && !clip_contains(clip, point)
        {
            return Ok(None);
        }
        let point = point + state.scroll;
        for child in state.children.iter().rev() {
            if let Some(hit) = self.hit_layer(*child, point, alpha)? {
                return Ok(Some(hit));
            }
        }
        if let Some(portal) = &state.platform
            && portal.binding.bounds.contains(point)
        {
            return Ok(Some(Hit::Platform(portal.binding.view)));
        }
        if state.content_ink.is_some_and(|ink| ink.contains(point)) {
            return Ok(Some(Hit::Content));
        }
        Ok(None)
    }
}

/// Whether the clip shape covers `point`, in the shape's own coordinates.
fn clip_contains(clip: &ShapeData, point: Point) -> bool {
    match clip {
        ShapeData::Rect(rect) => rect.contains(point),
        ShapeData::RoundedRect(rect) => rect.contains(point),
        ShapeData::Circle(circle) => circle.contains(point),
        ShapeData::Ellipse(ellipse) => ellipse.contains(point),
        ShapeData::Continuous(corner) => corner
            .to_path(cherenkov_record::PATH_TOLERANCE)
            .contains(point),
        ShapeData::Line(_) => false,
        ShapeData::Path { elements, rule } => {
            let winding = (&elements[..]).winding(point);
            match rule {
                waterui_graphics::draw::FillRule::NonZero => winding != 0,
                waterui_graphics::draw::FillRule::EvenOdd => winding & 1 != 0,
            }
        }
    }
}
