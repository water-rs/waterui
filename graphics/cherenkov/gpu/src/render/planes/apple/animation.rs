//! Native animations on the same layers that carry promoted pixels.

use cherenkov::{Instant, Playback, PlaybackRepeat, Timing};
use objc2::rc::Retained;
use objc2_foundation::{NSNumber, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CACurrentMediaTime, CALayer, CAMediaTiming, CAMediaTimingFunction,
    CASpringAnimation, kCAFillModeBackwards,
};

use super::super::animation::Scalar;

/// Signed clock conversion also accepts a predicted presentation time in
/// the future. Both clocks are sampled together on the main queue, where
/// the animation is installed, so dispatch latency never restarts it.
fn media_time(start: Instant) -> f64 {
    let now = Instant::now();
    let media = CACurrentMediaTime();
    if start >= now {
        media + start.duration_since(now).as_secs_f64()
    } else {
        media - now.duration_since(start).as_secs_f64()
    }
}

/// Core Animation `repeatCount` units become double-cycles when `autoreverses`
/// is enabled.
#[expect(
    clippy::cast_precision_loss,
    reason = "Core Animation repeatCount is f32"
)]
const fn ca_repeat(repeat: PlaybackRepeat) -> (f32, bool) {
    match repeat {
        PlaybackRepeat::Once | PlaybackRepeat::Count { count: 1, .. } => (1.0, false),
        PlaybackRepeat::Count { count, autoreverse } => {
            if autoreverse {
                ((count as f32) * 0.5, true)
            } else {
                (count as f32, false)
            }
        }
        PlaybackRepeat::Forever { autoreverse } => (f32::INFINITY, autoreverse),
    }
}

/// Builds a scalar animation in absolute property units. Unit mass with the
/// spring's own stiffness and damping coefficient gives exactly the engine
/// oscillator.
#[expect(
    clippy::cast_possible_truncation,
    reason = "CAMediaTiming stores f32 timing controls and speed"
)]
pub(super) fn build(
    value: Scalar,
    playback: Playback,
    path: &NSString,
) -> Retained<CABasicAnimation> {
    let animation = match playback.timing {
        Timing::Curve { curve, duration } => {
            let animation = CABasicAnimation::animationWithKeyPath(Some(path));
            animation.setDuration(duration);
            animation.setTimingFunction(Some(&CAMediaTimingFunction::functionWithControlPoints(
                curve.x1 as f32,
                curve.y1 as f32,
                curve.x2 as f32,
                curve.y2 as f32,
            )));
            animation
        }
        Timing::Spring(spring) => {
            let animation = CASpringAnimation::animationWithKeyPath(Some(path));
            animation.setMass(1.0);
            animation.setStiffness(spring.stiffness());
            animation.setDamping(spring.damping_coefficient());
            animation.setAllowsOverdamping(true);
            let distance = value.target - value.from;
            animation.setInitialVelocity(if distance == 0.0 {
                0.0
            } else {
                value.velocity / distance
            });
            animation.setDuration(value.duration());
            Retained::into_super(animation)
        }
    };
    if playback.speed.is_finite() && playback.speed > 0.0 {
        animation.setSpeed(playback.speed as f32);
    }
    let (repeat_count, autoreverses) = ca_repeat(playback.repeat);
    animation.setAutoreverses(autoreverses);
    animation.setRepeatCount(repeat_count);
    if playback.delay > 0.0 {
        // SAFETY: `kCAFillModeBackwards` is a Core Animation exported fill mode constant.
        animation.setFillMode(unsafe { kCAFillModeBackwards });
    }
    // SAFETY: `build` is reached only through `install` for the scalar
    // properties `position.x`, `position.y` and `opacity`, whose animatable
    // value type is `NSNumber` — the correct type for both endpoints.
    unsafe {
        animation.setFromValue(Some(&NSNumber::numberWithDouble(value.from)));
        animation.setToValue(Some(&NSNumber::numberWithDouble(value.target)));
    }
    animation
}

pub(super) fn install(layer: &CALayer, value: Scalar, path: &str) {
    let playback = value
        .animation
        .playback()
        .expect("decays never pass animation admission");
    let path = NSString::from_str(path);
    let animation = build(value, playback, &path);
    let start = media_time(value.start) + playback.delay.max(0.0);
    animation.setBeginTime(layer.convertTime_fromLayer(start, None));
    layer.addAnimation_forKey(&animation, Some(&path));
}

pub(super) fn remove(layer: &CALayer, path: &str) {
    layer.removeAnimationForKey(&NSString::from_str(path));
}
