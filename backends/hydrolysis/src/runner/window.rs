//! Per-window frame driving: the `FrameMode` state machine, `RuntimeWindow`,
//! scene rebuild/refresh/render phases, and input-event dispatch.

use super::*;

#[cfg(feature = "winit")]
#[cfg(not(target_os = "linux"))]
pub(super) fn probe_accessibility_runtime() -> bool {
    true
}

#[cfg(feature = "winit")]
#[cfg(target_os = "linux")]
pub(super) fn probe_accessibility_runtime() -> bool {
    let output = Command::new("busctl")
        .args([
            "--user",
            "get-property",
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.a11y.Status",
            "ScreenReaderEnabled",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            let stderr = str::from_utf8(&output.stderr)
                .map(str::trim)
                .unwrap_or("<non-utf8 stderr>");
            tracing::warn!(
                target: "waterui::hydrolysis::a11y",
                status = %output.status,
                stderr,
                "disabling accesskit adapter: org.a11y.Bus probe failed"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                target: "waterui::hydrolysis::a11y",
                error = %error,
                "disabling accesskit adapter: failed to execute busctl probe"
            );
            false
        }
    }
}

/// The work scheduled for the next pump of a window.
///
/// Every awake frame runs the full pass — apply pending patches, re-read
/// reactive inputs, run layout, re-encode the retained tree — game-engine
/// style. There is no cheaper "skip layout" frame kind: layout runs every
/// frame so the presented scene can never be stale against it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FrameMode {
    /// Nothing scheduled.
    Idle,
    /// Refresh the retained window tree on the next pump (building it first if this
    /// renderer has not built it yet).
    Refresh,
}

impl FrameMode {
    pub(super) const fn is_pending(self) -> bool {
        !matches!(self, FrameMode::Idle)
    }

    /// Whether this frame relayouts the retained tree, which every non-idle
    /// frame now does. Kept for the runner tests, which assert on the scheduled
    /// frame's kind directly.
    #[cfg(test)]
    pub(super) const fn needs_layout(self) -> bool {
        matches!(self, FrameMode::Refresh)
    }
}

pub(super) struct RuntimeWindow<P: PlatformWindow> {
    pub(super) window: Window,
    pub(super) platform: P,
    pub(super) renderer: HydrolysisRenderer,
    pub(super) mode: FrameMode,
    pub(super) pointer_position: Option<(f32, f32)>,
    pub(super) render_diagnostics: RenderDiagnostics,
    /// Last display refresh rate (Hz) observed from the platform, used to detect changes
    /// and re-derive the diagnostics frame budget. `None` until first observed.
    pub(super) refresh_rate_hz: Option<f64>,
}

impl<P: PlatformWindow> RuntimeWindow<P> {
    pub(super) fn new(
        window: Window,
        platform: P,
        mut renderer: HydrolysisRenderer,
        render_diagnostics_config: RenderDiagnosticsConfig,
    ) -> Self {
        if let Some(handle) = platform.gpu_surface_redraw_handle() {
            renderer.set_host_redraw_handle(handle);
        }
        Self {
            window,
            platform,
            renderer,
            mode: FrameMode::Refresh,
            pointer_position: None,
            render_diagnostics: RenderDiagnostics::new(render_diagnostics_config),
            refresh_rate_hz: None,
        }
    }

    /// Schedules a refresh of the retained window tree on the next pump (the first
    /// pump builds the tree).
    pub(super) fn request_refresh(&mut self) {
        self.mode = FrameMode::Refresh;
    }

    pub(super) fn clear_frame_mode(&mut self) {
        self.mode = FrameMode::Idle;
    }
}

/// Applies the window's effective content-size limits to the platform window:
/// the explicit `Window::min_size`/`max_size` signals when set (read through the
/// renderer so a change schedules a frame), with the content's measured layout
/// limits as the defaults.
/// Push this frame's window size limits to the platform window.
///
/// Content-derived limits cost four whole-tree measure passes, so they are only
/// measured when the answer will be used: never for a window that does not act
/// on limits at all, and never when the app has pinned both axes explicitly.
pub(super) fn apply_window_size_limits<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) {
    if !runtime.platform.applies_size_limits() {
        return;
    }
    let explicit_min = runtime
        .window
        .min_size
        .clone()
        .map(|signal| runtime.renderer.read_signal(&signal));
    let explicit_max = runtime
        .window
        .max_size
        .clone()
        .map(|signal| runtime.renderer.read_signal(&signal));
    let content_limits = if explicit_min.is_some() && explicit_max.is_some() {
        None
    } else {
        runtime.renderer.measure_content_size_limits(env)
    };
    let min = explicit_min.or_else(|| content_limits.map(|limits| limits.minimum));
    let max = explicit_max.or_else(|| content_limits.and_then(|limits| limits.maximum));
    runtime.platform.set_size_limits(min, max);
}

