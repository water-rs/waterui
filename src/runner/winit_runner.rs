//! Desktop event loop on winit, including AccessKit integration.

use async_task::spawn_local as spawn_local_task;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::mem;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use accesskit::ActivationHandler;
use accesskit_winit::{
    Adapter as AccessKitAdapter, Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent,
};
use executor_core::{
    LocalExecutor,
    async_task::{AsyncTask, Runnable},
    try_init_local_executor,
};
use nami::Signal;
use waterui::app::{App, AppParts, LastWindowPolicy};
use waterui::window::{Window, WindowState};
use waterui_core::Environment;
#[cfg(hydrolysis_wayland_platform)]
use waterui_core::Str;
use waterui_text::FontCollection;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, DeviceEvents, EventLoop};
#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
use winit::window::{Window as NativeWindow, WindowId};

use crate::platform::{PlatformWindow, WinitGpuContext, WinitWindow};
use crate::renderer::{
    HydrolysisRenderer, HydrolysisTextContextMenuMode, HydrolysisWindowOrigin,
    MenuShortcutRegistry, PopupWindowManager,
};
use crate::runner::{
    RenderDiagnosticsConfig, RuntimeWindow, advance_runtime, handle_input_events_with,
    pump_window_semantics, render_window, runtime_window_origin,
};

enum RunnerEvent {
    PollLocalTasks,
    MountPendingWindows,
    AccessKit(AccessKitEvent),
    /// A deferred vello stash's GPU-completion watch resolved: sent by the
    /// poll driver thread once the tickets' submissions have retired. Carries
    /// the stash generation the watch was registered for, so a stale wake
    /// landing beside a newer stash cannot resolve it early.
    DeferredVelloReady(WindowId, u64),
    /// Sent by the termination handler installed in [`run`].
    ///
    /// No windowing system turns a termination signal into a winit event, on
    /// any desktop platform, so the runner listens for the signals itself. The
    /// variant exists wherever that handler does — every target with signals or
    /// Windows console control events.
    #[cfg(any(unix, windows))]
    Terminate,
}

/// What a termination signal does, given how many arrived before it.
#[cfg(any(unix, windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminationAction {
    /// Ask the event loop to tear the runtime down, the way the last window
    /// closing does.
    RequestExit,
    /// Stop asking. The loop had its chance and did not take it.
    ForceExit,
}

/// Collapses repeated termination signals into "ask once, then stop asking".
///
/// Installing a handler replaces the default disposition of SIGINT, so without
/// this a wedged process becomes unkillable from the terminal that started it:
/// the graceful request is only reachable through the very event loop the hang
/// lives in.
#[cfg(any(unix, windows))]
#[derive(Debug, Default)]
struct TerminationRequests {
    requested: std::sync::atomic::AtomicBool,
}

#[cfg(any(unix, windows))]
impl TerminationRequests {
    fn record(&self) -> TerminationAction {
        if self
            .requested
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            TerminationAction::ForceExit
        } else {
            TerminationAction::RequestExit
        }
    }
}

/// Shell convention for a process killed by SIGINT (128 + 2). `ctrlc` does not
/// report which signal arrived, and Ctrl-C is what a user is pressing when the
/// forced path is reached.
#[cfg(any(unix, windows))]
const FORCED_TERMINATION_EXIT_CODE: i32 = 130;

/// Turns termination signals into [`RunnerEvent::Terminate`].
///
/// The `termination` feature of `ctrlc` covers SIGINT, SIGTERM and SIGHUP on
/// Unix and the console close/logoff/shutdown events on Windows, so every way a
/// desktop shell or session manager asks a windowed app to stop reaches the
/// same teardown the last window closing does.
#[cfg(any(unix, windows))]
fn install_termination_handler(event_proxy: &winit::event_loop::EventLoopProxy<RunnerEvent>) {
    let event_proxy = event_proxy.clone();
    let requests = TerminationRequests::default();
    ctrlc::set_handler(move || match requests.record() {
        TerminationAction::RequestExit => {
            // `ctrlc` runs this on a thread of its own rather than inside a
            // signal handler, so waking the loop from here is an ordinary send.
            let _ = event_proxy.send_event(RunnerEvent::Terminate);
        }
        TerminationAction::ForceExit => {
            tracing::warn!(
                "hydrolysis runner: termination signal repeated, exiting without runtime teardown"
            );
            std::process::exit(FORCED_TERMINATION_EXIT_CODE);
        }
    })
    .expect("hydrolysis runner: failed to install the termination handler");
}

/// Whether the event loop ends, given the application's last-window policy
/// and how many windows it still owns, mounted or waiting to be mounted.
///
/// Asked at startup, where an application that declares no window and quits
/// after its last one has nothing to run, and again whenever a window closes.
const fn ends_event_loop(policy: LastWindowPolicy, open_windows: usize) -> bool {
    match policy {
        LastWindowPolicy::Quit => open_windows == 0,
        LastWindowPolicy::StayResident => false,
    }
}

