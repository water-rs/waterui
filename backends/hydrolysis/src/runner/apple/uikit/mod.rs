//! The `UIKit` host: Hydrolysis windows on `cocoa-ui` scenes.
//!
//! [`run`] is the program entry on iOS and iPadOS. It installs tracing, the
//! executors and the inspector, then hands the process to `UIApplicationMain`
//! through `cocoa-ui`, whose scene delegate calls back as scenes connect and
//! change state. Launch builds the application's environment once and opens
//! the GPU context asynchronously; each scene's platform objects exist from
//! the moment `UIKit` connects it, and its session mounts when both the
//! scene and the process are ready.
//!
//! - [`host`]: the scene's shared state and its [`PlatformWindow`]
//! - [`scene`]: a connected scene and the session mounted into it
//! - [`insets`]: the container and keyboard inset regions
//!
//! [`PlatformWindow`]: crate::platform::PlatformWindow

mod host;
mod insets;
mod scene;

use std::any::Any;
use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use cocoa_ui::MainThreadMarker;
use cocoa_ui::main_queue::MainQueueWake;
use cocoa_ui::uikit::{self, ApplicationHandlers, Window, WindowScene};
use executor_core::{LocalExecutor as _, Task as _};
use waterui::app::App;
use waterui::theme::ColorScheme;
use waterui_apple_menu::ios::{Declared, build_handler, declared, install_declared};
use waterui_core::Environment;

use super::session::AppleExecutor;
use crate::platform::EngineGpuContext;
use crate::renderer::MenuShortcutRegistry;
use crate::runner::{HostEnvironment, assemble_host_environment, init_main_thread_executors};
use crate::text::ParleyEngine;
use crate::theme::WidgetTheme;
use scene::{UiKitScene, framework_scheme};

/// The environment variable a launcher sets to name the level the
/// application logs at — `water run --logs <level>` writes it.
const LOG_LEVEL_ENV: &str = "WATERUI_LOG";

/// What launch needs from `run`'s arguments.
struct Launch {
    app: Box<dyn FnOnce(Environment) -> App>,
    env: Environment,
    style: Rc<dyn crate::Style>,
    /// The environment carrying the inspector's recorders; the
    /// application's own is layered over it.
    services: Environment,
}

/// What launch produced once the GPU context opened: everything a scene
/// mounts with.
pub(super) struct Process {
    env: Environment,
    theme: Rc<dyn WidgetTheme>,
    fonts: waterui_text::FontCollection,
    shortcuts: MenuShortcutRegistry<ParleyEngine>,
    gpu: EngineGpuContext,
    /// The one window declaration; taken by the scene that mounts it.
    window: RefCell<Option<waterui::window::Window>>,
    pub(super) pending_window_queue: Rc<RefCell<Vec<waterui::window::Window>>>,
}

impl Process {
    /// The window declaration, for the one scene that mounts it.
    ///
    /// # Panics
    /// When it was already taken: a second scene has no declaration to
    /// mount until the facade can instantiate declarations.
    pub(super) fn take_window(&self) -> waterui::window::Window {
        self.window.borrow_mut().take().expect(
            "hydrolysis UIKit: a second scene connected, but the app declared one window and \
             windows cannot yet be instantiated per scene",
        )
    }
}

/// The application, shared by every `UIKit` handler.
struct UiKitApplication {
    executor: AppleExecutor,
    /// Consumed by launch.
    launch: RefCell<Option<Launch>>,
    /// The process-wide appearance, fed by the scene's trait observation.
    color_scheme: nami::Binding<ColorScheme>,
    process: OnceCell<Rc<Process>>,
    scenes: RefCell<BTreeMap<String, Rc<UiKitScene>>>,
    /// What the `build_menus` handler reads: the resolved menu bar and the
    /// environment its actions run under, filled by launch.
    menus: Declared,
    /// Keeps the menu bar's change watch alive for the process.
    menu_watch: RefCell<Option<Box<dyn Any>>>,
}