pub(super) fn schedule_animation_update<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    animations_active: bool,
) {
    if !animations_active {
        return;
    }
    // Every animated scalar is re-sampled in the render tree's node flush; the
    // tick schedules a full frame like every other content change.
    runtime.request_refresh();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlessSnapshot {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct RenderWindowResult {
    pub(super) rebuilt: bool,
    pub(super) snapshot: Option<HeadlessSnapshot>,
    pub(super) profile: FrameProfile,
}

/// Phase timing for one Hydrolysis frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramePhases {
    /// Time spent draining local executor work before input dispatch.
    pub executor_before: Duration,
    /// Time spent dispatching pending input.
    pub input: Duration,
    /// Time spent advancing animations and invalidation clocks.
    pub animation: Duration,
    /// Time spent updating the retained scene, including refresh and re-encode work.
    pub rebuild: Duration,
    /// Time spent building the root WaterUI view value during scene rebuild.
    pub build_content: Duration,
    /// Time spent dispatching WaterUI views into Hydrolysis scene/layout state.
    pub scene_dispatch: Duration,
    /// Time spent finalizing layout, interaction, and accessibility state after dispatch.
    pub scene_finish: Duration,
    /// Time spent acquiring the target frame.
    pub acquire: Duration,
    /// Time spent submitting rendering work.
    pub render: Duration,
    /// Time spent presenting the frame.
    pub present: Duration,
    /// Time spent draining local executor work after rendering.
    pub executor_after: Duration,
}

/// Counter snapshot for one Hydrolysis frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCounters {
    /// Number of rebuild loop iterations in this frame.
    pub rebuild_iterations: u32,
    /// Measurement cache hits in this frame.
    pub measurement_cache_hits: u32,
    /// Measurement cache misses in this frame.
    pub measurement_cache_misses: u32,
    /// Number of compositor layers submitted for this frame.
    pub scene_layers: u32,
    /// Number of Vello scene layers submitted for this frame.
    pub vello_scene_layers: u32,
    /// Number of embedded GPU surface layers submitted for this frame.
    pub gpu_surface_layers: u32,
    /// Number of Vello clip layers pushed while building this frame.
    pub clip_layers: u32,
    /// Maximum nested Vello clip depth while building this frame.
    pub max_clip_depth: u32,
    /// Number of AppliedFilter nodes dispatched in this frame.
    pub applied_filter_count: u32,
    /// Time spent capturing AppliedFilter input subtrees, in microseconds.
    pub applied_filter_capture_us: u64,
    /// Time spent running AppliedFilter GPU effects, in microseconds.
    pub applied_filter_effect_us: u64,
    /// Whether this frame rendered to the target.
    pub rendered: bool,
    /// Whether this frame captured a CPU snapshot.
    pub captured_snapshot: bool,
}

/// Detailed profile for one Hydrolysis frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameProfile {
    /// Total wall-clock duration for the frame pump.
    pub total: Duration,
    /// Phase timing breakdown.
    pub phases: FramePhases,
    /// Counter snapshot.
    pub counters: FrameCounters,
}

impl FrameProfile {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn with_total(mut self, total: Duration) -> Self {
        self.total = total;
        self
    }
}

/// Wakes the platform window after an input event: any content change —
/// structural rebuild, reactive patch, scroll offset, scrollbar drag — runs a
/// full refresh frame; only a no-op event falls through to a bare re-present.
pub(super) fn schedule_redraw_or_refresh<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    changed: bool,
) {
    if !changed {
        return;
    }
    // A pending renderer-side rebuild request is subsumed by the refresh;
    // consume it so it does not schedule a stale extra frame later.
    let _ = runtime.renderer.take_rebuild_request();
    runtime.request_refresh();
    runtime.platform.request_redraw();
}

pub(super) fn create_bounds(width: u32, height: u32, scale_factor: f64) -> vello::kurbo::Rect {
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis runner: invalid scale factor {scale_factor}"
    );
    vello::kurbo::Rect::new(
        0.0,
        0.0,
        f64::from(width) / scale_factor,
        f64::from(height) / scale_factor,
    )
}

pub(super) fn window_clear_color(window: &Window, env: &Environment) -> vello::peniko::Color {
    match &window.background {
        WindowBackground::Opaque => {
            resolve_window_clear_color(Color::new(theme::color::Background), env)
        }
        WindowBackground::Color(color) => resolve_window_clear_color(color.clone(), env),
    }
}

pub(super) fn resolve_window_clear_color(color: Color, env: &Environment) -> vello::peniko::Color {
    let resolved = color.resolve(env).get();
    let srgb = resolved.to_srgb_with_headroom();
    vello::peniko::Color::new([srgb.red, srgb.green, srgb.blue, resolved.opacity])
}

#[cfg(feature = "winit")]
pub(super) fn window_requires_transparency(window: &Window, env: &Environment) -> bool {
    match &window.background {
        WindowBackground::Opaque => false,
        WindowBackground::Color(color) => color.resolve(env).get().opacity < 1.0,
    }
}

