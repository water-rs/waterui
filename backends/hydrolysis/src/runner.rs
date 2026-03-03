use std::time::{Duration, Instant};

use nami::Signal as _;
use waterui::app::App;
use waterui::component::table::TableConfig;
use waterui::window::{Window, WindowBackground};
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::view::Hook;

#[cfg(not(feature = "winit"))]
use crate::platform::OffscreenWindow;
use crate::platform::PlatformWindow;
use crate::renderer::HydrolysisRenderer;

fn init_main_thread_executors() {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = waterui::inspector::maybe_init_from_env();
}

fn install_native_component_hooks(env: &mut Environment) {
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}

const DEFAULT_RENDER_DIAG_INTERVAL_MS: u64 = 1_000;
const DEFAULT_RENDER_DIAG_SLOW_FRAME_MS: u64 = 16;

#[derive(Clone, Copy)]
struct RenderDiagnosticsConfig {
    enabled: bool,
    interval: Duration,
    slow_frame_threshold: Duration,
}

impl RenderDiagnosticsConfig {
    fn from_env() -> Self {
        let enabled = parse_bool_env("WATERUI_HYDROLYSIS_RENDER_DIAG", false);
        let interval_ms = parse_positive_u64_env(
            "WATERUI_HYDROLYSIS_RENDER_DIAG_INTERVAL_MS",
            DEFAULT_RENDER_DIAG_INTERVAL_MS,
        );
        let slow_frame_ms = parse_positive_u64_env(
            "WATERUI_HYDROLYSIS_RENDER_DIAG_SLOW_FRAME_MS",
            DEFAULT_RENDER_DIAG_SLOW_FRAME_MS,
        );

        Self {
            enabled,
            interval: Duration::from_millis(interval_ms),
            slow_frame_threshold: Duration::from_millis(slow_frame_ms),
        }
    }
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => panic!(
                "hydrolysis runner: invalid {name} value `{raw}`; expected one of 1/0, true/false, yes/no, on/off"
            ),
        },
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => panic!("hydrolysis runner: invalid {name} environment value: {error}"),
    }
}

fn parse_positive_u64_env(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(raw) => {
            let parsed = raw.trim().parse::<u64>().unwrap_or_else(|error| {
                panic!("hydrolysis runner: invalid {name} `{raw}`: {error}")
            });
            if parsed == 0 {
                panic!("hydrolysis runner: {name} must be > 0");
            }
            parsed
        }
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => panic!("hydrolysis runner: invalid {name} environment value: {error}"),
    }
}

struct RenderPhaseSample {
    rebuild: Duration,
    acquire: Duration,
    render: Duration,
    present: Duration,
    total: Duration,
    rebuild_iterations: u32,
    rebuilt: bool,
}

#[derive(Default)]
struct RenderPhaseTotals {
    frames: u64,
    rebuild_frames: u64,
    rebuild_iterations: u64,
    slow_frames: u64,
    rebuild: Duration,
    acquire: Duration,
    render: Duration,
    present: Duration,
    total: Duration,
}

struct RenderDiagnostics {
    config: RenderDiagnosticsConfig,
    report_started_at: Instant,
    totals: RenderPhaseTotals,
}

