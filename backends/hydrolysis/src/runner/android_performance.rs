//! Android ADPF performance-hint targets and thermal/headroom scheduling policy.
//!
//! Pure calculations compiled on Android for [`super::android::host`] and on
//! non-wasm hosts for unit tests; zero JNI or platform calls live here.

use std::time::Duration;

use waterui_frame_rate::{DisplayCapabilities, FrameRate, resolve};

const DEFAULT_NORMAL_HZ: f32 = 60.0;
const CRITICAL_NON_FIXED_CAP_HZ: f32 = 30.0;
const THERMAL_CONSTRAINED_HEADROOM: f32 = 0.90;
const THERMAL_CRITICAL_HEADROOM: f32 = 1.0;
const COMPUTE_LOW_HEADROOM_PERCENT: f32 = 10.0;

/// Android `PowerManager.THERMAL_STATUS_*` levels (`0..=6`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThermalStatus {
    /// Not under thermal throttling (`THERMAL_STATUS_NONE`).
    #[default]
    None = 0,
    /// Light thermal status (`THERMAL_STATUS_LIGHT`).
    Light = 1,
    /// Moderate thermal status (`THERMAL_STATUS_MODERATE`).
    Moderate = 2,
    /// Severe thermal throttling imminent (`THERMAL_STATUS_SEVERE`).
    Severe = 3,
    /// Critical thermal state (`THERMAL_STATUS_CRITICAL`).
    Critical = 4,
    /// Emergency thermal shutdown imminent (`THERMAL_STATUS_EMERGENCY`).
    Emergency = 5,
    /// Device shutting down (`THERMAL_STATUS_SHUTDOWN`).
    Shutdown = 6,
}

impl ThermalStatus {
    /// Converts a platform `PowerManager.THERMAL_STATUS_*` integer to
    /// [`ThermalStatus`], clamping out-of-range values to the nearest endpoint.
    #[must_use]
    pub const fn from_android_status(raw: i32) -> Self {
        match raw {
            i32::MIN..=0 => Self::None,
            1 => Self::Light,
            2 => Self::Moderate,
            3 => Self::Severe,
            4 => Self::Critical,
            5 => Self::Emergency,
            6..=i32::MAX => Self::Shutdown,
        }
    }
}

/// Latest thermal and compute headroom sample reported by Android host services.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeadroomSnapshot {
    /// Discrete thermal severity from `PowerManager.OnThermalStatusChangedListener`.
    pub thermal_status: ThermalStatus,
    /// Forecast thermal headroom (`PowerManager.getThermalHeadroom`), where
    /// values `>= 1.0` indicate thermal throttling.
    pub thermal_headroom: Option<f32>,
    /// Android 16 CPU headroom (`SystemHealthManager.getCpuHeadroom`), `0.0..=100.0`.
    pub cpu_headroom: Option<f32>,
    /// Android 16 GPU headroom (`SystemHealthManager.getGpuHeadroom`), `0.0..=100.0`.
    pub gpu_headroom: Option<f32>,
}

fn sanitize_non_negative(value: f32) -> Option<f32> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

impl HeadroomSnapshot {
    /// Constructs a snapshot from JNI primitives where `NaN` or negative values
    /// represent an unavailable or unsupported reading.
    #[must_use]
    pub fn from_jni(
        cpu_headroom: f32,
        gpu_headroom: f32,
        thermal_status: i32,
        thermal_headroom: f32,
    ) -> Self {
        Self {
            thermal_status: ThermalStatus::from_android_status(thermal_status),
            thermal_headroom: sanitize_non_negative(thermal_headroom),
            cpu_headroom: sanitize_non_negative(cpu_headroom),
            gpu_headroom: sanitize_non_negative(gpu_headroom),
        }
    }

