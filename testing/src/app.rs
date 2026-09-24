use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, TreeId as AccessibilityTreeId,
};
use hydrolysis::{HeadlessRuntime, KeyCode, Modifiers, SemanticRuntime, Style};
use waterui::app::App;
use waterui::{Plugin, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment, View};

use crate::artifacts::{CapturedSnapshot, TestArtifacts};
use crate::driver::{
    self, DriverPumpResult, FrameTiming, ResourceSampler, RuntimeDriver, VIRTUAL_FRAME,
};
use crate::perf::{PerfApp, PerfConfig, PerfReport};
use crate::query::Query;
use crate::selector::{ElementAnchor, ElementRef, ElementSet, Selector};
use crate::semantics::{NodeId, TreeSnapshot};
use crate::snapshot::Snapshot;
use crate::wait::{Expectation, ExpectationKind, WaitOptions, WaitResult};

/// The style state of a [`UiBuilder`] that carries none: `ui()`'s starting
/// point. A `UiBuilder<NoStyle>` mounts only the semantic runtime — geometry,
/// pointer and capture belong to the styled builder.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoStyle;

/// The style state of a [`UiBuilder`] that carries a `hydrolysis` [`Style`].
///
/// The styled builder's mounts stay semantic — `mount` runs the same
/// GPU-free [`SemanticRuntime`], only with the style's tokens installed into
/// the mounted view's environment — while `mount_offscreen`, `mount_app` and
/// the `perf` entry points hand the style to the rendered headless runtime.
#[derive(Clone, Debug)]
pub struct Styled<S: Style> {
    pub(crate) style: S,
}

/// Creates a semantic UI test builder.
///
/// The returned `UiBuilder<NoStyle>` mounts views on `hydrolysis`'s semantic
/// runtime through [`UiBuilder::mount`] — no style package is involved, so a
/// test cannot read geometry, dispatch pointer gestures or capture snapshots.
/// [`UiBuilder::theme`] carries a [`Style`] into the builder and unlocks the
/// rendered mount points.
#[must_use]
pub fn ui() -> UiBuilder {
    UiBuilder::new()
}

/// Mounts a whole [`App`] on the rendered runtime and returns the offscreen
/// session, taking `style` the way `hydrolysis::run(app, style)` does.
///
/// The session mounts at the window's declared frame on the default builder
/// configuration — [`RuntimeFlavor::Test`] and scale factor 1. When the
/// session needs its own viewport, runtime flavor or scale factor, mount
/// through the styled builder instead:
/// `ui().theme(style).viewport(w, h).mount_app(app)`.
///
/// Only the main window's content is mounted: the headless runtime hosts a
/// single window, so the app's menu bar and any additional windows are not
/// mounted. Popup windows the app opens at runtime (context menus, pickers)
/// are still merged into the accessibility tree by Hydrolysis.
///
/// # Panics
///
/// Panics if the app declares no window, or if the initial Hydrolysis
/// offscreen frame does not produce an accessibility tree.
#[must_use]
pub fn mount_app(app: App, style: impl Style) -> OffscreenApp {
    let size = *app.main_window().frame.get().size();
    ui().theme(style)
        .viewport(
            frame_points_as_u32(size.width),
            frame_points_as_u32(size.height),
        )
        .mount_app(app)
}

/// Converts a window's declared frame extent — positive logical points —
/// into the pixel count the headless runtime takes.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a window frame is a small positive logical size; the clamp keeps the value inside u32 range"
)]
fn frame_points_as_u32(points: f32) -> u32 {
    u32::try_from(points.max(1.0).round() as i32).expect("clamped frame size is positive")
}

/// Which Hydrolysis headless runtime backs a session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RuntimeFlavor {
    /// Deterministic bundled fonts, software adapters permitted (the default for tests).
    #[default]
    Test,
    /// The application's resource fonts and production adapter selection — what `water run` shows.
    Application,
}

/// Runtime test host and configuration.
///
/// `S` is the builder's style state: [`NoStyle`] for the semantic builder
/// `ui()` returns, or [`Styled<S>`] once [`UiBuilder::theme`] carries a
/// style. Both states mount the GPU-free [`SemanticRuntime`] — the styled
/// builder's `mount` additionally applies `Style::install_tokens` to the
/// mounted view's environment — while `mount_offscreen`, `mount_app` and the
/// performance entry points exist only on the styled builder and run the
/// rendered [`HeadlessRuntime`].
///
/// Framework tokens are installed by the runtimes themselves; the builder's
/// environment carries only what the test installs into it.
#[derive(Clone)]
pub struct UiBuilder<S = NoStyle> {
    env: Environment,
    width: u32,
    height: u32,
    style: S,
    perf_config: PerfConfig,
    flavor: RuntimeFlavor,
    scale_factor: f64,
}

impl<S> core::fmt::Debug for UiBuilder<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiBuilder")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("perf_config", &self.perf_config)
            .field("flavor", &self.flavor)
            .field("scale_factor", &self.scale_factor)
            .finish_non_exhaustive()
    }
}

impl Default for UiBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl UiBuilder<NoStyle> {
    /// Creates a default semantic UI test builder (390x844 viewport).
    #[must_use]
    pub fn new() -> Self {
        Self {
            env: Environment::new(),
            width: 390,
            height: 844,
            style: NoStyle,
            perf_config: PerfConfig::default(),
            flavor: RuntimeFlavor::Test,
            scale_factor: 1.0,
        }
    }

    /// Carries a `hydrolysis` style into the builder.
    ///
    /// The returned `UiBuilder<Styled<S>>` keeps `mount` on the semantic
    /// runtime — still semantic, but with the style's tokens installed into
    /// the mounted view's environment — and unlocks `mount_offscreen`,
    /// `mount_app`, `perf` and `perf_with`, which mount the rendered runtime
    /// constructed with `style`.
    #[must_use]
    pub fn theme<S: Style>(self, style: S) -> UiBuilder<Styled<S>> {
        UiBuilder {
            env: self.env,
            width: self.width,
            height: self.height,
            style: Styled { style },
            perf_config: self.perf_config,
            flavor: self.flavor,
            scale_factor: self.scale_factor,
        }
    }

    /// Mounts a no-arg view builder on the semantic runtime.
    ///
    /// The semantic pipeline carries no style: the accessibility tree is a
    /// product of the view tree and the widgets' semantics, so the returned
    /// [`SemanticApp`] answers role, label, value, description, state,
    /// structure and focus queries and dispatches accessibility actions —
    /// taps, focus, text entry, increment/decrement, scroll, expand/collapse
    /// and key events — without a style package. The mounted view's
    /// environment resolves only the framework default tokens the runtime
    /// installs; a view whose body reads a style package's tokens mounts
    /// through [`UiBuilder::theme`] instead. Geometry, pointer gestures and
    /// capture live on the styled builder's [`UiBuilder::mount_offscreen`].
    ///
    /// # Panics
    ///
    /// Panics if the initial Hydrolysis semantic pass does not produce an
    /// accessibility tree.
    pub fn mount<V, F>(self, view_fn: F) -> SemanticApp
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        self.mount_semantic(AnyViewBuilder::new(move || AnyView::new(view_fn())))
    }
}

