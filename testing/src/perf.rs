use std::time::{Duration, Instant};

use waterui_core::View;

use crate::app::{OffscreenApp, ThemeInstaller, UiBuilder};
use crate::driver::FrameTiming;

const PERF_FRAME_RATE: u32 = 120;

/// Repeated offscreen render measurement configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfConfig {
    /// Number of unrecorded frames run before sampling.
    pub warmups: u32,
    /// Number of recorded frames per measurement.
    pub samples: u32,
    /// Number of independent measurement repetitions.
    pub repetitions: u32,
}

impl Default for PerfConfig {
    fn default() -> Self {
        Self {
            warmups: 10,
            samples: 120,
            repetitions: 7,
        }
    }
}

/// Aggregate render timing for one measured scenario.
#[derive(Clone, Debug)]
pub struct PerfMeasurement {
    /// Scenario name supplied to [`PerfApp::measure`].
    pub name: String,
    /// Recorded frame timings after warmup.
    pub frames: Vec<FrameTiming>,
}

impl PerfMeasurement {
    /// Computes aggregate statistics for this measurement.
    #[must_use]
    pub fn stats(&self) -> PerfStats {
        PerfStats::from_frames(&self.frames)
    }
}

/// Statistical summary of repeated Hydrolysis GPU frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerfStats {
    /// Number of sampled frames.
    pub samples: usize,
    /// Arithmetic mean frame duration.
    pub mean: Duration,
    /// Median frame duration.
    pub median: Duration,
    /// Fastest sampled frame.
    pub min: Duration,
    /// Slowest sampled frame.
    pub max: Duration,
    /// 95th percentile frame duration.
    pub p95: Duration,
    /// Number of sampled frames that rebuilt scene/layout state.
    pub rebuilt_frames: usize,
    /// Number of sampled frames that missed the 120fps frame budget.
    pub missed_120fps_frames: usize,
    /// Number of sampled frames that missed the 60fps frame budget.
    pub missed_60fps_frames: usize,
    /// Phase duration summaries for sampled frames.
    pub phases: PerfPhaseStats,
    /// Measurement cache hits across sampled frames.
    pub measurement_cache_hits: u64,
    /// Measurement cache misses across sampled frames.
    pub measurement_cache_misses: u64,
    /// Compositor layers submitted across sampled frames.
    pub scene_layers: u64,
    /// Vello scene layers submitted across sampled frames.
    pub vello_scene_layers: u64,
    /// Embedded GPU surface layers submitted across sampled frames.
    pub gpu_surface_layers: u64,
    /// Vello clip layers pushed across sampled frames.
    pub clip_layers: u64,
    /// Maximum nested Vello clip depth observed across sampled frames.
    pub max_clip_depth: u64,
}

/// Statistical summary for the measured frame phases.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerfPhaseStats {
    /// Time spent draining local executor work before input.
    pub executor_before: PerfDurationStats,
    /// Time spent dispatching pending input.
    pub input: PerfDurationStats,
    /// Time spent advancing animation and invalidation clocks.
    pub animation: PerfDurationStats,
    /// Time spent rebuilding scene/layout state.
    pub rebuild: PerfDurationStats,
    /// Time spent building the root WaterUI view value during scene rebuild.
    pub build_content: PerfDurationStats,
    /// Time spent dispatching WaterUI views into Hydrolysis scene/layout state.
    pub scene_dispatch: PerfDurationStats,
    /// Time spent finalizing layout, interaction, and accessibility state after dispatch.
    pub scene_finish: PerfDurationStats,
    /// Time spent acquiring an offscreen frame.
    pub acquire: PerfDurationStats,
    /// Time spent submitting Hydrolysis/Vello rendering work.
    pub render: PerfDurationStats,
    /// Time spent presenting the offscreen frame.
    pub present: PerfDurationStats,
    /// Time spent draining local executor work after rendering.
    pub executor_after: PerfDurationStats,
}

