use waterui_core::animation::Animation;
use waterui_core::easing::Timing;

/// Maps the public [`Animation`] onto the engine's execution type.
///
/// Both sides evaluate through the same shared timing: a curve keeps its
/// control points and its exact duration — [`Animation::Default`] is the
/// documented ease-in-out 250 ms — and a spring is the very
/// [`Spring`](crate::draw::Spring) core runs, so the engine samples the
/// animation to the values core would.
///
/// A recorded operand or a layer property bound to a signal animates under
/// the engine animation its change's metadata carries; a backend that drives
/// such a change with a framework animation converts it here.
#[must_use]
pub fn engine_animation(animation: &Animation) -> crate::draw::Animation {
    let animation = animation.clone().validated();
    match animation.timing() {
        Timing::Curve { curve, .. } => crate::draw::Animation::Curve(crate::draw::Curve::bezier(
            animation.duration(),
            curve.x1,
            curve.y1,
            curve.x2,
            curve.y2,
        )),
        Timing::Spring(spring) => crate::draw::Animation::Spring(spring),
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use waterui_core::animation::AnimationTrack;

    use super::*;
    use crate::draw::{self, Instant};

    /// The curves and springs the parity grid covers: the system default,
    /// every standard curve, an emphasized and an overshooting curve, a
    /// zero-length curve, and under-, critically and overdamped springs.
    fn animations() -> Vec<Animation> {
        vec![
            Animation::Default,
            Animation::linear(Duration::from_millis(200)),
            Animation::ease_in(Duration::from_millis(300)),
            Animation::ease_out(Duration::from_millis(300)),
            Animation::ease_in_out(Duration::from_millis(450)),
            Animation::bezier(Duration::from_millis(500), 0.2, 0.0, 0.0, 1.0),
            Animation::bezier(Duration::from_millis(350), 0.68, -0.6, 0.32, 1.6),
            Animation::bezier(Duration::ZERO, 0.42, 0.0, 0.58, 1.0),
            Animation::spring(100.0, 10.0),
            Animation::spring(300.0, 15.0),
            Animation::spring(400.0, 40.0),
            Animation::spring(150.0, 60.0),
        ]
    }

    /// Elapsed times from the start to well past every animation's end,
    /// including instants that are not whole frames.
    fn grid() -> impl Iterator<Item = Duration> {
        (0..=360)
            .map(|step| Duration::from_micros(step * 8_333))
            .chain([
                Duration::from_nanos(1),
                Duration::from_micros(123_457),
                Duration::from_secs(10),
            ])
    }

    /// Core's eased progress and the engine's sample of the same animation
    /// as a unit track are the same `f64`, bit for bit, at every instant.
    #[test]
    fn core_and_the_engine_sample_identically() {
        let start = Instant::now();
        for animation in animations() {
            let track = draw::AnimationTrack {
                from: 0.0_f64,
                velocity: [0.0],
                target: 1.0,
                animation: engine_animation(&animation),
                start,
            };
            for elapsed in grid() {
                let (engine, _, engine_done) = track.sample(start + elapsed);
                let core = animation.progress(elapsed);
                assert_eq!(
                    core.to_bits(),
                    engine.to_bits(),
                    "{animation:?} at {elapsed:?}: core {core}, engine {engine}"
                );
                assert_eq!(
                    animation.is_complete(elapsed),
                    engine_done,
                    "{animation:?} at {elapsed:?}: completion"
                );
            }
        }
    }

    /// A core track retargeted mid-flight and the engine track a retarget
    /// hands its sampled position and velocity agree exactly, lane by lane.
    #[test]
    fn retargeted_tracks_agree_with_the_engine() {
        let start = Instant::now();
        let first_leg = Duration::from_millis(90);
        for animation in animations() {
            let engine = engine_animation(&animation);
            let mut core = AnimationTrack::new(0.0_f64);
            core.set_target(1.0, Some(animation.clone()));
            core.advance(first_leg);
            let (from, velocity, _) = draw::AnimationTrack {
                from: 0.0_f64,
                velocity: [0.0],
                target: 1.0,
                animation: engine,
                start,
            }
            .sample(start + first_leg);
            assert_eq!(core.value().to_bits(), from.to_bits(), "{animation:?}");

            core.set_target(-2.5, Some(animation.clone()));
            let retargeted = draw::AnimationTrack {
                from,
                velocity,
                target: -2.5,
                animation: engine,
                start: start + first_leg,
            };
            let mut elapsed = Duration::ZERO;
            for _ in 0..240 {
                let frame = Duration::from_micros(8_333);
                core.advance(frame);
                elapsed += frame;
                let (expected, ..) = retargeted.sample(start + first_leg + elapsed);
                assert_eq!(
                    core.value().to_bits(),
                    expected.to_bits(),
                    "{animation:?} {elapsed:?} after the retarget"
                );
            }
        }
    }
}
