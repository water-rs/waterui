//! The measurement harness for the `canvas-bench` app (water-rs/waterui#1564).
//!
//! One run is one launch: the run reads `WATERUI_BENCH_*` environment, waits
//! for a nominal-or-fair thermal state, warms up, measures a steady window,
//! and writes `bench-<run_id>.json` into the app's Documents directory.
//!
//! Metrics collected in-process:
//! - display-link frame intervals and hitches (`CADisplayLink` at the
//!   screen's maximum rate — it keeps ticking while the app is idle, which
//!   is what makes the idle window measurable);
//! - main-thread busy per frame, from `CFRunLoop` observers bracketing every
//!   run-loop iteration (the post marker runs after Core Animation's own
//!   commit observer, so `busy_ms` includes CA commit work; the `commit_ms`
//!   field is the window between the pre- and post-commit markers and
//!   approximates CA commit time);
//! - `phys_footprint` plus the task VM ledger and per-tag VM region sums
//!   (IOSurface, IOAccelerator/Metal, IOKit) sampled once a second;
//! - `MTLDevice.currentAllocatedSize` on a device created in-process.
//!
//! GPU time, render-server time and energy come from `xctrace` recordings
//! made alongside each run; this file only aligns them by recording the
//! steady window's media-time bounds.

#![cfg(target_os = "ios")]

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cocoa_ui::MainThreadMarker;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::{NSObject, NSObjectProtocol};
use cocoa_ui::objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use cocoa_ui::objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFIndex, CFOptionFlags, CFRetained, CFRunLoop, CFRunLoopActivity,
    CFRunLoopObserver, CFRunLoopObserverContext, CFRunLoopTimer, CFRunLoopTimerContext,
    kCFRunLoopCommonModes,
};
use cocoa_ui::objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState, NSRunLoop};
use cocoa_ui::objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice};
use cocoa_ui::objc2_quartz_core::{CADisplayLink, CAFrameRateRange, CACurrentMediaTime};
use serde::Serialize;
use tracing::info;

/// The launch configuration of one measurement run, from `WATERUI_BENCH_*`.
#[derive(Debug, Clone)]
pub struct BenchConfig {
    /// Scenario number (1 and 3 this round; 2 and 4 land with the CA
    /// lowering prototype).
    pub scenario: u32,
    /// Variant letter: `a`, `b` (or `c` for scenario 4).
    pub variant: String,
    /// Row / icon / surface count.
    pub n: u32,
    /// Run identity, unique per launch.
    pub run_id: String,
    /// Commit of the worktree the app was built from.
    pub commit: String,
    /// Warm-up seconds before the steady window opens.
    pub warmup_s: f64,
    /// Steady-window seconds.
    pub steady_s: f64,
    /// Seconds allowed for the thermal gate to clear before rejecting.
    pub thermal_wait_s: f64,
    /// Scenario 3: seconds of animation at the head of the steady window;
    /// the remainder is the idle window.
    pub animate_s: f64,
    /// Scenario 1: scroll continuously during the steady window.
    pub scroll: bool,
    /// Auto-scroll speed in points per second.
    pub scroll_speed: f64,
    /// Total content the scroll drives over, in points.
    pub scroll_extent: f64,
}

impl BenchConfig {
    /// Reads the run configuration from the environment; `None` when the
    /// app was not launched as a bench run (manual launch).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let get = |key: &str| std::env::var(key).ok();
        let scenario: u32 = get("WATERUI_BENCH_SCENARIO")?.parse().ok()?;
        Some(Self {
            scenario,
            variant: get("WATERUI_BENCH_VARIANT").unwrap_or_else(|| "a".into()),
            n: get("WATERUI_BENCH_N")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            run_id: get("WATERUI_BENCH_RUN_ID").unwrap_or_else(|| "manual".into()),
            commit: get("WATERUI_BENCH_COMMIT").unwrap_or_else(|| "unknown".into()),
            warmup_s: get("WATERUI_BENCH_WARMUP_S")
                .and_then(|v| v.parse().ok())
                .unwrap_or(5.0),
            steady_s: get("WATERUI_BENCH_STEADY_S")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30.0),
            thermal_wait_s: get("WATERUI_BENCH_THERMAL_WAIT_S")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300.0),
            animate_s: get("WATERUI_BENCH_ANIMATE_S")
                .and_then(|v| v.parse().ok())
                .unwrap_or(20.0),
            scroll: get("WATERUI_BENCH_SCROLL")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(true),
            scroll_speed: get("WATERUI_BENCH_SCROLL_SPEED")
                .and_then(|v| v.parse().ok())
                .unwrap_or(240.0),
            scroll_extent: get("WATERUI_BENCH_SCROLL_EXTENT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
        })
    }
}