impl RenderDiagnostics {
    fn new(config: RenderDiagnosticsConfig) -> Self {
        Self {
            config,
            report_started_at: Instant::now(),
            totals: RenderPhaseTotals::default(),
        }
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    fn record_frame(&mut self, window_title: &str, sample: RenderPhaseSample) {
        if !self.config.enabled {
            return;
        }

        self.totals.frames = self
            .totals
            .frames
            .checked_add(1)
            .expect("hydrolysis runner: render diagnostics frame counter overflow");
        if sample.rebuilt {
            self.totals.rebuild_frames = self
                .totals
                .rebuild_frames
                .checked_add(1)
                .expect("hydrolysis runner: render diagnostics rebuild frame counter overflow");
        }
        self.totals.rebuild_iterations = self
            .totals
            .rebuild_iterations
            .checked_add(u64::from(sample.rebuild_iterations))
            .expect("hydrolysis runner: render diagnostics rebuild iteration counter overflow");
        self.totals.rebuild += sample.rebuild;
        self.totals.acquire += sample.acquire;
        self.totals.render += sample.render;
        self.totals.present += sample.present;
        self.totals.total += sample.total;

        if sample.total >= self.config.slow_frame_threshold {
            self.totals.slow_frames = self
                .totals
                .slow_frames
                .checked_add(1)
                .expect("hydrolysis runner: render diagnostics slow frame counter overflow");
            tracing::warn!(
                target: "waterui::hydrolysis::render",
                window_title = %window_title,
                total_ms = duration_ms(sample.total),
                rebuild_ms = duration_ms(sample.rebuild),
                acquire_ms = duration_ms(sample.acquire),
                render_ms = duration_ms(sample.render),
                present_ms = duration_ms(sample.present),
                rebuild_iterations = sample.rebuild_iterations,
                "Hydrolysis slow frame detected"
            );
        }

        self.maybe_report(window_title);
    }

    fn maybe_report(&mut self, window_title: &str) {
        if self.totals.frames == 0 {
            return;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.report_started_at);
        if elapsed < self.config.interval {
            return;
        }

        let frame_count = self.totals.frames as f64;
        let avg_total_ms = duration_ms(self.totals.total) / frame_count;
        let avg_rebuild_ms = duration_ms(self.totals.rebuild) / frame_count;
        let avg_acquire_ms = duration_ms(self.totals.acquire) / frame_count;
        let avg_render_ms = duration_ms(self.totals.render) / frame_count;
        let avg_present_ms = duration_ms(self.totals.present) / frame_count;
        let rebuild_ratio = self.totals.rebuild_frames as f64 / frame_count;
        let avg_rebuild_iterations = self.totals.rebuild_iterations as f64 / frame_count;
        let fps = self.totals.frames as f64 / elapsed.as_secs_f64();

        tracing::info!(
            target: "waterui::hydrolysis::render",
            window_title = %window_title,
            frames = self.totals.frames,
            interval_ms = duration_ms(elapsed),
            fps,
            rebuild_frames = self.totals.rebuild_frames,
            rebuild_ratio,
            avg_rebuild_iterations,
            avg_total_ms,
            avg_rebuild_ms,
            avg_acquire_ms,
            avg_render_ms,
            avg_present_ms,
            slow_frames = self.totals.slow_frames,
            slow_frame_threshold_ms = duration_ms(self.config.slow_frame_threshold),
            "Hydrolysis render diagnostics"
        );

        self.report_started_at = now;
        self.totals = RenderPhaseTotals::default();
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn elapsed_or_zero(started_at: Option<Instant>) -> Duration {
    started_at.map_or(Duration::ZERO, |value| value.elapsed())
}

struct RuntimeWindow<P: PlatformWindow> {
    window: Window,
    platform: P,
    renderer: HydrolysisRenderer,
    needs_rebuild: bool,
    pointer_position: Option<(f32, f32)>,
    render_diagnostics: RenderDiagnostics,
}

impl<P: PlatformWindow> RuntimeWindow<P> {
    fn new(
        window: Window,
        platform: P,
        renderer: HydrolysisRenderer,
        render_diagnostics_config: RenderDiagnosticsConfig,
    ) -> Self {
        Self {
            window,
            platform,
            renderer,
            needs_rebuild: true,
            pointer_position: None,
            render_diagnostics: RenderDiagnostics::new(render_diagnostics_config),
        }
    }
}

fn create_bounds(width: u32, height: u32, scale_factor: f64) -> vello::kurbo::Rect {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        panic!("hydrolysis runner: invalid scale factor {scale_factor}");
    }
    vello::kurbo::Rect::new(
        0.0,
        0.0,
        f64::from(width) / scale_factor,
        f64::from(height) / scale_factor,
    )
}

fn window_clear_color(window: &Window, env: &Environment) -> vello::peniko::Color {
    match &window.background {
        WindowBackground::Opaque => vello::peniko::Color::WHITE,
        WindowBackground::Color(color) => {
            let resolved = color.resolve(env).get();
            let srgb = resolved.to_srgb_with_headroom();
            vello::peniko::Color::new([srgb.red, srgb.green, srgb.blue, resolved.opacity])
        }
    }
}

fn render_window<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>, env: &Environment) {
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(feature = "winit")]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.get().as_str());
    {
        let diagnostics_enabled = runtime.render_diagnostics.enabled();
        let frame_started_at = diagnostics_enabled.then(Instant::now);
        let scale_factor = runtime.platform.scale_factor();
        let surface = runtime.platform.surface();
        let (width, height) = surface.size();
        let format = surface.format();
        let bounds = create_bounds(width, height, scale_factor);
        let root_transform = vello::kurbo::Affine::scale(scale_factor);
        runtime
            .renderer
            .set_frame_resources(surface.device(), surface.queue());

        let rebuild_started_at = diagnostics_enabled.then(Instant::now);
        if runtime.renderer.advance_animations() {
            runtime.needs_rebuild = true;
        }

        let mut rebuild_iterations = 0u32;
        loop {
            let should_rebuild = runtime.needs_rebuild || runtime.renderer.take_rebuild_request();
            if !should_rebuild {
                break;
            }
            rebuild_iterations = rebuild_iterations
                .checked_add(1)
                .expect("hydrolysis runner: rebuild iteration counter overflow");
            runtime.renderer.reset_scene();
            runtime.renderer.begin_rebuild_frame();
            let content = runtime.window.build_content();
            runtime
                .renderer
                .dispatch_with_transform(content, env, bounds, root_transform);
            runtime.renderer.finish_rebuild_frame();
            runtime.needs_rebuild = false;
            if let Some((x, y)) = runtime.pointer_position {
                if runtime.renderer.handle_pointer_move(x, y, env) {
                    runtime.needs_rebuild = true;
                }
            }
        }
        let rebuild_duration = elapsed_or_zero(rebuild_started_at);

        let clear_color = window_clear_color(&runtime.window, env);
        let acquire_started_at = diagnostics_enabled.then(Instant::now);
        let frame = surface
            .acquire()
            .expect("hydrolysis runner: failed to acquire frame");
        let acquire_duration = elapsed_or_zero(acquire_started_at);
        let render_started_at = diagnostics_enabled.then(Instant::now);
        runtime.renderer.render_scene_to_surface(
            surface.device(),
            surface.queue(),
            frame.view(),
            format,
            width,
            height,
            clear_color,
        );
        runtime.renderer.clear_frame_resources();
        let render_duration = elapsed_or_zero(render_started_at);
        let present_started_at = diagnostics_enabled.then(Instant::now);
        surface.present(frame);
        let present_duration = elapsed_or_zero(present_started_at);

        if diagnostics_enabled {
            let window_title = runtime.window.title.get();
            runtime.render_diagnostics.record_frame(
                window_title.as_str(),
                RenderPhaseSample {
                    rebuild: rebuild_duration,
                    acquire: acquire_duration,
                    render: render_duration,
                    present: present_duration,
                    total: elapsed_or_zero(frame_started_at),
                    rebuild_iterations,
                    rebuilt: rebuild_iterations > 0,
                },
            );
        }
    }

    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
    if let Some((x, y)) = runtime.pointer_position {
        runtime
            .platform
            .set_cursor_style(runtime.renderer.cursor_style_at(x, y));
    }
}

