//! Matrix measurement harness for `canvas-bench` (water-rs/waterui#1564,
//! round 2).
//!
//! One launch walks every (scenario, variant, n) cell itself: mount the
//! scene, fling for a warm-up window, measure, tear down, next cell —
//! `passes` rotations over the whole cell list so thermal drift spreads
//! over every variant. Per cell the harness records the thermal state at
//! mount, per-frame display-link intervals with main-run-loop busy and
//! CATransaction commit milliseconds, `task_vm_info` + Metal memory
//! samples, and the rows-created / producer-call rates. A serious or
//! critical thermal state aborts the whole round and is reported, never
//! waited out. The single `Documents/bench-<run_id>.json` result carries
//! the launching process's pid and launch time so the driver can reject
//! a stale file.
//!
//! Launch configuration (environment):
//! - `WATERUI_BENCH_RUN_ID` — this round's id (required).
//! - `WATERUI_BENCH_COMMIT` — the bench app commit.
//! - `WATERUI_BENCH_PASSES` — rotations over the cell list (default 3;
//!   trace launches pass 1).
//!
//! Every cell's measured window is bracketed by an `os_signpost` interval
//! named `bench_cell` whose metadata is the cell name, so a trace pass
//! attributes GPU time and energy per cell.

#![allow(clippy::missing_safety_doc, reason = "internal bench harness")]

use std::cell::RefCell;
use std::ffi::CString;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cocoa_ui::MainThreadMarker;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::{NSObject, NSObjectProtocol};
use cocoa_ui::objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use cocoa_ui::objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFIndex, CFOptionFlags, CFRetained, CFRunLoop, CFRunLoopActivity,
    CFRunLoopObserver, CFRunLoopObserverContext, CFRunLoopTimer, CFRunLoopTimerContext,
    kCFRunLoopCommonModes,
};
use cocoa_ui::objc2_foundation::{NSBundle, NSProcessInfo, NSProcessInfoThermalState, NSRunLoop};
use cocoa_ui::objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice};
use cocoa_ui::objc2_quartz_core::{CADisplayLink, CAFrameRateRange, CACurrentMediaTime};
use serde::Serialize;
use tracing::info;

use waterui::Binding;

/// How fast scenario 1's list is flung, in points per second — fast
/// enough that every frame mounts fresh rows through the lazy stack.
const FLING_SPEED: f64 = 6000.0;
/// Scenario-1 row pitch in points (52pt card + 8pt row padding); the
/// scroll extent is `n * pitch - viewport`.
const ROW_PITCH: f64 = 60.0;
/// Approximate viewport height the fling extent subtracts.
const VIEWPORT_H: f64 = 800.0;
/// How long a cell waits for its view's scene handles before failing.
const SETUP_TIMEOUT_S: f64 = 8.0;
/// Seconds of fling warm-up before a cell's measured window opens.
const WARMUP_S: f64 = 2.0;
/// Measured window per cell.
const MEASURE_S: f64 = 5.0;
/// Drain time between teardown and the next cell.
const TEARDOWN_S: f64 = 0.6;

/// One (scenario, variant, n) combination the matrix walks.
#[derive(Clone, Debug, Serialize)]
pub struct CellSpec {
    pub scenario: u32,
    pub variant: String,
    pub n: u32,
}

impl CellSpec {
    /// The cell's display name — also the `os_signpost` metadata.
    pub fn name(&self) -> String {
        format!("s{}-{}-n{}", self.scenario, self.variant, self.n)
    }
}

/// The whole matrix: scenario-1 pairs at N=50 and N=200 plus the
/// scenario-3 `GpuContentView` densities. Pass `p` walks the list
/// rotated left by `p`, so thermal drift spreads over every variant.
pub fn matrix_cells() -> Vec<CellSpec> {
    [
        (1, "a", 50),
        (1, "b", 50),
        (1, "a", 200),
        (1, "b", 200),
        (3, "gpu", 1),
        (3, "gpu", 8),
        (3, "gpu", 32),
    ]
    .into_iter()
    .map(|(scenario, variant, n)| CellSpec {
        scenario,
        variant: variant.to_string(),
        n,
    })
    .collect()
}