/// Duration summary for one measured value.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerfDurationStats {
    /// Arithmetic mean duration.
    pub mean: Duration,
    /// Median duration.
    pub median: Duration,
    /// 95th percentile duration.
    pub p95: Duration,
}

impl PerfStats {
    /// Builds a statistical summary from frame timings.
    #[must_use]
    pub fn from_frames(frames: &[FrameTiming]) -> Self {
        if frames.is_empty() {
            return Self::default();
        }
        let total = PerfDurationStats::from_durations(frames.iter().map(|frame| frame.total));
        let samples = frames.len();

        Self {
            samples,
            mean: total.mean,
            median: total.median,
            min: frames
                .iter()
                .map(|frame| frame.total)
                .min()
                .unwrap_or_default(),
            max: frames
                .iter()
                .map(|frame| frame.total)
                .max()
                .unwrap_or_default(),
            p95: total.p95,
            rebuilt_frames: frames.iter().filter(|frame| frame.rebuilt).count(),
            missed_120fps_frames: frames
                .iter()
                .filter(|frame| frame.total > Duration::from_nanos(8_333_333))
                .count(),
            missed_60fps_frames: frames
                .iter()
                .filter(|frame| frame.total > Duration::from_nanos(16_666_667))
                .count(),
            phases: PerfPhaseStats::from_frames(frames),
            measurement_cache_hits: frames
                .iter()
                .map(|frame| u64::from(frame.profile.counters.measurement_cache_hits))
                .sum(),
            measurement_cache_misses: frames
                .iter()
                .map(|frame| u64::from(frame.profile.counters.measurement_cache_misses))
                .sum(),
            scene_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile.counters.scene_layers))
                .sum(),
            vello_scene_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile.counters.vello_scene_layers))
                .sum(),
            gpu_surface_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile.counters.gpu_surface_layers))
                .sum(),
            clip_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile.counters.clip_layers))
                .sum(),
            max_clip_depth: frames
                .iter()
                .map(|frame| u64::from(frame.profile.counters.max_clip_depth))
                .max()
                .unwrap_or_default(),
        }
    }
}

impl PerfPhaseStats {
    fn from_frames(frames: &[FrameTiming]) -> Self {
        Self {
            executor_before: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile.phases.executor_before),
            ),
            input: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile.phases.input),
            ),
            animation: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile.phases.animation),
            ),
            rebuild: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile.phases.rebuild),
            ),
            build_content: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile.phases.build_content),
            ),
            scene_dispatch: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile.phases.scene_dispatch),
            ),
            scene_finish: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile.phases.scene_finish),
            ),
            acquire: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile.phases.acquire),
            ),
            render: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile.phases.render),
            ),
            present: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile.phases.present),
            ),
            executor_after: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile.phases.executor_after),
            ),
        }
    }
}

impl PerfDurationStats {
    fn from_durations(values: impl IntoIterator<Item = Duration>) -> Self {
        let mut durations = values.into_iter().collect::<Vec<_>>();
        if durations.is_empty() {
            return Self::default();
        }
        durations.sort_unstable();
        let sum = durations.iter().copied().sum::<Duration>();
        let samples = durations.len();
        let p95_index = ((samples - 1) * 95).div_ceil(100);
        Self {
            mean: sum / u32::try_from(samples).expect("perf sample count should fit u32"),
            median: durations[samples / 2],
            p95: durations[p95_index],
        }
    }
}

/// Result of a performance run.
#[derive(Clone, Debug, Default)]
pub struct PerfReport {
    measurements: Vec<PerfMeasurement>,
}

impl PerfReport {
    /// Returns all recorded measurements in insertion order.
    #[must_use]
    pub fn measurements(&self) -> &[PerfMeasurement] {
        &self.measurements
    }

    pub(crate) fn push(&mut self, measurement: PerfMeasurement) {
        self.measurements.push(measurement);
    }
}

/// Mutable app wrapper passed to performance automation closures.
#[derive(Debug)]
pub struct PerfRun<'a> {
    app: &'a mut OffscreenApp,
    frame_at: &'a mut Instant,
    frame_interval: Duration,
}