impl<S> UiBuilder<S> {
    /// Overrides the environment used by the mounted app.
    ///
    /// The runtime still installs its framework tokens on top at mount time
    /// — and a styled builder's `mount` applies the style's tokens above
    /// them.
    #[must_use]
    pub fn environment(mut self, env: Environment) -> Self {
        self.env = env;
        self
    }

    /// Overrides the logical viewport size used by the mounted app.
    #[must_use]
    pub const fn viewport(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Configures repeated offscreen performance measurement defaults.
    #[must_use]
    pub const fn perf_config(mut self, config: PerfConfig) -> Self {
        self.perf_config = config;
        self
    }

    /// Selects which Hydrolysis headless runtime backs the session.
    ///
    /// [`RuntimeFlavor::Test`] (the default) mounts on deterministic bundled
    /// fonts and permits software adapters; [`RuntimeFlavor::Application`]
    /// mounts the application's resource fonts under production adapter
    /// selection — the runtime `water run` hosts.
    #[must_use]
    pub const fn runtime(mut self, flavor: RuntimeFlavor) -> Self {
        self.flavor = flavor;
        self
    }

    /// Renders captures at `scale_factor` physical pixels per logical pixel.
    ///
    /// Layout stays in logical units — a `200x100` viewport captured at `2.0`
    /// produces a `400x200` snapshot. Defaults to `1.0`. Applies only to the
    /// rendered mounts.
    ///
    /// # Panics
    ///
    /// Panics at render-mount time if `scale_factor` is not finite or not
    /// positive.
    #[must_use]
    pub const fn scale_factor(mut self, scale_factor: f64) -> Self {
        self.scale_factor = scale_factor;
        self
    }

    fn mount_env(&self) -> Environment {
        let mut env = self.env.clone();
        // The harness mounts views without going through `App::new`, which is
        // where a real app installs the self-drawn realizations the facade
        // carries. Install them here so a test binary that enables
        // `waterui/video-gpu` exercises the same hooks an app would. A
        // realization from a component crate of its own — `waterui-map-gpu` —
        // is installed by the test itself, into the environment it passes here,
        // exactly as an application installs it in `app(env)`.
        waterui::realization::install(&mut env);
        // Hydrolysis owns no platform media bridge, so the self-drawn video
        // realization applies on every host OS, including the ones
        // `realization::install` skips because their system backend would
        // bridge a native player.
        waterui::realization::install_video(&mut env);
        env
    }

    /// Mounts `content` on the semantic runtime — the construction shared by
    /// both builders' `mount`, whose difference is only how the view builder
    /// is assembled.
    fn mount_semantic(self, content: AnyViewBuilder<AnyView>) -> SemanticApp {
        let env = self.mount_env();
        let runtime = match self.flavor {
            RuntimeFlavor::Test => {
                SemanticRuntime::new_for_tests(env, content, self.width, self.height)
            }
            RuntimeFlavor::Application => {
                SemanticRuntime::new(env, content, self.width, self.height)
            }
        };
        SemanticApp::new(runtime, (self.width, self.height))
    }
}

/// Applies a style's environment tokens to a mounted view's environment
/// scope.
///
/// [`SemanticRuntime`] installs the framework default tokens into an
/// environment layered on top of the one it is constructed with, so tokens
/// written into the builder environment beforehand would lose to the
/// defaults on every slot they carry. Scoping the install to the mounted
/// view reproduces the rendered runtime's ordering — framework defaults
/// first, then [`Style::install_tokens`] — on the environment the view tree
/// actually resolves.
struct StyleTokens<S: Style>(Rc<S>);

impl<S: Style> Plugin for StyleTokens<S> {
    fn install(self, env: &mut Environment) {
        self.0.install_tokens(env);
    }
}

impl<S: Style> UiBuilder<Styled<S>> {
    /// Splits the style out of the builder, leaving a `UiBuilder<NoStyle>`
    /// with the same environment, viewport, and runtime configuration.
    fn untheme(self) -> (UiBuilder<NoStyle>, S) {
        let Self {
            env,
            width,
            height,
            style: Styled { style },
            perf_config,
            flavor,
            scale_factor,
        } = self;
        (
            UiBuilder {
                env,
                width,
                height,
                style: NoStyle,
                perf_config,
                flavor,
                scale_factor,
            },
            style,
        )
    }

    /// Mounts a no-arg view builder on the semantic runtime with the
    /// builder's style tokens installed over the framework defaults.
    ///
    /// The mount is still semantic — the runtime is [`SemanticRuntime`] and
    /// the style is never handed to it, so there is no widget theme,
    /// geometry, pointer gestures or capture (those stay on
    /// [`UiBuilder::mount_offscreen`]). What the style contributes is its
    /// [`Style::install_tokens`] step, applied to the mounted view's
    /// environment after the runtime's framework defaults — the ordering the
    /// rendered runtime uses — so components whose bodies resolve their
    /// style package's tokens from the environment see them in a semantic
    /// test.
    ///
    /// # Panics
    ///
    /// Panics if the initial Hydrolysis semantic pass does not produce an
    /// accessibility tree.
    pub fn mount<V, F>(self, view_fn: F) -> SemanticApp
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        let (builder, style) = self.untheme();
        let style = Rc::new(style);
        builder.mount_semantic(AnyViewBuilder::new(move || {
            AnyView::new(view_fn().install(StyleTokens(Rc::clone(&style))))
        }))
    }

