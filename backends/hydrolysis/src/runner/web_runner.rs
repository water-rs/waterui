//! Browser event loop: canvas surface, RAF scheduling, DOM input listeners.

// Compiles only into wasm32 + `web`: the fetch/frame futures hold `Rc`,
// `Closure` and JS-object handles that are `!Send` by design on the
// single-threaded target, and every one is driven by `spawn_local`.
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::Future,
    rc::Rc,
};

use accesskit::ActionRequest as AccessibilityActionRequest;
use async_task::spawn_unchecked as spawn_local_task;
use executor_core::{
    LocalExecutor,
    async_task::{AsyncTask, Runnable},
    try_init_local_executor,
};
use js_sys::Uint8Array;
use nami::Signal;
use serde::Deserialize;
use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use waterui::app::{App, AppParts};
use waterui::window::{Window, WindowState};
use waterui_core::Environment;
use web_sys::Response;

use crate::text::fonts::WebFonts;

use crate::platform::{BrowserWindow, PlatformWindow};
use crate::renderer::{
    FontFamilyResolution, HydrolysisRenderer, HydrolysisTextContextMenuMode, MenuShortcutRegistry,
};
use crate::runner::web_accessibility::WebAccessibilityBridge;
use crate::runner::{
    RenderDiagnosticsConfig, RuntimeWindow, advance_runtime, handle_input_events, render_window,
};
use crate::text::SessionTextEngine;
use crate::time::Instant;

const WEB_FONT_MANIFEST_PATH: &str = "fonts/waterui-fonts.json";

#[derive(Debug, Deserialize)]
struct WebFontManifest {
    default_family: String,
    fonts: Vec<WebFontManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct WebFontManifestEntry {
    name: String,
    file_name: String,
}

impl WebFontManifestEntry {
    fn path(&self) -> String {
        format!("fonts/{}", self.file_name)
    }
}

#[expect(
    clippy::future_not_send,
    reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
)]
async fn fetch_response(path: &str) -> Response {
    let window = web_sys::window().expect("hydrolysis web font loader requires browser window");
    let response = JsFuture::from(window.fetch_with_str(path))
        .await
        .unwrap_or_else(|error| panic!("hydrolysis web font fetch failed for `{path}`: {error:?}"));
    let response: Response = response
        .dyn_into()
        .unwrap_or_else(|_| panic!("hydrolysis web font fetch returned non-Response for `{path}`"));
    assert!(
        response.ok(),
        "hydrolysis web font fetch failed for `{path}` with HTTP status {}",
        response.status()
    );
    response
}

#[expect(
    clippy::future_not_send,
    reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
)]
async fn fetch_bytes(path: &str) -> Vec<u8> {
    let response = fetch_response(path).await;
    let array_buffer = JsFuture::from(response.array_buffer().unwrap_or_else(|error| {
        panic!("hydrolysis web font response array_buffer failed for `{path}`: {error:?}")
    }))
    .await
    .unwrap_or_else(|error| {
        panic!("hydrolysis web font array_buffer await failed for `{path}`: {error:?}")
    });
    let bytes = Uint8Array::new(&array_buffer);
    let mut data = vec![0_u8; bytes.length() as usize];
    bytes.copy_to(&mut data);
    data
}

#[expect(
    clippy::future_not_send,
    reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
)]
async fn fetch_text(path: &str) -> String {
    String::from_utf8(fetch_bytes(path).await).unwrap_or_else(|error| {
        panic!("hydrolysis web font manifest `{path}` is not valid UTF-8: {error}")
    })
}

