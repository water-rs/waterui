//! Optional component tracks. Layers using only Affine allocate none of these.

use super::{
    Affine, Instant, KeyframeProp, LayerKeyframes, LayerNode, Prop, Track, Vec2, set_prop,
    without_decay,
};
use crate::animation::{Animatable, AnimationTrack};
use crate::ops::Axis;

pub(super) struct Components {
    pub(super) base: Affine,
    translation: Vec2,
    rotation: f64,
    scale: Vec2,
    skew: Vec2,
    pivot: Vec2,
    /// One track per axis, x first: x and y may run different animations,
    /// started at different instants.
    translation_tracks: AxisTracks,
    rotation_track: Option<Track<f64>>,
    /// One track per axis, x first, like `translation_tracks`.
    scale_tracks: AxisTracks,
    skew_track: Option<Track<Vec2>>,
    pivot_track: Option<Track<Vec2>>,
}

/// A two-axis component's tracks, x first.
type AxisTracks = [Option<Track<f64>>; 2];

/// A layer's transform component values: what
/// [`LayerEdit::translation`](crate::LayerEdit::translation),
/// [`rotation`](crate::LayerEdit::rotation),
/// [`scale`](crate::LayerEdit::scale), [`skew`](crate::LayerEdit::skew)
/// and [`pivot`](crate::LayerEdit::pivot) set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformComponents {
    /// The translation in local coordinates.
    pub translation: Vec2,
    /// The unwrapped rotation angle in radians.
    pub rotation: f64,
    /// The x/y scale factors.
    pub scale: Vec2,
    /// The x/y skew angles in radians.
    pub skew: Vec2,
    /// The pivot rotation, skew and scale turn about.
    pub pivot: Vec2,
}

impl TransformComponents {
    /// Every component at rest: the composition is `base` itself.
    pub const IDENTITY: Self = Self {
        translation: Vec2::ZERO,
        rotation: 0.,
        scale: Vec2::new(1., 1.),
        skew: Vec2::ZERO,
        pivot: Vec2::ZERO,
    };

    /// `base · translate(translation + pivot) · rotate(rotation) · skew ·
    /// scale · translate(−pivot)`: the local transform the engine samples
    /// for a layer with components, so a host that knows the component
    /// values composes the very matrix the engine draws with.
    #[must_use]
    pub fn matrix(&self, base: Affine) -> Affine {
        base * Affine::translate(self.translation + self.pivot)
            * Affine::rotate(self.rotation)
            * Affine::new([1., self.skew.y.tan(), self.skew.x.tan(), 1., 0., 0.])
            * Affine::scale_non_uniform(self.scale.x, self.scale.y)
            * Affine::translate(-self.pivot)
    }
}

/// The lane of `value` that `axis` names.
const fn lane(value: &mut Vec2, axis: Axis) -> &mut f64 {
    match axis {
        Axis::X => &mut value.x,
        Axis::Y => &mut value.y,
    }
}

/// `prop`'s value on one axis, under the same animation and start.
const fn axis_prop(prop: &Prop<Vec2>, axis: Axis) -> Prop<f64> {
    Prop {
        target: axis.of(prop.target),
        animation: prop.animation,
        start: prop.start,
    }
}

/// `prop`'s keyframes on one axis: the same cycle over that lane.
fn axis_keyframes(prop: &KeyframeProp<Vec2>, axis: Axis) -> KeyframeProp<f64> {
    KeyframeProp {
        target: axis.of(prop.target),
        keyframes: prop.keyframes.map(|value| axis.of(*value)),
        repeat: prop.repeat,
    }
}