    /// Mounts a no-arg view builder on the rendered runtime and returns the
    /// offscreen session, constructed with the builder's style.
    ///
    /// # Panics
    ///
    /// Panics if [`UiBuilder::scale_factor`] was configured with a non-finite
    /// or non-positive value, or if the initial Hydrolysis offscreen frame
    /// does not produce an accessibility tree.
    pub fn mount_offscreen<V, F>(self, view_fn: F) -> OffscreenApp
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        let env = self.mount_env();
        self.mount_rendered(env, AnyViewBuilder::new(move || AnyView::new(view_fn())))
    }

    /// Mounts a whole [`App`] on the rendered runtime and returns the
    /// offscreen session, constructed with the builder's style.
    ///
    /// This is the application path: the session runs the app's own
    /// [`Environment`] — the composition root [`App::new`] configured,
    /// realizations included — layered over the builder's, so the values the
    /// app carries win while what the test installed still applies
    /// underneath. The builder's [`Self::viewport`], [`Self::runtime`] and
    /// [`Self::scale_factor`] apply; [`mount_app`](crate::mount_app) is this
    /// method with the viewport sized from the window's declared frame.
    ///
    /// Only the main window's content is mounted: the headless runtime hosts
    /// a single window, so the app's menu bar and any additional windows are
    /// not mounted. Popup windows the app opens at runtime (context menus,
    /// pickers) are still merged into the accessibility tree by Hydrolysis.
    ///
    /// # Panics
    ///
    /// Panics if the app declares no window, if [`UiBuilder::scale_factor`]
    /// was configured with a non-finite or non-positive value, or if the
    /// initial Hydrolysis offscreen frame does not produce an accessibility
    /// tree.
    #[must_use]
    pub fn mount_app(self, app: App) -> OffscreenApp {
        let (windows, _menu_bar, app_env) = app.into_parts();
        let window = windows
            .into_iter()
            .next()
            .expect("App::into_parts yields the main window first");
        // The app's environment is the composition root, so it layers over
        // the builder's — what the test installed applies underneath it.
        let mut env = app_env.layered_on(&self.env);
        // As on `mount_env`, Hydrolysis owns no platform media bridge, so the
        // self-drawn video realization applies even where `App::new`'s
        // `realization::install` skipped it.
        waterui::realization::install_video(&mut env);
        self.mount_rendered(env, window.content)
    }

    /// Mounts `content` on the rendered runtime — the construction shared by
    /// `mount_offscreen` and `mount_app`, which differ only in where the
    /// environment and the view builder come from.
    fn mount_rendered(self, env: Environment, content: AnyViewBuilder<AnyView>) -> OffscreenApp {
        assert!(
            self.scale_factor.is_finite() && self.scale_factor > 0.0,
            "waterui-testing scale_factor must be finite and greater than zero, got {}",
            self.scale_factor
        );
        let runtime = match self.flavor {
            RuntimeFlavor::Test => HeadlessRuntime::new_for_tests(
                env,
                content,
                self.width,
                self.height,
                self.style.style,
            ),
            RuntimeFlavor::Application => {
                HeadlessRuntime::new(env, content, self.width, self.height, self.style.style)
            }
        };
        OffscreenApp {
            app: SemanticApp::new(
                runtime.with_scale_factor(self.scale_factor),
                (self.width, self.height),
            ),
        }
    }

    /// Measures steady-state offscreen frames for a view with the default
    /// `steady-redraw` scenario.
    pub fn perf<V, F>(self, view_fn: F) -> PerfReport
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
        S: Clone,
    {
        self.perf_with(view_fn, |perf| {
            perf.measure("steady-redraw", |run| {
                run.redraw();
            });
        })
    }

    /// Measures custom offscreen scenarios using a closure-driven automation
    /// API.
    pub fn perf_with<V, F, A>(self, view_fn: F, automation: A) -> PerfReport
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
        A: FnOnce(&mut PerfApp),
        S: Clone,
    {
        let config = self.perf_config;
        let mut app = PerfApp::new(self, view_fn, config);
        automation(&mut app);
        app.finish()
    }
}

/// Options controlling synthetic drag gestures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragOptions {
    /// Number of intermediate pointer-move samples (at least 1).
    pub steps: u16,
    /// Pump one virtual frame between samples so each move lands on its own
    /// frame: gesture recognizers then observe a real motion timeline
    /// (velocity, glide) instead of every sample arriving at once.
    pub frame_per_step: bool,
}

impl Default for DragOptions {
    fn default() -> Self {
        Self {
            steps: 6,
            frame_per_step: false,
        }
    }
}

/// Offscreen GPU-backed app session with snapshot and performance hooks.
///
/// Wraps the [`SemanticApp`] mounted on [`HeadlessRuntime`]: the semantic
/// surface (queries, assertions, accessibility actions, waits, key and text
/// input) is shared with the style-free session, and this wrapper adds what
/// only a rendered runtime can answer — geometry queries, pointer gestures,
/// magnification, and framebuffer snapshots.
#[derive(Debug)]
pub struct OffscreenApp {
    pub(crate) app: SemanticApp<HeadlessRuntime>,
}

impl OffscreenApp {
    /// Returns the semantic app API shared with non-rendering tests.
    #[must_use]
    pub const fn semantic(&self) -> &SemanticApp<HeadlessRuntime> {
        &self.app
    }

    /// Returns the mutable semantic app API shared with non-rendering tests.
    #[must_use]
    pub const fn semantic_mut(&mut self) -> &mut SemanticApp<HeadlessRuntime> {
        &mut self.app
    }

    /// Advances the animation clock by exactly `duration`, pumping one frame
    /// per virtual display interval.
    ///
    /// The clock is virtual: each pump advances it by a fixed frame step (with
    /// an exact remainder step at the end), so `pump_for(60ms)` always lands
    /// on the 60ms point of a transition regardless of host scheduling. No
    /// wall-clock time is slept and no snapshot readback happens; call
    /// [`Self::snapshot`] to capture the phase the clock landed on.
    pub fn pump_for(&mut self, duration: Duration) {
        let mut remaining = duration;
        while !remaining.is_zero() {
            let step = VIRTUAL_FRAME.min(remaining);
            remaining -= step;
            self.app.pump_step(step);
        }
    }

    /// Pumps frames in real time, running work `spawn_local` parked, until
    /// `ready` reports the app has what it needs or `timeout` elapses.
    ///
    /// [`Self::pump_for`] advances a virtual clock, which is right for
    /// animation but cannot let real I/O finish: a component that loads over
    /// the network never progresses, because no wall-clock time passes and the
    /// test executor parks its futures. This drives both, so an offscreen
    /// visual test can cover a component that has to fetch something first.
    ///
    /// Returns whether `ready` became true before the timeout.
    pub fn pump_until(&mut self, timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            self.app.pump_step(VIRTUAL_FRAME);

            if ready() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            // Per-frame pacing inside a pump loop: this is what lets real I/O
            // make progress between frames.
            std::thread::sleep(VIRTUAL_FRAME);
        }
    }

    /// Captures the latest RGBA snapshot from the offscreen renderer.
    ///
    /// # Panics
    ///
    /// Panics if the offscreen driver does not produce a snapshot.
    pub fn snapshot(&mut self) -> Snapshot {
        let _ = crate::executor::drain_parked_local_work();
        let at = self.app.tick(VIRTUAL_FRAME);
        let outcome = RuntimeDriver::pump_at(&mut self.app.runtime, at, true);
        self.app
            .apply_pump_result(outcome)
            .unwrap_or_else(|| panic!("waterui-testing driver did not produce a snapshot"))
    }

    /// Captures a snapshot and stores it in `WaterUI`'s canonical artifact layout.
    pub fn capture_snapshot(
        &mut self,
        suite: impl AsRef<str>,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        let artifacts = self.app.artifacts(suite);
        artifacts.capture_snapshot(case, stage, self.snapshot())
    }

    /// Queues a primary pointer-down without the semantic settle used by
    /// [`SemanticApp::pointer_down_at`]. The event is processed by the next
    /// pump (e.g. [`Self::snapshot`]), so visual stage tests can capture
    /// animation phases that begin at the event — the settle would otherwise
    /// pump frames for its full timeout and skip past short transients such
    /// as the Material ripple growth.
    pub fn queue_pointer_down(&mut self, x: f32, y: f32) {
        self.app.queue_pointer_down_at(x, y);
    }

    /// Queues a primary pointer-up without the semantic settle; see
    /// [`Self::queue_pointer_down`].
    pub fn queue_pointer_up(&mut self, x: f32, y: f32) {
        self.app.queue_pointer_up_at(x, y);
    }

    /// Queues a pointer move without the semantic settle; see
    /// [`Self::queue_pointer_down`]. Visual stage tests use this to park the
    /// pointer away from a widget so idle captures are free of hover state.
    pub fn queue_pointer_move(&mut self, x: f32, y: f32) {
        self.app
            .runtime
            .push_input_event(driver::pointer_move_event(x, y));
    }
}

