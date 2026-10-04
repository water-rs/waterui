//! The harness entry points the Swift host calls: `start` builds the
//! engine on the view, `tick` runs one frame per `CADisplayLink` call.
//! Both run on the main thread, as the view and the engine require.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::Arc;
#[cfg(target_os = "ios")]
use std::sync::Mutex;
#[cfg(target_os = "ios")]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use cherenkov::{Display, Engine, FrameTime, Layer, Surface, WorkingColor};
use cherenkov_gpu::interop::SharedDevice;
use cherenkov_gpu::interop::wgpu::rwh::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    UiKitWindowHandle, WindowHandle,
};
use cherenkov_gpu::{Gpu, GpuConfig, WindowTarget};
#[cfg(target_os = "ios")]
use objc2::MainThreadMarker;
use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState, NSThread};
#[cfg(target_os = "ios")]
use objc2_ui_kit::{UIScreen, UIView};

use crate::log;
use crate::observe::{self, Decisions};
use crate::pattern::{HEIGHT, WIDTH};
use crate::producer::Pool;
use crate::scenario::{self, Scenario};

thread_local! {
    /// The one run. The engine is `!Send`; every touch is on the main
    /// thread through the FFI entry points.
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

/// Builds the harness on `view` (a `UIView` pointer the host keeps
/// alive) with the view's pixel `size` and display `scale`.
///
/// Reads the launch arguments (`--scenario <name>`, `--paused`) from
/// the process arguments; an unknown or missing value logs the cause
/// and aborts — the harness never guesses a scenario.
///
/// # Safety
/// `view` must point at a live `UIView` that outlives the run.
///
/// Called from the Swift host's `viewDidLayoutSubviews` once the view
/// has its real bounds, on the main thread.
///
/// # Panics
/// When called off the main thread or twice, or when the launch
/// arguments are bad (a panic across this boundary aborts the process).
#[unsafe(no_mangle)]
pub extern "C" fn cherenkov_planes_start(
    view: *const std::ffi::c_void,
    width: f64,
    height: f64,
    scale: f64,
) {
    // A panic across the FFI boundary aborts the process; restore the
    // dimmed brightness before any abort, whichever path takes it down.
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        std::thread::sleep(Duration::from_millis(250));
        default(info);
    }));
    assert!(
        NSThread::isMainThread_class(),
        "cherenkov_planes_start runs on the main thread"
    );
    #[cfg(target_os = "ios")]
    VIEW.store(view as usize, Ordering::Relaxed);
    let run = Run::new(view, (width, height), scale);
    RUN.with(|cell| {
        assert!(
            cell.borrow_mut().replace(run).is_none(),
            "cherenkov_planes_start runs once"
        );
    });
}

/// One display-link frame: produce, install, render, heartbeat.
///
/// Called from the host's `CADisplayLink` callback on the main thread.
#[unsafe(no_mangle)]
pub extern "C" fn cherenkov_planes_tick() -> bool {
    RUN.with(|cell| cell.borrow_mut().as_mut().is_some_and(Run::tick))
}

/// True after the device report is persisted; the host can terminate the run.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn cherenkov_planes_finished() -> bool {
    RUN.with(|cell| {
        cell.borrow().as_ref().is_some_and(|run| {
            run.measurement
                .as_ref()
                .is_some_and(|measurement| measurement.finished)
        })
    })
}

#[cfg(target_os = "ios")]
unsafe extern "C" {
    fn cherenkov_planes_wake();
}

/// The host calls this from `applicationWillResignActive` and
/// `applicationWillTerminate` — and every fatal path below calls it —
/// so a run never leaves the owner's phone dark.
#[allow(
    clippy::missing_const_for_fn,
    reason = "the iOS restore() locks a mutex; const here would fail on that target"
)]
#[unsafe(no_mangle)]
pub extern "C" fn cherenkov_planes_brightness_restore() {
    restore();
}