/// The fonts the page's first frame needs, fetched and registered, and the
/// manifest's other faces, which load after it.
///
/// A face is needed first when the visitor reads the script it draws — their
/// preferred languages, which the browser reports — or when it is the default
/// family or a face selected by name (see
/// [`loads_before_first_frame`](crate::text::fonts::loads_before_first_frame)).
/// Built once for the application: the runner installs the collection as the
/// shared [`FontCollection`](waterui_text::FontCollection) and seeds the window's renderer from it.
#[expect(
    clippy::future_not_send,
    reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
)]
async fn load_web_fonts() -> (WebFonts, Vec<WebFontManifestEntry>) {
    let manifest_text = fetch_text(WEB_FONT_MANIFEST_PATH).await;
    let manifest: WebFontManifest = serde_json::from_str(&manifest_text).unwrap_or_else(|error| {
        panic!("hydrolysis web font manifest parse failed for `{WEB_FONT_MANIFEST_PATH}`: {error}")
    });
    let languages = waterui_locale::regional::current_settings()
        .preferred_languages()
        .to_vec();
    let (first, deferred): (Vec<_>, Vec<_>) = manifest.fonts.into_iter().partition(|font| {
        crate::text::fonts::loads_before_first_frame(
            &font.name,
            &manifest.default_family,
            &languages,
        )
    });

    // Every font file is in flight at once; registration order follows the
    // manifest so the collection is the same as a serial load would build.
    let font_files = futures::future::join_all(
        first
            .iter()
            .map(|font| async move { fetch_bytes(&font.path()).await }),
    )
    .await;

    let fonts = WebFonts::new(
        &manifest.default_family,
        first
            .iter()
            .zip(font_files)
            .map(|(font, font_data)| (font.name.as_str(), font_data)),
    );
    (fonts, deferred)
}

/// Fetches the faces the first frame did not wait for, all at once, and
/// registers each as it arrives: `arrived` tells the next frame to drop the
/// text shaped without it, and `wake` schedules that frame.
fn load_deferred_web_fonts(
    deferred: Vec<WebFontManifestEntry>,
    fonts: &Rc<RefCell<WebFonts>>,
    arrived: &Rc<Cell<bool>>,
    wake: &Rc<dyn Fn()>,
) {
    for font in deferred {
        let fonts = Rc::clone(fonts);
        let arrived = Rc::clone(arrived);
        let wake = Rc::clone(wake);
        wasm_bindgen_futures::spawn_local(async move {
            let font_data = fetch_bytes(&font.path()).await;
            fonts.borrow_mut().register(&font.name, font_data);
            arrived.set(true);
            wake();
        });
    }
}

#[derive(Clone)]
struct BrowserMainThreadExecutor {
    runnable_queue: Rc<RefCell<VecDeque<Runnable>>>,
    schedule_frame: Rc<dyn Fn()>,
}

impl LocalExecutor for BrowserMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let runnable_queue = self.runnable_queue.clone();
        let schedule_frame = self.schedule_frame.clone();
        let (runnable, task) = unsafe {
            // SAFETY: the browser executor is single-threaded and every runnable is queued
            // and polled on the same main-thread event loop.
            spawn_local_task(fut, move |runnable: Runnable| {
                runnable_queue.borrow_mut().push_back(runnable);
                schedule_frame();
            })
        };
        runnable.schedule();
        AsyncTask::from(task)
    }
}

struct BrowserRunner {
    env: Environment,
    runtime: RuntimeWindow<BrowserWindow>,
    runnable_queue: Rc<RefCell<VecDeque<Runnable>>>,
    accessibility_actions: Rc<RefCell<VecDeque<AccessibilityActionRequest>>>,
    accessibility_bridge: WebAccessibilityBridge,
    /// Whether the page has been told its first frame is up, which ends the
    /// launch screen the page shows until then.
    first_frame_announced: bool,
    /// The page's fonts; the faces the first frame did not wait for register
    /// into it as they arrive.
    fonts: Rc<RefCell<WebFonts>>,
    /// The manifest faces the first frame did not wait for; their fetches
    /// start once it is presented.
    deferred_fonts: Vec<WebFontManifestEntry>,
    /// A face arrived since the last frame: the renderer drops the text it
    /// shaped without it before this frame lays out.
    fonts_arrived: Rc<Cell<bool>>,
    /// Schedules a frame — what an arriving face wakes the page with.
    wake: Rc<dyn Fn()>,
}

