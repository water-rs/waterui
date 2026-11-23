#![no_std]
#![cfg_attr(
    test,
    allow(
        clippy::float_cmp,
        reason = "unit tests assert exact resolved panel frequencies"
    )
)]
//! Content-classified refresh-rate votes and host resolution shared by
//! `waterui-core`, the Cherenkov engine, and platform backends.

#[cfg(test)]
extern crate std;

use core::ops::RangeInclusive;
use core::time::Duration;

/// Fallback rate in hertz when an invalid non-positive or non-finite rate is
/// passed in a release build (debug builds fail fast).
#[cfg(not(debug_assertions))]
const FALLBACK_HZ: f32 = 60.0;

/// Minimum hertz required for the [`FrameRate::FAST`] class.
const FAST_MIN_HZ: f32 = 80.0;

/// Minimum hertz for the [`FrameRate::SLOW`] class.
const SLOW_MIN_HZ: f32 = 30.0;

/// Preferred and maximum hertz for the [`FrameRate::SLOW`] class.
const SLOW_MAX_HZ: f32 = 60.0;

/// Validates a finite positive rate in hertz.
const fn sanitize_finite_positive(hz: f32) -> f32 {
    if hz.is_finite() && hz > 0.0 {
        return hz;
    }
    #[cfg(debug_assertions)]
    panic!("FrameRate bounds must be finite and greater than zero");
    #[cfg(not(debug_assertions))]
    FALLBACK_HZ
}

/// Validates an upper bound in hertz (finite positive or `+inf` for open peak).
const fn sanitize_upper(hz: f32, minimum: f32) -> f32 {
    let normalized = if !hz.is_nan() && hz > 0.0 {
        hz
    } else {
        #[cfg(debug_assertions)]
        panic!("FrameRate upper bound must be positive (or +inf) and not NaN");
        #[cfg(not(debug_assertions))]
        FALLBACK_HZ
    };
    if normalized >= minimum {
        normalized
    } else {
        #[cfg(debug_assertions)]
        panic!("FrameRate maximum must not be less than minimum");
        #[cfg(not(debug_assertions))]
        minimum
    }
}

/// A window or content refresh-rate vote in hertz: `(minimum, preferred, maximum)`.
///
/// Mirrors `CAFrameRateRange`, using `+inf` for [`FrameRate::FAST`]'s upper
/// bounds so fast motion resolves to the host panel's real maximum.
#[derive(Clone, Copy, Debug)]
pub struct FrameRate {
    minimum: f32,
    preferred: f32,
    maximum: f32,
}

#[cfg(feature = "nami")]
nami::impl_constant!(FrameRate);

impl PartialEq for FrameRate {
    fn eq(&self, other: &Self) -> bool {
        self.minimum.to_bits() == other.minimum.to_bits()
            && self.preferred.to_bits() == other.preferred.to_bits()
            && self.maximum.to_bits() == other.maximum.to_bits()
    }
}

// Invariant: constructors forbid NaN and -0.0, so equality on fields is reflexive,
// symmetric, and transitive.
impl Eq for FrameRate {}

impl FrameRate {
    /// Fast geometry, scroll, fling, and pointer/touch interaction class:
    /// minimum 80 Hz, preferred and maximum open to the display's peak.
    pub const FAST: Self = Self {
        minimum: FAST_MIN_HZ,
        preferred: f32::INFINITY,
        maximum: f32::INFINITY,
    };

    /// Slow opacity / fade class: 30..=60 Hz, preferring 60 Hz.
    pub const SLOW: Self = Self {
        minimum: SLOW_MIN_HZ,
        preferred: SLOW_MAX_HZ,
        maximum: SLOW_MAX_HZ,
    };

    /// A fixed cadence for video and other source-paced content (`{ hz, hz, hz }`).
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `hz` is not finite and greater than zero.
    #[must_use]
    pub const fn fixed(hz: f32) -> Self {
        let hz = sanitize_finite_positive(hz);
        Self {
            minimum: hz,
            preferred: hz,
            maximum: hz,
        }
    }

    /// An explicit refresh-rate range with a preferred target inside `bounds`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if bounds are non-positive, `NaN`, inverted, or
    /// if `preferred` is non-positive or `NaN`.
    #[must_use]
    pub const fn range(bounds: RangeInclusive<f32>, preferred: f32) -> Self {
        let minimum = sanitize_finite_positive(*bounds.start());
        let maximum = sanitize_upper(*bounds.end(), minimum);
        let preferred = if !preferred.is_nan() && preferred > 0.0 {
            preferred.clamp(minimum, maximum)
        } else {
            #[cfg(debug_assertions)]
            panic!("FrameRate preferred rate must be positive and not NaN");
            #[cfg(not(debug_assertions))]
            minimum
        };
        Self {
            minimum,
            preferred,
            maximum,
        }
    }

