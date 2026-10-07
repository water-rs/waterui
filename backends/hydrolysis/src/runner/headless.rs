//! Pump-based headless runtime for tests, snapshots and offscreen rendering.

use super::*;
#[cfg(feature = "frame-profile")]
use crate::platform::SurfaceProvider as _;
use crate::renderer::MenuShortcutRegistry;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility::AccessibilityActivationPointError;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub(super) struct HeadlessPlatformWindow {
    inner: OffscreenWindow,
    pending_events: VecDeque<InputEvent>,
    redraw_requested: Cell<bool>,
    /// The occlusion report tests drive through [`Self::set_occluded`] —
    /// the default `false` a headless window behaves with in production.
    occluded: Cell<bool>,
    /// The pointer position the host knows right now — tracked from the
    /// positional events it dispatched, or set directly when the test's host
    /// knows a position it delivered no event for (an OS drag suppresses
    /// cursor events on some platforms).
    pointer_position: Option<(f32, f32)>,
    /// The touch-gesture parameters this host publishes — a test sets them
    /// through [`HeadlessRuntime::set_touch_scroll_config`], like a real
    /// touch platform pushing its `ViewConfiguration` values.
    touch_scroll_config: Cell<Option<crate::platform::TouchScrollConfig>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessPlatformWindow {
    #[cfg(test)]
    pub(super) fn new_for_tests(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self::on_context(
            OffscreenGpuContext::new_for_tests_blocking(),
            width,
            height,
            format,
        )
    }

    pub(super) fn on_context(
        gpu: OffscreenGpuContext,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            inner: OffscreenWindow::on_context(gpu, width, height, format),
            pending_events: VecDeque::new(),
            redraw_requested: Cell::new(false),
            occluded: Cell::new(false),
            pointer_position: None,
            touch_scroll_config: Cell::new(None),
        }
    }

    /// The touch-gesture parameters [`PlatformWindow::touch_scroll_config`]
    /// reports — `None` until a test supplies the platform's values.
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn set_touch_scroll_config(&self, config: crate::platform::TouchScrollConfig) {
        self.touch_scroll_config.set(Some(config));
    }

    pub(super) fn set_scale_factor(&mut self, scale_factor: f64) {
        self.inner.set_scale_factor(scale_factor);
    }

    pub(super) fn push_event(&mut self, event: InputEvent) {
        if let Some((x, y)) = event_position(&event) {
            self.pointer_position = Some((x, y));
        }
        self.pending_events.push_back(event);
    }

    pub(super) fn has_pending_events(&self) -> bool {
        !self.pending_events.is_empty()
    }

    pub(super) const fn take_redraw_request(&self) -> bool {
        self.redraw_requested.replace(false)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PlatformWindow for HeadlessPlatformWindow {
    fn content_size(&self) -> (u32, u32) {
        self.inner.content_size()
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

    fn pointer_position(&self) -> Option<(f32, f32)> {
        self.pointer_position
    }

    fn touch_scroll_config(&self) -> Option<crate::platform::TouchScrollConfig> {
        self.touch_scroll_config.get()
    }

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    fn is_occluded(&self) -> bool {
        self.occluded.get()
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

#[cfg(not(target_arch = "wasm32"))]
impl crate::platform::GpuSurfaceWindow for HeadlessPlatformWindow {
    fn surface(&mut self) -> &mut dyn crate::platform::SurfaceProvider {
        crate::platform::GpuSurfaceWindow::surface(&mut self.inner)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
impl HeadlessPlatformWindow {
    /// The occlusion report the next [`RuntimeWindow::sync_occlusion`] pulls
    /// — a test's stand-in for the window-system visibility signal.
    pub(super) fn set_occluded(&self, occluded: bool) {
        self.occluded.set(occluded);
    }

    /// The last (min, max) content-size limits the runner applied, for tests.
    pub(super) const fn applied_size_limits(
        &self,
    ) -> Option<(
        Option<waterui_core::layout::Size>,
        Option<waterui_core::layout::Size>,
    )> {
        self.inner.applied_size_limits()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
/// The outcome of one headless frame pump.
pub struct HeadlessPumpResult {
    /// Whether the pump rebuilt the view tree.
    pub rebuilt: bool,
    /// The frame's CPU/GPU stage profile.
    pub profile: FrameProfile,
    /// The frame's CPU/GPU stage split under `frame-profile`: GPU stages stay
    /// `None` on a device without `TIMESTAMP_QUERY`.
    #[cfg(feature = "frame-profile")]
    pub stages: crate::renderer::FrameStageTimes,
    #[cfg(feature = "accessibility")]
    /// The accessibility tree update the frame produced.
    pub tree_update: Option<AccessibilityTreeUpdate>,
    /// The captured frame snapshot, when capture was requested.
    pub snapshot: Option<HeadlessSnapshot>,
    #[cfg(feature = "accessibility")]
    /// The accessibility node holding UI focus.
    pub ui_focus: Option<accesskit::NodeId>,
}

#[cfg(not(target_arch = "wasm32"))]
/// A headless hydrolysis runtime for tests and tooling.
pub struct HeadlessRuntime {
    env: Environment,
    runtime: RuntimeWindow<HeadlessPlatformWindow>,
    pending_window_queue: Rc<RefCell<Vec<Window>>>,
    popup_windows: Vec<RuntimeWindow<HeadlessPlatformWindow>>,
    /// The style the runtime was launched with, kept so popup windows'
    /// renderers measure and encode with the same widget theme.
    theme: Rc<dyn crate::engine::WidgetTheme>,
    /// Every window this runtime opens renders on this one device: the main
    /// window, and each popup it later vends. Requesting a device per window
    /// made a runtime that opens a popup pay for two.
    gpu: OffscreenGpuContext,
    /// The application's font collection, the same one the environment carries.
    /// Popup windows get their own renderer, which is seeded from this so it
    /// shapes with the same faces as the main one.
    fonts: FontCollection,
    /// The family-resolution mode the constructor seeded every renderer with
    /// — popup windows' renderers are created under the same mode.
    family_resolution: FontFamilyResolution,
    local_executor: HeadlessMainThreadExecutor,
    /// Declared last so it drops after the runtime state above: consumes any
    /// still-queued spawned work while this thread's locals are intact, so no
    /// runnable is ever dropped during thread-local teardown.
    _executor_teardown: DrainExecutorOnDrop,
    /// Declared after everything that owns GPU resources, for the same reason.
    /// Fields drop in declaration order and `RuntimeWindow` holds its platform
    /// window before its renderer, so a reclaim run from the surface's own drop
    /// happens while the renderer still holds its pipelines and buffers — which
    /// is why a probe building hundreds of runtimes on one device still ran the
    /// machine out of memory. From here, both are already gone.
    _gpu_reclaim: ReclaimGpuOnDrop,
}

// GPU context, executor and window internals carry nothing printable; a
// name-only non-exhaustive form keeps the impl honest.
impl std::fmt::Debug for HeadlessRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeadlessRuntime").finish_non_exhaustive()
    }
}

/// Lets the device release a runtime's GPU resources once the runtime is gone.
#[cfg(not(target_arch = "wasm32"))]
struct ReclaimGpuOnDrop(OffscreenGpuContext);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ReclaimGpuOnDrop {
    fn drop(&mut self) {
        self.0.reclaim();
    }
}

/// The window headless constructors wrap a content builder in: the same
/// default background as platform windows (the theme `Background` slot), so
/// offscreen captures match what `water run` renders from the very first
/// frame.
#[cfg(not(target_arch = "wasm32"))]
fn default_window(content: AnyViewBuilder<AnyView>) -> Window {
    let content_builder = content;
    Window::new(
        "",
        waterui_core::binding(waterui::window::WindowState::Normal),
        move || content_builder.build(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessRuntime {
    #[must_use]
    /// Creates a headless runtime with `env` and the root view builder.
    ///
    /// `family_resolution` decides whether a named font family the collection
    /// cannot resolve is skipped ([`FontFamilyResolution::Lenient`]) or fails
    /// the shape naming it ([`FontFamilyResolution::Strict`]); see
    /// [`FontFamilyResolution`].
    pub fn new(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
        style: impl crate::Style,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self::on_gpu_context(
            pollster::block_on(OffscreenGpuContext::new()),
            env,
            default_window(content),
            width,
            height,
            style,
            family_resolution,
        )
    }

    /// Same as [`Self::new`] around a caller-built [`Window`] — the mount the
    /// window runner performs.
    ///
    /// `Self::new` mounts a synthetic default window, so the application's own
    /// `Window` — its `frame`, `state`, `min_size`/`max_size`, title, and
    /// background — is the runtime's: the viewport writes `Window::frame` at
    /// mount and on every `Moved`/`Resized` event land on the app's binding,
    /// and content reading the app's frame binding (a responsive layout, a
    /// `when()` keyed on width, a size derived from the window) agrees with the
    /// window runner for the same tree.
    #[must_use]
    pub fn new_with_window(
        env: Environment,
        window: Window,
        width: u32,
        height: u32,
        style: impl crate::Style,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self::on_gpu_context(
            pollster::block_on(OffscreenGpuContext::new()),
            env,
            window,
            width,
            height,
            style,
            family_resolution,
        )
    }

    /// Renders at `scale_factor` physical pixels per logical pixel.
    ///
    /// The layout is unchanged — it stays in logical units — so this only makes
    /// the captured image sharper. A preview meant to be viewed on a `HiDPI`
    /// display should raise this above 1.
    ///
    /// # Panics
    ///
    /// Panics when `scale_factor` is not finite and positive.
    #[must_use]
    pub fn with_scale_factor(mut self, scale_factor: f64) -> Self {
        self.set_scale_factor(scale_factor);
        self
    }

    /// Moves the display onto a new scale factor mid-run: a window dragged
    /// between monitors of different densities reports a scale change, and
    /// the runtime rebuilds its scale-dependent state — capture chains and
    /// other texel-parameterized content — for the frames after this call.
    ///
    /// # Panics
    ///
    /// Panics when `scale_factor` is not finite and positive.
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.runtime.platform.set_scale_factor(scale_factor);
    }

    /// Creates a headless runtime for `WaterUI` test hosts.
    ///
    /// This constructor allows compute-capable software adapters for CI-only
    /// semantic testing while keeping [`Self::new`] on production adapter
    /// selection, and resolves font families strictly
    /// ([`FontFamilyResolution::Strict`]): a named family the collection
    /// cannot resolve fails the shape naming it, so a style package's missing
    /// fonts fail the test instead of silently substituting a face.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
        style: impl crate::Style,
    ) -> Self {
        Self::on_gpu_context(
            OffscreenGpuContext::new_for_tests_blocking(),
            env,
            default_window(content),
            width,
            height,
            style,
            FontFamilyResolution::Strict,
        )
    }

    /// Same as [`Self::new_for_tests`] around a caller-built [`Window`], so a
    /// test can exercise window-level properties a plain content builder
    /// cannot express — a translucent [`Window::background`], for one.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests_with_window(
        env: Environment,
        window: Window,
        width: u32,
        height: u32,
        style: impl crate::Style,
    ) -> Self {
        Self::on_gpu_context(
            OffscreenGpuContext::new_for_tests_blocking(),
            env,
            window,
            width,
            height,
            style,
            FontFamilyResolution::Strict,
        )
    }

    /// Creates a test runtime on an already-requested [`OffscreenGpuContext`].
    ///
    /// A wgpu device is expensive to request and, on a runner whose only
    /// adapter is a software rasterizer, expensive to hold: a probe that builds
    /// a fresh runtime per sample exhausted the machine requesting one device
    /// per sample. Such a probe requests one context and passes it to every
    /// runtime. The device is all that is shared — the view tree, the renderer
    /// and the retained scene are still built from scratch per runtime, so what
    /// a measurement observes is unchanged. Family resolution is strict, as
    /// on [`Self::new_for_tests`].
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests_on_context(
        gpu: OffscreenGpuContext,
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
        style: impl crate::Style,
    ) -> Self {
        Self::on_gpu_context(
            gpu,
            env,
            default_window(content),
            width,
            height,
            style,
            FontFamilyResolution::Strict,
        )
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    fn on_gpu_context(
        gpu: OffscreenGpuContext,
        env: Environment,
        window: Window,
        width: u32,
        height: u32,
        style: impl crate::Style,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        let inspector = init_main_thread_executors();
        let inspector_probe = inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe);
        let mut env = env.extending(waterui_graphics::scene_view::SceneViewMergeToParent);
        waterui_core::install_application_resources(&mut env);
        waterui::inspector::install(&mut env, inspector);
        let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
        install_native_component_hooks(&mut env);
        install_headless_window_managers(&mut env, Rc::clone(&pending_window_queue));
        env.insert(HydrolysisTextContextMenuMode::Overlay);
        crate::theme::install_theme_tokens(&mut env, Some(&style));
        let theme: Rc<dyn crate::engine::WidgetTheme> = Rc::new(style);
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::new(Rc::clone(&theme)),
        ));
        // The application's fonts, built once: the system collection plus the
        // staged fonts directory, exactly as the windowed runners load them.
        // Every window's renderer is seeded from this collection, and a
        // self-drawn component that typesets text itself reads it out of the
        // environment instead of enumerating the system's fonts for itself.
        let fonts = crate::text::fonts::native_collection(&env);
        fonts.clone().install(&mut env);

        // Headless binaries (preview, tests) have no platform runner to install
        // a tracing subscriber; honor `RUST_LOG` here so they stay debuggable.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init();
        let local_executor = HeadlessMainThreadExecutor::thread_shared();
        // A headless host paces frames itself, so the executor budgets at the
        // headless rate.
        let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
            local_executor.clone(),
            waterui::task::RefreshRate::HEADLESS,
            inspector_probe,
        ));

        window.frame.set(waterui_core::layout::Rect::new(
            waterui_core::layout::Point::zero(),
            waterui_core::layout::Size::new(
                crate::num_cast::u32_as_f32(width.max(1)),
                crate::num_cast::u32_as_f32(height.max(1)),
            ),
        ));

        let mut platform = HeadlessPlatformWindow::on_context(
            gpu.clone(),
            width.max(1),
            height.max(1),
            wgpu::TextureFormat::Rgba8Unorm,
        );
        platform.apply_properties(&window);
        let mut renderer = HydrolysisRenderer::with_engine(
            Rc::clone(&theme),
            SessionTextEngine::from_collection(&fonts, family_resolution),
        );
        renderer.set_window_id(
            env.get::<MenuShortcutRegistry>()
                .expect("install_headless_window_managers seeds MenuShortcutRegistry")
                .mint_window_id(),
        );
        renderer.set_window_closable(window.closable);

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
            theme,
            fonts,
            family_resolution,
            _executor_teardown: DrainExecutorOnDrop::new(local_executor.clone()),
            _gpu_reclaim: ReclaimGpuOnDrop(gpu.clone()),
            gpu,
            local_executor,
        }
    }

    fn create_popup_runtime(&self, window: Window) -> RuntimeWindow<HeadlessPlatformWindow> {
        let frame = crate::platform::validated_window_frame(window.frame.snapshot());
        let width = crate::num_cast::f32_as_u32(frame.width().max(1.0));
        let height = crate::num_cast::f32_as_u32(frame.height().max(1.0));
        let mut platform = HeadlessPlatformWindow::on_context(
            self.gpu.clone(),
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        platform.apply_properties(&window);
        let mut renderer = HydrolysisRenderer::with_engine(
            Rc::clone(&self.theme),
            SessionTextEngine::from_collection(&self.fonts, self.family_resolution),
        );
        renderer.set_window_id(
            self.env
                .get::<MenuShortcutRegistry>()
                .expect("install_headless_window_managers seeds MenuShortcutRegistry")
                .mint_window_id(),
        );
        renderer.set_window_closable(window.closable);
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

    /// Queues an input event for the next pump.
    ///
    /// A real windowing system delivers positional input to the topmost
    /// window under the point: a mounted popup — a `.context_menu` floating
    /// over the main window — owns the presses inside its frame, translated
    /// to window-local coordinates before the popup sees them. Events outside
    /// every popup frame keep landing on the main window's queue.
    pub fn push_input_event(&mut self, event: InputEvent) {
        if let Some((x, y)) = event_position(&event) {
            // The last mounted popup is the topmost one.
            for popup in self.popup_windows.iter_mut().rev() {
                let frame = crate::platform::validated_window_frame(popup.window.frame.snapshot());
                let (fx, fy) = (frame.x(), frame.y());
                if x >= fx && x < fx + frame.width() && y >= fy && y < fy + frame.height() {
                    popup
                        .platform
                        .push_event(translate_input_event(event, fx, fy));
                    return;
                }
            }
        }
        self.runtime.platform.push_event(event);
    }

    /// Sets the pointer position the host reports when asked — independent
    /// of the events it has delivered.
    ///
    /// An OS file drag carries no coordinates, and some platforms suppress
    /// cursor events while the drag owns the pointer; this models a host
    /// that still knows where the pointer is, so the runner's platform query
    /// has an answer.
    pub const fn set_pointer_position(&mut self, position: Option<(f32, f32)>) {
        self.runtime.platform.pointer_position = position;
    }

    /// Requests a repaint on the headless window.
    pub fn request_redraw(&mut self) {
        self.runtime.request_redraw();
        self.runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    }

    /// The adapter and device this runtime renders on — attribution for a
    /// `frame-profile` report, including whether the device was opened with
    /// `TIMESTAMP_QUERY`.
    #[cfg(feature = "frame-profile")]
    #[must_use]
    pub fn gpu_identity(&self) -> crate::GpuIdentity {
        let surface = self.runtime.platform.inner.surface_ref();
        crate::GpuIdentity {
            adapter: surface.adapter().get_info(),
            adapter_features: surface.adapter().features(),
            device_features: surface.device().features(),
        }
    }

    /// Performs an accessibility action against the merged tree.
    ///
    /// Popup-window node ids are shifted into their own stride in the merged
    /// update (see [`SemanticCore::take_merged_accessibility_tree_update`]),
    /// so a request whose target lands in a popup's range demuxes back to that
    /// window's core — the action targets (a menu item's activation, a picker
    /// row's selection) live there. Returns whether the action changed state,
    /// in which case the next pump re-emits.
    ///
    /// # Panics
    /// Panics when the request targets the node of a popup window that is
    /// already closed.
    #[cfg(feature = "accessibility")]
    pub fn perform_accessibility_action(&mut self, request: AccessibilityActionRequest) -> bool {
        /// The same id range
        /// [`SemanticCore::take_merged_accessibility_tree_update`] assigns
        /// each popup.
        const WINDOW_ID_STRIDE: u64 = 1 << 32;

        let target = request.target_node.0;
        let (window, request) = if target >= WINDOW_ID_STRIDE {
            let index = target / WINDOW_ID_STRIDE - 1;
            let popup = self
                .popup_windows
                .get_mut(crate::num_cast::u64_as_usize(index))
                .unwrap_or_else(|| {
                    panic!(
                        "hydrolysis headless runtime: accessibility action {:?} targets closed popup \
                         node {target}",
                        request.action
                    )
                });
            let mut request = request;
            request.target_node = accesskit::NodeId(target % WINDOW_ID_STRIDE);
            (popup, request)
        } else {
            (&mut self.runtime, request)
        };
        let action_env = self.env.extending(runtime_window_origin(window));
        let changed = window
            .renderer
            .handle_accessibility_action(request, &action_env);
        if changed {
            window.request_refresh();
            window.request_redraw();
            window.renderer.frame_work_counters_mut().host_wakeups += 1;
        }
        changed
    }

    /// The accessibility tree of every open window, merged the same way a
    /// pump publishes it — a read-only query: pending per-window updates are
    /// included but stay pending for the next pump to publish, so the pump's
    /// "the tree changed" signal is untouched. Popups keep the id stride the
    /// published merge assigns. `None` when no window has ever produced a
    /// tree.
    #[cfg(feature = "accessibility")]
    pub fn accessibility_tree(&mut self) -> Option<AccessibilityTreeUpdate> {
        self.runtime.renderer.accessibility_tree(
            self.popup_windows
                .iter_mut()
                .map(|popup| &mut *popup.renderer),
        )
    }

    /// The point a pointer could actually reach inside `node`'s accessibility
    /// bounds.
    ///
    /// Node bounds stay the logical rectangle; visibility is a projection
    /// resolved here, at the point of use (water-rs/waterui#1323 §4). What
    /// projects is the region a pointer can activate: for a node whose
    /// `Click` was delegated by a silenced interaction owner (a `List` row
    /// standing in for its `on_tap` strip), that owner's own hit region and
    /// clip; otherwise the node's logical rectangle intersected with the
    /// clip chain in effect when it registered — either way, intersected with
    /// the window bounds (water-rs/waterui#1323 §5). The requested spot is
    /// clamped into the visible fragment. The callers that must produce a
    /// real point — a testing `tap_at`, an automation `pointer tap` —
    /// resolve through this query instead of the bounds' centre, which may
    /// sit inside a clipped region where nothing can be hit
    /// (water-rs/hydrolysis#27). A node with no visible fragment fails with
    /// [`AccessibilityActivationPointError::EmptyFragment`]; an off-screen
    /// point is never returned.
    ///
    /// `(x_fraction, y_fraction)` pick a spot inside the projected region —
    /// `0.5, 0.5` is its centre.
    ///
    /// Popup-window node ids are shifted into their own stride in the merged
    /// update the same way [`Self::perform_accessibility_action`] demultiplexes
    /// them back: a target in a popup's range resolves on that window's core —
    /// its own clip and window bounds — and translates the result by the
    /// popup frame's origin, so the returned point sits in the merged
    /// coordinates a pointer tap pushed to this runtime resolves against.
    /// A shifted id for a closed popup fails with
    /// [`AccessibilityActivationPointError::NoNode`].
    ///
    /// # Errors
    /// Returns [`AccessibilityActivationPointError::NoNode`] when the id
    /// resolves to a closed popup or a node with no projected region, and
    /// [`AccessibilityActivationPointError::UnknownNode`] when no window
    /// holds it.
    #[cfg(feature = "accessibility")]
    pub fn accessibility_activation_point(
        &self,
        node_id: accesskit::NodeId,
        x_fraction: f64,
        y_fraction: f64,
    ) -> Result<kurbo::Point, AccessibilityActivationPointError> {
        /// The same id range
        /// [`SemanticCore::take_merged_accessibility_tree_update`] assigns
        /// each popup.
        const WINDOW_ID_STRIDE: u64 = 1 << 32;

        let target = node_id.0;
        if target >= WINDOW_ID_STRIDE {
            let index = target / WINDOW_ID_STRIDE - 1;
            let Some(popup) = self.popup_windows.get(crate::num_cast::u64_as_usize(index)) else {
                return Err(AccessibilityActivationPointError::NoNode);
            };
            let point = popup.renderer.accessibility_activation_point(
                accesskit::NodeId(target % WINDOW_ID_STRIDE),
                x_fraction,
                y_fraction,
            )?;
            let frame = crate::platform::validated_window_frame(popup.window.frame.snapshot());
            return Ok(kurbo::Point::new(
                point.x + f64::from(frame.x()),
                point.y + f64::from(frame.y()),
            ));
        }
        self.runtime
            .renderer
            .accessibility_activation_point(node_id, x_fraction, y_fraction)
    }

    /// Where the runner would anchor the platform's input-method panel.
    ///
    /// This is the value the runner hands to
    /// [`PlatformWindow::sync_text_input_state`](crate::PlatformWindow::sync_text_input_state)
    /// every frame; a headless host has no panel to place, so tests read it
    /// from here.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn focused_text_input_state(&self) -> Option<crate::platform::TextInputState> {
        self.runtime.renderer.focused_text_input_state()
    }

    #[cfg(feature = "accessibility")]
    /// Clears UI focus; returns whether focus changed.
    pub fn clear_ui_focus(&mut self) -> bool {
        let changed = self.runtime.renderer.clear_ui_focus();
        if changed {
            self.runtime.request_refresh();
            self.runtime.request_redraw();
            self.runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
        }
        changed
    }

    #[cfg(feature = "accessibility")]
    #[must_use]
    /// The accessibility node holding UI focus, if any.
    pub fn focused_ui_node(&self) -> Option<accesskit::NodeId> {
        self.runtime.renderer.focused_ui_node()
    }

    /// Whether the runtime is quiescent: no queued input, no spawned work
    /// awaiting a drain, no pending popup mounts, no window — the main window
    /// or a popup — with a frame still pending, and no renderer-scheduled
    /// semantic work (patches, rebuilds, animations, gesture deadlines,
    /// gliding scrolls) in this window or any popup.
    ///
    /// Visual-only repaint requests (caret blink, the visible-window present
    /// cadence) do not count: they never move semantic state. Work scheduled
    /// entirely outside the runtime — an app future sleeping on a wall-clock
    /// timer, a worker thread that has not yet woken its task — is invisible
    /// here until it wakes, so callers waiting on such work must keep polling
    /// with their own timeout rather than trusting one settled probe.
    #[must_use]
    pub fn is_settled(&self) -> bool {
        !self.runtime.platform.has_pending_events()
            && !self.local_executor.has_pending()
            && self.pending_window_queue.borrow().is_empty()
            && !self.runtime.mode.is_pending()
            && !self.runtime.renderer.has_scheduled_semantic_work()
            && self.popup_windows.iter().all(|popup| {
                !popup.mode.is_pending()
                    && !popup.platform.has_pending_events()
                    && !popup.renderer.has_scheduled_semantic_work()
            })
    }

    /// Whether a state change has been requested but not yet flushed, so the
    /// semantics this runtime last produced are stale.
    ///
    /// Unlike [`Self::is_settled`] this says nothing about work that keeps
    /// going of its own accord — an animation, a gliding scroll, an armed
    /// gesture deadline. It answers only "is what I last observed still
    /// current?", which is what an observer needs before reading the tree: an
    /// app with a perpetual animation is never settled, but it is very often
    /// up to date.
    #[must_use]
    pub fn has_pending_semantic_update(&self) -> bool {
        self.runtime.mode.is_unapplied_change()
            || self.runtime.renderer.has_pending_semantic_update()
            || self.popup_windows.iter().any(|popup| {
                popup.mode.is_unapplied_change() || popup.renderer.has_pending_semantic_update()
            })
    }

    /// Pumps one frame, capturing a snapshot when `capture_snapshot` is set.
    pub fn pump(&mut self, capture_snapshot: bool) -> HeadlessPumpResult {
        self.pump_at(capture_snapshot, Instant::now())
    }

    /// Pumps one frame without a snapshot.
    pub fn pump_offscreen(&mut self) -> HeadlessPumpResult {
        self.pump_at(false, Instant::now())
    }

    /// Pumps one frame and captures a snapshot.
    pub fn pump_snapshot(&mut self) -> HeadlessPumpResult {
        self.pump_at(true, Instant::now())
    }

    /// The touch-gesture parameters this runtime's host reports — a test's
    /// stand-in for the platform's `ViewConfiguration` push, applied by the
    /// next input dispatch.
    #[cfg(any(test, feature = "testing"))]
    pub fn set_touch_scroll_config(&mut self, config: crate::platform::TouchScrollConfig) {
        self.runtime.platform.set_touch_scroll_config(config);
    }

    /// The environment the runtime renders its windows in, for tests that
    /// read what a frame resolved against — its colour scheme, say.
    #[cfg(test)]
    pub(crate) const fn env(&self) -> &Environment {
        &self.env
    }

    /// The main window's renderer, for tests that assert on frame internals.
    #[cfg(test)]
    pub(crate) const fn renderer(&self) -> &HydrolysisRenderer {
        &self.runtime.renderer
    }

    /// The main window's renderer mutably — editing-session tests write
    /// projections through it.
    #[cfg(test)]
    pub(crate) const fn renderer_mut(&mut self) -> &mut HydrolysisRenderer {
        &mut self.runtime.renderer
    }

    /// The `Window` the `index`th mounted popup was built from — the value the
    /// popup machinery emitted — so a test can assert on the window the
    /// platform layer will realize.
    #[cfg(test)]
    pub(crate) fn popup_window(&self, index: usize) -> Option<&Window> {
        self.popup_windows.get(index).map(|runtime| &runtime.window)
    }

    /// Captures the `index`th popup's own rendered frame through
    /// `render_window_with_capture` — the same pump-and-present path the
    /// winit runner drives per frame — rather than the semantic tree or the
    /// composited main-window snapshot.
    #[cfg(test)]
    pub(crate) fn popup_frame(&mut self, index: usize) -> Option<HeadlessSnapshot> {
        let popup = self.popup_windows.get_mut(index)?;
        render_window_with_capture(popup, &self.env, FrameReader::Snapshot, &mut || {
            self.local_executor.drain()
        })
        .snapshot
    }

    /// The mounted popup windows' logical frames — where each transient
    /// window was anchored, in the same coordinate space pointer input is
    /// delivered in — in mount order.
    #[cfg(test)]
    pub(crate) fn popup_frames(&self) -> Vec<waterui_core::layout::Rect> {
        self.popup_windows
            .iter()
            .map(|popup| crate::platform::validated_window_frame(popup.window.frame.snapshot()))
            .collect()
    }

    /// The open `.context_menu` presentation's drawn frames in hit space —
    /// `(menu, accessory)` — or `None` when no drawn presentation is open.
    /// The drawn menu mounts no popup window, so `popup_frames` stays empty
    /// while it is up.
    #[cfg(test)]
    pub(crate) fn context_menu_presentation_frames(
        &self,
    ) -> Option<(kurbo::Rect, Option<kurbo::Rect>)> {
        self.runtime.renderer.context_menu_presentation_frames()
    }

    /// The row frames inside the open drawn menu, in hit order — empty when
    /// no drawn presentation is open.
    #[cfg(test)]
    pub(crate) fn context_menu_row_frames(&self) -> Vec<kurbo::Rect> {
        self.runtime.renderer.context_menu_row_frames()
    }

    /// The drawn frames of the presented `.anchored_overlay` overlays, in hit
    /// order — empty when none is presented.
    #[cfg(test)]
    pub(crate) fn anchored_overlay_frames(&self) -> Vec<kurbo::Rect> {
        self.runtime.renderer.anchored_overlay_frames()
    }

    /// The lifted preview's frame in the open drawn `.context_menu`
    /// presentation — the source's rect unless fitting the stack moved or
    /// cropped it — or `None` when none is open.
    #[cfg(test)]
    pub(crate) fn context_menu_lift_frame(&self) -> Option<kurbo::Rect> {
        self.runtime.renderer.context_menu_lift_frame()
    }

    /// Digest of the last layout pass's placed bounds — the frame-profile
    /// example's byte-identical layout check.
    #[cfg(feature = "frame-profile")]
    #[must_use]
    pub const fn layout_signature(&self) -> Option<u64> {
        self.runtime.renderer.layout_signature()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    /// Pumps one frame with the frame instant pinned to `at`.
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
        // A popup whose state flipped `Closed` — its menu group dismissed it
        // — leaves the merged tree. Flag the main window so the update
        // re-emits without it.
        if self
            .popup_windows
            .iter()
            .any(|popup| popup.window.state.snapshot() == waterui::window::WindowState::Closed)
        {
            self.popup_windows.retain(|popup| {
                popup.window.state.snapshot() != waterui::window::WindowState::Closed
            });
            self.runtime.request_refresh();
            self.runtime.request_redraw();
            self.runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
        }
        let should_render = capture_snapshot
            || self.runtime.mode.is_pending()
            || self.runtime.platform.take_redraw_request();
        let mut render_result = should_render.then(|| {
            render_window_with_capture(
                &mut self.runtime,
                &self.env,
                FrameReader::headless(capture_snapshot),
                &mut || self.local_executor.drain(),
            )
        });
        // The popup renders below can rebuild too; `rebuilt` reports the OR
        // of every pass this pump ran, not only the main window's render.
        let mut rebuilt = render_result.as_ref().is_some_and(|result| result.rebuilt);
        // The pump's profile is the main window's frame: a capture's readback
        // ran inside that same render — queued after the frame's submissions
        // on the same queue — and the popup passes below only composite their
        // snapshots into it.
        let rendered_profile = render_result.as_ref().map(|result| result.profile);
        // A popup window pumps its scene the way the main window does: the
        // scene pump is where its retained tree — and with it the window's
        // accessibility update — is built. A popup that only ever rendered on
        // capture frames would composite into snapshots yet never reach the
        // merged tree, so an open popup gets a frame whenever its own work is
        // pending, while readback stays limited to the frames that composite
        // it into a snapshot.
        let mut popup_snapshots = Vec::new();
        for (popup_index, popup) in self.popup_windows.iter_mut().enumerate() {
            let composite = capture_snapshot
                && render_result
                    .as_ref()
                    .is_some_and(|result| result.snapshot.is_some());
            let redraw_requested = popup.platform.take_redraw_request();
            if !(composite || popup.mode.is_pending() || redraw_requested) {
                continue;
            }
            let popup_result = render_window_with_capture(
                popup,
                &self.env,
                FrameReader::headless(composite),
                &mut || self.local_executor.drain(),
            );
            rebuilt |= popup_result.rebuilt;
            if let Some(popup_snapshot) = popup_result.snapshot {
                popup_snapshots.push((
                    popup_index,
                    crate::platform::validated_window_frame(popup.window.frame.snapshot()),
                    popup_snapshot,
                ));
            }
        }
        if let Some(snapshot) = render_result
            .as_mut()
            .and_then(|result| result.snapshot.as_mut())
        {
            for (_, frame, popup_snapshot) in popup_snapshots {
                composite_popup_snapshot(snapshot, &popup_snapshot, frame);
            }
        }
        let executor_after_started_at = Instant::now();
        let drained_after = self.local_executor.drain();
        let executor_after = executor_after_started_at.elapsed();

        let mut profile = rendered_profile
            .or_else(|| render_result.as_ref().map(|result| result.profile))
            .unwrap_or_default();
        if render_result.is_some() {
            // The renderer's frame-work counters are sampled after the frame's
            // render and its readback have both submitted on the same queue,
            // so they cover all the GPU work this pump ran for it.
            profile.counters.frame_work = self.runtime.renderer.frame_work_counters();
        }
        profile.phases.executor_before = executor_before;
        profile.phases.input = input;
        profile.phases.animation = animation;
        profile.phases.executor_after = executor_after;

        HeadlessPumpResult {
            rebuilt: rebuilt
                || render_result.as_ref().is_some_and(|result| result.rebuilt)
                || drained_before
                || drained_after,
            profile: profile.with_total(frame_started_at.elapsed()),
            #[cfg(feature = "frame-profile")]
            stages: render_result
                .as_ref()
                .map_or_else(crate::renderer::FrameStageTimes::default, |result| {
                    result.stages
                }),
            #[cfg(feature = "accessibility")]
            tree_update: self.runtime.renderer.take_merged_accessibility_tree_update(
                self.popup_windows
                    .iter_mut()
                    .map(|popup| &mut *popup.renderer),
            ),
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
    let offset_x = crate::num_cast::f32_as_i32(frame.x().round());
    let offset_y = crate::num_cast::f32_as_i32(frame.y().round());
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
    let out_alpha = f32::mul_add(target_alpha, 1.0 - source_alpha, source_alpha);
    for channel in 0..3 {
        let source_channel = f32::from(source[channel]) / 255.0;
        let target_channel = f32::from(target[channel]) / 255.0;
        let out = f32::mul_add(
            target_channel * target_alpha,
            1.0 - source_alpha,
            source_channel * source_alpha,
        ) / out_alpha;
        target[channel] = crate::num_cast::f32_as_u8((out * 255.0).round().clamp(0.0, 255.0));
    }
    target[3] = crate::num_cast::f32_as_u8((out_alpha * 255.0).round().clamp(0.0, 255.0));
}

/// The window-local point a positional event carries, if it is one.
const fn event_position(event: &InputEvent) -> Option<(f32, f32)> {
    match event {
        InputEvent::PointerDown { x, y, .. }
        | InputEvent::PointerUp { x, y, .. }
        | InputEvent::PointerMove { x, y, .. }
        | InputEvent::Scroll { x, y, .. }
        | InputEvent::TrackpadPan { x, y, .. }
        | InputEvent::Magnification { x, y, .. }
        | InputEvent::Rotation { x, y, .. } => Some((*x, *y)),
        _ => None,
    }
}

/// Re-expresses a positional event in a mounted window's local coordinate
/// space — the subtraction the windowing system performs before an event
/// reaches the window under the point.
fn translate_input_event(event: InputEvent, dx: f32, dy: f32) -> InputEvent {
    match event {
        InputEvent::PointerDown {
            id,
            kind,
            x,
            y,
            button,
        } => InputEvent::PointerDown {
            id,
            kind,
            x: x - dx,
            y: y - dy,
            button,
        },
        InputEvent::PointerUp {
            id,
            kind,
            x,
            y,
            button,
        } => InputEvent::PointerUp {
            id,
            kind,
            x: x - dx,
            y: y - dy,
            button,
        },
        InputEvent::PointerMove { id, kind, x, y } => InputEvent::PointerMove {
            id,
            kind,
            x: x - dx,
            y: y - dy,
        },
        InputEvent::Scroll {
            x,
            y,
            dx: sx,
            dy: sy,
            is_line_delta,
        } => InputEvent::Scroll {
            x: x - dx,
            y: y - dy,
            dx: sx,
            dy: sy,
            is_line_delta,
        },
        InputEvent::TrackpadPan {
            x,
            y,
            dx: px,
            dy: py,
            phase,
        } => InputEvent::TrackpadPan {
            x: x - dx,
            y: y - dy,
            dx: px,
            dy: py,
            phase,
        },
        InputEvent::Magnification { x, y, delta, phase } => InputEvent::Magnification {
            x: x - dx,
            y: y - dy,
            delta,
            phase,
        },
        InputEvent::Rotation { x, y, delta, phase } => InputEvent::Rotation {
            x: x - dx,
            y: y - dy,
            delta,
            phase,
        },
        other => other,
    }
}

/// Real device-generation coverage for the retained renderer: private to
/// `cfg(test)` so no public API, trait, `testing` helper or export is added.
///
/// The fixture keeps the mounted [`HeadlessRuntime`]'s renderer and view
/// tree, destroys the original wgpu device, and swaps in a platform window
/// bound to a *new* [`OffscreenGpuContext`] — the same shape the platform
/// runner performs when a device is replaced — then drives the normal
/// production `pump_at` path.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod generation_tests {
    use super::*;
    use crate::platform::GpuSurfaceWindow;
    use crate::renderer::tests::{MinimalTestTheme, test_environment};
    use core::time::Duration;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Instant;
    use waterui_graphics::draw::{
        Draw as _, FontId, Glyph, GlyphRun, GlyphStyle, ImageId, Recorder, Sampling, WorkingColor,
    };
    use waterui_graphics::{
        FontSource, ImageData, RecordingResources, Registered, Rgba8, SceneContent,
        SceneInvalidator, SceneView,
    };

    /// Replaces `runtime`'s GPU context: the platform window is rebuilt on
    /// `gpu`'s own context id (fresh pooled `SharedEngineState`, fresh
    /// `SceneResources` table) while the retained `HydrolysisRenderer` and
    /// view tree — including every `Weak` table association — carry over
    /// unchanged. Returns the replaced context so the test can drop it last,
    /// matching the runner's own `ReclaimGpuOnDrop` discipline.
    #[expect(
        clippy::used_underscore_binding,
        reason = "the private test replaces the intentionally unread RAII guard when replacing its GPU context"
    )]
    fn replace_gpu_context(
        runtime: &mut HeadlessRuntime,
        gpu: OffscreenGpuContext,
    ) -> OffscreenGpuContext {
        let (width, height) = runtime.runtime.platform.content_size();
        runtime.runtime.platform = HeadlessPlatformWindow::on_context(
            gpu.clone(),
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        runtime
            .runtime
            .platform
            .apply_properties(&runtime.runtime.window);
        runtime._gpu_reclaim = ReclaimGpuOnDrop(gpu.clone());
        std::mem::replace(&mut runtime.gpu, gpu)
    }

    /// Scene content that registers real resources — the installed Roboto
    /// test font and a synthesized 4x4 image — on whatever
    /// `RecordingResources` table the engine hands it, then names them into
    /// the recording (which asserts the table that owns them) and draws a
    /// fill, a glyph run and the image. Engine-bound `Registered` handles
    /// and the invalidator are cleared by `rebuild_for_engine`; the
    /// semantic `builds`/`installs` counters are ordinary view state and
    /// survive replacement.
    struct ResourceSceneContent {
        builds: Rc<Cell<u32>>,
        rebuilds: Rc<Cell<u32>>,
        installs: Rc<Cell<u32>>,
        latest: Rc<RefCell<Option<SceneInvalidator>>>,
        font: Option<Registered<FontId>>,
        image: Option<Registered<ImageId>>,
    }

    impl SceneContent for ResourceSceneContent {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            let _ = (width, height);
            self.builds.set(self.builds.get() + 1);
            if self.font.is_none() {
                self.font = Some(
                    resources
                        .font(FontSource::bytes(crate::text::fonts::installed_font_bytes(
                            "Roboto",
                        )))
                        .expect("test font registers on the engine's resource table"),
                );
            }
            if self.image.is_none() {
                let mut pixels = Vec::with_capacity(4 * 4 * 4);
                for index in 0_u8..16 {
                    pixels.extend_from_slice(&[255, index * 16, 64, 255]);
                }
                self.image = Some(
                    resources
                        .image(ImageData::<Rgba8>::new(4, 4, pixels).expect("valid image data"))
                        .expect("test image registers on the engine's resource table"),
                );
            }
            // Naming a `Registered` whose table is not this recording's
            // table panics — the assertion itself proves the association
            // retargeted the new engine's table after replacement.
            let font_id = resources.name(self.font.as_ref().expect("registered"));
            let image_id = resources.name(self.image.as_ref().expect("registered"));
            // Red band across the top: fill pixels on the device.
            recorder.fill(
                kurbo::Rect::new(8.0, 8.0, 40.0, 24.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            // A row of real glyphs through the registered font: the band
            // collects whatever outlines the face maps these ids to.
            recorder.glyphs(
                GlyphRun {
                    font: font_id,
                    size: 16.0,
                    coords: Vec::new().into(),
                    glyphs: (10_u32..80)
                        .step_by(7)
                        .enumerate()
                        .map(|(i, id)| Glyph {
                            id,
                            x: crate::num_cast::usize_as_f32(i).mul_add(7.0, 8.0),
                            y: 38.0,
                            transform: None,
                        })
                        .collect::<Vec<_>>()
                        .into(),
                    style: GlyphStyle::Fill,
                },
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
            // The registered image, nearest-sampled so each texel lands on a
            // deterministic 2x2 pixel block.
            recorder.image(
                image_id,
                kurbo::Rect::new(8.0, 48.0, 16.0, 56.0),
                Sampling::Nearest,
            );
            false
        }

        fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
            if invalidator.is_some() {
                self.installs.set(self.installs.get() + 1);
            }
            *self.latest.borrow_mut() = invalidator;
        }

        /// Engine-bound state drops: the `Registered` handles belong to the
        /// old engine's table and — as the contract permits — the installed
        /// invalidator, which the host re-installs before the next record.
        /// Semantic counters stay.
        fn rebuild_for_engine(&mut self) {
            self.rebuilds.set(self.rebuilds.get() + 1);
            self.latest.replace(None);
            self.font = None;
            self.image = None;
        }
    }

    /// Mounts a resource-registering `SceneView` on a real GPU context,
    /// destroys its device (the asynchronous device-lost callback must
    /// arrive within a bounded observation), swaps in a fresh context, and
    /// verifies the retained content rebuilds its engine state exactly once
    /// on the new table while ordinary frames and captures stay quiet.
    #[test]
    fn device_replacement_rebuilds_scene_resources_once() {
        let builds = Rc::new(Cell::new(0_u32));
        let rebuilds = Rc::new(Cell::new(0_u32));
        let installs = Rc::new(Cell::new(0_u32));
        let latest = Rc::new(RefCell::new(None));
        let gpu_a = OffscreenGpuContext::new_for_tests_blocking();
        let mut runtime = HeadlessRuntime::new_for_tests_on_context(
            gpu_a,
            test_environment(),
            AnyViewBuilder::new({
                let builds = Rc::clone(&builds);
                let rebuilds = Rc::clone(&rebuilds);
                let installs = Rc::clone(&installs);
                let latest = Rc::clone(&latest);
                move || {
                    AnyView::new(SceneView::new(ResourceSceneContent {
                        builds: Rc::clone(&builds),
                        rebuilds: Rc::clone(&rebuilds),
                        installs: Rc::clone(&installs),
                        latest: Rc::clone(&latest),
                        font: None,
                        image: None,
                    }))
                }
            }),
            96,
            64,
            MinimalTestTheme::default(),
        );
        let t0 = Instant::now();
        runtime.pump_at(true, t0);
        assert_eq!(rebuilds.get(), 0, "first mount must not rebuild");
        assert!(builds.get() >= 1, "first frame records the scene");
        assert_eq!(installs.get(), 1, "mount installs the invalidator once");

        // Destroy the actual device through wgpu, then drive the device's own
        // reclaim — the mechanism every frame's `reclaim_device` uses — so
        // wgpu detects the loss lazily and delivers the platform-installed
        // callback. Bounded wait: a missing callback fails, never skips.
        let surface = runtime.runtime.platform.surface();
        let device_loss = surface.device_loss().clone();
        surface.device().destroy();
        let device = surface.device().clone();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !device_loss.is_lost() && Instant::now() < deadline {
            crate::platform::reclaim_device(&device);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            device_loss.is_lost(),
            "device.destroy() delivered no device-lost callback within 10s of device polls"
        );

        let gpu_b = OffscreenGpuContext::new_for_tests_blocking();
        let _replaced = replace_gpu_context(&mut runtime, gpu_b);

        let mut second = runtime.pump_at(true, t0 + Duration::from_millis(16));
        assert_eq!(
            rebuilds.get(),
            1,
            "replacement rebuilds content exactly once"
        );
        assert_eq!(installs.get(), 2, "invalidator re-installed after rebuild");
        let snapshot = second.snapshot.take().expect("capture after replacement");
        assert_eq!((snapshot.width, snapshot.height), (96, 64));
        let pixel = |x: usize, y: usize| {
            let offset = (y * 96 + x) * 4;
            &snapshot.rgba8[offset..offset + 4]
        };
        // The fill covers (8,8)..(40,24): real red pixels on the new device.
        assert!(pixel(24, 16)[0] > 200, "fill renders on the new device");
        // The image's first texel is (255,0,64,255); nearest sampling pins it
        // to a deterministic block on the NEW table's registration.
        assert_eq!(
            pixel(9, 49),
            &[255, 0, 64, 255],
            "registered image texel renders on the replacement device"
        );
        // The glyphs are opaque blue: require strong B, weak R and G, so the
        // red fill, image texels and an opaque background cannot pass.
        let glyph_ink = (26..42)
            .flat_map(|y| (8..80).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let px = pixel(x, y);
                px[3] > 0 && px[2] > 100 && px[0] < 100 && px[1] < 100
            })
            .count();
        assert!(glyph_ink > 0, "registered font's blue glyphs render");

        // Ordinary later frames and captures never rebuild again.
        runtime.pump_at(false, t0 + Duration::from_millis(32));
        runtime.pump_at(true, t0 + Duration::from_millis(48));
        assert_eq!(rebuilds.get(), 1, "later frames do not rebuild again");
        assert!(
            builds.get() >= 3,
            "semantic view state survives replacement"
        );

        // The re-installed invalidator is the retained semantic callback:
        // firing it still requests a frame on the live renderer, so the
        // fine-grained invalidation path resumed on the new engine.
        let invalidator = latest
            .borrow()
            .clone()
            .expect("the re-installed invalidator is held by the content");
        invalidator();
        assert!(
            runtime.runtime.renderer.take_patch_request(),
            "re-installed invalidator requests a frame on the new engine"
        );
    }
}
