use core::{
    any::type_name,
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use std::{
    backtrace::Backtrace,
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

#[cfg(any(unix, windows))]
use cpu_time::ThreadTime;
use executor_core::LocalExecutor;
use minstant::Instant;

const FALLBACK_REFRESH_RATE_HZ: f64 = 60.0;

#[cfg(any(unix, windows))]
type CpuClockSample = ThreadTime;

#[cfg(not(any(unix, windows)))]
type CpuClockSample = ();

#[cfg(any(unix, windows))]
fn cpu_clock_now() -> CpuClockSample {
    ThreadTime::now()
}

#[cfg(not(any(unix, windows)))]
fn cpu_clock_now() -> CpuClockSample {
    ()
}

#[cfg(any(unix, windows))]
fn cpu_clock_elapsed(start: CpuClockSample) -> Duration {
    start.elapsed()
}

#[cfg(not(any(unix, windows)))]
fn cpu_clock_elapsed(_start: CpuClockSample) -> Duration {
    Duration::ZERO
}

/// Configuration for main-thread stall detection.
#[derive(Debug, Clone, Copy)]
pub struct MainThreadStallProbeConfig {
    /// Emit `info` once this ratio of frame budget is reached.
    pub info_ratio: f64,
    /// Emit `warn` once this ratio of frame budget is reached.
    pub warn_ratio: f64,
    /// Per-task cool-down for `info` logs.
    pub info_cooldown: Duration,
    /// Per-task cool-down for `warn` logs.
    pub warn_cooldown: Duration,
}

impl Default for MainThreadStallProbeConfig {
    fn default() -> Self {
        Self {
            info_ratio: 0.60,
            warn_ratio: 0.90,
            info_cooldown: Duration::from_secs(2),
            warn_cooldown: Duration::from_secs(1),
        }
    }
}

/// Per-poll runtime sample captured from a local task on the main thread.
#[derive(Debug, Clone, Copy)]
pub struct TaskPollSample {
    /// Type name of the spawned future.
    pub task_type: &'static str,
    /// Whether this poll finished the future.
    pub poll_ready: bool,
    /// Wall-clock duration spent in this poll.
    pub wall: Duration,
    /// CPU time consumed by this thread during this poll.
    pub cpu: Duration,
    /// Frame budget used for thresholding.
    pub frame_budget: Duration,
    /// Refresh rate used to derive frame budget.
    pub refresh_hz: f64,
}

/// Extension point for runtime diagnostics based on per-poll samples.
pub trait RuntimeProbe: Send + Sync + 'static {
    /// Consumes one per-poll sample.
    fn on_poll_sample(&self, sample: &TaskPollSample);
}

static EXTRA_RUNTIME_PROBES: OnceLock<Mutex<Vec<Arc<dyn RuntimeProbe>>>> = OnceLock::new();

fn global_runtime_probes() -> &'static Mutex<Vec<Arc<dyn RuntimeProbe>>> {
    EXTRA_RUNTIME_PROBES.get_or_init(|| Mutex::new(Vec::new()))
}

fn snapshot_runtime_probes() -> Vec<Arc<dyn RuntimeProbe>> {
    let guard = match global_runtime_probes().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.clone()
}

/// Registers an additional runtime probe.
///
/// Registered probes are attached to all monitored local executors created
/// after registration.
pub fn install_runtime_probe(probe: Arc<dyn RuntimeProbe>) {
    let mut guard = match global_runtime_probes().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.push(probe);
}

#[cfg(test)]
fn clear_runtime_probes_for_tests() {
    let mut guard = match global_runtime_probes().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.clear();
}

/// A local executor wrapper that instruments per-poll main-thread occupancy.
///
/// The wrapper measures each `poll()` slice of spawned futures and emits
/// `tracing` logs when wall time approaches/exceeds frame budget.
pub struct MonitoredLocalExecutor<E> {
    inner: E,
    state: Arc<MonitorState>,
}

impl<E> fmt::Debug for MonitoredLocalExecutor<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MonitoredLocalExecutor")
            .finish_non_exhaustive()
    }
}

impl<E> MonitoredLocalExecutor<E>
where
    E: LocalExecutor,
{
    /// Creates a monitored executor with default thresholds.
    #[must_use]
    pub fn new(inner: E) -> Self {
        Self::with_config(inner, MainThreadStallProbeConfig::default())
    }

    /// Creates a monitored executor with custom thresholds.
    #[must_use]
    pub fn with_config(inner: E, config: MainThreadStallProbeConfig) -> Self {
        let refresh_hz = detect_max_refresh_rate_hz();
        let frame_budget = Duration::from_secs_f64(1.0 / refresh_hz.max(1.0));
        let probes: Vec<Arc<dyn RuntimeProbe>> = vec![Arc::new(MainThreadStallProbe::new(config))];

        Self {
            inner,
            state: Arc::new(MonitorState {
                refresh_hz,
                frame_budget,
                probes,
            }),
        }
    }
}