#[cfg(not(feature = "winit"))]
pub fn run(app: App) {
    init_main_thread_executors();
    let (windows, env) = app.into_parts();
    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    install_native_component_hooks(&mut env);
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::default(),
    ));
    for window in windows {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        let mut runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
        render_window(&mut runtime, &env);
    }
}

#[cfg(feature = "winit")]
pub fn run(app: App) {
    init_main_thread_executors();
    winit_runner::run(app);
}

#[cfg(feature = "winit")]
mod winit_runner {
    use std::collections::HashMap;
    use std::future::Future;
    use std::mem;
    use std::sync::{Arc, mpsc};
    use std::time::Instant;

    use accesskit_winit::{
        Adapter as AccessKitAdapter, Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent,
    };
    use executor_core::{
        LocalExecutor,
        async_task::{self, AsyncTask, Runnable},
        try_init_local_executor,
    };
    use nami::Signal;
    use waterui::app::App;
    use waterui::window::{Window, WindowState};
    use waterui_core::Environment;
    use waterui_core::layout::Size;
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{Window as NativeWindow, WindowId};

    use crate::platform::{InputEvent, KeyState, PlatformWindow, WinitWindow};
    use crate::renderer::HydrolysisRenderer;
    use crate::runner::{RenderDiagnosticsConfig, RuntimeWindow, render_window};