struct PendingWindow {
    window: Window,
    activates: bool,
}

impl PendingWindow {
    fn application(window: Window) -> Self {
        Self {
            window,
            activates: true,
        }
    }

    fn popup(window: Window) -> Self {
        Self {
            window,
            activates: false,
        }
    }
}

impl From<AccessKitEvent> for RunnerEvent {
    fn from(value: AccessKitEvent) -> Self {
        Self::AccessKit(value)
    }
}

#[derive(Clone)]
struct InitialAccessibilityTree {
    tree_update: accesskit::TreeUpdate,
}

impl ActivationHandler for InitialAccessibilityTree {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        Some(self.tree_update.clone())
    }
}

#[derive(Clone)]
struct WinitMainThreadExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
}

impl LocalExecutor for WinitMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let event_proxy = self.event_proxy.clone();
        let (runnable, task) = spawn_local_task(fut, move |runnable: Runnable| {
            if let Err(unsent) = runnable_tx.send(runnable) {
                // Teardown race: a waker held by another thread fired after
                // the event loop dropped the receiver. Dropping a
                // `spawn_local` runnable off its spawning thread panics by
                // design (async-task's thread check), so leak it instead —
                // bounded to shutdown, reclaimed at process exit.
                std::mem::forget(unsent);
                return;
            }
            let _ = event_proxy.send_event(RunnerEvent::PollLocalTasks);
        });
        runnable.schedule();
        AsyncTask::from(task)
    }
}

pub fn run(
    app: App,
    style: impl crate::Style,
    inspector: Option<waterui::inspector::InspectorRuntime>,
) {
    let AppParts {
        windows,
        menu_bar,
        env,
        last_window,
    } = app.into_parts();
    if ends_event_loop(last_window, windows.len()) {
        tracing::info!(
            "hydrolysis runner: the application declares no window and quits after its last one; \
             nothing to run"
        );
        return;
    }
    let mut event_loop_builder = EventLoop::<RunnerEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    event_loop_builder
        .with_activation_policy(ActivationPolicy::Regular)
        .with_activate_ignoring_other_apps(true);
    let event_loop = event_loop_builder
        .build()
        .expect("hydrolysis runner: failed to create event loop");
    let event_proxy = event_loop.create_proxy();
    let (local_runnable_tx, local_runnable_rx) = mpsc::channel::<Runnable>();
    let local_executor = WinitMainThreadExecutor {
        runnable_tx: local_runnable_tx,
        event_proxy: event_proxy.clone(),
    };

    // The reactive graph is thread-confined, so its observer is installed here,
    // on the thread that owns the event loop, and lives as long as the loop.
    #[cfg(feature = "inspector-signals")]
    let _signal_scope = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::observe_signals);

    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    waterui::inspector::install(&mut env, inspector);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    super::install_native_component_hooks(&mut env);
    #[cfg(any(unix, windows))]
    install_termination_handler(&event_proxy);
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    env.insert(waterui::window::WindowManager::new({
        let pending_window_queue = Rc::clone(&pending_window_queue);
        let event_proxy = event_proxy.clone();
        move |window| {
            pending_window_queue
                .borrow_mut()
                .push(PendingWindow::application(window));
            let _ = event_proxy.send_event(RunnerEvent::MountPendingWindows);
        }
    }));
    env.insert(PopupWindowManager::new({
        let pending_window_queue = Rc::clone(&pending_window_queue);
        let event_proxy = event_proxy.clone();
        move |window| {
            pending_window_queue
                .borrow_mut()
                .push(PendingWindow::popup(window));
            let _ = event_proxy.send_event(RunnerEvent::MountPendingWindows);
        }
    }));
    // Every window of the app resolves menu chords through the shared
    // registry: a compositor that hands a popup window keyboard focus routes
    // the menu's chords to that window's dispatch, which must resolve them
    // while the menu is open (water-rs/hydrolysis#247).
    let _ = env.get_or_insert_with::<MenuShortcutRegistry, _>(MenuShortcutRegistry::default);
    // The app's menu bar: its command chords arm on the shared registry,
    // and the native menu-bar surface installs where the platform has
    // one — see `menu_bar` for the per-platform contract (the two chord
    // paths see disjoint keys, so dispatch stays exactly-once).
    // Only the winit runner turns the resolved menus into a native
    // surface: `NSApp.mainMenu` on macOS, an `HMENU` per application
    // window on Windows. Every other runner just arms the chords. The
    // event loop is the main thread, which is what the install's
    // `MainThreadMarker` contract needs.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let native_menu_bar = crate::platform::native_menu_bar::NativeMenuBar::install(
        super::menu_bar::register_menu_bar(&menu_bar, &env),
        &env,
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    super::menu_bar::register_menu_bar(&menu_bar, &env);
    crate::theme::install_theme_tokens(&mut env, Some(&style));
    let theme: Rc<dyn crate::engine::WidgetTheme> = Rc::new(style);
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::new(Rc::clone(&theme)),
    ));
    // The application's fonts, discovered once. Every window's renderer is
    // seeded from this collection, and a self-drawn component that typesets
    // text itself reads it out of the environment instead of enumerating the
    // system's fonts for itself.
    let fonts = FontCollection::new(super::native_resource_fonts());
    fonts.clone().install(&mut env);
    let window_icon = load_staged_window_icon();
    let mut runner = WinitRunner {
        env,
        theme,
        fonts,
        window_icon,
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        native_menu_bar,
        pending_windows: windows
            .into_iter()
            .map(PendingWindow::application)
            .collect(),
        last_window,
        pending_window_queue,
        windows: HashMap::new(),
        popup_window_ids: std::collections::HashSet::new(),
        gpu_context: None,
        accesskit_adapters: HashMap::new(),
        last_accessibility_updates: HashMap::new(),
        local_executor: Some(local_executor),
        local_runnable_rx,
        event_proxy,
        render_diagnostics_config,
        outside_pointer_presses: 0,
    };

    event_loop.set_control_flow(ControlFlow::Wait);
    // Raw pointer presses are reported for every device on the seat, not just
    // presses over our own windows — that is the only way a press on the
    // desktop or in another app reaches us, and it is what dismisses popup
    // windows when the pointer goes down outside them.
    event_loop.listen_device_events(DeviceEvents::Always);
    let run_result = event_loop.run_app(&mut runner);
    waterui_locale::shutdown_current_thread_runtime_locale_state();
    let _ = runner.drain_local_executor_queue();
    run_result.expect("hydrolysis runner: event loop failed");
}

