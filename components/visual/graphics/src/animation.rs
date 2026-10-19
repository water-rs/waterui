use waterui_core::animation::Animation;
use waterui_core::easing::EasingCurve;

/// Maps the public [`Animation`] onto the engine's execution type through
/// its `curve()`/`duration()` contract — [`Animation::Default`] resolves
/// through those accessors to the documented ease-in-out 250 ms.
///
/// A recorded operand or a layer property bound to a signal animates under
/// the engine animation its change's metadata carries; a backend that drives
/// such a change with a framework animation converts it here.
#[must_use]
pub fn engine_animation(animation: &Animation) -> crate::draw::Animation {
    match animation.curve() {
        EasingCurve::CubicBezier(x1, y1, x2, y2) => {
            crate::draw::Animation::Curve(crate::draw::Curve::bezier(
                animation.duration(),
                f64::from(x1),
                f64::from(y1),
                f64::from(x2),
                f64::from(y2),
            ))
        }
        EasingCurve::Spring { stiffness, damping } => crate::draw::Animation::Spring(
            crate::draw::Spring::from_physics(f64::from(stiffness), f64::from(damping)),
        ),
    }
}
