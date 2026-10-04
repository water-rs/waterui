//! Optional component tracks. Layers using only Affine allocate none of these.

use super::{Affine, Animation, Instant, LayerNode, Prop, Track, Vec2, set_prop};
use crate::animation::Animatable;

pub(super) struct Components {
    pub(super) base: Affine,
    translation: Vec2,
    rotation: f64,
    scale: Vec2,
    skew: Vec2,
    pivot: Vec2,
    translation_track: Option<Track<Vec2>>,
    rotation_track: Option<Track<f64>>,
    scale_track: Option<Track<Vec2>>,
    skew_track: Option<Track<Vec2>>,
    pivot_track: Option<Track<Vec2>>,
}

impl Components {
    pub(super) fn translation_animation(&self) -> Option<crate::AnimationTrack<Affine>> {
        if self.rotation_track.is_some()
            || self.scale_track.is_some()
            || self.skew_track.is_some()
            || self.pivot_track.is_some()
        {
            return None;
        }
        let track = self.translation_track.as_ref()?.description()?;
        let matrix = self.matrix();
        let map = |translation: Vec2| {
            let delta = translation - self.translation;
            Affine::translate(self.base * delta.to_point() - self.base * kurbo::Point::ORIGIN)
                * matrix
        };
        let velocity = self.base * Vec2::from_lanes(track.velocity).to_point()
            - self.base * kurbo::Point::ORIGIN;
        Some(crate::AnimationTrack {
            from: map(track.from),
            velocity: [0., 0., 0., 0., velocity.x, velocity.y],
            target: map(track.target),
            animation: track.animation,
            start: track.start,
        })
    }

    const fn new(base: Affine) -> Self {
        Self {
            base,
            translation: Vec2::ZERO,
            rotation: 0.,
            scale: Vec2::new(1., 1.),
            skew: Vec2::ZERO,
            pivot: Vec2::ZERO,
            translation_track: None,
            rotation_track: None,
            scale_track: None,
            skew_track: None,
            pivot_track: None,
        }
    }

    pub(super) fn matrix(&self) -> Affine {
        self.base
            * Affine::translate(self.translation + self.pivot)
            * Affine::rotate(self.rotation)
            * Affine::new([1., self.skew.y.tan(), self.skew.x.tan(), 1., 0., 0.])
            * Affine::scale_non_uniform(self.scale.x, self.scale.y)
            * Affine::translate(-self.pivot)
    }

    /// The component values a projective pose composes, with the base.
    pub(super) const fn pose(
        &self,
        projection: crate::Projective,
        tilt: Vec2,
        depth: f64,
    ) -> crate::projective::Pose {
        crate::projective::Pose {
            base: self.base,
            translation: self.translation,
            pivot: self.pivot,
            rotation: self.rotation,
            skew: self.skew,
            scale: self.scale,
            projection,
            tilt,
            depth,
        }
    }

    pub(super) fn sample(&mut self, time: Instant) -> (bool, bool) {
        let steps = [
            sample(&mut self.translation_track, &mut self.translation, time),
            sample(&mut self.rotation_track, &mut self.rotation, time),
            sample(&mut self.scale_track, &mut self.scale, time),
            sample(&mut self.skew_track, &mut self.skew, time),
            sample(&mut self.pivot_track, &mut self.pivot, time),
        ];
        (steps.iter().any(|s| s.0), steps.iter().any(|s| s.1))
    }
}

pub(super) fn sample<T: Animatable>(
    track: &mut Option<Track<T>>,
    value: &mut T,
    time: Instant,
) -> (bool, bool) {
    let Some(active) = track else {
        return (false, false);
    };
    let (position, _, done) = active.sample(time);
    *value = if done {
        active.target
    } else {
        T::from_lanes(position)
    };
    if done {
        *track = None;
    }
    (true, !done)
}

macro_rules! property {
    ($method:ident, $field:ident, $track:ident, $ty:ty) => {
        pub(super) fn $method(&mut self, prop: Prop<$ty>) {
            assert!(
                !matches!(prop.animation, Some(Animation::Decay(_))),
                "Decay is only legal on scroll_offset"
            );
            let component = self
                .components
                .get_or_insert_with(|| Box::new(Components::new(self.transform)));
            set_prop(&mut component.$track, &mut component.$field, &prop);
            self.transform = component.matrix();
        }
    };
}

impl Components {
    /// Any component track still running.
    pub(super) const fn animating(&self) -> bool {
        self.translation_track.is_some()
            || self.rotation_track.is_some()
            || self.scale_track.is_some()
            || self.skew_track.is_some()
            || self.pivot_track.is_some()
    }
}