struct WinitRunner {
    env: Environment,
    /// The style the runtime was launched with: every window's renderer
    /// measures and encodes with this widget theme.
    theme: Rc<dyn crate::engine::WidgetTheme>,
    /// The application's font collection, the same one the environment carries.
    /// Every window's renderer is seeded from it.
    fonts: FontCollection,
    /// Taskbar/window icon staged by the water CLI next to the asset bundle.
    /// X11 and Windows honor it; macOS uses the bundle's icns and Wayland
    /// resolves icons through the desktop entry instead.
    window_icon: Option<winit::window::Icon>,
    /// The installed `App::menu_bar` native surface (macOS
    /// `NSApp.mainMenu`, Windows per-window `HMENU`): kept alive for the
    /// app's duration and pumped on every event-loop pass. Linux has no
    /// surface — the chords are still armed (see `menu_bar`).
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_menu_bar: crate::platform::native_menu_bar::NativeMenuBar,
    pending_windows: Vec<PendingWindow>,
    /// What the application does once it has no window left.
    last_window: LastWindowPolicy,
    pending_window_queue: Rc<RefCell<Vec<PendingWindow>>>,
    windows: HashMap<WindowId, RuntimeWindow<WinitWindow>>,
    /// Transient popup windows (mounted through `PopupWindowManager`): a
    /// pointer press that lands outside every app window dismisses them.
    popup_window_ids: std::collections::HashSet<WindowId>,
    gpu_context: Option<WinitGpuContext>,
    accesskit_adapters: HashMap<WindowId, AccessKitAdapter>,
    last_accessibility_updates: HashMap<WindowId, accesskit::TreeUpdate>,
    /// The executor is installed the first time the active event loop is
    /// reachable (the first `resumed`), which is the earliest winit 0.30
    /// exposes monitor enumeration.
    local_executor: Option<WinitMainThreadExecutor>,
    local_runnable_rx: mpsc::Receiver<Runnable>,
    event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
    render_diagnostics_config: RenderDiagnosticsConfig,
    /// Raw pointer presses seen at the device level but not (yet) matched by a
    /// window-level pointer event in the same event batch. What is left over
    /// when the batch settles is a press outside every window we own.
    outside_pointer_presses: u32,
}