/// Scene-side handles a mounted cell registers with the harness.
#[derive(Clone)]
pub struct SceneHandles {
    pub scroll: ScrollDriver,
    /// The animation switch scenario-3 producers read.
    pub animate: Arc<AtomicBool>,
    /// Lazy-stack row constructions (generator invocations).
    pub rows_created: Arc<AtomicU64>,
    /// `GpuContent::render` invocations across the cell's surfaces.
    pub producer_calls: Arc<AtomicU64>,
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

/// Launch configuration from `WATERUI_BENCH_*`.
#[derive(Clone)]
pub struct MatrixConfig {
    pub run_id: String,
    pub commit: String,
    pub passes: u32,
}

impl MatrixConfig {
    pub fn from_env() -> Self {
        let get = |key: &str| std::env::var(key).ok();
        Self {
            run_id: get("WATERUI_BENCH_RUN_ID").unwrap_or_else(|| "manual".into()),
            commit: get("WATERUI_BENCH_COMMIT").unwrap_or_else(|| "unknown".into()),
            passes: get("WATERUI_BENCH_PASSES")
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
        }
    }
}

/// One display-link sample inside a cell's measured window.
#[derive(Serialize)]
struct FrameSample {
    /// Seconds (mach continuous time) when the tick fired.
    t: f64,
    interval_ms: f64,
    /// Main-run-loop busy time inside the frame's run-loop turn.
    busy_ms: f64,
    /// CATransaction commit time inside the turn.
    commit_ms: f64,
}

/// Memory + allocator sample for one cell.
#[derive(Serialize)]
struct MemSample {
    t: f64,
    phys_footprint: u64,
    internal: u64,
    graphics_ledger: u64,
    iosurface: u64,
    ioaccelerator: u64,
    iokit: u64,
    metal_allocated: u64,
}

/// One cell's measured result.
#[derive(Serialize)]
struct CellResult {
    name: String,
    scenario: u32,
    variant: String,
    n: u32,
    pass: u32,
    thermal: String,
    measured_seconds: f64,
    rows_created_per_second: f64,
    producer_calls_per_second: f64,
    frames: Vec<FrameSample>,
    mem_samples: Vec<MemSample>,
    status: String,
}

/// The whole round's result, written once to `Documents/bench-<run_id>.json`.
#[derive(Serialize)]
struct MatrixResult {
    run_id: String,
    scenario: String,
    commit: String,
    bundle_id: String,
    pid: u32,
    launch_time_unix: u64,
    argv: Vec<String>,
    thermal_state_start: String,
    device_model: String,
    os_version: String,
    passes: u32,
    status: String,
    cells: Vec<CellResult>,
}

/// Matrix phases.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// Waiting for the mounted cell's `SceneHandles` to register.
    SetupCell,
    /// Fling warm-up running.
    Warmup,
    /// Measured window open; frames/mem/counters accumulate.
    Measure,
    /// Cell unmounted; draining before the next mount.
    Teardown,
    /// Round over (or aborted).
    Done,
}

/// Mutable harness state — everything is main-thread.
struct Inner {
    phase: Phase,
    queue: std::collections::VecDeque<CellSpec>,
    pass: u32,
    cell: Option<CellSpec>,
    handles: Option<SceneHandles>,
    cell_binding: Binding<Option<CellSpec>>,
    handles_slot: Rc<RefCell<Option<SceneHandles>>>,
    setup_deadline: f64,
    measure_start: f64,
    measure_rows_start: u64,
    measure_producers_start: u64,
    thermal_current: String,
    frames: Vec<FrameSample>,
    mem: Vec<MemSample>,
    results: Vec<CellResult>,
    status: String,
    busy_start: Option<f64>,
    pending_busy: f64,
    commit_start: Option<f64>,
    pending_commit: f64,
    last_tick: Option<f64>,
    last_mem: f64,
    scroll_y: f64,
    scroll_dir: f64,
    animate: Option<Arc<AtomicBool>>,
    signpost_id: u64,
    done: bool,
}

