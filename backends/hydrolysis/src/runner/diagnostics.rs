//! Frame-render diagnostics reporting, configured from `WATERUI_HYDROLYSIS_RENDER_DIAG*`.

use super::*;

pub(super) const DEFAULT_RENDER_DIAG_INTERVAL_MS: u64 = 1_000;
pub(super) const DEFAULT_RENDER_DIAG_SLOW_FRAME_MS: u64 = 16;

#[derive(Clone, Copy)]
pub(super) struct RenderDiagnosticsConfig {
    pub(super) enabled: bool,
    pub(super) interval: Duration,
    /// Explicit slow-frame threshold from the env var, when the operator sets one.
    /// `None` means "derive from the display refresh rate" (one frame budget), with the
    /// 16ms constant used only as the no-monitor fallback.
    pub(super) slow_frame_threshold_override: Option<Duration>,
}

impl RenderDiagnosticsConfig {
    pub(super) fn from_env() -> Self {
        let enabled = parse_bool_env("hydrolysis runner", "WATERUI_HYDROLYSIS_RENDER_DIAG", false);
        let interval_ms = parse_positive_u64_env(
            "hydrolysis runner",
            "WATERUI_HYDROLYSIS_RENDER_DIAG_INTERVAL_MS",
            DEFAULT_RENDER_DIAG_INTERVAL_MS,
        );
        let slow_frame_threshold_override =
            std::env::var_os("WATERUI_HYDROLYSIS_RENDER_DIAG_SLOW_FRAME_MS").map(|_| {
                Duration::from_millis(parse_positive_u64_env(
                    "hydrolysis runner",
                    "WATERUI_HYDROLYSIS_RENDER_DIAG_SLOW_FRAME_MS",
                    DEFAULT_RENDER_DIAG_SLOW_FRAME_MS,
                ))
            });

        Self {
            enabled,
            interval: Duration::from_millis(interval_ms),
            slow_frame_threshold_override,
        }
    }
}

pub(super) struct RenderPhaseSample {
    pub(super) rebuild: Duration,
    pub(super) build_content: Duration,
    pub(super) scene_dispatch: Duration,
    pub(super) scene_finish: Duration,
    pub(super) acquire: Duration,
    pub(super) render: Duration,
    pub(super) present: Duration,
    pub(super) total: Duration,
    pub(super) rebuild_iterations: u32,
    pub(super) applied_filter_count: u32,
    pub(super) applied_filter_capture_us: u64,
    pub(super) applied_filter_effect_us: u64,
    pub(super) rebuilt: bool,
}

#[derive(Default)]
pub(super) struct RenderPhaseTotals {
    pub(super) frames: u64,
    pub(super) rebuild_frames: u64,
    pub(super) rebuild_iterations: u64,
    pub(super) slow_frames: u64,
    pub(super) rebuild: Duration,
    pub(super) build_content: Duration,
    pub(super) scene_dispatch: Duration,
    pub(super) scene_finish: Duration,
    pub(super) acquire: Duration,
    pub(super) render: Duration,
    pub(super) present: Duration,
    pub(super) total: Duration,
    pub(super) applied_filter_count: u64,
    pub(super) applied_filter_capture: Duration,
    pub(super) applied_filter_effect: Duration,
}

pub(super) struct RenderDiagnostics {
    pub(super) config: RenderDiagnosticsConfig,
    /// Effective slow-frame threshold: the env override if set, otherwise derived from
    /// the display refresh rate via [`Self::set_refresh_rate`] (16ms until known).
    slow_frame_threshold: Duration,
    pub(super) report_started_at: Instant,
    pub(super) totals: RenderPhaseTotals,
}