/// Loads the window icon the water CLI stages into the asset bundle root.
///
/// Absence is a legitimate state (bare `cargo run`, tests, previews without
/// staging); a present-but-undecodable icon is reported and skipped.
fn load_staged_window_icon() -> Option<winit::window::Icon> {
    let root = waterui_assets::bundle_root().ok()?;
    let path = root.join(waterui_assets::WINDOW_ICON_FILE);
    let file = std::fs::File::open(&path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = match decoder.read_info() {
        Ok(reader) => reader,
        Err(error) => {
            tracing::warn!(
                "staged window icon {} is not decodable: {error}",
                path.display()
            );
            return None;
        }
    };
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let info = match reader.next_frame(&mut pixels) {
        Ok(info) => info,
        Err(error) => {
            tracing::warn!(
                "staged window icon {} is not decodable: {error}",
                path.display()
            );
            return None;
        }
    };
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        tracing::warn!(
            "staged window icon {} must be 8-bit RGBA, got {:?}/{:?}",
            path.display(),
            info.color_type,
            info.bit_depth
        );
        return None;
    }
    pixels.truncate(info.buffer_size());
    match winit::window::Icon::from_rgba(pixels, info.width, info.height) {
        Ok(icon) => Some(icon),
        Err(error) => {
            tracing::warn!("staged window icon {} is unusable: {error}", path.display());
            None
        }
    }
}

fn native_window_attributes(
    window: &Window,
    env: &Environment,
    activates: bool,
    icon: Option<winit::window::Icon>,
) -> winit::window::WindowAttributes {
    let frame = crate::platform::validated_window_frame(window.frame.snapshot());
    // A Fullscreen request present at creation travels as a window
    // attribute — the same way the position does — so the window manager
    // sees it with the map request instead of after it. The retry in
    // `apply_properties` still re-delivers it on the first mapped event:
    // a state written between creation and map, or a manager that ignored
    // the attribute, is covered by the same mapped signal.
    let fullscreen = matches!(
        window.state.snapshot(),
        waterui::window::WindowState::Fullscreen
    );
    let attributes = NativeWindow::default_attributes()
        .with_window_icon(icon)
        .with_title(window.display_title().snapshot().as_str())
        .with_resizable(window.resizable)
        .with_visible(false)
        .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
        .with_active(activates)
        .with_transparent(super::window_requires_transparency(window, env))
        .with_decorations(!matches!(
            window.style,
            waterui::window::WindowStyle::Borderless
        ))
        .with_inner_size(winit::dpi::LogicalSize::new(
            frame.width() as f64,
            frame.height() as f64,
        ))
        // The requested position is a creation-time attribute: on X11 a
        // position set on an unmapped window is dropped, so it must travel
        // with the map request for the window manager to honor it.
        .with_position(winit::dpi::LogicalPosition::new(
            frame.x() as f64,
            frame.y() as f64,
        ));

    // The window's desktop identity: on X11 the `WM_CLASS` pair is split
    // between its class (winit's `general`, the window's resolved `app_id`)
    // and its instance (`res_name`, the window's `instance_name` — which
    // resolves to the `app_id` when undeclared); on Wayland the `app_id`
    // alone names the `xdg_toplevel`. Window-manager rules, desktop-file
    // matching and dock grouping land on the identifier the build compiled
    // in as `WATERUI_APP_ID` or the window declared itself. An empty
    // identity leaves winit's default (the executable name) in place.
    #[cfg(hydrolysis_wayland_platform)]
    let attributes = {
        use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
        match window_desktop_identity(window) {
            Some((class, instance)) => WindowAttributesExtWayland::with_name(
                WindowAttributesExtX11::with_name(attributes, class.as_str(), instance.as_str()),
                class.as_str(),
                instance.as_str(),
            ),
            None => attributes,
        }
    };
    attributes
}

/// The window's desktop identity as a `(class, instance)` pair.
///
/// `class` is the window's resolved `app_id` — the X11 `WM_CLASS` class
/// part and the Wayland `app_id`; `instance` is the window's
/// `instance_name`, resolving to the `app_id` when undeclared so a window
/// that does not opt in keeps the historical one-name behaviour. `None`
/// when the window and the application both leave the identity unset, so
/// winit's default (the executable name) applies.
#[cfg(hydrolysis_wayland_platform)]
fn window_desktop_identity(window: &Window) -> Option<(Str, Str)> {
    let class = window.display_app_id();
    (!class.is_empty()).then(|| (class, window.display_instance_name()))
}

impl WinitRunner {
    fn drain_runnable_queue(local_runnable_rx: &mpsc::Receiver<Runnable>) -> bool {
        let mut drained = false;
        while let Ok(runnable) = local_runnable_rx.try_recv() {
            drained = true;
            runnable.run();
        }
        drained
    }

    fn drain_local_executor_queue(&self) -> bool {
        Self::drain_runnable_queue(&self.local_runnable_rx)
    }

    fn exit_after_runtime_cleanup(&self, event_loop: &ActiveEventLoop) {
        waterui_locale::shutdown_current_thread_runtime_locale_state();
        let _ = self.drain_local_executor_queue();
        event_loop.exit();
    }