pub(super) fn render_window<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = render_window_with_capture(runtime, env, false, drain_local_tasks);
    #[cfg(target_arch = "wasm32")]
    let result = render_window_with_capture(runtime, env, false, drain_local_tasks);
    #[cfg(target_arch = "wasm32")]
    let _ = (result.rebuilt, result.snapshot, result.profile);
}

pub(super) const fn surface_error_requires_reconfigure(
    error: crate::platform::SurfaceError,
) -> bool {
    matches!(
        error,
        crate::platform::SurfaceError::Lost | crate::platform::SurfaceError::Outdated
    )
}

pub(super) fn acquire_surface_frame(
    surface: &mut dyn crate::platform::SurfaceProvider,
) -> Result<crate::platform::SurfaceFrame, crate::platform::SurfaceError> {
    match surface.acquire() {
        Err(error) if surface_error_requires_reconfigure(error) => {
            // Lost/outdated means the swap chain itself is invalid. Reconfigure
            // at the current physical size and retry once in this same frame so
            // live resize does not expose a stale or empty buffer.
            let (width, height) = surface.size();
            surface.resize(width, height);
            surface.acquire()
        }
        result => result,
    }
}

/// Refreshes the retained window tree in place: apply pending `Dynamic` patches,
/// re-read every reactive input, run full layout, and re-encode the scene. A
/// geometry-static frame (animation, scroll, re-present) pays only re-encode.
fn refresh_window_scene<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    phases: &mut FramePhases,
) {
    let refresh_started_at = Instant::now();
    let scale_factor = runtime.platform.scale_factor();
    let (width, height) = runtime.platform.surface().size();
    let bounds = create_bounds(width, height, scale_factor);
    let transform = vello::kurbo::Affine::scale(scale_factor);
    runtime
        .renderer
        .flush_window_tree(env, bounds, transform, vello::kurbo::Affine::IDENTITY);
    // An in-flight press/drag must follow the re-laid-out widget, and hover must be
    // re-evaluated at the pointer so a reflow that moved a widget under the cursor
    // updates its hover chrome.
    runtime
        .renderer
        .sync_active_interactions_after_layout(runtime.pointer_position);
    phases.scene_dispatch += refresh_started_at.elapsed();
    if let Some((x, y)) = runtime.pointer_position
        && runtime.renderer.sync_pointer_hover_state(x, y, env)
    {
        // Hover changed under a static pointer (a reflow moved a widget): the change
        // is recorded in interaction state; schedule one more frame to re-encode the
        // updated chrome.
        runtime.renderer.request_redraw();
    }
}

/// Builds the retained window tree from the app's `body()`. Runs exactly once per
/// renderer lifetime — every later frame updates the retained tree instead.
fn build_window_scene<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    bounds: vello::kurbo::Rect,
    root_transform: vello::kurbo::Affine,
    drain_local_tasks: &mut dyn FnMut() -> bool,
    phases: &mut FramePhases,
) {
    runtime.renderer.reset_scene();
    runtime.renderer.begin_rebuild_frame();
    runtime.renderer.set_window_bounds(bounds);
    let build_content_started_at = Instant::now();
    let content = runtime.window.build_content();
    phases.build_content += build_content_started_at.elapsed();
    let _ = drain_local_tasks();
    let scene_dispatch_started_at = Instant::now();
    runtime.renderer.capture_window_tree(
        content,
        env,
        bounds,
        root_transform,
        vello::kurbo::Affine::IDENTITY,
    );
    runtime
        .renderer
        .render_active_text_context_menu_overlay(env, root_transform);
    phases.scene_dispatch += scene_dispatch_started_at.elapsed();
    let scene_finish_started_at = Instant::now();
    runtime.renderer.finish_rebuild_frame();
    runtime
        .renderer
        .sync_active_interactions_after_layout(runtime.pointer_position);
    phases.scene_finish += scene_finish_started_at.elapsed();
}