/// The host calls this from `applicationDidBecomeActive`: a run that
/// returns to the foreground darkens the panel again.
#[allow(
    clippy::missing_const_for_fn,
    reason = "the iOS dim() locks a mutex; const here would fail on that target"
)]
#[unsafe(no_mangle)]
pub extern "C" fn cherenkov_planes_brightness_dim() {
    dim();
}

/// The view resized: `size` is the new pixel size at display `scale`.
///
/// # Safety
/// As [`cherenkov_planes_start`].
#[unsafe(no_mangle)]
pub extern "C" fn cherenkov_planes_resize(width: f64, height: f64, scale: f64) {
    RUN.with(|cell| {
        if let Some(run) = cell.borrow_mut().as_mut() {
            run.resize((width, height), scale);
        }
    });
}

/// The launch arguments from the process: `--scenario <name>` (or
/// `--scenario=<name>`; required, one of the [`scenario::NAMES`]) and
/// `--paused` (the producer stops after the first frame for the
/// idle-video measurement).
fn launch_args() -> (Scenario, bool, Option<String>) {
    let arguments = NSProcessInfo::processInfo().arguments();
    let mut scenario = None;
    let mut paused = false;
    let mut measure = false;
    let mut run_id = None;
    let mut i = 1; // argv[0] is the executable
    while i < arguments.count() {
        let arg = arguments.objectAtIndex(i).to_string();
        if arg == "--" {
            // `devicectl process launch … -- args` passes the literal
            // separator through to the process's argv.
        } else if arg == "--measure" {
            measure = true;
        } else if arg == "--run-id" {
            i += 1;
            run_id = Some(arguments.objectAtIndex(i).to_string());
        } else if arg == "--paused" {
            paused = true;
        } else if arg == "--scenario" {
            i += 1;
            if i < arguments.count() {
                scenario = Some(arguments.objectAtIndex(i).to_string());
            } else {
                fail(&format!("--scenario needs a value ({})", scenario::NAMES));
            }
        } else if let Some(name) = arg.strip_prefix("--scenario=") {
            scenario = Some(name.to_owned());
        } else {
            fail(&format!("unknown argument {arg:?}"));
        }
        i += 1;
    }
    let Some(name) = scenario else {
        fail(&format!("--scenario is required ({})", scenario::NAMES));
    };
    match Scenario::parse(&name) {
        Ok(scenario) => (
            scenario,
            paused,
            measure.then(|| run_id.expect("--measure needs --run-id")),
        ),
        Err(e) => fail(&e),
    }
}

/// Logs the cause and dies — launch misconfiguration is a caller bug.
fn fail(cause: &str) -> ! {
    die(&format!("launch rejected: {cause}"));
}

/// The screen brightness saved before the run dimmed it; restored on
/// every exit path so a failure never leaves the owner's phone dark.
#[cfg(target_os = "ios")]
static SAVED_BRIGHTNESS: Mutex<Option<f64>> = Mutex::new(None);

/// The view the run presents into — remembered so the brightness code
/// can find its screen (`UIScreen.mainScreen` is deprecated in favour
/// of reaching the screen through the view's window).
#[cfg(target_os = "ios")]
static VIEW: AtomicUsize = AtomicUsize::new(0);

/// The screen showing the run's view; `None` before `start` stores the
/// view, off the main thread, or if the view is not windowed.
#[cfg(target_os = "ios")]
fn screen() -> Option<objc2::rc::Retained<UIScreen>> {
    let _mtm = MainThreadMarker::new()?;
    let view = NonNull::new(VIEW.load(Ordering::Relaxed) as *mut UIView)?;
    // SAFETY: the host keeps the view alive for the run's life.
    let window = unsafe { view.as_ref() }.window()?;
    Some(window.screen())
}

/// Saves the current brightness and sets it to the minimum — the
/// measurement runs the panel as dark as it goes. A no-op before the
/// view is remembered or without a screen to dim.
#[cfg(target_os = "ios")]
fn dim() {
    let Some(screen) = screen() else {
        return;
    };
    *SAVED_BRIGHTNESS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(screen.brightness());
    screen.setBrightness(0.0);
}