    /// Ends the event loop when the application's last-window policy says a
    /// runner with no window left stops.
    fn exit_if_last_window_closed(&self, event_loop: &ActiveEventLoop) {
        if ends_event_loop(
            self.last_window,
            self.windows.len() + self.pending_windows.len(),
        ) {
            self.exit_after_runtime_cleanup(event_loop);
        }
    }

    fn current_window_origin(runtime: &RuntimeWindow<WinitWindow>) -> HydrolysisWindowOrigin {
        let native_window = runtime.platform.native_window();
        if let Ok(position) = native_window.outer_position() {
            let logical = position.to_logical::<f64>(native_window.scale_factor());
            return HydrolysisWindowOrigin {
                x: logical.x as f32,
                y: logical.y as f32,
            };
        }
        HydrolysisWindowOrigin {
            ..runtime_window_origin(runtime)
        }
    }
    fn create_runtime_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        pending: PendingWindow,
    ) -> (RuntimeWindow<WinitWindow>, AccessKitAdapter) {
        let PendingWindow { window, activates } = pending;
        let attributes =
            native_window_attributes(&window, &self.env, activates, self.window_icon.clone());

        let native_window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("hydrolysis runner: failed to create winit window"),
        );
        #[cfg(target_os = "windows")]
        if activates {
            // Windows' menu bar lives on the window: attach the app bar's
            // HMENU to this HWND (see `menu_bar` for the platform contract).
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = native_window.window_handle()
                && let RawWindowHandle::Win32(win) = handle.as_raw()
            {
                self.native_menu_bar.attach_hwnd(win.hwnd.get());
            }
        }
        let (mut platform, gpu_context) = pollster::block_on(WinitWindow::new_with_shared_gpu(
            native_window,
            self.gpu_context.as_ref(),
            super::window_requires_transparency(&window, &self.env),
        ));
        if self.gpu_context.is_none() {
            self.gpu_context = Some(gpu_context);
        }
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.adapter(), surface.device(), Rc::clone(&self.theme))
        };
        super::seed_core(&mut renderer, &self.fonts);
        let mut runtime =
            RuntimeWindow::new(window, platform, renderer, self.render_diagnostics_config);
        runtime
            .renderer
            .set_window_id(crate::renderer::WindowId::Winit(runtime.platform.id()));
        {
            // The deferred-verification settle wakes through the runner's
            // own user-event path — the same mechanism the executor and
            // termination handler use. The shared per-device poll driver
            // parks on GPU completion and fires this wake; the main thread
            // never polls the device itself.
            let event_proxy = self.event_proxy.clone();
            let window_id = runtime.platform.id();
            runtime.deferred_vello_wake = Some(std::sync::Arc::new(move |stash_gen| {
                // send_event fails only once the event loop has exited — a
                // dead loop has no settle left to wake, so the error is
                // ignored.
                let _ =
                    event_proxy.send_event(RunnerEvent::DeferredVelloReady(window_id, stash_gen));
            }));
            runtime.deferred_poll_driver = self
                .gpu_context
                .as_ref()
                .map(|gpu| gpu.poll_driver().clone());
        }
        if !activates {
            self.popup_window_ids.insert(runtime.platform.id());
        }
        let _ = pump_window_semantics(&mut runtime, &self.env);
        // The adapter is created unconditionally: accesskit resolves the
        // platform accessibility bus itself, lazily, so a missing org.a11y.Bus
        // is its concern — probing it up front only raced the bus's startup.
        let adapter = {
            let initial_tree_update = runtime.renderer.take_accessibility_tree_update().expect(
                "hydrolysis winit accessibility: initial tree update missing after initial semantic rebuild",
            );
            let last_tree_update = initial_tree_update.clone();
            let adapter = AccessKitAdapter::with_mixed_handlers(
                event_loop,
                runtime.platform.native_window(),
                InitialAccessibilityTree {
                    tree_update: initial_tree_update,
                },
                self.event_proxy.clone(),
            );
            tracing::trace!(
                target: "waterui::hydrolysis::a11y",
                window_id = ?runtime.platform.id(),
                title = runtime.window.title.snapshot().as_str(),
                "created accesskit adapter for window"
            );
            self.last_accessibility_updates
                .insert(runtime.platform.id(), last_tree_update);
            adapter
        };
        runtime.platform.native_window().set_visible(true);
        if activates {
            runtime.platform.native_window().focus_window();
        }
        (runtime, adapter)
    }

    /// Installs the monitored main-thread executor on the first active event
    /// loop callback, still ahead of the first window mount.
    ///
    /// `available_monitors` exists only on `ActiveEventLoop`/`Window` in winit
    /// 0.30, so the highest connected refresh rate can only be read here.
    fn start_main_thread_executor(&mut self, event_loop: &ActiveEventLoop) {
        let Some(local_executor) = self.local_executor.take() else {
            return;
        };
        let refresh_rate = event_loop
            .available_monitors()
            .filter_map(|monitor| monitor.refresh_rate_millihertz())
            .max()
            .and_then(NonZeroU32::new)
            .map_or_else(
                || {
                    // Some Wayland compositors report no rate (winit returns
                    // None); pace the executor's frame budget at the headless rate.
                    tracing::debug!(
                        "hydrolysis runner: no monitor reported a refresh rate, executor paced at the headless rate"
                    );
                    waterui::task::RefreshRate::HEADLESS
                },
                waterui::task::RefreshRate::from_millihertz,
            );
        let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
            local_executor,
            refresh_rate,
            self.env
                .get::<waterui::inspector::InspectorRuntime>()
                .map(waterui::inspector::InspectorRuntime::runtime_probe),
        ));
        // Locale changes reach views through a mailbox, whose pump needs the
        // executor installed just above.
        waterui_locale::start_system_locale_listener();
    }

    fn mount_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
        self.start_main_thread_executor(event_loop);
        let mut pending = mem::take(&mut self.pending_windows);
        pending.extend(self.pending_window_queue.borrow_mut().drain(..));
        for pending in pending {
            let (runtime, adapter) = self.create_runtime_window(event_loop, pending);
            let id = runtime.platform.id();
            self.windows.insert(id, runtime);
            self.accesskit_adapters.insert(id, adapter);
        }
    }

    fn handle_input_events(runtime: &mut RuntimeWindow<WinitWindow>, env: &Environment) -> bool {
        handle_input_events_with(runtime, env, |runtime, env| {
            env.extending(Self::current_window_origin(runtime))
        })
    }

    /// Lets the native menu bar forget a closing window's HWND before
    /// the winit window (and its HWND) is destroyed, so `init_for_hwnd`
    /// and `remove_for_hwnd` only ever run on live handles.
    #[cfg(target_os = "windows")]
    fn detach_menu_bar_hwnd(&self, runtime: &RuntimeWindow<WinitWindow>) {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = runtime.platform.native_window().window_handle()
            && let RawWindowHandle::Win32(win) = handle.as_raw()
        {
            self.native_menu_bar.detach_hwnd(win.hwnd.get());
        }
    }

    fn remove_closed_windows(&mut self, event_loop: &ActiveEventLoop) {
        let mut close_ids = Vec::new();
        for (id, runtime) in &mut self.windows {
            runtime.platform.apply_properties(&runtime.window);
            if runtime.window.state.snapshot() == WindowState::Closed {
                close_ids.push(*id);
            }
        }

        for id in close_ids {
            #[cfg(target_os = "windows")]
            if let Some(runtime) = self.windows.get(&id) {
                self.detach_menu_bar_hwnd(runtime);
            }
            self.windows.remove(&id);
            self.popup_window_ids.remove(&id);
            self.accesskit_adapters.remove(&id);
            self.last_accessibility_updates.remove(&id);
        }

        self.exit_if_last_window_closed(event_loop);
    }

    fn flush_cross_window_rebuild_requests(&mut self) {
        for runtime in self.windows.values_mut() {
            if runtime.renderer.take_rebuild_request() {
                runtime.request_refresh();
                runtime.platform.request_redraw();
                runtime.renderer.migration_counters_mut().host_wakeups += 1;
            }
        }
    }
}

