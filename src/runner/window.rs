//! Per-window frame driving: the `FrameMode` state machine, `RuntimeWindow`,
//! scene rebuild/refresh/render phases, and input-event dispatch.

use super::*;
use crate::platform::GpuSurfaceWindow;

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
    /// Re-sample animated scalars on the next pump: the same full refresh pass
    /// as `Refresh`, but scheduled by the animation tick itself, so it marks
    /// the app busy rather than stale — the tree last emitted is current.
    Animate,
}

impl FrameMode {
    pub(super) const fn is_pending(self) -> bool {
        !matches!(self, FrameMode::Idle)
    }

    /// Whether the scheduled frame exists to apply an unapplied semantic
    /// change. `Animate` is scheduled continuation work, not staleness.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) const fn is_unapplied_change(self) -> bool {
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
    /// The (min, max) inner-size limits most recently pushed to the platform
    /// window. Only a change in them may move the window — an unchanged
    /// re-measure leaves the user's size untouched — so `None` until the
    /// first apply has run.
    pub(super) applied_size_limits: Option<(
        Option<waterui_core::layout::Size>,
        Option<waterui_core::layout::Size>,
    )>,
    /// A frame left a verified-but-unpresented vello composite stashed; the
    /// next wake should drain it (see
    /// [`HydrolysisRenderer::flush_deferred_legacy_frame_to_surface`]) unless
    /// real scene work arrived first.
    pub(super) queued_deferred_flush: bool,
    /// The runner's wake for a resolved deferred stash — fires once per
    /// registered watch through the runner's own event path, never touching
    /// the GPU from the main thread.
    #[cfg(feature = "winit")]
    pub(super) deferred_legacy_wake: Option<std::sync::Arc<dyn Fn(u64) + Send + Sync>>,
    /// The shared per-device poll driver carrying outstanding watches, set
    /// by runners whose platform can report GPU completion. `None` where
    /// there is no completion source (headless, web): those drive the same
    /// armed settle synchronously.
    #[cfg(feature = "winit")]
    pub(super) deferred_poll_driver: Option<crate::platform::GpuPollDriver>,
    /// Generation of the currently armed deferred stash, bumped by every
    /// frame that stashes one.
    pub(super) deferred_stash_gen: u64,
    /// Highest stash generation whose completion wake has been delivered.
    /// A settle comparing it against `deferred_stash_gen` cannot mistake a
    /// stale wake (an older watch landing beside a fresh stash) for the
    /// armed stash's own completion, and a wake that arrives while tickets
    /// still read unresolved means the device poll errored — the drain
    /// runs anyway and the verify surfaces the device loss.
    pub(super) deferred_wake_gen: u64,
}

// `RuntimeWindow` is generic over the host-services contract; the GPU
// painter's presentation attachment is the `GpuSurfaceWindow` refinement
// below, so the construction site of a GPU-pumped window is a
// compile-time painter boundary.
impl<P: GpuSurfaceWindow> RuntimeWindow<P> {
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
            applied_size_limits: None,
            queued_deferred_flush: false,
            #[cfg(feature = "winit")]
            deferred_legacy_wake: None,
            #[cfg(feature = "winit")]
            deferred_poll_driver: None,
            deferred_stash_gen: 0,
            deferred_wake_gen: 0,
        }
    }
}

impl<P: PlatformWindow> RuntimeWindow<P> {
    /// Schedules a refresh of the retained window tree on the next pump (the first
    /// pump builds the tree).
    pub(super) fn request_refresh(&mut self) {
        self.mode = FrameMode::Refresh;
    }

    pub(super) fn clear_frame_mode(&mut self) {
        self.mode = FrameMode::Idle;
    }
}

/// Applies the window's effective inner-size limits to the platform window:
/// the explicit `Window::min_size`/`max_size` signals when set (read through
/// the renderer so a change schedules a frame). The minimum defaults to the
/// content's measured minimum; the maximum stays unbounded unless the app
/// pins one — content never contributes one, since content that does not
/// stretch on an axis is laid out inside a larger offer per the layout spec
/// rather than capping the window.
///
/// The content probe costs a whole-tree measure pass, so it is only taken
/// when the answer will be used: never for a window that does not act on
/// limits at all, and never when the app pins the minimum itself.
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
        .map(|signal| validated_min_size(runtime.renderer.read_signal(&signal)));
    let explicit_max = runtime
        .window
        .max_size
        .clone()
        .map(|signal| validated_max_size(runtime.renderer.read_signal(&signal)));
    let min = match explicit_min {
        Some(min) => Some(min),
        None => runtime.renderer.measure_content_minimum(env),
    };
    let max = explicit_max;
    // A limit apply never moves the window onto the content's size — installing
    // or re-installing limits only constrains the sizes it can take. The size
    // the user settled on survives a re-measure: the window is clamped into
    // the new limits only when the applied limits themselves changed, and only
    // on the axes that fell outside them. The first apply installs limits on
    // the geometry the window was created with, untouched.
    let limits = (min, max);
    let limits_changed = runtime
        .applied_size_limits
        .is_some_and(|applied| applied != limits);
    runtime.applied_size_limits = Some(limits);
    runtime.platform.set_size_limits(min, max);
    if limits_changed {
        let frame = crate::platform::validated_window_frame(runtime.window.frame.snapshot());
        let clamped = clamp_window_size(*frame.size(), min, max);
        if clamped != *frame.size() {
            runtime
                .window
                .frame
                .set(waterui_core::layout::Rect::new(frame.origin(), clamped));
        }
    }
}

