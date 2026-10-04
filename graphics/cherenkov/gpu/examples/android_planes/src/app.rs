//! The `NativeActivity` entry point: builds the engine, the
//! `SurfaceControlTarget`, the video producers and the scenario's layer
//! tree, then renders and logs the heartbeat until killed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use android_activity::{AndroidApp, MainEvent, PollEvent};
use cherenkov::kurbo::{Affine, Rect};
use cherenkov::{
    Color, Content, Draw, Engine, Font, FrameTime, Layer, Srgb, Surface, WorkingColor,
};
use cherenkov_gpu::interop::android::{SurfaceControl, SurfaceControlTarget};
use cherenkov_gpu::interop::{ExternalFrame, SharedDevice, vulkan};
use cherenkov_gpu::{Gpu, GpuConfig};
use ndk::native_window::NativeWindow;

use crate::ahb::{HEIGHT, WIDTH};
use crate::observe::{self, Decisions};
use crate::producer::Pool;
use crate::scenario::Scenario;
use crate::{logcat, text};

const FONT: &str = "/system/fonts/Roboto-Regular.ttf";

/// Invoked by the `NativeActivity` glue once the app thread starts.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the android-activity glue declares extern \"Rust\" fn android_main(AndroidApp)"
)]
#[unsafe(no_mangle)]
pub extern "Rust" fn android_main(app: AndroidApp) {
    let decisions = observe::install();
    let (scenario, paused) = read_launch(&app);
    logcat::line(&format!(
        "scenario={} starting paused={paused}",
        scenario.name()
    ));

    let mut run: Option<Run> = None;
    let mut inset_top: Option<i32> = None;
    let mut wait_log = Instant::now();
    loop {
        let mut terminated = false;
        let mut resized = false;
        let mut destroyed = false;
        app.poll_events(Some(Duration::ZERO), |event| {
            if let PollEvent::Main(main) = &event {
                logcat::line(&format!("event {main:?}"));
            }
            match event {
                PollEvent::Main(MainEvent::Destroy) => destroyed = true,
                PollEvent::Main(MainEvent::TerminateWindow { .. }) => terminated = true,
                PollEvent::Main(MainEvent::WindowResized { .. }) => resized = true,
                _ => {}
            }
        });
        if destroyed {
            break;
        }
        if terminated {
            run = None;
            continue;
        }
        let Some(window) = app.native_window() else {
            if wait_log.elapsed() >= Duration::from_secs(1) {
                wait_log = Instant::now();
                logcat::line("waiting for native_window");
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if run.is_none() {
            logcat::line(&format!(
                "native_window {}x{}; building the pipeline",
                window.width(),
                window.height()
            ));
        }
        let run = run.get_or_insert_with(|| Run::new(&window, scenario, paused, &decisions));
        if resized {
            run.resize(&window);
        }
        let top = app.content_rect().top;
        if inset_top != Some(top) {
            inset_top = Some(top);
            run.inset(top);
        }
        run.frame();
        std::thread::sleep(Duration::from_millis(8));
    }
    logcat::line(&format!("scenario={} exiting", scenario.name()));
}

/// The intent's launch extras via JNI: the `scenario` string (unknown
/// or missing values run `overlay`) and `paused` (the producer stops
/// after the first frame for the idle-video measurement).
fn read_launch(app: &AndroidApp) -> (Scenario, bool) {
    match intent_extras(app) {
        Ok((name, paused)) => (
            Scenario::parse(name.as_deref().unwrap_or("overlay")),
            paused,
        ),
        Err(e) => {
            logcat::warn(&format!(
                "reading the launch extras failed ({e}); running overlay"
            ));
            (Scenario::Overlay, false)
        }
    }
}

fn intent_extras(app: &AndroidApp) -> Result<(Option<String>, bool), String> {
    use jni::objects::{JObject, JString, JValue};
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| -> jni::errors::Result<(Option<String>, bool)> {
        // android-activity owns the activity's global ref; hold it
        // through a local ref so this `JObject` only deletes what it
        // created.
        let local = unsafe {
            let env_raw = env.get_raw();
            ((**env_raw).v1_2.NewLocalRef)(env_raw, app.activity_as_ptr().cast())
        };
        let activity = unsafe { JObject::from_raw(env, local) };
        let intent = env
            .call_method(
                &activity,
                jni::jni_str!("getIntent"),
                jni::jni_sig!("()Landroid/content/Intent;"),
                &[],
            )?
            .l()?;
        if intent.is_null() {
            return Ok((None, false));
        }
        let key = env.new_string("scenario")?;
        let extra = env
            .call_method(
                &intent,
                jni::jni_str!("getStringExtra"),
                jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(key.as_ref())],
            )?
            .l()?;
        let scenario = if extra.is_null() {
            None
        } else {
            Some(env.cast_local::<JString>(extra)?.try_to_string(env)?)
        };
        let key = env.new_string("paused")?;
        let paused = env
            .call_method(
                &intent,
                jni::jni_str!("getBooleanExtra"),
                jni::jni_sig!("(Ljava/lang/String;Z)Z"),
                &[JValue::Object(key.as_ref()), JValue::Bool(false)],
            )?
            .z()?;
        Ok((scenario, paused))
    })
    .map_err(|e| e.to_string())
}

