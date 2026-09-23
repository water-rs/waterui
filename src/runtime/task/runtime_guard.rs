use core::{
    any::type_name,
    fmt,
    future::Future,
    num::NonZeroU32,
    pin::Pin,
    task::{Context, Poll},
};
use std::{
    backtrace::Backtrace,
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[cfg(all(any(unix, windows), not(target_os = "espidf")))]
use cpu_time::ThreadTime;
use executor_core::LocalExecutor;
use minstant::Instant;

/// The refresh rate a monitored executor derives its frame budget from.
///
/// It is supplied by the host that owns the main loop and the displays — the
/// same host that supplies the [`LocalExecutor`] itself. The executor never
/// queries a display: a headless or test host has none to query, and a
/// display-server connection opened from an arbitrary thread is exactly the
/// kind of process-global side effect an executor constructor must not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RefreshRate {
    millihertz: NonZeroU32,
}

impl RefreshRate {
    /// The rate for a host that drives no display — a test pump, a headless
    /// runner, a preview session. Frames there are paced by the host, not by a
    /// panel, so the budget is the conventional 60 Hz.
    pub const HEADLESS: Self = Self::from_hz(NonZeroU32::new(60).unwrap());

    /// A rate in millihertz, the unit windowing systems report
    /// (`winit::monitor::MonitorHandle::refresh_rate_millihertz`).
    #[must_use]
    pub const fn from_millihertz(millihertz: NonZeroU32) -> Self {
        Self { millihertz }
    }

    /// A whole-hertz rate.
    ///
    /// # Panics
    ///
    /// Panics if `hz` is too large to express in millihertz as a `u32`.
    #[must_use]
    pub const fn from_hz(hz: NonZeroU32) -> Self {
        match hz.checked_mul(NonZeroU32::new(1000).unwrap()) {
            Some(millihertz) => Self { millihertz },
            None => panic!("refresh rate in hertz overflows u32 millihertz"),
        }
    }

    /// The rate in hertz.
    #[must_use]
    pub fn hz(self) -> f64 {
        f64::from(self.millihertz.get()) / 1000.0
    }

    /// The time one frame may take at this rate.
    #[must_use]
    pub fn frame_budget(self) -> Duration {
        Duration::from_secs_f64(1000.0 / f64::from(self.millihertz.get()))
    }
}

#[cfg(all(any(unix, windows), not(target_os = "espidf")))]
type CpuClockSample = ThreadTime;

/// Stands in for [`ThreadTime`] on targets with no per-thread CPU clock, so the
/// sample a poll carries is a named "this platform cannot measure CPU time"
/// rather than a bare `()`.
#[cfg(not(all(any(unix, windows), not(target_os = "espidf"))))]
#[derive(Debug, Clone, Copy)]
struct NoCpuClock;

#[cfg(not(all(any(unix, windows), not(target_os = "espidf"))))]
type CpuClockSample = NoCpuClock;

#[cfg(all(any(unix, windows), not(target_os = "espidf")))]
fn cpu_clock_now() -> CpuClockSample {
    ThreadTime::now()
}

#[cfg(not(all(any(unix, windows), not(target_os = "espidf"))))]
const fn cpu_clock_now() -> CpuClockSample {
    NoCpuClock
}

#[cfg(all(any(unix, windows), not(target_os = "espidf")))]
fn cpu_clock_elapsed(start: CpuClockSample) -> Duration {
    start.elapsed()
}