/// Asserts an app-pinned `Window::min_size` is finite on both axes — a NaN
/// or infinite minimum is a programming error, not a bound to repair.
fn validated_min_size(size: waterui_core::layout::Size) -> waterui_core::layout::Size {
    for (axis, value) in [("width", size.width), ("height", size.height)] {
        assert!(
            value.is_finite(),
            "hydrolysis runner: Window::min_size.{axis} must be finite, got {value}"
        );
    }
    size
}

/// Asserts an app-pinned `Window::max_size` component is finite or `+∞` —
/// the explicit per-axis "unbounded" an app writes to leave one side open.
/// Any other non-finite value is a programming error.
fn validated_max_size(size: waterui_core::layout::Size) -> waterui_core::layout::Size {
    for (axis, value) in [("width", size.width), ("height", size.height)] {
        assert!(
            value.is_finite() || value == f32::INFINITY,
            "hydrolysis runner: Window::max_size.{axis} must be finite or +inf for an unbounded axis, got {value}"
        );
    }
    size
}

/// Clamps a window size into the new limits, axis by axis. A size already
/// inside the limits passes through untouched — a re-measure keeps the size
/// the user set — and only an out-of-bounds axis moves, to the nearer bound.
pub(super) fn clamp_window_size(
    size: waterui_core::layout::Size,
    min: Option<waterui_core::layout::Size>,
    max: Option<waterui_core::layout::Size>,
) -> waterui_core::layout::Size {
    waterui_core::layout::Size::new(
        clamp_axis(size.width, min.map(|s| s.width), max.map(|s| s.width)),
        clamp_axis(size.height, min.map(|s| s.height), max.map(|s| s.height)),
    )
}

/// Every input is already validated by then: a `+∞` max component is the
/// app's explicit per-axis unbounded, passing through as the high bound and
/// leaving the axis uncapped.
fn clamp_axis(value: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    let lo = min.unwrap_or(0.0);
    value.clamp(lo, max.unwrap_or(f32::INFINITY).max(lo))
}

pub(super) fn schedule_animation_update<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    animations_active: bool,
) {
    if !animations_active {
        return;
    }
    // Every animated scalar is re-sampled in the render tree's node flush; the
    // tick schedules a full frame like every other content change. It is
    // scheduled as `Animate` rather than `Refresh`: the frame continues work
    // already in flight, so it must not read as an unapplied semantic update.
    // A `Refresh` already armed by a patch or rebuild is never downgraded.
    if matches!(runtime.mode, FrameMode::Idle) {
        runtime.mode = FrameMode::Animate;
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
    /// The frame's CPU/GPU stage split under `frame-profile`.
    #[cfg(feature = "frame-profile")]
    pub(super) stages: crate::renderer::FrameStageTimes,
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
    /// Number of GPU surfaces that rendered straight into the window's own
    /// target this frame, skipping the offscreen intermediate and the
    /// compositor pass. At most one: the path exists only for a surface that is
    /// the window's whole content.
    pub direct_gpu_surfaces: u32,
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
    /// Migration-acceptance counters for this frame (water-rs/hydrolysis#205):
    /// semantic builds, patches, layout/measure traffic, recorded content and
    /// GPU submissions, so a fixture can assert the port shrinks them to the
    /// retained-update floor.
    pub migration: crate::renderer::MigrationCounters,
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
    runtime.renderer.migration_counters_mut().host_wakeups += 1;
}

pub(super) fn create_bounds(width: u32, height: u32, scale_factor: f64) -> kurbo::Rect {
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis runner: invalid scale factor {scale_factor}"
    );
    kurbo::Rect::new(
        0.0,
        0.0,
        f64::from(width) / scale_factor,
        f64::from(height) / scale_factor,
    )
}

pub(super) fn window_clear_color(window: &Window, env: &Environment) -> peniko::Color {
    match &window.background {
        WindowBackground::Opaque => {
            resolve_window_clear_color(Color::new(theme::color::Background), env)
        }
        WindowBackground::Color(color) => resolve_window_clear_color(color.clone(), env),
    }
}