/// One scenario's running state.
struct Run {
    recorded: Option<crate::recorded::Scene>,
    engine: Engine<Gpu>,
    surface: Surface<Gpu>,
    videos: Vec<Video>,
    decisions: Arc<Decisions>,
    scenario: Scenario,
    next_log: Instant,
    /// `false` until the first frame renders — a crash before then is a
    /// device/import problem, not a presentation one.
    logged_first_frame: bool,
    /// `LayerId`s whose first settled verdict was already logged.
    logged_verdicts: Vec<u64>,
    /// Parent layer handles: keeps their layers alive for the run
    /// (dropping a live layer queues its `Remove`).
    _rest: Vec<Layer>,
    /// The engine-composited controls layer, kept alive and moved with
    /// the window's top inset.
    controls: Layer,
    _font: Font,
}

struct Video {
    layer: Layer,
    producer: Pool,
    video: cherenkov::GpuProducer<Gpu>,
    sink: cherenkov::FrameSink<Gpu>,
}

impl Run {
    /// Brings the whole pipeline up: shared Vulkan device, engine, the
    /// surface-control surface and the scenario's layers and producers.
    fn new(
        window: &NativeWindow,
        scenario: Scenario,
        paused: bool,
        decisions: &Arc<Decisions>,
    ) -> Self {
        logcat::line("creating the shared GPU device");
        let shared = SharedDevice::create(&GpuConfig::default()).expect("shared GPU device");
        let vk = vulkan::Device::new(&shared).expect("vulkan import context");
        let engine = Engine::<Gpu>::new(GpuConfig {
            device: Some(shared),
            ..GpuConfig::default()
        })
        .expect("engine");
        logcat::line("engine up");

        let size = (
            window.width().cast_unsigned(),
            window.height().cast_unsigned(),
        );
        let parent = unsafe { SurfaceControl::from_window(window.ptr(), c"cherenkov harness") }
            .expect("surface control from window");
        logcat::line("SurfaceControl \"cherenkov harness\" created");
        let surface = engine
            .surface(SurfaceControlTarget::new(parent, size))
            .expect("surface-control surface");
        logcat::line(&format!("surface target {}x{} created", size.0, size.1));
        surface.clear_color(WorkingColor::new([0.01, 0.012, 0.018, 1.0]));

        let font_data = std::fs::read(FONT).expect("Roboto is present on Android");
        let font = engine
            .font(cherenkov::FontSource::bytes(font_data.clone()))
            .expect("font registration");
        let panel = controls_content(&surface, font.id(), &font_data, scenario);

        let specs = scenario.videos();
        let (layers, rest, controls, recorded) = if let Scenario::Recorded(spec) = scenario {
            (
                Vec::new(),
                Vec::new(),
                surface.layer(),
                Some(crate::recorded::Scene::new(&surface, spec)),
            )
        } else {
            let (layers, rest, controls) = scenario.build(&surface, panel);
            (layers, rest, controls, None)
        };
        assert_eq!(specs.len(), layers.len(), "videos and layers pair");
        let videos: Vec<Video> = specs
            .into_iter()
            .zip(layers)
            .map(|(spec, layer)| {
                let (video, sink) = engine.frame_producer();
                surface.update(|tx| {
                    tx[&layer].content(video.at((WIDTH, HEIGHT)));
                });
                Video {
                    layer,
                    producer: Pool::new(&vk, spec, paused).expect("producer pool"),
                    video,
                    sink,
                }
            })
            .collect();
        logcat::line(&format!("{} video layer(s) built", videos.len()));

        Self {
            recorded,
            engine,
            surface,
            videos,
            decisions: Arc::clone(decisions),
            scenario,
            next_log: Instant::now(),
            logged_first_frame: false,
            logged_verdicts: Vec::new(),
            _rest: rest,
            controls,
            _font: font,
        }
    }

