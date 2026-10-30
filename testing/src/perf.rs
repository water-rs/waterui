use std::rc::Rc;
use std::time::{Duration, Instant};

use hydrolysis::{PointerKind, Style};
use waterui_core::{AnyView, View};

use crate::app::{OffscreenApp, Styled, UiBuilder};
use crate::driver::{self, FrameTiming};

const PERF_FRAME_RATE: u32 = 120;

/// Repeated offscreen render measurement configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PerfConfig {
    /// Number of unrecorded frames run before sampling.
    warmups: u32,
    /// Number of recorded frames per measurement.
    samples: u32,
    /// Number of independent measurement repetitions.
    repetitions: u32,
}

impl PerfConfig {
    /// Creates a `PerfConfig` from its parts.
    #[must_use]
    pub const fn new(warmups: u32, samples: u32, repetitions: u32) -> Self {
        Self {
            warmups,
            samples,
            repetitions,
        }
    }

    /// Number of unrecorded frames run before sampling.
    #[must_use]
    pub const fn warmups(&self) -> u32 {
        self.warmups
    }

    /// Number of recorded frames per measurement.
    #[must_use]
    pub const fn samples(&self) -> u32 {
        self.samples
    }

    /// Number of independent measurement repetitions.
    #[must_use]
    pub const fn repetitions(&self) -> u32 {
        self.repetitions
    }
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
#[non_exhaustive]
pub struct PerfMeasurement {
    /// Scenario name supplied to [`PerfApp::measure`].
    name: String,
    /// Recorded frame timings after warmup.
    frames: Vec<FrameTiming>,
}

impl PerfMeasurement {
    /// Scenario name supplied to [`PerfApp::measure`].
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Recorded frame timings after warmup.
    #[must_use]
    pub const fn frames(&self) -> &[FrameTiming] {
        self.frames.as_slice()
    }
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
#[non_exhaustive]
pub struct PerfStats {
    /// Number of sampled frames.
    samples: usize,
    /// Arithmetic mean frame duration.
    mean: Duration,
    /// Median frame duration.
    median: Duration,
    /// Fastest sampled frame.
    min: Duration,
    /// Slowest sampled frame.
    max: Duration,
    /// 95th percentile frame duration.
    p95: Duration,
    /// Number of sampled frames that rebuilt scene/layout state.
    rebuilt_frames: usize,
    /// Number of sampled frames that rendered into the target surface.
    rendered_frames: usize,
    /// Number of sampled frames that had no render work.
    idle_frames: usize,
    /// Duration summary over rendered frames only.
    rendered_total: PerfDurationStats,
    /// Number of sampled frames that missed the 120fps frame budget.
    missed_120fps_frames: usize,
    /// Number of sampled frames that missed the 60fps frame budget.
    missed_60fps_frames: usize,
    /// Phase duration summaries for sampled frames.
    phases: PerfPhaseStats,
    /// Measurement cache hits across sampled frames.
    measurement_cache_hits: u64,
    /// Measurement cache misses across sampled frames.
    measurement_cache_misses: u64,
    /// Maximum compositor layers submitted by one sampled frame.
    scene_layers: u64,
    /// Maximum recorded scene-segment layers submitted by one sampled frame.
    scene_segment_layers: u64,
    /// Maximum embedded GPU content mounts submitted by one sampled frame.
    gpu_content_layers: u64,
    /// Maximum clip layers pushed by one sampled frame.
    clip_layers: u64,
    /// Maximum nested clip depth observed across sampled frames.
    max_clip_depth: u64,
    /// `AppliedFilter` nodes dispatched across sampled frames.
    applied_filter_count: u64,
    /// `AppliedFilter` subtree capture time across sampled frames, in microseconds.
    applied_filter_capture_us: u64,
    /// `AppliedFilter` GPU effect time across sampled frames, in microseconds.
    applied_filter_effect_us: u64,
}

impl PerfStats {
    /// Number of sampled frames.
    #[must_use]
    pub const fn samples(&self) -> usize {
        self.samples
    }

    /// Arithmetic mean frame duration.
    #[must_use]
    pub const fn mean(&self) -> Duration {
        self.mean
    }

    /// Median frame duration.
    #[must_use]
    pub const fn median(&self) -> Duration {
        self.median
    }

    /// Fastest sampled frame.
    #[must_use]
    pub const fn min(&self) -> Duration {
        self.min
    }

    /// Slowest sampled frame.
    #[must_use]
    pub const fn max(&self) -> Duration {
        self.max
    }

    /// 95th percentile frame duration.
    #[must_use]
    pub const fn p95(&self) -> Duration {
        self.p95
    }

    /// Number of sampled frames that rebuilt scene/layout state.
    #[must_use]
    pub const fn rebuilt_frames(&self) -> usize {
        self.rebuilt_frames
    }

    /// Number of sampled frames that rendered into the target surface.
    #[must_use]
    pub const fn rendered_frames(&self) -> usize {
        self.rendered_frames
    }

