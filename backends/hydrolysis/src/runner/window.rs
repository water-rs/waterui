//! Per-window frame driving: the `FrameMode` state machine, `RuntimeWindow`,
//! scene rebuild/refresh/render phases, and input-event dispatch.

use super::*;
use crate::platform::{GpuSurfaceWindow, PresentationSurface as _};
use crate::renderer::material::{Blending, WithinWindowLevel};
#[cfg(any(target_os = "android", test))]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use waterui::theme::color::Background;
use waterui::window::ResolvedWindowBackground;
use waterui_graphics::{Color, color::WorkingColor};

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
        !matches!(self, Self::Idle)
    }

    /// Whether the scheduled frame exists to apply an unapplied semantic
    /// change. `Animate` is scheduled continuation work, not staleness.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) const fn is_unapplied_change(self) -> bool {
        matches!(self, Self::Refresh)
    }
}

pub(super) struct RuntimeWindow<P: PlatformWindow> {
    pub(super) window: Window,
    pub(super) platform: P,
    pub(super) renderer: HydrolysisRenderer,
    pub(super) mode: FrameMode,
    /// The pump is parked: the host reports the window cannot be seen
    /// (minimized, occluded, backgrounded or surface-less). While parked no
    /// frame is produced and no wake is posted — armed work stays armed for
    /// the frame `set_hidden(false)` schedules when visibility returns.
    pub(super) hidden: bool,
    /// Frames this window has presented, counted for the debug-level
    /// `frame presented` log a hidden-window verification reads: while the
    /// pump is parked that line must go quiet.
    pub(super) presented_frames: u64,
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
    /// Subscriptions on every reactive input of the window declaration,
    /// installed once by `new` through `subscribe_window_declaration_signals`
    /// and held for the window's lifetime: `title`, `frame`, `state`,
    /// `level`, `attention`, `style`, `background`, and `resize_increments`,
    /// `min_size` and `max_size` when present. A write while the pump is
    /// parked requests a refresh through the renderer's frame signals, so
    /// the next pump applies it.
    ///
    /// Declared last so the guards drop after `renderer` — the subscriptions
    /// outlive every frame-scoped watch the core holds, the same tail
    /// position the frame-level `lifecycle` teardown takes inside a flush
    /// (water-rs/waterui#1213). Never read again — the `Retain`s exist only
    /// to keep the subscriptions alive.
    _declaration_watches: Vec<Retain>,
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
        // The window's host wake, installed before any subscription can
        // record a request: from here on, a `Binding::set` outside a frame
        // — an executor drain, a platform-view callback, an accessibility
        // action — fires it on the none→some edge and schedules the frame
        // the request needs, by construction.
        renderer.install_host_wake(platform.frame_wake());
        if let Some(handle) = platform.gpu_surface_redraw_handle() {
            renderer.set_host_redraw_handle(handle);
        }
        let declaration_watches = subscribe_window_declaration_signals(&window, &renderer);
        Self {
            window,
            platform,
            renderer,
            mode: FrameMode::Refresh,
            hidden: false,
            presented_frames: 0,
            pointer_position: None,
            render_diagnostics: RenderDiagnostics::new(render_diagnostics_config),
            refresh_rate_hz: None,
            applied_size_limits: None,
            _declaration_watches: declaration_watches,
        }
    }
}

impl<P: PlatformWindow> RuntimeWindow<P> {
    /// Schedules a refresh of the retained window tree on the next pump (the first
    /// pump builds the tree).
    pub(super) const fn request_refresh(&mut self) {
        self.mode = FrameMode::Refresh;
    }

    pub(super) const fn clear_frame_mode(&mut self) {
        self.mode = FrameMode::Idle;
    }

    /// Whether the pump is parked — the host reports the window cannot be
    /// seen (minimized, occluded, backgrounded or surface-less).
    ///
    /// Exercised by the web and Android runners, which gate their frame
    /// loops on it; feature-gated builds without them keep it for them.
    #[allow(dead_code)]
    pub(super) const fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Parks or unparks the pump. Un-hiding arms a refresh — the first
    /// visible frame is rendered from the current state and time — but
    /// posts nothing itself: the platform event that carried the
    /// visibility change wakes the loop, and the mode arm makes
    /// `advance_runtime` schedule the redraw. Posting one here too would
    /// double the restore frame on hosts that deliver a platform redraw
    /// alongside un-hide (X11 `Expose`, macOS `drawRect`, Windows
    /// `WM_PAINT`).
    #[allow(dead_code)] // see is_hidden
    pub(super) fn set_hidden(&mut self, hidden: bool) {
        if self.hidden == hidden {
            return;
        }
        self.hidden = hidden;
        tracing::debug!(hidden, "window pump visibility changed");
        if !hidden {
            self.request_refresh();
        }
    }

    /// Pulls the platform window's occlusion report into the pump state;
    /// hosts call it after delivering an event that may have moved
    /// visibility.
    #[allow(dead_code)] // see is_hidden
    pub(super) fn sync_occlusion(&mut self) {
        self.set_hidden(self.platform.is_occluded());
    }

    /// [`Self::sync_occlusion`] plus the wake the restore frame needs on
    /// hosts whose frame loop never notices an armed [`FrameMode`] on its
    /// own: Android's pump is only reached by a Choreographer post, so an
    /// un-hide there that only armed the mode would leave the last
    /// presented frame up until the next unrelated wake. Hosts whose
    /// platform posts its own restore frame (winit desktops: `WM_PAINT`,
    /// `drawRect`, `Expose`) use [`Self::sync_occlusion`], which arms
    /// without posting — posting there would double the restore frame.
    #[allow(dead_code)] // see is_hidden
    pub(super) fn sync_occlusion_and_post_restore(&mut self) {
        let was_hidden = self.hidden;
        self.sync_occlusion();
        if was_hidden && !self.hidden {
            self.request_redraw();
        }
    }

