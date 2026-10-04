//! Native animations on the same layers that carry promoted pixels.

use cherenkov::{Animation, Instant};
use objc2::rc::Retained;
use objc2_foundation::{NSNumber, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CACurrentMediaTime, CALayer, CAMediaTiming, CAMediaTimingFunction,
    CASpringAnimation,
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

/// Builds a scalar animation in absolute property units. Unit mass gives
/// exactly the engine oscillator: k = (2π/response)², c = 2ζ√k.
#[expect(
    clippy::cast_possible_truncation,
    reason = "CAMediaTimingFunction stores f32 controls"
)]
pub(super) fn build(value: Scalar, path: &NSString) -> Retained<CABasicAnimation> {
    let animation = match value.animation {
        Animation::Curve(curve) => {
            let animation = CABasicAnimation::animationWithKeyPath(Some(path));
            animation.setDuration(curve.duration.as_secs_f64());
            animation.setTimingFunction(Some(&CAMediaTimingFunction::functionWithControlPoints(
                curve.p1.x as f32,
                curve.p1.y as f32,
                curve.p2.x as f32,
                curve.p2.y as f32,
            )));
            animation
        }
        Animation::Spring(spring) => {
            let animation = CASpringAnimation::animationWithKeyPath(Some(path));
            let omega = std::f64::consts::TAU / spring.response;
            animation.setMass(1.0);
            animation.setStiffness(omega * omega);
            animation.setDamping(2.0 * spring.damping * omega);
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
        Animation::Decay(_) => unreachable!("decays never pass animation admission"),
    };
    unsafe {
        animation.setFromValue(Some(&NSNumber::numberWithDouble(value.from)));
        animation.setToValue(Some(&NSNumber::numberWithDouble(value.target)));
    }
    animation
}

pub(super) fn install(layer: &CALayer, value: Scalar, path: &str) {
    let path = NSString::from_str(path);
    let animation = build(value, &path);
    animation.setBeginTime(layer.convertTime_fromLayer(media_time(value.start), None));
    layer.addAnimation_forKey(&animation, Some(&path));
}

pub(super) fn remove(layer: &CALayer, path: &str) {
    layer.removeAnimationForKey(&NSString::from_str(path));
}
