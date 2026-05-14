use std::time::Duration;

use waterui_core::View;

use crate::app::{OffscreenApp, ThemeInstaller, UiBuilder};
use crate::driver::FrameTiming;

/// Repeated offscreen render measurement configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfConfig {
    /// Number of unrecorded frames run before sampling.
    pub warmups: u32,
    /// Number of recorded frames per measurement.
    pub samples: u32,
}

impl Default for PerfConfig {
    fn default() -> Self {
        Self {
            warmups: 5,
            samples: 60,
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
}

impl PerfStats {
    /// Builds a statistical summary from frame timings.
    #[must_use]
    pub fn from_frames(frames: &[FrameTiming]) -> Self {
        if frames.is_empty() {
            return Self::default();
        }
        let mut totals = frames.iter().map(|frame| frame.total).collect::<Vec<_>>();
        totals.sort_unstable();
        let sum = totals.iter().copied().sum::<Duration>();
        let samples = totals.len();
        let p95_index = ((samples - 1) * 95).div_ceil(100);

        Self {
            samples,
            mean: sum / u32::try_from(samples).expect("perf sample count should fit u32"),
            median: totals[samples / 2],
            min: totals[0],
            max: totals[samples - 1],
            p95: totals[p95_index],
            rebuilt_frames: frames.iter().filter(|frame| frame.rebuilt).count(),
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
}

impl PerfRun<'_> {
    /// Advances one complete offscreen Hydrolysis GPU frame without snapshot readback.
    pub fn frame(&mut self) -> FrameTiming {
        self.app
            .app
            .driver
            .pump_frame(&self.app.app.content, &self.app.app.env)
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
        let mut app = self.builder.clone().mount(self.view_fn.clone());
        for _ in 0..self.config.warmups {
            let mut run = PerfRun { app: &mut app };
            automation(&mut run);
            let _ = run.frame();
        }

        let mut frames = Vec::with_capacity(
            usize::try_from(self.config.samples).expect("perf sample count should fit usize"),
        );
        for _ in 0..self.config.samples {
            let mut run = PerfRun { app: &mut app };
            automation(&mut run);
            frames.push(run.frame());
        }
        self.report.push(PerfMeasurement {
            name: name.into(),
            frames,
        });
    }

    pub(crate) fn finish(self) -> PerfReport {
        self.report
    }
}