pub(super) fn resolve_window_clear_color(color: Color, env: &Environment) -> peniko::Color {
    let resolved = color.resolve(env).snapshot();
    let srgb = resolved.to_srgb_with_headroom();
    peniko::Color::new([srgb.red, srgb.green, srgb.blue, resolved.opacity])
}

#[cfg(feature = "winit")]
pub(crate) fn window_requires_transparency(window: &Window, env: &Environment) -> bool {
    match &window.background {
        WindowBackground::Opaque => false,
        WindowBackground::Color(color) => color.resolve(env).snapshot().opacity < 1.0,
    }
}

/// Runs one frame and reports whether it was presented to the surface — an
/// idle frame, or one whose surface had to be reconfigured, is not.
pub(super) fn render_window<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) -> bool {
    if runtime.queued_deferred_flush {
        runtime.queued_deferred_flush = false;
        // A stashing frame asked for one settle pass. If real scene work also
        // arrived, the full frame below drains the stash itself; otherwise
        // drain and present it here so the last rendered frame is never
        // stranded off-screen. `take_redraw_request` may only consume the
        // flush's own request — the other two checks see real damage.
        let damage_pending = runtime.mode.is_pending()
            || runtime.renderer.has_patch_request()
            || runtime.renderer.take_redraw_request();
        if !damage_pending {
            if !runtime.renderer.has_deferred_legacy_frame() {
                return false;
            }
            // The wake arrives once every submission queued at stash time
            // has retired, so a resolved stash is verified content. The
            // generation check refuses stale wakes — an older watch that
            // lands beside a fresh, unresolved stash resolves nothing; a
            // wake that arrived while tickets still read unresolved means
            // the device poll errored — drain anyway and let the verify
            // report the lost device.
            if runtime.renderer.deferred_legacy_frame_resolved()
                || runtime.deferred_wake_gen >= runtime.deferred_stash_gen
            {
                let rendered = flush_deferred_window(runtime, env, false)
                    .profile
                    .counters
                    .rendered;
                // The drain presents without pumping the scene, so the
                // RedrawRequested it consumed may have carried the scene's
                // animation continuation — without a repump the loop can
                // park with an armed animation (seen on the M4 as sporadic
                // W5 freezes at the growth→tail transition). One redraw
                // lets the next full pass re-arm whatever was absorbed.
                runtime.platform.request_redraw();
                return rendered;
            }
            // The completion watch is still in flight: stay armed and sleep —
            // the poll driver's wake produces the settle, not a clock.
            runtime.queued_deferred_flush = true;
            return false;
        }
    }
    let result = render_window_with_capture(runtime, env, FrameReader::Display, drain_local_tasks);
    // The rebuild flag and the snapshot belong to the headless harness; a live
    // window only asks whether the frame reached its surface.
    let _ = (result.rebuilt, result.snapshot);
    result.profile.counters.rendered
}