    /// Number of sampled frames that had no render work.
    #[must_use]
    pub const fn idle_frames(&self) -> usize {
        self.idle_frames
    }

    /// Duration summary over rendered frames only.
    #[must_use]
    pub const fn rendered_total(&self) -> PerfDurationStats {
        self.rendered_total
    }

    /// Number of sampled frames that missed the 120fps frame budget.
    #[must_use]
    pub const fn missed_120fps_frames(&self) -> usize {
        self.missed_120fps_frames
    }

    /// Number of sampled frames that missed the 60fps frame budget.
    #[must_use]
    pub const fn missed_60fps_frames(&self) -> usize {
        self.missed_60fps_frames
    }

    /// Phase duration summaries for sampled frames.
    #[must_use]
    pub const fn phases(&self) -> PerfPhaseStats {
        self.phases
    }

    /// Measurement cache hits across sampled frames.
    #[must_use]
    pub const fn measurement_cache_hits(&self) -> u64 {
        self.measurement_cache_hits
    }

    /// Measurement cache misses across sampled frames.
    #[must_use]
    pub const fn measurement_cache_misses(&self) -> u64 {
        self.measurement_cache_misses
    }

    /// Maximum compositor layers submitted by one sampled frame.
    #[must_use]
    pub const fn scene_layers(&self) -> u64 {
        self.scene_layers
    }

    /// Maximum recorded scene-segment layers submitted by one sampled frame.
    #[must_use]
    pub const fn scene_segment_layers(&self) -> u64 {
        self.scene_segment_layers
    }

    /// Maximum embedded GPU content mounts submitted by one sampled frame.
    #[must_use]
    pub const fn gpu_content_layers(&self) -> u64 {
        self.gpu_content_layers
    }

    /// Maximum clip layers pushed by one sampled frame.
    #[must_use]
    pub const fn clip_layers(&self) -> u64 {
        self.clip_layers
    }

    /// Maximum nested clip depth observed across sampled frames.
    #[must_use]
    pub const fn max_clip_depth(&self) -> u64 {
        self.max_clip_depth
    }

    /// `AppliedFilter` nodes dispatched across sampled frames.
    #[must_use]
    pub const fn applied_filter_count(&self) -> u64 {
        self.applied_filter_count
    }

    /// `AppliedFilter` subtree capture time across sampled frames, in microseconds.
    #[must_use]
    pub const fn applied_filter_capture_us(&self) -> u64 {
        self.applied_filter_capture_us
    }

    /// `AppliedFilter` GPU effect time across sampled frames, in microseconds.
    #[must_use]
    pub const fn applied_filter_effect_us(&self) -> u64 {
        self.applied_filter_effect_us
    }
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
    /// Time spent building the root `WaterUI` view value during scene rebuild.
    pub build_content: PerfDurationStats,
    /// Time spent dispatching `WaterUI` views into Hydrolysis scene/layout state.
    pub scene_dispatch: PerfDurationStats,
    /// Time spent finalizing layout, interaction, and accessibility state after dispatch.
    pub scene_finish: PerfDurationStats,
    /// Time spent acquiring an offscreen frame.
    pub acquire: PerfDurationStats,
    /// Time spent submitting Hydrolysis/Cherenkov rendering work.
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
    /// Slowest duration in the summarized set.
    pub max: Duration,
}

impl PerfStats {
    /// Builds a statistical summary from frame timings.
    #[must_use]
    pub fn from_frames(frames: &[FrameTiming]) -> Self {
        if frames.is_empty() {
            return Self::default();
        }
        let total = PerfDurationStats::from_durations(frames.iter().map(FrameTiming::total));
        let samples = frames.len();

        let rendered_frames = frames
            .iter()
            .filter(|frame| frame.profile().counters().rendered())
            .count();

        Self {
            samples,
            mean: total.mean,
            median: total.median,
            min: frames
                .iter()
                .map(FrameTiming::total)
                .min()
                .unwrap_or_default(),
            max: frames
                .iter()
                .map(FrameTiming::total)
                .max()
                .unwrap_or_default(),
            p95: total.p95,
            rebuilt_frames: frames.iter().filter(|frame| frame.rebuilt()).count(),
            rendered_frames,
            idle_frames: samples - rendered_frames,
            rendered_total: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .filter(|frame| frame.profile().counters().rendered())
                    .map(FrameTiming::total),
            ),
            missed_120fps_frames: frames
                .iter()
                .filter(|frame| frame.total() > Duration::from_nanos(8_333_333))
                .count(),
            missed_60fps_frames: frames
                .iter()
                .filter(|frame| frame.total() > Duration::from_nanos(16_666_667))
                .count(),
            phases: PerfPhaseStats::from_frames(frames),
            measurement_cache_hits: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().measurement_cache_hits()))
                .sum(),
            measurement_cache_misses: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().measurement_cache_misses()))
                .sum(),
            scene_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().scene_layers()))
                .max()
                .unwrap_or_default(),
            scene_segment_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().scene_segment_layers()))
                .max()
                .unwrap_or_default(),
            gpu_content_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().gpu_content_layers()))
                .max()
                .unwrap_or_default(),
            clip_layers: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().clip_layers()))
                .max()
                .unwrap_or_default(),
            max_clip_depth: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().max_clip_depth()))
                .max()
                .unwrap_or_default(),
            applied_filter_count: frames
                .iter()
                .map(|frame| u64::from(frame.profile().counters().applied_filter_count()))
                .sum(),
            applied_filter_capture_us: frames
                .iter()
                .map(|frame| frame.profile().counters().applied_filter_capture_us())
                .sum(),
            applied_filter_effect_us: frames
                .iter()
                .map(|frame| frame.profile().counters().applied_filter_effect_us())
                .sum(),
        }
    }
}