/// One pump of the window's retained render tree: builds the tree from the app's
/// `body()` on the first pump, then either refreshes geometry-affecting state or
/// performs a visual-only re-encode.
///
/// Returns whether the tree was built this pump, the number of build passes (0 or 1,
/// kept for frame diagnostics), and the phase timing breakdown.
pub(super) fn pump_window_scene<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) -> ScenePumpOutcome {
    let scale_factor = runtime.platform.scale_factor();
    let surface = runtime.platform.surface();
    let (width, height) = surface.size();
    let bounds = create_bounds(width, height, scale_factor);
    let root_transform = vello::kurbo::Affine::scale(scale_factor);
    runtime
        .renderer
        .set_frame_resources(surface.adapter(), surface.device(), surface.queue());

    let pump_started_at = Instant::now();
    let mut phases = FramePhases::default();
    let animations_active = runtime.renderer.advance_animations();
    schedule_animation_update(runtime, animations_active);

    let renderer_requested_rebuild = runtime.renderer.take_rebuild_request();
    if renderer_requested_rebuild {
        runtime.request_refresh();
    }

    let mut built = false;
    let mut flushed = false;
    match runtime.mode {
        FrameMode::Idle => {}
        FrameMode::Refresh if !runtime.renderer.has_render_tree() => {
            build_window_scene(
                runtime,
                env,
                bounds,
                root_transform,
                drain_local_tasks,
                &mut phases,
            );
            built = true;
            flushed = true;
            runtime.clear_frame_mode();
            // Anything the build itself flagged as needing another pass — a hover
            // change under the pointer, a renderer-side structural request raised
            // mid-build — is satisfied by refreshing the freshly built tree in the
            // same frame.
            let mut refresh_after_build = false;
            if let Some((x, y)) = runtime.pointer_position
                && runtime.renderer.sync_pointer_hover_state(x, y, env)
            {
                if runtime.renderer.take_rebuild_request() {
                    refresh_after_build = true;
                } else {
                    runtime.renderer.request_redraw();
                }
            }
            if runtime.renderer.take_rebuild_request() {
                refresh_after_build = true;
            }
            if refresh_after_build {
                refresh_window_scene(runtime, env, &mut phases);
            }
        }
        FrameMode::Refresh => {
            refresh_window_scene(runtime, env, &mut phases);
            runtime.clear_frame_mode();
            flushed = true;
        }
    }
    if runtime.renderer.take_next_frame_rebuild_request() {
        // An effect needs another frame.
        runtime.request_refresh();
        runtime.platform.request_redraw();
    } else if runtime.renderer.animations_active() && !runtime.mode.is_pending() {
        schedule_animation_update(runtime, true);
        runtime.platform.request_redraw();
    }
    phases.rebuild = pump_started_at.elapsed();
    ScenePumpOutcome {
        built,
        flushed,
        phases,
    }
}

/// What one scene pump did: whether the retained tree was built for the first
/// time, and whether any flush (build, re-encode, or refresh) ran at all this
/// frame. An idle pump leaves both false.
pub(super) struct ScenePumpOutcome {
    pub(super) built: bool,
    pub(super) flushed: bool,
    pub(super) phases: FramePhases,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn pump_window_semantics<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> bool {
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(feature = "winit")]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.get().as_str());

    if runtime.renderer.take_rebuild_request() {
        runtime.request_refresh();
    }
    let work_pending = runtime.mode.is_pending()
        || runtime.renderer.has_patch_request()
        || runtime.renderer.take_redraw_request();
    if !work_pending {
        return false;
    }
    // Semantic mode has no GPU present, but content changes still move the
    // accessibility tree, which the render tree emits during `flush`. Re-flush
    // the retained tree — patch, layout, and re-encode, the same full pass as a
    // rendered frame — so semantics stay in sync; if no tree exists yet, the
    // pump below builds it first.
    if runtime.renderer.has_render_tree() {
        let scale_factor = runtime.platform.scale_factor();
        let (width, height) = runtime.platform.surface().size();
        let bounds = create_bounds(width, height, scale_factor);
        let transform = vello::kurbo::Affine::scale(scale_factor);
        let flushed = runtime.renderer.flush_window_tree(
            env,
            bounds,
            transform,
            vello::kurbo::Affine::IDENTITY,
        );
        assert!(
            flushed,
            "hydrolysis runner: retained render tree vanished during semantics pump"
        );
        apply_window_size_limits(runtime, env);
        runtime.clear_frame_mode();
        return true;
    }

    let rebuilt = pump_window_scene(runtime, env, &mut || false).built;
    apply_window_size_limits(runtime, env);
    runtime.renderer.clear_frame_resources();
    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
    if let Some((x, y)) = runtime.pointer_position {
        runtime
            .platform
            .set_cursor_style(runtime.renderer.cursor_style_at(x, y));
    }
    if runtime.renderer.take_redraw_request() {
        runtime.platform.request_redraw();
    }
    rebuilt
}

struct SurfaceRenderResult {
    acquire: Duration,
    render: Duration,
    present: Duration,
    snapshot: Option<HeadlessSnapshot>,
}