/// The drain-only settle pass: verifies and presents the frame the previous
/// render deferred, without pumping the scene. Surfaces with nothing stashed
/// produce an empty frame — the caller only reaches this with a queued flush.
pub(super) fn flush_deferred_window<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    capture_snapshot: bool,
) -> RenderWindowResult {
    let clear_color = window_clear_color(&runtime.window, env);
    let render_result = render_to_surface(
        &mut runtime.renderer,
        runtime.platform.surface(),
        clear_color,
        capture_snapshot,
        |renderer, target, premultiply_alpha| {
            renderer.flush_deferred_legacy_frame_to_surface(target, premultiply_alpha);
        },
    );
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
            return RenderWindowResult {
                rebuilt: false,
                snapshot: None,
                #[cfg(feature = "frame-profile")]
                stages: runtime.renderer.take_frame_stage_times(),
                profile: FrameProfile::default(),
            };
        }
        Err(crate::platform::SurfaceError::Validation) => {
            panic!("hydrolysis surface acquisition failed validation")
        }
    };
    let snapshot = rendered.snapshot;
    #[cfg(feature = "frame-profile")]
    let stages = runtime.renderer.take_frame_stage_times();
    runtime.renderer.clear_frame_resources();
    RenderWindowResult {
        rebuilt: false,
        snapshot,
        #[cfg(feature = "frame-profile")]
        stages,
        profile: FrameProfile {
            phases: FramePhases {
                acquire: rendered.acquire,
                render: rendered.render,
                present: rendered.present,
                ..FramePhases::default()
            },
            counters: FrameCounters {
                rendered: true,
                captured_snapshot: capture_snapshot,
                ..FrameCounters::default()
            },
            ..FrameProfile::default()
        },
    }
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
    let (width, height) = runtime.platform.content_size();
    let bounds = create_bounds(width, height, scale_factor);
    let transform = kurbo::Affine::scale(scale_factor);
    runtime
        .renderer
        .flush_window_tree(env, bounds, transform, kurbo::Affine::IDENTITY);
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
    bounds: kurbo::Rect,
    root_transform: kurbo::Affine,
    drain_local_tasks: &mut dyn FnMut() -> bool,
    phases: &mut FramePhases,
) {
    runtime.renderer.reset_scene();
    runtime.renderer.begin_rebuild_frame();
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
        kurbo::Affine::IDENTITY,
    );
    runtime
        .renderer
        .render_active_text_context_menu_overlay(env, root_transform);
    // The same for an open `.context_menu` presentation — this one-time build
    // path is where its sub-views are first built and placed.
    runtime
        .renderer
        .render_context_menu_presentation(root_transform);
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
pub(super) fn pump_window_scene<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) -> ScenePumpOutcome {
    let scale_factor = runtime.platform.scale_factor();
    let surface = runtime.platform.surface();
    let (width, height) = surface.size();
    let bounds = create_bounds(width, height, scale_factor);
    let root_transform = kurbo::Affine::scale(scale_factor);
    runtime.renderer.set_frame_resources(
        surface.adapter(),
        surface.device(),
        surface.queue(),
        surface.device_loss(),
    );

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
        FrameMode::Refresh | FrameMode::Animate if !runtime.renderer.has_render_tree() => {
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
        FrameMode::Refresh | FrameMode::Animate => {
            refresh_window_scene(runtime, env, &mut phases);
            runtime.clear_frame_mode();
            flushed = true;
        }
    }
    if runtime.renderer.take_next_frame_rebuild_request() {
        // An effect needs another frame.
        runtime.request_refresh();
        runtime.platform.request_redraw();
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
    } else if runtime.renderer.animations_active() && !runtime.mode.is_pending() {
        schedule_animation_update(runtime, true);
        runtime.platform.request_redraw();
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
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

#[cfg(any(test, all(not(target_arch = "wasm32"), feature = "winit")))]
pub(super) fn pump_window_semantics<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> bool {
    // `frame` and `state` drive `apply_properties` below: keep them
    // subscribed so an app write to either binding schedules a pump
    // instead of needing an unrelated event to wake the loop.
    let _ = runtime.renderer.read_signal(&runtime.window.frame);
    let _ = runtime.renderer.read_signal(&runtime.window.state);
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(feature = "winit")]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.snapshot().as_str());

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
        let (width, height) = runtime.platform.content_size();
        let bounds = create_bounds(width, height, scale_factor);
        let transform = kurbo::Affine::scale(scale_factor);
        let flushed =
            runtime
                .renderer
                .flush_window_tree(env, bounds, transform, kurbo::Affine::IDENTITY);
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
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
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
    clear_color: peniko::Color,
    capture_snapshot: bool,
    render: impl FnOnce(&mut HydrolysisRenderer, crate::renderer::HydrolysisRenderTarget<'_>, bool),
) -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
    let (width, height) = surface.size();
    let format = surface.format();
    let premultiply_alpha = surface.premultiply_alpha();
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
            device_loss: surface.device_loss().clone(),
            texture: Some(frame.texture()),
            view: frame.view(),
            format,
            width,
            height,
            base_color: clear_color,
        },
        premultiply_alpha,
    );
    let render = render_started_at.elapsed();
    #[cfg(feature = "frame-profile")]
    {
        // The timestamp resolve blocks until the frame's submits finish — the
        // headless frame's "present wait", kept separate from the CPU submit
        // time `render` measures.
        renderer.finish_gpu_frame_profile(surface.device(), surface.queue());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let snapshot = {
        #[cfg(feature = "frame-profile")]
        let readback_started_at = Instant::now();
        let snapshot = capture_snapshot.then(|| {
            renderer.migration_counters_mut().gpu_submissions += 1;
            HeadlessSnapshot {
                width,
                height,
                rgba8: readback_texture_rgba8(
                    surface.device(),
                    surface.queue(),
                    frame.texture(),
                    width,
                    height,
                ),
            }
        });
        #[cfg(feature = "frame-profile")]
        {
            renderer.frame_stage_times.readback += readback_started_at.elapsed();
        }
        snapshot
    };
    #[cfg(target_arch = "wasm32")]
    let snapshot = {
        assert!(
            !capture_snapshot,
            "browser surfaces cannot be synchronously read back for a headless snapshot"
        );
        None
    };
    let present_started_at = Instant::now();
    surface.present(frame);
    let present = present_started_at.elapsed();
    renderer.note_frame_presented();
    Ok(SurfaceRenderResult {
        acquire,
        render,
        present,
        snapshot,
    })
}

