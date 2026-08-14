//! Desktop event loop on winit, including AccessKit integration.

use async_task::spawn_local as spawn_local_task;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::mem;
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
use waterui::app::App;
use waterui::window::{Window, WindowState};
use waterui_core::Environment;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
#[cfg(any(hydrolysis_wayland_platform, docsrs))]
use winit::platform::wayland::EventLoopExtWayland;
use winit::window::{Window as NativeWindow, WindowId};

use crate::platform::{PlatformWindow, WinitGpuContext, WinitWindow};
use crate::renderer::{
    HydrolysisRenderer, HydrolysisTextContextMenuMode, HydrolysisWindowOrigin, PopupWindowManager,
};
use crate::runner::{
    RenderDiagnosticsConfig, RuntimeWindow, advance_runtime, handle_input_events_with,
    pump_window_semantics, render_window, runtime_window_origin,
};

enum RunnerEvent {
    PollLocalTasks,
    MountPendingWindows,
    AccessKit(AccessKitEvent),
    Terminate,
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

pub fn run(app: App, inspector: Option<waterui::inspector::InspectorRuntime>) {
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
    let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
        local_executor,
        inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe),
    ));

    // Locale changes reach views through a mailbox, whose pump needs the
    // executor installed just above.
    waterui_locale::start_system_locale_listener();
    // The reactive graph is thread-confined, so its observer is installed here,
    // on the thread that owns the event loop, and lives as long as the loop.
    #[cfg(feature = "inspector-signals")]
    let _signal_scope = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::observe_signals);

    let (windows, _menu_bar, env) = app.into_parts();
    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    waterui::inspector::install(&mut env, inspector);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    super::install_native_component_hooks(&mut env);
    #[cfg(target_os = "macos")]
    ctrlc::set_handler({
        let event_proxy = event_proxy.clone();
        move || {
            let _ = event_proxy.send_event(RunnerEvent::Terminate);
        }
    })
    .expect("hydrolysis runner: failed to install the macOS termination handler");
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
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::default(),
    ));
    let mut runner = WinitRunner {
        env,
        pending_windows: windows
            .into_iter()
            .map(PendingWindow::application)
            .collect(),
        pending_window_queue,
        windows: HashMap::new(),
        gpu_context: None,
        accesskit_adapters: HashMap::new(),
        last_accessibility_updates: HashMap::new(),
        accessibility_enabled: super::probe_accessibility_runtime(),
        local_runnable_rx,
        event_proxy,
        render_diagnostics_config,
    };

    event_loop.set_control_flow(ControlFlow::Wait);
    let run_result = event_loop.run_app(&mut runner);
    waterui_locale::shutdown_current_thread_runtime_locale_state();
    let _ = runner.drain_local_executor_queue();
    run_result.expect("hydrolysis runner: event loop failed");
}

struct WinitRunner {
    env: Environment,
    pending_windows: Vec<PendingWindow>,
    pending_window_queue: Rc<RefCell<Vec<PendingWindow>>>,
    windows: HashMap<WindowId, RuntimeWindow<WinitWindow>>,
    gpu_context: Option<WinitGpuContext>,
    accesskit_adapters: HashMap<WindowId, AccessKitAdapter>,
    last_accessibility_updates: HashMap<WindowId, accesskit::TreeUpdate>,
    accessibility_enabled: bool,
    local_runnable_rx: mpsc::Receiver<Runnable>,
    event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
    render_diagnostics_config: RenderDiagnosticsConfig,
}