impl core::ops::Deref for OffscreenApp {
    type Target = SemanticApp<HeadlessRuntime>;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl core::ops::DerefMut for OffscreenApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

/// Mounted semantic app session used in `#[waterui::test(...)]`.
///
/// `R` is the mounted runtime: [`SemanticRuntime`] for the style-free
/// pipeline `UiBuilder::mount` returns (the default parameter), or
/// [`HeadlessRuntime`] underneath [`OffscreenApp`]. The parameter is what
/// carries the split — a semantic query has no `bounds()` method because the
/// semantic session's element handles and pointer entry points exist only on
/// `SemanticApp<HeadlessRuntime>`.
pub struct SemanticApp<R = SemanticRuntime> {
    pub(crate) runtime: R,
    pub(crate) tree: TreeSnapshot,
    pub(crate) ui_focus: Option<NodeId>,
    pub(crate) revision: u64,
    pub(crate) viewport: (u32, u32),
    /// Virtual frame clock: starts at the first pump's wall time and advances
    /// by a fixed step per pump, decoupling animation sampling from host
    /// scheduling. Perf pumps overwrite it so interleaved clocks stay
    /// monotone.
    pub(crate) clock: Option<Instant>,
    pub(crate) resources: ResourceSampler,
}

impl<R> core::fmt::Debug for SemanticApp<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SemanticApp")
            .field("revision", &self.tree.revision())
            .field("nodes", &self.tree.nodes().len())
            .finish_non_exhaustive()
    }
}

/// Snapshot reads every session exposes, whatever its runtime.
impl<R> SemanticApp<R> {
    /// Returns the latest accessibility tree snapshot.
    #[must_use]
    pub const fn tree(&self) -> &TreeSnapshot {
        &self.tree
    }

    /// Returns the latest UI focus target tracked by Hydrolysis.
    #[must_use]
    pub const fn ui_focus(&self) -> Option<NodeId> {
        self.ui_focus
    }

    /// Creates a canonical artifact helper rooted at the provided suite.
    #[must_use]
    pub fn artifacts(&self, suite: impl AsRef<str>) -> TestArtifacts {
        TestArtifacts::new(suite.as_ref())
    }

    /// The viewport size this session was mounted with, in logical pixels.
    #[must_use]
    pub const fn viewport(&self) -> (u32, u32) {
        self.viewport
    }
}

#[allow(
    clippy::missing_panics_doc,
    reason = "assertion helpers intentionally panic with WaterUI-specific diagnostics"
)]
impl<R: RuntimeDriver> SemanticApp<R> {
    /// Mounts `runtime` and settles to quiescence so async-mounted content
    /// (spawned setup tasks, chrome that appears once a controller reports
    /// ready) is part of the initial tree, mirroring `XCUITest`'s
    /// launch-waits-for-idle semantics.
    ///
    /// # Panics
    ///
    /// Panics if the initial pump does not produce a semantic tree.
    pub(crate) fn new(runtime: R, viewport: (u32, u32)) -> Self {
        let mut app = Self {
            runtime,
            tree: TreeSnapshot::empty(),
            ui_focus: None,
            revision: 1,
            viewport,
            clock: None,
            resources: ResourceSampler::new(),
        };
        let rebuilt = app.pump_once();
        assert!(
            rebuilt,
            "waterui-testing initial mount did not produce a semantic tree"
        );
        app.settle();
        app
    }