/// Who reads the pixels of a frame `render_window_with_capture` renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "the browser has no headless runtime, so only `Display` frames exist there"
    )
)]
pub(super) enum FrameReader {
    /// A live window: the display reads the presented surface.
    Display,
    /// A headless capture: the harness reads the surface back as a snapshot.
    Snapshot,
    /// A headless pump nobody reads. The frame still pumps the scene, ticks
    /// embedded `GpuSurface` views and composites, but rasterizes no scene
    /// layers: on a software rasterizer each of those submissions is a full
    /// device-bound frame, and no consumer could observe its pixels.
    Nobody,
}

impl FrameReader {
    /// The reader of a headless frame: the snapshot when the pump captures
    /// one, nobody otherwise.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) const fn headless(capture_snapshot: bool) -> Self {
        if capture_snapshot {
            Self::Snapshot
        } else {
            Self::Nobody
        }
    }

    const fn captures(self) -> bool {
        matches!(self, Self::Snapshot)
    }

    const fn rasterizes(self) -> bool {
        !matches!(self, Self::Nobody)
    }
}

pub(super) fn render_window_with_capture<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    reader: FrameReader,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) -> RenderWindowResult {
    let capture_snapshot = reader.captures();
    let _ = runtime.renderer.read_signal(&runtime.window.frame);
    let _ = runtime.renderer.read_signal(&runtime.window.state);
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(feature = "winit")]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.snapshot().as_str());
    let mut snapshot = None;
    let mut rebuilt = false;
    let profile;
    #[cfg(feature = "frame-profile")]
    let stages: crate::renderer::FrameStageTimes;
    // What the inspector is told about this frame, captured before the pump:
    // the scheduled mode is cleared while the scene is pumped, and the elapsed
    // total has to start before any of it runs. A browser page hosts no
    // inspector endpoint, so neither is measured there.
    #[cfg(not(target_arch = "wasm32"))]
    let frame_mode = runtime.mode;
    #[cfg(not(target_arch = "wasm32"))]
    let frame_pump_started_at = Instant::now();
    {
        let diagnostics_enabled = runtime.render_diagnostics.enabled();
        let frame_started_at = diagnostics_enabled.then(Instant::now);
        let pump_outcome = pump_window_scene(runtime, env, drain_local_tasks);
        let rebuild_phases = pump_outcome.phases;
        rebuilt |= pump_outcome.built;
        apply_window_size_limits(runtime, env);
        let clear_color = window_clear_color(&runtime.window, env);

        let root_transform = kurbo::Affine::scale(runtime.platform.scale_factor());
        #[cfg(hydrolysis_macos_system_webview)]
        let (width, height) = runtime.platform.content_size();
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
                    peniko::Color::TRANSPARENT
                };
                match render_to_surface(
                    &mut runtime.renderer,
                    surface,
                    segment_clear_color,
                    false,
                    |renderer, target, premultiply_alpha| {
                        renderer.render_hybrid_segment_to_surface(
                            segment,
                            transient_scene,
                            target,
                            premultiply_alpha,
                        );
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
                |renderer, target, premultiply_alpha| {
                    renderer.render_scene_to_surface_with_alpha_mode(
                        target,
                        premultiply_alpha,
                        reader.rasterizes(),
                    );
                },
            )
        };

        #[cfg(not(hydrolysis_macos_system_webview))]
        let render_result = render_to_surface(
            &mut runtime.renderer,
            runtime.platform.surface(),
            clear_color,
            capture_snapshot,
            |renderer, target, premultiply_alpha| {
                renderer.render_scene_to_surface_with_alpha_mode(
                    target,
                    premultiply_alpha,
                    reader.rasterizes(),
                );
            },
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
                runtime.renderer.migration_counters_mut().host_wakeups += 1;
                let (measurement_cache_hits, measurement_cache_misses) =
                    runtime.renderer.measurement_cache_stats();
                let layer_stats = runtime.renderer.render_layer_stats();
                let (clip_layers, max_clip_depth) = runtime.renderer.clip_layer_stats();
                let (applied_filter_count, applied_filter_capture_us, applied_filter_effect_us) =
                    runtime.renderer.applied_filter_stats();
                return RenderWindowResult {
                    rebuilt,
                    snapshot,
                    #[cfg(feature = "frame-profile")]
                    stages: runtime.renderer.take_frame_stage_times(),
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
                            scene_layers: layer_stats.composited_scene_layers,
                            vello_scene_layers: layer_stats.legacy_scene_layers,
                            gpu_surface_layers: layer_stats.gpu_surface_layers,
                            direct_gpu_surfaces: layer_stats.direct_gpu_surfaces,
                            clip_layers,
                            max_clip_depth,
                            applied_filter_count,
                            applied_filter_capture_us,
                            applied_filter_effect_us,
                            rendered: false,
                            captured_snapshot: false,
                            migration: runtime.renderer.migration_counters(),
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
        #[cfg(feature = "frame-profile")]
        {
            stages = runtime.renderer.take_frame_stage_times();
        }
        runtime.renderer.clear_frame_resources();
        let (measurement_cache_hits, measurement_cache_misses) =
            runtime.renderer.measurement_cache_stats();
        let layer_stats = runtime.renderer.render_layer_stats();
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
                scene_layers: layer_stats.composited_scene_layers,
                vello_scene_layers: layer_stats.legacy_scene_layers,
                gpu_surface_layers: layer_stats.gpu_surface_layers,
                direct_gpu_surfaces: layer_stats.direct_gpu_surfaces,
                clip_layers,
                max_clip_depth,
                applied_filter_count,
                applied_filter_capture_us,
                applied_filter_effect_us,
                rendered: true,
                captured_snapshot: capture_snapshot,
                migration: runtime.renderer.migration_counters(),
            },
            ..FrameProfile::default()
        };

        if diagnostics_enabled {
            let window_title = runtime.window.title.snapshot();
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
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
    }
    if runtime.renderer.has_deferred_legacy_frame() {
        // The frame deferred its legacy verification — arm the settle and
        // park a GPU-completion watch on the poll driver. Its wake fires
        // through the runner's user-event path the moment the stash's
        // submissions retire; the verified composite is then presented
        // rather than stranded in the stash when the stream settles.
        runtime.queued_deferred_flush = true;
        runtime.deferred_stash_gen += 1;
        #[cfg(feature = "winit")]
        let stash_gen = runtime.deferred_stash_gen;
        #[cfg(feature = "winit")]
        if let (Some(driver), Some(wake)) =
            (&runtime.deferred_poll_driver, &runtime.deferred_legacy_wake)
        {
            let wake = wake.clone();
            let submissions = runtime.renderer.deferred_legacy_watch_submissions();
            if !driver.watch(submissions, move || wake(stash_gen)) {
                // The driver thread is gone — treat it like a broken poll:
                // the armed settle drains and lets the verify surface the
                // device loss.
                runtime.deferred_wake_gen = stash_gen;
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        super::inspector::publish_frame(
            env,
            frame_mode,
            &profile,
            frame_pump_started_at.elapsed(),
            runtime.refresh_rate_hz,
        );
        #[cfg(feature = "accessibility")]
        if let Some(update) = runtime.renderer.peek_accessibility_tree_update() {
            super::inspector::publish_tree(env, update);
        }
    }

    RenderWindowResult {
        rebuilt,
        snapshot,
        #[cfg(feature = "frame-profile")]
        stages,
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

pub(super) fn handle_input_events<P: GpuSurfaceWindow>(
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
    let frame = crate::platform::validated_window_frame(runtime.window.frame.snapshot());
    HydrolysisWindowOrigin {
        x: frame.x(),
        y: frame.y(),
    }
}

/// Brings the retained hit-test geometry up to date before a queued scroll
/// event is dispatched.
///
/// Reactive layout and platform input are delivered independently. If a scroll
/// wheel event arrives while a Dynamic/lazy item size refresh is pending, using
/// the previous frame's scroll extent can incorrectly reject the event at
/// `max_y == 0`. This preflight patches and lays out the retained tree without
/// presenting it; the already-pending render still presents the refreshed scene
/// normally after input has been applied.
///
/// The refreshed registrations describe a frame the user has not seen yet, so
/// the caller only runs this for scroll input — pointer events must keep
/// resolving against the presented frame's geometry.
fn refresh_pending_input_geometry<P: GpuSurfaceWindow>(
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
    let (width, height, adapter, device, queue, device_loss) = {
        let surface = runtime.platform.surface();
        let (width, height) = surface.size();
        (
            width,
            height,
            surface.adapter().clone(),
            surface.device().clone(),
            surface.queue().clone(),
            surface.device_loss().clone(),
        )
    };
    runtime
        .renderer
        .set_frame_resources(&adapter, &device, &queue, &device_loss);
    let bounds = create_bounds(width, height, scale_factor);
    let transform = kurbo::Affine::scale(scale_factor);
    assert!(
        runtime
            .renderer
            .flush_window_tree(env, bounds, transform, kurbo::Affine::IDENTITY,),
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
    P: GpuSurfaceWindow,
    F: Fn(&RuntimeWindow<P>, &Environment) -> Environment,
{
    let mut should_close = runtime.window.state.snapshot() == waterui::window::WindowState::Closed;
    let events = runtime.platform.drain_events();
    // Platform IMEs mark their own keystrokes by what they emit: ownership
    // follows the event order inside the batch (see `ime_owned_events`), so
    // a confirming keystroke that arrives before its commit is the
    // composition's while a key after the commit is ordinary input again.
    // Those owned keys must never reach ordinary key handling or an
    // embedded sink — the committing Enter/Backspace in particular must not
    // activate a form or delete committed text.
    let ime_owned = ime::ime_owned_events(&events, runtime.renderer.ime_composition_active());
    let mut geometry_refreshed = false;
    // A key press a handler consumed suppresses the `TextInput` that pairs
    // with it — the press precedes its text in `pending_events`, so this
    // flag set at the press is read by the very next `TextInput` event.
    let mut suppress_key_text = false;
    for (event, ime_owned) in events.into_iter().zip(ime_owned) {
        let key_consumed = suppress_key_text;
        suppress_key_text = false;
        // The preflight re-registers every hit target at the geometry a
        // pending refresh is *about to* paint, so it is reserved for the
        // input that reads scroll extents: a wheel or trackpad-pan delta
        // applied against the last presented extent can be rejected at a
        // stale `max_y`. Pointer input instead resolves against the last
        // *presented* frame — a tap targets the pixels the user saw, so
        // hit-testing it against un-presented geometry would move every
        // region out from under it (water-rs/hydrolysis#208).
        if !geometry_refreshed
            && matches!(
                event,
                InputEvent::Scroll { .. } | InputEvent::TrackpadPan { .. }
            )
        {
            refresh_pending_input_geometry(runtime, env);
            geometry_refreshed = true;
        }
        match event {
            InputEvent::CloseRequested => {
                runtime
                    .window
                    .state
                    .set(waterui::window::WindowState::Closed);
                should_close = true;
            }
            InputEvent::Moved { x, y } => {
                let frame =
                    crate::platform::validated_window_frame(runtime.window.frame.snapshot());
                runtime.window.frame.set(waterui_core::layout::Rect::new(
                    waterui_core::layout::Point::new(x, y),
                    *frame.size(),
                ));
            }
            InputEvent::Resize { width, height } => {
                let frame =
                    crate::platform::validated_window_frame(runtime.window.frame.snapshot());
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
                runtime.renderer.migration_counters_mut().host_wakeups += 1;
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
                let changed = !ime_owned
                    && (runtime.renderer.handle_embedded_text_input(text.as_str())
                        || runtime.renderer.handle_text_input(text.as_str()));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "text_input",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            // The text half of a key press, suppressed when the press was
            // consumed by a handler — the web's preventDefault on keydown
            // cancelling beforeinput.
            InputEvent::KeyText { text } => {
                let changed = !ime_owned
                    && !key_consumed
                    && (runtime.renderer.handle_embedded_text_input(text.as_str())
                        || runtime.renderer.handle_text_input(text.as_str()));
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
                logical_key,
                physical_code,
                repeat,
                state: KeyState::Pressed,
                modifiers,
            } => {
                let changed = if ime_owned {
                    // The press was consumed by the composition; record it so
                    // its release — which wl_keyboard may deliver in a later
                    // batch, after the commit — is swallowed too.
                    runtime.renderer.swallow_ime_key_press(physical_code);
                    false
                } else {
                    let key_env = input_env(runtime, env);
                    let press = KeyPress {
                        key: logical_key.clone(),
                        code: physical_code,
                        modifiers: modifiers.into(),
                        repeat,
                    };
                    let outcome = if runtime.renderer.handle_embedded_key(&KeyDelivery {
                        pressed: true,
                        logical: &logical_key,
                        code: physical_code,
                        repeat,
                        modifiers,
                    }) {
                        // Forwarded, not consumed — the surface owns its
                        // key+text pair; the paired text is still delivered.
                        KeyPressOutcome::ForwardedToSurface
                    } else {
                        runtime
                            .renderer
                            .handle_key_press(&key, modifiers, &key_env, &press)
                    };
                    suppress_key_text = outcome == KeyPressOutcome::Consumed;
                    outcome != KeyPressOutcome::Ignored
                };
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
            InputEvent::ImePreedit { text, caret } => {
                let changed = runtime
                    .renderer
                    .handle_embedded_ime_preedit(text.as_str(), caret)
                    || runtime.renderer.handle_ime_preedit(text.as_str(), caret);
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
                let changed = runtime.renderer.handle_embedded_ime_commit(text.as_str())
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
                let changed = runtime.renderer.handle_embedded_ime_disabled()
                    || runtime.renderer.handle_ime_disabled();
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
                logical_key,
                physical_code,
                repeat,
                state: KeyState::Released,
                modifiers,
            } => {
                let changed =
                    !runtime.renderer.take_ime_swallowed_release(physical_code) && !ime_owned && {
                        let key_env = input_env(runtime, env);
                        runtime.renderer.handle_bubbled_key_release(&KeyDelivery {
                            pressed: false,
                            logical: &logical_key,
                            code: physical_code,
                            repeat,
                            modifiers,
                        }) || runtime.renderer.handle_embedded_key(&KeyDelivery {
                            pressed: false,
                            logical: &logical_key,
                            code: physical_code,
                            repeat,
                            modifiers,
                        }) || runtime.renderer.handle_key_release_with_env(&key, &key_env)
                    };
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::KeyboardCancel => {
                let changed = runtime.renderer.cancel_keyboard_press();
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "keyboard_cancel",
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::ModifiersChanged(modifiers) => {
                runtime.renderer.update_embedded_modifiers(modifiers);
            }
            InputEvent::Focused(focused) => {
                let changed = runtime.renderer.handle_window_focused(focused);
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "window_focused",
                    focused,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::FileHovered { path } => {
                let event_env = input_env(runtime, env);
                let changed = runtime.renderer.handle_file_hovered(path, &event_env);
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::FileDropped { path } => {
                // The file joins the drag's collected list on the renderer;
                // delivery is deferred to `finish_os_file_drop` below —
                // winit reports one event per file and a drop's files can
                // outlive a single batch.
                runtime.renderer.handle_file_dropped(path);
            }
            InputEvent::FileHoverCancelled => {
                let event_env = input_env(runtime, env);
                let changed = runtime.renderer.handle_file_hover_cancelled(&event_env);
                schedule_redraw_or_refresh(runtime, changed);
            }
        }
    }
    {
        let event_env = input_env(runtime, env);
        let changed = runtime.renderer.finish_os_file_drop(&event_env);
        if runtime.renderer.os_file_drop_pending() {
            // The drop's files may still be landing and winit sends no
            // drop-end marker, so the drain that will deliver it exists only
            // if the runner asks for it — request one follow-up pump through
            // the platform redraw request, the same wake a signal change
            // triggers (platform.rs's signal waker calls `request_redraw`).
            runtime.platform.request_redraw();
            runtime.renderer.migration_counters_mut().host_wakeups += 1;
        }
        schedule_redraw_or_refresh(runtime, changed);
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
        tracing::debug!("wake cause: gpu surface redraw handle");
        runtime.platform.request_redraw();
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
    }
    // A gesture tick can mount a popup window — an armed context-menu hold
    // fires here — and the popup anchors in absolute coordinates through
    // `HydrolysisWindowOrigin`, the same extension pointer dispatch gets.
    let gesture_env = env.extending(runtime_window_origin(runtime));
    if runtime.renderer.handle_gesture_tick(now, &gesture_env) {
        tracing::debug!("wake cause: gesture tick fired");
        runtime.request_refresh();
    }
    // Smoothed wheel scrolling eases offsets toward their targets per frame;
    // while any scroll view is still gliding, keep running full frames on the
    // redraw cadence.
    if runtime.renderer.tick_smooth_scrolls(now) {
        tracing::debug!("wake cause: smooth scroll still gliding");
        runtime.request_refresh();
    }
    let animations_active = runtime.renderer.advance_animations();
    if animations_active {
        tracing::debug!("wake cause: animations active");
    }
    schedule_animation_update(runtime, animations_active);
    // A pending fine-grained reactive patch composites through the window-refresh path,
    // which re-dispatches only the dirty Dynamic nodes. If there is no retained window
    // frame yet (or a structural rebuild is already pending), fall back to a rebuild.
    if runtime.renderer.take_patch_request() {
        tracing::debug!("wake cause: reactive patch request");
        // The refresh re-flushes the retained tree, which applies the pending
        // Dynamic patch to only the affected subtree and relays out if it changed size.
        runtime.request_refresh();
        runtime.platform.request_redraw();
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
    }
    if runtime.renderer.advance_text_caret_animation(now) {
        tracing::debug!("wake cause: text caret animation");
        runtime.renderer.request_redraw();
        runtime.platform.request_redraw();
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
    }
    if runtime.renderer.take_rebuild_request() {
        tracing::debug!("wake cause: rebuild request");
        runtime.request_refresh();
    }
    let next_deadline = runtime.renderer.next_gesture_deadline();
    if next_deadline.is_some() {
        tracing::debug!(?next_deadline, "wake armed: engine deadline");
    }
    if runtime.mode.is_pending() {
        tracing::debug!("wake cause: frame mode still pending");
        runtime.platform.request_redraw();
        runtime.renderer.migration_counters_mut().host_wakeups += 1;
    }
    next_deadline
}