    /// Minimum acceptable refresh rate in hertz.
    #[must_use]
    pub const fn minimum(self) -> f32 {
        self.minimum
    }

    /// Preferred refresh rate in hertz (`f32::INFINITY` when open to panel peak).
    #[must_use]
    pub const fn preferred(self) -> f32 {
        self.preferred
    }

    /// Maximum acceptable refresh rate in hertz (`f32::INFINITY` when open to panel peak).
    #[must_use]
    pub const fn maximum(self) -> f32 {
        self.maximum
    }

    /// Whether this vote names an exact fixed source cadence (`min == preferred == max`).
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        self.maximum.is_finite()
            && self.minimum.to_bits() == self.preferred.to_bits()
            && self.preferred.to_bits() == self.maximum.to_bits()
    }

    /// Clamps this vote against a finite display `peak_hz` (`> 0`).
    #[must_use]
    pub const fn clamped(self, peak_hz: f32) -> Self {
        let peak = sanitize_finite_positive(peak_hz);
        let maximum = if self.maximum < peak {
            self.maximum
        } else {
            peak
        };
        let minimum = if self.minimum < maximum {
            self.minimum
        } else {
            maximum
        };
        let preferred = self.preferred.clamp(minimum, maximum);
        Self {
            minimum,
            preferred,
            maximum,
        }
    }

    /// Combines two active votes field-wise by maximum so the most demanding
    /// voter wins.
    #[must_use]
    pub const fn combine_with(self, other: Self) -> Self {
        Self {
            minimum: self.minimum.max(other.minimum),
            preferred: self.preferred.max(other.preferred),
            maximum: self.maximum.max(other.maximum),
        }
    }

    /// Frame interval for the resolved refresh rate on `display`.
    #[must_use]
    pub fn interval(self, display: DisplayCapabilities<'_>) -> Duration {
        let hz = resolve(self, display);
        Duration::from_secs_f64(1.0 / f64::from(hz))
    }
}

/// Combines two optional votes field-wise by maximum; `None` is the identity.
#[must_use]
pub const fn combine(a: Option<FrameRate>, b: Option<FrameRate>) -> Option<FrameRate> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.combine_with(b)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// Combines an iterator of optional votes; returns `None` when no vote is active.
#[must_use]
pub fn combine_all(votes: impl IntoIterator<Item = Option<FrameRate>>) -> Option<FrameRate> {
    votes.into_iter().fold(None, combine)
}

/// What a host display reports about its refresh capabilities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayCapabilities<'a> {
    peak_hz: f32,
    steps: &'a [f32],
}

impl<'a> DisplayCapabilities<'a> {
    /// A display reporting only a peak refresh rate (Apple `ProMotion`, winit,
    /// headless fallback).
    #[must_use]
    pub const fn continuous(peak_hz: f32) -> Self {
        Self {
            peak_hz: sanitize_finite_positive(peak_hz),
            steps: &[],
        }
    }

    /// A display reporting discrete panel refresh steps alongside `peak_hz`.
    #[must_use]
    pub const fn discrete(peak_hz: f32, steps: &'a [f32]) -> Self {
        Self {
            peak_hz: sanitize_finite_positive(peak_hz),
            steps,
        }
    }

    /// Peak panel refresh rate in hertz.
    #[must_use]
    pub const fn peak(self) -> f32 {
        self.peak_hz
    }

    /// Discrete supported refresh steps, or empty when continuous/unknown.
    #[must_use]
    pub const fn steps(self) -> &'a [f32] {
        self.steps
    }
}