impl Components {
    /// The translation's motion as one affine track, for a compositor to
    /// take over: `None` unless translation is the only moving component
    /// and its moving axes share one animation and start. An axis at rest
    /// holds its value across the track.
    pub(super) fn translation_animation(&self) -> Option<AnimationTrack<Affine>> {
        if self.rotation_track.is_some()
            || self.scale_tracks.iter().any(Option::is_some)
            || self.skew_track.is_some()
            || self.pivot_track.is_some()
        {
            return None;
        }
        let (mut from, mut target, mut velocity) = (self.translation, self.translation, Vec2::ZERO);
        let mut motion = None;
        for (axis, track) in Axis::BOTH.into_iter().zip(&self.translation_tracks) {
            let Some(track) = track else {
                continue;
            };
            let track = track.description()?;
            match motion {
                None => motion = Some((track.animation, track.start)),
                Some(shared) if shared == (track.animation, track.start) => {}
                Some(_) => return None,
            }
            *lane(&mut from, axis) = track.from;
            *lane(&mut target, axis) = track.target;
            *lane(&mut velocity, axis) = track.velocity[0];
        }
        let (animation, start) = motion?;
        let matrix = self.matrix();
        let map = |translation: Vec2| {
            let delta = translation - self.translation;
            Affine::translate(self.base * delta.to_point() - self.base * kurbo::Point::ORIGIN)
                * matrix
        };
        let velocity = self.base * velocity.to_point() - self.base * kurbo::Point::ORIGIN;
        Some(AnimationTrack {
            from: map(from),
            velocity: [0., 0., 0., 0., velocity.x, velocity.y],
            target: map(target),
            animation,
            start,
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
            translation_tracks: [None, None],
            rotation_track: None,
            scale_tracks: [None, None],
            skew_track: None,
            pivot_track: None,
        }
    }

    pub(super) fn matrix(&self) -> Affine {
        TransformComponents {
            translation: self.translation,
            rotation: self.rotation,
            scale: self.scale,
            skew: self.skew,
            pivot: self.pivot,
        }
        .matrix(self.base)
    }

    /// The component values a projective pose composes, with the base.
    pub(super) const fn pose(
        &self,
        projection: crate::projective::Projective,
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
        let [translation_x, translation_y] = &mut self.translation_tracks;
        let [scale_x, scale_y] = &mut self.scale_tracks;
        let steps = [
            sample(translation_x, &mut self.translation.x, time),
            sample(translation_y, &mut self.translation.y, time),
            sample(&mut self.rotation_track, &mut self.rotation, time),
            sample(scale_x, &mut self.scale.x, time),
            sample(scale_y, &mut self.scale.y, time),
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
            let prop = without_decay(prop);
            let component = self
                .components
                .get_or_insert_with(|| Box::new(Components::new(self.transform)));
            set_prop(&mut component.$track, &mut component.$field, &prop);
            self.transform = component.matrix();
        }
    };
}

impl Components {
    pub(super) const fn note_running(&self, running: &mut super::RunningTracks) {
        let [translation_x, translation_y] = &self.translation_tracks;
        let [scale_x, scale_y] = &self.scale_tracks;
        running.note_track(translation_x.as_ref());
        running.note_track(translation_y.as_ref());
        running.note_track(self.rotation_track.as_ref());
        running.note_track(scale_x.as_ref());
        running.note_track(scale_y.as_ref());
        running.note_track(self.skew_track.as_ref());
        running.note_track(self.pivot_track.as_ref());
    }

    /// Any component track still running.
    pub(super) const fn animating(&self) -> bool {
        let [translation_x, translation_y] = &self.translation_tracks;
        let [scale_x, scale_y] = &self.scale_tracks;
        translation_x.is_some()
            || translation_y.is_some()
            || self.rotation_track.is_some()
            || scale_x.is_some()
            || scale_y.is_some()
            || self.skew_track.is_some()
            || self.pivot_track.is_some()
    }
}

/// A two-axis component's setters: the joint value sets each axis's own
/// track under the same animation and start, exactly as the two
/// per-axis ops would.
macro_rules! axis_property {
    ($joint:ident, $axis:ident, $keyframes:ident, $field:ident, $tracks:ident) => {
        pub(super) fn $joint(&mut self, prop: &Prop<Vec2>) {
            for axis in Axis::BOTH {
                self.$axis(axis, axis_prop(prop, axis));
            }
        }

        pub(super) fn $axis(&mut self, axis: Axis, prop: Prop<f64>) {
            let prop = without_decay(prop);
            let component = self
                .components
                .get_or_insert_with(|| Box::new(Components::new(self.transform)));
            set_prop(
                &mut component.$tracks[axis as usize],
                lane(&mut component.$field, axis),
                &prop,
            );
            self.transform = component.matrix();
        }

        fn $keyframes(&mut self, prop: &KeyframeProp<Vec2>) {
            let component = self
                .components
                .get_or_insert_with(|| Box::new(Components::new(self.transform)));
            for axis in Axis::BOTH {
                component.$tracks[axis as usize] =
                    Some(Track::from_prop(&axis_keyframes(prop, axis)));
            }
        }
    };
}

/// A component's keyframe setter: the track replaces the component's
/// running one; sampling moves the value.
macro_rules! keyframes {
    ($method:ident, $track:ident, $ty:ty) => {
        pub(super) fn $method(&mut self, prop: &KeyframeProp<$ty>) {
            let component = self
                .components
                .get_or_insert_with(|| Box::new(Components::new(self.transform)));
            component.$track = Some(Track::from_prop(prop));
        }
    };
}

impl LayerNode {
    axis_property!(
        set_translation,
        set_translation_axis,
        translation_keyframes,
        translation,
        translation_tracks
    );
    property!(set_rotation, rotation, rotation_track, f64);
    axis_property!(
        set_scale,
        set_scale_axis,
        scale_keyframes,
        scale,
        scale_tracks
    );
    property!(set_skew, skew, skew_track, Vec2);
    property!(set_pivot, pivot, pivot_track, Vec2);
    keyframes!(rotation_keyframes, rotation_track, f64);