#[cfg(not(all(any(unix, windows), not(target_os = "espidf"))))]
const fn cpu_clock_elapsed(_start: CpuClockSample) -> Duration {
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
    /// Creates a monitored executor with default thresholds, budgeting each
    /// frame for the host-supplied `refresh` rate.
    #[must_use]
    pub fn new(inner: E, refresh: RefreshRate) -> Self {
        Self::with_config(inner, refresh, MainThreadStallProbeConfig::default())
    }

    /// Creates a monitored executor with custom thresholds.
    #[must_use]
    pub fn with_config(inner: E, refresh: RefreshRate, config: MainThreadStallProbeConfig) -> Self {
        Self::with_config_and_probes(inner, refresh, config, [])
    }

    /// Creates a monitored executor with custom thresholds and additional
    /// explicitly owned runtime probes.
    #[must_use]
    pub fn with_config_and_probes(
        inner: E,
        refresh: RefreshRate,
        config: MainThreadStallProbeConfig,
        probes: impl IntoIterator<Item = Arc<dyn RuntimeProbe>>,
    ) -> Self {
        let probes =
            core::iter::once(Arc::new(MainThreadStallProbe::new(config)) as Arc<dyn RuntimeProbe>)
                .chain(probes)
                .collect();

        Self {
            inner,
            state: Arc::new(MonitorState {
                refresh_hz: refresh.hz(),
                frame_budget: refresh.frame_budget(),
                probes,
                outstanding: AtomicUsize::new(0),
                registered: AtomicBool::new(false),
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
        if !self.state.registered.swap(true, Ordering::SeqCst) {
            EXECUTOR_MONITORS.with(|monitors| {
                monitors.borrow_mut().push(Arc::downgrade(&self.state));
            });
        }
        self.state.outstanding.fetch_add(1, Ordering::SeqCst);
        let guarded = GuardedFuture {
            inner: fut,
            task_type: type_name::<Fut>(),
            state: Arc::clone(&self.state),
        };
        self.inner.spawn_local(guarded)
    }
}

/// Wraps a local executor with main-thread stall instrumentation, budgeting
/// each frame for the host-supplied `refresh` rate.
#[must_use]
pub fn monitored_local_executor<E>(inner: E, refresh: RefreshRate) -> MonitoredLocalExecutor<E>
where
    E: LocalExecutor,
{
    MonitoredLocalExecutor::new(inner, refresh)
}

/// Wraps a local executor with custom main-thread stall settings.
#[must_use]
pub fn monitored_local_executor_with_config<E>(
    inner: E,
    refresh: RefreshRate,
    config: MainThreadStallProbeConfig,
) -> MonitoredLocalExecutor<E>
where
    E: LocalExecutor,
{
    MonitoredLocalExecutor::with_config(inner, refresh, config)
}

/// Wraps a local executor with explicitly owned runtime probes.
#[must_use]
pub fn monitored_local_executor_with_probes<E>(
    inner: E,
    refresh: RefreshRate,
    probes: impl IntoIterator<Item = Arc<dyn RuntimeProbe>>,
) -> MonitoredLocalExecutor<E>
where
    E: LocalExecutor,
{
    MonitoredLocalExecutor::with_config_and_probes(
        inner,
        refresh,
        MainThreadStallProbeConfig::default(),
        probes,
    )
}

struct GuardedFuture<F> {
    inner: F,
    task_type: &'static str,
    state: Arc<MonitorState>,
}

impl<F> Drop for GuardedFuture<F> {
    fn drop(&mut self) {
        // Dropping is the one event every task ending shares: finishing,
        // cancellation, and executor teardown all land here.
        self.state.outstanding.fetch_sub(1, Ordering::SeqCst);
    }
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

        poll_result
    }
}

struct MonitorState {
    refresh_hz: f64,
    frame_budget: Duration,
    probes: Vec<Arc<dyn RuntimeProbe>>,
    /// `spawn_local` futures alive on the owning thread: queued, parked on an
    /// external wake, or mid-poll. Incremented at spawn, decremented when the
    /// guarded future is dropped.
    outstanding: AtomicUsize,
    /// Set once this state is listed in the owning thread's monitor registry.
    registered: AtomicBool,
}

std::thread_local! {
    /// Monitor states that have spawned work on this thread.
    ///
    /// The installed executor is type-erased inside `executor-core`'s
    /// thread-local, so this registry is how a driving host reaches the
    /// counters. `spawn_local` can only reach an executor through the thread
    /// that installed it, so every state registered here belongs to this
    /// thread's executor.
    static EXECUTOR_MONITORS: RefCell<Vec<Weak<MonitorState>>> =
        const { RefCell::new(Vec::new()) };
}

/// `spawn_local` tasks still alive on this thread's local executor — queued,
/// parked on an external wake, or mid-poll.
///
/// This is the completion signal a pending-runnable probe cannot express: a
/// task parked on a wall-clock timer or an in-flight network read holds no
/// queued runnable, so "nothing queued" does not mean "work done". Hosts that
/// drive the executor frame by frame — test pumps, preview and MCP sessions —
/// pace real time while this is nonzero so work that completes after real I/O
/// is published before they report the result.
///
/// The count drops when the task's future finishes or is dropped, so cancelled
/// work is covered.
///
/// The blind spot, by design: only work spawned through a monitored executor
/// is counted. A thread whose local executor was installed bare — a direct
/// `init_local_executor` without [`monitored_local_executor`] or
/// [`monitored_local_executor_with_probes`] — reports zero while work is
/// outstanding; "no monitored work" is truthfully zero and must not be
/// misread as quiescence. Threads with no executor installed at all likewise
/// report zero.
#[must_use]
pub fn outstanding_local_tasks() -> usize {
    EXECUTOR_MONITORS.with(|monitors| {
        let mut monitors = monitors.borrow_mut();
        let mut total = 0;
        monitors.retain(|state| {
            state.upgrade().is_some_and(|state| {
                total += state.outstanding.load(Ordering::SeqCst);
                true
            })
        });
        total
    })
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use executor_core::LocalExecutor;

    use super::{LogLevel, MainThreadStallProbeConfig, classify_level};
    use super::{RuntimeProbe, TaskPollSample};

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

    /// An executor that holds each future without polling it, so a spawn stays
    /// "parked" — the state a runnable-queue probe cannot observe — until the
    /// task handle is dropped.
    #[derive(Debug, Clone, Copy)]
    struct ParkingExecutor;

    struct ParkingTask<T> {
        _future: core::pin::Pin<Box<dyn core::future::Future<Output = ()>>>,
        _output: core::marker::PhantomData<fn() -> T>,
    }

    impl<T> core::fmt::Debug for ParkingTask<T> {
        fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            formatter
                .debug_struct("ParkingTask")
                .finish_non_exhaustive()
        }
    }

    impl<T> core::future::Future for ParkingTask<T> {
        type Output = T;

        fn poll(
            self: core::pin::Pin<&mut Self>,
            _cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<Self::Output> {
            core::task::Poll::Pending
        }
    }

    impl<T: 'static> executor_core::Task<T> for ParkingTask<T> {
        fn poll_result(
            self: core::pin::Pin<&mut Self>,
            _cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<Result<T, Box<dyn core::any::Any + Send>>> {
            core::task::Poll::Pending
        }
    }

    impl LocalExecutor for ParkingExecutor {
        type Task<T: 'static> = ParkingTask<T>;

        fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
        where
            Fut: core::future::Future + 'static,
        {
            ParkingTask {
                _future: Box::pin(async move {
                    let _ = fut.await;
                }),
                _output: core::marker::PhantomData,
            }
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
    fn explicit_runtime_probe_is_attached_to_executor() {
        let hits = Arc::new(AtomicUsize::new(0));
        let probe = Arc::new(CountingProbe(Arc::clone(&hits))) as Arc<dyn RuntimeProbe>;
        let executor = super::MonitoredLocalExecutor::with_config_and_probes(
            PollOnceExecutor,
            super::RefreshRate::HEADLESS,
            MainThreadStallProbeConfig::default(),
            [probe],
        );
        executor.spawn_local(async {});

        assert!(hits.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn outstanding_local_tasks_counts_work_parked_on_a_wake() {
        use super::{monitored_local_executor, outstanding_local_tasks};

        let executor = monitored_local_executor(ParkingExecutor, super::RefreshRate::HEADLESS);
        let parked = executor.spawn_local(core::future::pending::<()>());
        assert_eq!(
            outstanding_local_tasks(),
            1,
            "a parked future holds no runnable, so only the task count sees it"
        );

        drop(parked);
        assert_eq!(
            outstanding_local_tasks(),
            0,
            "dropping the task handle drops the guarded future"
        );
    }
}