    /// Quantized pressure level driving refresh-rate ceiling and target budget.
    #[must_use]
    pub fn pressure_tier(self) -> PressureTier {
        if self.thermal_status >= ThermalStatus::Critical
            || self
                .thermal_headroom
                .is_some_and(|headroom| headroom >= THERMAL_CRITICAL_HEADROOM)
        {
            return PressureTier::Critical;
        }
        let compute_low = [self.cpu_headroom, self.gpu_headroom]
            .into_iter()
            .flatten()
            .any(|headroom| headroom <= COMPUTE_LOW_HEADROOM_PERCENT);
        if self.thermal_status >= ThermalStatus::Moderate
            || self
                .thermal_headroom
                .is_some_and(|headroom| headroom >= THERMAL_CONSTRAINED_HEADROOM)
            || compute_low
        {
            return PressureTier::Constrained;
        }
        PressureTier::Nominal
    }
}

/// Scheduling pressure classification derived from [`HeadroomSnapshot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressureTier {
    /// Normal envelope: full content refresh demand applies.
    Nominal,
    /// Elevated thermal or low CPU/GPU headroom: step non-fixed demand down to normal refresh.
    Constrained,
    /// Critical thermal load: aggressively reduce cadence to avoid thermal runaway.
    Critical,
}

/// Display refresh capabilities reported by the Android host or surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayRefreshSpec {
    /// Highest supported panel refresh rate in hertz.
    pub peak: f32,
    /// Normal continuous-animation refresh step (typically `60.0`).
    pub normal: f32,
    /// High continuous-animation refresh step (or `peak` on non-ARR displays).
    pub high: f32,
}

impl DisplayRefreshSpec {
    /// Constructs display refresh capabilities from a single reported rate (used
    /// when discrete ARR steps are unavailable, such as HWUI before surface attachment).
    #[must_use]
    pub const fn from_peak(peak_hz: f32) -> Self {
        let peak = if peak_hz.is_finite() && peak_hz > 0.0 {
            peak_hz
        } else {
            DEFAULT_NORMAL_HZ
        };
        let normal = if peak < DEFAULT_NORMAL_HZ {
            peak
        } else {
            DEFAULT_NORMAL_HZ
        };
        Self {
            peak,
            normal,
            high: peak,
        }
    }

    /// Resolves `vote` against these discrete display capabilities.
    #[must_use]
    pub fn resolve_rate(self, vote: FrameRate) -> f32 {
        let steps = [self.normal, self.high];
        resolve(vote, DisplayCapabilities::discrete(self.peak, &steps))
    }
}

/// Adjusts `vote` according to [`HeadroomSnapshot`]:
/// - `Nominal`: preserves `vote` unmodified;
/// - `Constrained`: caps non-fixed votes at `normal_hz.max(60.0)` while preserving fixed sources;
/// - `Critical`: caps non-fixed votes at `30.0` Hz and fixed votes at `normal_hz.max(60.0)`.
#[must_use]
pub fn effective_refresh_vote(
    vote: Option<FrameRate>,
    headroom: HeadroomSnapshot,
    normal_hz: f32,
) -> Option<FrameRate> {
    let vote = vote?;
    let normal_cap = if normal_hz.is_finite() && normal_hz > 0.0 {
        normal_hz.max(DEFAULT_NORMAL_HZ)
    } else {
        DEFAULT_NORMAL_HZ
    };
    match headroom.pressure_tier() {
        PressureTier::Nominal => Some(vote),
        PressureTier::Constrained => {
            if vote.is_fixed() {
                Some(vote)
            } else {
                Some(vote.clamped(normal_cap))
            }
        }
        PressureTier::Critical => {
            if vote.is_fixed() {
                Some(vote.clamped(normal_cap))
            } else {
                Some(vote.clamped(CRITICAL_NON_FIXED_CAP_HZ.min(normal_cap)))
            }
        }
    }
}

/// Performance-hint target pushed to `HydrolysisSession`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PerformanceTarget {
    /// ADPF target work duration in nanoseconds (`0` closes/suspends the hint session).
    pub target_duration_nanos: i64,
    /// Maximum refresh rate ceiling for the HWUI painter (`0.0` clears the cap).
    pub max_refresh_hz: f32,
}