impl<E> LocalExecutor for MonitoredLocalExecutor<E>
where
    E: LocalExecutor,
{
    type Task<T: 'static> = E::Task<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let guarded = GuardedFuture {
            inner: fut,
            task_type: type_name::<Fut>(),
            state: Arc::clone(&self.state),
        };
        self.inner.spawn_local(guarded)
    }
}

/// Wraps a local executor with main-thread stall instrumentation.
#[must_use]
pub fn monitored_local_executor<E>(inner: E) -> MonitoredLocalExecutor<E>
where
    E: LocalExecutor,
{
    MonitoredLocalExecutor::new(inner)
}

/// Wraps a local executor with custom main-thread stall settings.
#[must_use]
pub fn monitored_local_executor_with_config<E>(
    inner: E,
    config: MainThreadStallProbeConfig,
) -> MonitoredLocalExecutor<E>
where
    E: LocalExecutor,
{
    MonitoredLocalExecutor::with_config(inner, config)
}

struct GuardedFuture<F> {
    inner: F,
    task_type: &'static str,
    state: Arc<MonitorState>,
}

impl<F> Future for GuardedFuture<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We never move `inner` after pinning `Self`.
        let this = unsafe { self.get_unchecked_mut() };
        let wall_start = Instant::now();
        let cpu_start = cpu_clock_now();

        // SAFETY: `inner` is pinned together with `Self`.
        let poll_result = unsafe { Pin::new_unchecked(&mut this.inner) }.poll(cx);

        let sample = TaskPollSample {
            task_type: this.task_type,
            poll_ready: poll_result.is_ready(),
            wall: wall_start.elapsed(),
            cpu: cpu_clock_elapsed(cpu_start),
            frame_budget: this.state.frame_budget,
            refresh_hz: this.state.refresh_hz,
        };
        for probe in &this.state.probes {
            probe.on_poll_sample(&sample);
        }
        for probe in snapshot_runtime_probes() {
            probe.on_poll_sample(&sample);
        }

        poll_result
    }
}

struct MonitorState {
    refresh_hz: f64,
    frame_budget: Duration,
    probes: Vec<Arc<dyn RuntimeProbe>>,
}

#[derive(Debug)]
struct MainThreadStallProbe {
    config: MainThreadStallProbeConfig,
    last_emitted: Mutex<HashMap<RateLimitKey, Instant>>,
}

impl MainThreadStallProbe {
    fn new(config: MainThreadStallProbeConfig) -> Self {
        Self {
            config,
            last_emitted: Mutex::new(HashMap::new()),
        }
    }

    fn on_main_thread_poll(&self, sample: &TaskPollSample) {
        let frame_budget_secs = sample.frame_budget.as_secs_f64();
        if frame_budget_secs <= 0.0 {
            return;
        }

        let usage_ratio = sample.wall.as_secs_f64() / frame_budget_secs;
        let level = classify_level(usage_ratio, &self.config);
        let Some(level) = level else {
            return;
        };

        if !self.should_emit(sample.task_type, level) {
            return;
        }

        let wall_us = sample.wall.as_micros();
        let cpu_us = sample.cpu.as_micros();
        let budget_us = sample.frame_budget.as_micros();
        let overrun_us = sample.wall.saturating_sub(sample.frame_budget).as_micros();
        let usage_pct = usage_ratio * 100.0;

        match level {
            LogLevel::Info => {
                tracing::info!(
                    target: "waterui::runtime_guard",
                    task_type = sample.task_type,
                    poll_ready = sample.poll_ready,
                    wall_us,
                    cpu_us,
                    budget_us,
                    overrun_us,
                    usage_pct,
                    refresh_hz = sample.refresh_hz,
                    "Main-thread task poll is approaching frame budget"
                );
            }
            LogLevel::Warn => {
                let backtrace = Backtrace::force_capture();
                tracing::warn!(
                    target: "waterui::runtime_guard",
                    task_type = sample.task_type,
                    poll_ready = sample.poll_ready,
                    wall_us,
                    cpu_us,
                    budget_us,
                    overrun_us,
                    usage_pct,
                    refresh_hz = sample.refresh_hz,
                    backtrace = %backtrace,
                    "Main-thread task poll reached frame-budget warning threshold"
                );
            }
        }
    }

    fn should_emit(&self, task_type: &'static str, level: LogLevel) -> bool {
        let cooldown = match level {
            LogLevel::Info => self.config.info_cooldown,
            LogLevel::Warn => self.config.warn_cooldown,
        };
        let now = Instant::now();
        let key = RateLimitKey { task_type, level };

        let mut last_emitted = match self.last_emitted.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let stale_after = self
            .config
            .info_cooldown
            .max(self.config.warn_cooldown)
            .saturating_mul(4);
        last_emitted.retain(|_, previous| now.duration_since(*previous) <= stale_after);

        if let Some(previous) = last_emitted.get(&key)
            && now.duration_since(*previous) < cooldown
        {
            return false;
        }

        last_emitted.insert(key, now);
        true
    }
}