impl RenderDiagnostics {
    pub(super) fn new(config: RenderDiagnosticsConfig) -> Self {
        Self {
            slow_frame_threshold: config
                .slow_frame_threshold_override
                .unwrap_or_else(|| Duration::from_millis(DEFAULT_RENDER_DIAG_SLOW_FRAME_MS)),
            config,
            report_started_at: Instant::now(),
            totals: RenderPhaseTotals::default(),
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// Derive the slow-frame threshold from the display refresh rate (one frame budget),
    /// unless the operator pinned an explicit threshold via the env var.
    pub(super) fn set_refresh_rate(&mut self, refresh_hz: f64) {
        if self.config.slow_frame_threshold_override.is_some() {
            return;
        }
        if refresh_hz > 0.0 {
            self.slow_frame_threshold = Duration::from_secs_f64(1.0 / refresh_hz);
        }
    }

    pub(super) fn record_frame(&mut self, window_title: &str, sample: RenderPhaseSample) {
        if !self.config.enabled {
            return;
        }

        self.totals.frames = self
            .totals
            .frames
            .checked_add(1)
            .expect("hydrolysis runner: render diagnostics frame counter overflow");
        if sample.rebuilt {
            self.totals.rebuild_frames = self
                .totals
                .rebuild_frames
                .checked_add(1)
                .expect("hydrolysis runner: render diagnostics rebuild frame counter overflow");
        }
        self.totals.rebuild_iterations = self
            .totals
            .rebuild_iterations
            .checked_add(u64::from(sample.rebuild_iterations))
            .expect("hydrolysis runner: render diagnostics rebuild iteration counter overflow");
        self.totals.rebuild += sample.rebuild;
        self.totals.build_content += sample.build_content;
        self.totals.scene_dispatch += sample.scene_dispatch;
        self.totals.scene_finish += sample.scene_finish;
        self.totals.acquire += sample.acquire;
        self.totals.render += sample.render;
        self.totals.present += sample.present;
        self.totals.total += sample.total;
        self.totals.applied_filter_count = self
            .totals
            .applied_filter_count
            .checked_add(u64::from(sample.applied_filter_count))
            .expect("hydrolysis runner: render diagnostics applied filter counter overflow");
        self.totals.applied_filter_capture +=
            Duration::from_micros(sample.applied_filter_capture_us);
        self.totals.applied_filter_effect += Duration::from_micros(sample.applied_filter_effect_us);

        if sample.total >= self.slow_frame_threshold {
            self.totals.slow_frames = self
                .totals
                .slow_frames
                .checked_add(1)
                .expect("hydrolysis runner: render diagnostics slow frame counter overflow");
            tracing::warn!(
                target: "waterui::hydrolysis::render",
                window_title = %window_title,
                total_ms = duration_ms(sample.total),
                rebuild_ms = duration_ms(sample.rebuild),
                build_content_ms = duration_ms(sample.build_content),
                scene_dispatch_ms = duration_ms(sample.scene_dispatch),
                scene_finish_ms = duration_ms(sample.scene_finish),
                acquire_ms = duration_ms(sample.acquire),
                render_ms = duration_ms(sample.render),
                present_ms = duration_ms(sample.present),
                rebuild_iterations = sample.rebuild_iterations,
                applied_filter_count = sample.applied_filter_count,
                applied_filter_capture_ms =
                    duration_ms(Duration::from_micros(sample.applied_filter_capture_us)),
                applied_filter_effect_ms =
                    duration_ms(Duration::from_micros(sample.applied_filter_effect_us)),
                "Hydrolysis slow frame detected"
            );
        }

        self.maybe_report(window_title);
    }

    pub(super) fn maybe_report(&mut self, window_title: &str) {
        if self.totals.frames == 0 {
            return;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.report_started_at);
        if elapsed < self.config.interval {
            return;
        }

        let frame_count = self.totals.frames as f64;
        let avg_total_ms = duration_ms(self.totals.total) / frame_count;
        let avg_rebuild_ms = duration_ms(self.totals.rebuild) / frame_count;
        let avg_build_content_ms = duration_ms(self.totals.build_content) / frame_count;
        let avg_scene_dispatch_ms = duration_ms(self.totals.scene_dispatch) / frame_count;
        let avg_scene_finish_ms = duration_ms(self.totals.scene_finish) / frame_count;
        let avg_acquire_ms = duration_ms(self.totals.acquire) / frame_count;
        let avg_render_ms = duration_ms(self.totals.render) / frame_count;
        let avg_present_ms = duration_ms(self.totals.present) / frame_count;
        let avg_applied_filter_count = self.totals.applied_filter_count as f64 / frame_count;
        let avg_applied_filter_capture_ms =
            duration_ms(self.totals.applied_filter_capture) / frame_count;
        let avg_applied_filter_effect_ms =
            duration_ms(self.totals.applied_filter_effect) / frame_count;
        let rebuild_ratio = self.totals.rebuild_frames as f64 / frame_count;
        let avg_rebuild_iterations = self.totals.rebuild_iterations as f64 / frame_count;
        let fps = self.totals.frames as f64 / elapsed.as_secs_f64();

        tracing::info!(
            target: "waterui::hydrolysis::render",
            window_title = %window_title,
            frames = self.totals.frames,
            interval_ms = duration_ms(elapsed),
            fps,
            rebuild_frames = self.totals.rebuild_frames,
            rebuild_ratio,
            avg_rebuild_iterations,
            avg_total_ms,
            avg_rebuild_ms,
            avg_build_content_ms,
            avg_scene_dispatch_ms,
            avg_scene_finish_ms,
            avg_acquire_ms,
            avg_render_ms,
            avg_present_ms,
            avg_applied_filter_count,
            avg_applied_filter_capture_ms,
            avg_applied_filter_effect_ms,
            slow_frames = self.totals.slow_frames,
            slow_frame_threshold_ms = duration_ms(self.slow_frame_threshold),
            "Hydrolysis render diagnostics"
        );

        self.report_started_at = now;
        self.totals = RenderPhaseTotals::default();
    }
}

pub(super) fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

pub(super) fn elapsed_or_zero(started_at: Option<Instant>) -> Duration {
    started_at.map_or(Duration::ZERO, |value| value.elapsed())
}