/// Host test runs own no screen.
#[cfg(not(target_os = "ios"))]
const fn dim() {}

/// Hands back the brightness `dim` captured; a no-op before the first
/// dim or without a reachable screen — `UIScreen` is main-thread state.
/// The saved value stays put: a re-activated run dims and restores
/// again.
#[cfg(target_os = "ios")]
fn restore() {
    let saved = *SAVED_BRIGHTNESS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let (Some(value), Some(screen)) = (saved, screen()) {
        screen.setBrightness(value);
    }
}

/// Host test runs own no screen.
#[cfg(not(target_os = "ios"))]
const fn restore() {}

/// Fatal exit: the cause at error level, the brightness restored, then
/// the process dies. Every unrecoverable path funnels here — a
/// heartbeat that kept ticking while the screen showed nothing was the
/// failure mode this replaces.
pub fn die(cause: &str) -> ! {
    log::error(cause);
    restore();
    // The brightness write needs a beat to reach the display server.
    std::thread::sleep(Duration::from_millis(250));
    std::process::exit(1);
}

/// The display's thermal state as a log token and whether production
/// must pause for cooling.
fn thermal() -> (&'static str, bool) {
    let state = NSProcessInfo::processInfo().thermalState();
    let (name, hot) = match state {
        NSProcessInfoThermalState::Nominal => ("nominal", false),
        NSProcessInfoThermalState::Fair => ("fair", false),
        NSProcessInfoThermalState::Serious => ("serious", true),
        NSProcessInfoThermalState::Critical => ("critical", true),
        // A state the harness does not name means the contract changed:
        // the measurement is invalid, not merely unknown.
        other => die(&format!("unknown thermal state {other:?}")),
    };
    (name, hot)
}

/// A live `UIView` as a window handle: the engine captures its backing
/// layer as the system-compositor parent. The view is the app's own
/// content view — alive for the process's life and touched only on the
/// main thread — so holding its address across threads is sound here.
struct View {
    view: usize,
}

impl HasWindowHandle for View {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let view = NonNull::new(self.view as *mut std::ffi::c_void).expect("a view");
        let raw = RawWindowHandle::UiKit(UiKitWindowHandle::new(view));
        // SAFETY: the view outlives the handle — the host keeps it for
        // the app's life.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for View {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::uikit())
    }
}

/// One scenario's running state.
struct Run {
    #[cfg(target_os = "ios")]
    measurement: Option<crate::measurement::Measurement>,
    engine: Engine<Gpu>,
    surface: Surface<Gpu>,
    video: Option<Video>,
    recorded: Option<crate::recorded::Scene>,
    /// Display-probe channel: the probe arrives on the main queue after
    /// the surface's first part is created.
    probe: std::sync::mpsc::Receiver<cherenkov_gpu::interop::DisplayProbe>,
    /// The headroom last announced to the surface — probed once the
    /// parts exist, 1.0 (SDR) until then.
    headroom: f32,
    /// Still waiting on the display probe's reply.
    probe_pending: bool,
    scale: f64,
    decisions: Arc<Decisions>,
    scenario: Scenario,
    next_log: Instant,
    /// `frame=` over the heartbeat second.
    presented: u64,
    /// The one-time log lines the run has emitted.
    logged: Logged,
    /// True while thermal state pauses production.
    cooling: bool,
    /// Parent/shade layer handles, kept alive for the run.
    _rest: Vec<Layer>,
}

/// The one-time log lines: the first render and the first settled
/// verdict.
#[derive(Default)]
struct Logged {
    /// `false` until the first frame renders — a crash before then is a
    /// device/import problem, not a presentation one.
    first_frame: bool,
    /// Whether the first settled verdict was logged.
    verdict: bool,
}