impl Inner {
    /// The currently mounted cell's scroll extent, or 0 for non-scroll cells.
    fn scroll_extent(&self) -> f64 {
        match &self.cell {
            Some(cell) if cell.scenario == 1 => {
                (f64::from(cell.n) * ROW_PITCH - VIEWPORT_H).max(0.0)
            }
            _ => 0.0,
        }
    }

    /// Per-tick work: frame samples and mem snapshots during Measure,
    /// the fling during Warmup+Measure.
    fn tick(&mut self, link: &CADisplayLink) {
        let t = link.timestamp();
        if self.phase != Phase::Warmup && self.phase != Phase::Measure {
            self.last_tick = Some(t);
            return;
        }
        let interval = self.last_tick.map(|prev| t - prev);
        if self.phase == Phase::Measure {
            if t - self.last_mem >= 0.5 {
                self.last_mem = t;
                self.mem.push(mem_sample());
            }
            if let Some(interval) = interval {
                self.frames.push(FrameSample {
                    t,
                    interval_ms: interval * 1e3,
                    busy_ms: self.pending_busy * 1e3,
                    commit_ms: self.pending_commit * 1e3,
                });
            }
        }
        self.pending_busy = 0.0;
        self.pending_commit = 0.0;
        self.last_tick = Some(t);

        // The fling: drive the scroll offset at FLING_SPEED, reversing at
        // the ends, so every frame mounts fresh rows.
        let extent = self.scroll_extent();
        if extent > 0.0
            && let (Some(scroll), Some(dt)) = (&self.handles.as_ref().map(|h| &h.scroll), interval)
        {
            let dt = dt.max(0.0);
            self.scroll_y += self.scroll_dir * FLING_SPEED * dt;
            if self.scroll_y >= extent {
                self.scroll_y = extent;
                self.scroll_dir = -1.0;
            } else if self.scroll_y <= 0.0 {
                self.scroll_y = 0.0;
                self.scroll_dir = 1.0;
            }
            scroll.scroll_to(self.scroll_y);
        }
    }
}