fn render_to_surface(
    renderer: &mut HydrolysisRenderer,
    surface: &mut dyn crate::platform::SurfaceProvider,
    clear_color: vello::peniko::Color,
    capture_snapshot: bool,
    render: impl FnOnce(&mut HydrolysisRenderer, crate::renderer::HydrolysisRenderTarget<'_>),
) -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
    let (width, height) = surface.size();
    let format = surface.format();
    let acquire_started_at = Instant::now();
    let frame = acquire_surface_frame(surface)?;
    let acquire = acquire_started_at.elapsed();
    let render_started_at = Instant::now();
    render(
        renderer,
        crate::renderer::HydrolysisRenderTarget {
            adapter: surface.adapter(),
            device: surface.device(),
            queue: surface.queue(),
            texture: Some(frame.texture()),
            view: frame.view(),
            format,
            width,
            height,
            base_color: clear_color,
        },
    );
    #[cfg(not(target_arch = "wasm32"))]
    let snapshot = capture_snapshot.then(|| HeadlessSnapshot {
        width,
        height,
        rgba8: readback_texture_rgba8(
            surface.device(),
            surface.queue(),
            frame.texture(),
            width,
            height,
        ),
    });
    #[cfg(target_arch = "wasm32")]
    let snapshot = {
        assert!(
            !capture_snapshot,
            "browser surfaces cannot be synchronously read back for a headless snapshot"
        );
        None
    };
    let render = render_started_at.elapsed();
    let present_started_at = Instant::now();
    surface.present(frame);
    let present = present_started_at.elapsed();
    Ok(SurfaceRenderResult {
        acquire,
        render,
        present,
        snapshot,
    })
}