    /// Posts the host wake the next frame needs. A hidden window posts no
    /// wakes: the work the wake carried stays armed and applies to the
    /// frame visibility restores.
    pub(super) fn request_redraw(&self) {
        if !self.hidden {
            self.platform.request_redraw();
        }
    }
}

/// Android's frame transaction and the gate shared with its installed wake.
/// Requests made during a transaction join its continuation; occluded requests
/// wait for the restore frame. Winit and web retain their own scheduling rules.
#[cfg(any(target_os = "android", test))]
#[derive(Debug, Default)]
pub(super) struct FrameTransaction {
    open: Cell<bool>,
    /// Written only on the UI thread; atomic because the cross-thread wake's
    /// post carries a share of it onto the UI thread's queue.
    wake_gate: Arc<AtomicBool>,
}

#[cfg(any(target_os = "android", test))]
impl FrameTransaction {
    /// Wraps the host's frame post in the gate: the returned wake posts only
    /// while no transaction is open and the window is not occluded.
    pub(super) fn frame_wake(&self, post: Rc<dyn Fn()>) -> Rc<dyn Fn()> {
        let gate = Arc::clone(&self.wake_gate);
        Rc::new(move || {
            if gate.load(Ordering::Relaxed) {
                post();
            }
        })
    }

    /// [`Self::frame_wake`] for a wake a thread-safe handle queues onto the
    /// UI thread: the returned post may cross threads, but it must run on
    /// the UI thread, where it reads the gate as it stands when it runs — a
    /// post that lands inside a transaction joins its continuation instead
    /// of posting into the frame already running.
    pub(super) fn ui_thread_wake(
        &self,
        post: impl Fn() + Send + Sync + 'static,
    ) -> impl Fn() + Send + Sync + 'static {
        let gate = Arc::clone(&self.wake_gate);
        move || {
            if gate.load(Ordering::Relaxed) {
                post();
            }
        }
    }

    /// Re-derives the gate from the host's occlusion report; an open
    /// transaction keeps it closed.
    pub(super) fn sync_occlusion(&self, occluded: bool) {
        self.wake_gate
            .store(!self.open.get() && !occluded, Ordering::Relaxed);
    }

    /// Opens the transaction: requests raised until [`Self::finish`] post no
    /// wake and count toward its continuation instead.
    pub(super) fn begin(&self) {
        self.open.set(true);
        self.wake_gate.store(false, Ordering::Relaxed);
    }

    /// Whether a request raised now may post to the host's scheduler —
    /// `false` while a transaction is open (the request joins its
    /// continuation instead) and while the window is occluded (it stays
    /// armed for the restore frame).
    #[cfg_attr(
        not(target_os = "android"),
        allow(dead_code, reason = "read by the Android host's request_redraw")
    )]
    pub(super) fn may_post(&self) -> bool {
        self.wake_gate.load(Ordering::Relaxed)
    }

    /// Whether this transaction renders: armed work, a host redraw request
    /// latched before the render, or a redraw-only engine request (a caret
    /// blink). Every one is demand the render itself serves, so the request
    /// flags are consumed here at the render boundary — a request that
    /// survived into the transaction's close would count work this frame
    /// already did toward its continuation (water-rs/waterui#2325). A window
    /// that cannot present consumes nothing — its requests stay armed for
    /// the restore frame.
    pub(super) fn take_render_request<P: PlatformWindow>(
        runtime: &mut RuntimeWindow<P>,
        surface_attached: bool,
        take_redraw_pending: impl FnOnce(&P) -> bool,
    ) -> bool {
        if !surface_attached || runtime.is_hidden() {
            return false;
        }
        let redraw_pending = take_redraw_pending(&runtime.platform);
        let redraw_requested = runtime.renderer.take_redraw_request();
        redraw_pending || runtime.mode.is_pending() || redraw_requested
    }

    /// Closes the transaction, reopens the gate unless occluded, and returns
    /// whether another frame is owed.
    pub(super) fn finish(&self, occluded: bool, demand: FrameDemand) -> bool {
        let next = wants_next_frame(occluded, demand);
        self.open.set(false);
        self.sync_occlusion(occluded);
        next
    }
}

/// Work still pending when Android closes its frame transaction.
#[cfg(any(target_os = "android", test))]
#[derive(Clone, Copy)]
pub(super) struct FrameDemand {
    pub(super) mode: FrameMode,
    pub(super) redraw_pending: bool,
    pub(super) signals_pending: bool,
}

/// A request raised after the pump drained the flags fired no wake while the
/// transaction was open, so it must count toward continuation. Hidden work waits
/// for the restore frame instead.
#[cfg(any(target_os = "android", test))]
const fn wants_next_frame(hidden: bool, demand: FrameDemand) -> bool {
    !hidden && (demand.mode.is_pending() || demand.redraw_pending || demand.signals_pending)
}

/// Whether a frame transaction may report the pump's "first frame presented;
/// ui idle" readiness line: it has presented at least once, and this wake
/// leaves it idle — but never while hidden. A wake that arrives on a parked
/// pump presents nothing, and a present-named readiness line emitted there
/// reads as a frame presented while hidden.
#[allow(dead_code)] // see RuntimeWindow::is_hidden
pub(super) const fn reports_ui_idle(
    presented_once: bool,
    wants_next_frame: bool,
    hidden: bool,
) -> bool {
    presented_once && !wants_next_frame && !hidden
}