/// Picks the panel step nearest `vote.preferred()` within the clamped vote
/// range, rounding up on ties.
///
/// When no panel step falls inside the clamped range, selects the smallest step
/// at or above `preferred` (else the highest step up to `display.peak()`).
#[must_use]
pub fn resolve(vote: FrameRate, display: DisplayCapabilities<'_>) -> f32 {
    let clamped = vote.clamped(display.peak());
    if display.steps.is_empty() {
        return clamped.preferred;
    }

    let mut in_range_best: Option<(f32, f32)> = None;
    let mut above_pref: Option<f32> = None;
    let mut fallback_peak: Option<f32> = None;

    for &step in display.steps {
        if !step.is_finite() || step <= 0.0 || step > display.peak_hz {
            continue;
        }
        fallback_peak = Some(fallback_peak.map_or(step, |best| best.max(step)));
        if step >= clamped.minimum && step <= clamped.maximum {
            let dist = (step - clamped.preferred).abs();
            in_range_best = Some(match in_range_best {
                None => (step, dist),
                Some((best_step, best_dist)) => {
                    if dist < best_dist
                        || (dist.to_bits() == best_dist.to_bits() && step > best_step)
                    {
                        (step, dist)
                    } else {
                        (best_step, best_dist)
                    }
                }
            });
        }
        if step >= clamped.preferred {
            above_pref = Some(above_pref.map_or(step, |best| best.min(step)));
        }
    }

    in_range_best
        .map(|(step, _)| step)
        .or(above_pref)
        .or(fallback_peak)
        .unwrap_or(clamped.preferred)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_uses_field_wise_max_with_none_as_identity() {
        assert_eq!(combine(None, None), None);
        assert_eq!(combine(Some(FrameRate::SLOW), None), Some(FrameRate::SLOW));
        assert_eq!(combine(None, Some(FrameRate::FAST)), Some(FrameRate::FAST));

        // Fade (SLOW) alongside geometry motion (FAST) keeps FAST instead of
        // letting the fade's 30 Hz floor pull the window down.
        assert_eq!(
            combine(Some(FrameRate::SLOW), Some(FrameRate::FAST)),
            Some(FrameRate::FAST)
        );

        // 30 fps video alongside a 60 Hz fade takes the fade's 60 Hz preferred/max.
        assert_eq!(
            combine(Some(FrameRate::fixed(30.0)), Some(FrameRate::SLOW)),
            Some(FrameRate::SLOW)
        );

        // App vote at 144 Hz raises SLOW without lowering FAST's floor.
        let high_app = FrameRate::range(60.0..=144.0, 144.0);
        assert_eq!(
            combine(Some(FrameRate::SLOW), Some(high_app)),
            Some(FrameRate::range(60.0..=144.0, 144.0))
        );
        assert_eq!(
            combine(Some(FrameRate::FAST), Some(high_app)),
            Some(FrameRate::FAST)
        );
    }

    #[test]
    fn resolve_over_panel_table() {
        let panel_60 = DisplayCapabilities::discrete(60.0, &[60.0]);
        let panel_120 = DisplayCapabilities::discrete(120.0, &[60.0, 120.0]);
        let panel_ltpo =
            DisplayCapabilities::discrete(120.0, &[24.0, 30.0, 48.0, 60.0, 90.0, 120.0]);
        let panel_cont_120 = DisplayCapabilities::continuous(120.0);
        let panel_cont_240 = DisplayCapabilities::continuous(240.0);

        // FAST on 60, 120, 240
        assert_eq!(resolve(FrameRate::FAST, panel_60), 60.0);
        assert_eq!(resolve(FrameRate::FAST, panel_120), 120.0);
        assert_eq!(resolve(FrameRate::FAST, panel_ltpo), 120.0);
        assert_eq!(resolve(FrameRate::FAST, panel_cont_120), 120.0);
        assert_eq!(resolve(FrameRate::FAST, panel_cont_240), 240.0);

        // SLOW stays at 60 even on 120/240 panels
        assert_eq!(resolve(FrameRate::SLOW, panel_60), 60.0);
        assert_eq!(resolve(FrameRate::SLOW, panel_120), 60.0);
        assert_eq!(resolve(FrameRate::SLOW, panel_ltpo), 60.0);
        assert_eq!(resolve(FrameRate::SLOW, panel_cont_120), 60.0);
        assert_eq!(resolve(FrameRate::SLOW, panel_cont_240), 60.0);

        // 30 fps video picks 30 when available, else nearest step at or above (60), never 120
        let video_30 = FrameRate::fixed(30.0);
        assert_eq!(resolve(video_30, panel_ltpo), 30.0);
        assert_eq!(resolve(video_30, panel_120), 60.0);
        assert_eq!(resolve(video_30, panel_cont_120), 30.0);

        // 23.976 fps video on LTPO rounds up to 24 or stays 23.976 continuous
        let film = FrameRate::fixed(23.976);
        assert_eq!(resolve(film, panel_ltpo), 24.0);
        assert_eq!(resolve(film, panel_cont_120), 23.976);

        // Tie-breaking rounds up: equidistant 60 and 90 from preferred 75 picks 90
        let tied = FrameRate::range(30.0..=120.0, 75.0);
        assert_eq!(resolve(tied, panel_ltpo), 90.0);
    }

    #[test]
    fn clamped_caps_fast_on_60_and_120_and_240() {
        assert_eq!(
            FrameRate::FAST.clamped(60.0),
            FrameRate::range(60.0..=60.0, 60.0)
        );
        assert_eq!(
            FrameRate::FAST.clamped(120.0),
            FrameRate::range(80.0..=120.0, 120.0)
        );
        assert_eq!(
            FrameRate::FAST.clamped(240.0),
            FrameRate::range(80.0..=240.0, 240.0)
        );
    }
}