impl UiKitApplication {
    /// Builds the application environment and starts opening the GPU
    /// context; scenes that connected first mount when it resolves.
    fn did_finish_launching(self: &Rc<Self>) {
        waterui_locale::start_system_locale_listener();
        let Launch {
            app,
            env,
            style,
            services,
        } = self
            .launch
            .borrow_mut()
            .take()
            .expect("the application finishes launching once");
        let waterui::app::AppParts {
            windows,
            menu_bar,
            env,
            // iOS ends the process without notice: the last-window policy
            // has nothing to decide, and the termination hooks are never
            // called, so the machine is never started.
            last_window: _,
            termination: _,
        } = app(env).into_parts();
        let mut windows = windows.into_iter();
        let window = windows
            .next()
            .expect("an iOS application must declare at least one window");
        assert!(
            windows.next().is_none(),
            "hydrolysis UIKit: multiple windows are unsupported — a scene mounts exactly one window"
        );
        let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
        let HostEnvironment {
            env,
            theme,
            fonts,
            shortcuts,
        } = assemble_host_environment::<ParleyEngine>(
            &env.layered_on(&services),
            &menu_bar,
            style,
            self.color_scheme.clone(),
            &pending_window_queue,
        );

        // The iPadOS main menu is built from the environment the chords above
        // were registered under, so its actions run with the application
        // environment as the macOS winit bar's do. `UIKit` builds the menu
        // after launch returns; the rebuild request covers a build that ran
        // before this fill.
        *self.menu_watch.borrow_mut() = Some(install_declared(&menu_bar, &env, &self.menus));
        uikit::request_main_menu_rebuild(
            MainThreadMarker::new().expect("launch runs on the main thread"),
        );

        let application = Rc::clone(self);
        self.executor
            .spawn_local(async move {
                // The engine presents each view itself, so no surface exists
                // to pick the adapter by; the context opens before any scene
                // renders and never blocks the main thread.
                let gpu = EngineGpuContext::request("hydrolysis uikit").await;
                application.process_ready(Process {
                    env,
                    theme,
                    fonts,
                    shortcuts,
                    gpu,
                    window: RefCell::new(Some(window)),
                    pending_window_queue,
                });
            })
            .detach();
    }

    fn process_ready(&self, process: Process) {
        let process = Rc::new(process);
        assert!(
            self.process.set(Rc::clone(&process)).is_ok(),
            "the GPU context opens once"
        );
        let waiting: Vec<_> = self.scenes.borrow().values().cloned().collect();
        for scene in waiting {
            scene.mount(&process, &self.executor);
        }
    }

    /// `UIKit` connected a scene: build its platform objects, and mount the
    /// session now if the process is ready.
    fn connect(&self, scene: &WindowScene) -> Window {
        assert!(
            self.scenes.borrow().is_empty(),
            "hydrolysis UIKit: a second scene connected, but one scene is supported until \
             windows can be instantiated per scene"
        );
        let (window, connected) = UiKitScene::connect(scene, &self.color_scheme);
        self.scenes
            .borrow_mut()
            .insert(connected.session_id().to_owned(), Rc::clone(&connected));
        if let Some(process) = self.process.get() {
            connected.mount(process, &self.executor);
        }
        window
    }

    /// Runs `lifecycle` against the scene `id` names.
    fn with_scene(&self, id: &str, lifecycle: impl FnOnce(&UiKitScene)) {
        let scene = self.scenes.borrow().get(id).cloned();
        let scene = scene.unwrap_or_else(|| {
            panic!("UIKit reported a lifecycle change for the unknown scene session {id}")
        });
        lifecycle(&scene);
    }

    fn disconnect(&self, id: &str) {
        let scene = self.scenes.borrow_mut().remove(id);
        scene
            .unwrap_or_else(|| panic!("UIKit disconnected the unknown scene session {id}"))
            .disconnect();
    }
}