/// Resolves the surface frame rate (`Hz`) for `vote` under `headroom` and `display`.
/// Non-fixed votes under [`PressureTier::Critical`] are capped at `30.0` Hz.
#[must_use]
pub fn resolve_surface_refresh_rate(
    vote: FrameRate,
    headroom: HeadroomSnapshot,
    display: DisplayRefreshSpec,
) -> f32 {
    let effective = effective_refresh_vote(Some(vote), headroom, display.normal).unwrap_or(vote);
    let resolved = display.resolve_rate(effective);
    match headroom.pressure_tier() {
        PressureTier::Critical if !vote.is_fixed() => {
            resolved.min(CRITICAL_NON_FIXED_CAP_HZ.min(display.normal.max(DEFAULT_NORMAL_HZ)))
        }
        PressureTier::Nominal | PressureTier::Constrained | PressureTier::Critical => resolved,
    }
}

/// Computes the ADPF target work duration (`ns`) and HWUI refresh ceiling (`Hz`)
/// for the current frame state.
#[must_use]
pub fn resolve_target_performance(
    active_work: bool,
    vote: Option<FrameRate>,
    headroom: HeadroomSnapshot,
    display: Option<DisplayRefreshSpec>,
) -> PerformanceTarget {
    let display = display.unwrap_or_else(|| DisplayRefreshSpec::from_peak(DEFAULT_NORMAL_HZ));
    let pressure = headroom.pressure_tier();
    let max_refresh_hz = match pressure {
        PressureTier::Nominal => 0.0,
        PressureTier::Constrained => display.normal.max(DEFAULT_NORMAL_HZ),
        PressureTier::Critical => {
            CRITICAL_NON_FIXED_CAP_HZ.min(display.normal.max(DEFAULT_NORMAL_HZ))
        }
    };
    if !active_work {
        return PerformanceTarget {
            target_duration_nanos: 0,
            max_refresh_hz,
        };
    }
    let target_hz = vote.map_or_else(
        || match pressure {
            PressureTier::Critical => CRITICAL_NON_FIXED_CAP_HZ.min(display.normal),
            PressureTier::Nominal | PressureTier::Constrained => display.normal,
        },
        |rate| resolve_surface_refresh_rate(rate, headroom, display),
    );
    let duration = Duration::from_secs_f64(1.0 / f64::from(target_hz.max(1.0)));
    let target_duration_nanos = i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX);
    PerformanceTarget {
        target_duration_nanos,
        max_refresh_hz,
    }
}

