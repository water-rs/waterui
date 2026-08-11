//! Pump-based headless runtime for tests, snapshots and offscreen rendering.

use super::*;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub(super) struct HeadlessPlatformWindow {
    inner: OffscreenWindow,
    pending_events: VecDeque<InputEvent>,
    redraw_requested: Cell<bool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessPlatformWindow {
    pub(super) fn new(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            inner: OffscreenWindow::new(width, height, format),
            pending_events: VecDeque::new(),
            redraw_requested: Cell::new(false),
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub(super) fn new_for_tests(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            inner: OffscreenWindow::new_for_tests(width, height, format),
            pending_events: VecDeque::new(),
            redraw_requested: Cell::new(false),
        }
    }

    pub(super) fn set_scale_factor(&mut self, scale_factor: f64) {
        self.inner.set_scale_factor(scale_factor);
    }

    pub(super) fn push_event(&mut self, event: InputEvent) {
        self.pending_events.push_back(event);
    }

    pub(super) fn take_redraw_request(&self) -> bool {
        self.redraw_requested.replace(false)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PlatformWindow for HeadlessPlatformWindow {
    fn surface(&mut self) -> &mut dyn crate::platform::SurfaceProvider {
        self.inner.surface()
    }

    fn apply_properties(&mut self, window: &Window) {
        self.inner.apply_properties(window);
    }

    fn set_size_limits(
        &mut self,
        min: Option<waterui_core::layout::Size>,
        max: Option<waterui_core::layout::Size>,
    ) {
        self.inner.set_size_limits(min, max);
    }

    fn applies_size_limits(&self) -> bool {
        self.inner.applies_size_limits()
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        self.pending_events.drain(..).collect()
    }

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    fn scale_factor(&self) -> f64 {
        self.inner.scale_factor()
    }

    fn sync_text_input_state(&mut self, state: Option<crate::platform::TextInputState>) {
        self.inner.sync_text_input_state(state);
    }

    fn set_cursor_style(&mut self, style: waterui::cursor::CursorStyle) {
        self.inner.set_cursor_style(style);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
impl HeadlessPlatformWindow {
    /// The last (min, max) content-size limits the runner applied, for tests.
    pub(super) fn applied_size_limits(
        &self,
    ) -> Option<(
        Option<waterui_core::layout::Size>,
        Option<waterui_core::layout::Size>,
    )> {
        self.inner.applied_size_limits()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub(crate) struct HeadlessMainThreadExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    runnable_rx: Rc<mpsc::Receiver<Runnable>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for HeadlessMainThreadExecutor {
    fn default() -> Self {
        let (runnable_tx, runnable_rx) = mpsc::channel();
        Self {
            runnable_tx,
            runnable_rx: Rc::new(runnable_rx),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessMainThreadExecutor {
    /// Runs every runnable currently queued, returning whether any ran.
    ///
    /// The offscreen runner must call this while rendering: a `GpuView`'s
    /// `setup` is an async future spawned onto this executor, so a frame
    /// rendered without draining would run `render` against a renderer that has
    /// not built its pipelines yet and would emit nothing.
    pub(super) fn drain(&self) -> bool {
        let mut ran = false;
        loop {
            let Ok(runnable) = self.runnable_rx.try_recv() else {
                return ran;
            };
            ran = true;
            runnable.run();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl LocalExecutor for HeadlessMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            if let Err(unsent) = runnable_tx.send(runnable) {
                // Teardown race: a waker held by another thread (decoder,
                // audio, dispatch callback) fired after the runtime dropped
                // the receiver. The task can never run again, and dropping a
                // `spawn_local` runnable off its spawning thread panics by
                // design (async-task's thread check), so leak it instead —
                // bounded to shutdown, reclaimed at process exit.
                std::mem::forget(unsent);
            }
        });
        runnable.schedule();
        task
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct HeadlessPumpResult {
    pub rebuilt: bool,
    pub profile: FrameProfile,
    #[cfg(feature = "accessibility")]
    pub tree_update: Option<AccessibilityTreeUpdate>,
    pub snapshot: Option<HeadlessSnapshot>,
    #[cfg(feature = "accessibility")]
    pub ui_focus: Option<accesskit::NodeId>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct HeadlessRuntime {
    env: Environment,
    runtime: RuntimeWindow<HeadlessPlatformWindow>,
    pending_window_queue: Rc<RefCell<Vec<Window>>>,
    popup_windows: Vec<RuntimeWindow<HeadlessPlatformWindow>>,
    create_platform: fn(u32, u32, wgpu::TextureFormat) -> HeadlessPlatformWindow,
    local_executor: HeadlessMainThreadExecutor,
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessRuntime {
    #[must_use]
    pub fn new(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::new_with_platform_window(env, content, width, height, HeadlessPlatformWindow::new)
    }

    /// Renders at `scale_factor` physical pixels per logical pixel.
    ///
    /// The layout is unchanged — it stays in logical units — so this only makes
    /// the captured image sharper. A preview meant to be viewed on a HiDPI
    /// display should raise this above 1.
    #[must_use]
    pub fn with_scale_factor(mut self, scale_factor: f64) -> Self {
        self.runtime.platform.set_scale_factor(scale_factor);
        self
    }

    /// Creates a headless runtime for WaterUI test hosts.
    ///
    /// This constructor allows compute-capable software adapters for CI-only
    /// semantic testing while keeping [`Self::new`] on production adapter
    /// selection.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::new_with_platform_window(
            env,
            content,
            width,
            height,
            HeadlessPlatformWindow::new_for_tests,
        )
    }

    fn new_with_platform_window(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
        create_platform: fn(u32, u32, wgpu::TextureFormat) -> HeadlessPlatformWindow,
    ) -> Self {
        let inspector = init_main_thread_executors();
        let inspector_probe = inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe);
        let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
        waterui::inspector::install(&mut env, inspector);
        let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
        install_native_component_hooks(&mut env);
        install_headless_window_managers(&mut env, Rc::clone(&pending_window_queue));
        env.insert(HydrolysisTextContextMenuMode::Overlay);
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::default(),
        ));

        // Headless binaries (preview, tests) have no platform runner to install
        // a tracing subscriber; honor `RUST_LOG` here so they stay debuggable.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init();
        let local_executor = HeadlessMainThreadExecutor::default();
        let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
            local_executor.clone(),
            inspector_probe,
        ));

        // The headless window uses the same default background as platform
        // windows (the theme `Background` slot), so offscreen captures match
        // what `water run` renders from the very first frame.
        let content_builder = content.clone();
        let window = Window::new(
            "",
            waterui_core::binding(waterui::window::WindowState::Normal),
            move || content_builder.build(),
        );
        window.frame.set(waterui_core::layout::Rect::new(
            waterui_core::layout::Point::zero(),
            waterui_core::layout::Size::new(width.max(1) as f32, height.max(1) as f32),
        ));

        let mut platform =
            create_platform(width.max(1), height.max(1), wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_native_resource_fonts(&mut renderer);

        Self {
            env,
            runtime: RuntimeWindow::new(
                window,
                platform,
                renderer,
                RenderDiagnosticsConfig {
                    enabled: false,
                    interval: Duration::from_secs(1),
                    slow_frame_threshold_override: None,
                },
            ),
            pending_window_queue,
            popup_windows: Vec::new(),
            create_platform,
            local_executor,
        }
    }

    fn create_popup_runtime(&self, window: Window) -> RuntimeWindow<HeadlessPlatformWindow> {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = (self.create_platform)(width, height, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_native_resource_fonts(&mut renderer);
        RuntimeWindow::new(
            window,
            platform,
            renderer,
            RenderDiagnosticsConfig {
                enabled: false,
                interval: Duration::from_secs(1),
                slow_frame_threshold_override: None,
            },
        )
    }

    fn mount_pending_popup_windows(&mut self) {
        let pending = self
            .pending_window_queue
            .borrow_mut()
            .drain(..)
            .collect::<Vec<_>>();
        for window in pending {
            self.popup_windows.push(self.create_popup_runtime(window));
        }
    }

    pub fn push_input_event(&mut self, event: InputEvent) {
        self.runtime.platform.push_event(event);
    }

    pub fn request_redraw(&mut self) {
        self.runtime.platform.request_redraw();
    }

    #[cfg(feature = "accessibility")]
    pub fn perform_accessibility_action(&mut self, request: AccessibilityActionRequest) -> bool {
        let action_env = self.env.extending(runtime_window_origin(&self.runtime));
        let changed = self
            .runtime
            .renderer
            .handle_accessibility_action(request, &action_env);
        if changed {
            self.runtime.request_refresh();
            self.runtime.platform.request_redraw();
        }
        changed
    }

    #[cfg(feature = "accessibility")]
    pub fn clear_ui_focus(&mut self) -> bool {
        let changed = self.runtime.renderer.clear_ui_focus();
        if changed {
            self.runtime.request_refresh();
            self.runtime.platform.request_redraw();
        }
        changed
    }

    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn focused_ui_node(&self) -> Option<accesskit::NodeId> {
        self.runtime.renderer.focused_ui_node()
    }

    pub fn pump(&mut self, capture_snapshot: bool) -> HeadlessPumpResult {
        self.pump_at(capture_snapshot, Instant::now())
    }

    pub fn pump_semantic(&mut self) -> HeadlessPumpResult {
        self.pump_semantic_at(Instant::now())
    }

    pub fn pump_offscreen(&mut self) -> HeadlessPumpResult {
        self.pump_at(false, Instant::now())
    }

    pub fn pump_snapshot(&mut self) -> HeadlessPumpResult {
        self.pump_at(true, Instant::now())
    }

    pub fn pump_semantic_at(&mut self, at: Instant) -> HeadlessPumpResult {
        let frame_started_at = Instant::now();
        self.runtime.renderer.set_frame_instant(at);
        let executor_before_started_at = Instant::now();
        let drained_before = self.local_executor.drain();
        let executor_before = executor_before_started_at.elapsed();
        let input_started_at = Instant::now();
        let _ = handle_input_events(&mut self.runtime, &self.env);
        let input = input_started_at.elapsed();
        let animation_started_at = Instant::now();
        let _ = advance_runtime(&mut self.runtime, &self.env, at);
        let animation = animation_started_at.elapsed();
        self.mount_pending_popup_windows();
        for popup in &mut self.popup_windows {
            popup.renderer.set_frame_instant(at);
            let _ = handle_input_events(popup, &self.env);
            let _ = advance_runtime(popup, &self.env, at);
        }
        let rebuilt = pump_window_semantics(&mut self.runtime, &self.env);
        for popup in &mut self.popup_windows {
            let _ = pump_window_semantics(popup, &self.env);
        }
        let executor_after_started_at = Instant::now();
        let drained_after = self.local_executor.drain();
        let executor_after = executor_after_started_at.elapsed();

        HeadlessPumpResult {
            rebuilt: rebuilt || drained_before || drained_after,
            profile: FrameProfile {
                phases: FramePhases {
                    executor_before,
                    input,
                    animation,
                    executor_after,
                    ..FramePhases::default()
                },
                ..FrameProfile::default()
            }
            .with_total(frame_started_at.elapsed()),
            #[cfg(feature = "accessibility")]
            tree_update: self.runtime.renderer.take_accessibility_tree_update(),
            snapshot: None,
            #[cfg(feature = "accessibility")]
            ui_focus: self.runtime.renderer.focused_ui_node(),
        }
    }

    pub fn pump_at(&mut self, capture_snapshot: bool, at: Instant) -> HeadlessPumpResult {
        let frame_started_at = Instant::now();
        self.runtime.renderer.set_frame_instant(at);
        let executor_before_started_at = Instant::now();
        let drained_before = self.local_executor.drain();
        let executor_before = executor_before_started_at.elapsed();
        let input_started_at = Instant::now();
        let _ = handle_input_events(&mut self.runtime, &self.env);
        let input = input_started_at.elapsed();
        let animation_started_at = Instant::now();
        let _ = advance_runtime(&mut self.runtime, &self.env, at);
        let animation = animation_started_at.elapsed();
        self.mount_pending_popup_windows();
        for popup in &mut self.popup_windows {
            popup.renderer.set_frame_instant(at);
            let _ = handle_input_events(popup, &self.env);
            let _ = advance_runtime(popup, &self.env, at);
        }
        let should_render = capture_snapshot
            || self.runtime.mode.is_pending()
            || self.runtime.platform.take_redraw_request();
        let mut render_result = should_render.then(|| {
            render_window_with_capture(&mut self.runtime, &self.env, capture_snapshot, &mut || {
                self.local_executor.drain()
            })
        });
        if capture_snapshot
            && !self.popup_windows.is_empty()
            && let Some(snapshot) = render_result
                .as_mut()
                .and_then(|result| result.snapshot.as_mut())
        {
            for popup in &mut self.popup_windows {
                let Some(popup_snapshot) =
                    render_window_with_capture(popup, &self.env, true, &mut || {
                        self.local_executor.drain()
                    })
                    .snapshot
                else {
                    continue;
                };
                composite_popup_snapshot(snapshot, &popup_snapshot, popup.window.frame.get());
            }
        }
        let executor_after_started_at = Instant::now();
        let drained_after = self.local_executor.drain();
        let executor_after = executor_after_started_at.elapsed();

        let mut profile = render_result
            .as_ref()
            .map_or_else(FrameProfile::default, |result| result.profile);
        profile.phases.executor_before = executor_before;
        profile.phases.input = input;
        profile.phases.animation = animation;
        profile.phases.executor_after = executor_after;

        HeadlessPumpResult {
            rebuilt: render_result.as_ref().is_some_and(|result| result.rebuilt)
                || drained_before
                || drained_after,
            profile: profile.with_total(frame_started_at.elapsed()),
            #[cfg(feature = "accessibility")]
            tree_update: self.runtime.renderer.take_accessibility_tree_update(),
            snapshot: render_result.and_then(|result| result.snapshot),
            #[cfg(feature = "accessibility")]
            ui_focus: self.runtime.renderer.focused_ui_node(),
        }
    }
}

fn composite_popup_snapshot(
    target: &mut HeadlessSnapshot,
    source: &HeadlessSnapshot,
    frame: waterui_core::layout::Rect,
) {
    let offset_x = frame.x().round() as i32;
    let offset_y = frame.y().round() as i32;
    for source_y in 0..source.height {
        let target_y = offset_y + i32::try_from(source_y).expect("source y should fit i32");
        if target_y < 0 || target_y >= i32::try_from(target.height).expect("height should fit i32")
        {
            continue;
        }
        for source_x in 0..source.width {
            let target_x = offset_x + i32::try_from(source_x).expect("source x should fit i32");
            if target_x < 0
                || target_x >= i32::try_from(target.width).expect("width should fit i32")
            {
                continue;
            }
            let source_index = ((source_y * source.width + source_x) * 4) as usize;
            let target_index = ((u32::try_from(target_y).expect("target y should be non-negative")
                * target.width
                + u32::try_from(target_x).expect("target x should be non-negative"))
                * 4) as usize;
            composite_pixel(
                &mut target.rgba8[target_index..target_index + 4],
                &source.rgba8[source_index..source_index + 4],
            );
        }
    }
}

fn composite_pixel(target: &mut [u8], source: &[u8]) {
    let source_alpha = f32::from(source[3]) / 255.0;
    if source_alpha <= 0.0 {
        return;
    }
    let target_alpha = f32::from(target[3]) / 255.0;
    let out_alpha = source_alpha + target_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let source_channel = f32::from(source[channel]) / 255.0;
        let target_channel = f32::from(target[channel]) / 255.0;
        let out = (source_channel * source_alpha
            + target_channel * target_alpha * (1.0 - source_alpha))
            / out_alpha;
        target[channel] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    target[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}
