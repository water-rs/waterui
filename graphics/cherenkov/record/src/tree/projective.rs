//! Sparse projective layer state. An affine-only tree allocates none of it:
//! the state lives in a side map keyed by layer, created by the first
//! `projection`, `tilt` or `depth` op and removed by `ClearProjection`.

use super::components::sample;
use super::{Animation, Instant, LayerNode, Prop, Track, Vec2, set_prop};
use crate::projective::{Pose, Projective, ProjectiveError};

/// One projective layer's projection base, depth components and their
/// tracks, plus the complete pose sampled for the current frame.
#[derive(Debug)]
pub(super) struct State {
    base: Projective,
    tilt: Vec2,
    depth: f64,
    tilt_track: Option<Track<Vec2>>,
    depth_track: Option<Track<f64>>,
    /// The complete local-to-parent pose, validated after composition.
    pub(super) pose: Result<Projective, ProjectiveError>,
}

impl State {
    pub(super) const fn new() -> Self {
        Self {
            base: Projective::IDENTITY,
            tilt: Vec2::ZERO,
            depth: 0.0,
            tilt_track: None,
            depth_track: None,
            pose: Ok(Projective::IDENTITY),
        }
    }

    pub(super) const fn set_projection(&mut self, base: Projective) {
        self.base = base;
    }

    pub(super) fn set_tilt(&mut self, prop: &Prop<Vec2>) {
        assert_not_decay(prop.animation);
        set_prop(&mut self.tilt_track, &mut self.tilt, prop);
    }

    pub(super) fn set_depth(&mut self, prop: &Prop<f64>) {
        assert_not_decay(prop.animation);
        set_prop(&mut self.depth_track, &mut self.depth, prop);
    }

    /// Steps both tracks; returns `(stepped, still running)`.
    pub(super) fn sample(&mut self, time: Instant) -> (bool, bool) {
        let tilt = sample(&mut self.tilt_track, &mut self.tilt, time);
        let depth = sample(&mut self.depth_track, &mut self.depth, time);
        (tilt.0 || depth.0, tilt.1 || depth.1)
    }

    /// Recomposes the pose from `node`'s affine base and components;
    /// without components the layer's `transform` is its affine base.
    pub(super) fn refresh(&mut self, node: &LayerNode) {
        let pose = node.components.as_ref().map_or_else(
            || Pose {
                base: node.transform,
                translation: Vec2::ZERO,
                pivot: Vec2::ZERO,
                rotation: 0.0,
                skew: Vec2::ZERO,
                scale: Vec2::new(1.0, 1.0),
                projection: self.base,
                tilt: self.tilt,
                depth: self.depth,
            },
            |components| components.pose(self.base, self.tilt, self.depth),
        );
        self.pose = pose.matrix();
    }
}

fn assert_not_decay(animation: Option<Animation>) {
    assert!(
        !matches!(animation, Some(Animation::Decay(_))),
        "Decay is only legal on scroll_offset"
    );
}

#[cfg(test)]
mod tests {
    use crate::ops::{LayerOp, Prop};
    use crate::{Curve, Instant, LayerId, Projective, SurfaceTree};
    use kurbo::{Affine, Vec2};
    use std::f64::consts::PI;
    use std::time::Duration;

    fn snap<T>(target: T) -> Prop<T> {
        Prop {
            target,
            animation: None,
            start: None,
        }
    }

    #[test]
    fn affine_trees_allocate_no_projective_state() {
        let mut tree = SurfaceTree::new();
        tree.apply(LayerOp::Create(LayerId::new(1)));
        tree.apply(LayerOp::Rotation(LayerId::new(1), snap(0.5)));
        assert!(!tree.has_projective());
        assert!(tree.projective_pose(LayerId::new(1)).is_none());
    }