    /// Runs a keyframe track on the property it names, replacing that
    /// property's track.
    pub(super) fn set_keyframes(&mut self, keyframes: &LayerKeyframes) {
        match keyframes {
            LayerKeyframes::Transform(prop) => self.transform_track = Some(Track::from_prop(prop)),
            LayerKeyframes::Translation(prop) => self.translation_keyframes(prop),
            LayerKeyframes::Rotation(prop) => self.rotation_keyframes(prop),
            LayerKeyframes::Scale(prop) => self.scale_keyframes(prop),
            LayerKeyframes::Opacity(prop) => self.opacity_track = Some(Track::from_prop(prop)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::LayerOp;
    use crate::{Curve, LayerId, SurfaceTree};
    use std::f64::consts::{PI, TAU};
    use std::time::Duration;

    fn rotation(tree: &mut SurfaceTree, angle: f64) {
        tree.apply(LayerOp::Rotation(
            tree.root(),
            Prop {
                target: angle,
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
                start: None,
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
                let sample = tree.sample(start + elapsed, 1.0);
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
            assert!(!tree.sample(start + Duration::from_secs(2), 1.0).stepped);
        }
    }

    #[test]
    fn component_order_pivot_and_matrix_binding_are_independent() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let set = |target| Prop {
            target,
            animation: None,
            start: None,
        };
        let base = Affine::translate((13., 17.));
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: base,
                animation: None,
                start: None,
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
                start: None,
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
                start: None,
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
            start: None,
        });
        let components = node.components.as_mut().unwrap();
        let now = Instant::now();
        components.sample(now);
        components.sample(now + Duration::from_millis(80));
        let before = components.rotation_track.as_ref().unwrap().last.unwrap();
        node.set_rotation(Prop {
            target: -PI,
            animation: Some(animation),
            start: None,
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
        node.set_translation(&Prop {
            target: Vec2::new(5., 7.),
            animation: None,
            start: None,
        });
        assert!(node.components.as_ref().unwrap().rotation_track.is_some());
    }

    /// The shared evaluator's position for one lane `dt` seconds into a
    /// track from `from` at rest toward `target`.
    fn lane_at(animation: crate::Animation, from: f64, target: f64, dt: f64) -> f64 {
        let (mut position, mut rate) = ([0.], [0.]);
        animation
            .timing()
            .expect("a curve or a spring")
            .sample_lanes(&[from], &[0.], &[target], dt, &mut position, &mut rate);
        position[0]
    }

    #[test]
    fn scale_axes_run_their_own_animations_from_their_own_starts() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let start = Instant::now();
        let later = start + Duration::from_millis(100);
        let curve = crate::Animation::from(Curve::linear(Duration::from_millis(400)));
        let spring = crate::Animation::from(crate::Spring::bouncy());
        tree.apply(LayerOp::ScaleAxis(
            root,
            Axis::X,
            Prop {
                target: 2.,
                animation: Some(curve),
                start: Some(start),
            },
        ));
        tree.apply(LayerOp::ScaleAxis(
            root,
            Axis::Y,
            Prop {
                target: 3.,
                animation: Some(spring),
                start: Some(later),
            },
        ));
        for ms in [0, 50, 100, 250, 400, 600, 2000] {
            let at = start + Duration::from_millis(ms);
            tree.sample(at, 1.0);
            let [a, _, _, d, _, _] = tree.layer(root).transform.as_coeffs();
            let x = lane_at(curve, 1., 2., at.duration_since(start).as_secs_f64());
            let y = lane_at(spring, 1., 3., at.duration_since(later).as_secs_f64());
            assert_eq!(a.to_bits(), x.to_bits(), "{ms} ms: x {a} vs {x}");
            assert_eq!(d.to_bits(), y.to_bits(), "{ms} ms: y {d} vs {y}");
        }
    }

    #[test]
    fn a_joint_scale_is_two_axis_tracks_under_one_animation() {
        let animation = crate::Animation::from(crate::Spring::snappy());
        let start = Instant::now();
        let mut joint = SurfaceTree::new();
        let mut axes = SurfaceTree::new();
        let target = Vec2::new(0.5, 4.);
        joint.apply(LayerOp::Scale(
            joint.root(),
            Prop {
                target,
                animation: Some(animation),
                start: Some(start),
            },
        ));
        for axis in Axis::BOTH {
            axes.apply(LayerOp::ScaleAxis(
                axes.root(),
                axis,
                Prop {
                    target: axis.of(target),
                    animation: Some(animation),
                    start: Some(start),
                },
            ));
        }
        for ms in [0, 16, 120, 333, 5000] {
            let at = start + Duration::from_millis(ms);
            joint.sample(at, 1.0);
            axes.sample(at, 1.0);
            assert_eq!(
                joint.layer(joint.root()).transform,
                axes.layer(axes.root()).transform,
                "{ms} ms"
            );
        }
    }

    #[test]
    fn translation_axes_hand_off_only_under_one_animation_and_start() {
        let start = Instant::now();
        let curve = crate::Animation::from(Curve::ease_in_out(Duration::from_millis(300)));
        let moving = |y_start: Instant| {
            let mut tree = SurfaceTree::new();
            let root = tree.root();
            for (axis, target, start) in [(Axis::X, 10., start), (Axis::Y, -20., y_start)] {
                tree.apply(LayerOp::TranslationAxis(
                    root,
                    axis,
                    Prop {
                        target,
                        animation: Some(curve),
                        start: Some(start),
                    },
                ));
            }
            tree
        };
        let together = moving(start);
        let track = together
            .layer(together.root())
            .animations()
            .and_then(|tracks| tracks.transform)
            .expect("both axes share one curve and start");
        assert_eq!(track.target, Affine::translate((10., -20.)));
        assert_eq!(track.from, Affine::IDENTITY);
        let apart = moving(start + Duration::from_millis(50));
        assert!(
            apart.layer(apart.root()).animations().is_none(),
            "axes started apart are not one affine track"
        );
    }

    /// One second turning a full circle, eased in on the first half.
    fn spin() -> crate::Keyframes<f64> {
        let unit = Duration::from_secs(1);
        crate::Keyframes::new(unit, 0.)
            .then(0.5, Curve::ease_in(unit), PI)
            .then(1., Curve::linear(unit), TAU)
    }

    #[test]
    fn a_keyframed_rotation_is_a_true_rotation_that_repeats() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        tree.apply(LayerOp::Keyframes(
            root,
            crate::LayerKeyframes::Rotation(crate::KeyframeProp {
                target: 0.,
                keyframes: spin(),
                repeat: crate::Repeat::Forever,
            }),
        ));
        let start = Instant::now();
        for frame in 0..360u32 {
            let elapsed = Duration::from_secs(frame.into()) / 120;
            let sampling = tree.sample(start + elapsed, 1.0);
            assert!(sampling.stepped);
            assert_eq!(sampling.rate, Some(super::super::RATE_FAST));
            let angle = spin()
                .sample(elapsed, crate::Repeat::Forever)
                .expect("forever runs");
            let node = tree.layer(root);
            assert!((node.transform.determinant() - 1.).abs() < 1e-12, "{frame}");
            for (got, want) in node
                .transform
                .as_coeffs()
                .into_iter()
                .zip(Affine::rotate(angle).as_coeffs())
            {
                assert!((got - want).abs() < 1e-12, "{frame}: {got} vs {want}");
            }
            assert!(
                node.animations().is_none(),
                "only the engine samples a keyframe track"
            );
        }

        // A curve retargets from the sampled angle and settles.
        let at = start + Duration::from_millis(3010);
        let _ = tree.sample(at, 1.0);
        let sampled = tree.layer(root).components.as_ref().unwrap().rotation;
        rotation(&mut tree, 0.);
        let _ = tree.sample(at, 1.0);
        let continued = tree.layer(root).components.as_ref().unwrap().rotation;
        assert!(
            sampled > 0. && (continued - sampled).abs() < 1e-12,
            "{sampled} -> {continued}"
        );
        let settled = tree.sample(at + Duration::from_secs(2), 1.0);
        assert!(settled.rate.is_none(), "the curve settled");
        assert!(tree.layer(root).components.as_ref().unwrap().rotation.abs() < 1e-12);
        assert!(
            !tree.sample(at + Duration::from_secs(3), 1.0).stepped,
            "nothing is left to sample"
        );
    }

    #[test]
    fn a_finite_opacity_track_ends_on_its_target() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let unit = Duration::from_millis(200);
        tree.apply(LayerOp::Keyframes(
            root,
            crate::LayerKeyframes::Opacity(crate::KeyframeProp {
                target: 0.25,
                keyframes: crate::Keyframes::new(unit, 0.0_f32).then(1., Curve::linear(unit), 1.0),
                repeat: crate::Repeat::ONCE,
            }),
        ));
        let start = Instant::now();
        let _ = tree.sample(start, 1.0);
        let _ = tree.sample(start + Duration::from_millis(100), 1.0);
        assert!((tree.layer(root).opacity - 0.5).abs() < 1e-6);
        let sampling = tree.sample(start + Duration::from_millis(200), 1.0);
        assert!(sampling.stepped && sampling.rate.is_none());
        assert!((tree.layer(root).opacity - 0.25).abs() < f32::EPSILON);
        assert!(!tree.sample(start + Duration::from_millis(300), 1.0).stepped);
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
                start: None,
            },
        ));
        assert!(tree.layer(LayerId::new(1)).components.is_none());
    }
}