    /// Starts a chainable semantic query.
    pub fn query(&mut self) -> Query<'_, R> {
        Query {
            app: self,
            selector: Selector::default(),
        }
    }

    /// Convenience existence assertion.
    pub fn assert_exists(&mut self, selector: &Selector) {
        let results = self.resolve_elements(selector);
        let count = results.len();
        assert!(
            (count != 0),
            "waterui-testing assertion failed: selector {} expected to exist but matched 0 nodes on revision {}",
            selector.describe(),
            self.tree.revision()
        );
    }

    /// Convenience non-existence assertion.
    pub fn assert_not_exists(&mut self, selector: &Selector) {
        let results = self.resolve_elements(selector);
        let count = results.len();
        assert!(
            (count == 0),
            "waterui-testing assertion failed: selector {} expected to be absent but matched {count} nodes; candidates: {}",
            selector.describe(),
            results.debug_summary(3)
        );
    }

    /// Asserts that the selector resolves to the current UI-focused element.
    pub fn assert_ui_focus(&mut self, selector: &Selector) {
        let element = self.resolve_single(selector);
        if self.ui_focus == Some(element.id()) {
            return;
        }
        let actual = self.ui_focus.map_or_else(
            || String::from("none"),
            |id| {
                self.tree.node(id).map_or_else(
                    || format!("id={} (no longer in tree)", id.as_u64()),
                    |node| {
                        ElementAnchor::new(id, node.clone(), self.tree.revision()).debug_summary()
                    },
                )
            },
        );
        panic!(
            "waterui-testing assertion failed: selector {} resolved ({}) but UI focus is on {actual}",
            selector.describe(),
            element.debug_summary()
        );
    }

    /// Asserts that the selector resolves to exactly one node with the expected value.
    pub fn assert_value_eq(&mut self, selector: &Selector, value: impl Into<String>) {
        let expected = value.into();
        let element = self.resolve_single(selector);
        let actual = element.node().value();
        assert!(
            actual == Some(expected.as_str()),
            "waterui-testing assertion failed: selector value mismatch (expected {expected:?}, got {actual:?})"
        );
    }

    /// Creates an existence expectation.
    #[must_use]
    pub const fn expect_exists(&self, selector: Selector) -> Expectation {
        Expectation {
            kind: ExpectationKind::Exists(selector),
            inverted: false,
        }
    }

    /// Creates a non-existence expectation.
    #[must_use]
    pub const fn expect_not_exists(&self, selector: Selector) -> Expectation {
        Expectation {
            kind: ExpectationKind::NotExists(selector),
            inverted: false,
        }
    }

    /// Creates a value-equality expectation.
    #[must_use]
    pub fn expect_value_eq(&self, selector: Selector, value: impl Into<String>) -> Expectation {
        Expectation {
            kind: ExpectationKind::ValueEquals {
                selector,
                value: value.into(),
            },
            inverted: false,
        }
    }

    /// Creates a UI-focus expectation: fulfilled once the selector resolves to
    /// the element holding Hydrolysis UI focus.
    #[must_use]
    pub const fn expect_ui_focus(&self, selector: Selector) -> Expectation {
        Expectation {
            kind: ExpectationKind::UiFocus(selector),
            inverted: false,
        }
    }

    /// Resolves a node id to an [`ElementRef`] bound to the current tree, or
    /// `None` when the id is absent.
    ///
    /// External drivers hold node ids rather than query results; this is the
    /// entry point that turns one into a handle usable for scoped queries
    /// ([`Selector::within`], [`Selector::children_of`]) and element-relative
    /// pointer work.
    pub fn element(&mut self, id: NodeId) -> Option<ElementRef<R>> {
        self.sync_tree();
        let node = self.tree.node(id)?.clone();
        Some(ElementRef::new(id, node, self.tree.revision()))
    }

    /// Waits for expectations using XCTest-like semantics.
    pub fn wait_for(&mut self, expectations: &[Expectation], options: WaitOptions) -> WaitResult {
        const MIN_IDLE_BACKOFF: Duration = Duration::from_millis(1);
        const MAX_IDLE_BACKOFF: Duration = Duration::from_millis(16);

        assert!(
            !(expectations.is_empty()),
            "waterui-testing wait_for requires at least one expectation"
        );

        let has_inverted = expectations.iter().any(|e| e.inverted);
        // Order enforcement applies to non-inverted expectations only: an
        // inverted expectation never "fulfills", so it holds no position in
        // the required order.
        let order_ranks = {
            let mut rank = 0usize;
            expectations
                .iter()
                .map(|expectation| {
                    if expectation.inverted {
                        None
                    } else {
                        let current = rank;
                        rank += 1;
                        Some(current)
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut fulfilled = vec![false; expectations.len()];
        let mut next_order_rank = 0usize;
        let deadline = Instant::now() + options.timeout;
        let mut idle_backoff = Duration::ZERO;

        loop {
            for (idx, expectation) in expectations.iter().enumerate() {
                let condition = self.evaluate_expectation(expectation);
                if expectation.inverted {
                    if condition {
                        return WaitResult::InvertedFulfillment;
                    }
                    continue;
                }

                if fulfilled[idx] {
                    continue;
                }

                if condition {
                    if options.enforce_order {
                        let rank = order_ranks[idx]
                            .expect("non-inverted expectation must carry an order rank");
                        if rank != next_order_rank {
                            return WaitResult::IncorrectOrder;
                        }
                        next_order_rank += 1;
                    }
                    fulfilled[idx] = true;
                }
            }

            let all_non_inverted = expectations
                .iter()
                .enumerate()
                .all(|(idx, expectation)| expectation.inverted || fulfilled[idx]);

            if all_non_inverted && !has_inverted {
                return WaitResult::Completed;
            }

            let now = Instant::now();
            if now >= deadline {
                return if all_non_inverted {
                    WaitResult::Completed
                } else {
                    WaitResult::TimedOut
                };
            }

            let _ = self.pump_once();
            if !self.runtime.is_settled() {
                // Scheduled work remains (animations, patches, queued input):
                // keep pumping virtual frames without wall-clock sleeps.
                idle_backoff = Duration::ZERO;
                continue;
            }

            // Quiescent but unfulfilled: the awaited change can only arrive
            // from outside the runtime (a worker thread, wall-clock async), so
            // yield real time with exponential backoff.
            let next_backoff = if idle_backoff.is_zero() {
                MIN_IDLE_BACKOFF
            } else {
                idle_backoff.saturating_mul(2).min(MAX_IDLE_BACKOFF)
            };
            idle_backoff = next_backoff;

            let now = Instant::now();
            if now >= deadline {
                continue;
            }
            let remaining = deadline.saturating_duration_since(now);
            let sleep_for = next_backoff.min(remaining);
            if !sleep_for.is_zero() {
                std::thread::sleep(sleep_for);
            }
        }
    }

    /// Convenience API mirroring `XCTest` `waitForExistence`.
    pub fn wait_for_existence(&mut self, selector: &Selector, timeout: Duration) -> bool {
        let expectation = self.expect_exists(selector.clone());
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Convenience API mirroring `XCTest` `waitForNonexistence`.
    pub fn wait_for_nonexistence(&mut self, selector: &Selector, timeout: Duration) -> bool {
        let expectation = self.expect_not_exists(selector.clone());
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Waits for one node's value to equal the expected value.
    pub fn wait_for_value_eq(
        &mut self,
        selector: &Selector,
        value: impl Into<String>,
        timeout: Duration,
    ) -> bool {
        let expectation = self.expect_value_eq(selector.clone(), value);
        self.wait_for(&[expectation], WaitOptions::new(timeout)) == WaitResult::Completed
    }

    /// Waits until the selector resolves to the current UI-focused element.
    pub fn wait_for_ui_focus(&mut self, selector: &Selector, timeout: Duration) -> bool {
        const MIN_IDLE_BACKOFF: Duration = Duration::from_millis(1);
        const MAX_IDLE_BACKOFF: Duration = Duration::from_millis(16);

        let deadline = Instant::now() + timeout;
        let mut idle_backoff = Duration::ZERO;
        loop {
            if self.matches_ui_focus(selector) {
                return true;
            }

            if Instant::now() >= deadline {
                return false;
            }

            let _ = self.pump_once();
            if !self.runtime.is_settled() {
                idle_backoff = Duration::ZERO;
                continue;
            }

            let next_backoff = if idle_backoff.is_zero() {
                MIN_IDLE_BACKOFF
            } else {
                idle_backoff.saturating_mul(2).min(MAX_IDLE_BACKOFF)
            };
            idle_backoff = next_backoff;
            std::thread::sleep(
                next_backoff.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn evaluate_expectation(&mut self, expectation: &Expectation) -> bool {
        match &expectation.kind {
            ExpectationKind::Exists(selector) => !self.matching_ids(selector).is_empty(),
            ExpectationKind::NotExists(selector) => self.matching_ids(selector).is_empty(),
            ExpectationKind::ValueEquals { selector, value } => {
                let ids = self.matching_ids(selector);
                if ids.len() != 1 {
                    return false;
                }
                self.tree[ids[0]].value() == Some(value.as_str())
            }
            ExpectationKind::UiFocus(selector) => self.matches_ui_focus(selector),
        }
    }

    /// Brings the held tree up to date with any state change that has already
    /// been requested but not yet flushed.
    ///
    /// Every *input* path settles after dispatching, so a tap's consequences
    /// are in the tree by the time the call returns. A test that changes state
    /// directly — setting a `Binding` it owns — goes through no such path, and
    /// without this the next query would answer from the tree as it was before
    /// the change, reporting the old label and passing assertions that should
    /// fail. Reading the tree is therefore what pulls the update through.
    ///
    /// This waits on unapplied work only, never on work that continues by
    /// itself: an app with a running animation is never settled, so settling
    /// here would spend the full pump budget on every query.
    fn sync_tree(&mut self) {
        /// Enough pumps for a change to cascade (a patch that schedules the
        /// next), far below anything a real update needs. Exceeding it means
        /// the app re-dirties itself every frame, which the settle path — with
        /// its own budget — is the right tool for.
        const MAX_SYNC_PUMPS: usize = 8;

        for _ in 0..MAX_SYNC_PUMPS {
            if !self.runtime.has_pending_semantic_update() {
                return;
            }
            self.pump_once();
        }
    }

    fn matching_ids(&mut self, selector: &Selector) -> Vec<NodeId> {
        self.sync_tree();
        self.validate_selector_scope(selector);
        self.tree.matching(selector)
    }

    /// Resolves every element matching `selector` against the current tree.
    pub fn resolve_elements(&mut self, selector: &Selector) -> ElementSet<R> {
        let ids = self.matching_ids(selector);
        // The sync inside `matching_ids` can apply a newer tree; the handles'
        // revision must be read after it so they are not born stale.
        let revision = self.tree.revision();
        let elements = ids
            .into_iter()
            .map(|id| ElementRef::new(id, self.tree[id].clone(), revision))
            .collect();
        ElementSet::new(elements)
    }

    pub(crate) fn resolve_single(&mut self, selector: &Selector) -> ElementRef<R> {
        let results = self.resolve_elements(selector);
        match results.len() {
            1 => results[0].clone(),
            0 => panic!(
                "waterui-testing selector {} resolved 0 nodes, expected exactly 1 on revision {}",
                selector.describe(),
                self.tree.revision()
            ),
            n => panic!(
                "waterui-testing selector {} resolved {n} nodes, expected exactly 1; candidates: {}",
                selector.describe(),
                results.debug_summary(3)
            ),
        }
    }

    /// Performs an accessibility action on a node, then settles the resulting
    /// updates.
    ///
    /// Returns whether the runtime handled the action. Unknown node ids and
    /// actions the node does not support report `false` — they never panic,
    /// which makes this the entry point for external drivers that must surface
    /// failures as data rather than aborting the session. [`Query`] and
    /// [`ElementRef`] use the same path through `perform_action_expect`, which
    /// keeps the panicking test semantics.
    pub fn perform_action(
        &mut self,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) -> bool {
        let handled = self.queue_action(node_id, action, data);
        self.settle();
        handled
    }

    /// Performs an accessibility action on a node without the semantic settle
    /// used by [`Self::perform_action`].
    ///
    /// The request is processed by the next pump, so a caller stepping the
    /// virtual clock frame by frame — [`OffscreenApp::pump_for`] followed by
    /// [`OffscreenApp::snapshot`] — observes the transient the action
    /// triggers. Returns whether the runtime handled the action, with the
    /// same failure semantics as [`Self::perform_action`].
    pub fn queue_action(
        &mut self,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) -> bool {
        let request = AccessibilityActionRequest {
            target_tree: AccessibilityTreeId::ROOT,
            target_node: node_id.as_accesskit(),
            action,
            data,
        };
        self.runtime.perform_accessibility_action(request)
    }

    pub(crate) fn perform_action_expect(
        &mut self,
        node_id: NodeId,
        action: AccessibilityAction,
        data: Option<AccessibilityActionData>,
    ) {
        assert!(
            self.perform_action(node_id, action, data),
            "waterui-testing: accessibility action {action:?} on {} was not handled by the runtime — the target does not support this action",
            self.describe_node(node_id),
        );
    }

    fn describe_node(&self, node_id: NodeId) -> String {
        self.tree.node(node_id).map_or_else(
            || format!("node id={} (no longer in tree)", node_id.as_u64()),
            |node| ElementAnchor::new(node_id, node.clone(), self.tree.revision()).debug_summary(),
        )
    }

    /// Clears the latest Hydrolysis-managed UI focus target, then settles
    /// resulting updates.
    ///
    /// A no-op when nothing holds UI focus.
    pub fn clear_ui_focus(&mut self) {
        if self.queue_clear_ui_focus() {
            self.settle();
        }
    }

    /// Clears the latest Hydrolysis-managed UI focus target without the
    /// semantic settle used by [`Self::clear_ui_focus`]. Returns whether a
    /// focus target was released.
    pub fn queue_clear_ui_focus(&mut self) -> bool {
        self.runtime.clear_ui_focus()
    }

    /// Dispatches committed text through the Hydrolysis text input path.
    pub fn text_input(&mut self, text: impl Into<String>) {
        self.queue_text_input(text);
        self.settle();
    }

    /// Dispatches committed text through the Hydrolysis text input path
    /// without the semantic settle used by [`Self::text_input`].
    pub fn queue_text_input(&mut self, text: impl Into<String>) {
        self.runtime
            .push_input_event(driver::text_input_event(text.into()));
    }

    /// Dispatches a named keyboard key stroke — press, then release — such as
    /// `Backspace`, `Delete`, or `ArrowLeft`.
    pub fn press_named_key(&mut self, key: impl Into<String>) {
        self.press_named_key_with(key, Modifiers::default());
    }

    /// Dispatches a named keyboard key stroke — press, then release — with
    /// explicit modifiers held.
    pub fn press_named_key_with(&mut self, key: impl Into<String>, modifiers: Modifiers) {
        let key = KeyCode::Named(key.into());
        self.queue_key_press(key.clone(), modifiers);
        self.queue_key_release(key, modifiers);
        self.settle();
    }

    /// Dispatches a character keyboard key stroke — press, then release —
    /// without text-input synthesis.
    pub fn press_character_key(&mut self, key: impl Into<String>) {
        self.press_character_key_with(key, Modifiers::default());
    }

    /// Dispatches a character keyboard key stroke — press, then release —
    /// with explicit modifiers held.
    pub fn press_character_key_with(&mut self, key: impl Into<String>, modifiers: Modifiers) {
        let key = KeyCode::Character(key.into());
        self.queue_key_press(key.clone(), modifiers);
        self.queue_key_release(key, modifiers);
        self.settle();
    }

    /// Presses a keyboard key and leaves it held, settling after the key-down.
    /// Pair with [`Self::key_up`] to hold a key across calls.
    pub fn key_down(&mut self, key: KeyCode, modifiers: Modifiers) {
        self.queue_key_press(key, modifiers);
        self.settle();
    }

    /// Releases a keyboard key held by [`Self::key_down`], settling after the
    /// key-up.
    pub fn key_up(&mut self, key: KeyCode, modifiers: Modifiers) {
        self.queue_key_release(key, modifiers);
        self.settle();
    }

    /// Queues a keyboard key-down with explicit modifiers held, without the
    /// semantic settle used by [`Self::key_down`]. The event is processed by
    /// the next pump, so a key-triggered transient — a focus ring appearing
    /// on `Tab`, a sheet dismissing on `Escape` — stays observable to
    /// [`OffscreenApp::pump_for`] and [`OffscreenApp::snapshot`].
    pub fn queue_key_press(&mut self, key: KeyCode, modifiers: Modifiers) {
        self.runtime
            .push_input_event(driver::key_press_event(key, modifiers));
    }

    /// Queues a keyboard key-up matching [`Self::queue_key_press`], without a
    /// settle — the release half of the stroke [`Self::press_named_key_with`]
    /// and [`Self::key_up`] dispatch.
    pub fn queue_key_release(&mut self, key: KeyCode, modifiers: Modifiers) {
        self.runtime
            .push_input_event(driver::key_release_event(key, modifiers));
    }

    /// Pumps virtual frames until the runtime reports quiescence — no queued
    /// input, no spawned work awaiting a drain, and no renderer-scheduled
    /// semantic work — or until the virtual cap elapses.
    ///
    /// Quiescence alone is not the whole story: a `spawn_local` task parked on
    /// a wall-clock timer or in-flight I/O holds no queued runnable, so the
    /// runtime cannot see it. While [`waterui::task::outstanding_local_tasks`]
    /// reports such work, settling paces real time — each pump runs whatever
    /// the last wake re-queued — until the task publishes its result or the
    /// wall-clock cap elapses. This is what lets a `Photo` fetch or an
    /// `avatar` image that completes after real I/O reach the tree.
    ///
    /// Pacing holds the virtual clock still. The scene is quiescent at that
    /// point, so nothing it schedules needs a frame, and moving the clock
    /// while a test waits on the outside world would run gestures, timers and
    /// playback ahead of the assertions that follow: a long press held while
    /// an image decodes would activate, play, and finish inside one settle.
    /// Once a paced pump changes the tree, settling returns to advancing
    /// frames so whatever the change scheduled can run.
    ///
    /// In-flight work is waited for, whatever its length: a playback task
    /// or a stream keeps settling busy until it ends or the wall-clock cap
    /// elapses. A test that has to observe such a transient — motion that
    /// plays while a press is held — dispatches the press with
    /// [`Self::queue_pointer_down_at`] and waits on the tree instead of
    /// settling.
    ///
    /// The virtual cap exists solely for perpetual animations (an
    /// indeterminate progress spinner keeps the animation controller active
    /// forever); every finite transition ends well before it. Each pump
    /// advances the virtual clock one frame, so the cap costs pump work, never
    /// wall-clock sleeps.
    ///
    /// Public because a test that drives non-visual work — a handler that spawns
    /// onto the local executor, a coalesced push that lands on the next tick —
    /// has to be able to say "let queued work finish" without inventing an
    /// accessibility node to wait on.
    pub fn settle(&mut self) {
        /// Virtual time budget for perpetual animations; ~62 pumps at 16ms.
        const SETTLE_CAP: Duration = Duration::from_secs(1);
        /// Wall-clock budget for local tasks parked on real I/O or timers.
        /// Long enough for a remote fetch on a slow link; bounded so a
        /// permanently parked task cannot hang the caller forever.
        const SETTLE_WALL_CAP: Duration = Duration::from_secs(5);

        let mut remaining = SETTLE_CAP;
        let wall_deadline = Instant::now() + SETTLE_WALL_CAP;
        loop {
            let _ = self.pump_once();
            if !self.runtime.is_settled() {
                remaining = remaining.saturating_sub(VIRTUAL_FRAME);
                if remaining.is_zero() {
                    return;
                }
                continue;
            }
            // Quiescent but a local task may be parked on a wall-clock wake:
            // give it real time, then run what it re-queued at the same
            // virtual instant.
            loop {
                if waterui::task::outstanding_local_tasks() == 0 || Instant::now() >= wall_deadline
                {
                    return;
                }
                std::thread::sleep(VIRTUAL_FRAME);
                if self.pump_held() || !self.runtime.is_settled() {
                    break;
                }
            }
        }
    }

    /// Pumps one frame without advancing the virtual clock, returning whether
    /// the tree changed.
    fn pump_held(&mut self) -> bool {
        self.pump_step(Duration::ZERO)
    }

    /// Whether the runtime is quiescent: no queued input, no spawned work
    /// awaiting a drain, and no renderer-scheduled semantic work.
    ///
    /// A running animation counts as scheduled work — an app that never comes
    /// to rest never reports settled, which is what lets a caller stepping
    /// [`OffscreenApp::pump_for`] tell "still animating" apart from "idle".
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.runtime.is_settled()
    }

    /// Advances the virtual clock by `step` and returns the new frame instant.
    fn tick(&mut self, step: Duration) -> Instant {
        let next = self
            .clock
            .map_or_else(Instant::now, |current| current + step);
        self.clock = Some(next);
        next
    }

    fn pump_once(&mut self) -> bool {
        self.pump_step(VIRTUAL_FRAME)
    }

    /// Advances the virtual clock by `step`, pumps one frame at the landed
    /// instant, and applies the produced tree update. Returns whether the
    /// frame rebuilt the tree.
    fn pump_step(&mut self, step: Duration) -> bool {
        let _ = crate::executor::drain_parked_local_work();
        let at = self.tick(step);
        let outcome = self.runtime.pump_at(at, false);
        let rebuilt = outcome.rebuilt;
        let _ = self.apply_pump_result(outcome);
        rebuilt
    }

    fn apply_pump_result(&mut self, outcome: DriverPumpResult) -> Option<Snapshot> {
        self.ui_focus = outcome.ui_focus;
        if let Some(update) = outcome.tree_update {
            self.tree = TreeSnapshot::from_update(self.revision, update);
            self.revision = self
                .revision
                .checked_add(1)
                .expect("waterui-testing tree revision overflow");
        } else {
            assert!(
                !self.tree.nodes().is_empty(),
                "waterui-testing did not receive an accessibility tree update after mount"
            );
        }
        outcome.snapshot
    }

    fn matches_ui_focus(&mut self, selector: &Selector) -> bool {
        let ids = self.matching_ids(selector);
        ids.len() == 1 && self.ui_focus == Some(ids[0])
    }

    pub(crate) fn assert_current_element(&self, element: &ElementRef<R>, context: &str) {
        self.assert_current_anchor(&element.anchor(), context);
    }

    pub(crate) fn assert_current_anchor(&self, anchor: &ElementAnchor, context: &str) {
        assert!(
            anchor.revision() == self.tree.revision(),
            "waterui-testing stale element handle during {context}: handle revision {} does not match current tree revision {}; re-query the element before interacting. handle={}",
            anchor.revision(),
            self.tree.revision(),
            anchor.debug_summary()
        );
        assert!(
            self.tree.node(anchor.id()).is_some(),
            "waterui-testing missing current node for handle during {context}: handle={} is not present in revision {}",
            anchor.debug_summary(),
            self.tree.revision()
        );
    }

    fn validate_selector_scope(&self, selector: &Selector) {
        if let Some(scope) = selector.scope() {
            self.assert_current_anchor(scope.handle(), "scoped query");
        }
    }

    pub(crate) fn tap_node(&mut self, node_id: NodeId) {
        self.perform_action_expect(node_id, AccessibilityAction::Click, None);
    }

    pub(crate) fn focus_node(&mut self, node_id: NodeId) {
        self.perform_action_expect(node_id, AccessibilityAction::Focus, None);
    }

    pub(crate) fn set_text_node(&mut self, node_id: NodeId, value: impl Into<String>) {
        self.perform_action_expect(
            node_id,
            AccessibilityAction::SetValue,
            Some(AccessibilityActionData::Value(
                value.into().into_boxed_str(),
            )),
        );
    }

    pub(crate) fn increment_node(&mut self, node_id: NodeId) {
        self.perform_action_expect(node_id, AccessibilityAction::Increment, None);
    }

    pub(crate) fn decrement_node(&mut self, node_id: NodeId) {
        self.perform_action_expect(node_id, AccessibilityAction::Decrement, None);
    }

    pub(crate) fn scroll_down_node(&mut self, node_id: NodeId) {
        self.perform_action_expect(node_id, AccessibilityAction::ScrollDown, None);
    }

    pub(crate) fn expand_node(&mut self, node_id: NodeId) {
        self.perform_action_expect(node_id, AccessibilityAction::Expand, None);
    }

    pub(crate) fn collapse_node(&mut self, node_id: NodeId) {
        self.perform_action_expect(node_id, AccessibilityAction::Collapse, None);
    }
}

/// Geometry and pointer surface, available only on the rendered runtime: the
/// semantic pipeline's accessibility tree is a product of the view tree and
/// widgets' semantics and carries no layout, so none of this exists on
/// `SemanticApp<SemanticRuntime>`.
#[allow(
    clippy::missing_panics_doc,
    reason = "assertion helpers intentionally panic with WaterUI-specific diagnostics"
)]
impl SemanticApp<HeadlessRuntime> {
    /// Moves the pointer to viewport coordinates without pressing a button,
    /// then settles resulting updates.
    pub fn hover_at(&mut self, x: f32, y: f32) {
        self.queue_hover_at(x, y);
        self.settle();
    }

    /// Moves the pointer to viewport coordinates without the semantic settle
    /// used by [`Self::hover_at`]. The event is processed by the next pump,
    /// so a hover-triggered transient stays observable to
    /// [`OffscreenApp::pump_for`] and [`OffscreenApp::snapshot`].
    pub fn queue_hover_at(&mut self, x: f32, y: f32) {
        self.runtime
            .push_input_event(driver::pointer_move_event(x, y));
    }

    /// Dispatches a pointer tap at viewport coordinates and settles resulting updates.
    pub fn tap_at(&mut self, x: f32, y: f32) {
        self.runtime
            .push_input_event(driver::pointer_down_event(x, y));
        self.runtime
            .push_input_event(driver::pointer_up_event(x, y));
        self.settle();
    }

    /// Dispatches a primary pointer-down event at viewport coordinates.
    pub fn pointer_down_at(&mut self, x: f32, y: f32) {
        self.queue_pointer_down_at(x, y);
        self.settle();
    }

    /// Dispatches a primary pointer-down event at viewport coordinates
    /// without the settle used by [`Self::pointer_down_at`]. The event is
    /// processed by the next pump, so work the press starts — a long press's
    /// motion, a drag preview — stays observable to [`Self::wait_for`] rather
    /// than being waited out.
    pub fn queue_pointer_down_at(&mut self, x: f32, y: f32) {
        self.runtime
            .push_input_event(driver::pointer_down_event(x, y));
    }

    /// Right-clicks at viewport coordinates, opening a context menu if there is
    /// one there.
    pub fn secondary_click_at(&mut self, x: f32, y: f32) {
        self.queue_secondary_click(x, y);
        self.settle();
    }

    /// Right-clicks at viewport coordinates without the semantic settle used
    /// by [`Self::secondary_click_at`]. The event is processed by the next
    /// pump, so a menu's opening transient stays observable to
    /// [`OffscreenApp::pump_for`] and [`OffscreenApp::snapshot`].
    pub fn queue_secondary_click(&mut self, x: f32, y: f32) {
        for event in driver::secondary_click_events(x, y) {
            self.runtime.push_input_event(event);
        }
    }

    /// Dispatches a primary pointer-up event at viewport coordinates.
    pub fn pointer_up_at(&mut self, x: f32, y: f32) {
        self.queue_pointer_up_at(x, y);
        self.settle();
    }

    /// Dispatches a primary pointer-up event at viewport coordinates without
    /// the settle used by [`Self::pointer_up_at`]; the release is processed
    /// by the next pump.
    pub fn queue_pointer_up_at(&mut self, x: f32, y: f32) {
        self.runtime
            .push_input_event(driver::pointer_up_event(x, y));
    }

    pub(crate) fn drag_from_to(&mut self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) {
        self.drag_from_to_with(from_x, from_y, to_x, to_y, DragOptions::default());
    }

    /// Dispatches a drag between viewport coordinates with explicit step and
    /// timing control, then settles resulting updates.
    pub fn drag_from_to_with(
        &mut self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        options: DragOptions,
    ) {
        self.dispatch_drag(from_x, from_y, to_x, to_y, options);
        self.settle();
    }

    /// Dispatches a drag between viewport coordinates without the semantic
    /// settle used by [`Self::drag_from_to_with`]. The events — and the
    /// per-step pumps when [`DragOptions::frame_per_step`] is set — are
    /// processed immediately; only the final settle is skipped, so a
    /// release-triggered transient stays observable to
    /// [`OffscreenApp::pump_for`] and [`OffscreenApp::snapshot`].
    pub fn queue_drag_from_to_with(
        &mut self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        options: DragOptions,
    ) {
        self.dispatch_drag(from_x, from_y, to_x, to_y, options);
    }

    fn dispatch_drag(
        &mut self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        options: DragOptions,
    ) {
        let steps = options.steps.max(1);
        self.runtime
            .push_input_event(driver::pointer_down_event(from_x, from_y));
        for step in 1..=steps {
            let t = f32::from(step) / f32::from(steps);
            let x = (to_x - from_x).mul_add(t, from_x);
            let y = (to_y - from_y).mul_add(t, from_y);
            self.runtime
                .push_input_event(driver::pointer_move_event(x, y));
            if options.frame_per_step {
                let _ = self.pump_step(VIRTUAL_FRAME);
            }
        }
        self.runtime
            .push_input_event(driver::pointer_up_event(to_x, to_y));
    }

    /// Dispatches a wheel/trackpad scroll at viewport coordinates and settles
    /// resulting updates.
    pub fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) {
        self.queue_scroll_at(x, y, dx, dy, is_line_delta);
        self.settle();
    }

    /// Dispatches a wheel/trackpad scroll at viewport coordinates without the
    /// semantic settle used by [`Self::scroll_at`]. The event is processed by
    /// the next pump, so a scroll's glide transient stays observable to
    /// [`OffscreenApp::pump_for`] and [`OffscreenApp::snapshot`].
    pub fn queue_scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) {
        self.runtime
            .push_input_event(driver::scroll_event(x, y, dx, dy, is_line_delta));
    }

    /// Dispatches a magnification (pinch) gesture centered at viewport
    /// coordinates and settles resulting updates.
    pub fn magnify_at(&mut self, x: f32, y: f32, factor: f32) {
        self.queue_magnify_at(x, y, factor);
        self.settle();
    }

    /// Dispatches a magnification (pinch) gesture without the semantic settle
    /// used by [`Self::magnify_at`]. The events are processed by the next
    /// pump, so a zoom transient stays observable to
    /// [`OffscreenApp::pump_for`] and [`OffscreenApp::snapshot`].
    pub fn queue_magnify_at(&mut self, x: f32, y: f32, factor: f32) {
        for event in driver::magnification_events(x, y, factor) {
            self.runtime.push_input_event(event);
        }
    }

    /// Pumps one complete offscreen frame at `at`, adopting the instant as the
    /// session's virtual clock so interleaved semantic pumps stay monotone,
    /// and reports the frame's timing and a process resource sample.
    ///
    /// Perf runs drive this directly; the produced tree update is applied so a
    /// mid-run semantic query reads the state the frame landed on.
    pub(crate) fn pump_frame_at(&mut self, at: Instant) -> FrameTiming {
        self.clock = Some(at);
        let _ = crate::executor::drain_parked_local_work();
        let started_at = Instant::now();
        let outcome = RuntimeDriver::pump_at(&mut self.runtime, at, false);
        let timing = FrameTiming {
            total: outcome.profile.total.max(started_at.elapsed()),
            rebuilt: outcome.rebuilt,
            profile: outcome.profile,
            resources: self.resources.sample(),
        };
        let _ = self.apply_pump_result(outcome);
        timing
    }
}
