//! Property tracks that have an exact scalar Core Animation realization.

use cherenkov::{Animation, AnimationTrack, Instant, LayerId, SurfaceTree};
use kurbo::Affine;

/// One scalar animation, in the engine's presentation clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scalar {
    pub from: f64,
    pub velocity: f64,
    pub target: f64,
    pub animation: Animation,
    pub start: Instant,
}

impl Scalar {
    fn expressible(self) -> bool {
        match self.animation {
            Animation::Curve(_) => true,
            Animation::Spring(spring) => {
                spring.response > 0.0
                    && spring.damping > 0.0
                    && spring.response.is_finite()
                    && spring.damping.is_finite()
                    && (self.target - self.from != 0.0 || self.velocity == 0.0)
            }
            Animation::Decay(_) => false,
        }
    }

    /// A duration after which both displacement and velocity stay below
    /// the engine's absolute 1e-3 settlement tolerance. Core Animation's
    /// normalized settling duration alone cannot account for layer size.
    pub fn duration(self) -> f64 {
        let Animation::Spring(spring) = self.animation else {
            let Animation::Curve(curve) = self.animation else {
                unreachable!("only springs and curves are handed off");
            };
            return curve.duration.as_secs_f64();
        };
        let omega = std::f64::consts::TAU / spring.response;
        let displacement = self.from - self.target;
        let velocity = self.velocity;
        let damping = spring.damping;
        let (bound, decay) = if damping < 1.0 {
            let decay = damping * omega;
            let frequency = omega * damping.mul_add(-damping, 1.0).sqrt();
            let amplitude = displacement.hypot(decay.mul_add(displacement, velocity) / frequency);
            (amplitude * omega.max(1.0), decay)
        } else if damping > 1.0 {
            let sum = damping + damping.mul_add(damping, -1.0).sqrt();
            let slow = -omega / sum;
            let fast = -omega * sum;
            let a = fast.mul_add(-displacement, velocity) / (slow - fast);
            let b = displacement - a;
            (
                (a.abs() + b.abs()).max((a * slow).abs() + (b * fast).abs()),
                -slow,
            )
        } else {
            let slope = omega.mul_add(displacement, velocity);
            let bound = displacement
                .abs()
                .max(slope.abs())
                .max(velocity.abs())
                .max((omega * slope).abs());
            if bound <= 1e-3 {
                return 0.0;
            }
            let envelope = |time: f64| bound * (1.0 + time) * (-omega * time).exp();
            let mut low = 1.0 / omega;
            let mut high = low;
            loop {
                if envelope(high) <= 1e-3 {
                    break;
                }
                high *= 2.0;
            }
            for _ in 0..48 {
                let middle = low.midpoint(high);
                if envelope(middle) > 1e-3 {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return high;
        };
        (bound / 1e-3).ln().max(0.0) / decay
    }
}

/// Motion of one leaf plane. Translation is expressed through `position`,
/// keeping the affine linear part constant: Core Animation's decomposed
/// matrix interpolation is not the engine's six-lane interpolation.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub layer: LayerId,
    pub linear: [f64; 4],
    pub position: Option<[Scalar; 2]>,
    pub opacity: Option<Scalar>,
}

fn translation(track: AnimationTrack<Affine>) -> Option<[Scalar; 2]> {
    let from = track.from.as_coeffs();
    let target = track.target.as_coeffs();
    if from[..4] != target[..4] || track.velocity[..4] != [0.0; 4] {
        return None;
    }
    let lanes = [4, 5].map(|i| Scalar {
        from: from[i],
        velocity: track.velocity[i],
        target: target[i],
        animation: track.animation,
        start: track.start,
    });
    lanes.iter().all(|s| s.expressible()).then_some(lanes)
}

/// Describes every moving property or declines the handoff as a whole.
/// A leaf's property cannot move engine-composited descendants. A scroll
/// track or nonlinear component never disappears from frame scheduling.
pub fn motion(tree: &SurfaceTree, layer: LayerId) -> Option<Motion> {
    let node = tree.layer(layer);
    if !node.children.is_empty() {
        return None;
    }
    let tracks = node.animations()?;
    let position = match tracks.transform {
        Some(track) => Some(translation(track)?),
        None => None,
    };
    let opacity = tracks.opacity.map(|track| Scalar {
        from: f64::from(track.from),
        velocity: track.velocity[0],
        target: f64::from(track.target),
        animation: track.animation,
        start: track.start,
    });
    if opacity.is_some_and(|s| !s.expressible()) || (position.is_none() && opacity.is_none()) {
        return None;
    }
    let [a, b, c, d, _, _] = node.transform.as_coeffs();
    Some(Motion {
        layer,
        linear: [a, b, c, d],
        position,
        opacity,
    })
}

/// A moving plane must remain eligible throughout its trajectory. The
/// sampled non-overlap proof is insufficient for an animation that moves
/// without another engine frame, so reject translucent content anywhere
/// later in paint order, including isolated descendants.
pub fn safe_path(
    tree: &SurfaceTree,
    layer: LayerId,
    planes: impl Iterator<Item = LayerId>,
) -> bool {
    let fades = tree
        .layer(layer)
        .animations()
        .is_some_and(|tracks| tracks.opacity.is_some());
    if (super::translucent(tree, layer) || fades)
        && planes.take_while(|&id| id != layer).next().is_some()
    {
        return false;
    }
    let mut pending = vec![tree.root()];
    let mut after = false;
    while let Some(id) = pending.pop() {
        if after && super::translucent(tree, id) {
            return false;
        }
        after |= id == layer;
        pending.extend(tree.layer(id).children.iter().rev().copied());
    }
    after
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov::{Curve, Display, Prop, Spring, testing::LayerOp};
    use std::time::Duration;

    #[test]
    fn translation_preserves_each_lane_and_matrix_motion_is_not_misrepresented() {
        let mut tree = SurfaceTree::new();
        let root = tree.root();
        let start = Instant::now();
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::translate((40., -70.)),
                animation: Some(Spring::bouncy().into()),
            },
        ));
        tree.sample(start, Display::default());
        let description = motion(&tree, root).expect("translation is expressible");
        let [x, y] = description.position.expect("position lanes");
        assert_eq!([x.target, y.target], [40., -70.]);
        assert_eq!(x.start, start);
        tree.apply(LayerOp::Transform(
            root,
            Prop {
                target: Affine::rotate(1.),
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
            },
        ));
        tree.sample(start, Display::default());
        assert!(motion(&tree, root).is_none());
    }

    #[test]
    fn a_plane_cannot_move_without_frames_when_it_can_cross_translucent_content() {
        let mut tree = SurfaceTree::new();
        let plane = LayerId::new(1);
        let above = LayerId::new(2);
        for layer in [plane, above] {
            tree.apply(LayerOp::Create(layer));
            tree.apply(LayerOp::Push {
                parent: tree.root(),
                child: layer,
            });
        }
        tree.apply(LayerOp::Opacity(
            above,
            Prop {
                target: 0.5,
                animation: None,
            },
        ));
        assert!(!safe_path(&tree, plane, [plane].into_iter()));
    }

    #[test]
    fn a_fading_plane_cannot_move_above_an_earlier_plane() {
        let mut tree = SurfaceTree::new();
        let below = LayerId::new(1);
        let moving = LayerId::new(2);
        for layer in [below, moving] {
            tree.apply(LayerOp::Create(layer));
            tree.apply(LayerOp::Push {
                parent: tree.root(),
                child: layer,
            });
        }
        assert!(safe_path(&tree, moving, [below, moving].into_iter()));
        tree.apply(LayerOp::Opacity(
            moving,
            Prop {
                target: 0.5,
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
            },
        ));
        tree.sample(Instant::now(), Display::default());
        assert_eq!(tree.layer(moving).opacity, 1.0);
        assert!(!safe_path(&tree, moving, [below, moving].into_iter()));
    }
}
