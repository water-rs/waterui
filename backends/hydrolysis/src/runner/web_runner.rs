//! Browser event loop: canvas surface, RAF scheduling, DOM input listeners.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::Future,
    rc::Rc,
    sync::Arc,
};

use async_task::spawn_unchecked as spawn_local_task;
use executor_core::{
    LocalExecutor,
    async_task::{AsyncTask, Runnable},
    try_init_local_executor,
};
use js_sys::Uint8Array;
use parley::fontique::{Blob, FontInfoOverride};
use serde::Deserialize;
use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use waterui::app::App;
use waterui::window::WindowState;
use waterui_core::Environment;
use web_sys::Response;

use super::fonts::ResourceFontFamilies;
use crate::platform::{BrowserWindow, PlatformWindow};
use crate::renderer::{HydrolysisRenderer, HydrolysisTextContextMenuMode};
use crate::runner::{RenderDiagnosticsConfig, RuntimeWindow, handle_input_events, render_window};

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

async fn fetch_text(path: &str) -> String {
    String::from_utf8(fetch_bytes(path).await).unwrap_or_else(|error| {
        panic!("hydrolysis web font manifest `{path}` is not valid UTF-8: {error}")
    })
}

async fn load_web_fonts(renderer: &mut HydrolysisRenderer) {
    let manifest_text = fetch_text(WEB_FONT_MANIFEST_PATH).await;
    let manifest: WebFontManifest = serde_json::from_str(&manifest_text).unwrap_or_else(|error| {
        panic!("hydrolysis web font manifest parse failed for `{WEB_FONT_MANIFEST_PATH}`: {error}")
    });

    let mut default_family_ids = Vec::new();
    let mut resource_fonts = ResourceFontFamilies::default();
    let font_cx = renderer.state_mut().text_fonts_mut();
    for font in manifest.fonts {
        let font_path = format!("fonts/{}", font.file_name);
        let font_data = fetch_bytes(&font_path).await;
        let families = font_cx.collection.register_fonts(
            Blob::new(Arc::new(font_data)),
            Some(FontInfoOverride {
                family_name: Some(font.name.as_str()),
                ..Default::default()
            }),
        );
        if font.name == manifest.default_family {
            default_family_ids.extend(families.iter().map(|(family_id, _)| *family_id));
        }
        resource_fonts.classify(font.name.as_str(), &families);
    }

    assert!(
        !default_family_ids.is_empty(),
        "hydrolysis web font manifest default family `{}` did not register any fonts",
        manifest.default_family
    );
    resource_fonts.install(&mut font_cx.collection);
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
}

impl BrowserRunner {
    fn drain_runnable_queue(runnable_queue: &RefCell<VecDeque<Runnable>>) -> bool {
        let mut drained = false;
        while let Some(runnable) = runnable_queue.borrow_mut().pop_front() {
            drained = true;
            runnable.run();
        }
        drained
    }

    fn drain_local_executor_queue(&self) -> bool {
        Self::drain_runnable_queue(&self.runnable_queue)
    }

    fn frame(&mut self) -> bool {
        let _ = self.drain_local_executor_queue();
        let should_close = handle_input_events(&mut self.runtime, &self.env);
        if should_close || self.runtime.window.state.get() == WindowState::Closed {
            return false;
        }
        render_window(&mut self.runtime, &self.env, &mut || {
            Self::drain_runnable_queue(&self.runnable_queue)
        });
        true
    }

    fn needs_next_frame(&self) -> bool {
        self.runtime.platform.take_redraw_request() || !self.runnable_queue.borrow().is_empty()
    }
}

struct BrowserRunnerHandle {
    runner: RefCell<BrowserRunner>,
    raf_pending: Cell<bool>,
    raf_callback: RefCell<Option<Closure<dyn FnMut(f64)>>>,
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

    fn frame(self: &Rc<Self>) {
        self.raf_pending.set(false);
        let should_continue = self.runner.borrow_mut().frame();
        if !should_continue {
            return;
        }

        if self.runner.borrow().needs_next_frame() {
            self.schedule_frame();
        }
    }
}

pub fn run(app: App, inspector_probe: Option<std::sync::Arc<dyn waterui::task::RuntimeProbe>>) {
    wasm_bindgen_futures::spawn_local(async move {
        let schedule_frame_ref: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let browser_schedule = {
            let schedule_frame_ref = schedule_frame_ref.clone();
            Rc::new(move || {
                let schedule = schedule_frame_ref
                    .borrow()
                    .as_ref()
                    .cloned()
                    .expect("hydrolysis web runner: frame scheduler is not ready");
                schedule();
            }) as Rc<dyn Fn()>
        };
        let runnable_queue = Rc::new(RefCell::new(VecDeque::new()));
        let local_executor = BrowserMainThreadExecutor {
            runnable_queue: runnable_queue.clone(),
            schedule_frame: browser_schedule.clone(),
        };
        let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
            local_executor,
            inspector_probe,
        ));

        let (windows, _menu_bar, env) = app.into_parts();
        let mut windows = windows.into_iter();
        let window = windows
            .next()
            .expect("hydrolysis web runner requires exactly one window");
        assert!(
            windows.next().is_none(),
            "hydrolysis web runner supports exactly one window"
        );

        let mut env = env;
        let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
        super::install_native_component_hooks(&mut env);
        env.insert(HydrolysisTextContextMenuMode::Overlay);
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::default(),
        ));

        let mut platform = BrowserWindow::new(browser_schedule).await;
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_web_fonts(&mut renderer).await;
        let runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
        let runner = BrowserRunner {
            env,
            runtime,
            runnable_queue,
        };

        let handle = Rc::new(BrowserRunnerHandle {
            runner: RefCell::new(runner),
            raf_pending: Cell::new(false),
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
        handle.schedule_frame();
    });
}