/// Applies the window's effective inner-size limits to the platform window:
/// the explicit `Window::min_size`/`max_size` signals when set (read by
/// snapshot — the declaration's lifetime subscriptions on the window are
/// what schedule the frame a change needs). The minimum defaults to the
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
        .as_ref()
        .map(|signal| validated_min_size(signal.snapshot()));
    let explicit_max = runtime
        .window
        .max_size
        .as_ref()
        .map(|signal| validated_max_size(signal.snapshot()));
    let min = match explicit_min {
        Some(min) => Some(min),
        None => runtime.renderer.measure_content_minimum(env),
    };
    let max = explicit_max;
    // A limit apply never moves the window onto the content's size — installing
    // or re-installing limits only constrains the sizes it can take. The size
    // the user settled on survives a re-measure: the window is clamped into
    // the new limits only on the axes whose applied limits themselves changed
    // and that fall outside them. The first apply installs limits on the
    // geometry the window was created with, untouched — with nothing applied
    // before, no axis has changed yet.
    let limits = (min, max);
    let moved = axes_whose_limits_changed(runtime.applied_size_limits.replace(limits), limits);
    runtime.platform.set_size_limits(min, max);
    if moved != (false, false) {
        let frame = crate::platform::validated_window_frame(runtime.window.frame.snapshot());
        let size = *frame.size();
        let clamped = clamp_window_size(size, min, max, moved);
        if clamped != size {
            runtime
                .window
                .frame
                .set(waterui_core::layout::Rect::new(frame.origin(), clamped));
        }
    }
}

