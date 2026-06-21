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

/// The work scheduled for the next pump of a window. Exactly one mode is active at a
/// time; `Rebuild` always takes precedence over the parametric refreshes, so a pending
/// rebuild can never be silently dropped by a later refresh request.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FrameMode {
    /// Nothing scheduled.
    Idle,
    /// Full structural rebuild (re-dispatch the view tree). `downgradable` is true when
    /// the rebuild was requested only by an animation/visual effect, so animation
    /// scheduling may downgrade it to a parametric refresh; false for an explicit
    /// structural rebuild that must run. `reuse_scroll` is true for a rebuild that fell
    /// back from a failed scroll refresh and may reuse retained scroll-content caches.
    Rebuild {
        downgradable: bool,
        reuse_scroll: bool,
    },
    /// Parametric refresh of the retained window frame — animation replay, reactive patch,
    /// and scroll-offset changes (scrolling is subsumed into the window frame).
    WindowRefresh,
}

impl FrameMode {
    pub(super) const fn is_pending(self) -> bool {
        !matches!(self, FrameMode::Idle)
    }

    pub(super) const fn is_rebuild(self) -> bool {
        matches!(self, FrameMode::Rebuild { .. })
    }

    pub(super) const fn is_explicit_rebuild(self) -> bool {
        matches!(
            self,
            FrameMode::Rebuild {
                downgradable: false,
                ..
            }
        )
    }

    pub(super) const fn is_window_refresh(self) -> bool {
        matches!(self, FrameMode::WindowRefresh)
    }

    /// Whether a rebuild in this mode may reuse retained scroll-content caches (a scroll
    /// refresh that fell back to a rebuild) rather than invalidating them.
    pub(super) const fn reuses_scroll_caches(self) -> bool {
        matches!(
            self,
            FrameMode::Rebuild {
                reuse_scroll: true,
                ..
            }
        )
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
        renderer: HydrolysisRenderer,
        render_diagnostics_config: RenderDiagnosticsConfig,
    ) -> Self {
        Self {
            window,
            platform,
            renderer,
            mode: FrameMode::Rebuild {
                downgradable: false,
                reuse_scroll: false,
            },
            pointer_position: None,
            render_diagnostics: RenderDiagnostics::new(render_diagnostics_config),
            refresh_rate_hz: None,
        }
    }

    /// Schedules a full structural rebuild, superseding any pending parametric refresh.
    pub(super) fn request_structural_rebuild(&mut self) {
        self.mode = FrameMode::Rebuild {
            downgradable: false,
            reuse_scroll: false,
        };
    }

    /// Schedules a full structural rebuild, used when the renderer reports a
    /// structural change that the retained render tree cannot reflect parametrically.
    pub(super) fn request_invalidating_rebuild(&mut self) {
        self.request_structural_rebuild();
    }

    /// Schedules a full rebuild that animation scheduling may still downgrade to a
    /// parametric refresh.
    pub(super) fn request_downgradable_rebuild(&mut self) {
        self.mode = FrameMode::Rebuild {
            downgradable: true,
            reuse_scroll: false,
        };
    }

    /// Schedules a rebuild that fell back from a failed scroll refresh; the rebuild may
    /// reuse retained scroll-content caches rather than re-dispatching them.
    pub(super) fn request_scroll_fallback_rebuild(&mut self) {
        self.mode = FrameMode::Rebuild {
            downgradable: false,
            reuse_scroll: true,
        };
    }

    pub(super) fn request_window_refresh(&mut self) {
        self.mode = FrameMode::WindowRefresh;
    }

    pub(super) fn clear_frame_mode(&mut self) {
        self.mode = FrameMode::Idle;
    }
}