impl RuntimeProbe for MainThreadStallProbe {
    fn on_poll_sample(&self, sample: &TaskPollSample) {
        self.on_main_thread_poll(sample);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RateLimitKey {
    task_type: &'static str,
    level: LogLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LogLevel {
    Info,
    Warn,
}

fn classify_level(usage_ratio: f64, config: &MainThreadStallProbeConfig) -> Option<LogLevel> {
    if usage_ratio >= config.warn_ratio {
        Some(LogLevel::Warn)
    } else if usage_ratio >= config.info_ratio {
        Some(LogLevel::Info)
    } else {
        None
    }
}

fn detect_max_refresh_rate_hz() -> f64 {
    match waterkit_screen::max_refresh_rate_hz() {
        Ok(hz) if hz.is_finite() && hz > 0.0 => hz as f64,
        Ok(hz) => {
            tracing::debug!(
                target: "waterui::runtime_guard",
                invalid_refresh_hz = hz,
                fallback_refresh_hz = FALLBACK_REFRESH_RATE_HZ,
                "Display refresh rate metadata was invalid; using fallback"
            );
            FALLBACK_REFRESH_RATE_HZ
        }
        Err(waterkit_screen::Error::Unsupported | waterkit_screen::Error::MonitorNotFound) => {
            tracing::debug!(
                target: "waterui::runtime_guard",
                fallback_refresh_hz = FALLBACK_REFRESH_RATE_HZ,
                "Display refresh rate metadata is unavailable; using fallback"
            );
            FALLBACK_REFRESH_RATE_HZ
        }
        Err(err) => {
            tracing::info!(
                target: "waterui::runtime_guard",
                error = ?err,
                fallback_refresh_hz = FALLBACK_REFRESH_RATE_HZ,
                "Failed to read display refresh rate; using fallback"
            );
            FALLBACK_REFRESH_RATE_HZ
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use executor_core::LocalExecutor;

    use super::{LogLevel, MainThreadStallProbeConfig, classify_level};
    use super::{
        RuntimeProbe, TaskPollSample, clear_runtime_probes_for_tests, install_runtime_probe,
    };

    #[derive(Debug, Clone, Copy)]
    struct PollOnceExecutor;

    #[derive(Debug)]
    struct ImmediateTask<T>(Option<T>);

    impl<T> core::future::Future for ImmediateTask<T> {
        type Output = T;

        fn poll(
            self: core::pin::Pin<&mut Self>,
            _cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<Self::Output> {
            // SAFETY: ImmediateTask does not move its inner value after pinning.
            let this = unsafe { self.get_unchecked_mut() };
            core::task::Poll::Ready(
                this.0
                    .take()
                    .expect("ImmediateTask polled after completion"),
            )
        }
    }

    impl<T: 'static> executor_core::Task<T> for ImmediateTask<T> {
        fn poll_result(
            self: core::pin::Pin<&mut Self>,
            _cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<Result<T, Box<dyn core::any::Any + Send>>> {
            // SAFETY: ImmediateTask does not move its inner value after pinning.
            let this = unsafe { self.get_unchecked_mut() };
            core::task::Poll::Ready(Ok(this
                .0
                .take()
                .expect("ImmediateTask polled after completion")))
        }
    }

    impl LocalExecutor for PollOnceExecutor {
        type Task<T: 'static> = ImmediateTask<T>;

        fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
        where
            Fut: core::future::Future + 'static,
        {
            let waker = futures::task::noop_waker();
            let mut cx = core::task::Context::from_waker(&waker);
            let mut fut = Box::pin(fut);
            let output = match fut.as_mut().poll(&mut cx) {
                core::task::Poll::Ready(output) => output,
                core::task::Poll::Pending => {
                    panic!("PollOnceExecutor expects immediately-ready futures in tests")
                }
            };
            ImmediateTask(Some(output))
        }
    }

    #[derive(Debug)]
    struct CountingProbe(Arc<AtomicUsize>);

    impl RuntimeProbe for CountingProbe {
        fn on_poll_sample(&self, _sample: &TaskPollSample) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn level_classification_uses_expected_thresholds() {
        let config = MainThreadStallProbeConfig::default();
        assert_eq!(classify_level(0.59, &config), None);
        assert_eq!(classify_level(0.60, &config), Some(LogLevel::Info));
        assert_eq!(classify_level(0.89, &config), Some(LogLevel::Info));
        assert_eq!(classify_level(0.90, &config), Some(LogLevel::Warn));
    }

    #[test]
    fn installed_runtime_probe_is_attached_to_new_executor() {
        clear_runtime_probes_for_tests();
        let hits = Arc::new(AtomicUsize::new(0));
        install_runtime_probe(Arc::new(CountingProbe(Arc::clone(&hits))));

        let executor = super::MonitoredLocalExecutor::new(PollOnceExecutor);
        executor.spawn_local(async {});

        assert!(hits.load(Ordering::Relaxed) >= 1);
        clear_runtime_probes_for_tests();
    }
}