/// Run state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting on the thermal gate.
    ThermalWait,
    /// Collecting nothing; letting the scene settle.
    Warmup,
    /// Recording.
    Steady,
    /// Result written; idle until the driver terminates the app.
    Done,
}

/// One display-link tick inside the steady window.
#[derive(Serialize, Clone)]
struct FrameSample {
    /// `CADisplayLink.timestamp`, media-time seconds.
    t: f64,
    /// Milliseconds since the previous tick.
    interval_ms: f64,
    /// Main-thread busy milliseconds attributed to this frame.
    busy_ms: f64,
    /// Milliseconds inside the post-commit observer window (approx. CA
    /// commit).
    commit_ms: f64,
}

/// One memory sample, taken once a second while recording.
#[derive(Serialize, Clone)]
struct MemSample {
    /// Media-time seconds.
    t: f64,
    /// `task_vm_info.phys_footprint`.
    phys_footprint: u64,
    /// `task_vm_info.internal` — dirty + compressed, no shared/file pages.
    internal: u64,
    /// `task_vm_info.ledger_tag_graphics_footprint` — graphics ledger.
    graphics_ledger: i64,
    /// `mach_vm_region` sums by user tag: IOSurface.
    iosurface: u64,
    /// Region sums by user tag: IOAccelerator (Metal resources).
    ioaccelerator: u64,
    /// Region sums by user tag: IOKit.
    iokit: u64,
    /// `MTLDevice.currentAllocatedSize` for the shared Metal device.
    metal_allocated: usize,
}

/// A phase transition, recorded in both clocks for trace alignment.
#[derive(Serialize, Clone)]
struct PhaseMark {
    name: &'static str,
    /// `CACurrentMediaTime` — mach-time seconds, matches trace timestamps.
    media_time: f64,
    /// `CFAbsoluteTimeGetCurrent` — seconds since 2001-01-01, wall aligned.
    cf_time: f64,
}

/// The whole run result written to `Documents/bench-<run_id>.json`.
#[derive(Serialize)]
struct RunResult {
    run_id: String,
    scenario: u32,
    variant: String,
    n: u32,
    commit: String,
    bundle_id: String,
    argv: Vec<String>,
    thermal_state_start: String,
    thermal_state_end: String,
    device_model: String,
    os_version: String,
    status: String,
    phases: Vec<PhaseMark>,
    /// Seconds the animation flag was set for (scenario 3); the frames
    /// after the `animate_end` mark form the idle window.
    animate_window_s: f64,
    frames: Vec<FrameSample>,
    mem_samples: Vec<MemSample>,
}

/// Mutable harness state — everything is main-thread.
struct Inner {
    phase: Phase,
    marks: Vec<PhaseMark>,
    frames: Vec<FrameSample>,
    mem: Vec<MemSample>,
    /// Start of the current run-loop iteration's busy segment, if the
    /// loop is awake.
    busy_start: Option<f64>,
    /// Busy seconds accumulated since the last display-link tick.
    pending_busy: f64,
    /// Timestamp of the pre-commit marker in this iteration.
    commit_start: Option<f64>,
    /// Commit-window seconds accumulated since the last tick.
    pending_commit: f64,
    /// Previous tick's `link.timestamp`.
    last_tick: Option<f64>,
    /// Last memory-sample tick.
    last_mem: f64,
    /// Scroll driver state: offset, direction.
    scroll_y: f64,
    scroll_dir: f64,
    /// Scenario 3 animation switch.
    animate: Arc<AtomicBool>,
    /// Media time at which animation stops (start of the idle window).
    animate_until: f64,
    /// Deadline for the thermal gate.
    thermal_deadline: f64,
    /// First-observed thermal state, for the report.
    thermal_state_start: String,
    /// Written already.
    done: bool,
}