pub(super) fn schedule_animation_update<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    animations_active: bool,
) {
    if !animations_active {
        return;
    }
    let explicit_rebuild_pending =
        runtime.mode.is_explicit_rebuild() || runtime.renderer.has_rebuild_request();
    if !explicit_rebuild_pending {
        // Every animated scalar is re-sampled in the render tree's node flush, so a
        // window refresh re-encodes the active animation with no rebuild or re-measure.
        runtime.request_window_refresh();
        return;
    }
    // The animation cannot be driven by a parametric refresh; force a rebuild. Keep it
    // downgradable unless an explicit structural rebuild is already pending.
    if explicit_rebuild_pending {
        runtime.request_structural_rebuild();
    } else {
        runtime.request_downgradable_rebuild();
    }
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
    /// Time spent rebuilding scene/layout state.
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
    pub(super) fn with_total(mut self, total: Duration) -> Self {
        self.total = total;
        self
    }
}

pub(super) fn schedule_redraw_or_rebuild<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    changed: bool,
) {
    if !changed {
        return;
    }
    if runtime.renderer.take_rebuild_request() {
        runtime.request_invalidating_rebuild();
    } else {
        runtime.renderer.request_redraw();
    }
    runtime.platform.request_redraw();
}

pub(super) fn schedule_scroll_scene_rebuild<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    changed: bool,
) {
    if !changed {
        return;
    }
    if runtime.renderer.take_rebuild_request() {
        runtime.request_invalidating_rebuild();
    } else if !runtime.mode.is_rebuild() {
        // Scrolling re-composites at the new offset via a window refresh; the render
        // tree's ScrollNode re-reads its handle's offset on each flush.
        runtime.request_window_refresh();
    } else {
        runtime.request_structural_rebuild();
    }
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
    let _ = render_window_with_capture(runtime, env, false, drain_local_tasks);
}