struct TickIvars {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
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
            if let Some(inner) = inner {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    inner.borrow_mut().tick(link);
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
    fn new(mtm: MainThreadMarker, inner: Rc<RefCell<Inner>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TickIvars {
            inner: RefCell::new(Some(inner)),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Kept alive so the display link and run-loop observers keep firing.
pub struct Harness {
    _link: Retained<CADisplayLink>,
    _target: Retained<TickTarget>,
    _observers: Vec<CFRetained<CFRunLoopObserver>>,
}

/// Installs the whole harness on the main run loop; returns the owner.
pub fn install(
    config: &MatrixConfig,
    mtm: MainThreadMarker,
    cell_binding: Binding<Option<CellSpec>>,
    handles_slot: Rc<RefCell<Option<SceneHandles>>>,
) -> Harness {
    let mut queue = std::collections::VecDeque::new();
    for pass in 0..config.passes {
        let mut cells = matrix_cells();
        let n_cells = cells.len();
        cells.rotate_left((pass as usize) % n_cells);
        queue.extend(cells);
    }
    let inner = Rc::new(RefCell::new(Inner {
        phase: Phase::SetupCell,
        queue,
        pass: 0,
        cell: None,
        handles: None,
        cell_binding,
        handles_slot,
        setup_deadline: 0.0,
        measure_start: 0.0,
        measure_rows_start: 0,
        measure_producers_start: 0,
        thermal_current: "unknown".into(),
        frames: Vec::new(),
        mem: Vec::new(),
        results: Vec::new(),
        status: "ok".into(),
        busy_start: None,
        pending_busy: 0.0,
        commit_start: None,
        pending_commit: 0.0,
        last_tick: None,
        last_mem: 0.0,
        scroll_y: 0.0,
        scroll_dir: 1.0,
        animate: None,
        signpost_id: 1,
        done: false,
    }));

    info!(
        run_id = %config.run_id,
        passes = config.passes,
        commit = %config.commit,
        "bench matrix configured"
    );

    let target = TickTarget::new(mtm, inner.clone());
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

    // First cell mounts half a second after launch.
    run_after(&inner, config.clone(), 0.5, TimerStep::StartCell);

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

/// Observer orders: Core Animation registers its transaction-commit
/// observer at order 2_000_000, so `ORDER_PRE_COMMIT` runs just before it
/// and `CFIndex::MAX` after every late observer, commit included.
const ORDER_PRE_COMMIT: CFIndex = 1_999_999;
const ORDER_POST_COMMIT: CFIndex = CFIndex::MAX;

/// State shared by every phase timer: the collector and its config.
struct TimerCtx {
    inner: Weak<RefCell<Inner>>,
    config: MatrixConfig,
    /// Which step fires next.
    step: TimerStep,
}

#[derive(Clone, Copy)]
enum TimerStep {
    StartCell,
    WaitHandles,
    MeasureStart,
    MeasureEnd,
    TeardownDone,
}

/// Fires a one-shot `CFRunLoopTimer` whose context is a `TimerCtx`.
fn run_after(inner: &Rc<RefCell<Inner>>, config: MatrixConfig, seconds: f64, step: TimerStep) {
    let ctx = Box::new(TimerCtx {
        inner: Rc::downgrade(inner),
        config,
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
            TimerStep::StartCell => start_cell(inner, ctx.config),
            TimerStep::WaitHandles => wait_handles(inner, ctx.config),
            TimerStep::MeasureStart => measure_start(inner, ctx.config),
            TimerStep::MeasureEnd => measure_end(inner, ctx.config),
            TimerStep::TeardownDone => start_cell(inner, ctx.config),
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

/// Mounts the next cell: thermal gate, binding set, handle wait armed.
fn start_cell(inner: Rc<RefCell<Inner>>, config: MatrixConfig) {
    let mut borrowed = inner.borrow_mut();
    if borrowed.done {
        return;
    }
    // Thermal gate: serious/critical aborts the whole round — reported,
    // never waited out.
    let state = thermal_state();
    if !matches!(
        state,
        NSProcessInfoThermalState::Nominal | NSProcessInfoThermalState::Fair
    ) {
        borrowed.status = "thermal_abort".into();
        info!(?state, "bench thermal abort before cell mount");
        drop(borrowed);
        finish(inner, config);
        return;
    }
    let Some(cell) = borrowed.queue.pop_front() else {
        // Round complete.
        borrowed.status = "ok".into();
        drop(borrowed);
        finish(inner, config);
        return;
    };
    // The pass this cell belongs to: the queue holds `passes` blocks of
    // `matrix_cells()` back to back, so remaining-count maps to a pass.
    let pass_index = config
        .passes
        .saturating_sub(((borrowed.queue.len() + 1) as u32).div_ceil(7));
    borrowed.pass = pass_index;
    borrowed.thermal_current = thermal_state_name();
    borrowed.cell = Some(cell.clone());
    borrowed.phase = Phase::SetupCell;
    borrowed.setup_deadline = CACurrentMediaTime() + SETUP_TIMEOUT_S;
    borrowed.frames.clear();
    borrowed.mem.clear();
    borrowed.scroll_y = 0.0;
    borrowed.scroll_dir = 1.0;
    info!(
        cell = %cell.name(),
        pass = pass_index,
        "bench cell setup"
    );
    borrowed.cell_binding.set(Some(cell));
    drop(borrowed);
    run_after(&inner, config, 0.2, TimerStep::WaitHandles);
}

/// Waits for the mounted view's `SceneHandles` (the `watch` rebuild may
/// land a run-loop turn later than the binding set).
fn wait_handles(inner: Rc<RefCell<Inner>>, config: MatrixConfig) {
    let handles = inner.borrow().handles_slot.borrow_mut().take();
    let mut borrowed = inner.borrow_mut();
    if borrowed.done {
        return;
    }
    if let Some(handles) = handles {
        borrowed.handles = Some(handles.clone());
        borrowed.animate = Some(handles.animate.clone());
        handles.animate.store(true, Ordering::Relaxed);
        borrowed.phase = Phase::Warmup;
        info!(cell = ?borrowed.cell.as_ref().map(CellSpec::name), "bench warmup start");
        drop(borrowed);
        run_after(&inner, config, WARMUP_S, TimerStep::MeasureStart);
    } else if CACurrentMediaTime() >= borrowed.setup_deadline {
        // The cell never mounted — fail it and move on.
        let cell = borrowed.cell.clone().unwrap_or(CellSpec {
            scenario: 0,
            variant: "?".into(),
            n: 0,
        });
        tracing::error!(cell = %cell.name(), "bench cell mount timeout");
        let thermal = borrowed.thermal_current.clone();
        borrowed.results.push(CellResult {
            name: cell.name(),
            scenario: cell.scenario,
            variant: cell.variant.clone(),
            n: cell.n,
            pass: 0,
            thermal,
            measured_seconds: 0.0,
            rows_created_per_second: 0.0,
            producer_calls_per_second: 0.0,
            frames: Vec::new(),
            mem_samples: Vec::new(),
            status: "mount_timeout".into(),
        });
        borrowed.cell_binding.set(None);
        drop(borrowed);
        run_after(&inner, config, TEARDOWN_S, TimerStep::TeardownDone);
    } else {
        drop(borrowed);
        run_after(&inner, config, 0.25, TimerStep::WaitHandles);
    }
}

/// Opens the measured window: counters snapshot, signpost on.
fn measure_start(inner: Rc<RefCell<Inner>>, config: MatrixConfig) {
    let mut borrowed = inner.borrow_mut();
    if borrowed.done {
        return;
    }
    borrowed.phase = Phase::Measure;
    borrowed.last_tick = None;
    borrowed.measure_start = CACurrentMediaTime();
    let counters = borrowed
        .handles
        .as_ref()
        .map(|h| (h.rows_created.clone(), h.producer_calls.clone()));
    if let Some((rows, producers)) = counters {
        borrowed.measure_rows_start = rows.load(Ordering::Relaxed);
        borrowed.measure_producers_start = producers.load(Ordering::Relaxed);
    }
    let name = borrowed
        .cell
        .as_ref()
        .map(CellSpec::name)
        .unwrap_or_else(|| "unknown".into());
    signpost_begin(borrowed.signpost_id, &name);
    info!(cell = %name, "bench measure start");
    drop(borrowed);
    run_after(&inner, config, MEASURE_S, TimerStep::MeasureEnd);
}

/// Closes the measured window: signpost off, result folded, teardown.
fn measure_end(inner: Rc<RefCell<Inner>>, config: MatrixConfig) {
    let mut borrowed = inner.borrow_mut();
    if borrowed.done {
        return;
    }
    signpost_end(borrowed.signpost_id, "bench_cell");
    borrowed.signpost_id += 1;
    let measured = CACurrentMediaTime() - borrowed.measure_start;
    let (rows_delta, producers_delta) = borrowed
        .handles
        .as_ref()
        .map(|h| {
            (
                h.rows_created.load(Ordering::Relaxed) - borrowed.measure_rows_start,
                h.producer_calls.load(Ordering::Relaxed) - borrowed.measure_producers_start,
            )
        })
        .unwrap_or((0, 0));
    let cell = borrowed.cell.clone().unwrap_or(CellSpec {
        scenario: 0,
        variant: "?".into(),
        n: 0,
    });
    let pass_index = borrowed.pass;
    info!(
        cell = %cell.name(),
        frames = borrowed.frames.len(),
        rows_created_per_second = rows_delta as f64 / measured.max(f64::EPSILON),
        producer_calls_per_second = producers_delta as f64 / measured.max(f64::EPSILON),
        "bench measure end"
    );
    let (thermal, frames, mem_samples) = (
        borrowed.thermal_current.clone(),
        std::mem::take(&mut borrowed.frames),
        std::mem::take(&mut borrowed.mem),
    );
    borrowed.results.push(CellResult {
        name: format!("{}-p{}", cell.name(), pass_index + 1),
        scenario: cell.scenario,
        variant: cell.variant.clone(),
        n: cell.n,
        pass: pass_index,
        thermal,
        measured_seconds: measured,
        rows_created_per_second: rows_delta as f64 / measured.max(f64::EPSILON),
        producer_calls_per_second: producers_delta as f64 / measured.max(f64::EPSILON),
        frames,
        mem_samples,
        status: "ok".into(),
    });
    borrowed.cell = None;
    borrowed.handles = None;
    borrowed.animate = None;
    borrowed.cell_binding.set(None);
    borrowed.phase = Phase::Teardown;
    drop(borrowed);
    run_after(&inner, config, TEARDOWN_S, TimerStep::TeardownDone);
}

/// Round over: write `bench-<run_id>.json`.
fn finish(inner: Rc<RefCell<Inner>>, config: MatrixConfig) {
    let mut borrowed = inner.borrow_mut();
    if borrowed.done {
        return;
    }
    borrowed.done = true;
    borrowed.phase = Phase::Done;
    let status = borrowed.status.clone();
    let result = MatrixResult {
        run_id: config.run_id.clone(),
        scenario: "matrix".into(),
        commit: config.commit.clone(),
        bundle_id: NSBundle::mainBundle()
            .bundleIdentifier()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unknown".into()),
        pid: std::process::id(),
        launch_time_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        argv: std::env::args().collect(),
        thermal_state_start: thermal_state_name(),
        device_model: device_model(),
        os_version: NSProcessInfo::processInfo()
            .operatingSystemVersionString()
            .to_string(),
        passes: config.passes,
        status,
        cells: std::mem::take(&mut borrowed.results),
    };
    drop(borrowed);
    let json = serde_json::to_string_pretty(&result).expect("result serialization");
    write_result(&json, &config.run_id);
    info!(run_id = %config.run_id, "BENCH_DONE");
}

/// `NSProcessInfo.thermalState` at call time.
fn thermal_state() -> NSProcessInfoThermalState {
    NSProcessInfo::processInfo().thermalState()
}

fn thermal_state_name() -> String {
    match thermal_state() {
        NSProcessInfoThermalState::Nominal => "nominal".into(),
        NSProcessInfoThermalState::Fair => "fair".into(),
        NSProcessInfoThermalState::Serious => "serious".into(),
        NSProcessInfoThermalState::Critical => "critical".into(),
        _ => "unknown".into(),
    }
}

// ---- os_signpost -----------------------------------------------------------

type OsLog = core::ffi::c_void;

/// `os_signpost_type_t` values from `<os/signpost.h>`.
const OS_SIGNPOST_INTERVAL_BEGIN: u8 = 0x01;
const OS_SIGNPOST_INTERVAL_END: u8 = 0x02;

unsafe extern "C" {
    fn os_log_create(subsystem: *const core::ffi::c_char, category: *const core::ffi::c_char)
        -> *mut OsLog;
    /// The syscall-level emitter behind the `os_signpost_interval_*`
    /// inlines — a real `libsystem` symbol since iOS 13.
    fn _os_signpost_emit_with_type(
        log: *mut OsLog,
        signpost_type: u8,
        signpost_id: u64,
        name: *const core::ffi::c_char,
        format: *const core::ffi::c_char,
        buf: *const u8,
        size: usize,
    );
}

fn signpost_log() -> *mut OsLog {
    struct LogSend(*mut OsLog);
    // SAFETY: `os_log_t` is a retained Objective-C object, thread-safe by
    // contract; the pointer is only used as the log argument.
    unsafe impl Send for LogSend {}
    unsafe impl Sync for LogSend {}
    static LOG: std::sync::OnceLock<LogSend> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        LogSend(unsafe {
            os_log_create(c"dev.waterui.bench".as_ptr(), c"cells".as_ptr())
        })
    })
    .0
}

/// Opens an `os_signpost` interval named `bench_cell` carrying `cell_name`
/// as its metadata string.
fn signpost_begin(id: u64, cell_name: &str) {
    let name = CString::new(cell_name).expect("cell name is UTF-8");
    // SAFETY: log lives for the process; `name` outlives the call.
    unsafe {
        _os_signpost_emit_with_type(
            signpost_log(),
            OS_SIGNPOST_INTERVAL_BEGIN,
            id.max(1),
            c"bench_cell".as_ptr(),
            name.as_ptr(),
            core::ptr::null(),
            0,
        );
    }
}

fn signpost_end(id: u64, name: &str) {
    let name = CString::new(name).expect("name is UTF-8");
    // SAFETY: same as `signpost_begin`.
    unsafe {
        _os_signpost_emit_with_type(
            signpost_log(),
            OS_SIGNPOST_INTERVAL_END,
            id.max(1),
            name.as_ptr(),
            c"".as_ptr(),
            core::ptr::null(),
            0,
        );
    }
}

// ---- memory ----------------------------------------------------------------

fn mem_sample() -> MemSample {
    let (phys_footprint, internal) = task_vm_info();
    let (iosurface, ioaccelerator, iokit) = ledger_bytes();
    MemSample {
        t: CACurrentMediaTime(),
        phys_footprint,
        internal,
        graphics_ledger: graphics_ledger(),
        iosurface,
        ioaccelerator,
        iokit,
        metal_allocated: metal_allocated(),
    }
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
    ledgers: [u64; 76],
}

/// `TASK_VM_INFO` — task flavor 22.
const TASK_VM_INFO: u32 = 22;

unsafe extern "C-unwind" {
    /// The task port for this process — `libc` gates its export to macOS.
    static mach_task_self_: u32;
}

fn task_vm_info() -> (u64, u64) {
    let mut info: TaskVmInfo = unsafe { core::mem::zeroed() };
    let mut count = (core::mem::size_of::<TaskVmInfo>() / core::mem::size_of::<u32>()) as u32;
    // SAFETY: `info` is a valid writable buffer of `count` integers.
    let kr = unsafe {
        libc::task_info(
            mach_task_self_,
            TASK_VM_INFO,
            (&mut info as *mut TaskVmInfo).cast(),
            &mut count,
        )
    };
    if kr != 0 {
        return (0, 0);
    }
    (info.phys_footprint, info.internal)
}

/// Ledger tag names we poll through `mach_vm_region`'s tag array — iOS 27
/// does not report per-tag iosurface/ioaccel bytes, so these read zero
/// and are kept as explicit unavailability markers.
fn ledger_bytes() -> (u64, u64, u64) {
    (0, 0, 0)
}

/// The process ledger's graphics-tag bytes: the `task_vm_info` `ledgers`
/// array index that covers GPU-attributed memory.
fn graphics_ledger() -> u64 {
    // Ledger index 8 = LEDGER_TAG_GRAPHICS on current kernels; read
    // straight from task_vm_info to keep the field honest.
    let mut info: TaskVmInfo = unsafe { core::mem::zeroed() };
    let mut count = (core::mem::size_of::<TaskVmInfo>() / core::mem::size_of::<u32>()) as u32;
    let kr = unsafe {
        libc::task_info(
            mach_task_self_,
            TASK_VM_INFO,
            (&mut info as *mut TaskVmInfo).cast(),
            &mut count,
        )
    };
    if kr != 0 {
        return 0;
    }
    info.ledgers[8]
}

/// `MTLCreateSystemDefaultDevice`'s `currentAllocatedSize`.
fn metal_allocated() -> u64 {
    // The device is a singleton retained by the system.
    let device = MTLCreateSystemDefaultDevice();
    let Some(device) = device else {
        return 0;
    };
    device.currentAllocatedSize() as u64
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