impl BrowserRunner {
    fn drain_runnable_queue(runnable_queue: &RefCell<VecDeque<Runnable>>) -> bool {
        let mut drained = false;
        // The pop borrow must end before `run`: running a task can schedule
        // more runnables, which pushes onto this same queue.
        loop {
            let runnable = runnable_queue.borrow_mut().pop_front();
            let Some(runnable) = runnable else { break };
            drained = true;
            runnable.run();
        }
        drained
    }

    fn drain_local_executor_queue(&self) -> bool {
        Self::drain_runnable_queue(&self.runnable_queue)
    }

    /// Async because the engine render inside awaits the browser's GPU
    /// device; the caller drives it through `spawn_local` — wasm32 only ever
    /// runs this path.
    #[expect(
        clippy::future_not_send,
        reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
    )]
    async fn frame(&mut self) -> bool {
        let _ = self.drain_local_executor_queue();
        if self.fonts_arrived.take() {
            self.runtime.renderer.fonts_changed();
        }
        // The page's occlusion report drives the pump state each frame — a
        // hidden page still drains events and executor work; only drawing
        // stops.
        self.runtime.sync_occlusion();
        // Same borrow discipline: handling an action may schedule work that
        // queues further accessibility requests.
        loop {
            let request = self.accessibility_actions.borrow_mut().pop_front();
            let Some(request) = request else { break };
            if self
                .runtime
                .renderer
                .handle_accessibility_action(request, &self.env)
            {
                self.runtime.request_refresh();
            }
        }
        let should_close = handle_input_events(&mut self.runtime, &self.env);
        if should_close || self.runtime.window.state.snapshot() == WindowState::Closed {
            return false;
        }
        let _ = advance_runtime(&mut self.runtime, &self.env, Instant::now());
        // A hidden window produces no frame — a tick already posted when
        // the occluding listener landed must not present either — and a
        // wake that carried no armed work answers without one. Only armed
        // work on a visible window encodes a frame.
        if !self.runtime.is_hidden()
            && (self.runtime.mode.is_pending() || self.runtime.renderer.take_redraw_request())
        {
            let presented = render_window(&mut self.runtime, &self.env, &mut || {
                Self::drain_runnable_queue(&self.runnable_queue)
            })
            .await;
            if presented && !self.first_frame_announced {
                self.first_frame_announced = true;
                self.runtime.platform.announce_first_frame();
                load_deferred_web_fonts(
                    core::mem::take(&mut self.deferred_fonts),
                    &self.fonts,
                    &self.fonts_arrived,
                    &self.wake,
                );
            }
        }
        if let Some(update) = self.runtime.renderer.take_accessibility_tree_update() {
            self.accessibility_bridge.update(update);
        }
        true
    }

    fn needs_next_frame(&self) -> bool {
        // A hidden page schedules nothing: the armed mode and queued
        // redraws survive for the restore frame, but no rAF is posted
        // into a parked pump.
        !self.runtime.is_hidden()
            && (self.runtime.platform.take_redraw_request()
                || !self.runnable_queue.borrow().is_empty())
    }
}

/// The `requestAnimationFrame` closure, which has to stay alive on the Rust
/// side for as long as the browser may call back into it.
type AnimationFrameCallback = Closure<dyn FnMut(f64)>;

/// Holds the frame scheduler, which cannot be built until the runner it
/// schedules exists, so the slot is filled once construction has finished.
type ScheduleFrameSlot = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

struct BrowserRunnerHandle {
    runner: RefCell<BrowserRunner>,
    raf_pending: Cell<bool>,
    /// A frame suspended inside an engine `await`: another rAF arriving while
    /// it is pending cannot borrow the runner, so it takes a repeat ticket and
    /// the suspended frame's continuation schedules again.
    frame_in_flight: Cell<bool>,
    frame_again: Cell<bool>,
    raf_callback: RefCell<Option<AnimationFrameCallback>>,
}