impl Inner {
    fn mark(&mut self, name: &'static str) {
        self.marks.push(PhaseMark {
            name,
            media_time: CACurrentMediaTime(),
            cf_time: CFAbsoluteTimeGetCurrent(),
        });
    }

    fn tick(&mut self, config: &BenchConfig, link: &CADisplayLink, scroll: &ScrollDriver) {
        let t = link.timestamp();
        if self.phase != Phase::Steady {
            return;
        }
        // Scenario 3: stop the producers' redraws at the animation
        // deadline; the display link keeps ticking into the idle window.
        if self.animate.load(Ordering::Relaxed) && t >= self.animate_until {
            self.animate.store(false, Ordering::Relaxed);
            self.mark("animate_end");
        }
        if t - self.last_mem >= 1.0 {
            self.last_mem = t;
            self.mem.push(mem_sample());
        }
        let interval = self.last_tick.map(|prev| t - prev);
        if let Some(interval) = interval {
            self.frames.push(FrameSample {
                t,
                interval_ms: interval * 1e3,
                busy_ms: self.pending_busy * 1e3,
                commit_ms: self.pending_commit * 1e3,
            });
        }
        self.pending_busy = 0.0;
        self.pending_commit = 0.0;
        self.last_tick = Some(t);
        // Continuous scrolling: a triangle ramp clamped to the extent.
        if let (true, Some(dt)) = (
            config.scroll && config.scenario == 1 && config.scroll_extent > 0.0,
            interval,
        ) {
            self.scroll_y += self.scroll_dir * config.scroll_speed * dt.max(0.0);
            if self.scroll_y >= config.scroll_extent {
                self.scroll_y = config.scroll_extent;
                self.scroll_dir = -1.0;
            } else if self.scroll_y <= 0.0 {
                self.scroll_y = 0.0;
                self.scroll_dir = 1.0;
            }
            scroll.scroll_to(self.scroll_y);
        }
    }
}

/// The scroll drive the harness needs from the scene.
#[derive(Clone)]
pub enum ScrollDriver {
    /// The scene does not scroll.
    None,
    /// Drives a `ScrollController<Point>`'s vertical offset.
    Controller(waterui::layout::scroll::ScrollController<waterui::layout::Point>),
}

impl ScrollDriver {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "scroll offsets are points, f64 -> f32"
    )]
    fn scroll_to(&self, y: f64) {
        if let Self::Controller(controller) = self {
            controller.scroll_to(waterui::layout::Point::new(0.0, y as f32));
        }
    }
}

struct TickIvars {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
    config: RefCell<Option<BenchConfig>>,
    scroll: RefCell<ScrollDriver>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the target only
    // forwards `tick:` to the main-thread collector.
    #[unsafe(super(NSObject))]
    #[name = "CanvasBenchTickTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TickIvars]
    /// The `CADisplayLink` action target for the bench harness.
    struct TickTarget;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for TickTarget {}

    impl TickTarget {
        // SAFETY: `tick:` is the signature `CADisplayLink` posts, always on
        // the run loop it was added to — the main one.
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            let inner = self.ivars().inner.borrow().clone();
            let config = self.ivars().config.borrow().clone();
            let scroll = self.ivars().scroll.borrow().clone();
            if let (Some(inner), Some(config)) = (inner, config) {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    inner.borrow_mut().tick(&config, link, &scroll);
                }));
                // A panic must not unwind through Objective-C frames.
                if result.is_err() {
                    std::process::abort();
                }
            }
        }
    }
);