impl PerfPhaseStats {
    fn from_frames(frames: &[FrameTiming]) -> Self {
        Self {
            executor_before: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().executor_before()),
            ),
            input: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile().phases().input()),
            ),
            animation: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().animation()),
            ),
            rebuild: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().rebuild()),
            ),
            build_content: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().build_content()),
            ),
            scene_dispatch: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().scene_dispatch()),
            ),
            scene_finish: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().scene_finish()),
            ),
            acquire: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().acquire()),
            ),
            render: PerfDurationStats::from_durations(
                frames.iter().map(|frame| frame.profile().phases().render()),
            ),
            present: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().present()),
            ),
            executor_after: PerfDurationStats::from_durations(
                frames
                    .iter()
                    .map(|frame| frame.profile().phases().executor_after()),
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
            max: *durations.last().expect("duration list should not be empty"),
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
    ///
    /// # Panics
    ///
    /// Panics if the synthetic frame clock overflows.
    pub fn frame(&mut self) -> FrameTiming {
        let timing = self.app.app.pump_frame_at(*self.frame_at);
        *self.frame_at = self
            .frame_at
            .checked_add(self.frame_interval)
            .expect("perf frame clock overflow");
        timing
    }

    /// Queues a pointer move for the next measured frame without settling the app.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.app
            .app
            .runtime
            .push_input_event(driver::pointer_move_event(PointerKind::Mouse, x, y));
    }

    /// Queues a primary pointer down for the next measured frame without settling the app.
    pub fn pointer_down(&mut self, x: f32, y: f32) {
        self.app
            .app
            .runtime
            .push_input_event(driver::pointer_down_event(PointerKind::Mouse, x, y));
    }

    /// Queues a primary pointer up for the next measured frame without settling the app.
    pub fn pointer_up(&mut self, x: f32, y: f32) {
        self.app
            .app
            .runtime
            .push_input_event(driver::pointer_up_event(PointerKind::Mouse, x, y));
    }

    /// Queues a wheel/trackpad scroll event for the next measured frame without settling the app.
    pub fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) {
        self.app
            .app
            .runtime
            .push_input_event(driver::scroll_event(x, y, dx, dy, is_line_delta));
    }

    /// Requests a redraw for the next measured frame without changing semantic state.
    pub fn redraw(&mut self) {
        self.app.app.runtime.request_redraw();
    }

    /// Accesses semantic assertions and interactions during a performance run.
    #[must_use]
    pub const fn app(&mut self) -> &mut OffscreenApp {
        self.app
    }
}

/// Records performance scenarios for one view.
///
/// The builder — style included — and the view factory are erased internally,
/// so user-facing signatures (bench bodies, automation closures) stay a plain
/// `&mut PerfApp`.
pub struct PerfApp {
    mount: Box<dyn Fn() -> OffscreenApp>,
    config: PerfConfig,
    report: PerfReport,
}

impl core::fmt::Debug for PerfApp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PerfApp")
            .field("config", &self.config)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

impl PerfApp {
    pub(crate) fn new<F, V, S>(
        builder: UiBuilder<Styled<S>>,
        view_fn: F,
        config: PerfConfig,
    ) -> Self
    where
        F: Fn() -> V + 'static,
        V: View + 'static,
        S: Style + Clone,
    {
        let view_fn = Rc::new(move || AnyView::new(view_fn()));
        Self {
            mount: Box::new(move || {
                builder.clone().mount_offscreen({
                    let view_fn = Rc::clone(&view_fn);
                    move || view_fn()
                })
            }),
            config,
            report: PerfReport {
                measurements: Vec::new(),
            },
        }
    }

    /// Measures one scenario across warmup and sample frames.
    ///
    /// # Panics
    ///
    /// Panics if the configured sample count does not fit in memory on this target.
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
            let mut app = (self.mount)();
            // Seed from the session's virtual clock (the mount already pumped)
            // so perf frames and interleaved semantic pumps stay monotone.
            let mut frame_at = app.app.clock.unwrap_or_else(Instant::now);
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
    pub const fn measurement_count(&self) -> usize {
        self.report.measurements.len()
    }

    pub(crate) fn finish(self) -> PerfReport {
        self.report
    }
}