/// Runs `app` on the `UIKit` host and never returns: the process lives in
/// `UIApplicationMain`.
///
/// # Panics
/// When called off the main thread or twice.
pub(in crate::runner) fn run(
    app: impl FnOnce(Environment) -> App + 'static,
    env: Environment,
    style: impl crate::Style,
) -> ! {
    let mtm = MainThreadMarker::new().expect("hydrolysis::run is called on the main thread");
    std::panic::set_hook(Box::new(tracing_panic::panic_hook));
    let inspector = init_main_thread_executors();
    init_tracing(
        inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::tracing_layer),
    );
    let inspector_probe = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::runtime_probe);

    let wake = MainQueueWake::new(mtm);
    let executor = AppleExecutor::new(wake.clone());
    wake.set_callback(mtm, {
        let executor = executor.clone();
        move || {
            executor.drain();
        }
    });
    executor_core::try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
        executor.clone(),
        display_refresh_rate(mtm),
        inspector_probe,
    ))
    .unwrap_or_else(|_| panic!("hydrolysis UIKit: the main thread's executor is already attached"));

    let mut services = Environment::new();
    waterui::inspector::install(&mut services, inspector);
    let menus = declared();
    let application = Rc::new(UiKitApplication {
        executor,
        launch: RefCell::new(Some(Launch {
            app: Box::new(app),
            env,
            style: Rc::new(style),
            services,
        })),
        color_scheme: nami::binding(framework_scheme(uikit::current_scheme())),
        process: OnceCell::new(),
        scenes: RefCell::new(BTreeMap::new()),
        menus: Rc::clone(&menus),
        menu_watch: RefCell::new(None),
    });
    let handlers = ApplicationHandlers::new({
        let application = Rc::clone(&application);
        move |scene| application.connect(scene)
    })
    .build_menus(build_handler(menus))
    .did_finish_launching({
        let application = Rc::clone(&application);
        move |_| application.did_finish_launching()
    })
    .scene_will_enter_foreground({
        let application = Rc::clone(&application);
        move |id| application.with_scene(id, UiKitScene::entered_foreground)
    })
    .scene_did_enter_background({
        let application = Rc::clone(&application);
        move |id| application.with_scene(id, UiKitScene::entered_background)
    })
    .scene_did_become_active({
        let application = Rc::clone(&application);
        move |id| application.with_scene(id, UiKitScene::became_active)
    })
    .scene_will_resign_active({
        let application = Rc::clone(&application);
        move |id| application.with_scene(id, UiKitScene::resigned_active)
    })
    .scene_did_disconnect(move |id| application.disconnect(id));
    uikit::run(mtm, handlers)
}

/// The fastest attached screen's refresh rate, for the executor's frame
/// budget.
fn display_refresh_rate(mtm: MainThreadMarker) -> waterui::task::RefreshRate {
    use cocoa_ui::objc2_ui_kit::UIScreen;
    use waterui::task::RefreshRate;

    // `screens` is deprecated in favour of scene-session discovery, but at
    // startup no scene session exists yet — this is the only API that can
    // answer here.
    #[expect(deprecated, reason = "no scene session exists at process startup")]
    let fastest = UIScreen::screens(mtm)
        .iter()
        .map(|screen| screen.maximumFramesPerSecond())
        .max()
        .filter(|fps| *fps > 0);
    fastest.map_or_else(
        || {
            // The budget only scales stall diagnostics, so metadata the
            // platform does not expose is budgeted at the nominal rate.
            tracing::info!(
                target: "waterui::runtime_guard",
                "display refresh rate is unavailable; budgeting frames at the nominal rate"
            );
            RefreshRate::HEADLESS
        },
        |fps| {
            let hertz = u32::try_from(fps).expect("the filtered positive rate fits a `u32`");
            RefreshRate::from_millihertz(
                std::num::NonZeroU32::new(hertz * 1000).expect("a refresh rate of at least 1 Hz"),
            )
        },
    )
}

/// The `tracing` filter this process runs with: `RUST_LOG` wins outright,
/// otherwise `WATERUI_LOG` names the level and the graphics stack stays at
/// `error`.
fn env_filter() -> tracing_subscriber::EnvFilter {
    use tracing_subscriber::EnvFilter;

    if let Ok(filter) = EnvFilter::try_from_default_env() {
        return filter;
    }
    let level = std::env::var(LOG_LEVEL_ENV).unwrap_or_else(|_| String::from("error"));
    EnvFilter::try_new(format!(
        "{level},wgpu_core=error,wgpu_hal=error,naga=error,metal=error"
    ))
    .unwrap_or_else(|error| panic!("{LOG_LEVEL_ENV}={level:?} is not a tracing level: {error}"))
}

/// Sends `tracing` records to `os_log`, plus stderr when `WATERUI_LOG` asked
/// for a level — a physical device's unified log is unreachable from the
/// host, and `devicectl --console` only carries stderr.
fn init_tracing(inspector: Option<waterui::inspector::InspectorLayer>) {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let console_layer = std::env::var_os(LOG_LEVEL_ENV).is_some().then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .without_time()
    });
    let native_layer = tracing_subscriber::fmt::layer()
        .with_writer(cocoa_ui::log::NativeLog)
        .without_time()
        .with_ansi(false);
    tracing_subscriber::registry()
        .with(env_filter())
        .with(native_layer)
        .with(console_layer)
        .with(inspector)
        .init();
}