impl PerfRun<'_> {
    /// Advances one complete offscreen Hydrolysis GPU frame without snapshot readback.
    pub fn frame(&mut self) -> FrameTiming {
        let timing = self.app.app.driver.pump_frame_at(
            &self.app.app.content,
            &self.app.app.env,
            *self.frame_at,
        );
        *self.frame_at = self
            .frame_at
            .checked_add(self.frame_interval)
            .expect("perf frame clock overflow");
        timing
    }

    /// Queues a pointer move for the next measured frame without settling the app.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        let _ = self.app.app.driver.pointer_move(x, y, &self.app.app.env);
    }

    /// Queues a primary pointer down for the next measured frame without settling the app.
    pub fn pointer_down(&mut self, x: f32, y: f32) {
        let _ = self.app.app.driver.pointer_down(x, y, &self.app.app.env);
    }

    /// Queues a primary pointer up for the next measured frame without settling the app.
    pub fn pointer_up(&mut self, x: f32, y: f32) {
        let _ = self.app.app.driver.pointer_up(x, y, &self.app.app.env);
    }

    /// Queues a wheel/trackpad scroll event for the next measured frame without settling the app.
    pub fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) {
        let _ = self
            .app
            .app
            .driver
            .scroll_at(x, y, dx, dy, is_line_delta, &self.app.app.env);
    }

    /// Accesses semantic assertions and interactions during a performance run.
    #[must_use]
    pub fn app(&mut self) -> &mut OffscreenApp {
        self.app
    }
}

/// Records performance scenarios for one view.
pub struct PerfApp<T, F, V> {
    builder: UiBuilder<T>,
    view_fn: F,
    config: PerfConfig,
    report: PerfReport,
    _view: core::marker::PhantomData<fn() -> V>,
}

impl<T, F, V> core::fmt::Debug for PerfApp<T, F, V>
where
    T: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PerfApp")
            .field("builder", &self.builder)
            .field("config", &self.config)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

impl<T, F, V> PerfApp<T, F, V>
where
    T: ThemeInstaller,
    F: Fn() -> V + Clone + 'static,
    V: View + 'static,
{
    pub(crate) const fn new(builder: UiBuilder<T>, view_fn: F, config: PerfConfig) -> Self {
        Self {
            builder,
            view_fn,
            config,
            report: PerfReport {
                measurements: Vec::new(),
            },
            _view: core::marker::PhantomData,
        }
    }

    /// Measures one scenario across warmup and sample frames.
    pub fn measure<A>(&mut self, name: impl Into<String>, mut automation: A)
    where
        A: FnMut(&mut PerfRun<'_>),
    {
        let mut frames = Vec::with_capacity(
            usize::try_from(self.config.samples)
                .and_then(|samples| {
                    usize::try_from(self.config.repetitions).map(|repetitions| {
                        samples
                            .checked_mul(repetitions)
                            .expect("perf sample count should fit usize")
                    })
                })
                .expect("perf sample count should fit usize"),
        );
        for _ in 0..self.config.repetitions {
            let mut app = self.builder.clone().mount(self.view_fn.clone());
            let mut frame_at = Instant::now();
            let frame_interval = Duration::from_secs(1) / PERF_FRAME_RATE;
            for _ in 0..self.config.warmups {
                let mut run = PerfRun {
                    app: &mut app,
                    frame_at: &mut frame_at,
                    frame_interval,
                };
                automation(&mut run);
                let _ = run.frame();
            }

            for _ in 0..self.config.samples {
                let mut run = PerfRun {
                    app: &mut app,
                    frame_at: &mut frame_at,
                    frame_interval,
                };
                automation(&mut run);
                frames.push(run.frame());
            }
        }
        self.report.push(PerfMeasurement {
            name: name.into(),
            frames,
        });
    }

    /// Returns the number of scenarios recorded so far.
    #[must_use]
    pub fn measurement_count(&self) -> usize {
        self.report.measurements.len()
    }

    pub(crate) fn finish(self) -> PerfReport {
        self.report
    }
}
