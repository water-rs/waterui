//! One-time layer motion for the cherenkov adapters: a scene `motion`
//! translated into the front-end's animation types and driven on a fixed
//! frame clock.
//!
//! A `motion` starts from its `from` state (a plain set) and animates or
//! decays to the layer's static property, so a `readback` run renders
//! frames until `Next::Idle` and compares the settled scene against the
//! oracle.

use std::time::{Duration, Instant};

use cherenkov::{
    Animation, Curve, Decay, FrameTime, Layer, LayerEdit, Projective, ProjectiveLayers, Spring,
    Surface,
};
use cherenkov_scene::{Motion, MotionAnimation, Projection};

use crate::BenchError;

/// A scene [`Projection`] in front-end types.
#[derive(Debug)]
pub struct LayerProjection {
    /// The projection base.
    projection: Projective,
    /// The settled tilt.
    tilt: Vec2,
    /// The Z translation.
    depth: f64,
    /// The pivot the pose is centred on.
    pivot: Vec2,
}

impl LayerProjection {
    /// Translates a scene projection.
    ///
    /// # Errors
    /// [`BenchError::Engine`] when the matrix is not a valid transform.
    pub fn from_scene(projection: &Projection) -> Result<Self, BenchError> {
        Ok(Self {
            projection: Projective::from_rows(projection.matrix)
                .map_err(|e| BenchError::Engine(format!("cherenkov: projection: {e}")))?,
            tilt: projection.tilt,
            depth: projection.depth,
            pivot: projection.pivot,
        })
    }

    /// Sets the projective pose on `edit`, after its affine `transform`.
    pub fn apply<B: ProjectiveLayers>(&self, edit: &mut LayerEdit<B>) {
        edit.pivot(self.pivot)
            .projection(self.projection)
            .tilt(self.tilt)
            .depth(self.depth);
    }
}
use kurbo::{Affine, Vec2};

/// A scene [`Motion`] translated into front-end animation types.
#[derive(Debug)]
pub enum LayerMotion {
    /// Engine-sampled scalar rotation, independent of the affine base.
    Rotation {
        /// Fixed base matrix.
        base: Affine,
        /// Unwrapped starting angle in radians.
        from: f64,
        /// Unwrapped target angle in radians.
        to: f64,
        /// Rotation pivot in local coordinates.
        pivot: Vec2,
        /// Curve or spring.
        animation: Animation,
    },
    /// `transform` animates `from` → `to` under `animation`.
    Transform {
        /// The start transform.
        from: Affine,
        /// The static (settled) transform.
        to: Affine,
        /// How it moves.
        animation: Animation,
    },
    /// The projection's `tilt` animates `from` → `to` under `animation`.
    Tilt {
        /// The start tilt.
        from: Vec2,
        /// The static (settled) tilt.
        to: Vec2,
        /// How it moves.
        animation: Animation,
    },
    /// `scroll_offset` decays from `from` under `decay`; a decay starts
    /// at the committed value and travels `velocity / deceleration`, so
    /// the generator arranges `from + v/k` to be the static offset.
    Scroll {
        /// The start offset.
        from: Vec2,
        /// The decay parameters.
        decay: Decay,
    },
}

impl LayerMotion {
    /// Translates a scene `motion`, given the layer's static `transform`
    /// and `projection`. The static `scroll_offset` is unused: a decay
    /// commits its `from` state and comes to rest at `from + velocity /
    /// deceleration`.
    ///
    /// # Panics
    /// On a tilt motion without a projection, which scene validation
    /// rejects.
    #[must_use]
    pub fn from_scene(motion: &Motion, transform: Affine, projection: Option<&Projection>) -> Self {
        match motion {
            Motion::Rotation {
                from,
                to,
                pivot,
                animation,
            } => Self::Rotation {
                base: transform
                    * Affine::translate(*pivot)
                    * Affine::rotate(-to)
                    * Affine::translate(-*pivot),
                from: *from,
                to: *to,
                pivot: *pivot,
                animation: motion_animation(*animation),
            },
            Motion::Transform { from, animation } => Self::Transform {
                from: *from,
                to: transform,
                animation: motion_animation(*animation),
            },
            Motion::Tilt { from, animation } => Self::Tilt {
                from: *from,
                to: projection
                    .expect("a validated tilt motion has a projection")
                    .tilt,
                animation: motion_animation(*animation),
            },
            Motion::Scroll {
                from,
                velocity,
                deceleration,
                bounds,
            } => Self::Scroll {
                from: *from,
                decay: Decay {
                    velocity: *velocity,
                    deceleration: *deceleration,
                    rubber_band: *bounds,
                },
            },
            // Never passed in: `prep_layer` routes a `Motion::Paint` into
            // the content run's bindings instead of `from_scene`.
            Motion::Paint { .. } => unreachable!("a paint motion is a content binding"),
        }
    }

    /// Commits the motion on `layer`: the plain `from` set followed by the
    /// animated commit to the static value.
    pub fn apply<B: cherenkov::Backend + ProjectiveLayers>(
        &self,
        surface: &Surface<B>,
        layer: &Layer,
    ) {
        match self {
            Self::Rotation {
                base,
                from,
                to,
                pivot,
                animation,
            } => {
                surface.update(|tx| {
                    tx[layer].transform(*base).pivot(*pivot).rotation(*from);
                });
                surface.update_animated(*animation, |tx| {
                    tx[layer].rotation(*to);
                });
            }
            Self::Transform {
                from,
                to,
                animation,
            } => {
                surface.update(|tx| {
                    tx[layer].transform(*from);
                });
                surface.update_animated(*animation, |tx| {
                    tx[layer].transform(*to);
                });
            }
            Self::Tilt {
                from,
                to,
                animation,
            } => {
                surface.update(|tx| {
                    tx[layer].tilt(*from);
                });
                surface.update_animated(*animation, |tx| {
                    tx[layer].tilt(*to);
                });
            }
            Self::Scroll { from, decay } => {
                surface.update(|tx| {
                    tx[layer].scroll_offset(*from);
                });
                surface.update(|tx| {
                    tx[layer].scroll_offset(*from).animation(*decay);
                });
            }
        }
    }
}

/// The scene `MotionAnimation` → the front-end `Animation`.
pub(crate) fn motion_animation(animation: MotionAnimation) -> Animation {
    match animation {
        MotionAnimation::Spring { response, damping } => {
            Animation::from(Spring { response, damping })
        }
        MotionAnimation::Curve {
            duration_ms,
            x1,
            y1,
            x2,
            y2,
        } => Animation::from(Curve::bezier(
            Duration::from_millis(duration_ms),
            x1,
            y1,
            x2,
            y2,
        )),
    }
}

/// The adapter's frame clock: a fixed origin advanced a 1/120 s tick per
/// frame, so animation sampling is deterministic across engines.
#[derive(Debug)]
pub struct Clock {
    origin: Instant,
    frame: u64,
}

/// The tick the frame clock advances per frame.
const TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);

impl Clock {
    /// A clock at frame zero, origin now.
    #[must_use]
    #[allow(clippy::new_without_default)] // `Instant::now` cannot run in `Default`.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            frame: 0,
        }
    }

    /// The current frame time.
    #[must_use]
    pub fn time(&self) -> FrameTime {
        FrameTime::at(self.origin + TICK * u32::try_from(self.frame).unwrap_or(u32::MAX))
    }

    /// Advances the clock one tick.
    pub const fn advance(&mut self) {
        self.frame += 1;
    }
}