struct Video {
    layer: Layer,
    producer: Pool,
    video: cherenkov::GpuProducer<Gpu>,
    sink: cherenkov::FrameSink<Gpu>,
}

/// A view dimension in points → device pixels at `scale`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a view's bounds are finite and positive; the cast rounds to the pixel count"
)]
fn pixels(points: f64, scale: f64) -> u32 {
    (points * scale).round() as u32
}

impl Run {
    /// Brings the whole pipeline up: shared Metal device, engine, the
    /// window surface and the scenario's layers and producer.
    fn new(view: *const std::ffi::c_void, size_pt: (f64, f64), scale: f64) -> Self {
        let decisions = observe::install();
        // The arguments gate the dim: a rejected launch never touches
        // the owner's brightness.
        let (scenario, paused, measure) = launch_args();
        if measure.is_some() {
            assert!(
                matches!(scenario, Scenario::Recorded(spec) if !spec.animated),
                "the counter window measures continuously ticking recorded content"
            );
        }
        dim();
        log::line(&format!(
            "scenario={} starting paused={paused}",
            scenario.name()
        ));
        let size = (pixels(size_pt.0, scale), pixels(size_pt.1, scale));
        let shared = SharedDevice::create(&GpuConfig::default()).expect("shared GPU device");
        let engine = Engine::<Gpu>::new(GpuConfig {
            device: Some(shared.clone()),
            ..GpuConfig::default()
        })
        .expect("engine");
        #[cfg(target_os = "ios")]
        engine.set_waker(|| unsafe { cherenkov_planes_wake() });
        log::line("engine up");
        let mut target = WindowTarget::new(
            View {
                view: view as usize,
            },
            size,
        );
        let probe = target.output_probe();
        let surface = engine.surface(target).expect("window surface");
        log::line(&format!("surface target {}x{} created", size.0, size.1));
        surface
            .display(Display {
                scale,
                headroom: 1.0,
            })
            .expect("display announcement");
        surface.clear_color(WorkingColor::new([0.01, 0.012, 0.018, 1.0]));
        let built = scenario.build(&surface);
        let video = built.video.map(|layer| {
            let (video, sink) = engine.frame_producer();
            surface.update(|tx| {
                tx[&layer].content(video.at((WIDTH, HEIGHT)));
            });
            Video {
                layer,
                producer: Pool::new(&shared, paused),
                video,
                sink,
            }
        });
        Self {
            #[cfg(target_os = "ios")]
            measurement: measure.map(crate::measurement::Measurement::new),
            engine,
            surface,
            video,
            recorded: built.recorded,
            probe,
            headroom: 1.0,
            probe_pending: true,
            scale,
            decisions,
            scenario,
            next_log: Instant::now(),
            presented: 0,
            logged: Logged::default(),
            cooling: false,
            _rest: built.rest,
        }
    }

    /// The view resized: resize the surface and recompute the video's
    /// placement (the plane parts reallocate on the next frame).
    fn resize(&mut self, size_pt: (f64, f64), scale: f64) {
        let size = (pixels(size_pt.0, scale), pixels(size_pt.1, scale));
        self.scale = scale;
        if let Err(e) = self.surface.resize(size) {
            die(&format!("resize failed: {e}; render thread is gone"));
        }
        let surface = &self.surface;
        let scenario = self.scenario;
        if let Some(video) = &self.video {
            surface.update(|tx| scenario.relayout(&mut tx[&video.layer], size));
        }
        let _ = self.surface.display(Display {
            scale,
            headroom: self.headroom,
        });
    }