fn native_window_attributes(
    window: &Window,
    env: &Environment,
    activates: bool,
) -> winit::window::WindowAttributes {
    let frame = window.frame.get();
    NativeWindow::default_attributes()
        .with_title(window.title.get().as_str())
        .with_resizable(window.resizable)
        .with_visible(false)
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
    ) -> (RuntimeWindow<WinitWindow>, Option<AccessKitAdapter>) {
        let PendingWindow { window, activates } = pending;
        let attributes = native_window_attributes(&window, &self.env, activates);

        let native_window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("hydrolysis runner: failed to create winit window"),
        );
        let (mut platform, gpu_context) = pollster::block_on(WinitWindow::new_with_shared_gpu(
            native_window,
            self.gpu_context.as_ref(),
        ));
        if self.gpu_context.is_none() {
            self.gpu_context = Some(gpu_context);
        }
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        super::load_native_resource_fonts(&mut renderer);
        let mut runtime =
            RuntimeWindow::new(window, platform, renderer, self.render_diagnostics_config);
        let _ = pump_window_semantics(&mut runtime, &self.env);
        let adapter = if self.accessibility_enabled {
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
                title = runtime.window.title.get().as_str(),
                "created accesskit adapter for window"
            );
            self.last_accessibility_updates
                .insert(runtime.platform.id(), last_tree_update);
            Some(adapter)
        } else {
            tracing::warn!(
                target: "waterui::hydrolysis::a11y",
                window_id = ?runtime.platform.id(),
                title = runtime.window.title.get().as_str(),
                "accessibility adapter disabled: org.a11y.Bus is unavailable"
            );
            None
        };
        runtime.platform.native_window().set_visible(true);
        if activates {
            runtime.platform.native_window().focus_window();
        }
        (runtime, adapter)
    }

    fn mount_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
        let mut pending = mem::take(&mut self.pending_windows);
        pending.extend(self.pending_window_queue.borrow_mut().drain(..));
        for pending in pending {
            let (runtime, adapter) = self.create_runtime_window(event_loop, pending);
            let id = runtime.platform.id();
            self.windows.insert(id, runtime);
            if let Some(adapter) = adapter {
                self.accesskit_adapters.insert(id, adapter);
            }
        }
    }

    fn handle_input_events(runtime: &mut RuntimeWindow<WinitWindow>, env: &Environment) -> bool {
        handle_input_events_with(runtime, env, |runtime, env| {
            env.extending(Self::current_window_origin(runtime))
        })
    }

    fn remove_closed_windows(&mut self, event_loop: &ActiveEventLoop) {
        let mut close_ids = Vec::new();
        for (id, runtime) in &mut self.windows {
            runtime.platform.apply_properties(&runtime.window);
            if runtime.window.state.get() == WindowState::Closed {
                close_ids.push(*id);
            }
        }

        for id in close_ids {
            self.windows.remove(&id);
            self.accesskit_adapters.remove(&id);
            self.last_accessibility_updates.remove(&id);
        }

        if self.windows.is_empty() && self.pending_windows.is_empty() {
            self.exit_after_runtime_cleanup(event_loop);
        }
    }

    fn flush_cross_window_rebuild_requests(&mut self) {
        for runtime in self.windows.values_mut() {
            if runtime.renderer.take_rebuild_request() {
                runtime.request_refresh();
                runtime.platform.request_redraw();
            }
        }
    }
}

impl ApplicationHandler<RunnerEvent> for WinitRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let _ = self.drain_local_executor_queue();
        self.mount_pending_windows(event_loop);
        for runtime in self.windows.values() {
            runtime.platform.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let _ = self.drain_local_executor_queue();
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
            self.windows.remove(&window_id);
            self.accesskit_adapters.remove(&window_id);
            self.last_accessibility_updates.remove(&window_id);
            if self.windows.is_empty() && self.pending_windows.is_empty() {
                self.exit_after_runtime_cleanup(event_loop);
            }
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
        self.remove_closed_windows(event_loop);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: RunnerEvent) {
        match event {
            RunnerEvent::PollLocalTasks => {
                let _ = self.drain_local_executor_queue();
            }
            RunnerEvent::MountPendingWindows => {
                self.mount_pending_windows(_event_loop);
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
                        }
                        self.flush_cross_window_rebuild_requests();
                    }
                    AccessKitWindowEvent::AccessibilityDeactivated => {}
                }
            }
            RunnerEvent::Terminate => {
                self.exit_after_runtime_cleanup(_event_loop);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::native_window_attributes;
    use waterui::window::{Window, WindowState};
    use waterui_core::{Environment, binding};

    #[test]
    fn popup_window_attributes_do_not_activate() {
        let window = Window::new("", binding(WindowState::Normal), || ());
        let env = Environment::new();

        assert!(native_window_attributes(&window, &env, true).active);
        assert!(!native_window_attributes(&window, &env, false).active);
    }
}