impl BrowserRunnerHandle {
    fn schedule_frame(self: &Rc<Self>) {
        if self.raf_pending.replace(true) {
            return;
        }

        let browser_window = web_sys::window()
            .expect("hydrolysis web runner: browser window unavailable for animation frame");
        let callback = self.raf_callback.borrow();
        let callback = callback
            .as_ref()
            .expect("hydrolysis web runner: animation frame callback not initialized");
        browser_window
            .request_animation_frame(callback.as_ref().unchecked_ref())
            .expect("hydrolysis web runner: failed to schedule animation frame");
    }

    // The runner borrow spans the engine's await by design: `frame_in_flight`
    // bars the reentrant borrow a suspended frame would otherwise let a
    // second rAF take, so the RefMut-across-await is sound.
    #[allow(clippy::await_holding_refcell_ref)]
    fn frame(self: &Rc<Self>) {
        self.raf_pending.set(false);
        if self.frame_in_flight.replace(true) {
            self.frame_again.set(true);
            return;
        }
        let handle = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let should_continue = handle.runner.borrow_mut().frame().await;
            handle.frame_in_flight.set(false);
            if !should_continue {
                return;
            }
            if handle.frame_again.replace(false) || handle.runner.borrow().needs_next_frame() {
                handle.schedule_frame();
            }
        });
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the runner wires every browser subsystem once at startup; the length is the enumeration, not logic"
)]
pub fn run(app: App, style: impl crate::Style) {
    wasm_bindgen_futures::spawn_local(async move {
        let schedule_frame_ref: ScheduleFrameSlot = Rc::new(RefCell::new(None));
        let browser_schedule = {
            let schedule_frame_ref = schedule_frame_ref.clone();
            Rc::new(move || {
                // Setup observers — the IntersectionObserver's initial
                // delivery, `visibilitychange`, a queued runnable — can
                // request a frame before the runner installs the
                // scheduler below. The `handle.schedule_frame()` after
                // installation starts the pump unconditionally, so an
                // early request needs no reply.
                if let Some(schedule) = schedule_frame_ref.borrow().as_ref().cloned() {
                    schedule();
                }
            }) as Rc<dyn Fn()>
        };
        // `visibilitychange` and the IntersectionObserver cannot report
        // through the frame loop: a hidden tab's rAF callback never fires,
        // so the pump would never learn it became hidden. This wake pulls
        // the occlusion report into the pump synchronously — logging the
        // transition and suppressing wake requests while hidden — then
        // posts the frame that presents the restore.
        let occlusion_wake_ref: ScheduleFrameSlot = Rc::new(RefCell::new(None));
        let browser_occlusion_wake = {
            let occlusion_wake_ref = occlusion_wake_ref.clone();
            Rc::new(move || {
                if let Some(wake) = occlusion_wake_ref.borrow().as_ref().cloned() {
                    wake();
                }
            }) as Rc<dyn Fn()>
        };
        let runnable_queue = Rc::new(RefCell::new(VecDeque::new()));
        let local_executor = BrowserMainThreadExecutor {
            runnable_queue: runnable_queue.clone(),
            schedule_frame: browser_schedule.clone(),
        };
        // Nothing probes the browser executor: the inspector endpoint is a TCP
        // server the page cannot host, so no probe exists to hand it. The
        // browser's rAF rate is not queryable up front, so the executor budgets
        // at the headless rate.
        let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
            local_executor,
            waterui::task::RefreshRate::HEADLESS,
            None,
        ));

        // The page is the application's one window: it has no windowless
        // state to stay resident in, and closing the tab ends the application
        // whatever its policy says. The browser kills the page without
        // notice, so the termination hooks are never called and the machine
        // is never started.
        let AppParts {
            windows,
            menu_bar,
            env,
            last_window: _,
            termination: _,
        } = app.into_parts();
        let window_count = windows.len();
        let Ok([window]) = <[Window; 1]>::try_from(windows) else {
            panic!(
                "hydrolysis web runner requires exactly one window, got {window_count}: a browser \
                 page is the application's only window and has no windowless state"
            );
        };

        let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
        let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
        super::install_native_component_hooks(&mut env);
        // Every runner seeds the chord table so mounted menus resolve
        // shortcuts through the same path (water-rs/hydrolysis#247).
        let _ = env.get_or_insert_with::<MenuShortcutRegistry, _>(MenuShortcutRegistry::default);
        // A browser page cannot own the browser's menu bar, so the app menus
        // contribute their chords only — nothing renders.
        super::menu_bar::register_menu_bar(&menu_bar, &env);
        env.insert(HydrolysisTextContextMenuMode::Overlay);
        crate::theme::install_theme_tokens(&mut env, Some(&style));
        let theme: Rc<dyn crate::engine::WidgetTheme> = Rc::new(style);
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::new(Rc::clone(&theme)),
        ));

        // The application's fonts are fetched while the GPU adapter and device
        // are requested; neither waits on the other. The window's renderer is
        // seeded from the collection, and a self-drawn component that typesets
        // text itself reads it out of the environment instead of building a
        // collection of its own.
        let (mut platform, (fonts, deferred_fonts)) = futures::join!(
            BrowserWindow::new(
                Rc::clone(&browser_schedule),
                Rc::clone(&browser_occlusion_wake),
            ),
            load_web_fonts()
        );
        platform.apply_properties(&window);
        // The root content lays out inside the page's safe area while
        // backgrounds reach under the browser and system chrome around it.
        env.insert(crate::platform::WindowSafeArea(platform.safe_area()));
        fonts.collection().clone().install(&mut env);
        let mut renderer = HydrolysisRenderer::with_engine(
            theme,
            SessionTextEngine::from_collection(fonts.collection(), FontFamilyResolution::Lenient),
        );
        renderer.set_window_id(
            env.get::<MenuShortcutRegistry>()
                .expect("the web runner seeds MenuShortcutRegistry")
                .mint_window_id(),
        );
        renderer.set_window_closable(window.closable);
        let runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
        let accessibility_actions = Rc::new(RefCell::new(VecDeque::new()));
        let accessibility_bridge = WebAccessibilityBridge::new(
            Rc::clone(&accessibility_actions),
            Rc::clone(&browser_schedule),
        );
        let runner = BrowserRunner {
            env,
            runtime,
            runnable_queue,
            accessibility_actions,
            accessibility_bridge,
            first_frame_announced: false,
            fonts: Rc::new(RefCell::new(fonts)),
            deferred_fonts,
            fonts_arrived: Rc::new(Cell::new(false)),
            wake: browser_schedule,
        };

        let handle = Rc::new(BrowserRunnerHandle {
            runner: RefCell::new(runner),
            raf_pending: Cell::new(false),
            frame_in_flight: Cell::new(false),
            frame_again: Cell::new(false),
            raf_callback: RefCell::new(None),
        });
        let callback_handle = handle.clone();
        let callback =
            Closure::wrap(Box::new(move |_ts: f64| callback_handle.frame()) as Box<dyn FnMut(f64)>);
        *handle.raf_callback.borrow_mut() = Some(callback);
        *schedule_frame_ref.borrow_mut() = Some({
            let handle = handle.clone();
            Rc::new(move || handle.schedule_frame())
        });
        *occlusion_wake_ref.borrow_mut() = Some({
            let handle = handle.clone();
            Rc::new(move || {
                // DOM listeners fire between frames, so the runner is
                // normally free; if a frame is mid-borrow the occlusion
                // report is read there anyway (`frame` syncs every tick).
                let hidden = if let Ok(mut runner) = handle.runner.try_borrow_mut() {
                    runner.runtime.sync_occlusion();
                    runner.runtime.is_hidden()
                } else {
                    false
                };
                // A hidden pump posts nothing: the armed work survives
                // for the restore frame the next wake schedules.
                if !hidden {
                    handle.schedule_frame();
                }
            })
        });
        waterui_locale::start_system_locale_listener();
        handle.schedule_frame();
    });
}