/// The axes whose applied limits differ between `previous` and `next` — the
/// only axes a re-apply may clamp, so a width-only move never snaps the
/// height. `None` means the first apply: limits install on the created
/// geometry untouched, so a launch frame below the content minimum keeps its
/// size until that axis's own limit changes, exactly as a re-apply with
/// unchanged limits leaves it.
pub(super) fn axes_whose_limits_changed(
    previous: Option<(
        Option<waterui_core::layout::Size>,
        Option<waterui_core::layout::Size>,
    )>,
    next: (
        Option<waterui_core::layout::Size>,
        Option<waterui_core::layout::Size>,
    ),
) -> (bool, bool) {
    let Some((previous_min, previous_max)) = previous else {
        return (false, false);
    };
    let (min, max) = next;
    (
        previous_min.map(|size| size.width) != min.map(|size| size.width)
            || previous_max.map(|size| size.width) != max.map(|size| size.width),
        previous_min.map(|size| size.height) != min.map(|size| size.height)
            || previous_max.map(|size| size.height) != max.map(|size| size.height),
    )
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

/// Clamps a window size into the new limits, axis by axis — but only on
/// `moved`, the axes whose applied limits changed: an axis whose limits did
/// not move keeps its size even outside them, so a launch geometry below
/// the content minimum stays the user's until that axis's own limit moves.
/// A size inside the limits passes through untouched — a re-measure keeps
/// the size the user set — and an out-of-bounds axis moves to the nearer
/// bound.
pub(super) fn clamp_window_size(
    size: waterui_core::layout::Size,
    min: Option<waterui_core::layout::Size>,
    max: Option<waterui_core::layout::Size>,
    moved: (bool, bool),
) -> waterui_core::layout::Size {
    waterui_core::layout::Size::new(
        if moved.0 {
            clamp_axis(size.width, min.map(|s| s.width), max.map(|s| s.width))
        } else {
            size.width
        },
        if moved.1 {
            clamp_axis(size.height, min.map(|s| s.height), max.map(|s| s.height))
        } else {
            size.height
        },
    )
}

/// Every input is already validated by then: a `+∞` max component is the
/// app's explicit per-axis unbounded, passing through as the high bound and
/// leaving the axis uncapped.
fn clamp_axis(value: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    let lo = min.unwrap_or(0.0);
    value.clamp(lo, max.unwrap_or(f32::INFINITY).max(lo))
}

pub(super) const fn schedule_animation_update<P: PlatformWindow>(
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
/// A captured headless frame: raw pixels, the alpha convention the
/// producing surface reports, plus dimensions.
pub struct HeadlessSnapshot {
    /// Snapshot width in pixels.
    pub width: u32,
    /// Snapshot height in pixels.
    pub height: u32,
    /// Raw RGBA8 pixel data, `width * height * 4` bytes, top-left origin,
    /// in the alpha convention [`Self::output_alpha`] reports.
    pub rgba8: Vec<u8>,
    /// The alpha convention the producing surface presented with — the
    /// convention `rgba8` reads, not a comment's claim about it.
    pub output_alpha: cherenkov_gpu::interop::OutputAlpha,
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
    /// Time spent building the root `WaterUI` view value during scene rebuild.
    pub build_content: Duration,
    /// Time spent dispatching `WaterUI` views into Hydrolysis scene/layout state.
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
    /// Number of recorded scene segment layers submitted for this frame.
    pub scene_segment_layers: u32,
    /// Number of mounted `GpuContentView` layers submitted for this frame.
    pub gpu_content_layers: u32,
    /// Number of clip scopes pushed while building this frame.
    pub clip_layers: u32,
    /// Maximum nested clip depth while building this frame.
    pub max_clip_depth: u32,
    /// Number of `FilteredView` mounts submitted in this frame.
    pub filtered_layers: u32,
    /// Filter effects that encoded in this frame.
    pub applied_filter_count: u32,
    /// Filtered-subtree capture time in this frame, in microseconds: `0` —
    /// the engine captures inside the same render pass as the effect encode
    /// and reports no per-phase split.
    pub applied_filter_capture_us: u64,
    /// Filter effect encode time in this frame, in microseconds.
    pub applied_filter_effect_us: u64,
    /// Whether this frame rendered to the target.
    pub rendered: bool,
    /// Whether this frame captured a CPU snapshot.
    pub captured_snapshot: bool,
    /// Per-frame work counters for this frame (water-rs/hydrolysis#205):
    /// semantic builds, patches, layout/measure traffic, recorded content and
    /// GPU submissions — the numbers the fine-grained frame model is measured
    /// by.
    pub frame_work: crate::renderer::FrameWorkCounters,
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
    pub(super) const fn with_total(mut self, total: Duration) -> Self {
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
    runtime.request_refresh();
    runtime.request_redraw();
    runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
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

/// How the presentation path realizes a window's resolved background: whether
/// the window is transparent, the colour the surface clears to, and the
/// within-window material the root is mounted over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SurfaceBackground(ResolvedWindowBackground);

impl SurfaceBackground {
    /// `window`'s background as it resolves now.
    pub(super) fn of(window: &Window, env: &Environment) -> Self {
        Self(window.resolved_background(env).snapshot())
    }

    /// Whether the window must be transparent: a colour with alpha, or a
    /// behind-window material, whatever its tint — the level's blending
    /// alone decides, so no colour scheme is consulted.
    pub(super) fn transparent(self) -> bool {
        match self.0 {
            ResolvedWindowBackground::Color(color) => color.components[3] < 1.0,
            ResolvedWindowBackground::Material(material) => {
                matches!(Blending::of(material), Blending::BehindWindow(_))
            }
        }
    }

    /// Whether the window asks the compositor to blur what lies behind it:
    /// exactly a behind-window material — false for every colour and every
    /// within-window level.
    pub(super) const fn blurs_behind(self) -> bool {
        match self.0 {
            ResolvedWindowBackground::Color(_) => false,
            ResolvedWindowBackground::Material(material) => {
                matches!(Blending::of(material), Blending::BehindWindow(_))
            }
        }
    }

    /// The within-window material the window's root is mounted over.
    pub(super) const fn backdrop(self) -> Option<WithinWindowLevel> {
        match self.0 {
            ResolvedWindowBackground::Material(material) => match Blending::of(material) {
                Blending::WithinWindow(level) => Some(level),
                Blending::BehindWindow(_) => None,
            },
            ResolvedWindowBackground::Color(_) => None,
        }
    }

    /// The colour the surface clears to, straight alpha in encoded sRGB.
    ///
    /// - A colour is the clear colour; `Opaque` resolved it to the theme
    ///   background.
    /// - A within-window material keeps the surface opaque, cleared to the
    ///   theme background, which the material's backdrop treatment covers.
    /// - A behind-window material clears the transparent surface to the
    ///   level's [`Tint`](crate::renderer::material::Tint) in `env`'s colour
    ///   scheme under the content: clearing to the tint is compositing it
    ///   source-over onto a cleared-transparent surface, which leaves it
    ///   unchanged. The compositor then blends the window over the desktop
    ///   it blurs.
    pub(super) fn clear(self, env: &Environment) -> peniko::Color {
        let srgb = |color: WorkingColor| {
            let srgb = waterui_graphics::color::working::to_srgb(color);
            peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.components[3]])
        };
        match self.0 {
            ResolvedWindowBackground::Color(color) => srgb(color),
            ResolvedWindowBackground::Material(material) => match Blending::of(material) {
                Blending::WithinWindow(_) => srgb(Color::new(Background).resolve(env).snapshot()),
                Blending::BehindWindow(level) => level
                    .tint(waterui::theme::current_color_scheme(env).snapshot())
                    .srgb(),
            },
        }
    }
}

/// Realizes the window's reactive background for the frame about to be
/// flushed and painted: hands the platform whether the window must be
/// transparent — the composite alpha mode and the native window's
/// transparency follow a switch between opaque and translucent — hands the
/// renderer the within-window material the root mounts over, and returns the
/// clear colour. This is the one place the background reaches the
/// presentation path.
pub(super) fn apply_window_background<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> peniko::Color {
    let background = SurfaceBackground::of(&runtime.window, env);
    runtime.platform.set_transparent(background.transparent());
    runtime.platform.set_blur_behind(background.blurs_behind());
    runtime.renderer.set_window_backdrop(background.backdrop());
    background.clear(env)
}

#[cfg(hydrolysis_winit)]
pub fn window_requires_transparency(window: &Window, env: &Environment) -> bool {
    SurfaceBackground::of(window, env).transparent()
}

crate::engine::cfg_async_fn! {
    /// Runs one frame and reports whether it was presented to the surface — an
    /// idle frame, or one whose surface had to be reconfigured, is not. The
    /// frame renders through the window's presentation kind
    /// ([`GpuSurfaceFrame`]).
    ///
    /// Async on wasm32, where the engine render inside awaits the browser device.
    pub(super) fn render_window<P: GpuSurfaceWindow>(
        runtime: &mut RuntimeWindow<P>,
        env: &Environment,
        drain_local_tasks: &mut dyn FnMut() -> bool,
    ) -> bool {

    // A hidden window produces no frame: nothing is encoded, submitted or
    // presented, and a redraw already in flight when it hid is stale —
    // dropped here rather than rendered.
    if runtime.hidden {
        return false;
    }

    let result = crate::engine::engine_await!(render_window_pumped(
        runtime,
        env,
        FrameReader::Display,
        drain_local_tasks,
        #[cfg(not(target_arch = "wasm32"))]
        GpuSurfaceFrame::render_frame,
        #[cfg(target_arch = "wasm32")]
        async |surface: &mut P::Presentation, renderer, clear_color, display_scale| {
            surface
                .render_frame(renderer, clear_color, display_scale)
                .await
        },
    ));
    // The rebuild flag and the snapshot belong to the headless harness; a live
    // window only asks whether the frame reached its surface.
    let _ = (result.rebuilt, result.snapshot);
    let rendered = result.profile.counters.rendered;
    if rendered {
        runtime.presented_frames += 1;
        tracing::debug!(frames = runtime.presented_frames, "frame presented");
    }
    rendered
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
    // The capture records the presentation hosts too — an open text
    // context menu or `.context_menu` presentation builds and places its
    // sub-views there, under its host cell.
    runtime.renderer.capture_window_tree(
        content,
        env,
        bounds,
        root_transform,
        kurbo::Affine::IDENTITY,
    );
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

    let pump_started_at = Instant::now();
    let mut phases = FramePhases::default();
    let animations_active = runtime.renderer.advance_animations();
    schedule_animation_update(runtime, animations_active);

    // Producer wakes posted since the last frame mark their owners; the
    // marks that brought this pump here then decide the work.
    runtime.renderer.drain_producer_wakes();
    // Structural marks raised since the last flush land on the root cell
    // through the owner chain and still arm the full refresh.
    if runtime.renderer.has_structure_marks() {
        runtime.request_refresh();
    }

    let mut built = false;
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
            runtime.clear_frame_mode();
            // Anything the build itself flagged as needing another pass — a hover
            // change under the pointer, a renderer-side structural request raised
            // mid-build — is satisfied by refreshing the freshly built tree in the
            // same frame.
            let mut refresh_after_build = false;
            if let Some((x, y)) = runtime.pointer_position
                && runtime.renderer.sync_pointer_hover_state(x, y, env)
            {
                if runtime.renderer.has_structure_marks() {
                    refresh_after_build = true;
                } else {
                    runtime.renderer.request_redraw();
                }
            }
            if runtime.renderer.has_structure_marks() {
                refresh_after_build = true;
            }
            if refresh_after_build {
                refresh_window_scene(runtime, env, &mut phases);
            }
        }
        FrameMode::Refresh | FrameMode::Animate => {
            refresh_window_scene(runtime, env, &mut phases);
            runtime.clear_frame_mode();
        }
    }
    // Every presented frame reports its platform-view set — an Idle pump
    // re-presenting the retained layers included: the host publishes once
    // per encoded frame, and a frame whose `current` is empty reads as
    // "no views". The record is idempotent, so the build and refresh arms
    // running their own frame-end step cost nothing here.
    runtime.renderer.registries();
    runtime.renderer.record_platform_views();
    if runtime.renderer.has_structure_marks() {
        // A structural mark raised mid-flush — an effect needs another frame.
        runtime.request_refresh();
        runtime.request_redraw();
        runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    } else if runtime.renderer.animations_active() && !runtime.mode.is_pending() {
        schedule_animation_update(runtime, true);
        runtime.request_redraw();
        runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    }
    phases.rebuild = pump_started_at.elapsed();
    ScenePumpOutcome { built, phases }
}

/// What one scene pump did: whether the retained tree was built for the first
/// time. An idle pump leaves it false.
pub(super) struct ScenePumpOutcome {
    pub(super) built: bool,
    pub(super) phases: FramePhases,
}

#[cfg(any(test, all(not(target_arch = "wasm32"), hydrolysis_winit)))]
pub(super) fn pump_window_semantics<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> bool {
    // The declaration's reactive inputs — `title`, `frame`, `state`,
    // `level`, `attention`, `style`, `background`, and `resize_increments`,
    // `min_size`/`max_size` when present — are subscribed once on the
    // `RuntimeWindow` for its lifetime; the pump only consumes the values.
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(hydrolysis_winit)]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.snapshot().as_str());

    if runtime.renderer.has_structure_marks() {
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
    sync_platform_input(runtime);
    if runtime.renderer.take_redraw_request() {
        runtime.request_redraw();
        runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    }
    rebuilt
}