pub(super) fn render_window_with_capture<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    capture_snapshot: bool,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) -> RenderWindowResult {
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(feature = "winit")]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.get().as_str());
    let mut snapshot = None;
    let mut rebuilt = false;
    let profile;
    // The scheduled mode is cleared while the scene is pumped, so record what
    // this frame was asked to do before that happens.
    let frame_mode = runtime.mode;
    let frame_pump_started_at = Instant::now();
    {
        let diagnostics_enabled = runtime.render_diagnostics.enabled();
        let frame_started_at = diagnostics_enabled.then(Instant::now);
        let pump_outcome = pump_window_scene(runtime, env, drain_local_tasks);
        let rebuild_phases = pump_outcome.phases;
        rebuilt |= pump_outcome.built;
        apply_window_size_limits(runtime, env);
        let clear_color = window_clear_color(&runtime.window, env);

        let root_transform = vello::kurbo::Affine::scale(runtime.platform.scale_factor());
        #[cfg(hydrolysis_macos_system_webview)]
        let (width, height) = runtime.platform.surface().size();
        // The redraw-only filter refresh exists for frames that present without
        // re-flushing the tree (an animated filter while the scene is idle). Any
        // flush already ran every filter through its node, so refreshing again
        // here would execute animated filters twice per frame.
        if !pump_outcome.flushed {
            runtime.renderer.begin_redraw_frame();
            let surface = runtime.platform.surface();
            runtime
                .renderer
                .refresh_active_applied_filters(surface.device(), surface.queue());
        }
        runtime
            .renderer
            .prepare_transient_text_input_overlay(env, root_transform);

        #[cfg(hydrolysis_macos_system_webview)]
        let mut hybrid_composition = runtime.renderer.take_hybrid_composition();

        #[cfg(hydrolysis_macos_system_webview)]
        let render_result = if let Some(composition) = hybrid_composition.as_mut() {
            assert!(
                !capture_snapshot,
                "Hydrolysis cannot capture native WKWebView pixels through GPU readback"
            );
            let platform = (&mut runtime.platform as &mut dyn std::any::Any)
                .downcast_mut::<crate::platform::WinitWindow>()
                .expect("Hydrolysis native WebView composition requires a winit window");
            platform.sync_hybrid_composition(&composition.native_views, width, height);

            let segment_count = composition.segments.len();
            let mut totals = SurfaceRenderResult {
                acquire: Duration::ZERO,
                render: Duration::ZERO,
                present: Duration::ZERO,
                snapshot: None,
            };
            let mut result = Ok(());
            for (index, segment) in composition.segments.iter_mut().enumerate() {
                let transient_scene = (index + 1 == segment_count)
                    .then(|| composition.transient_scene.take())
                    .flatten();
                let surface = if index == 0 {
                    platform.surface()
                } else {
                    platform.hybrid_overlay_surface(index - 1)
                };
                let segment_clear_color = if index == 0 {
                    clear_color
                } else {
                    vello::peniko::Color::TRANSPARENT
                };
                match render_to_surface(
                    &mut runtime.renderer,
                    surface,
                    segment_clear_color,
                    false,
                    |renderer, target| {
                        renderer.render_hybrid_segment_to_surface(segment, transient_scene, target);
                    },
                ) {
                    Ok(rendered) => {
                        totals.acquire += rendered.acquire;
                        totals.render += rendered.render;
                        totals.present += rendered.present;
                    }
                    Err(error) => {
                        result = Err(error);
                        break;
                    }
                }
            }
            composition.transient_scene.take();
            result.map(|()| totals)
        } else {
            if let Some(platform) = (&mut runtime.platform as &mut dyn std::any::Any)
                .downcast_mut::<crate::platform::WinitWindow>()
            {
                platform.clear_hybrid_composition();
            }
            render_to_surface(
                &mut runtime.renderer,
                runtime.platform.surface(),
                clear_color,
                capture_snapshot,
                HydrolysisRenderer::render_scene_to_surface,
            )
        };

        #[cfg(not(hydrolysis_macos_system_webview))]
        let render_result = render_to_surface(
            &mut runtime.renderer,
            runtime.platform.surface(),
            clear_color,
            capture_snapshot,
            HydrolysisRenderer::render_scene_to_surface,
        );

        #[cfg(hydrolysis_macos_system_webview)]
        if let Some(composition) = hybrid_composition.take() {
            runtime.renderer.restore_hybrid_composition(composition);
        }

        let rendered = match render_result {
            Ok(rendered) => rendered,
            Err(
                crate::platform::SurfaceError::Lost
                | crate::platform::SurfaceError::Outdated
                | crate::platform::SurfaceError::Timeout
                | crate::platform::SurfaceError::Occluded,
            ) => {
                runtime.request_refresh();
                runtime.platform.request_redraw();
                let (measurement_cache_hits, measurement_cache_misses) =
                    runtime.renderer.measurement_cache_stats();
                let (scene_layers, vello_scene_layers, gpu_surface_layers) =
                    runtime.renderer.render_layer_stats();
                let (clip_layers, max_clip_depth) = runtime.renderer.clip_layer_stats();
                let (applied_filter_count, applied_filter_capture_us, applied_filter_effect_us) =
                    runtime.renderer.applied_filter_stats();
                return RenderWindowResult {
                    rebuilt,
                    snapshot,
                    profile: FrameProfile {
                        phases: FramePhases {
                            rebuild: rebuild_phases.rebuild,
                            build_content: rebuild_phases.build_content,
                            scene_dispatch: rebuild_phases.scene_dispatch,
                            scene_finish: rebuild_phases.scene_finish,
                            ..FramePhases::default()
                        },
                        counters: FrameCounters {
                            rebuild_iterations: u32::from(pump_outcome.built),
                            measurement_cache_hits,
                            measurement_cache_misses,
                            scene_layers,
                            vello_scene_layers,
                            gpu_surface_layers,
                            clip_layers,
                            max_clip_depth,
                            applied_filter_count,
                            applied_filter_capture_us,
                            applied_filter_effect_us,
                            rendered: false,
                            captured_snapshot: false,
                        },
                        ..FrameProfile::default()
                    },
                };
            }
            Err(crate::platform::SurfaceError::Validation) => {
                panic!("hydrolysis surface acquisition failed validation")
            }
        };
        let acquire_duration = rendered.acquire;
        let render_duration = rendered.render;
        let present_duration = rendered.present;
        snapshot = rendered.snapshot;
        runtime.renderer.clear_frame_resources();
        let (measurement_cache_hits, measurement_cache_misses) =
            runtime.renderer.measurement_cache_stats();
        let (scene_layers, vello_scene_layers, gpu_surface_layers) =
            runtime.renderer.render_layer_stats();
        let (clip_layers, max_clip_depth) = runtime.renderer.clip_layer_stats();
        let (applied_filter_count, applied_filter_capture_us, applied_filter_effect_us) =
            runtime.renderer.applied_filter_stats();
        profile = FrameProfile {
            phases: FramePhases {
                rebuild: rebuild_phases.rebuild,
                build_content: rebuild_phases.build_content,
                scene_dispatch: rebuild_phases.scene_dispatch,
                scene_finish: rebuild_phases.scene_finish,
                acquire: acquire_duration,
                render: render_duration,
                present: present_duration,
                ..FramePhases::default()
            },
            counters: FrameCounters {
                rebuild_iterations: u32::from(pump_outcome.built),
                measurement_cache_hits,
                measurement_cache_misses,
                scene_layers,
                vello_scene_layers,
                gpu_surface_layers,
                clip_layers,
                max_clip_depth,
                applied_filter_count,
                applied_filter_capture_us,
                applied_filter_effect_us,
                rendered: true,
                captured_snapshot: capture_snapshot,
            },
            ..FrameProfile::default()
        };

        if diagnostics_enabled {
            let window_title = runtime.window.title.get();
            runtime.render_diagnostics.record_frame(
                window_title.as_str(),
                RenderPhaseSample {
                    rebuild: rebuild_phases.rebuild,
                    build_content: rebuild_phases.build_content,
                    scene_dispatch: rebuild_phases.scene_dispatch,
                    scene_finish: rebuild_phases.scene_finish,
                    acquire: acquire_duration,
                    render: render_duration,
                    present: present_duration,
                    total: elapsed_or_zero(frame_started_at),
                    rebuild_iterations: u32::from(pump_outcome.built),
                    applied_filter_count,
                    applied_filter_capture_us,
                    applied_filter_effect_us,
                    rebuilt: pump_outcome.built,
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
    if runtime.renderer.take_redraw_request() {
        runtime.platform.request_redraw();
    }

    super::inspector::publish_frame(env, frame_mode, &profile, frame_pump_started_at.elapsed());
    #[cfg(feature = "accessibility")]
    if let Some(update) = runtime.renderer.peek_accessibility_tree_update() {
        super::inspector::publish_tree(env, update);
    }

    RenderWindowResult {
        rebuilt,
        snapshot,
        profile,
    }
}

pub(super) fn physical_to_logical_dimension(value: u32, scale_factor: f64) -> f32 {
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis runner: invalid scale factor {scale_factor}"
    );
    (f64::from(value) / scale_factor) as f32
}

pub(super) fn handle_input_events<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> bool {
    handle_input_events_with(runtime, env, |runtime, env| {
        env.extending(runtime_window_origin(runtime))
    })
}

pub(super) fn runtime_window_origin<P: PlatformWindow>(
    runtime: &RuntimeWindow<P>,
) -> HydrolysisWindowOrigin {
    HydrolysisWindowOrigin {
        x: runtime.window.frame.get().x(),
        y: runtime.window.frame.get().y(),
    }
}

/// Brings the retained hit-test geometry up to date before queued input is
/// dispatched.
///
/// Reactive layout and platform input are delivered independently. If a scroll
/// wheel event arrives while a Dynamic/lazy item size refresh is pending, using
/// the previous frame's scroll extent can incorrectly reject the event at
/// `max_y == 0`. This preflight patches and lays out the retained tree without
/// presenting it; the already-pending render still presents the refreshed scene
/// normally after input has been applied.
fn refresh_pending_input_geometry<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) {
    // A scheduled frame (`mode == Refresh`) alone does not make hit-test
    // geometry stale: every awake frame ends with a full layout, so geometry
    // only lags behind an *unapplied* content change — a pending reactive
    // patch or structural rebuild.
    let geometry_pending =
        runtime.renderer.has_patch_request() || runtime.renderer.has_rebuild_request();
    if !geometry_pending || !runtime.renderer.has_render_tree() {
        return;
    }

    runtime.request_refresh();
    let scale_factor = runtime.platform.scale_factor();
    let (width, height, adapter, device, queue) = {
        let surface = runtime.platform.surface();
        let (width, height) = surface.size();
        (
            width,
            height,
            surface.adapter().clone(),
            surface.device().clone(),
            surface.queue().clone(),
        )
    };
    runtime
        .renderer
        .set_frame_resources(&adapter, &device, &queue);
    let bounds = create_bounds(width, height, scale_factor);
    let transform = vello::kurbo::Affine::scale(scale_factor);
    assert!(
        runtime
            .renderer
            .flush_window_tree(env, bounds, transform, vello::kurbo::Affine::IDENTITY,),
        "hydrolysis input geometry refresh lost the retained window tree"
    );
    apply_window_size_limits(runtime, env);
}

pub(super) fn handle_input_events_with<P, F>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    input_env: F,
) -> bool
where
    P: PlatformWindow,
    F: Fn(&RuntimeWindow<P>, &Environment) -> Environment,
{
    let mut should_close = runtime.window.state.get() == waterui::window::WindowState::Closed;
    let events = runtime.platform.drain_events();
    if !events.is_empty() {
        refresh_pending_input_geometry(runtime, env);
    }
    for event in events {
        match event {
            InputEvent::CloseRequested => {
                runtime
                    .window
                    .state
                    .set(waterui::window::WindowState::Closed);
                should_close = true;
            }
            InputEvent::Moved { x, y } => {
                let frame = runtime.window.frame.get();
                runtime.window.frame.set(waterui_core::layout::Rect::new(
                    waterui_core::layout::Point::new(x, y),
                    *frame.size(),
                ));
            }
            InputEvent::Resize { width, height } => {
                let frame = runtime.window.frame.get();
                let logical_width =
                    physical_to_logical_dimension(width, runtime.platform.scale_factor());
                let logical_height =
                    physical_to_logical_dimension(height, runtime.platform.scale_factor());
                let frame = waterui_core::layout::Rect::new(
                    frame.origin(),
                    waterui_core::layout::Size::new(logical_width, logical_height),
                );
                runtime.window.frame.set(frame);
                runtime.request_refresh();
                runtime.platform.request_redraw();
            }
            InputEvent::PointerDown {
                id,
                kind,
                x,
                y,
                button,
            } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_pointer_down_with_source(
                    id,
                    kind,
                    x,
                    y,
                    button,
                    &input_env(runtime, env),
                );
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_down",
                    x,
                    y,
                    button = ?button,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::PointerUp {
                id,
                kind,
                x,
                y,
                button,
            } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_pointer_up_with_source(
                    id,
                    kind,
                    x,
                    y,
                    button,
                    &input_env(runtime, env),
                );
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_up",
                    x,
                    y,
                    button = ?button,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::PointerMove { id, kind, x, y } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_pointer_move_with_source(
                    id,
                    kind,
                    x,
                    y,
                    &input_env(runtime, env),
                );
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_move",
                    x,
                    y,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::PointerCancel { id, kind } => {
                let changed = runtime.renderer.handle_pointer_cancel_with_source(
                    id,
                    kind,
                    &input_env(runtime, env),
                );
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_cancel",
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
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
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::TrackpadPan {
                x,
                y,
                dx,
                dy,
                phase,
            } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_trackpad_pan(x, y, dx, dy, phase);
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "trackpad_pan",
                    x,
                    y,
                    dx,
                    dy,
                    ?phase,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::Magnification { x, y, delta, phase } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime
                    .renderer
                    .handle_magnification(x, y, delta, phase, env);
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::Rotation { x, y, delta, phase } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_rotation(x, y, delta, phase, env);
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::TextInput { text } => {
                let changed = runtime.renderer.handle_browser_text_input(text.as_str())
                    || runtime.renderer.handle_text_input(text.as_str());
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "text_input",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::Key {
                key,
                native,
                state: KeyState::Pressed,
                modifiers,
            } => {
                let changed = runtime
                    .renderer
                    .handle_browser_key(true, &key, native, modifiers)
                    || runtime.renderer.handle_key_with_env(
                        &key,
                        modifiers,
                        &input_env(runtime, env),
                    );
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "key_pressed",
                    key = ?key,
                    modifiers = ?modifiers,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::ImePreedit { text } => {
                let changed = runtime.renderer.browser_has_focus()
                    || runtime.renderer.handle_ime_preedit(text.as_str());
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "ime_preedit",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::ImeCommit { text } => {
                let changed = runtime.renderer.handle_browser_commit_text(text.as_str())
                    || runtime.renderer.handle_ime_commit(text.as_str());
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "ime_commit",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::ImeDisabled => {
                let changed = runtime.renderer.handle_ime_disabled();
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "ime_disabled",
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::Key {
                key,
                native,
                state: KeyState::Released,
                modifiers,
            } => {
                let changed = runtime
                    .renderer
                    .handle_browser_key(false, &key, native, modifiers)
                    || runtime
                        .renderer
                        .handle_key_release_with_env(&key, &input_env(runtime, env));
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::ModifiersChanged(modifiers) => {
                runtime.renderer.update_browser_modifiers(modifiers);
            }
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

pub(super) fn advance_runtime<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    now: Instant,
) -> Option<Instant> {
    runtime.renderer.set_frame_instant(now);
    // Track the display refresh rate so the diagnostics slow-frame threshold reflects the
    // real frame budget (e.g. 8.33ms on a 120Hz panel) instead of a hardcoded 60fps.
    let refresh_rate = runtime.platform.refresh_rate_hz();
    if refresh_rate != runtime.refresh_rate_hz {
        runtime.refresh_rate_hz = refresh_rate;
        if let Some(hz) = refresh_rate {
            runtime.render_diagnostics.set_refresh_rate(hz);
        }
    }
    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
    if runtime.renderer.poll_gpu_surface_redraw_handles() {
        runtime.platform.request_redraw();
    }
    if runtime.renderer.handle_gesture_tick(now, env) {
        runtime.request_refresh();
    }
    // Smoothed wheel scrolling eases offsets toward their targets per frame;
    // while any scroll view is still gliding, keep running full frames on the
    // redraw cadence.
    if runtime.renderer.tick_smooth_scrolls(now) {
        runtime.request_refresh();
    }
    let animations_active = runtime.renderer.advance_animations();
    schedule_animation_update(runtime, animations_active);
    // A pending fine-grained reactive patch composites through the window-refresh path,
    // which re-dispatches only the dirty Dynamic nodes. If there is no retained window
    // frame yet (or a structural rebuild is already pending), fall back to a rebuild.
    if runtime.renderer.take_patch_request() {
        // The refresh re-flushes the retained tree, which applies the pending
        // Dynamic patch to only the affected subtree and relays out if it changed size.
        runtime.request_refresh();
        runtime.platform.request_redraw();
    }
    if runtime.renderer.advance_text_caret_animation(now) {
        runtime.renderer.request_redraw();
        runtime.platform.request_redraw();
    }
    if runtime.renderer.take_rebuild_request() {
        runtime.request_refresh();
    }
    let next_deadline = runtime.renderer.next_gesture_deadline();
    #[cfg(any(hydrolysis_cef_webview, feature = "chromium"))]
    let next_deadline = env
        .get::<waterui_browser_cef::CefRuntime>()
        .map_or(next_deadline, |cef| {
            let cef_deadline = cef.pump().instant();
            Some(next_deadline.map_or(cef_deadline, |gesture_deadline| {
                gesture_deadline.min(cef_deadline)
            }))
        });
    if runtime.mode.is_pending() {
        runtime.platform.request_redraw();
    }
    next_deadline
}