pub(super) fn rebuild_window_scene<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) -> (bool, u32, FramePhases) {
    let scale_factor = runtime.platform.scale_factor();
    let surface = runtime.platform.surface();
    let (width, height) = surface.size();
    let bounds = create_bounds(width, height, scale_factor);
    let root_transform = vello::kurbo::Affine::scale(scale_factor);
    runtime
        .renderer
        .set_frame_resources(surface.device(), surface.queue());

    let rebuild_started_at = Instant::now();
    let mut phases = FramePhases::default();
    let animations_active = runtime.renderer.advance_animations();
    schedule_animation_update(runtime, animations_active);

    let mut rebuilt = false;
    let mut rebuild_iterations = 0u32;
    loop {
        let renderer_requested_rebuild = runtime.renderer.take_rebuild_request();
        if renderer_requested_rebuild {
            runtime.request_invalidating_rebuild();
        }
        if !runtime.mode.is_rebuild() {
            break;
        }
        let reuse_filter_inputs =
            !renderer_requested_rebuild && !runtime.mode.reuses_scroll_caches();
        rebuild_iterations = rebuild_iterations
            .checked_add(1)
            .expect("hydrolysis runner: rebuild iteration counter overflow");
        assert!(
            rebuild_iterations <= 64,
            "hydrolysis runner: rebuild loop exceeded 64 iterations in a single pump"
        );
        runtime.renderer.reset_scene();
        runtime
            .renderer
            .set_applied_filter_input_cache_reuse(reuse_filter_inputs);
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
        runtime.clear_frame_mode();
        rebuilt = true;
        if let Some((x, y)) = runtime.pointer_position
            && runtime.renderer.sync_pointer_hover_state(x, y, env)
        {
            if runtime.renderer.take_rebuild_request() {
                runtime.request_structural_rebuild();
            } else {
                runtime.renderer.request_redraw();
            }
        }
    }
    if runtime.renderer.take_next_frame_rebuild_request() {
        // An effect needs another frame. Keep it downgradable only if nothing else was
        // already scheduled; an in-flight refresh/rebuild forces an explicit rebuild.
        if runtime.mode.is_pending() {
            runtime.request_structural_rebuild();
        } else {
            runtime.request_downgradable_rebuild();
        }
        runtime.platform.request_redraw();
    } else if runtime.renderer.animations_active() && !runtime.mode.is_rebuild() {
        schedule_animation_update(runtime, true);
        runtime.platform.request_redraw();
    }
    phases.rebuild = rebuild_started_at.elapsed();
    (rebuilt, rebuild_iterations, phases)
}

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
        runtime.request_invalidating_rebuild();
    }
    if !runtime.mode.is_rebuild() {
        let replay_requested = runtime.mode.is_window_refresh()
            || runtime.renderer.has_patch_request()
            || runtime.renderer.take_redraw_request();
        if !replay_requested {
            return false;
        }
        // Semantic mode has no GPU present, but replay-driven changes (re-sampled
        // transforms, patched dynamic nodes, redraw-only scalar updates) still move
        // the accessibility tree, which the render tree emits during `flush`. Re-flush
        // the retained tree so semantics stay in sync; if no tree exists yet, fall
        // back to a structural rebuild.
        let scale_factor = runtime.platform.scale_factor();
        let (width, height) = runtime.platform.surface().size();
        let bounds = create_bounds(width, height, scale_factor);
        let transform = vello::kurbo::Affine::scale(scale_factor);
        if runtime
            .renderer
            .flush_window_tree(env, bounds, transform, vello::kurbo::Affine::IDENTITY)
        {
            runtime.clear_frame_mode();
            return true;
        }
        runtime.request_scroll_fallback_rebuild();
    }

    let (rebuilt, _, _) = rebuild_window_scene(runtime, env, &mut || false);
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
    {
        let diagnostics_enabled = runtime.render_diagnostics.enabled();
        let frame_started_at = diagnostics_enabled.then(Instant::now);
        let (scene_rebuilt, rebuild_iterations, mut rebuild_phases) =
            rebuild_window_scene(runtime, env, drain_local_tasks);
        rebuilt |= scene_rebuilt;
        let clear_color = window_clear_color(&runtime.window, env);
        if runtime.mode.is_window_refresh() {
            let refresh_started_at = Instant::now();
            // Re-flush the retained render tree: a geometry-static frame
            // (animation/scroll/re-present) pays only re-encode; a reactive patch
            // relays out the (cheap) retained tree in place. No rebuild fallback —
            // the tree drives every parametric change.
            let scale_factor = runtime.platform.scale_factor();
            let (width, height) = runtime.platform.surface().size();
            let bounds = create_bounds(width, height, scale_factor);
            let transform = vello::kurbo::Affine::scale(scale_factor);
            runtime
                .renderer
                .flush_window_tree(env, bounds, transform, vello::kurbo::Affine::IDENTITY);
            let refresh_duration = refresh_started_at.elapsed();
            rebuild_phases.rebuild += refresh_duration;
            rebuild_phases.scene_dispatch += refresh_duration;
            runtime.clear_frame_mode();
        }

        let root_transform = vello::kurbo::Affine::scale(runtime.platform.scale_factor());
        let surface = runtime.platform.surface();
        let (width, height) = surface.size();
        let format = surface.format();
        let acquire_started_at = Instant::now();
        let frame = match surface.acquire() {
            Ok(frame) => frame,
            Err(crate::platform::SurfaceError::Surface(
                wgpu::SurfaceError::Lost
                | wgpu::SurfaceError::Outdated
                | wgpu::SurfaceError::Timeout
                | wgpu::SurfaceError::Other,
            )) => {
                runtime.request_structural_rebuild();
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
                            acquire: acquire_started_at.elapsed(),
                            ..FramePhases::default()
                        },
                        counters: FrameCounters {
                            rebuild_iterations,
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
            Err(error) => panic!("hydrolysis runner: failed to acquire frame: {error}"),
        };
        let acquire_duration = acquire_started_at.elapsed();
        let render_started_at = Instant::now();
        if !rebuilt {
            runtime.renderer.begin_redraw_frame();
            runtime
                .renderer
                .refresh_active_applied_filters(surface.device(), surface.queue());
        }
        runtime
            .renderer
            .prepare_transient_text_input_overlay(env, root_transform);
        runtime
            .renderer
            .render_scene_to_surface(crate::renderer::HydrolysisRenderTarget {
                device: surface.device(),
                queue: surface.queue(),
                texture: Some(frame.texture()),
                view: frame.view(),
                format,
                width,
                height,
                base_color: clear_color,
            });
        if capture_snapshot {
            snapshot = Some(HeadlessSnapshot {
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
        }
        runtime.renderer.clear_frame_resources();
        let render_duration = render_started_at.elapsed();
        let present_started_at = Instant::now();
        surface.present(frame);
        let present_duration = present_started_at.elapsed();
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
                rebuild_iterations,
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
                    rebuild_iterations,
                    applied_filter_count,
                    applied_filter_capture_us,
                    applied_filter_effect_us,
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
    if runtime.renderer.take_redraw_request() {
        runtime.platform.request_redraw();
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
    for event in runtime.platform.drain_events() {
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
                runtime.request_structural_rebuild();
                runtime.platform.request_redraw();
            }
            InputEvent::PointerDown { x, y, button } => {
                runtime.pointer_position = Some((x, y));
                let changed =
                    runtime
                        .renderer
                        .handle_pointer_down(x, y, button, &input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_down",
                    x,
                    y,
                    button = ?button,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::PointerUp { x, y, button } => {
                runtime.pointer_position = Some((x, y));
                let changed =
                    runtime
                        .renderer
                        .handle_pointer_up(x, y, button, &input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_up",
                    x,
                    y,
                    button = ?button,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::PointerMove { x, y } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime
                    .renderer
                    .handle_pointer_move(x, y, &input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_move",
                    x,
                    y,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::PointerCancel => {
                let changed = runtime
                    .renderer
                    .handle_pointer_cancel(&input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_cancel",
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
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
                schedule_scroll_scene_rebuild(runtime, changed);
            }
            InputEvent::Magnification { x, y, delta, phase } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime
                    .renderer
                    .handle_magnification(x, y, delta, phase, env);
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::Rotation { x, y, delta, phase } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_rotation(x, y, delta, phase, env);
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::TextInput { text } => {
                let changed = runtime.renderer.handle_text_input(text.as_str());
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "text_input",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
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
                schedule_redraw_or_rebuild(runtime, changed);
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
                schedule_redraw_or_rebuild(runtime, changed);
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
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::ImeDisabled => {
                let changed = runtime.renderer.handle_ime_disabled();
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "ime_disabled",
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
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

/// Whether the window opted into game-engine continuous rendering and is currently
/// visible. Minimized/closed windows are excluded so a backgrounded continuous window
/// does not keep driving the GPU.
fn window_wants_continuous_render(window: &Window) -> bool {
    window.continuous_render
        && !matches!(
            window.state.get(),
            waterui::window::WindowState::Minimized | waterui::window::WindowState::Closed
        )
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
        runtime.request_structural_rebuild();
    }
    let animations_active = runtime.renderer.advance_animations();
    schedule_animation_update(runtime, animations_active);
    // A pending fine-grained reactive patch composites through the window-refresh path,
    // which re-dispatches only the dirty Dynamic nodes. If there is no retained window
    // frame yet (or a structural rebuild is already pending), fall back to a rebuild.
    if runtime.renderer.take_patch_request() && !runtime.mode.is_rebuild() {
        // The window refresh re-flushes the retained tree, which applies the pending
        // Dynamic patch to only the affected subtree and relays out if it changed size.
        runtime.request_window_refresh();
        runtime.platform.request_redraw();
    }
    if runtime.renderer.advance_text_caret_animation(now) {
        runtime.renderer.request_redraw();
        runtime.platform.request_redraw();
    }
    if window_wants_continuous_render(&runtime.window) && !runtime.mode.is_rebuild() {
        // Game-engine mode: keep presenting every display refresh while the window is
        // visible. The redraw is delivered through the AutoVsync-gated present, so this
        // paces to the monitor refresh rather than spinning the CPU.
        runtime.request_window_refresh();
        runtime.platform.request_redraw();
    }
    if runtime.renderer.take_rebuild_request() {
        runtime.request_invalidating_rebuild();
    }
    let next_deadline = runtime.renderer.next_gesture_deadline();
    if runtime.mode.is_pending() {
        runtime.platform.request_redraw();
    }
    next_deadline
}