pub struct SurfaceRenderResult {
    acquire: Duration,
    render: Duration,
    present: Duration,
    snapshot: Option<HeadlessSnapshot>,
}

/// How a window's presentation kind renders one frame and gets it to the
/// display. Host-acquired surfaces ([`SurfaceProvider`]) acquire, copy the
/// engine output in and present; the macOS winit window's and the browser
/// page's engine targets present inside `Engine::render`, so they have no
/// acquire, copy or present to call.
///
/// [`SurfaceProvider`]: crate::platform::SurfaceProvider
pub trait GpuSurfaceFrame {
    /// Renders and presents one frame into the surface, reporting the
    /// stage durations.
    #[cfg(not(target_arch = "wasm32"))]
    fn render_frame(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError>;

    /// [`GpuSurfaceFrame::render_frame`], async on wasm32 where the browser
    /// device renders inside an await.
    #[cfg(target_arch = "wasm32")]
    fn render_frame(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> impl Future<Output = Result<SurfaceRenderResult, crate::platform::SurfaceError>>;
}

/// Every host-acquired surface renders through [`render_host_acquired_frame`];
/// a live window reads no snapshot.
impl<S: crate::platform::SurfaceProvider> GpuSurfaceFrame for S {
    #[cfg(not(target_arch = "wasm32"))]
    fn render_frame(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
        render_host_acquired_frame(renderer, self, clear_color, display_scale, false)
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    async fn render_frame(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
        render_host_acquired_frame(renderer, self, clear_color, display_scale, false).await
    }
}

crate::engine::cfg_async_fn! {
    /// The host-acquired frame: the engine renders the scene into its
    /// retained texture, then the frame is acquired, the engine output
    /// copied in, and the frame presented. `capture_snapshot` asks for a
    /// GPU readback of the acquired frame — headless targets only; a live
    /// window passes `false`.
    ///
    /// Async on wasm32, where the engine render inside awaits the browser
    /// device.
    pub fn render_host_acquired_frame {
        renderer: &mut HydrolysisRenderer,
        surface: &mut dyn crate::platform::SurfaceProvider,
        clear_color: peniko::Color,
        display_scale: f64,
        capture_snapshot: bool,
    } {
        renderer: &mut HydrolysisRenderer,
        surface: &mut dyn crate::platform::SurfaceProvider,
        clear_color: peniko::Color,
        display_scale: f64,
        capture_snapshot: bool,
    } -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
    let (width, height) = surface.size();
    let format = surface.format();
    let output_alpha = surface.output_alpha();
    let context = surface.device_loss().gpu_context();
    // The engine renders into its own retained output before the swapchain
    // image is acquired: the render awaits the GPU device on wasm32, and a
    // browser expires a canvas texture when the task that acquired it ends,
    // so the image must be acquired, filled and presented without an await
    // in between.
    let render_started_at = Instant::now();
    let engine_frame = crate::engine::engine_await!(renderer.render_texture_frame(
        crate::renderer::FrameRenderTarget {
            adapter: surface.adapter(),
            device: surface.device(),
            queue: surface.queue(),
            device_loss: surface.device_loss().clone(),
            gpu_context_id: context.context_id,
            shared_device: context.shared_device,
            display_scale,
            width,
            height,
            base_color: crate::renderer::working_color(clear_color),
        },
        format,
        surface.display_headroom(),
    ))
    .unwrap_or_else(|error| {
        panic!("hydrolysis renderer: engine render failed: {error:#}")
    });
    let engine_render = render_started_at.elapsed();
    let acquire_started_at = Instant::now();
    let frame = acquire_surface_frame(surface)?;
    let acquire = acquire_started_at.elapsed();
    let copy_started_at = Instant::now();
    renderer.present_engine_frame(
        engine_frame,
        surface.device(),
        surface.queue(),
        frame.texture(),
        surface.output_color(),
        output_alpha,
    );
    let render = engine_render + copy_started_at.elapsed();
    #[cfg(feature = "frame-profile")]
    {
        // The timestamp resolve blocks until the frame's submits finish — the
        // headless frame's "present wait", kept separate from the CPU submit
        // time `render` measures.
        renderer.finish_gpu_frame_profile(context.context_id, surface.device(), surface.queue());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let snapshot = {
        #[cfg(feature = "frame-profile")]
        let readback_started_at = Instant::now();
        let snapshot = capture_snapshot.then(|| {
            renderer.frame_work_counters_mut().gpu_submissions += 1;
            HeadlessSnapshot {
                width,
                height,
                rgba8: readback_surface_texture_rgba8(&*surface, frame.texture(), width, height)
                    .unwrap_or_else(|error| {
                        panic!(
                            "hydrolysis headless snapshot readback failed: {:#}",
                            waterui_core::Error::from(error)
                        )
                    }),
                output_alpha,
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
    Ok(SurfaceRenderResult {
        acquire,
        render,
        present,
        snapshot,
    })
    }
}

/// The macOS winit window's frame: `Engine::render` is the whole
/// presentation — the engine acquires the drawable and presents inside it,
/// so there is no host acquire, copy or present. `acquire` and `present`
/// report zero because no such calls run; the drawable wait and the layer
/// presents are inside `render`. No snapshot can be asked of this kind: the
/// capture path is bound on [`SurfaceProvider`], which it does not
/// implement.
///
/// [`SurfaceProvider`]: crate::platform::SurfaceProvider
#[cfg(all(target_os = "macos", hydrolysis_winit))]
impl GpuSurfaceFrame for crate::platform::WinitSurface {
    fn render_frame(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
        let render_started_at = Instant::now();
        let (target, engine_target) =
            self.render_targets(display_scale, crate::renderer::working_color(clear_color));
        renderer
            .render_window_frame(&target, engine_target)
            .unwrap_or_else(|error| panic!("hydrolysis renderer: engine render failed: {error:#}"));
        Ok(SurfaceRenderResult {
            acquire: Duration::ZERO,
            render: render_started_at.elapsed(),
            present: Duration::ZERO,
            snapshot: None,
        })
    }
}

/// The browser page's frame: `Engine::render` is the whole presentation —
/// the engine presents its canvases and places the page's hosted elements
/// inside it, so there is no host acquire, copy or present, and `acquire`
/// and `present` report zero. No snapshot can be asked of this kind: a
/// browser surface is never read back.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
impl GpuSurfaceFrame for crate::platform::BrowserSurface {
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    async fn render_frame(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
        let render_started_at = Instant::now();
        let (target, root, window_slot) =
            self.render_targets(display_scale, crate::renderer::working_color(clear_color));
        renderer
            .render_dom_frame(&target, root, window_slot)
            .await
            .unwrap_or_else(|error| panic!("hydrolysis renderer: engine render failed: {error:#}"));
        Ok(SurfaceRenderResult {
            acquire: Duration::ZERO,
            render: render_started_at.elapsed(),
            present: Duration::ZERO,
            snapshot: None,
        })
    }
}

/// Who reads the pixels of a frame the window pump renders.
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
}

/// Runs one frame of a host-acquired window for `reader`: a
/// [`FrameReader::Snapshot`] reads the acquired frame back. Bound on
/// [`SurfaceProvider`](crate::platform::SurfaceProvider), because only an
/// acquired frame can be read back — an engine-presented window has none,
/// so a snapshot of one does not compile.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn render_window_with_capture<P: GpuSurfaceWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    reader: FrameReader,
    drain_local_tasks: &mut dyn FnMut() -> bool,
) -> RenderWindowResult
where
    P::Presentation: crate::platform::SurfaceProvider,
{
    render_window_pumped(
        runtime,
        env,
        reader,
        drain_local_tasks,
        |surface: &mut P::Presentation, renderer, clear_color, display_scale| {
            render_host_acquired_frame(
                renderer,
                surface,
                clear_color,
                display_scale,
                reader.captures(),
            )
        },
    )
}

crate::engine::cfg_async_fn! {
    /// The frame pump [`render_window`] and [`render_window_with_capture`]
    /// share: `render_frame` renders the frame through the window's surface,
    /// and `reader` says who reads its pixels.
    ///
    /// Async on wasm32, where the surface render inside awaits the browser
    /// device.
    // the pump renders, encodes and delivers in one pass; the sequence is the feature
    #[allow(clippy::too_many_lines)]
    fn render_window_pumped<P: GpuSurfaceWindow> {
        runtime: &mut RuntimeWindow<P>,
        env: &Environment,
        reader: FrameReader,
        drain_local_tasks: &mut dyn FnMut() -> bool,
        render_frame: impl FnOnce(
            &mut P::Presentation,
            &mut HydrolysisRenderer,
            peniko::Color,
            f64,
        ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError>,
    } {
        runtime: &mut RuntimeWindow<P>,
        env: &Environment,
        reader: FrameReader,
        drain_local_tasks: &mut dyn FnMut() -> bool,
        render_frame: impl AsyncFnOnce(
            &mut P::Presentation,
            &mut HydrolysisRenderer,
            peniko::Color,
            f64,
        ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError>,
    } -> RenderWindowResult {
    let capture_snapshot = reader.captures();
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(hydrolysis_winit)]
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
        // The background applies before the pump: the flush mounts the
        // root over a within-window material backdrop.
        let clear_color = apply_window_background(runtime, env);
        let pump_outcome = pump_window_scene(runtime, env, drain_local_tasks);
        let rebuild_phases = pump_outcome.phases;
        rebuilt |= pump_outcome.built;
        apply_window_size_limits(runtime, env);

        let render_result = {
            let scale_factor = runtime.platform.scale_factor();
            crate::engine::engine_await!(render_frame(
                runtime.platform.surface(),
                &mut runtime.renderer,
                clear_color,
                scale_factor,
            ))
        };

        let rendered = match render_result {
            Ok(rendered) => rendered,
            Err(
                crate::platform::SurfaceError::Lost
                | crate::platform::SurfaceError::Outdated
                | crate::platform::SurfaceError::Timeout
                | crate::platform::SurfaceError::Occluded,
            ) => {
                runtime.request_refresh();
                runtime.request_redraw();
                runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
                let (measurement_cache_hits, measurement_cache_misses) =
                    runtime.renderer.measurement_cache_stats();
                let mount_stats = runtime.renderer.mount_stats();
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
                            scene_layers: mount_stats.scene_layers,
                            scene_segment_layers: mount_stats.scene_segments,
                            gpu_content_layers: mount_stats.gpu_content,
                            filtered_layers: mount_stats.filtered,
                            clip_layers: mount_stats.clip_layers,
                            max_clip_depth: mount_stats.max_clip_depth,
                            applied_filter_count,
                            applied_filter_capture_us,
                            applied_filter_effect_us,
                            rendered: false,
                            captured_snapshot: false,
                            frame_work: runtime.renderer.frame_work_counters(),
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
        let (measurement_cache_hits, measurement_cache_misses) =
            runtime.renderer.measurement_cache_stats();
        let mount_stats = runtime.renderer.mount_stats();
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
                scene_layers: mount_stats.scene_layers,
                scene_segment_layers: mount_stats.scene_segments,
                gpu_content_layers: mount_stats.gpu_content,
                filtered_layers: mount_stats.filtered,
                clip_layers: mount_stats.clip_layers,
                max_clip_depth: mount_stats.max_clip_depth,
                applied_filter_count,
                applied_filter_capture_us,
                applied_filter_effect_us,
                rendered: true,
                captured_snapshot: capture_snapshot,
                frame_work: runtime.renderer.frame_work_counters(),
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
                    filtered_layers: mount_stats.filtered,
                    rebuilt: pump_outcome.built,
                },
            );
        }
    }

    sync_platform_input(runtime);
    if runtime.renderer.take_redraw_request() {
        runtime.request_redraw();
        runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    }
    // The engine's own scheduling answer: an in-flight animation asks for its
    // next frame through `Next::At` (the surface's wake may already have woken
    // the host too — the request is idempotent).
    if matches!(
        runtime.renderer.take_engine_next(),
        Some(cherenkov::Next::At { .. })
    ) {
        runtime.platform.request_redraw();
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
        profile,
        #[cfg(feature = "frame-profile")]
        stages,
    }
    }
}

pub(super) fn physical_to_logical_dimension(value: u32, scale_factor: f64) -> f32 {
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis runner: invalid scale factor {scale_factor}"
    );
    crate::num_cast::f64_as_f32(f64::from(value) / scale_factor)
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
        runtime.renderer.has_patch_request() || runtime.renderer.has_structure_marks();
    if !geometry_pending || !runtime.renderer.has_render_tree() {
        return;
    }

    runtime.request_refresh();
    let scale_factor = runtime.platform.scale_factor();
    let (width, height) = runtime.platform.surface().size();
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

/// Seeds the pointer position an OS file event conceptually arrives at:
/// the host's live answer when it can give one.
///
/// winit's `HoveredFile`/`DroppedFile` carry no coordinates, and platforms
/// that suppress cursor events while an external drag owns the pointer
/// leave the stream's last position stale or unset — the drop then lands
/// on a stale point or is discarded outright (water-rs/hydrolysis#127).
/// Asking the platform where the pointer actually is before dispatching a
/// file event restores the position winit withheld.
fn sync_os_pointer_position<P: GpuSurfaceWindow>(runtime: &mut RuntimeWindow<P>) {
    let Some((x, y)) = runtime.platform.pointer_position() else {
        return;
    };
    runtime.pointer_position = Some((x, y));
    runtime.renderer.note_pointer_position(x, y);
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
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
    // The platform's touch-gesture parameters ride the event drain: a host
    // update (metrics, configuration) reaches the gesture before the
    // events it applies to.
    runtime
        .renderer
        .set_touch_scroll_config(runtime.platform.touch_scroll_config());
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
        // Hosted content holding platform focus receives its own key and
        // IME delivery; none of it reaches Hydrolysis's dispatch.
        if runtime.renderer.sync_hosted_focus()
            && matches!(
                event,
                InputEvent::Key { .. }
                    | InputEvent::TextInput { .. }
                    | InputEvent::KeyText { .. }
                    | InputEvent::ImePreedit { .. }
                    | InputEvent::ImeCommit { .. }
                    | InputEvent::ImeDisabled
            )
        {
            continue;
        }
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
                // The one close path every request takes — the title-bar
                // button, which X11 and Wayland keep enabled whatever
                // `closable` says, a window-manager close and a `WM_CLOSE`
                // sent straight to a Windows window: a non-closable window
                // ignores them all.
                if runtime.window.closable {
                    runtime
                        .window
                        .state
                        .set(waterui::window::WindowState::Closed);
                    should_close = true;
                }
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
                runtime.request_redraw();
                runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
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
            InputEvent::Maximized(maximized) => {
                // Chrome-driven maximize/restore reached the window server
                // directly; write it back so `Window::state` tracks the real
                // window. Only Normal/Maximized are touched — a minimized or
                // fullscreen window's state is not overridden by the flag.
                let state = runtime.window.state.snapshot();
                let next = if maximized {
                    waterui::window::WindowState::Maximized
                } else {
                    waterui::window::WindowState::Normal
                };
                if matches!(
                    state,
                    waterui::window::WindowState::Normal | waterui::window::WindowState::Maximized
                ) && state != next
                {
                    runtime.window.state.set(next);
                }
            }
            InputEvent::Focused(focused) => {
                // The window gained focus: any outstanding attention request
                // is spent — the contract hands the binding back as `None`.
                if focused && runtime.window.attention.snapshot().is_some() {
                    runtime.window.attention.set(None);
                }
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
                sync_os_pointer_position(runtime);
                let event_env = input_env(runtime, env);
                let changed = runtime.renderer.handle_file_hovered(path, &event_env);
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::FileDropped { path } => {
                // The file joins the drag's collected list on the renderer;
                // delivery is deferred to `finish_os_file_drop` below —
                // winit reports one event per file and a drop's files can
                // outlive a single batch.
                sync_os_pointer_position(runtime);
                runtime.renderer.handle_file_dropped(path);
            }
            InputEvent::FileHoverCancelled => {
                let event_env = input_env(runtime, env);
                let changed = runtime.renderer.handle_file_hover_cancelled(&event_env);
                schedule_redraw_or_refresh(runtime, changed);
            }
            InputEvent::BackNavigation(navigation) => {
                let event_env = input_env(runtime, env);
                let changed = runtime
                    .renderer
                    .handle_back_navigation(navigation, &event_env);
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "back_navigation",
                    ?navigation,
                    changed,
                    "runner dispatched input event"
                );
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
            runtime.request_redraw();
            runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
        }
        schedule_redraw_or_refresh(runtime, changed);
    }
    sync_platform_input(runtime);
    should_close
}

/// Settles focus ownership between Hydrolysis and hosted content, then pushes
/// the focused text input's state to the platform IME.
fn sync_platform_focus<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>) {
    runtime.renderer.sync_hosted_focus();
    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
}

/// [`sync_platform_focus`], then the cursor under the pointer — left to the
/// hosted view wherever uncovered hosted content is under it.
fn sync_platform_input<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>) {
    sync_platform_focus(runtime);
    if let Some((x, y)) = runtime.pointer_position
        && !runtime.renderer.hosted_owns_cursor(x, y)
    {
        runtime
            .platform
            .set_cursor_style(runtime.renderer.cursor_style_at(x, y));
    }
}

pub(super) fn advance_runtime<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    now: Instant,
) -> Option<Instant> {
    runtime.renderer.set_frame_instant(now);
    // A hidden window does no rendering work — no ticks, no wakes, no
    // GPU-content pulls. Armed work stays armed: patch, rebuild and
    // animation requests pending in the renderer apply to the frame
    // `set_hidden(false)` schedules on un-hide.
    if runtime.hidden {
        return None;
    }
    // Track the display refresh rate so the diagnostics slow-frame threshold reflects the
    // real frame budget (e.g. 8.33ms on a 120Hz panel) instead of a hardcoded 60fps.
    let refresh_rate = runtime.platform.refresh_rate_hz();
    if refresh_rate != runtime.refresh_rate_hz {
        runtime.refresh_rate_hz = refresh_rate;
        if let Some(hz) = refresh_rate {
            runtime.render_diagnostics.set_refresh_rate(hz);
        }
    }
    sync_platform_focus(runtime);
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
    // A touch fling decelerates the same way — offsets advance per frame
    // until the spline settles or a touch down has stopped it.
    if runtime.renderer.tick_touch_scroll(now) {
        tracing::debug!("wake cause: touch fling still gliding");
        runtime.request_refresh();
    }
    let animations_active = runtime.renderer.advance_animations();
    if animations_active {
        tracing::debug!("wake cause: animations active");
    }
    schedule_animation_update(runtime, animations_active);
    // Producer wakes posted since the last frame mark their owners; the
    // marks that brought this pump here then decide the work.
    runtime.renderer.drain_producer_wakes();
    // Marks raised since the last flush — reactive updates, structural
    // patches, widget signals — land on the root cell; any mark still arms
    // the full refresh.
    if runtime.renderer.take_patch_request() {
        tracing::debug!("wake cause: dirty mark");
        runtime.request_refresh();
        runtime.request_redraw();
        runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    }
    if runtime.renderer.advance_text_caret_animation(now) {
        tracing::debug!("wake cause: text caret animation");
        runtime.renderer.request_redraw();
        runtime.request_redraw();
        runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    }
    if runtime.renderer.has_structure_marks() {
        tracing::debug!("wake cause: structural mark");
        runtime.request_refresh();
    }
    let next_deadline = runtime.renderer.next_gesture_deadline();
    if next_deadline.is_some() {
        tracing::debug!(?next_deadline, "wake armed: engine deadline");
    }
    if runtime.mode.is_pending() {
        tracing::debug!("wake cause: frame mode still pending");
        runtime.request_redraw();
        runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
    }
    next_deadline
}