impl LayerNode {
    property!(set_translation, translation, translation_track, Vec2);
    property!(set_rotation, rotation, rotation_track, f64);
    property!(set_scale, scale, scale_track, Vec2);
    property!(set_skew, skew, skew_track, Vec2);
    property!(set_pivot, pivot, pivot_track, Vec2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::LayerOp;
    use crate::{Curve, Display, LayerId, SurfaceTree};
    use std::f64::consts::{PI, TAU};
    use std::time::Duration;

    fn rotation(tree: &mut SurfaceTree, angle: f64) {
        tree.apply(LayerOp::Rotation(
            tree.root(),
            Prop {
                target: angle,
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
            },
        ));
    }

    #[test]
    fn half_and_full_turns_keep_orientation_and_winding() {
        for angle in [PI, TAU, -TAU, TAU * 2.] {
            let mut tree = SurfaceTree::new();
            let start = Instant::now();
            rotation(&mut tree, angle);
            for step in 0..=8 {
                let elapsed = Duration::from_millis(step * 125);
                let sample = tree.sample(start + elapsed, Display::default());
                let matrix = tree.layer(tree.root()).transform;
                assert!((matrix.determinant() - 1.).abs() < 1e-12);
                let expected = Affine::rotate(angle * elapsed.as_secs_f64());
                for (actual, expected) in matrix.as_coeffs().into_iter().zip(expected.as_coeffs()) {
                    let error = (actual - expected).abs();
                    // The cubic inverse solver stops at time error 1e-6; sin/cos
                    // are 1-Lipschitz, so coefficient error <= |angle| * 1e-6.
                    assert!(
                        error <= angle.abs().mul_add(1e-6, 1e-12),
                        "angle {angle} step {step}: actual {actual} != expected {expected} (abs error {error})"
                    );
                }
                assert_eq!(sample.rate.is_some(), step < 8);
            }
            assert!(
                !tree
                    .sample(start + Duration::from_secs(2), Display::default())
                    .stepped
            );
        }
    }

    #[test]
    fn component_order_pivot_and_matrix_binding_are_independent() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let set = |target| Prop {
            target,
            animation: None,
        };
        let base = Affine::translate((13., 17.));
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: base,
                animation: None,
            },
        ));
        tree.apply(LayerOp::Translation(root, set(Vec2::new(2., 3.))));
        tree.apply(LayerOp::Scale(root, set(Vec2::new(2., 3.))));
        tree.apply(LayerOp::Skew(root, set(Vec2::new(0.2, -0.1))));
        tree.apply(LayerOp::Pivot(root, set(Vec2::new(4., 5.))));
        tree.apply(LayerOp::Rotation(
            root,
            Prop {
                target: PI / 2.,
                animation: None,
            },
        ));
        let p = kurbo::Point::new(7., 9.);
        let scaled = Vec2::new((p.x - 4.) * 2., (p.y - 5.) * 3.);
        let skewed = Vec2::new(
            0.2_f64.tan().mul_add(scaled.y, scaled.x),
            (-0.1_f64.tan()).mul_add(scaled.x, scaled.y),
        );
        let expected = kurbo::Point::new(13. + 2. + 4. - skewed.y, 17. + 3. + 5. + skewed.x);
        assert!((tree.layer(root).transform * p - expected).hypot() < 1e-12);
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::IDENTITY,
                animation: None,
            },
        ));
        assert!(
            (tree.layer(root).transform * p - (expected - Vec2::new(13., 17.))).hypot() < 1e-12
        );
    }

    #[test]
    fn spring_retarget_preserves_sampled_position_and_velocity() {
        let mut node = LayerNode::new();
        let animation = crate::Spring::smooth().into();
        node.set_rotation(Prop {
            target: TAU,
            animation: Some(animation),
        });
        let components = node.components.as_mut().unwrap();
        let now = Instant::now();
        components.sample(now);
        components.sample(now + Duration::from_millis(80));
        let before = components.rotation_track.as_ref().unwrap().last.unwrap();
        node.set_rotation(Prop {
            target: -PI,
            animation: Some(animation),
        });
        let track = node
            .components
            .as_ref()
            .unwrap()
            .rotation_track
            .as_ref()
            .unwrap();
        assert_eq!(track.from.map(f64::to_bits), before.1.map(f64::to_bits));
        assert_eq!(track.velocity.map(f64::to_bits), before.2.map(f64::to_bits));
        // An independent property does not replace the rotation track.
        node.set_translation(Prop {
            target: Vec2::new(5., 7.),
            animation: None,
        });
        assert!(node.components.as_ref().unwrap().rotation_track.is_some());
    }

    #[test]
    fn matrix_only_layers_have_no_component_allocation() {
        let mut tree = SurfaceTree::new();
        tree.apply(LayerOp::Create(LayerId::new(1)));
        tree.apply(LayerOp::Transform(
            LayerId::new(1),
            Prop {
                target: Affine::scale(2.),
                animation: None,
            },
        ));
        assert!(tree.layer(LayerId::new(1)).components.is_none());
        tracing::debug!(
            component_bytes = std::mem::size_of::<Components>(),
            layer_bytes = std::mem::size_of::<LayerNode>(),
            "tree node sizes",
        );
    }
}