    #[derive(Debug)]
    enum RunnerEvent {
        PollLocalTasks,
        AccessKit(AccessKitEvent),
    }

    impl From<AccessKitEvent> for RunnerEvent {
        fn from(value: AccessKitEvent) -> Self {
            Self::AccessKit(value)
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
            let (runnable, task) = async_task::spawn_local(fut, move |runnable: Runnable| {
                if runnable_tx.send(runnable).is_err() {
                    return;
                }
                let _ = event_proxy.send_event(RunnerEvent::PollLocalTasks);
            });
            runnable.schedule();
            task
        }
    }

    pub fn run(app: App) {
        let event_loop = EventLoop::<RunnerEvent>::with_user_event()
            .build()
            .expect("hydrolysis runner: failed to create event loop");
        let event_proxy = event_loop.create_proxy();
        let (local_runnable_tx, local_runnable_rx) = mpsc::channel::<Runnable>();
        let local_executor = WinitMainThreadExecutor {
            runnable_tx: local_runnable_tx,
            event_proxy: event_proxy.clone(),
        };
        let _ = try_init_local_executor(waterui::task::monitored_local_executor(local_executor));

        let (windows, env) = app.into_parts();
        let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
        let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
        super::install_native_component_hooks(&mut env);
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::default(),
        ));
        let mut runner = WinitRunner {
            env,
            pending_windows: windows,
            windows: HashMap::new(),
            accesskit_adapters: HashMap::new(),
            local_runnable_rx,
            event_proxy,
            render_diagnostics_config,
        };

        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop
            .run_app(&mut runner)
            .expect("hydrolysis runner: event loop failed");
    }

    struct WinitRunner {
        env: Environment,
        pending_windows: Vec<Window>,
        windows: HashMap<WindowId, RuntimeWindow<WinitWindow>>,
        accesskit_adapters: HashMap<WindowId, AccessKitAdapter>,
        local_runnable_rx: mpsc::Receiver<Runnable>,
        event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
        render_diagnostics_config: RenderDiagnosticsConfig,
    }

    impl WinitRunner {
        fn drain_local_executor_queue(&self) {
            while let Ok(runnable) = self.local_runnable_rx.try_recv() {
                runnable.run();
            }
        }

        fn physical_to_logical_dimension(value: u32, scale_factor: f64) -> f32 {
            if !scale_factor.is_finite() || scale_factor <= 0.0 {
                panic!("hydrolysis runner: invalid scale factor {scale_factor}");
            }
            (f64::from(value) / scale_factor) as f32
        }

        fn create_runtime_window(
            &mut self,
            event_loop: &ActiveEventLoop,
            window: Window,
        ) -> (RuntimeWindow<WinitWindow>, AccessKitAdapter) {
            let frame = window.frame.get();
            let attributes = NativeWindow::default_attributes()
                .with_title(window.title.get().as_str())
                .with_resizable(window.resizable)
                .with_visible(false)
                .with_inner_size(winit::dpi::LogicalSize::new(
                    frame.width() as f64,
                    frame.height() as f64,
                ));

            let native_window = Arc::new(
                event_loop
                    .create_window(attributes)
                    .expect("hydrolysis runner: failed to create winit window"),
            );
            let mut platform = pollster::block_on(WinitWindow::new(native_window));
            platform.apply_properties(&window);
            let renderer = {
                let surface = platform.surface();
                HydrolysisRenderer::new(surface.device())
            };
            let adapter = AccessKitAdapter::with_event_loop_proxy(
                event_loop,
                platform.native_window(),
                self.event_proxy.clone(),
            );
            tracing::trace!(
                target: "waterui::hydrolysis::a11y",
                window_id = ?platform.id(),
                title = window.title.get().as_str(),
                "created accesskit adapter for window"
            );
            platform.native_window().set_visible(true);
            (
                RuntimeWindow::new(window, platform, renderer, self.render_diagnostics_config),
                adapter,
            )
        }

        fn mount_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
            let pending = mem::take(&mut self.pending_windows);
            for window in pending {
                let (runtime, adapter) = self.create_runtime_window(event_loop, window);
                let id = runtime.platform.id();
                self.windows.insert(id, runtime);
                self.accesskit_adapters.insert(id, adapter);
            }
        }

        fn handle_input_events(
            runtime: &mut RuntimeWindow<WinitWindow>,
            env: &Environment,
        ) -> bool {
            let mut should_close = runtime.window.state.get() == WindowState::Closed;
            for event in runtime.platform.drain_events() {
                match event {
                    InputEvent::CloseRequested => {
                        runtime.window.state.set(WindowState::Closed);
                        should_close = true;
                    }
                    InputEvent::Resize { width, height } => {
                        let frame = runtime.window.frame.get();
                        let logical_width = Self::physical_to_logical_dimension(
                            width,
                            runtime.platform.scale_factor(),
                        );
                        let logical_height = Self::physical_to_logical_dimension(
                            height,
                            runtime.platform.scale_factor(),
                        );
                        let frame = waterui_core::layout::Rect::new(
                            frame.origin(),
                            Size::new(logical_width, logical_height),
                        );
                        runtime.window.frame.set(frame);
                        runtime.needs_rebuild = true;
                    }
                    InputEvent::PointerDown { x, y, button } => {
                        runtime.pointer_position = Some((x, y));
                        let changed = runtime.renderer.handle_pointer_down(x, y, button, env);
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "pointer_down",
                            x,
                            y,
                            button = ?button,
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::PointerUp { x, y, button } => {
                        runtime.pointer_position = Some((x, y));
                        let changed = runtime.renderer.handle_pointer_up(x, y, button, env);
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "pointer_up",
                            x,
                            y,
                            button = ?button,
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::PointerMove { x, y } => {
                        runtime.pointer_position = Some((x, y));
                        let changed = runtime.renderer.handle_pointer_move(x, y, env);
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "pointer_move",
                            x,
                            y,
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::PointerCancel => {
                        let changed = runtime.renderer.handle_pointer_cancel(env);
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "pointer_cancel",
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::Scroll {
                        x,
                        y,
                        dx,
                        dy,
                        is_line_delta,
                    } => {
                        runtime.pointer_position = Some((x, y));
                        let changed = runtime.renderer.handle_scroll(x, y, dx, dy, is_line_delta);
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "scroll",
                            x,
                            y,
                            dx,
                            dy,
                            is_line_delta,
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::Magnification { x, y, delta, phase } => {
                        runtime.pointer_position = Some((x, y));
                        if runtime
                            .renderer
                            .handle_magnification(x, y, delta, phase, env)
                        {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::Rotation { x, y, delta, phase } => {
                        runtime.pointer_position = Some((x, y));
                        if runtime.renderer.handle_rotation(x, y, delta, phase, env) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::Key {
                        key,
                        state: KeyState::Pressed,
                        modifiers,
                    } => {
                        let changed = runtime.renderer.handle_key(&key, modifiers);
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "key_pressed",
                            key = ?key,
                            modifiers = ?modifiers,
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::ImePreedit { text } => {
                        let changed = runtime.renderer.handle_ime_preedit(text.as_str());
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "ime_preedit",
                            text = text.as_str(),
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::ImeCommit { text } => {
                        let changed = runtime.renderer.handle_ime_commit(text.as_str());
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "ime_commit",
                            text = text.as_str(),
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::ImeDisabled => {
                        let changed = runtime.renderer.handle_ime_disabled();
                        tracing::trace!(
                            target: "waterui::hydrolysis::input",
                            event = "ime_disabled",
                            changed,
                            "runner dispatched input event"
                        );
                        if changed {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::Key {
                        state: KeyState::Released,
                        ..
                    } => {}
                }
            }
            runtime
                .platform
                .sync_text_input_state(runtime.renderer.focused_text_input_state());
            if let Some((x, y)) = runtime.pointer_position {
                runtime
                    .platform
                    .set_cursor_style(runtime.renderer.cursor_style_at(x, y));
            }
            should_close
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
            }

            if self.windows.is_empty() && self.pending_windows.is_empty() {
                event_loop.exit();
            }
        }
    }

    impl ApplicationHandler<RunnerEvent> for WinitRunner {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.drain_local_executor_queue();
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
            self.drain_local_executor_queue();
            let Some(runtime) = self.windows.get_mut(&window_id) else {
                return;
            };
            let adapter = self
                .accesskit_adapters
                .get_mut(&window_id)
                .expect("hydrolysis runner missing AccessKit adapter for window");
            adapter.process_event(runtime.platform.native_window(), &event);
            runtime.platform.handle_window_event(&event);
            let should_close = Self::handle_input_events(runtime, &self.env);

            if should_close {
                self.windows.remove(&window_id);
                self.accesskit_adapters.remove(&window_id);
                if self.windows.is_empty() && self.pending_windows.is_empty() {
                    event_loop.exit();
                }
                return;
            }

            if let WindowEvent::RedrawRequested = event {
                render_window(runtime, &self.env);
                if let Some(update) = runtime.renderer.take_accessibility_tree_update() {
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

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            self.drain_local_executor_queue();
            self.mount_pending_windows(event_loop);
            let now = Instant::now();
            let mut next_gesture_deadline: Option<Instant> = None;
            for runtime in self.windows.values_mut() {
                runtime
                    .platform
                    .sync_text_input_state(runtime.renderer.focused_text_input_state());
                if runtime.renderer.handle_gesture_tick(now, &self.env) {
                    runtime.needs_rebuild = true;
                }
                if runtime.renderer.advance_animations() {
                    runtime.needs_rebuild = true;
                }
                if runtime.renderer.take_rebuild_request() {
                    runtime.needs_rebuild = true;
                }
                if let Some(deadline) = runtime.renderer.next_gesture_deadline() {
                    next_gesture_deadline = Some(match next_gesture_deadline {
                        Some(existing) => existing.min(deadline),
                        None => deadline,
                    });
                }
                if runtime.needs_rebuild {
                    runtime.platform.request_redraw();
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
                RunnerEvent::PollLocalTasks => self.drain_local_executor_queue(),
                RunnerEvent::AccessKit(event) => {
                    let Some(runtime) = self.windows.get_mut(&event.window_id) else {
                        return;
                    };
                    let adapter = self
                        .accesskit_adapters
                        .get_mut(&event.window_id)
                        .expect("hydrolysis runner missing AccessKit adapter for user event");
                    match event.window_event {
                        AccessKitWindowEvent::InitialTreeRequested => {
                            tracing::trace!(
                                target: "waterui::hydrolysis::a11y",
                                window_id = ?event.window_id,
                                "accesskit initial tree requested"
                            );
                            if let Some(update) = runtime.renderer.take_accessibility_tree_update()
                            {
                                tracing::trace!(
                                    target: "waterui::hydrolysis::a11y",
                                    window_id = ?event.window_id,
                                    "publishing accessibility tree update for initial request"
                                );
                                adapter.update_if_active(|| update);
                            } else {
                                tracing::trace!(
                                    target: "waterui::hydrolysis::a11y",
                                    window_id = ?event.window_id,
                                    "missing accessibility tree update for initial request, scheduling rebuild"
                                );
                                runtime.needs_rebuild = true;
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
                            if runtime
                                .renderer
                                .handle_accessibility_action(request, &self.env)
                            {
                                runtime.needs_rebuild = true;
                                runtime.platform.request_redraw();
                            }
                        }
                        AccessKitWindowEvent::AccessibilityDeactivated => {}
                    }
                }
            }
        }
    }
}