    #[test]
    fn tilt_without_projection_is_an_orthographic_depth_rotation() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        tree.apply(LayerOp::Tilt(root, snap(Vec2::new(0.0, PI / 3.0))));
        let pose = tree.projective_pose(root).unwrap().unwrap();
        let r = pose.as_rows();
        assert!((r[0][0] - 0.5).abs() < 1e-12);
        assert_eq!(
            r[3].map(f64::to_bits),
            [0.0, 0.0, 0.0, 1.0].map(f64::to_bits)
        );
        tree.apply(LayerOp::ClearProjection(root));
        assert!(!tree.has_projective());
    }

    #[test]
    fn components_compose_around_the_projection() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        tree.apply(LayerOp::Transform(
            root,
            snap(Affine::translate((10.0, 0.0))),
        ));
        tree.apply(LayerOp::Pivot(root, snap(Vec2::new(5.0, 0.0))));
        tree.apply(LayerOp::Projection(
            root,
            Projective::perspective(100.0).unwrap(),
        ));
        tree.apply(LayerOp::Depth(root, snap(-100.0)));
        // Depth −100 at camera distance 100 halves every offset from the
        // pivot: w = 1 − (−100)/100 = 2.
        let pose = tree.projective_pose(root).unwrap().unwrap();
        let r = pose.as_rows();
        let (x, w) = (
            r[0][0].mul_add(25.0, r[0][3]),
            r[3][0].mul_add(25.0, r[3][3]),
        );
        assert!((x / w - (10.0 + 5.0 + 10.0)).abs() < 1e-12, "{}", x / w);
    }

    #[test]
    fn a_tilt_track_animates_retains_winding_and_settles() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let start = Instant::now();
        tree.apply(LayerOp::Tilt(
            root,
            Prop {
                target: Vec2::new(0.0, 2.0 * PI),
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
                start: None,
            },
        ));
        let stamp = tree.content_stamp(root);
        tree.sample(start, 1.0);
        let half = tree.sample(start + Duration::from_millis(500), 1.0);
        assert_eq!(half.rate, Some(crate::tree::RATE_FAST));
        // Half of a full turn about Y mirrors x.
        let r = *tree.projective_pose(root).unwrap().unwrap().as_rows();
        assert!((r[0][0] + 1.0).abs() < 1e-5, "{}", r[0][0]);
        // Pose motion never changes what the local image depends on.
        assert_eq!(tree.content_stamp(root), stamp);
        let done = tree.sample(start + Duration::from_secs(1), 1.0);
        assert_eq!(done.rate, None);
    }

    #[test]
    fn an_invalid_composed_pose_is_reported() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        tree.apply(LayerOp::Tilt(root, snap(Vec2::ZERO)));
        tree.apply(LayerOp::Scale(root, snap(Vec2::new(0.0, 1.0))));
        assert_eq!(
            tree.projective_pose(root).unwrap(),
            Err(crate::projective::ProjectiveError::NonInvertible)
        );
    }

    #[test]
    fn content_stamps_follow_inner_changes_only() {
        let mut tree = SurfaceTree::new();
        let (card, child) = (LayerId::new(1), LayerId::new(2));
        for id in [card, child] {
            tree.apply(LayerOp::Create(id));
        }
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child: card,
        });
        tree.apply(LayerOp::Push {
            parent: card,
            child,
        });
        tree.apply(LayerOp::Tilt(card, snap(Vec2::new(0.1, 0.2))));
        let stamp = tree.content_stamp(card);
        tree.apply(LayerOp::Opacity(card, snap(0.5)));
        tree.apply(LayerOp::Transform(
            card,
            snap(Affine::translate((3.0, 0.0))),
        ));
        tree.apply(LayerOp::Tilt(card, snap(Vec2::new(0.3, 0.2))));
        assert_eq!(tree.content_stamp(card), stamp);
        tree.apply(LayerOp::Opacity(child, snap(0.5)));
        let child_changed = tree.content_stamp(card);
        assert!(child_changed > stamp);
        tree.apply(LayerOp::ScrollOffset(card, snap(Vec2::new(0.0, 4.0))));
        assert!(tree.content_stamp(card) > child_changed);
    }
}