/// Converts finite non-negative seconds into clamped nanoseconds for ADPF reporting.
#[cfg(any(all(target_os = "android", hydrolysis_gpu_painter), test))]
#[must_use]
pub fn seconds_to_nanos(seconds: f64) -> i64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    let duration = Duration::from_secs_f64(seconds);
    i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{
        DisplayRefreshSpec, HeadroomSnapshot, PerformanceTarget, PressureTier, ThermalStatus,
        effective_refresh_vote, resolve_surface_refresh_rate, resolve_target_performance,
        seconds_to_nanos,
    };
    use waterui_frame_rate::FrameRate;

    #[test]
    fn thermal_status_clamps_bounds() {
        assert_eq!(ThermalStatus::from_android_status(-3), ThermalStatus::None);
        assert_eq!(ThermalStatus::from_android_status(0), ThermalStatus::None);
        assert_eq!(
            ThermalStatus::from_android_status(2),
            ThermalStatus::Moderate
        );
        assert_eq!(
            ThermalStatus::from_android_status(4),
            ThermalStatus::Critical
        );
        assert_eq!(
            ThermalStatus::from_android_status(99),
            ThermalStatus::Shutdown
        );
    }

    #[test]
    fn headroom_pressure_tier_gates_on_thermal_and_compute_headroom() {
        assert_eq!(
            HeadroomSnapshot::from_jni(f32::NAN, f32::NAN, 0, f32::NAN).pressure_tier(),
            PressureTier::Nominal
        );
        assert_eq!(
            HeadroomSnapshot::from_jni(45.0, 60.0, 1, 0.85).pressure_tier(),
            PressureTier::Nominal
        );
        assert_eq!(
            HeadroomSnapshot::from_jni(9.5, 60.0, 0, 0.5).pressure_tier(),
            PressureTier::Constrained
        );
        assert_eq!(
            HeadroomSnapshot::from_jni(50.0, 10.0, 0, 0.5).pressure_tier(),
            PressureTier::Constrained
        );
        assert_eq!(
            HeadroomSnapshot::from_jni(50.0, 50.0, 2, 0.5).pressure_tier(),
            PressureTier::Constrained
        );
        assert_eq!(
            HeadroomSnapshot::from_jni(50.0, 50.0, 0, 0.90).pressure_tier(),
            PressureTier::Constrained
        );
        assert_eq!(
            HeadroomSnapshot::from_jni(50.0, 50.0, 0, 1.0).pressure_tier(),
            PressureTier::Critical
        );
        assert_eq!(
            HeadroomSnapshot::from_jni(50.0, 50.0, 4, 0.5).pressure_tier(),
            PressureTier::Critical
        );
    }

    #[test]
    fn constrained_headroom_steps_down_non_fixed_votes_and_preserves_fixed_sources() {
        let display = DisplayRefreshSpec {
            peak: 120.0,
            normal: 60.0,
            high: 120.0,
        };
        let constrained = HeadroomSnapshot::from_jni(8.0, f32::NAN, 0, f32::NAN);
        let fast = effective_refresh_vote(Some(FrameRate::FAST), constrained, display.normal)
            .expect("active vote survives");
        assert!(
            (display.resolve_rate(fast) - 60.0).abs() < 0.01,
            "constrained headroom must cap non-fixed FAST vote to 60 Hz"
        );
        let fixed =
            effective_refresh_vote(Some(FrameRate::fixed(24.0)), constrained, display.normal)
                .expect("fixed vote survives");
        assert!(fixed.is_fixed());
        assert_eq!(fixed, FrameRate::fixed(24.0));
        assert!(
            (resolve_surface_refresh_rate(
                FrameRate::FAST,
                HeadroomSnapshot::from_jni(f32::NAN, f32::NAN, 4, 1.0),
                display
            ) - 30.0)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn resolve_target_performance_closes_session_when_idle_and_computes_budgets() {
        let display = Some(DisplayRefreshSpec {
            peak: 120.0,
            normal: 60.0,
            high: 120.0,
        });
        let nominal = HeadroomSnapshot::default();
        assert_eq!(
            resolve_target_performance(false, Some(FrameRate::FAST), nominal, display),
            PerformanceTarget {
                target_duration_nanos: 0,
                max_refresh_hz: 0.0,
            }
        );

        let fast_target = resolve_target_performance(true, Some(FrameRate::FAST), nominal, display);
        assert_eq!(fast_target.target_duration_nanos, 8_333_333);
        assert!((fast_target.max_refresh_hz - 0.0).abs() < f32::EPSILON);

        let slow_target = resolve_target_performance(true, Some(FrameRate::SLOW), nominal, display);
        assert_eq!(slow_target.target_duration_nanos, 16_666_667);

        let constrained = HeadroomSnapshot::from_jni(f32::NAN, f32::NAN, 2, f32::NAN);
        let capped = resolve_target_performance(true, Some(FrameRate::FAST), constrained, display);
        assert_eq!(capped.target_duration_nanos, 16_666_667);
        assert!((capped.max_refresh_hz - 60.0).abs() < f32::EPSILON);

        let critical = HeadroomSnapshot::from_jni(f32::NAN, f32::NAN, 4, 1.0);
        let throttled = resolve_target_performance(true, Some(FrameRate::FAST), critical, display);
        assert_eq!(throttled.target_duration_nanos, 33_333_333);
        assert!((throttled.max_refresh_hz - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn seconds_to_nanos_handles_edge_inputs() {
        assert_eq!(seconds_to_nanos(0.0), 0);
        assert_eq!(seconds_to_nanos(-1.0), 0);
        assert_eq!(seconds_to_nanos(f64::NAN), 0);
        assert_eq!(seconds_to_nanos(0.0025), 2_500_000);
    }
}