    /// Moves the controls layer inside the window's top inset (the
    /// status bar) — the surface spans the whole window, inset or not.
    fn inset(&self, top: i32) {
        self.surface.update(|tx| {
            tx[&self.controls].transform(Affine::translate((0.0, f64::from(top))));
        });
    }

    /// The window resized: resize the surface (the plane parts reallocate
    /// on the next frame).
    fn resize(&self, window: &NativeWindow) {
        let size = (
            window.width().cast_unsigned(),
            window.height().cast_unsigned(),
        );
        if let Err(e) = self.surface.resize(size) {
            logcat::error(&format!(
                "resize failed: {e}; render thread is gone, exiting"
            ));
            std::process::exit(1);
        }
    }

    /// Produces one video generation per layer, renders and emits the
    /// per-second heartbeat.
    fn frame(&mut self) {
        if let Some(scene) = &mut self.recorded {
            scene.tick(&self.surface);
        }
        for video in &mut self.videos {
            let Some(frame) = video.producer.produce() else {
                continue;
            };
            match ExternalFrame::native(frame) {
                Ok(external) => video.sink.submit(external),
                Err(e) => logcat::error(&format!("external frame rejected: {e}")),
            }
        }
        match self.engine.render(FrameTime::now()) {
            Ok(_) => {
                if !self.logged_first_frame {
                    self.logged_first_frame = true;
                    logcat::line("first engine frame rendered");
                }
            }
            Err(e) => {
                // A dead render thread leaves the harness logging a
                // heartbeat while the screen shows nothing — die loudly
                // instead of looking alive.
                logcat::error(&format!("render failed: {e}; exiting"));
                std::process::exit(1);
            }
        }
        for video in &self.videos {
            let layer = video.layer.id().raw();
            if self.logged_verdicts.contains(&layer) {
                continue;
            }
            let decision = self.decisions.decision(layer);
            if decision != "unseen" && decision != "pending" {
                self.logged_verdicts.push(layer);
                logcat::line(&format!(
                    "verdict layer=LayerId({layer}) decision={decision}"
                ));
            }
        }
        let now = Instant::now();
        if now >= self.next_log {
            self.next_log = now + Duration::from_secs(1);
            if let Some(scene) = &self.recorded {
                let memory = self.engine.memory();
                for layer in &scene.layers {
                    logcat::line(&format!(
                        "scenario={} side={} count={} lifetime={} engine={} frame={} layer=LayerId({}) decision={} gpu_bytes={} cpu_bytes={} passes={}",
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
                        self.engine.stats().passes
                    ));
                }
            }
            for video in &self.videos {
                let layer = video.layer.id().raw();
                logcat::line(&format!(
                    "scenario={} frame={} layer=LayerId({}) decision={} fences={} fill={}ms import={}ms stalls={}",
                    self.scenario.name(),
                    video.producer.produced,
                    layer,
                    self.decisions.decision(layer),
                    video.producer.signalled,
                    video.producer.fill_ms,
                    video.producer.import_ms,
                    video.producer.stalls,
                ));
            }
        }
    }
}

/// The engine-composited controls drawn above the video: a translucent
/// panel and the scenario label.
fn controls_content(
    surface: &cherenkov::Surface<Gpu>,
    font: cherenkov::FontId,
    data: &[u8],
    scenario: Scenario,
) -> Content {
    let label = format!("cherenkov planes \u{2014} {}", scenario.name());
    surface.record(|c| {
        c.fill(
            Rect::new(24.0, 24.0, 660.0, 140.0),
            Color::<Srgb>::new([0.07, 0.09, 0.14, 0.72]),
        );
        c.transform(Affine::translate((48.0, 96.0)), |c| {
            c.glyphs(
                text::run(font, data, 42.0, &label),
                Color::<Srgb>::new([0.92, 0.93, 0.97, 1.0]),
            );
        });
    })
}