impl TickTarget {
    fn new(
        mtm: MainThreadMarker,
        inner: Rc<RefCell<Inner>>,
        config: BenchConfig,
        scroll: ScrollDriver,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TickIvars {
            inner: RefCell::new(Some(inner)),
            config: RefCell::new(Some(config)),
            scroll: RefCell::new(scroll),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Observer orders: Core Animation registers its transaction-commit
/// observer at order 2_000_000, so `ORDER_PRE_COMMIT` runs just before it
/// and `CFIndex::MAX` after every late observer, commit included.
const ORDER_PRE_COMMIT: CFIndex = 1_999_999;
const ORDER_POST_COMMIT: CFIndex = CFIndex::MAX;

/// The harness installed for the lifetime of one bench run.
pub struct Harness {
    _link: Retained<CADisplayLink>,
    _target: Retained<TickTarget>,
    _observers: Vec<CFRetained<CFRunLoopObserver>>,
}

/// The scene-side handles the harness drives.
pub struct SceneHandles {
    /// Scroll drive, when the scene scrolls.
    pub scroll: ScrollDriver,
    /// Producer-side animation switch, when the scene animates.
    pub animate: Arc<AtomicBool>,
}

/// Installs the harness: display link, run-loop observers, and the phase
/// machine driving thermal-gate → warm-up → steady → write.
#[must_use]
pub fn install(config: &BenchConfig, mtm: MainThreadMarker, handles: SceneHandles) -> Harness {
    let animate_until = if config.scenario == 3 {
        CACurrentMediaTime() + config.warmup_s + config.animate_s
    } else {
        f64::INFINITY
    };
    let inner = Rc::new(RefCell::new(Inner {
        phase: Phase::ThermalWait,
        marks: Vec::new(),
        frames: Vec::new(),
        mem: Vec::new(),
        busy_start: None,
        pending_busy: 0.0,
        commit_start: None,
        pending_commit: 0.0,
        last_tick: None,
        last_mem: 0.0,
        scroll_y: 0.0,
        scroll_dir: 1.0,
        animate: handles.animate.clone(),
        animate_until,
        thermal_deadline: CACurrentMediaTime() + config.thermal_wait_s,
        thermal_state_start: thermal_state_name(),
        done: false,
    }));
    inner.borrow_mut().mark("launch");

    info!(
        run_id = %config.run_id,
        scenario = config.scenario,
        variant = %config.variant,
        n = config.n,
        commit = %config.commit,
        argv = ?std::env::args().collect::<Vec<_>>(),
        "bench run configured"
    );

    let target = TickTarget::new(mtm, inner.clone(), config.clone(), handles.scroll);
    // SAFETY: `displayLinkWithTarget:selector:` retains the pair until the
    // link invalidates; the ivars own the Rust state.
    let link: Retained<CADisplayLink> = unsafe {
        msg_send![cocoa_ui::objc2::class!(CADisplayLink), displayLinkWithTarget:&*target, selector:sel!(tick:)]
    };
    link.setPreferredFrameRateRange(CAFrameRateRange::new(30.0, 120.0, 120.0));
    // SAFETY: `addToRunLoop:forMode:` schedules the link on this run loop;
    // common modes keeps it ticking through tracking.
    unsafe {
        link.addToRunLoop_forMode(
            &NSRunLoop::currentRunLoop(),
            cocoa_ui::objc2_foundation::NSRunLoopCommonModes,
        );
    }

    let observers = install_observers(&inner);

    // The phase machine starts on the thermal gate.
    run_after(&inner, config, 1.0, TimerStep::ThermalCheck);

    Harness {
        _link: link,
        _target: target,
        _observers: observers,
    }
}

/// Installs the three run-loop observers feeding busy/commit accounting.
fn install_observers(inner: &Rc<RefCell<Inner>>) -> Vec<CFRetained<CFRunLoopObserver>> {
    unsafe extern "C-unwind" fn after_waiting(
        _observer: *mut CFRunLoopObserver,
        _activity: CFRunLoopActivity,
        info: *mut core::ffi::c_void,
    ) {
        let weak = unsafe { &*(info as *const Weak<RefCell<Inner>>) };
        if let Some(inner) = weak.upgrade() {
            inner.borrow_mut().busy_start = Some(CFAbsoluteTimeGetCurrent());
        }
    }
    unsafe extern "C-unwind" fn pre_commit(
        _observer: *mut CFRunLoopObserver,
        _activity: CFRunLoopActivity,
        info: *mut core::ffi::c_void,
    ) {
        let weak = unsafe { &*(info as *const Weak<RefCell<Inner>>) };
        if let Some(inner) = weak.upgrade() {
            inner.borrow_mut().commit_start = Some(CFAbsoluteTimeGetCurrent());
        }
    }
    unsafe extern "C-unwind" fn post_commit(
        _observer: *mut CFRunLoopObserver,
        _activity: CFRunLoopActivity,
        info: *mut core::ffi::c_void,
    ) {
        let weak = unsafe { &*(info as *const Weak<RefCell<Inner>>) };
        if let Some(inner) = weak.upgrade() {
            let now = CFAbsoluteTimeGetCurrent();
            let mut inner = inner.borrow_mut();
            if let Some(start) = inner.busy_start.take() {
                inner.pending_busy += (now - start).max(0.0);
            }
            if let Some(start) = inner.commit_start.take() {
                inner.pending_commit += (now - start).max(0.0);
            }
        }
    }

    // One leaked `Weak` backs every observer context: observers live for
    // the harness's lifetime, which is the app's.
    let weak: &'static Weak<RefCell<Inner>> = Box::leak(Box::new(Rc::downgrade(inner)));
    let mut context = CFRunLoopObserverContext {
        version: 0,
        info: (weak as *const Weak<RefCell<Inner>>).cast_mut().cast(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    let run_loop = CFRunLoop::main().expect("main run loop exists");
    let mut observers = Vec::new();
    for (activities, order, callback) in [
        (
            CFRunLoopActivity::AfterWaiting,
            CFIndex::MIN,
            after_waiting as unsafe extern "C-unwind" fn(_, _, _),
        ),
        (
            CFRunLoopActivity::BeforeWaiting,
            ORDER_PRE_COMMIT,
            pre_commit as unsafe extern "C-unwind" fn(_, _, _),
        ),
        (
            CFRunLoopActivity::BeforeWaiting,
            ORDER_POST_COMMIT,
            post_commit as unsafe extern "C-unwind" fn(_, _, _),
        ),
    ] {
        // SAFETY: valid activities and order; `context.info` aliases the
        // leaked `Weak`, which outlives every observer.
        if let Some(observer) = unsafe {
            CFRunLoopObserver::new(
                None,
                activities.0 as CFOptionFlags,
                true,
                order,
                Some(callback),
                &mut context,
            )
        } {
            run_loop.add_observer(Some(&observer), unsafe { kCFRunLoopCommonModes });
            observers.push(observer);
        }
    }
    observers
}

/// State shared by every phase timer: the collector and its config.
struct TimerCtx {
    inner: Weak<RefCell<Inner>>,
    config: BenchConfig,
    /// Which step fires next.
    step: TimerStep,
}

#[derive(Clone, Copy)]
enum TimerStep {
    ThermalCheck,
    SteadyStart,
    Finish,
}

/// Fires a one-shot `CFRunLoopTimer` whose context is a `TimerCtx`.
fn run_after(inner: &Rc<RefCell<Inner>>, config: &BenchConfig, seconds: f64, step: TimerStep) {
    let ctx = Box::new(TimerCtx {
        inner: Rc::downgrade(inner),
        config: config.clone(),
        step,
    });
    let mut context = CFRunLoopTimerContext {
        version: 0,
        info: Box::into_raw(ctx).cast(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    unsafe extern "C-unwind" fn fired(_timer: *mut CFRunLoopTimer, info: *mut core::ffi::c_void) {
        // SAFETY: `info` is the `TimerCtx` box `run_after` installed; the
        // timer is one-shot and invalidated below, so the box is reclaimed
        // exactly once.
        let ctx = unsafe { Box::from_raw(info as *mut TimerCtx) };
        let Some(inner) = ctx.inner.upgrade() else {
            return;
        };
        match ctx.step {
            TimerStep::ThermalCheck => thermal_step(inner, ctx.config),
            TimerStep::SteadyStart => steady_step(inner, ctx.config),
            TimerStep::Finish => finish_step(inner, ctx.config),
        }
    }
    let run_loop = CFRunLoop::main().expect("main run loop exists");
    let fire_date = CFAbsoluteTimeGetCurrent() + seconds.max(0.001);
    // SAFETY: the context box is reclaimed in `fired` when the one-shot
    // timer fires; a `None` creation (only on allocator failure) frees it
    // here instead.
    let timer = unsafe {
        CFRunLoopTimer::new(None, fire_date, 0.0, 0, 0, Some(fired), &mut context)
    };
    match timer {
        Some(timer) => run_loop.add_timer(Some(&timer), unsafe { kCFRunLoopCommonModes }),
        None => drop(unsafe { Box::from_raw(context.info as *mut TimerCtx) }),
    }
}

/// The thermal gate: re-arm every 2 s until nominal/fair or the deadline.
fn thermal_step(inner: Rc<RefCell<Inner>>, config: BenchConfig) {
    let state = thermal_state();
    let now = CACurrentMediaTime();
    {
        let mut borrowed = inner.borrow_mut();
        if borrowed.done {
            return;
        }
        match state {
            NSProcessInfoThermalState::Nominal | NSProcessInfoThermalState::Fair => {
                borrowed.phase = Phase::Warmup;
                borrowed.mark("thermal_ok");
                borrowed.mark("warmup_start");
            }
            _ if now >= borrowed.thermal_deadline => {
                borrowed.mark("thermal_rejected");
                borrowed.done = true;
                let json = result_json(&borrowed, &config, "thermal_rejected");
                write_result(&json, &config.run_id);
                info!(run_id = %config.run_id, ?state, "bench thermal rejected");
                return;
            }
            _ => {}
        }
    }
    match state {
        NSProcessInfoThermalState::Nominal | NSProcessInfoThermalState::Fair => {
            run_after(&inner, &config, config.warmup_s, TimerStep::SteadyStart);
        }
        _ => run_after(&inner, &config, 2.0, TimerStep::ThermalCheck),
    }
}

/// Opens the steady window.
fn steady_step(inner: Rc<RefCell<Inner>>, config: BenchConfig) {
    {
        let mut borrowed = inner.borrow_mut();
        if borrowed.done {
            return;
        }
        borrowed.phase = Phase::Steady;
        borrowed.mark("steady_start");
        borrowed.last_tick = None;
        borrowed.pending_busy = 0.0;
        borrowed.pending_commit = 0.0;
        borrowed.last_mem = CACurrentMediaTime() - 1.0;
        borrowed.animate.store(true, Ordering::Relaxed);
    }
    run_after(&inner, &config, config.steady_s, TimerStep::Finish);
}

/// Closes the steady window and writes the result file.
fn finish_step(inner: Rc<RefCell<Inner>>, config: BenchConfig) {
    let mut borrowed = inner.borrow_mut();
    if borrowed.done {
        return;
    }
    borrowed.done = true;
    borrowed.phase = Phase::Done;
    borrowed.animate.store(false, Ordering::Relaxed);
    borrowed.mark("steady_end");
    let json = result_json(&borrowed, &config, "ok");
    write_result(&json, &config.run_id);
    info!(
        run_id = %config.run_id,
        frames = borrowed.frames.len(),
        mem_samples = borrowed.mem.len(),
        "bench run complete"
    );
}

/// The process thermal state.
fn thermal_state() -> NSProcessInfoThermalState {
    NSProcessInfo::processInfo().thermalState()
}

/// The thermal state's issue name.
fn thermal_state_name() -> String {
    match thermal_state() {
        NSProcessInfoThermalState::Nominal => "nominal".into(),
        NSProcessInfoThermalState::Fair => "fair".into(),
        NSProcessInfoThermalState::Serious => "serious".into(),
        NSProcessInfoThermalState::Critical => "critical".into(),
        _ => "unknown".into(),
    }
}

/// Builds the serializable result from the collected state.
fn result_json(inner: &Inner, config: &BenchConfig, status: &str) -> String {
    let result = RunResult {
        run_id: config.run_id.clone(),
        scenario: config.scenario,
        variant: config.variant.clone(),
        n: config.n,
        commit: config.commit.clone(),
        bundle_id: "dev.waterui.canvas_bench".into(),
        argv: std::env::args().collect(),
        thermal_state_start: inner.thermal_state_start.clone(),
        thermal_state_end: thermal_state_name(),
        device_model: device_model(),
        os_version: NSProcessInfo::processInfo()
            .operatingSystemVersionString()
            .to_string(),
        status: status.into(),
        phases: inner.marks.clone(),
        animate_window_s: config.animate_s,
        frames: inner.frames.clone(),
        mem_samples: inner.mem.clone(),
    };
    serde_json::to_string_pretty(&result).expect("result serialization")
}

/// Writes `json` to `Documents/bench-<run_id>.json` inside the app sandbox.
fn write_result(json: &str, run_id: &str) {
    let path = std::env::home_dir()
        .unwrap_or_else(|| "/tmp".into())
        .join("Documents")
        .join(format!("bench-{run_id}.json"));
    match std::fs::write(&path, json) {
        Ok(()) => info!(run_id = %run_id, path = %path.display(), "BENCH_DONE"),
        Err(error) => tracing::error!(run_id = %run_id, %error, "result write failed"),
    }
}

/// `utsname.machine` — e.g. `iPhone17,1`.
fn device_model() -> String {
    let mut name: libc::utsname = unsafe { core::mem::zeroed() };
    if unsafe { libc::uname(&mut name) } != 0 {
        return "unknown".into();
    }
    let bytes = unsafe {
        core::slice::from_raw_parts(name.machine.as_ptr().cast::<u8>(), name.machine.len())
    };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// `task_vm_info` flavor — REV7 layout (iOS 17+).
#[repr(C)]
struct TaskVmInfo {
    virtual_size: u64,
    region_count: i32,
    page_size: i32,
    resident_size: u64,
    resident_size_peak: u64,
    device: u64,
    device_peak: u64,
    internal: u64,
    internal_peak: u64,
    external: u64,
    external_peak: u64,
    reusable: u64,
    reusable_peak: u64,
    purgeable_volatile_pmap: u64,
    purgeable_volatile_resident: u64,
    purgeable_volatile_virtual: u64,
    compressed: u64,
    compressed_peak: u64,
    compressed_lifetime: u64,
    phys_footprint: u64,
    min_address: u64,
    max_address: u64,
    ledger_phys_footprint_peak: i64,
    ledger_purgeable_nonvolatile: i64,
    ledger_purgeable_novolatile_compressed: i64,
    ledger_purgeable_volatile: i64,
    ledger_purgeable_volatile_compressed: i64,
    ledger_tag_network_nonvolatile: i64,
    ledger_tag_network_nonvolatile_compressed: i64,
    ledger_tag_network_volatile: i64,
    ledger_tag_network_volatile_compressed: i64,
    ledger_tag_media_footprint: i64,
    ledger_tag_media_footprint_compressed: i64,
    ledger_tag_media_nofootprint: i64,
    ledger_tag_media_nofootprint_compressed: i64,
    ledger_tag_graphics_footprint: i64,
    ledger_tag_graphics_footprint_compressed: i64,
    ledger_tag_graphics_nofootprint: i64,
    ledger_tag_graphics_nofootprint_compressed: i64,
    ledger_tag_neural_footprint: i64,
    ledger_tag_neural_footprint_compressed: i64,
    ledger_tag_neural_nofootprint: i64,
    ledger_tag_neural_nofootprint_compressed: i64,
    limit_bytes_remaining: u64,
    decompressions: i32,
    ledger_swapins: i64,
    ledger_tag_neural_nofootprint_total: i64,
    ledger_tag_neural_nofootprint_peak: i64,
}

const TASK_VM_INFO: u32 = 22;
const TASK_VM_INFO_COUNT: u32 =
    (core::mem::size_of::<TaskVmInfo>() / core::mem::size_of::<u32>()) as u32;

/// `VM_REGION_BASIC_INFO_64` reply layout.
#[repr(C)]
struct VmRegionBasicInfo64 {
    protection: i32,
    max_protection: i32,
    inheritance: i32,
    shared: u32,
    reserved: u32,
    offset: u64,
    behavior: i32,
    user_wired_count: u16,
    user_tag: u16,
}

const VM_REGION_BASIC_INFO_64: i32 = 9;
const VM_REGION_BASIC_INFO_COUNT_64: u32 =
    (core::mem::size_of::<VmRegionBasicInfo64>() / core::mem::size_of::<u32>()) as u32;

const VM_MEMORY_IOKIT: u16 = 21;
const VM_MEMORY_IOSURFACE: u16 = 88;
const VM_MEMORY_IOACCELERATOR: u16 = 100;

unsafe extern "C-unwind" {
    /// The task port for this process — `libc` gates its export to macOS.
    static mach_task_self_: u32;
    fn mach_vm_region(
        target_task: libc::mach_port_t,
        address: *mut u64,
        size: *mut u64,
        flavor: i32,
        info: *mut VmRegionBasicInfo64,
        count: *mut u32,
        object_name: *mut u32,
    ) -> i32;
}

/// Sums `mach_vm_region` extents by user tag for the three graphics tags.
fn region_tag_sums() -> (u64, u64, u64) {
    let (mut iosurface, mut ioaccel, mut iokit) = (0u64, 0u64, 0u64);
    let task = unsafe { mach_task_self_ };
    let mut address = 1u64;
    loop {
        let mut size = 0u64;
        let mut info = VmRegionBasicInfo64 {
            protection: 0,
            max_protection: 0,
            inheritance: 0,
            shared: 0,
            reserved: 0,
            offset: 0,
            behavior: 0,
            user_wired_count: 0,
            user_tag: 0,
        };
        let mut count = VM_REGION_BASIC_INFO_COUNT_64;
        let mut object = 0u32;
        let kr = unsafe {
            mach_vm_region(
                task,
                &mut address,
                &mut size,
                VM_REGION_BASIC_INFO_64,
                &mut info,
                &mut count,
                &mut object,
            )
        };
        if kr != 0 {
            break;
        }
        match info.user_tag {
            VM_MEMORY_IOSURFACE => iosurface += size,
            VM_MEMORY_IOACCELERATOR => ioaccel += size,
            VM_MEMORY_IOKIT => iokit += size,
            _ => {}
        }
        address += size;
    }
    (iosurface, ioaccel, iokit)
}

/// One memory sample: task VM ledger plus the per-tag region sums and the
/// shared Metal device's allocation counter.
fn mem_sample() -> MemSample {
    let mut info: TaskVmInfo = unsafe { core::mem::zeroed() };
    let mut count = TASK_VM_INFO_COUNT;
    let kr = unsafe {
        libc::task_info(
            mach_task_self_,
            TASK_VM_INFO,
            (&mut info as *mut TaskVmInfo).cast::<i32>(),
            &mut count,
        )
    };
    let _ = kr;
    let (iosurface, ioaccel, iokit) = region_tag_sums();
    MemSample {
        t: CACurrentMediaTime(),
        phys_footprint: info.phys_footprint,
        internal: info.internal,
        graphics_ledger: info.ledger_tag_graphics_footprint,
        iosurface,
        ioaccelerator: ioaccel,
        iokit,
        metal_allocated: metal_allocated_size(),
    }
}

/// `MTLCreateSystemDefaultDevice().currentAllocatedSize` — the counter is
/// device-global on Apple silicon, so a fresh handle still reports all
/// Metal allocations the process made.
fn metal_allocated_size() -> usize {
    thread_local! {
        static DEVICE: Option<Retained<objc2::runtime::ProtocolObject<dyn MTLDevice>>> =
            MTLCreateSystemDefaultDevice();
    }
    DEVICE.with(|device| {
        device
            .as_ref()
            .map_or(0, |device| device.currentAllocatedSize())
    })
}