    /// One display-link frame.
    fn tick(&mut self) -> bool {
        // A display-probe reply rides the main queue between frames:
        // once the surface's parts exist, announce the live headroom.
        if self.probe_pending
            && let Ok(probe) = self.probe.try_recv()
        {
            self.probe_pending = false;
            self.headroom = probe.tone_map_headroom().unwrap_or(1.0);
            let _ = self.surface.display(Display {
                scale: self.scale,
                headroom: self.headroom,
            });
            log::line(&format!("display headroom {}", self.headroom));
        }
        let (thermal_name, hot) = thermal();
        #[cfg(target_os = "ios")]
        if let Some(measurement) = &mut self.measurement {
            assert!(!hot, "thermal state invalidated the measurement window");
            measurement.begin_frame();
        }
        #[cfg(target_os = "ios")]
        let frame_start = Instant::now();
        if hot && !self.cooling {
            self.cooling = true;
            log::error("thermal state serious — pausing production to let the device cool");
        } else if !hot && self.cooling {
            self.cooling = false;
            log::line("thermal state recovered — resuming production");
        }
        if !self.cooling
            && let Some(video) = &mut self.video
            && let Some(frame) = video.producer.produce()
        {
            video.sink.submit(frame);
            self.presented += 1;
        }
        if !self.cooling
            && let Some(scene) = &mut self.recorded
        {
            scene.tick(&self.surface);
        }
        let idle = match self.engine.render(FrameTime::now()) {
            Ok(next) => {
                if let Some(video) = &mut self.video {
                    video.producer.rendered();
                }
                if !self.logged.first_frame {
                    self.logged.first_frame = true;
                    log::line("first engine frame rendered");
                }
                next == cherenkov::Next::Idle
            }
            Err(e) => {
                // A dead render thread leaves the harness logging a
                // heartbeat while the screen shows nothing — die loudly
                // instead of looking alive.
                die(&format!("render failed: {e}"));
            }
        };
        if !self.logged.verdict
            && let Some(video) = &self.video
        {
            let layer = video.layer.id().raw();
            let decision = self.decisions.decision(layer);
            if decision != "unseen" {
                self.logged.verdict = true;
                log::line(&format!(
                    "verdict layer=LayerId({layer}) decision={decision}"
                ));
            }
        }
        let now = Instant::now();
        #[cfg(target_os = "ios")]
        if let Some(measurement) = &mut self.measurement {
            let scene = self.recorded.as_ref().expect("recorded measurement");
            measurement.end_frame(
                frame_start,
                now,
                scene.spec,
                self.decisions.decision(scene.layers[0].id().raw()),
                || self.engine.memory(),
            );
        }
        let pause = idle
            && self.recorded.as_ref().is_some_and(|scene| {
                scene.spec.animated && scene.frames >= 4 && scene.spec.lifetime == 0
            });
        self.heartbeat(now, pause, thermal_name);
        pause
    }

    fn heartbeat(&mut self, now: Instant, pause: bool, thermal_name: &str) {
        if now >= self.next_log || pause {
            self.next_log = now + Duration::from_secs(1);
            if let Some(scene) = &self.recorded {
                let memory = self.engine.memory();
                for layer in &scene.layers {
                    log::line(&format!(
                        "scenario={} side={} count={} lifetime={} engine={} frame={} layer=LayerId({}) decision={} gpu_bytes={} cpu_bytes={} passes={} idle={} thermal={}",
                        self.scenario.name(),
                        scene.spec.side,
                        scene.spec.count,
                        scene.spec.lifetime,
                        scene.spec.engine,
                        scene.frames,
                        layer.id().raw(),
                        self.decisions.decision(layer.id().raw()),
                        memory.gpu.0,
                        memory.cpu.0,
                        self.engine.stats().passes,
                        pause,
                        thermal_name
                    ));
                }
            }
            if let Some(video) = &self.video {
                let layer = video.layer.id().raw();
                log::line(&format!(
                    "scenario={} frame={} layer=LayerId({}) decision={} fill={}ms import={}ms stalls={} thermal={}",
                    self.scenario.name(),
                    self.presented,
                    layer,
                    self.decisions.decision(layer),
                    video.producer.fill_ms,
                    video.producer.import_ms,
                    video.producer.stalls,
                    thermal_name,
                ));
            }
            self.presented = 0;
        }
    }
}