impl ApplicationHandler<RunnerEvent> for WinitRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let _ = self.drain_local_executor_queue();
        self.mount_pending_windows(event_loop);
        for runtime in self.windows.values_mut() {
            runtime.platform.request_redraw();
            runtime.renderer.migration_counters_mut().host_wakeups += 1;
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let _ = self.drain_local_executor_queue();
        let is_pointer_press = match &event {
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => true,
            WindowEvent::Touch(touch) => touch.phase == TouchPhase::Started,
            _ => false,
        };
        if is_pointer_press {
            // A press that lands on one of our windows cancels out one raw
            // device-level press; what remains at the end of the batch is
            // presses outside every window we own.
            self.outside_pointer_presses = self.outside_pointer_presses.saturating_sub(1);
        }
        let should_close = {
            let Some(runtime) = self.windows.get_mut(&window_id) else {
                return;
            };
            if let Some(adapter) = self.accesskit_adapters.get_mut(&window_id) {
                adapter.process_event(runtime.platform.native_window(), &event);
            }
            runtime.platform.handle_window_event(&event);
            Self::handle_input_events(runtime, &self.env)
        };

        if !self.pending_window_queue.borrow().is_empty() || !self.pending_windows.is_empty() {
            self.mount_pending_windows(event_loop);
        }

        self.flush_cross_window_rebuild_requests();

        if should_close {
            #[cfg(target_os = "windows")]
            if let Some(runtime) = self.windows.get(&window_id) {
                self.detach_menu_bar_hwnd(runtime);
            }
            self.windows.remove(&window_id);
            self.popup_window_ids.remove(&window_id);
            self.accesskit_adapters.remove(&window_id);
            self.last_accessibility_updates.remove(&window_id);
            self.exit_if_last_window_closed(event_loop);
            return;
        }

        if let WindowEvent::RedrawRequested = event {
            let Some(runtime) = self.windows.get_mut(&window_id) else {
                return;
            };
            render_window(runtime, &self.env, &mut || {
                Self::drain_runnable_queue(&self.local_runnable_rx)
            });
            if let Some(adapter) = self.accesskit_adapters.get_mut(&window_id) {
                if let Some(update) = runtime.renderer.take_accessibility_tree_update() {
                    self.last_accessibility_updates
                        .insert(window_id, update.clone());
                    tracing::trace!(
                        target: "waterui::hydrolysis::a11y",
                        window_id = ?window_id,
                        "publishing accessibility tree update on redraw"
                    );
                    adapter.update_if_active(|| update);
                } else {
                    tracing::trace!(
                        target: "waterui::hydrolysis::a11y",
                        window_id = ?window_id,
                        "no accessibility tree update available on redraw"
                    );
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _ = self.drain_local_executor_queue();
        // Native menu-bar clicks: muda posts them on its channel from the
        // main thread — drain here so each dispatches on the event loop.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        self.native_menu_bar.pump_menu_events();
        self.mount_pending_windows(event_loop);
        let now = Instant::now();
        let mut next_gesture_deadline: Option<Instant> = None;
        for runtime in self.windows.values_mut() {
            if let Some(deadline) = advance_runtime(runtime, &self.env, now) {
                next_gesture_deadline = Some(match next_gesture_deadline {
                    Some(existing) => existing.min(deadline),
                    None => deadline,
                });
            }
        }
        if let Some(deadline) = next_gesture_deadline {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
        if self.outside_pointer_presses > 0 {
            self.outside_pointer_presses = 0;
            // A raw device press left over after window-level presses are
            // accounted for landed outside every window we own — on X11 the
            // server only delivers pointer events for presses on our own
            // windows, so this is the only signal a press on the desktop or
            // another app ever reaches us. That outside press is what popup
            // windows dismiss on.
            for id in &self.popup_window_ids {
                if let Some(runtime) = self.windows.get(id) {
                    runtime.window.state.set(WindowState::Closed);
                }
            }
        }
        self.remove_closed_windows(event_loop);
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::Button {
            state: ElementState::Pressed,
            ..
        } = event
        {
            self.outside_pointer_presses += 1;
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: RunnerEvent) {
        match event {
            RunnerEvent::PollLocalTasks => {
                let _ = self.drain_local_executor_queue();
            }
            RunnerEvent::MountPendingWindows => {
                self.mount_pending_windows(_event_loop);
            }
            RunnerEvent::DeferredVelloReady(window_id, stash_gen) => {
                let Some(runtime) = self.windows.get_mut(&window_id) else {
                    return;
                };
                // The generation records unconditionally: a wake that lands
                // while damage is pending still retired the stash's
                // submissions — skipping it parks an armed settle forever.
                runtime.deferred_wake_gen = runtime.deferred_wake_gen.max(stash_gen);
                // Only a stash still awaiting its present needs the settle
                // redraw; a wake for one the frame loop already presented
                // resolves nothing.
                if runtime.renderer.has_deferred_vello_frame() {
                    runtime.platform.request_redraw();
                }
            }
            RunnerEvent::AccessKit(event) => {
                let Some(runtime) = self.windows.get_mut(&event.window_id) else {
                    return;
                };
                let Some(adapter) = self.accesskit_adapters.get_mut(&event.window_id) else {
                    return;
                };
                match event.window_event {
                    AccessKitWindowEvent::InitialTreeRequested => {
                        tracing::trace!(
                            target: "waterui::hydrolysis::a11y",
                            window_id = ?event.window_id,
                            "accesskit initial tree requested"
                        );
                        if let Some(update) = runtime.renderer.take_accessibility_tree_update() {
                            self.last_accessibility_updates
                                .insert(event.window_id, update.clone());
                            tracing::trace!(
                                target: "waterui::hydrolysis::a11y",
                                window_id = ?event.window_id,
                                "publishing accessibility tree update for initial request"
                            );
                            adapter.update_if_active(|| update);
                        } else if let Some(update) = self
                            .last_accessibility_updates
                            .get(&event.window_id)
                            .cloned()
                        {
                            tracing::trace!(
                                target: "waterui::hydrolysis::a11y",
                                window_id = ?event.window_id,
                                "replaying cached accessibility tree update for initial request"
                            );
                            adapter.update_if_active(|| update);
                        } else {
                            tracing::trace!(
                                target: "waterui::hydrolysis::a11y",
                                window_id = ?event.window_id,
                                "missing accessibility tree update for initial request, scheduling rebuild"
                            );
                            runtime.request_refresh();
                            runtime.platform.request_redraw();
                            runtime.renderer.migration_counters_mut().host_wakeups += 1;
                        }
                    }
                    AccessKitWindowEvent::ActionRequested(request) => {
                        tracing::trace!(
                            target: "waterui::hydrolysis::a11y",
                            window_id = ?event.window_id,
                            action = ?request.action,
                            target = ?request.target_node,
                            "accesskit action requested"
                        );
                        let action_env = self.env.extending(Self::current_window_origin(runtime));
                        if runtime
                            .renderer
                            .handle_accessibility_action(request, &action_env)
                        {
                            runtime.request_refresh();
                            runtime.platform.request_redraw();
                            runtime.renderer.migration_counters_mut().host_wakeups += 1;
                        }
                        self.flush_cross_window_rebuild_requests();
                    }
                    AccessKitWindowEvent::AccessibilityDeactivated => {}
                }
            }
            #[cfg(any(unix, windows))]
            RunnerEvent::Terminate => {
                self.exit_after_runtime_cleanup(_event_loop);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(unix, windows))]
    use super::{TerminationAction, TerminationRequests};
    use super::{ends_event_loop, native_window_attributes};
    use waterui::window::{Window, WindowState};
    use waterui_core::{Binding, Environment, binding};

    #[cfg(any(unix, windows))]
    #[test]
    fn first_termination_signal_asks_the_loop_and_later_ones_do_not() {
        let requests = TerminationRequests::default();

        assert_eq!(requests.record(), TerminationAction::RequestExit);
        assert_eq!(requests.record(), TerminationAction::ForceExit);
        assert_eq!(requests.record(), TerminationAction::ForceExit);
    }

    #[test]
    fn quit_policy_ends_the_loop_once_no_window_is_left() {
        use waterui::app::LastWindowPolicy;

        // At startup, a quitting app that declares no window has nothing to run.
        assert!(ends_event_loop(LastWindowPolicy::Quit, 0));
        assert!(!ends_event_loop(LastWindowPolicy::Quit, 1));
        assert!(!ends_event_loop(LastWindowPolicy::Quit, 2));
    }

    #[test]
    fn stay_resident_policy_keeps_the_loop_with_no_window() {
        use waterui::app::LastWindowPolicy;

        assert!(!ends_event_loop(LastWindowPolicy::StayResident, 0));
        assert!(!ends_event_loop(LastWindowPolicy::StayResident, 1));
    }

    #[test]
    fn popup_window_attributes_do_not_activate() {
        let window = Window::new("", binding(WindowState::Normal), || ());
        let env = Environment::new();

        assert!(native_window_attributes(&window, &env, true, None).active);
        assert!(!native_window_attributes(&window, &env, false, None).active);
    }

    #[cfg(hydrolysis_wayland_platform)]
    #[test]
    fn window_attributes_split_wm_class_from_instance_name() {
        use waterui_core::Str;

        let mut window = Window::new("", binding(WindowState::Normal), || ());
        window.app_id = Some(Str::from("myclass"));
        window.instance_name = Some(Str::from("myinstance"));
        assert_eq!(
            super::window_desktop_identity(&window),
            Some((Str::from("myclass"), Str::from("myinstance"))),
            "WM_CLASS carries (app_id, instance_name)"
        );

        let mut window = Window::new("", binding(WindowState::Normal), || ());
        window.app_id = Some(Str::from("myclass"));
        assert_eq!(
            super::window_desktop_identity(&window),
            Some((Str::from("myclass"), Str::from("myclass"))),
            "an undeclared instance_name resolves to the app_id"
        );
    }

    #[test]
    fn window_attributes_carry_the_requested_frame() {
        use waterui_core::layout::{Point, Rect, Size};
        let frame = Binding::container(Rect::new(Point::new(12.0, 34.0), Size::new(800.0, 300.0)));
        let mut window = Window::new("", binding(WindowState::Normal), || ());
        window.frame = frame;
        let env = Environment::new();

        let attributes = native_window_attributes(&window, &env, false, None);
        assert_eq!(
            attributes.position,
            Some(winit::dpi::Position::Logical(
                winit::dpi::LogicalPosition::new(12.0, 34.0)
            )),
            "the requested origin must travel with the window attributes: a position set while unmapped is dropped on X11"
        );
        assert_eq!(
            attributes.inner_size,
            Some(winit::dpi::Size::Logical(winit::dpi::LogicalSize::new(
                800.0, 300.0
            ))),
        );
    }
}
