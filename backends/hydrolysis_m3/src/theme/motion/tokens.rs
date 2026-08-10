//! The Material Design 3 motion token scale.
//!
//! M3 states motion as tokens, not numbers: a component specifies *which*
//! duration and easing token it uses, and the scale is defined once. Mirroring
//! that here means a component's motion reads as the token it is supposed to
//! use, the whole scale can be checked against the specification in one place
//! (see the tests below), and a value that deliberately sits *off* the scale is
//! visibly not a token.
//!
//! Values mirror `mdui/packages/mdui/src/styles/motion.less`.

use core::time::Duration;
use waterui::animation::Animation;
use waterui_core::EasingCurve;

/// The M3 duration scale.
///
/// Four steps per band, rising in even increments: short 50–200ms, medium
/// 250–400ms, long 450–600ms, extra-long 700–1000ms.
pub mod duration {
    use core::time::Duration;

    /// `short1` — 50ms.
    pub const SHORT_1: Duration = Duration::from_millis(50);
    /// `short2` — 100ms.
    pub const SHORT_2: Duration = Duration::from_millis(100);
    /// `short3` — 150ms.
    pub const SHORT_3: Duration = Duration::from_millis(150);
    /// `short4` — 200ms.
    pub const SHORT_4: Duration = Duration::from_millis(200);
    /// `medium1` — 250ms.
    pub const MEDIUM_1: Duration = Duration::from_millis(250);
    /// `medium2` — 300ms.
    pub const MEDIUM_2: Duration = Duration::from_millis(300);
    /// `medium3` — 350ms.
    pub const MEDIUM_3: Duration = Duration::from_millis(350);
    /// `medium4` — 400ms.
    pub const MEDIUM_4: Duration = Duration::from_millis(400);
    /// `long1` — 450ms.
    pub const LONG_1: Duration = Duration::from_millis(450);
    /// `long2` — 500ms.
    pub const LONG_2: Duration = Duration::from_millis(500);
    /// `long3` — 550ms.
    pub const LONG_3: Duration = Duration::from_millis(550);
    /// `long4` — 600ms.
    pub const LONG_4: Duration = Duration::from_millis(600);
    /// `extra-long1` — 700ms.
    pub const EXTRA_LONG_1: Duration = Duration::from_millis(700);
    /// `extra-long2` — 800ms.
    pub const EXTRA_LONG_2: Duration = Duration::from_millis(800);
    /// `extra-long3` — 900ms.
    pub const EXTRA_LONG_3: Duration = Duration::from_millis(900);
    /// `extra-long4` — 1000ms.
    pub const EXTRA_LONG_4: Duration = Duration::from_millis(1000);
}

/// The M3 easing set.
///
/// *Standard* is for ordinary, in-place component motion. *Emphasized* is for
/// motion that carries the eye across the screen — full-screen transitions and
/// large containers. Each has an accelerate variant for elements leaving the
/// screen and a decelerate variant for elements entering it.
pub mod easing {
    use waterui_core::EasingCurve;

    /// No easing.
    ///
    /// M3 uses it for pure opacity changes, where an eased fade reads as a
    /// flicker.
    pub const LINEAR: EasingCurve = EasingCurve::bezier(0.0, 0.0, 1.0, 1.0);

    /// Ordinary in-place motion for small components.
    pub const STANDARD: EasingCurve = EasingCurve::bezier(0.2, 0.0, 0.0, 1.0);
    /// Standard motion for an element leaving the screen.
    pub const STANDARD_ACCELERATE: EasingCurve = EasingCurve::bezier(0.3, 0.0, 1.0, 1.0);
    /// Standard motion for an element entering the screen.
    pub const STANDARD_DECELERATE: EasingCurve = EasingCurve::bezier(0.0, 0.0, 0.0, 1.0);

    /// Motion that carries the eye across the screen.
    ///
    /// M3's emphasized curve is a two-segment path that a single cubic bezier
    /// cannot express; the reference implementations approximate it with
    /// [`STANDARD`], and so do we.
    pub const EMPHASIZED: EasingCurve = STANDARD;
    /// Emphasized motion for an element leaving the screen.
    pub const EMPHASIZED_ACCELERATE: EasingCurve = EasingCurve::bezier(0.3, 0.0, 0.8, 0.15);
    /// Emphasized motion for an element entering the screen.
    pub const EMPHASIZED_DECELERATE: EasingCurve = EasingCurve::bezier(0.05, 0.7, 0.1, 1.0);
}

/// Pair an easing token with a duration token.
///
/// # Panics
///
/// Panics if `easing` is not a cubic bezier. Every M3 easing token is one, so
/// this cannot fire for a token from [`easing`].
#[must_use]
pub const fn motion(easing: EasingCurve, duration: Duration) -> Animation {
    match easing {
        EasingCurve::CubicBezier(x1, y1, x2, y2) => Animation::bezier(duration, x1, y1, x2, y2),
        EasingCurve::Spring { .. } => {
            panic!("Material motion easing tokens are cubic-bezier curves")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{duration, easing, motion};
    use core::time::Duration;
    use waterui::animation::Animation;
    use waterui_core::EasingCurve;

    /// Pins the duration scale to the M3 specification. Every component timing
    /// that claims to be a token is built from this table, so this is the one
    /// place the numbers have to be right.
    #[test]
    fn duration_scale_matches_the_material_3_specification() {
        let scale = [
            (duration::SHORT_1, 50),
            (duration::SHORT_2, 100),
            (duration::SHORT_3, 150),
            (duration::SHORT_4, 200),
            (duration::MEDIUM_1, 250),
            (duration::MEDIUM_2, 300),
            (duration::MEDIUM_3, 350),
            (duration::MEDIUM_4, 400),
            (duration::LONG_1, 450),
            (duration::LONG_2, 500),
            (duration::LONG_3, 550),
            (duration::LONG_4, 600),
            (duration::EXTRA_LONG_1, 700),
            (duration::EXTRA_LONG_2, 800),
            (duration::EXTRA_LONG_3, 900),
            (duration::EXTRA_LONG_4, 1000),
        ];
        for (token, millis) in scale {
            assert_eq!(token, Duration::from_millis(millis));
        }
    }

    /// Pins the easing set to the M3 specification.
    #[test]
    fn easing_set_matches_the_material_3_specification() {
        assert_eq!(easing::LINEAR, EasingCurve::bezier(0.0, 0.0, 1.0, 1.0));
        assert_eq!(easing::STANDARD, EasingCurve::bezier(0.2, 0.0, 0.0, 1.0));
        assert_eq!(
            easing::STANDARD_ACCELERATE,
            EasingCurve::bezier(0.3, 0.0, 1.0, 1.0)
        );
        assert_eq!(
            easing::STANDARD_DECELERATE,
            EasingCurve::bezier(0.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(easing::EMPHASIZED, easing::STANDARD);
        assert_eq!(
            easing::EMPHASIZED_ACCELERATE,
            EasingCurve::bezier(0.3, 0.0, 0.8, 0.15)
        );
        assert_eq!(
            easing::EMPHASIZED_DECELERATE,
            EasingCurve::bezier(0.05, 0.7, 0.1, 1.0)
        );
    }

    /// The linear token and [`Animation::linear`] must describe the same curve,
    /// so a motion written either way is interchangeable.
    #[test]
    fn linear_token_agrees_with_the_linear_animation_constructor() {
        assert_eq!(
            motion(easing::LINEAR, duration::SHORT_4),
            Animation::linear(duration::SHORT_4)
        );
    }
}
