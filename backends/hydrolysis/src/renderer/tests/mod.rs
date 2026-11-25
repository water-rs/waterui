mod collection_update;
// Suites that drive the GPU painter itself — its device, GPU-content views,
// filters or backdrop shaders — run on builds that have it.
mod frame_rate_vote;
#[cfg(hydrolysis_gpu_painter)]
mod frame_work;
mod slider_look;
mod slider_repaint;
use super::*;
use crate::text::{Affinity, TextEngine as _, TextLayout as _, TextPosition};
use std::borrow::Cow;
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
#[cfg(hydrolysis_gpu_painter)]
use std::rc::Weak;
use std::sync::mpsc;

use executor_core::LocalExecutor;
use executor_core::async_task::{self, AsyncTask, Runnable};

#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod anchored_overlay;
#[cfg(hydrolysis_gpu_painter)]
mod animation_slots;
mod change_animation;
mod chrome_primitives;
#[cfg(hydrolysis_gpu_painter)]
mod chrome_safe_area;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod context_menu_occlusion;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod context_menu_presentation;
#[cfg(hydrolysis_cpu_painter)]
mod cpu_refusals;
#[cfg(hydrolysis_software_surface)]
mod cpu_window_present;
mod drag_drop;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod dynamic_remeasure;
// The patch-free capture this pins is an applied filter's, which both
// Cherenkov painters realize.
#[cfg(all(hydrolysis_cherenkov, not(target_arch = "wasm32")))]
mod emoji_atlas;
// Drives the GPU mirror's `SurfaceTree`, the sampler the GPU painter runs.
#[cfg(hydrolysis_gpu_painter)]
mod content_effect;
#[cfg(hydrolysis_gpu_painter)]
mod engine_motion;
mod gesture_buttons;
mod gesture_capture;
mod gesture_env;
mod gesture_retention;
mod gesture_surface;
#[cfg(hydrolysis_gpu_painter)]
mod glass_background;
#[cfg(hydrolysis_gpu_painter)]
mod gpu_surface_direct;
#[cfg(hydrolysis_gpu_painter)]
mod gpu_surface_idle;
#[cfg(hydrolysis_gpu_painter)]
mod gpu_surface_input;
mod group_opacity;
mod image_ingest;
#[cfg(hydrolysis_gpu_painter)]
mod ime;
mod interaction_state;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod interactive_dynamics;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod layer_occlusion;
mod layout_contract;
mod lazy_cross;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod list_remeasure;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod list_row_focus;
mod list_row_hit;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod list_row_metrics;
mod list_swipe_frames;
#[cfg(feature = "accessibility")]
mod list_table_scroll_frames;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod list_visibility;
#[cfg(hydrolysis_gpu_painter)]
mod material;
#[cfg(hydrolysis_gpu_painter)]
mod material_chrome;
#[cfg(hydrolysis_gpu_painter)]
mod material_group;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod menu_shortcuts;
mod mid_flush_subview;
#[cfg(hydrolysis_gpu_painter)]
pub mod mirror;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod nav_tabs_look;
mod navigation_back;
#[cfg(hydrolysis_gpu_painter)]
mod navigation_layers;
mod navigation_text_input;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod nested_menu_dispatch;
mod opacity_binding;
mod painted;
#[cfg(hydrolysis_gpu_painter)]
mod perf_full_rebuild;
mod perf_scroll;
#[cfg(not(target_arch = "wasm32"))]
mod popup_frame;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod popup_windows;
mod progress_looks;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod registrations;
mod render_identity;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod response_geometry;
mod retained_scene;
mod scene_offer;
#[cfg(feature = "accessibility")]
mod scroll_frames;
mod scroll_hit_clip;
mod segmented_look;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod semantic_runtime;
#[cfg(hydrolysis_cherenkov)]
mod shader_paint;
mod shadow;
mod shadow_border_signals;
mod stepper_look;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod structural_transitions;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod tab_item_layout;
mod teardown_order;
mod text_field_look;
mod text_ink;
mod text_measure_direction;
mod text_wrap_width;
mod toggle_glass;
mod toggle_looks;
#[cfg(hydrolysis_gpu_painter)]
mod top_edge;
mod touch_scroll;
mod tree;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod when_flex_sibling;
mod when_payload;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
#[cfg(not(target_arch = "wasm32"))]
mod window_background;
#[cfg(not(target_arch = "wasm32"))]
mod window_mount;
use kurbo::{Affine, Point, Rect, Size};
use waterui::gesture::{
    DragEvent, DragGesture, GestureObserver, MagnificationEvent, MagnificationGesture,
};
use waterui::interaction::InteractionState;
use waterui::prelude::text;
use waterui::style::FloatingStyle;
use waterui::widget::Divider;
use waterui::{Binding, Color, Computed, Signal, SignalExt as _, ViewExt as _};
use waterui_controls::button::button;
use waterui_controls::label::{LabelDisplayMode, label};
use waterui_controls::slider::slider;
#[cfg(feature = "accessibility")]
use waterui_controls::text_field::field;
use waterui_controls::toggle::toggle;
use waterui_form::picker::PickerStyle;
#[cfg(feature = "accessibility")]
use waterui_form::picker::{Picker, color::ColorPicker, date::DatePicker};
use waterui_graphics::canvas::Canvas;
use waterui_layout::scroll;
use waterui_layout::stack::{VStackLayout, hstack, vstack, zstack};
#[cfg(feature = "accessibility")]
use waterui_navigation::NavigationView;
#[cfg(feature = "accessibility")]
use waterui_navigation::tab::{Tab, TabStyle, Tabs};

#[cfg(feature = "list")]
use crate::testing::plain_list_look;
use crate::theme::WidgetTheme;
use crate::theme::{
    BadgeLook, ButtonLook, ButtonSpec, Chrome, CornerShape, Custom, DragMapping, Driven, Elevation,
    FocusIndicator, HitRegion, IndicatorLook, InputFieldMetrics, InteractionFocusBinding,
    InteractionMotion, Knob, Label, LinearProgressLook, ListLook, ListSpec, MenuLayout, MenuLook,
    MenuPickerLook, MenuSpec, ModalInteraction, Motion, NavigationBarLook, NavigationSpec,
    PickerSpec, PressFeedback, ProgressControl, ProgressSpec, RadioGroupLook, ScrollIndicatorLook,
    SliderLayout, SliderLook, SliderSpec, StateLayer, StepperLook, StepperSpec, Surface,
    TabItemGeometry, TabItemLayout, TableLook, TabsLook, TabsSpec, TextCaretMotion, TextFieldLook,
    TextFieldSpec, ToggleSpec, ValueIndicator,
};
use crate::theme::{CheckboxLook, SwitchDrag, SwitchLook};
use cherenkov_record::{
    BackdropShaderSource, CaptureClass, ContainerRecipe, ContentShader, ContentShaderSource, Draw,
    GroupMember, MaterialCapture, MaterialEffect, MaterialRegistry, MaterialShader, Paint, Shadow,
    SharedLive, WorkingColor,
};
use waterui_core::handler::SharedAction;
use waterui_text::font::Font;

/// A view realized into a retained node and measured through it — the path
/// layout measures every child on.
pub(super) struct Realized {
    renderer: HydrolysisRenderer<ParleyEngine>,
    node: RenderNode<ParleyEngine>,
    env: Environment,
}

impl Realized {
    pub(super) fn new(view: impl View, env: &Environment) -> Self {
        Self::with_renderer(view, env, test_renderer())
    }

    pub(super) fn with_theme(
        view: impl View,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
    ) -> Self {
        Self::with_renderer(
            view,
            env,
            HydrolysisRenderer::with_engine(
                Rc::clone(theme),
                ParleyEngine::system(FontFamilyResolution::Strict),
            ),
        )
    }

    pub(super) fn with_renderer(
        view: impl View,
        env: &Environment,
        mut renderer: HydrolysisRenderer<ParleyEngine>,
    ) -> Self {
        let node = RenderNode::build(AnyView::new(view), env, &mut renderer);
        Self {
            renderer,
            node,
            env: env.clone(),
        }
    }

    pub(super) fn measure(&mut self, proposal: ProposalSize) -> ViewDimensions {
        self.node.prepare_for_measure(&mut self.renderer);
        let theme = self.renderer.theme();
        self.node
            .measure(&mut self.renderer.state, &self.env, &theme, proposal)
    }

    pub(super) fn intrinsic(&mut self) -> LayoutSize {
        self.measure(ProposalSize::UNSPECIFIED).size
    }

    pub(super) fn stretch_axis(&self) -> StretchAxis {
        self.node.stretch()
    }
}

fn test_renderer() -> HydrolysisRenderer<crate::text::ParleyEngine> {
    test_renderer_with_theme(MinimalTestTheme::default())
}

fn test_renderer_with_theme(
    theme: MinimalTestTheme,
) -> HydrolysisRenderer<crate::text::ParleyEngine> {
    HydrolysisRenderer::with_engine(
        Rc::new(theme),
        ParleyEngine::system(FontFamilyResolution::Strict),
    )
}

/// Emits the semantic node a real widget emits for an interaction identity:
/// it advertises `Focus` (and `Click` when it activates), its action target is
/// registered, and the interaction key is linked so the pointer machinery —
/// and keyboard traversal, which reads the semantic tree — resolves the node.
#[cfg(feature = "accessibility")]
fn emit_focusable_node<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    key: &InteractionKey,
    bounds: Rect,
    env: &Environment,
    activation: Option<AccessibilityActivation<E>>,
) -> AccessibilityNodeId {
    let mut node = AccessibilityNode::new(AccessibilityNodeRole::Button);
    node.add_action(AccessibilityAction::Focus);
    let action_target = activation.map(|action| {
        node.add_action(AccessibilityAction::Click);
        AccessibilityActionTarget::Activate { action }
    });
    let node_id = renderer
        .register_accessibility_node(node, bounds, env, action_target)
        .expect("a focusable node in bounds registers");
    renderer.register_accessibility_focus_link(key, node_id);
    node_id
}

/// Queues `spawn_local` futures for unit tests without running them.
///
/// `NativeExecutor` cannot be used here. On non-Apple targets it delegates to the
/// polyfill, whose `spawn_main_local` asserts it runs on the thread registered by
/// `start_main_executor` — a blocking entry point a test binary never calls, so
/// `MAIN_THREAD_ID` is never set and every spawn panics. Test threads are also
/// assigned by the harness, so no process-global main thread can be pinned.
///
/// This mirrors what the Apple path does in a test: `spawn_local` hands the work
/// to the main queue and returns, and a unit test never runs a main loop, so the
/// future is simply never polled. Runnables are parked in a channel the test
/// environment owns (`ParkedRunnables` below), so they are dropped when the
/// test's last `Environment` clone drops — while this thread's locals are still
/// alive — instead of inside thread-local teardown, where a task future whose
/// drop touches a dead thread-local aborts the process
/// (water-rs/hydrolysis#332). Do not run them inline — these futures re-enter
/// the renderer and its GPU work, which deadlocks when polled in the middle of
/// the render call that spawned them.
#[derive(Clone, Debug)]
struct TestLocalExecutor {
    parked_tx: mpsc::Sender<Runnable>,
}

/// Owns the queue [`TestLocalExecutor`] parks runnables into.
///
/// Stored in the environment by [`test_environment`], so the queue's lifetime
/// is the test's `Environment`: dropping the receiver empties the channel on
/// the owning thread. A schedule arriving after the owner is gone finds a dead
/// channel and takes the same bounded-leak path the headless executor uses for
/// its teardown race — the runnable cannot be dropped on the waker's thread
/// (async-task's `spawn_local` thread check) and must not wait for
/// thread-local teardown.
pub(super) struct ParkedRunnables {
    rx: mpsc::Receiver<Runnable>,
}

impl ParkedRunnables {
    pub(super) fn drain(&self) {
        while let Ok(runnable) = self.rx.try_recv() {
            runnable.run();
        }
    }
}

impl LocalExecutor for TestLocalExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let parked_tx = self.parked_tx.clone();
        let (runnable, task) = async_task::spawn_local(fut, move |runnable| {
            if let Err(unsent) = parked_tx.send(runnable) {
                std::mem::forget(unsent.0);
            }
        });
        runnable.schedule();
        task
    }
}

pub fn test_environment() -> Environment {
    let (parked_tx, parked_rx) = mpsc::channel();
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        TestLocalExecutor { parked_tx },
        waterui::task::RefreshRate::HEADLESS,
    ));
    let mut env = themed_test_environment();
    // The receiver's owner is the environment itself: it drops with the
    // test's last env clone, emptying the parked queue while thread-locals
    // are still alive.
    env.insert(ParkedRunnables { rx: parked_rx });
    env
}

/// The same environment, but without pinning this thread's local executor, so
/// that a [`HeadlessRuntime`](crate::HeadlessRuntime) built with it installs its
/// own draining one instead.
///
/// The first install wins, and building the environment happens before the
/// runtime that would install a real executor exists, so a test that needs
/// spawned work to actually *run* — a `GpuView`'s async `setup`, which never
/// completes on the parking [`TestLocalExecutor`] — has to leave the slot open.
pub fn pumped_test_environment() -> Environment {
    themed_test_environment()
}

fn themed_test_environment() -> Environment {
    test_environment_with_scheme(waterui::theme::ColorScheme::Light)
}

/// [`themed_test_environment`] with the installed colour scheme set to
/// `scheme` — the same install the runners perform, so the default colour
/// tokens are signals over it. A test passes a binding it owns when the
/// point is that a scheme flip re-resolves them, the way a platform
/// appearance flip does on a live runner.
pub fn test_environment_with_scheme(
    scheme: impl nami::signal::IntoComputed<waterui::theme::ColorScheme>,
) -> Environment {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let mut env = Environment::new();
    crate::theme::install_theme_tokens(&mut env, None, scheme);
    crate::localization::install(&mut env);
    // The runners seed the chord table; a test that mounts menus resolves
    // shortcuts through the same path (water-rs/hydrolysis#247).
    env.insert(crate::renderer::MenuShortcutRegistry::<
        crate::text::ParleyEngine,
    >::default());
    // `App::into_parts` installs the application's decided Close Window
    // chord; the test environment installs the same value over an empty
    // bar, so a Close Window item reads the free ⌘W / Ctrl+W.
    env.insert(waterui_controls::menu::CloseWindowChord::new(
        &nami::Computed::constant(Vec::new()),
        &env,
    ));
    env.insert(BadgeDrawLog(Rc::new(RefCell::new(Vec::new()))));
    env
}

/// The member layer ids of every backdrop membership the frame's nodes
/// hold, flush order — including the members inside filtered groups, which
/// the walk descends into.
#[cfg(hydrolysis_gpu_painter)]
pub fn material_layers(runtime: &crate::HeadlessRuntime) -> Vec<cherenkov::LayerId> {
    struct Members(Vec<cherenkov::LayerId>);
    impl<E: crate::text::TextEngine> mount::layers::LayerVisitor<E> for Members {
        fn node(
            &mut self,
            layers: &mount::layers::NodeLayers<E>,
            _world: kurbo::Affine,
            _alphas: &[f32],
        ) {
            if let Some(member) = layers.material_member() {
                self.0.push(member);
            }
        }
    }
    let mut members = Members(Vec::new());
    mount::layers::visit(&runtime.renderer().mount_roots(), &mut members);
    members.0
}

#[cfg(hydrolysis_gpu_painter)]
pub fn mounted_anchor_parent_children(
    runtime: &crate::HeadlessRuntime,
    scope: std::num::NonZeroU64,
    canvas: Option<cherenkov::LayerId>,
    child: cherenkov::LayerId,
) -> Option<(cherenkov::LayerId, Vec<cherenkov::LayerId>, bool)> {
    struct Parent {
        scope: std::num::NonZeroU64,
        canvas: Option<cherenkov::LayerId>,
        child: cherenkov::LayerId,
        children: Option<(cherenkov::LayerId, Vec<cherenkov::LayerId>, bool)>,
    }
    impl<E: crate::text::TextEngine> mount::layers::LayerVisitor<E> for Parent {
        fn node(
            &mut self,
            layers: &mount::layers::NodeLayers<E>,
            _world: kurbo::Affine,
            _alphas: &[f32],
        ) {
            if self.children.is_none()
                && let Some(children) =
                    layers.committed_anchor_parent_children(self.scope, self.canvas, self.child)
            {
                self.children = Some(children);
            }
        }
    }
    let window = window_mount(runtime);
    if window.window_children().contains(&child) {
        return Some((window.window().id(), window.window_children(), true));
    }
    let mut visitor = Parent {
        scope,
        canvas,
        child,
        children: None,
    };
    mount::layers::visit(&runtime.renderer().mount_roots(), &mut visitor);
    visitor.children
}

#[cfg(hydrolysis_gpu_painter)]
pub fn inner_layers(runtime: &crate::HeadlessRuntime) -> Vec<cherenkov::LayerId> {
    struct Inner(Vec<cherenkov::LayerId>);
    impl<E: crate::text::TextEngine> mount::layers::LayerVisitor<E> for Inner {
        fn node(
            &mut self,
            layers: &mount::layers::NodeLayers<E>,
            _world: kurbo::Affine,
            _alphas: &[f32],
        ) {
            if let Some(inner) = layers.inner_id() {
                self.0.push(inner);
            }
        }
    }
    let mut visitor = Inner(Vec::new());
    mount::layers::visit(&runtime.renderer().mount_roots(), &mut visitor);
    visitor.0
}

/// The window's mount — the test renders one window.
#[cfg(hydrolysis_gpu_painter)]
pub fn window_mount(
    runtime: &crate::HeadlessRuntime,
) -> &mount::Mount<cherenkov_gpu::Gpu, crate::text::ParleyEngine> {
    &runtime
        .renderer()
        .cherenkov_window
        .as_ref()
        .expect("the frame installed into a window")
        .mount
}

/// Swaps the window host's engine-surface link for `surface` and returns
/// the link it held: swapping in `Weak::new()` stands in for a surface
/// dropped during the window's commit, swapping the returned link back
/// restores it.
#[cfg(hydrolysis_gpu_painter)]
pub fn swap_host_surface(
    runtime: &mut crate::HeadlessRuntime,
    surface: Weak<cherenkov::Surface<cherenkov_gpu::Gpu>>,
) -> Weak<cherenkov::Surface<cherenkov_gpu::Gpu>> {
    std::mem::replace(
        &mut runtime
            .renderer_mut()
            .cherenkov_window
            .as_mut()
            .expect("the frame installed into a window")
            .host
            .surface,
        surface,
    )
}

/// The window's backdrop-group table — the test renders one window.
#[cfg(hydrolysis_gpu_painter)]
pub fn mounts(
    runtime: &crate::HeadlessRuntime,
) -> &mount::backdrop::MaterialBackdropGroups<cherenkov::BackdropGroup, crate::text::ParleyEngine> {
    window_mount(runtime).groups()
}

/// The engine's current backdrop-capture bytes.
#[cfg(hydrolysis_gpu_painter)]
pub fn capture_bytes(runtime: &crate::HeadlessRuntime) -> u64 {
    runtime
        .renderer()
        .cherenkov_window
        .as_ref()
        .expect("the frame installed into a window")
        .host
        .engine
        .memory()
        .expect("the render thread answers memory queries")
        .backdrop_captures
        .0
}

/// Every badge indicator rect the test theme was asked to draw, in window
/// coordinates. Tests read it back via `env.get::<BadgeDrawLog>()`.
#[derive(Clone, Default)]
pub struct BadgeDrawLog(pub Rc<RefCell<Vec<Rect>>>);

#[derive(Clone)]
struct EmitsDuringSignalSubscription<T> {
    subscribed: Rc<Cell<bool>>,
    snapshot: T,
    update: nami::watcher::Context<T>,
}

impl<T: Clone + 'static> Signal for EmitsDuringSignalSubscription<T> {
    type Output = T;
    type Guard = ();

    fn snapshot(&self) -> Self::Output {
        assert!(
            self.subscribed.get(),
            "animated signal must subscribe before reading its snapshot"
        );
        self.snapshot.clone()
    }

    fn identity(&self) -> Option<nami::SignalIdentity> {
        Some(nami::SignalIdentity::from_rc(&self.subscribed))
    }

    fn watch(
        &self,
        watcher: impl Fn(nami::watcher::Context<Self::Output>) + 'static,
    ) -> Self::Guard {
        self.subscribed.set(true);
        watcher(self.update.clone());
    }
}

fn registration_signal<T: Clone + 'static>(
    snapshot: T,
    update: nami::watcher::Context<T>,
) -> EmitsDuringSignalSubscription<T> {
    EmitsDuringSignalSubscription {
        subscribed: Rc::new(Cell::new(false)),
        snapshot,
        update,
    }
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "the snapshot is the signal's stored literal read back untouched; any drift means the read went through arithmetic it must not"
)]
fn subscribed_snapshot_preserves_registration_animation_metadata() {
    let signal = registration_signal(
        0.25,
        nami::watcher::Context::from(0.25).with(Animation::linear(Duration::from_millis(250))),
    );
    let metadata_replayed = Rc::new(Cell::new(false));

    let (subscription, snapshot) = super::signals::SubscribedSnapshot::new(&signal);
    subscription.activate({
        let metadata_replayed = Rc::clone(&metadata_replayed);
        move |update| {
            metadata_replayed.set(update.metadata().try_get::<Animation>().is_some());
        }
    });

    assert_eq!(snapshot, 0.25);
    assert!(signal.subscribed.get());
    assert!(metadata_replayed.get());
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "the snapshot is the signal's stored literal read back untouched; any drift means the read went through arithmetic it must not"
)]
fn wrapper_motion_subscribes_before_reading_its_snapshot() {
    let signal = registration_signal(0.25, nami::watcher::Context::from(0.25));
    let renderer = test_renderer();

    let motion = super::mount::motion::Motion::new(
        &signal,
        f64::from,
        renderer.frame_signals(),
        Rc::clone(renderer.placement_clock()),
        Box::new(|| {}),
        false,
    );

    assert_eq!(motion.target(), 0.25);
    assert!(signal.subscribed.get());
}

#[test]
fn labeled_toggle_keeps_label_activation_out_of_switch_visual_interaction() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let enabled = Binding::new(false);
    let bounds = Rect::new(0.0, 0.0, 160.0, 40.0);

    capture_root_window(
        &mut renderer,
        toggle("Enable Feature", &enabled).switch(),
        &env,
        bounds,
    );

    let label_target = renderer
        .hit_test
        .pointer_targets
        .iter()
        .find(|target| target.interaction.is_none())
        .expect("labeled toggle must register a visual-free label activation target")
        .clone();
    let switch_target = renderer
        .hit_test
        .pointer_targets
        .iter()
        .find(|target| target.interaction.is_some())
        .expect("labeled toggle must register a visual switch target")
        .clone();
    let switch_interaction = switch_target
        .interaction
        .as_ref()
        .expect("switch target interaction was checked")
        .clone();

    assert!(!label_target.keyboard_focusable);
    assert!(switch_target.keyboard_focusable);
    assert_eq!(label_target.bounds, bounds);
    assert!(
        switch_target.bounds.width() < label_target.bounds.width(),
        "switch visual feedback must be scoped to the control bounds"
    );

    let label_point = Point::new(10.0, 20.0);
    assert!(label_target.bounds.contains(label_point));
    assert!(!switch_target.bounds.contains(label_point));
    for expected in [true, false, true, false] {
        let _ = renderer.handle_pointer_down(
            crate::num_cast::f64_as_f32(label_point.x),
            crate::num_cast::f64_as_f32(label_point.y),
            PointerButton::Primary,
            &env,
        );
        assert!(
            !switch_interaction.pressing(),
            "label press must not set the switch's pressed chrome"
        );
        assert!(
            switch_interaction
                .sample_waves(renderer.frame_instant())
                .is_empty(),
            "label press must not spawn a switch ripple"
        );
        let _ = renderer.handle_pointer_up(
            crate::num_cast::f64_as_f32(label_point.x),
            crate::num_cast::f64_as_f32(label_point.y),
            PointerButton::Primary,
            &env,
        );
        assert_eq!(enabled.snapshot(), expected);
    }

    let switch_point = Point::new(
        f64::midpoint(switch_target.bounds.x0, switch_target.bounds.x1),
        f64::midpoint(switch_target.bounds.y0, switch_target.bounds.y1),
    );
    let _ = renderer.handle_pointer_down(
        crate::num_cast::f64_as_f32(switch_point.x),
        crate::num_cast::f64_as_f32(switch_point.y),
        PointerButton::Primary,
        &env,
    );
    assert!(
        switch_interaction.pressing(),
        "switch press must retain its local pressed chrome"
    );
    assert_eq!(
        switch_interaction
            .sample_waves(renderer.frame_instant())
            .last()
            .and_then(|wave| wave.origin),
        Some(switch_point),
        "switch ripple must originate inside the switch control"
    );
}

/// One bit of `source` as a two-way `Binding<bool>` — every call reuses this
/// one `Binding::mapping` call site, so the bindings it returns all carry the
/// same `SignalIdentity` (nami keys a mapping's identity by the source plus a
/// `#[track_caller]` call-site discriminator).
fn bit(source: &Binding<u8>, bit: u8) -> Binding<bool> {
    Binding::mapping(
        source,
        move |value| value & bit != 0,
        move |source, on| {
            source.set(if on {
                source.snapshot() | bit
            } else {
                source.snapshot() & !bit
            });
        },
    )
}

fn empty_selection_menu() -> nami::Computed<Vec<ResolvedMenuItem>> {
    nami::Computed::new(Vec::new())
}

fn text_field_model(value: &str, line_limit: Option<usize>) -> TextInputModel {
    TextInputModel::TextField {
        value: Binding::new(StyledStr::plain(value.to_owned())),
        line_limit,
        selection_menu: empty_selection_menu(),
        on_submit: None,
    }
}

fn secure_field_model(value: &str) -> TextInputModel {
    let mut secure = FormSecure::default();
    secure.set(value.to_owned());
    TextInputModel::SecureField {
        value: Binding::new(secure),
    }
}

fn text_input_target(
    model: TextInputModel,
    selection: Rc<RefCell<TextSelectionSlot>>,
) -> TextInputTarget<crate::text::ParleyEngine> {
    let interaction_key = InteractionKey::for_rc(&selection, 0);
    TextInputTarget {
        owner: std::rc::Weak::new(),
        interaction_key,
        modal: false,
        bounds: Rect::ZERO,
        frame: Rect::ZERO,
        cursor_area: Rect::ZERO,
        text_bounds: Rect::ZERO,
        text_clip_bounds: Rect::ZERO,
        content_alpha: 1.0,
        paints: None,
        layout: ParleyEngine::system(FontFamilyResolution::Strict).empty_layout(),
        display_text: waterui_core::Str::default(),
        display_layout: ParleyEngine::system(FontFamilyResolution::Strict).empty_layout(),
        purpose: TextInputPurpose::Normal,
        depth: 0,
        order: 0,
        model,
        selection,
        env: test_environment(),
        show_context_menu: HydrolysisRenderer::<ParleyEngine>::show_text_context_menu,
        key_handlers: None,
        focus_binding: None,
        #[cfg(feature = "accessibility")]
        accessibility_node_id: None,
    }
}

#[test]
fn a_stack_collects_alignment_keys_from_wrapper_layouts() {
    use waterui_layout::container::FixedContainer;
    let env = test_environment();
    let child = AnyView::new(().size(20.0, 10.0).horizontal_alignment_guide(
        HorizontalAlignment::Leading,
        |dimensions: &ViewDimensions| dimensions.size.width * 0.5,
    ));
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: Computed::constant(0.0),
    };
    let dimensions = Realized::new(FixedContainer::new(layout, vec![child]), &env)
        .measure(ProposalSize::UNSPECIFIED);

    assert_eq!(
        dimensions.explicit_horizontal(HorizontalAlignment::Leading),
        Some(10.0)
    );
}

/// A plain `FixedContainer` — before `body` wraps its layout in
/// `DirectionalLayout` — must still resolve explicit horizontal
/// guides against the placements `DirectionalLayout` produces: under RTL the
/// leading-aligned child sits on the trailing edge, so a guide anchored in
/// its frame mirrors to `width - offset`.
#[test]
fn plain_fixed_container_measurement_mirrors_guides_under_rtl() {
    use waterui_core::layout::LayoutDirection;
    use waterui_layout::container::FixedContainer;
    let mut env = test_environment();
    env.insert(LayoutDirection::RightToLeft);
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let view = AnyView::new(FixedContainer::new(
        VStackLayout {
            alignment: HorizontalAlignment::Leading,
            spacing: Computed::constant(0.0),
        },
        vec![
            AnyView::new(().size(20.0, 10.0).horizontal_alignment_guide(
                HorizontalAlignment::Leading,
                |dimensions: &ViewDimensions| dimensions.size.width * 0.5,
            )),
            AnyView::new(().size(100.0, 10.0)),
        ],
    ));
    let dimensions = Realized::with_theme(view, &env, &theme).measure(ProposalSize::UNSPECIFIED);

    // Under RTL the 20-wide leading child mirrors to the trailing edge —
    // placed at `width - 20`, so its mid-child guide resolves at
    // `width - 10` (unmirrored it would sit at 10) — the same value the
    // built tree's DirectionalLayout reports.
    assert_eq!(
        dimensions.explicit_horizontal(HorizontalAlignment::Leading),
        Some(dimensions.size.width - 10.0)
    );
}

#[test]
fn scale_metadata_is_layout_transparent() {
    let env = test_environment();
    let scale = Binding::new(1.0_f32);
    let mut realized = Realized::new(
        ().size(80.0, 80.0).scale(scale.clone()).min_height(120.0),
        &env,
    );

    let initial = realized.intrinsic();
    scale.set(2.0);
    let scaled = realized.intrinsic();

    assert_eq!(initial, LayoutSize::new(80.0, 120.0));
    assert_eq!(scaled, initial);
}

#[test]
fn a_stretch_control_keeps_its_minimum_under_a_zero_width_proposal() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let value = Binding::new(0.5_f64);
    let view = AnyView::new(slider("Playback position", &value).hide_label());
    let mut subview = Realized::with_theme(view, &env, &theme);

    let measured = subview.measure(ProposalSize::new(Some(0.0), None));

    assert!(
        measured.size.width > 0.0,
        "Hydrolysis stretch controls must preserve their intrinsic minimum under constrained measurement"
    );
}

/// A tighter proposal caps a button's minimum-size floor but never its
/// padded label: every proposal below that footprint answers it.
#[test]
fn a_compressed_button_keeps_its_padded_label() {
    let env = test_environment();
    let mut subview = Realized::new(button("Medium (0.7)").action(|| {}), &env);

    let intrinsic = subview.measure(ProposalSize::UNSPECIFIED);
    let zero = subview.measure(ProposalSize::new(Some(0.0), None));
    let narrow = subview.measure(ProposalSize::new(Some(10.0), None));

    assert!(
        zero.size.width > 0.0 && zero.size.width <= intrinsic.size.width,
        "the zero-width answer is the padded label, within the floored intrinsic: {zero:?} vs {intrinsic:?}"
    );
    assert!(
        approx::relative_eq!(narrow.size.width, zero.size.width),
        "Hydrolysis non-stretch controls must not be compressed below their padded label: left {:?}, right {:?}",
        narrow.size.width,
        zero.size.width
    );
    assert!(approx::relative_eq!(
        zero.size.height,
        intrinsic.size.height
    ));
}

#[test]
fn state_wrapped_button_remains_non_stretch_for_layout() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let expanded = Binding::new(false);
    let view = AnyView::new(
        button("Toggle Bars")
            .action(|waterui::State(value): waterui::State<Binding<bool>>| {
                value.toggle();
            })
            .state(&expanded),
    );
    let mut subview = Realized::with_theme(view, &env, &theme);

    let intrinsic = subview.measure(ProposalSize::UNSPECIFIED);
    let proposed = subview.measure(ProposalSize::new(Some(720.0), None));

    assert_eq!(subview.stretch_axis(), StretchAxis::None);
    assert!(
        approx::relative_eq!(proposed.size.width, intrinsic.size.width),
        "environment state metadata must not make a button stretch across its VStack row: left {:?}, right {:?}",
        proposed.size.width,
        intrinsic.size.width
    );
}

#[test]
fn vstack_places_state_wrapped_button_at_intrinsic_width() {
    let env = test_environment();
    let expanded = Binding::new(false);
    let view = vstack((
        hstack((
            ().size(50.0, 80.0).min_height(100.0).min_width(60.0),
            ().size(50.0, 80.0).min_height(100.0).min_width(60.0),
            ().size(50.0, 80.0).min_height(100.0).min_width(60.0),
            ().size(50.0, 80.0).min_height(100.0).min_width(60.0),
        )),
        button("Toggle Bars")
            .action(|waterui::State(value): waterui::State<Binding<bool>>| {
                value.toggle();
            })
            .state(&expanded),
    ));
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 720.0, 320.0);

    capture_root_window(&mut renderer, view, &env, bounds);

    let target = renderer
        .hit_test
        .pointer_targets
        .first()
        .expect("state-wrapped button should register a pointer target");
    assert!(
        target.bounds.width() < 200.0,
        "state-wrapped button hit bounds must stay intrinsic, got width {}",
        target.bounds.width()
    );
}

#[test]
fn floating_button_measurement_uses_style_tokens() {
    let env = test_environment();
    let floating_style = FloatingStyle {
        minimum_width: 37.0,
        minimum_height: 41.0,
        ..FloatingStyle::default()
    };

    let mut renderer = test_renderer();
    capture_root_window(
        &mut renderer,
        vstack((button(label("Token Sized").icon(()))
            .label_style(LabelDisplayMode::IconOnly)
            .plain()
            .action(|| {})
            .floating_with(floating_style),)),
        &env,
        Rect::new(0.0, 0.0, 160.0, 160.0),
    );
    let intrinsic_bounds = renderer
        .hit_test
        .pointer_targets
        .first()
        .expect("floating button must register an intrinsic pointer target")
        .bounds;
    approx::assert_relative_eq!(intrinsic_bounds.width(), 37.0);
    approx::assert_relative_eq!(intrinsic_bounds.height(), 41.0);
}

/// A button takes its size and chrome from floating-surface tokens only when it
/// is actually inside such a surface.
///
/// `FloatingStyle` in the environment is ambient theme data — it is what a bare
/// `.floating()` resolves against, so every themed app has one. A backend that
/// reads its presence as "I am inside a floating surface" reaches that
/// conclusion for every button in the app, and they all lose their own
/// container. The enclosing surface announces itself with `FloatingScope`
/// instead.
#[test]
fn button_outside_a_floating_surface_ignores_ambient_floating_tokens() {
    let env = test_environment();
    // Distinctive minimums: a button that wrongly adopted them is unmistakable.
    let ambient_tokens = FloatingStyle {
        minimum_width: 137.0,
        minimum_height: 141.0,
        ..FloatingStyle::default()
    };

    let mut renderer = test_renderer();
    capture_root_window(
        &mut renderer,
        vstack((button(label("Plain").icon(()))
            .label_style(LabelDisplayMode::IconOnly)
            .plain()
            .action(|| {}),))
        .install(ambient_tokens),
        &env,
        Rect::new(0.0, 0.0, 400.0, 400.0),
    );

    let bounds = renderer
        .hit_test
        .pointer_targets
        .first()
        .expect("button must register a pointer target")
        .bounds;
    assert!(
        bounds.width() < 137.0 && bounds.height() < 141.0,
        "a button merely in an app with floating tokens sized itself as a floating \
         surface ({}x{}); only a button inside `.floating()` may do that",
        bounds.width(),
        bounds.height()
    );
}

#[test]
fn stacked_icon_buttons_above_gesture_surface_receive_clicks() {
    let env = test_environment();
    let zoom: Binding<f64> = nami::binding(1.0);
    let zoom_in = button(label("Zoom In").icon(()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action(|waterui::State(value): waterui::State<Binding<f64>>| {
            *value.get_mut() *= 0.5;
        })
        .state(&zoom)
        .size(48.0, 48.0);
    let zoom_out = button(label("Zoom Out").icon(()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action(|waterui::State(value): waterui::State<Binding<f64>>| {
            *value.get_mut() *= 2.0;
        })
        .state(&zoom)
        .size(48.0, 48.0);
    let gesture_surface = Metadata::new(
        Metadata::new(
            ().size(160.0, 160.0),
            GestureObserver::new(DragGesture::new(0.0), |_: DragEvent| {}),
        ),
        GestureObserver::new(MagnificationGesture::new(1.0), |_: MagnificationEvent| {}),
    );
    let controls = vstack((zoom_in, Divider, zoom_out)).size(48.0, 97.0);
    let mut renderer = test_renderer();
    capture_root_window(
        &mut renderer,
        zstack((gesture_surface, controls)),
        &env,
        Rect::new(0.0, 0.0, 160.0, 160.0),
    );

    let mut buttons = renderer.hit_test.pointer_targets.clone();
    buttons.sort_by(|left, right| {
        left.bounds
            .y0
            .partial_cmp(&right.bounds.y0)
            .expect("finite button bounds must be ordered")
    });
    assert_eq!(buttons.len(), 2);
    let zoom_in = &buttons[0];
    let point = Point::new(
        f64::midpoint(zoom_in.bounds.x0, zoom_in.bounds.x1),
        f64::midpoint(zoom_in.bounds.y0, zoom_in.bounds.y1),
    );
    let _ = renderer.handle_pointer_down(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
        PointerButton::Primary,
        &env,
    );
    assert!(
        renderer.flush_window_tree(
            &env,
            Rect::new(0.0, 0.0, 160.0, 160.0),
            Affine::IDENTITY,
            Affine::IDENTITY,
        ),
        "pressed controls must survive a retained redraw before pointer release"
    );
    // The release moves only the button's live chrome operands, so it
    // reports no visual change of its own; the action's write is what
    // schedules the refresh asserted below.
    let _ = renderer.handle_pointer_up(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
        PointerButton::Primary,
        &env,
    );

    approx::assert_relative_eq!(zoom.snapshot(), 0.5);
    assert!(
        renderer.root_is_dirty(),
        "a synchronous button action must schedule a retained-tree refresh"
    );
}

#[test]
fn draggable_metadata_delivers_typed_payload_to_drop_destination() {
    use std::{cell::RefCell, rc::Rc};
    use waterui::Str;
    use waterui::prelude::hstack;

    let dropped = Rc::new(RefCell::new(None::<String>));
    let dropped_target = Rc::clone(&dropped);
    let view = hstack((
        ().size(60.0, 60.0).draggable(Str::from("🍎 Apple")),
        ().size(60.0, 60.0).drop_destination(move |text: Str| {
            *dropped_target.borrow_mut() = Some(text.to_string());
        }),
    ))
    .spacing(20.0);

    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 160.0, 80.0);

    capture_root_window(&mut renderer, view, &env, bounds);

    let _ = renderer.handle_pointer_down(30.0, 30.0, PointerButton::Primary, &env);
    assert!(renderer.handle_pointer_move(110.0, 30.0, &env));
    assert!(renderer.handle_pointer_up(110.0, 30.0, PointerButton::Primary, &env));
    assert_eq!(dropped.borrow().as_deref(), Some("🍎 Apple"));
}

#[test]
fn nested_gesture_observers_on_one_view_share_a_group() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = Metadata::new(
        Metadata::new(
            ().size(20.0, 10.0).opacity(0.5),
            GestureObserver::new(DragGesture::new(8.0), |_: DragEvent| {}),
        ),
        GestureObserver::new(MagnificationGesture::new(1.0), |_: MagnificationEvent| {}),
    );
    let node = RenderNode::build(AnyView::new(view), &env, &mut renderer);

    let (outer, inner) = node
        .observed_gesture_group()
        .expect("the outer gesture observer wraps the tree");
    let (inner, observed) = inner
        .observed_gesture_group()
        .expect("the inner gesture observer wraps the view");
    assert_eq!(outer, inner);
    assert_ne!(
        inner,
        Rc::as_ptr(&observed.core().cell) as usize,
        "the opacity wrapper is transparent to the group"
    );
}

#[test]
fn renderer_magnification_targets_outer_observer_in_stacked_gesture_chain() {
    use std::{cell::Cell, rc::Rc};
    use waterui_core::Metadata;

    let offset = Binding::new(0.0_f32);
    let scale = Binding::new(1.0_f32);
    let drag_hits = Rc::new(Cell::new(0u32));
    let magnify_hits = Rc::new(Cell::new(0u32));
    let view = {
        let transform = offset.zip(&scale);
        let canvas =
            Canvas::new(move |cx| cx.scope(transform, |_cx, _transform| {})).size(120.0, 120.0);
        let canvas = {
            let drag_hits = Rc::clone(&drag_hits);
            Metadata::new(
                canvas,
                GestureObserver::new(DragGesture::new(0.0), move |_: DragEvent| {
                    drag_hits.set(drag_hits.get() + 1);
                }),
            )
        };
        {
            let magnify_hits = Rc::clone(&magnify_hits);
            Metadata::new(
                canvas,
                GestureObserver::new(
                    MagnificationGesture::new(1.0),
                    move |_: MagnificationEvent| {
                        magnify_hits.set(magnify_hits.get() + 1);
                    },
                ),
            )
        }
    };

    let mut renderer = HydrolysisRenderer::with_engine(
        Rc::new(MinimalTestTheme::default()),
        ParleyEngine::system(FontFamilyResolution::Strict),
    );
    let env = test_environment();
    let bounds = kurbo::Rect::new(0.0, 0.0, 160.0, 160.0);
    capture_root_window(&mut renderer, view, &env, bounds);

    let point = kurbo::Point::new(60.0, 60.0);
    let debug_targets = renderer.gesture_engine.debug_targets_at(point);
    assert_eq!(
        debug_targets.len(),
        2,
        "expected stacked drag+magnification gesture targets at point, got {debug_targets:?}",
    );
    assert_eq!(debug_targets[0].2, debug_targets[1].2);

    assert!(renderer.apply_magnification_gesture(60.0, 60.0, 1.2, &env));
    assert_eq!(drag_hits.get(), 0);
    assert_eq!(magnify_hits.get(), 3);
}

#[test]
fn string_views_measure_through_body_recursion() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let proposal = ProposalSize::UNSPECIFIED;

    let raw =
        Realized::with_theme(AnyView::new(Str::from("Hydrolysis")), &env, &theme).measure(proposal);
    let borrowed = Realized::with_theme(AnyView::new("Hydrolysis"), &env, &theme).measure(proposal);
    let owned = Realized::with_theme(AnyView::new(String::from("Hydrolysis")), &env, &theme)
        .measure(proposal);
    let cow = Realized::with_theme(AnyView::new(Cow::Borrowed("Hydrolysis")), &env, &theme)
        .measure(proposal);

    assert_eq!(borrowed.size, raw.size);
    assert_eq!(owned.size, raw.size);
    assert_eq!(cow.size, raw.size);
}

#[test]
fn fixed_scroll_content_keeps_offscreen_children_registered() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = scroll(vstack((
        ().size(120.0, 600.0),
        button("Offscreen").action(|| {}),
    )));
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    capture_root_window(&mut renderer, view, &env, bounds);

    assert!(
        renderer
            .hit_test
            .pointer_targets
            .iter()
            .any(|target| target.bounds.y0 >= 600.0),
        "fixed scroll content must not unload offscreen children; only explicit lazy containers may virtualize children"
    );
}

fn capture_root_window<V: waterui_core::View, E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    view: V,
    env: &Environment,
    bounds: Rect,
) {
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(view),
        env,
        bounds,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
}

/// `docs/layout-spec.md` §6: a segmented picker answers the proposal width and
/// its intrinsic height, and its intrinsic width when the width is unbounded;
/// a menu picker answers its intrinsic size under any offer.
#[cfg(feature = "accessibility")]
#[test]
fn a_segmented_picker_takes_the_width_it_is_offered() {
    fn picked_width(view: impl waterui_core::View + 'static, window: Rect) -> f64 {
        let env = test_environment();
        let mut renderer = test_renderer();
        capture_root_window(&mut renderer, view, &env, window);
        let update = renderer
            .take_accessibility_tree_update()
            .expect("a picker must publish an accessibility tree");
        let (_, node) = update
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Mode"))
            .expect("the picker node must exist");
        node.bounds().expect("the picker must carry bounds").width()
    }
    let picker = |style| {
        let selection = Binding::new(0_i32);
        Picker::new(
            "Mode",
            vec![text("One").tag(0), text("Two").tag(1), text("Three").tag(2)],
            &selection,
        )
        .style(style)
    };
    let window = Rect::new(0.0, 0.0, 480.0, 200.0);

    // The retained node's own answers, straight from its measure: an
    // unbounded width proposal is the intrinsic width, and a finite one is
    // taken whole by a segmented picker only.
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let mut renderer = test_renderer();
    let mut hydro = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let mut measure = |style, width: Option<f32>| {
        let mut node = crate::widgets::controls::picker::PickerRenderState::from_config(
            waterui_core::backend::MapSlots::map_slots(
                picker(style).config(),
                waterui_core::backend::ChildSlot::from_view,
            ),
        );
        node.prebuild(&mut renderer, &env);
        crate::widgets::controls::picker::measure_picker_node(
            &node,
            ProposalSize::new(width, None),
            &mut hydro,
            &env,
            &theme,
        )
        .size
    };
    let segmented = measure(PickerStyle::Segmented, None);
    assert!(
        segmented.width < 300.0,
        "an unbounded width proposal answers the intrinsic width, got {}",
        segmented.width
    );
    let offered = measure(PickerStyle::Segmented, Some(300.0));
    approx::assert_relative_eq!(offered.width, 300.0);
    approx::assert_relative_eq!(offered.height, segmented.height);
    let menu = measure(PickerStyle::Menu, None);
    assert_eq!(measure(PickerStyle::Menu, Some(300.0)), menu);

    approx::assert_relative_eq!(
        picked_width(picker(PickerStyle::Segmented).width(300.0), window),
        300.0
    );
    approx::assert_relative_eq!(picked_width(picker(PickerStyle::Segmented), window), 480.0);
}

/// A composed container carrying a non-`Group` role is a real container in the
/// accessibility tree: a navigation bar/rail declares itself a tab list so
/// assistive technology can tell that the tabs inside belong together. Only the
/// `Group` role synthesized a node before, so every other container role — and
/// every label on a container — was silently dropped.
#[cfg(feature = "accessibility")]
#[test]
fn container_role_other_than_group_emits_its_own_node() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = hstack((text("Home"), text("Search")))
        .height(80.0)
        .background(Color::srgb(0, 0, 255))
        .a11y_label("Navigation")
        .a11y_role(AccessibilityRole::TabList);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("tab-list render must publish an accessibility tree");
    let tab_lists = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::TabList)
        .collect::<Vec<_>>();
    assert_eq!(
        tab_lists.len(),
        1,
        "the container must emit exactly one tab-list node"
    );
    let (_, tab_list) = tab_lists[0];
    assert_eq!(tab_list.label(), Some("Navigation"));
    let child_labels = tab_list
        .children()
        .iter()
        .map(|child_id| {
            update
                .nodes
                .iter()
                .find_map(|(id, node)| (id == child_id).then(|| node.label()))
                .flatten()
                .expect("tab-list child must have a label")
        })
        .collect::<Vec<_>>();
    assert_eq!(child_labels, ["Home", "Search"]);
}

/// A label on a container names the container itself; it must not be pasted onto
/// every leaf inside it, which announced "Navigation" once per tab.
#[cfg(feature = "accessibility")]
#[test]
fn container_label_without_role_names_the_container_only() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = vstack((text("First"), text("Second"))).a11y_label("Settings");

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("labelled container render must publish an accessibility tree");
    let labelled = update
        .nodes
        .iter()
        .filter(|(_, node)| node.label() == Some("Settings"))
        .collect::<Vec<_>>();
    assert_eq!(
        labelled.len(),
        1,
        "only the container may carry the container's label"
    );
    let (_, container) = labelled[0];
    assert_eq!(container.role(), AccessibilityNodeRole::Group);
    assert_eq!(container.children().len(), 2);
}

/// A naming scope collapses onto the single element it names, and the element
/// keeps the bounds it was actually placed in — not the labelled view's
/// assigned frame. Under the negotiated-placement contract a parent may stretch
/// a container past its own answer: a window's overlay places its base over the
/// whole bounds, so a root `view.size(8, 8)` is assigned the window while its
/// content lands in the resolved 8x8 box. Reporting the container's outer
/// bounds would announce a window-sized element around an 8x8 drawing.
#[cfg(feature = "accessibility")]
#[test]
fn a_naming_scope_keeps_the_elements_own_bounds_when_the_parent_stretched_it() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let recording = waterui_graphics::backend::Picture::record(|_scene| {});
    let picture = waterui_graphics::backend::Picture::new(
        waterui_core::layout::Size::new(24.0, 24.0),
        nami::constant(recording),
    );
    let view = waterui_layout::frame::Frame::new(picture)
        .width(8.0)
        .height(8.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Sized");

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a labelled root frame must publish an accessibility tree");
    let (_, node) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Sized"))
        .expect("the labelled element must exist");
    let bounds = node.bounds().expect("the element must carry bounds");
    assert!(
        (bounds.width() - 8.0).abs() < 0.5 && (bounds.height() - 8.0).abs() < 0.5,
        "the element must report the 8x8 box it was placed in, got {}x{}",
        bounds.width(),
        bounds.height(),
    );
}

/// water-rs/hydrolysis#221: naming metadata on a tap-wrapped leaf names the
/// *gesture's* node — the wrapped leaf must not repeat the claim as a second
/// `Button` at the same bounds, or assistive technology announces the same
/// element twice.
#[cfg(feature = "accessibility")]
#[test]
fn a_named_tap_leaf_emits_one_button() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let activations = Rc::new(RefCell::new(0usize));
    let view = text("Hi")
        .on_tap({
            let activations = Rc::clone(&activations);
            move || *activations.borrow_mut() += 1
        })
        .a11y_label("Go")
        .a11y_role(AccessibilityRole::Button);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a named tap must publish an accessibility tree");
    let buttons = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::Button)
        .collect::<Vec<_>>();
    assert_eq!(
        buttons.len(),
        1,
        "the named tap must emit exactly one Button node"
    );
    let (button_id, button) = buttons[0];
    assert_eq!(button.label(), Some("Go"));
    assert!(
        button.supports_action(AccessibilityAction::Click),
        "the Button must advertise the tap's Click activation"
    );
    renderer.handle_accessibility_action(
        AccessibilityActionRequest {
            action: AccessibilityAction::Click,
            target_node: *button_id,
            target_tree: AccessibilityTreeId::ROOT,
            data: None,
        },
        &env,
    );
    assert_eq!(
        *activations.borrow(),
        1,
        "activating the announced Button must run the tap action"
    );
}

/// water-rs/hydrolysis#221: when a container stands between the naming
/// metadata and a tappable leaf, the container claims the scope — and the
/// silenced tap's activation must still reach the announced node rather than
/// dying with the claim.
#[cfg(feature = "accessibility")]
#[test]
fn a_naming_container_keeps_the_silenced_taps_activation() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let activations = Rc::new(RefCell::new(0usize));
    let view = vstack((
        text("Hi").on_tap({
            let activations = Rc::clone(&activations);
            move || *activations.borrow_mut() += 1
        }),
        text("there"),
    ))
    .a11y_label("Go")
    .a11y_role(AccessibilityRole::Button);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a named container must publish an accessibility tree");
    let buttons = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::Button)
        .collect::<Vec<_>>();
    assert_eq!(
        buttons.len(),
        1,
        "the container claim must emit exactly one Button node"
    );
    let (button_id, button) = buttons[0];
    assert_eq!(button.label(), Some("Go"));
    assert!(
        button.supports_action(AccessibilityAction::Click),
        "the claimed Button must stay activatable"
    );
    renderer.handle_accessibility_action(
        AccessibilityActionRequest {
            action: AccessibilityAction::Click,
            target_node: *button_id,
            target_tree: AccessibilityTreeId::ROOT,
            data: None,
        },
        &env,
    );
    assert_eq!(
        *activations.borrow(),
        1,
        "Click on the claimed Button must run the delegated tap action"
    );
}

/// water-rs/hydrolysis#229: a container whose role names it from its content —
/// a tab, a checkbox, a link — computes its accessible name from descendant
/// text. Those text leaves must not also emit `Label` nodes or a screen reader
/// reads the same words twice; a descendant that is itself a control — a close
/// button, or an edit control like a text field — keeps its own node, and its
/// own label stays out of the claim's name.
#[cfg(feature = "accessibility")]
#[test]
fn a_container_named_by_its_content_consumes_text_but_not_controls() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let field_value = Binding::new(Str::from(""));
    let view = hstack((
        text("Shell"),
        field("Nickname", &field_value),
        button("Close tab").action(|| {}),
    ))
    .a11y_role(AccessibilityRole::Tab);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a tab container must publish an accessibility tree");
    let tabs = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::Tab)
        .collect::<Vec<_>>();
    assert_eq!(
        tabs.len(),
        1,
        "the container must emit exactly one tab node"
    );
    let (_, tab) = tabs[0];
    assert_eq!(
        tab.label(),
        Some("Shell"),
        "the tab's accessible name comes from its descendant text"
    );

    let buttons = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::Button)
        .collect::<Vec<_>>();
    assert_eq!(
        buttons.len(),
        1,
        "the nested close control stays exposed as its own node"
    );
    let (button_id, button) = buttons[0];
    assert_eq!(button.label(), Some("Close tab"));
    assert!(
        tab.children().contains(button_id),
        "the close button is a child of the tab it belongs to"
    );

    let fields = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::TextInput)
        .collect::<Vec<_>>();
    assert_eq!(
        fields.len(),
        1,
        "a text field inside the claim keeps its own node"
    );
    let (field_id, field_node) = fields[0];
    assert_eq!(field_node.label(), Some("Nickname"));
    assert!(
        tab.children().contains(field_id),
        "the text field is a child of the tab it belongs to"
    );

    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.role() != AccessibilityNodeRole::Label),
        "text the tab was named from must not emit label nodes: {:?}",
        update
            .nodes
            .iter()
            .map(|(_, node)| (node.role(), node.label().map(str::to_owned)))
            .collect::<Vec<_>>()
    );
}

/// water-rs/hydrolysis#229: an explicit `.a11y_label` still wins over the name
/// a container would compute from its descendant text — and the text is still
/// consumed, not emitted as `Label` nodes beside it.
#[cfg(feature = "accessibility")]
#[test]
fn a_container_with_an_explicit_label_still_consumes_its_text() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = hstack((text("Shell"), text("Beta")))
        .a11y_label("Pinned")
        .a11y_role(AccessibilityRole::Tab);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a labelled tab must publish an accessibility tree");
    let tabs = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::Tab)
        .collect::<Vec<_>>();
    assert_eq!(
        tabs.len(),
        1,
        "the container must emit exactly one tab node"
    );
    assert_eq!(
        tabs[0].1.label(),
        Some("Pinned"),
        "the explicit label wins over the computed name"
    );
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.role() != AccessibilityNodeRole::Label),
        "consumed text children must not emit label nodes: {:?}",
        update
            .nodes
            .iter()
            .map(|(_, node)| (node.role(), node.label().map(str::to_owned)))
            .collect::<Vec<_>>()
    );
}

/// A view hook wraps whatever it returns in a snapshot of the environment it was
/// called with; when that body resolves before the naming scope exists, the
/// snapshot carries no label, and flattening it replaces the scoped
/// environment the build installed. Every `Map` in a `map-gpu` build
/// and every `Video` in a `video-gpu` one sits under such a hook, and
/// `.a11y_label()` on them named nothing at all: the engine, finding no ambient
/// label, announced its own generated description instead.
#[cfg(feature = "accessibility")]
#[test]
fn a_label_reaches_the_view_a_hook_realizes() {
    use waterui::map::{Coordinate, Map, MapConfig, Region};
    use waterui_core::view::hook::Hook;

    /// Stands in for a map realization: this build has no map engine.
    #[derive(Clone)]
    struct TextMap;

    impl Hook for TextMap {
        type Target = MapConfig;

        fn apply(&self, _config: MapConfig, _env: &Environment) -> Result<impl View, MapConfig> {
            Ok(text("map").a11y_role(AccessibilityRole::Image))
        }
    }

    let env = test_environment();
    let mut renderer = test_renderer();
    // The frame puts the hooked map inside a layout container, one level below
    // the naming scope the build installs.
    let camera = Binding::new(Region::new(Coordinate::default(), 1.0, 1.0));
    let view = Map::new(&camera)
        .hooks(TextMap)
        .a11y_label("City map")
        .size(120.0, 120.0);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a hooked map render must publish an accessibility tree");
    let labelled = update
        .nodes
        .iter()
        .filter(|(_, node)| node.label() == Some("City map"))
        .collect::<Vec<_>>();
    assert_eq!(
        labelled.len(),
        1,
        "the hooked map must carry the caller's name, exactly once"
    );
    assert_eq!(labelled[0].1.role(), AccessibilityNodeRole::Image);
}

/// A `WebView` reaching a build bridging no engine fails cleanly through
/// `UnavailableWebViewController` rather than occupying a layout slot with no
/// page behind it.
#[cfg(not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview)))]
#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "no web engine is bridged"))]
fn a_webview_with_no_engine_to_draw_it_is_reported_and_not_drawn() {
    use waterui::webview::WebView;

    let env = test_environment();
    let mut renderer = test_renderer();
    let view = WebView::open("https://github.com/water-rs/waterui");

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));
}

/// A reactive collection is a container too. Rows are how a tab bar, a menu, or a
/// sidebar is actually written, so a container node that only appeared for fixed
/// tuple children would still leave every dynamic list unannounced.
#[cfg(feature = "accessibility")]
#[test]
fn lazy_stack_role_emits_the_container_node_over_its_rows() {
    use nami::collection::StateList;
    use waterui_core::id::SelfId;
    use waterui_layout::stack::VStack;

    let env = test_environment();
    let mut renderer = test_renderer();
    let rows: StateList<SelfId<u64>> = StateList::from(vec![SelfId::new(0), SelfId::new(1)]);
    let view = VStack::for_each(rows, |_: SelfId<u64>| text("Row"))
        .a11y_label("Rows")
        .a11y_role(AccessibilityRole::List);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("collection render must publish an accessibility tree");
    let (_, list) = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == AccessibilityNodeRole::List)
        .expect("the collection must emit its own list node");
    assert_eq!(list.label(), Some("Rows"));
    assert_eq!(list.children().len(), 2, "both rows belong to the list");
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("Rows")
                || node.role() == AccessibilityNodeRole::List),
        "the list's name must not be repeated on its rows"
    );
}

/// The non-virtualized collection path (an overlay stack) reaches a different node
/// kind than the lazy stack, and needs its own container node just the same.
#[cfg(feature = "accessibility")]
#[test]
fn overlay_collection_role_emits_the_container_node() {
    use nami::collection::StateList;
    use waterui_core::id::SelfId;
    use waterui_layout::stack::ZStack;

    let env = test_environment();
    let mut renderer = test_renderer();
    let items: StateList<SelfId<u64>> = StateList::from(vec![SelfId::new(0), SelfId::new(1)]);
    let view = ZStack::for_each(items, |_: SelfId<u64>| text("Card"))
        .a11y_label("Cards")
        .a11y_role(AccessibilityRole::Group);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("overlay collection render must publish an accessibility tree");
    let (_, group) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Cards"))
        .expect("the overlay collection must emit its own node");
    assert_eq!(group.role(), AccessibilityNodeRole::Group);
    assert_eq!(group.children().len(), 2);
}

/// A role on a control decorates the control's own node. The control consumes the
/// semantics first, so the containers composing its chrome must not emit a second
/// node carrying the same role and label.
#[cfg(feature = "accessibility")]
#[test]
fn control_consumes_its_role_before_the_containers_inside_it() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = button("Play")
        .action(|| {})
        .a11y_label("Play episode")
        .a11y_role(AccessibilityRole::Link);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("button render must publish an accessibility tree");
    let links = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::Link)
        .collect::<Vec<_>>();
    assert_eq!(
        links.len(),
        1,
        "the control owns the role; its chrome containers must not repeat it"
    );
    assert_eq!(links[0].1.label(), Some("Play episode"));
}

#[cfg(feature = "accessibility")]
#[test]
fn accessibility_group_owns_preserved_child_semantics() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = vstack((text("First"), text("Second")))
        .a11y_label("Settings")
        .a11y_role(AccessibilityRole::Group);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("group render must publish an accessibility tree");
    let (group_id, group) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Settings"))
        .expect("group must emit its own labelled node");
    assert_eq!(group.role(), AccessibilityNodeRole::Group);
    assert_eq!(group.children().len(), 2);
    let child_labels = group
        .children()
        .iter()
        .map(|child_id| {
            update
                .nodes
                .iter()
                .find_map(|(id, node)| (id == child_id).then(|| node.label()))
                .flatten()
                .expect("group child must have a text label")
        })
        .collect::<Vec<_>>();
    assert_eq!(child_labels, ["First", "Second"]);
    let root = update
        .nodes
        .iter()
        .find_map(|(id, node)| (*id == ACCESSIBILITY_ROOT_NODE_ID).then_some(node))
        .expect("accessibility tree must contain its root");
    assert_eq!(root.children(), &[*group_id]);
}

#[cfg(feature = "accessibility")]
#[test]
fn reactive_hidden_accessibility_group_suppresses_descendants() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let visible = Binding::new(false);
    let state = visible.map(|visible| AccessibilityState::new().hidden(!visible));
    let view = vstack((text("Hidden child"),))
        .a11y_label("Hidden group")
        .a11y_role(AccessibilityRole::Group)
        .a11y_state(state);

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("hidden group render must publish an accessibility tree");
    let (_, group) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Hidden group"))
        .expect("reactive hidden group must remain represented");
    assert!(group.is_hidden());
    assert_eq!(group.children(), []);
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("Hidden child"))
    );
}

#[cfg(feature = "accessibility")]
#[test]
fn mixed_busy_state_is_preserved_and_node_identity_survives_refresh() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let busy = Binding::new(false);
    let state = busy.map(|busy| AccessibilityState::new().mixed().busy(busy));
    let view = text("Tri-state")
        .a11y_role(AccessibilityRole::Checkbox)
        .a11y_state(state);
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    capture_root_window(&mut renderer, view, &env, bounds);
    let first = renderer
        .take_accessibility_tree_update()
        .expect("initial accessibility update must exist");
    let (first_id, first_node) = first
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Tri-state"))
        .expect("tri-state node must exist");
    assert_eq!(first_node.toggled(), Some(AccessibilityToggled::Mixed));
    assert!(!first_node.is_busy());

    busy.set(true);
    assert!(renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY));
    let second = renderer
        .take_accessibility_tree_update()
        .expect("refreshed accessibility update must exist");
    let (second_id, second_node) = second
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Tri-state"))
        .expect("refreshed tri-state node must exist");
    assert_eq!(
        first_id, second_id,
        "reactive refresh must preserve node identity"
    );
    assert_eq!(second_node.toggled(), Some(AccessibilityToggled::Mixed));
    assert!(second_node.is_busy());
}

#[cfg(feature = "accessibility")]
#[test]
fn reactive_a11y_hidden_toggles_leaf_and_container_without_rebuild() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let hidden = Binding::new(true);
    let view = vstack((
        text("Leaf").a11y_hidden(hidden.clone()),
        vstack((text("Inside A"), text("Inside B")))
            .a11y_label("Panel")
            .a11y_role(AccessibilityRole::Group)
            .a11y_hidden(hidden.clone()),
    ));
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    capture_root_window(&mut renderer, view, &env, bounds);
    let first = renderer
        .take_accessibility_tree_update()
        .expect("initial accessibility update must exist");
    assert!(
        first
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("Leaf")
                && node.label() != Some("Panel")
                && node.label() != Some("Inside A")
                && node.label() != Some("Inside B")),
        "nodes starting a11y_hidden(true) must not emit initially"
    );

    hidden.set(false);
    assert!(renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY));
    let second = renderer
        .take_accessibility_tree_update()
        .expect("unhidden accessibility update must exist");
    assert!(
        second
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Leaf")),
        "leaf hidden signal flipping false must emit its node"
    );
    assert!(
        second
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Panel")),
        "container hidden signal flipping false must emit its node"
    );
    assert!(
        second
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Inside A"))
            && second
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Inside B")),
        "container children must emit once container hidden signal flips false"
    );

    hidden.set(true);
    assert!(renderer.flush_window_tree(&env, bounds, Affine::IDENTITY, Affine::IDENTITY));
    let third = renderer
        .take_accessibility_tree_update()
        .expect("re-hidden accessibility update must exist");
    assert!(
        third
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("Leaf")
                && node.label() != Some("Panel")
                && node.label() != Some("Inside A")
                && node.label() != Some("Inside B")),
        "re-hiding via reactive signal must suppress leaf and container again"
    );
}

#[cfg(feature = "accessibility")]
#[test]
fn accessibility_identifier_is_consumed_by_only_one_descendant() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = vstack((text("First"), text("Second"))).a11y_id("settings.section");

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));
    let update = renderer
        .take_accessibility_tree_update()
        .expect("identifier render must publish an accessibility tree");
    let identified = update
        .nodes
        .iter()
        .filter(|(_, node)| node.author_id() == Some("settings.section"))
        .collect::<Vec<_>>();
    assert_eq!(
        identified.len(),
        1,
        "an identifier scope must have one nearest consumer"
    );
    assert_eq!(identified[0].1.label(), Some("First"));
}

/// The value channel rides beside the label on the same node: a labelled
/// formula keeps its spoken mathematics instead of the name silencing it.
#[cfg(feature = "accessibility")]
#[test]
fn a11y_value_lands_beside_the_label_on_the_same_node() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = text("𝑒^{𝑖𝜋} + 1 = 0")
        .a11y_label("Euler's identity")
        .a11y_value("e to the i pi plus 1 equals 0");

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("labelled text must publish an accessibility tree");
    let carrying = update
        .nodes
        .iter()
        .filter(|(_, node)| node.value() == Some("e to the i pi plus 1 equals 0"))
        .collect::<Vec<_>>();
    assert_eq!(carrying.len(), 1, "the value must land on exactly one node");
    assert_eq!(carrying[0].1.label(), Some("Euler's identity"));
}

/// A value on a container describes the container itself; like the label it
/// must not be pasted onto every leaf inside it.
#[cfg(feature = "accessibility")]
#[test]
fn container_value_describes_the_container_only() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = vstack((text("First"), text("Second"))).a11y_value("Two settings");

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("valued container render must publish an accessibility tree");
    let carrying = update
        .nodes
        .iter()
        .filter(|(_, node)| node.value() == Some("Two settings"))
        .collect::<Vec<_>>();
    assert_eq!(
        carrying.len(),
        1,
        "only the container may carry the container's value"
    );
    let (_, container) = carrying[0];
    assert_eq!(container.role(), AccessibilityNodeRole::Group);
    assert_eq!(container.children().len(), 2);
}

/// A control that publishes its own value keeps it; an explicit `.a11y_value`
/// is the override, exactly as `.a11y_label` is for the name.
#[cfg(feature = "accessibility")]
#[test]
fn a11y_value_overrides_a_controls_own_value() {
    use jiff::civil::Date;

    let env = test_environment();
    let mut renderer = test_renderer();
    let date = Binding::new(Date::new(2025, 1, 10).expect("valid test date"));
    let plain = DatePicker::new("Date", &date);
    let overridden = DatePicker::new("Override", &date).a11y_value("January tenth, 2025");
    let view = vstack((plain, overridden));

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 240.0, 240.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("date pickers must publish an accessibility tree");
    let values = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessibilityNodeRole::ComboBox)
        .map(|(_, node)| node.value())
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        [Some("2025-01-10"), Some("January tenth, 2025")],
        "the control's own value stands beside an explicit override"
    );
}

#[cfg(feature = "accessibility")]
#[test]
fn disabled_picker_family_and_tabs_expose_no_mutating_actions() {
    use jiff::civil::Date;

    let env = test_environment();
    let mut renderer = test_renderer();
    let picker_selection = Binding::new("Alpha");
    let date = Binding::new(Date::new(2025, 1, 10).expect("valid test date"));
    let color = Binding::new(Color::srgb(0, 0, 0));
    let tabs = Tabs::new(
        &Binding::new(0usize),
        (
            Tab::new("First tab", || NavigationView::new("First", text("One"))),
            Tab::new("Second tab", || NavigationView::new("Second", text("Two"))),
        ),
    );
    let view = vstack((
        Picker::new(
            "Letter",
            vec![text("Alpha").tag("Alpha"), text("Beta").tag("Beta")],
            &picker_selection,
        )
        .style(PickerStyle::Menu)
        .disabled(true),
        DatePicker::new("Date", &date).disabled(true),
        ColorPicker::new("Color", &color).disabled(true),
        tabs.disabled(true),
    ));

    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 480.0, 480.0));
    let update = renderer
        .take_accessibility_tree_update()
        .expect("disabled controls must publish an accessibility tree");
    let mut verified = 0;
    for (_, node) in &update.nodes {
        if matches!(
            node.role(),
            AccessibilityNodeRole::ComboBox
                | AccessibilityNodeRole::ListBoxOption
                | AccessibilityNodeRole::Button
                | AccessibilityNodeRole::Tab
        ) {
            assert!(
                node.is_disabled(),
                "disabled control node must be announced disabled"
            );
            assert!(!node.supports_action(AccessibilityAction::Click));
            assert!(!node.supports_action(AccessibilityAction::SetValue));
            verified += 1;
        }
    }
    assert!(
        verified >= 7,
        "expected picker, options, date, color, and tab nodes"
    );
}

#[test]
fn interaction_state_does_not_migrate_between_semantic_identities() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let first_owner = Rc::new(());
    let second_owner = Rc::new(());
    let first_key = InteractionKey::for_rc(&first_owner, 0);
    let second_key = InteractionKey::for_rc(&second_owner, 0);

    renderer.begin_rebuild_frame();
    let (_, slot, _) =
        renderer.bind_interaction_target(first_key, Rect::new(0.0, 0.0, 80.0, 80.0), &env);
    let at = renderer.frame_instant();
    renderer
        .hit_test
        .interaction
        .begin_press(&slot, Point::new(20.0, 20.0), None, at);
    renderer.finish_rebuild_frame();

    renderer.begin_rebuild_frame();
    let (state, _, handles) =
        renderer.bind_interaction_target(second_key, Rect::new(100.0, 100.0, 180.0, 180.0), &env);

    assert!(!state.contains(InteractionState::PRESSED));
    assert_eq!(handles.sample_waves(renderer.frame_instant()), []);
}

fn test_interaction_motion() -> InteractionMotion {
    InteractionMotion {
        hover_opacity: 0.08,
        focus_opacity: 0.12,
        pressed_opacity: 0.12,
        dragged_opacity: 0.16,
        hover_enter: Animation::linear(Duration::from_millis(15)),
        hover_exit: Animation::linear(Duration::from_millis(15)),
        focus_enter: Animation::linear(Duration::from_millis(15)),
        focus_exit: Animation::linear(Duration::from_millis(15)),
        press_fade_in: Animation::linear(Duration::from_millis(105)),
        press_fade_out: Animation::linear(Duration::from_millis(375)),
        press_grow: Animation::bezier(Duration::from_millis(450), 0.2, 0.0, 0.0, 1.0),
        press_shape: Animation::linear(Duration::from_millis(15)),
        minimum_press_duration: Duration::from_millis(225),
        touch_delay: Duration::from_millis(150),
    }
}

/// Regression guard for the retained-path state-layer loss: a began press must
/// surface through the re-bound interaction handles as a visible press
/// layer once the fade-in has run.
#[test]
fn began_press_samples_a_visible_press_layer_after_fade_in() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let motion = test_interaction_motion();

    renderer.begin_rebuild_frame();
    let (_, slot, _) =
        renderer.bind_interaction_target_with_motion(key.clone(), bounds, &env, motion.clone());
    let at = renderer.frame_instant();
    renderer
        .hit_test
        .interaction
        .begin_press(&slot, Point::new(20.0, 20.0), None, at);
    renderer.finish_rebuild_frame();

    // Advance past the press fade-in (105ms) and re-bind:
    // the sampled handles must carry a visible press layer, its origin, and
    // non-zero grow progress.
    let later = renderer
        .frame_instant()
        .checked_add(Duration::from_millis(200))
        .expect("test press deadline overflow");
    renderer.set_frame_instant(later);
    renderer.begin_rebuild_frame();
    let (state, _, handles) =
        renderer.bind_interaction_target_with_motion(key, bounds, &env, motion);
    assert!(
        state.contains(InteractionState::PRESSED),
        "held press must stay visually pressed"
    );
    let waves = handles.sample_waves(later);
    let wave = waves.last().expect("held press must sample a visible wave");
    assert!(
        wave.opacity > 0.0,
        "press layer must be visible after fade-in"
    );
    assert!(wave.progress > 0.0, "press ripple must have grow progress");
    assert_eq!(wave.origin, Some(Point::new(20.0, 20.0)));
}

#[test]
fn interaction_engine_resolves_focus_state() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);

    renderer.begin_rebuild_frame();
    let (state, _, _) = renderer.bind_focused_control_interaction_target(
        key,
        Rect::new(0.0, 0.0, 80.0, 80.0),
        &env,
        true,
        false,
        test_interaction_motion(),
    );

    assert!(state.contains(InteractionState::FOCUSED));
}

/// `press_progress` is the physical-press morph value: it animates towards
/// 1 while the press is held and — unlike a ripple wave's grow progress,
/// which holds its grown shape through the fade-out — plays back towards 0
/// on release (water-rs/waterui#1588).
#[test]
fn press_progress_animates_while_pressed_and_back_on_release() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let motion = test_interaction_motion();

    renderer.begin_rebuild_frame();
    let (_, slot, handles) =
        renderer.bind_interaction_target_with_motion(key.clone(), bounds, &env, motion.clone());
    renderer.finish_rebuild_frame();
    approx::assert_relative_eq!(handles.press_progress_at(renderer.frame_instant()), 0.0);

    // The press lands between flushes; the next bind resolves it as PRESSED
    // and the morph (15ms linear) is in flight.
    let pressed_at = renderer.frame_instant();
    renderer
        .hit_test
        .interaction
        .begin_press(&slot, Point::new(20.0, 20.0), None, pressed_at);

    renderer.begin_rebuild_frame();
    let (state, _, _) =
        renderer.bind_interaction_target_with_motion(key.clone(), bounds, &env, motion.clone());
    renderer.finish_rebuild_frame();
    assert!(state.contains(InteractionState::PRESSED));

    let mid_press = pressed_at + Duration::from_millis(8);
    renderer.set_frame_instant(mid_press);
    renderer.begin_rebuild_frame();
    let (_, _, handles) =
        renderer.bind_interaction_target_with_motion(key.clone(), bounds, &env, motion.clone());
    renderer.finish_rebuild_frame();
    let progress = handles.press_progress_at(mid_press);
    assert!(
        progress > 0.0 && progress < 1.0,
        "the press morph must be in flight while the press is held: {progress}",
    );

    // Once the animation has settled the held press reports 1.
    let held_at = pressed_at + Duration::from_millis(60);
    renderer.set_frame_instant(held_at);
    renderer.begin_rebuild_frame();
    let (_, _, handles) =
        renderer.bind_interaction_target_with_motion(key.clone(), bounds, &env, motion.clone());
    renderer.finish_rebuild_frame();
    approx::assert_relative_eq!(handles.press_progress_at(held_at), 1.0);

    // Release: the morph plays `press_shape` backwards from the release
    // instant — it must not snap back to 0 in one frame.
    renderer.hit_test.interaction.clear_all_presses(held_at);
    renderer.begin_rebuild_frame();
    let (state, _, _) =
        renderer.bind_interaction_target_with_motion(key.clone(), bounds, &env, motion.clone());
    renderer.finish_rebuild_frame();
    assert!(!state.contains(InteractionState::PRESSED));

    let mid_release = held_at + Duration::from_millis(8);
    renderer.set_frame_instant(mid_release);
    renderer.begin_rebuild_frame();
    let (_, _, handles) =
        renderer.bind_interaction_target_with_motion(key.clone(), bounds, &env, motion.clone());
    renderer.finish_rebuild_frame();
    let progress = handles.press_progress_at(mid_release);
    assert!(
        progress > 0.0 && progress < 1.0,
        "the morph must animate back on release instead of snapping: {progress}",
    );

    let settled_at = held_at + Duration::from_millis(60);
    renderer.set_frame_instant(settled_at);
    renderer.begin_rebuild_frame();
    let (_, _, handles) = renderer.bind_interaction_target_with_motion(key, bounds, &env, motion);
    renderer.finish_rebuild_frame();
    approx::assert_relative_eq!(handles.press_progress_at(settled_at), 0.0);
}

#[test]
fn interactive_pointer_target_activates_on_release_inside() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);
    let activations = Rc::new(Cell::new(0));
    let action_activations = Rc::clone(&activations);

    renderer.begin_rebuild_frame();
    let (_, press_slot, _) = renderer.bind_interaction_target(key, bounds, &env);
    renderer.register_interactive_pointer_target(bounds, press_slot, move |_, _, _| {
        action_activations.set(action_activations.get() + 1);
        true
    });

    assert!(renderer.handle_pointer_down(20.0, 20.0, PointerButton::Primary, &env));
    assert_eq!(activations.get(), 0, "pointer-down must not commit a click");
    assert!(renderer.handle_pointer_up(20.0, 20.0, PointerButton::Primary, &env));
    assert_eq!(activations.get(), 1);
}

#[test]
fn touch_press_delays_ripple_and_move_cancels_click() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);
    let activations = Rc::new(Cell::new(0));
    let action_activations = Rc::clone(&activations);

    renderer.begin_rebuild_frame();
    let (_, press_slot, handles) =
        renderer.bind_interaction_target_with_motion(key, bounds, &env, test_interaction_motion());
    renderer.register_interactive_pointer_target(bounds, press_slot, move |_, _, _| {
        action_activations.set(action_activations.get() + 1);
        true
    });
    let started = renderer.frame_instant();
    // The host reports the touch-gesture parameters a real touch
    // platform publishes — a touch press without them panics.
    renderer.set_touch_scroll_config(Some(crate::platform::TouchScrollConfig::android_default()));

    assert!(!renderer.handle_pointer_down_with_source(
        7,
        PointerKind::Touch,
        20.0,
        20.0,
        PointerButton::Primary,
        &env,
    ));
    assert!(!handles.pressing(), "touch ripple must wait for its delay");
    renderer.set_frame_instant(
        started
            .checked_add(Duration::from_millis(149))
            .expect("test timestamp overflow"),
    );
    assert!(renderer.advance_animations());
    assert!(!handles.pressing());
    assert!(!renderer.handle_pointer_move_with_source(7, PointerKind::Touch, 21.0, 20.0, &env,));
    assert!(!renderer.handle_pointer_up_with_source(
        7,
        PointerKind::Touch,
        21.0,
        20.0,
        PointerButton::Primary,
        &env,
    ));
    assert_eq!(activations.get(), 0);
    assert!(!handles.pressing());
}

#[test]
fn keyboard_focus_activates_control_on_key_release() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);
    let activations = Rc::new(Cell::new(0));
    let action_activations = Rc::clone(&activations);

    renderer.begin_rebuild_frame();
    let (_, press_slot, handles) = renderer.bind_interaction_target(key.clone(), bounds, &env);
    renderer.register_interactive_pointer_target(bounds, press_slot, move |_, _, _| {
        action_activations.set(action_activations.get() + 1);
        true
    });
    #[cfg(feature = "accessibility")]
    let semantic_activations = Rc::new(Cell::new(0));
    #[cfg(feature = "accessibility")]
    {
        let counter = Rc::clone(&semantic_activations);
        let activation: AccessibilityActivation<crate::text::ParleyEngine> = Rc::new(RefCell::new(
            move |_: &mut SemanticCore<crate::text::ParleyEngine>, _: &Environment| {
                counter.set(counter.get() + 1);
                true
            },
        ));
        emit_focusable_node(&mut renderer, &key, bounds, &env, Some(activation));
    }

    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Tab".to_owned()),
        Modifiers::default(),
        &env,
    ));
    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Enter".to_owned()),
        Modifiers::default(),
        &env,
    ));
    // The rendered contract is the pointer contract: key-down presses and
    // holds the affordance, and the control activates on key-up.
    assert_eq!(activations.get(), 0, "key-down must not activate");
    assert!(handles.pressing(), "key-down holds the pressed affordance");
    #[cfg(feature = "accessibility")]
    assert_eq!(
        semantic_activations.get(),
        0,
        "a rendered runtime dispatches no Click"
    );
    assert!(renderer.handle_key_release_with_env(&KeyCode::Named("Enter".to_owned()), &env,));
    assert_eq!(activations.get(), 1);
    assert!(!handles.pressing(), "key-up releases the affordance");
}

/// water-rs/hydrolysis#211: winit synthesizes a release for every held key
/// when the window loses focus. That release aborts the press it belonged
/// to — the armed target drops without firing and the pressed affordance
/// comes down — so a real release arriving later finds nothing stale.
#[test]
fn synthetic_focus_release_cancels_armed_keyboard_press() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);
    let activations = Rc::new(Cell::new(0));
    let action_activations = Rc::clone(&activations);

    renderer.begin_rebuild_frame();
    let (_, press_slot, handles) = renderer.bind_interaction_target(key.clone(), bounds, &env);
    renderer.register_interactive_pointer_target(bounds, press_slot, move |_, _, _| {
        action_activations.set(action_activations.get() + 1);
        true
    });
    #[cfg(feature = "accessibility")]
    emit_focusable_node(&mut renderer, &key, bounds, &env, None);

    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Tab".to_owned()),
        Modifiers::default(),
        &env,
    ));
    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Enter".to_owned()),
        Modifiers::default(),
        &env,
    ));
    assert!(handles.pressing(), "key-down holds the pressed affordance");

    // Focus-out: the synthetic release cancels the press without activating.
    assert!(renderer.cancel_keyboard_press());
    assert_eq!(activations.get(), 0, "a cancelled press never activates");
    assert!(
        !handles.pressing(),
        "the cancel releases the pressed affordance"
    );

    // The real release of the same key then has nothing armed to fire.
    assert!(
        !renderer.handle_key_release_with_env(&KeyCode::Named("Enter".to_owned()), &env,),
        "no stale target may activate on the real release"
    );
    assert_eq!(activations.get(), 0);

    // A fresh press-release pair still activates exactly once.
    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Enter".to_owned()),
        Modifiers::default(),
        &env,
    ));
    assert!(renderer.handle_key_release_with_env(&KeyCode::Named("Enter".to_owned()), &env,));
    assert_eq!(activations.get(), 1);
    assert!(!handles.pressing());
}

#[test]
fn interaction_focus_binding_tracks_keyboard_focus() {
    let mut renderer = test_renderer();
    let mut env = test_environment();
    let focused = Binding::new(false);
    env.insert(InteractionFocusBinding::new(&focused));
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);

    renderer.begin_rebuild_frame();
    let (_, press_slot, _) = renderer.bind_interaction_target(key.clone(), bounds, &env);
    renderer.register_interactive_pointer_target(bounds, press_slot, |_, _, _| true);
    #[cfg(feature = "accessibility")]
    emit_focusable_node(&mut renderer, &key, bounds, &env, None);

    assert!(!focused.snapshot());
    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Tab".to_owned()),
        Modifiers::default(),
        &env,
    ));
    assert!(focused.snapshot());
    assert!(renderer.set_keyboard_focus(None, false));
    assert!(!focused.snapshot());
}

#[test]
fn modal_scope_traps_keyboard_focus_and_handles_escape() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let mut modal_env = env.clone();
    let escape_activations = Rc::new(Cell::new(0));
    let escape_action_activations = Rc::clone(&escape_activations);
    modal_env.insert(ModalInteraction::new(
        true,
        SharedAction::new(move |_: Environment| {
            escape_action_activations.set(escape_action_activations.get() + 1);
        }),
    ));
    let background_owner = Rc::new(());
    let background_key = InteractionKey::for_rc(&background_owner, 0);
    let modal_owner = Rc::new(());
    let modal_key = InteractionKey::for_rc(&modal_owner, 0);
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);

    renderer.begin_rebuild_frame();
    let (_, background_press_slot, _) =
        renderer.bind_interaction_target(background_key.clone(), bounds, &env);
    renderer.register_interactive_pointer_target(bounds, background_press_slot, |_, _, _| true);
    let (_, modal_press_slot, _) =
        renderer.bind_interaction_target(modal_key.clone(), bounds, &modal_env);
    renderer.register_interactive_pointer_target(bounds, modal_press_slot, |_, _, _| true);
    #[cfg(feature = "accessibility")]
    {
        emit_focusable_node(&mut renderer, &background_key, bounds, &env, None);
        emit_focusable_node(&mut renderer, &modal_key, bounds, &modal_env, None);
    }

    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Tab".to_owned()),
        Modifiers::default(),
        &modal_env,
    ));
    assert_eq!(renderer.hit_test.keyboard_focus, Some(modal_key));
    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Escape".to_owned()),
        Modifiers::default(),
        &modal_env,
    ));
    assert_eq!(escape_activations.get(), 1);
}

#[test]
fn inactive_modal_scope_does_not_trap_keyboard_focus() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let mut dialog_env = env;
    let active = Binding::new(false);
    dialog_env.insert(
        ModalInteraction::new(false, SharedAction::new(|_: Environment| {})).active(active),
    );
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);

    renderer.begin_rebuild_frame();
    let (_, press_slot, _) = renderer.bind_interaction_target(key.clone(), bounds, &dialog_env);
    assert!(!press_slot.modal);
    renderer.register_interactive_pointer_target(bounds, press_slot, |_, _, _| true);
    #[cfg(feature = "accessibility")]
    emit_focusable_node(&mut renderer, &key, bounds, &dialog_env, None);

    assert!(renderer.handle_key_with_env(
        &KeyCode::Named("Tab".to_owned()),
        Modifiers::default(),
        &dialog_env,
    ));
    assert_eq!(renderer.hit_test.keyboard_focus, Some(key));
    assert!(renderer.hit_test.modal_interaction.is_none());
}

/// The switch look a test theme hands out for a toggle spec.
type SwitchLookFor = Rc<dyn Fn(&ToggleSpec, &Environment) -> SwitchLook>;

/// The slider look a test theme hands out for a slider spec.
type SliderLookFor = Rc<dyn Fn(&SliderSpec, &Environment) -> SliderLook>;

/// The stepper look a test theme hands out for a stepper spec.
type StepperLookFor = Rc<dyn Fn(&StepperSpec, &Environment) -> StepperLook>;
/// The text field look a test theme hands out for a field spec.
type TextFieldLookFor = Rc<dyn Fn(&TextFieldSpec, &Environment) -> TextFieldLook>;

/// The segmented look a test theme hands out for a segmented picker spec.
type SegmentedLookFor =
    Rc<dyn Fn(&crate::theme::SegmentedSpec, &Environment) -> crate::theme::SegmentedLook>;

/// The glass look a test theme hands out for a glass background.
type GlassLookFor =
    Rc<dyn Fn(&waterui::background::Glass, &Environment) -> Option<crate::theme::GlassLook>>;

/// A linear progress bar the theme realized: `(frame, value)`.
type ProgressBarRealization = (Rect, f64);
/// A radio indicator the theme realized: `(frame, selected)`.
type RadioIndicatorRealization = (Rect, bool);

/// How a test theme's indeterminate progress looks move.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum IndeterminateProgress {
    /// Plain controls with no moving parts.
    #[default]
    Still,
    /// A bar that travels, a tick that turns, a square that swells.
    Moving,
    /// As [`Self::Moving`], the tick turning with two nested rotation layers.
    Nested,
    /// The linear look is a bar whose colour keyframes are built from the
    /// colour scheme.
    Scheme,
}

impl IndeterminateProgress {
    /// Whether the looks travel, turn and swell.
    const fn moves(self) -> bool {
        matches!(self, Self::Moving | Self::Nested)
    }
}

#[derive(Default)]
pub struct MinimalTestTheme {
    badge_draws: Rc<RefCell<Vec<Rect>>>,
    /// Every navigation-bar surface bounds the theme was asked to draw —
    /// the painted geometry §7.1's chrome extension lives in.
    navigation_bar_draws: Rc<RefCell<Vec<Rect>>>,
    /// Every navigation-bar separator bounds the theme was asked to draw.
    navigation_bar_separator_draws: Rc<RefCell<Vec<Rect>>>,
    /// Every tabs-bar surface bounds the theme was asked to draw.
    tabs_bar_draws: Rc<RefCell<Vec<Rect>>>,
    /// Every tabs-bar divider bounds the theme was asked to draw.
    tabs_bar_divider_draws: Rc<RefCell<Vec<Rect>>>,
    /// Forces the tab item layout the theme's look reports; `None` is
    /// `Vertical`, the look's layout when it names no horizontal width.
    forced_tab_item_layout: Option<TabItemLayout>,
    /// `(bounds, layout)` for every highlight the selected tab item records.
    tabs_highlight_draws: Rc<RefCell<Vec<(Rect, TabItemLayout)>>>,
    /// When set, the tabs look lays its items out horizontally from this bar
    /// extent up — a width-class theme like M3's medium-width boundary.
    horizontal_from_width: Option<f64>,
    /// Optional explicit [`TabsLook`] override for tests.
    tabs_look: Option<TabsLook>,
    /// Optional explicit inline title font override for [`NavigationBarLook`].
    navigation_title_font: Option<Font>,
    /// Every frame the slider's value-indicator bubble records on.
    slider_indicator_draws: Rc<RefCell<Vec<Rect>>>,
    /// Every spec the theme was asked for a slider look.
    slider_specs: Rc<RefCell<Vec<SliderSpec>>>,
    /// Every frame the slider's track part records on.
    slider_track_draws: Rc<RefCell<Vec<Rect>>>,
    /// The drag mapping the slider look answers; `Absolute` when unset.
    slider_drag: Option<DragMapping>,
    /// Whether the slider look draws no label, so reserves no band for one.
    slider_without_label: bool,
    /// The slider look the theme hands out for a spec and environment;
    /// `None` answers the fixture look the slider tests above read back.
    slider_look: Option<SliderLookFor>,
    /// The backdrop-material plan a chrome test registers
    /// (water-rs/waterui#1788): the registry entries, and the materials the
    /// plan's [`ChromePlan::chrome`] replays.
    chrome: ChromePlan,
    /// The switch look the theme hands out for a spec and environment;
    /// `None` answers a plain control with no parts.
    switch_look: Option<SwitchLookFor>,
    /// The stepper look the theme hands out for a spec and environment;
    /// `None` answers [`crate::testing::plain_stepper_look`].
    stepper_look: Option<StepperLookFor>,
    /// The segmented look the theme hands out for a spec and environment;
    /// `None` answers [`crate::testing::plain_segmented_look`].
    segmented_look: Option<SegmentedLookFor>,
    /// The glass look the theme hands out for a glass background; `None`
    /// answers the trait's default, no glass.
    glass_look: Option<GlassLookFor>,
    /// Every spec the theme was asked for a stepper look.
    stepper_specs: Rc<RefCell<Vec<StepperSpec>>>,
    /// Every linear progress bar the theme's look realized, with the value
    /// its part read.
    progress_bar_realizations: Rc<RefCell<Vec<ProgressBarRealization>>>,
    /// How many times a linear progress bar's part was recorded.
    progress_bar_recordings: Rc<std::cell::Cell<u32>>,
    /// Every spec the theme was asked for a progress look.
    progress_specs: Rc<RefCell<Vec<ProgressSpec>>>,
    /// How the indeterminate progress looks move.
    indeterminate_progress: IndeterminateProgress,
    /// When set, `circular_progress` suppresses visible label bands.
    circular_without_labels: bool,
    /// Every radio indicator the theme realized.
    radio_indicator_realizations: Rc<RefCell<Vec<RadioIndicatorRealization>>>,
    /// The text field look the theme hands out for a spec and environment;
    /// `None` answers [`crate::testing::plain_text_field_look`] over
    /// `input_metrics`.
    text_field_look: Option<TextFieldLookFor>,
    /// Every spec the theme was asked for a text field look.
    text_field_specs: Rc<RefCell<Vec<TextFieldSpec>>>,
    /// The `InputFieldMetrics` the plain text field look lays out by; `None`
    /// answers [`crate::testing::plain_input_metrics`].
    input_metrics: Option<InputFieldMetrics>,
    /// The chrome the button look carries; `None` answers
    /// [`invisible_chrome`].
    button_chrome: Option<Chrome>,
    /// The label styling the button look carries; `None` keeps the
    /// label's own.
    button_label: Option<Label>,
    /// The button look's group opacity; `None` draws it opaque.
    button_group_opacity: Option<crate::theme::Driven<f32>>,
    /// The space the button look's group composites in.
    button_group_blend_space: cherenkov_record::BlendSpace,
    /// Content shaders registered by `register_backdrop_shaders`.
    content_shaders: Vec<(ContentShader, ContentShaderSource)>,
    /// Optional override for the menu look returned by `WidgetTheme::menu`.
    #[cfg(feature = "menu")]
    menu_look: Option<MenuLook>,
    /// Optional override for the list look returned by `WidgetTheme::list`.
    #[cfg(feature = "list")]
    list_look: Option<ListLook>,
}

/// The caret blink the test themes' text field looks carry.
pub fn test_caret_motion() -> TextCaretMotion {
    crate::testing::plain_text_field_look(invisible_chrome(), crate::testing::plain_input_metrics())
        .caret()
        .blink()
}

/// A chrome that draws nothing at rest and tints nothing on interaction —
/// the button look a test theme with no chrome of its own hands out. A
/// touch press waits the theme's `touch_delay` before it shows.
pub fn invisible_chrome() -> Chrome {
    Chrome::new(
        Surface::clear(CornerShape::radius(0.0)),
        StateLayer::new(waterui::graphics::Color::new(
            waterui::theme::color::Foreground,
        )),
        PressFeedback::new(),
        FocusIndicator::Tint,
        Motion::uniform(Animation::linear(Duration::from_millis(15))),
    )
    .hit(HitRegion::NONE.press_delay(Duration::from_millis(150)))
}

impl MinimalTestTheme {
    /// Hands out the look `look` makes for each text field's spec.
    #[must_use]
    pub fn with_text_field(
        mut self,
        look: impl Fn(&TextFieldSpec, &Environment) -> TextFieldLook + 'static,
    ) -> Self {
        self.text_field_look = Some(Rc::new(look));
        self
    }

    /// Lays the plain text field look out by `metrics` instead of the
    /// fixture defaults.
    #[must_use]
    pub fn with_input_metrics(mut self, metrics: InputFieldMetrics) -> Self {
        self.input_metrics = Some(metrics);
        self
    }

    /// Puts `chrome` on the button look, so buttons realize it with live
    /// operands.
    #[must_use]
    pub fn with_button_chrome(mut self, chrome: Chrome) -> Self {
        self.button_chrome = Some(chrome);
        self
    }

    /// Hands out `look` for every tab container.
    #[must_use]
    pub fn with_tabs_look(mut self, look: TabsLook) -> Self {
        self.tabs_look = Some(look);
        self
    }

    /// Hands out `look` for every switch.
    #[must_use]
    pub fn with_switch_look(self, look: SwitchLook) -> Self {
        self.with_switch(move |_, _| look.clone())
    }

    /// Hands out the look `look` makes for each slider's spec.
    #[must_use]
    pub fn with_slider(
        mut self,
        look: impl Fn(&SliderSpec, &Environment) -> SliderLook + 'static,
    ) -> Self {
        self.slider_look = Some(Rc::new(look));
        self
    }

    /// Hands out the look `look` makes for each switch's spec.
    #[must_use]
    pub fn with_switch(
        mut self,
        look: impl Fn(&ToggleSpec, &Environment) -> SwitchLook + 'static,
    ) -> Self {
        self.switch_look = Some(Rc::new(look));
        self
    }

    /// Hands out the look `look` makes for each segmented picker's spec.
    #[must_use]
    pub fn with_segmented(
        mut self,
        look: impl Fn(&crate::theme::SegmentedSpec, &Environment) -> crate::theme::SegmentedLook
        + 'static,
    ) -> Self {
        self.segmented_look = Some(Rc::new(look));
        self
    }

    /// Hands out `look` for every stepper.
    #[must_use]
    pub fn with_stepper_look(mut self, look: StepperLook) -> Self {
        self.stepper_look = Some(Rc::new(move |_, _| look.clone()));
        self
    }

    /// Hands out the glass look `look` makes for each glass background.
    #[must_use]
    #[cfg_attr(
        not(hydrolysis_gpu_painter),
        expect(
            dead_code,
            reason = "its tests mirror the GPU painter's backdrop layers and build only with it"
        )
    )]
    pub fn with_glass_background(
        mut self,
        look: impl Fn(&waterui::background::Glass, &Environment) -> Option<crate::theme::GlassLook>
        + 'static,
    ) -> Self {
        self.glass_look = Some(Rc::new(look));
        self
    }

    /// Registers a content-effect shader on the theme's [`MaterialRegistry`].
    #[cfg_attr(
        not(hydrolysis_gpu_painter),
        expect(
            dead_code,
            reason = "content-effect tests mirror GPU/CPU shader layers and build with the GPU painter"
        )
    )]
    #[must_use]
    pub fn with_content_shader(mut self, key: ContentShader, source: ContentShaderSource) -> Self {
        self.content_shaders.push((key, source));
        self
    }

    /// Styles the button look's label with `label`.
    #[must_use]
    pub fn with_button_label(mut self, label: Label) -> Self {
        self.button_label = Some(label);
        self
    }

    /// Draws the button look as one group at `opacity`.
    #[must_use]
    pub fn with_button_group_opacity(mut self, opacity: crate::theme::Driven<f32>) -> Self {
        self.button_group_opacity = Some(opacity);
        self
    }

    /// Composites the button look's group in `space`.
    #[must_use]
    pub const fn with_button_group_blend_space(
        mut self,
        space: cherenkov_record::BlendSpace,
    ) -> Self {
        self.button_group_blend_space = space;
        self
    }

    /// Hands out `look` for every menu.
    #[cfg(feature = "menu")]
    #[must_use]
    pub fn with_menu_look(mut self, look: MenuLook) -> Self {
        self.menu_look = Some(look);
        self
    }

    /// Hands out `look` for every list.
    #[cfg(feature = "list")]
    #[must_use]
    pub fn with_list_look(mut self, look: ListLook) -> Self {
        self.list_look = Some(look);
        self
    }
}

/// One `backdrop_material` a chrome test's plan replays each time its
/// chrome records (water-rs/waterui#1788).
#[derive(Clone)]
pub struct ChromeDraw {
    /// The member's live clip shape — a signal for the signal-rebind test.
    pub shape: SharedLive<kurbo::RoundedRect>,
    /// The shader key the material names.
    pub shader: MaterialShader,
    /// The capture class the material names.
    pub capture: CaptureClass,
    /// The member's live effect — its uniforms.
    pub effect: SharedLive<MaterialEffect>,
}

/// A [`ContainerRecipe`] a [`ChromePlan`] shares with the registry it
/// fills: the plan is `Clone`, the registry takes its recipe by value.
#[derive(Debug)]
struct SharedRecipe(Rc<dyn ContainerRecipe>);

impl ContainerRecipe for SharedRecipe {
    fn capture(&self, members: &[GroupMember<'_>]) -> CaptureClass {
        self.0.capture(members)
    }

    fn effects(&self, members: &[GroupMember<'_>]) -> Vec<MaterialEffect> {
        self.0.effects(members)
    }
}

/// The backdrop-material terms a chrome test declares: the registry
/// `register_backdrop_shaders` fills, and the materials its
/// [`chrome`](Self::chrome) replays.
#[derive(Clone, Default)]
pub struct ChromePlan {
    /// `(key, source)` pairs the registry registers.
    pub shaders: Vec<(MaterialShader, BackdropShaderSource)>,
    /// `(key, capture)` pairs the registry registers.
    pub captures: Vec<(CaptureClass, MaterialCapture)>,
    /// `(shader, recipe)` pairs the registry registers as container
    /// recipes (water-rs/waterui#2834).
    pub containers: Vec<(MaterialShader, Rc<dyn ContainerRecipe>)>,
    /// The materials the chrome's custom part records.
    pub draws: Vec<ChromeDraw>,
    /// Scene fills the custom part records after the materials — each
    /// lands in the segment above the last member, so a test can mount a
    /// keyed mount and a scene segment over it in one program
    /// (water-rs/waterui#2002).
    pub fills: Vec<(kurbo::RoundedRect, Paint)>,
}

impl ChromePlan {
    /// [`invisible_chrome`] with a custom part replaying the plan's
    /// materials, then its fills — installed as a plugin, every tappable
    /// view under it records them.
    #[must_use]
    pub fn chrome(&self) -> Chrome {
        let draws = self.draws.clone();
        let fills = self.fills.clone();
        invisible_chrome().custom(Custom::new((), move |recorder, _frame, ()| {
            for chrome in &draws {
                recorder.backdrop_material(
                    chrome.shape.clone(),
                    chrome.shader,
                    chrome.capture,
                    chrome.effect.clone(),
                );
            }
            for (shape, paint) in &fills {
                recorder.fill(*shape, paint.clone());
            }
        }))
    }
}

impl crate::Style for MinimalTestTheme {
    /// The minimal theme installs no tokens of its own — the runtime's
    /// framework defaults (`install_theme_tokens`) are all a test needs.
    fn install_tokens(&self, _env: &mut Environment) {}
}

impl MinimalTestTheme {
    /// The measures of the theme's menus.
    pub(crate) const fn menu_layout() -> MenuLayout {
        MenuLayout {
            row_height: 56.0,
            horizontal_padding: 16.0,
            vertical_padding: 12.0,
            min_width: 112.0,
            max_width: 320.0,
            width_per_char: 8.5,
            separator_horizontal_inset: 16.0,
            separator_thickness: 1.0,
        }
    }

    /// The theme's menu look: a 4-point panel under a level-2-like shadow
    /// that reaches past 14 points, deep enough for tests to distinguish it
    /// from the scrim's uniform dim and from a fixed popup margin. The
    /// `WorkingColor` components are linear Display P3; the intended panel
    /// colour is sRGB (0.96, 0.94, 0.97), so it converts rather than passing
    /// raw.
    #[cfg(feature = "menu")]
    pub(crate) fn menu_look() -> MenuLook {
        MenuLook::new(
            Self::menu_layout(),
            Surface::new(
                CornerShape::radius(4.0),
                crate::renderer::working_color(peniko::Color::new([0.96, 0.94, 0.97, 1.0])),
            )
            .elevation(Elevation::new(
                Shadow::new(6.0, WorkingColor::new([0.0, 0.0, 0.0, 0.35]))
                    .offset(kurbo::Vec2::new(0.0, 3.0)),
                Elevation::NONE.ambient,
            )),
            Custom::new((), |_, _, ()| {}),
        )
    }

    /// The transparent margin a popup menu window leaves around this
    /// theme's menu panel: its elevation's reach.
    #[cfg(feature = "menu")]
    pub(crate) fn panel_margin() -> f64 {
        Self::menu_look().panel_extent(Rect::ZERO)
    }
}

/// This theme's tab indicator is a strip: 40 thick and the item's whole
/// extent, centered, for horizontal items; otherwise 3 thick along the bar
/// item's top (the sidebar item's trailing edge), as wide (tall) as the
/// title.
fn tab_highlight_strip(item: TabItemGeometry) -> Rect {
    let thickness: f64 = match item.layout {
        TabItemLayout::Vertical => 3.0,
        TabItemLayout::Horizontal => 40.0,
    };
    let bounds = item.bounds;
    if matches!(item.layout, TabItemLayout::Horizontal) {
        let height = thickness.min(bounds.height());
        let y0 = (bounds.height() - height).mul_add(0.5, bounds.y0);
        Rect::new(bounds.x0, y0, bounds.x1, y0 + height)
    } else if matches!(item.placement, TabStyle::Sidebar) {
        Rect::new(
            (bounds.x1 - thickness).max(bounds.x0),
            item.title.y0,
            bounds.x1,
            item.title.y1,
        )
    } else {
        Rect::new(
            item.title.x0,
            bounds.y0,
            item.title.x1,
            (bounds.y0 + thickness).min(bounds.y1),
        )
    }
}

impl WidgetTheme for MinimalTestTheme {
    fn button(&self, spec: &ButtonSpec, _env: &Environment) -> ButtonLook {
        let chrome = self.button_chrome.clone().unwrap_or_else(invisible_chrome);
        let mut look = ButtonLook::new(chrome, self.button_label.clone().unwrap_or_default())
            .group_blend_space(self.button_group_blend_space);
        if let Some(opacity) = &self.button_group_opacity {
            look = look.group_opacity(opacity.clone());
        }
        if spec.icon_only() {
            look.minimum_size(Size::new(41.0, 43.0))
        } else {
            look.padding(1.0, 2.0).minimum_size(Size::new(123.0, 45.0))
        }
    }

    /// Fills `registry` with the test's shader sources and capture
    /// classes — the renderer calls this once at `with_engine`.
    fn register_backdrop_shaders(&self, registry: &mut MaterialRegistry) {
        for (key, source) in &self.chrome.shaders {
            registry.register_shader(*key, source.clone());
        }
        for (key, capture) in &self.chrome.captures {
            registry.register_capture_class(*key, *capture);
        }
        for (shader, recipe) in &self.chrome.containers {
            registry.register_container_recipe(*shader, SharedRecipe(Rc::clone(recipe)));
        }
        for (key, source) in &self.content_shaders {
            registry.register_content_shader(*key, source.clone());
        }
    }

    #[cfg(feature = "toggle")]
    fn switch(&self, spec: &ToggleSpec, env: &Environment) -> SwitchLook {
        self.switch_look.as_ref().map_or_else(
            || {
                SwitchLook::new(
                    crate::testing::plain_toggle_control(),
                    SwitchDrag::new(6.0, spec.direction()),
                )
            },
            |look| look(spec, env),
        )
    }

    #[cfg(feature = "toggle")]
    fn checkbox(&self, _spec: &ToggleSpec, _env: &Environment) -> CheckboxLook {
        CheckboxLook::new(crate::testing::plain_toggle_control())
    }

    #[cfg(feature = "stepper")]
    fn stepper(&self, spec: &StepperSpec, env: &Environment) -> StepperLook {
        self.stepper_specs.borrow_mut().push(spec.clone());
        self.stepper_look
            .as_ref()
            .map_or_else(crate::testing::plain_stepper_look, |look| look(spec, env))
    }

    #[cfg(feature = "picker")]
    fn segmented_picker(
        &self,
        spec: &crate::theme::SegmentedSpec,
        env: &Environment,
    ) -> crate::theme::SegmentedLook {
        self.segmented_look
            .as_ref()
            .map_or_else(crate::testing::plain_segmented_look, |look| look(spec, env))
    }

    fn glass_background(
        &self,
        glass: &waterui::background::Glass,
        env: &Environment,
    ) -> Option<crate::theme::GlassLook> {
        self.glass_look.as_ref().and_then(|look| look(glass, env))
    }

    #[cfg(any(
        feature = "text-field",
        feature = "secure-field",
        feature = "date-picker",
        feature = "color-picker"
    ))]
    fn text_field(&self, spec: &TextFieldSpec, env: &Environment) -> TextFieldLook {
        self.text_field_specs.borrow_mut().push(spec.clone());
        self.text_field_look.as_ref().map_or_else(
            || {
                crate::testing::plain_text_field_look(
                    invisible_chrome(),
                    self.input_metrics
                        .unwrap_or_else(crate::testing::plain_input_metrics),
                )
            },
            |look| look(spec, env),
        )
    }

    #[cfg(feature = "menu")]
    fn menu(&self, _spec: &MenuSpec, _env: &Environment) -> MenuLook {
        self.menu_look.clone().unwrap_or_else(Self::menu_look)
    }

    #[cfg(any(feature = "picker", feature = "date-picker"))]
    fn menu_picker(&self, _spec: &PickerSpec, _env: &Environment) -> MenuPickerLook {
        crate::testing::plain_menu_picker_look()
    }

    #[cfg(feature = "picker")]
    fn radio_group(&self, _spec: &PickerSpec, _env: &Environment) -> RadioGroupLook {
        let realizations = Rc::clone(&self.radio_indicator_realizations);
        let plain = crate::testing::plain_radio_group_look();
        RadioGroupLook::new(
            plain.indicator().clone().part(Custom::new(
                (Driven::<bool, bool>::new(move |inputs| {
                    realizations
                        .borrow_mut()
                        .push((inputs.frame, *inputs.value));
                    *inputs.value
                }),),
                |_recorder, _frame, (_selected,)| {},
            )),
            plain.layout(),
        )
    }

    #[cfg(feature = "slider")]
    fn slider(&self, spec: &SliderSpec, env: &Environment) -> SliderLook {
        self.slider_specs.borrow_mut().push(spec.clone());
        if let Some(look) = &self.slider_look {
            return look(spec, env);
        }
        let tracks = Rc::clone(&self.slider_track_draws);
        let indicators = Rc::clone(&self.slider_indicator_draws);
        let layout = SliderLayout {
            // 44pt for the ExtraSmall default, four points deeper per size:
            // the per-size tests read this back off the measured row.
            row_height: 4.0f64.mul_add(f64::from(spec.size() as u8), 44.0),
            track_inset: 12.0,
            label_inset: 12.0,
            label_gap: 8.0,
            minimum_track: 72.0,
            label_spacing: (!self.slider_without_label).then_some(6.0),
        };
        let knob = Knob {
            size: Size::new(4.0, 44.0),
            hit: HitRegion::NONE,
        };
        SliderLook::new(
            layout,
            knob,
            self.slider_drag.unwrap_or(DragMapping::Absolute),
            Motion::uniform(Animation::linear(Duration::from_millis(15))),
        )
        .part(Custom::<f64, _>::new((), move |_draw, frame, ()| {
            tracks.borrow_mut().push(frame);
        }))
        .value_indicator(
            ValueIndicator::new(
                waterui_text::font::Font::default(),
                Color::srgb(255, 255, 255),
                Custom::<f64, _>::new((), move |_draw, frame, ()| {
                    indicators.borrow_mut().push(frame);
                }),
            )
            .padding(8.0, 4.0)
            .gap(4.0),
        )
    }

    fn linear_progress(&self, spec: &ProgressSpec, env: &Environment) -> LinearProgressLook {
        self.progress_specs.borrow_mut().push(spec.clone());
        let control = if self.indeterminate_progress.moves() && !spec.determinate() {
            progress_looks::moving_bar()
        } else if self.indeterminate_progress == IndeterminateProgress::Scheme
            && !spec.determinate()
        {
            progress_looks::scheme_bar(env)
        } else {
            let realizations = Rc::clone(&self.progress_bar_realizations);
            let recordings = Rc::clone(&self.progress_bar_recordings);
            ProgressControl::new(
                Size::new(72.0, 8.0),
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            )
            .part(Custom::new(
                (Driven::<f64, f64>::new(move |inputs| {
                    realizations
                        .borrow_mut()
                        .push((inputs.frame, *inputs.value));
                    *inputs.value
                }),),
                move |_recorder, _frame, (_fill,)| recordings.set(recordings.get() + 1),
            ))
        };
        LinearProgressLook::new(control, 8.0)
            .label_band(18.0, 10.0)
            .value_label_band(6.0)
    }

    fn circular_progress(&self, spec: &ProgressSpec, _env: &Environment) -> IndicatorLook {
        self.progress_specs.borrow_mut().push(spec.clone());
        let control = if self.indeterminate_progress.moves() && !spec.determinate() {
            progress_looks::spinner(self.indeterminate_progress == IndeterminateProgress::Nested)
        } else {
            let realizations = Rc::clone(&self.progress_bar_realizations);
            let recordings = Rc::clone(&self.progress_bar_recordings);
            ProgressControl::new(
                Size::new(32.0, 32.0),
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            )
            .part(Custom::new(
                (Driven::<f64, f64>::new(move |inputs| {
                    realizations
                        .borrow_mut()
                        .push((inputs.frame, *inputs.value));
                    *inputs.value
                }),),
                move |_recorder, _frame, (_fill,)| recordings.set(recordings.get() + 1),
            ))
        };
        let look = IndicatorLook::new(control);
        if self.circular_without_labels {
            look.without_labels()
        } else {
            look
        }
    }

    fn loading_indicator(&self, spec: &ProgressSpec, _env: &Environment) -> IndicatorLook {
        self.progress_specs.borrow_mut().push(spec.clone());
        let control = if self.indeterminate_progress.moves() {
            progress_looks::morphing()
        } else {
            let realizations = Rc::clone(&self.progress_bar_realizations);
            let recordings = Rc::clone(&self.progress_bar_recordings);
            ProgressControl::new(
                Size::new(48.0, 48.0),
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            )
            .part(Custom::new(
                (Driven::<f64, f64>::new(move |inputs| {
                    realizations
                        .borrow_mut()
                        .push((inputs.frame, *inputs.value));
                    *inputs.value
                }),),
                move |_recorder, _frame, (_fill,)| recordings.set(recordings.get() + 1),
            ))
        };
        IndicatorLook::new(control)
    }

    #[cfg(feature = "navigation")]
    fn navigation_bar(&self, _spec: &NavigationSpec, _env: &Environment) -> NavigationBarLook {
        let bar_draws = Rc::clone(&self.navigation_bar_draws);
        let separator_draws = Rc::clone(&self.navigation_bar_separator_draws);
        let mut look = NavigationBarLook::new(
            crate::testing::plain_navigation_metrics(),
            crate::testing::plain_navigation_motion(),
            invisible_chrome(),
            Custom::new((), move |_recorder, frame, ()| {
                bar_draws.borrow_mut().push(frame);
            }),
        )
        .separator(Custom::new((), move |_recorder, frame, ()| {
            separator_draws.borrow_mut().push(frame);
        }));
        if let Some(font) = &self.navigation_title_font {
            look = look.title_font(font.clone());
        }
        look
    }

    #[cfg(feature = "navigation")]
    fn tabs(&self, _spec: &TabsSpec, _env: &Environment) -> TabsLook {
        if let Some(look) = &self.tabs_look {
            return look.clone();
        }
        let bar_draws = Rc::clone(&self.tabs_bar_draws);
        let divider_draws = Rc::clone(&self.tabs_bar_divider_draws);
        let highlight_draws = Rc::clone(&self.tabs_highlight_draws);
        let metrics = crate::testing::plain_tabs_metrics();
        let mut look = TabsLook::new(
            metrics,
            Custom::new((), move |_recorder, frame, ()| {
                bar_draws.borrow_mut().push(frame);
            }),
            Custom::new((), move |_recorder, frame, ()| {
                divider_draws.borrow_mut().push(frame);
            }),
            crate::testing::plain_tabs_motion(),
            |_item, _selected| invisible_chrome(),
        )
        .highlight(move |item| {
            let item = *item;
            let highlight_draws = Rc::clone(&highlight_draws);
            Custom::new((), move |_recorder, _frame, ()| {
                // The selected item's indicator alone is observed: the other
                // items' highlights are parts at rest.
                if item.selected {
                    highlight_draws
                        .borrow_mut()
                        .push((tab_highlight_strip(item.geometry), item.geometry.layout));
                }
            })
        });
        // The layout the theme reports: forced, or by width class.
        if let Some(threshold) = self.horizontal_from_width {
            look = look.horizontal(threshold, metrics);
        } else if self.forced_tab_item_layout == Some(TabItemLayout::Horizontal) {
            look = look.horizontal(0.0, metrics);
        }
        look
    }
    fn scroll_indicator(&self, _env: &Environment) -> ScrollIndicatorLook {
        crate::testing::plain_scroll_indicator_look()
    }

    /// The plain badge look, its dot and its pill listing the frame each
    /// drew on.
    fn badge(&self, _env: &Environment) -> BadgeLook {
        let plain = crate::testing::plain_badge_look();
        let drawing = || {
            let draws = Rc::clone(&self.badge_draws);
            Custom::new((), move |_recorder, frame, ()| {
                draws.borrow_mut().push(frame);
            })
        };
        BadgeLook::new(
            plain.metrics(),
            plain.label_font().clone(),
            plain.label_color().clone(),
            drawing(),
            drawing(),
        )
    }

    #[cfg(feature = "list")]
    fn list(&self, _spec: &ListSpec, _env: &Environment) -> ListLook {
        self.list_look.clone().unwrap_or_else(plain_list_look)
    }

    #[cfg(feature = "table")]
    fn table(&self, _env: &Environment) -> TableLook {
        crate::testing::plain_table_look()
    }
}

#[test]
fn ime_preedit_commit_and_disable_update_focused_text_target() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(test_caret_motion());
    let selection = Rc::new(RefCell::new(TextSelectionSlot {
        anchor: 0,
        focus: 0,
        initialized: true,
    }));
    seed_text_input_targets(
        &mut renderer,
        vec![text_input_target(
            text_field_model("", None),
            Rc::clone(&selection),
        )],
    );

    assert!(renderer.set_focused_text_input(Some(0)));
    assert!(
        renderer.root_is_dirty(),
        "text input focus changes must refresh the retained tree so focus animations start on click"
    );
    assert!(
        !renderer
            .root_marks()
            .contains(crate::renderer::Dirty::STRUCTURE),
        "text input focus changes must not mark structure — the view body is never re-dispatched"
    );
    assert!(renderer.handle_ime_preedit("拼音", Some(0)));
    assert_eq!(renderer.text_editing.ime_preedit.as_deref(), Some("拼音"));
    assert!(renderer.handle_ime_commit("中"));
    assert_eq!(renderer.text_editing.ime_preedit, None);
    assert_eq!(
        renderer.text_editing.text_input_targets[0]
            .model
            .plain_text(),
        "中"
    );
    assert_eq!(
        (selection.borrow().anchor, selection.borrow().focus),
        ("中".len(), "中".len())
    );

    assert!(renderer.handle_ime_preedit("候选", Some(0)));
    assert!(renderer.handle_ime_disabled());
    assert_eq!(renderer.text_editing.ime_preedit, None);
    assert_eq!(
        renderer.text_editing.text_input_targets[0]
            .model
            .plain_text(),
        "中"
    );
}

/// Targets are re-emitted in flush order every frame, so a field's position is
/// not its identity. Inserting a row above the focused field must not hand focus
/// — and with it the caret, selection and IME — to whichever field slid into the
/// old position.
#[test]
fn text_input_focus_stays_on_its_field_when_a_row_is_inserted_above_it() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(test_caret_motion());
    let first = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let focused = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let emit = |renderer: &mut HydrolysisRenderer<crate::text::ParleyEngine>,
                targets: &[(&str, &Rc<RefCell<TextSelectionSlot>>)]| {
        seed_text_input_targets(
            renderer,
            targets
                .iter()
                .map(|(value, selection)| {
                    text_input_target(text_field_model(value, None), Rc::clone(selection))
                })
                .collect(),
        );
    };

    emit(&mut renderer, &[("first", &first), ("focused", &focused)]);
    assert!(renderer.set_focused_text_input(Some(1)));
    let focused_key = renderer
        .text_editing
        .focused_key()
        .expect("the second field is focused");

    // The next frame emits a newly inserted row ahead of both fields, so the
    // focused field moves from position 1 to position 2.
    let inserted = Rc::new(RefCell::new(TextSelectionSlot::default()));
    emit(
        &mut renderer,
        &[
            ("inserted", &inserted),
            ("first", &first),
            ("focused", &focused),
        ],
    );
    renderer.validate_focused_text_input_after_flush();

    assert_eq!(
        renderer.text_editing.focused_key().as_ref(),
        Some(&focused_key),
        "focus identity must survive a reflow that reorders targets"
    );
    assert_eq!(
        renderer.text_editing.focused_index(),
        Some(2),
        "focus must resolve to the focused field's new position"
    );
    let (_, model, _) = renderer
        .focused_text_target_data()
        .expect("the focused field is still emitted");
    assert_eq!(
        model.plain_text(),
        "focused",
        "typing must reach the field the user focused, not the one now at its old position"
    );
}

/// The counterpart: when the focused field is genuinely gone from the tree,
/// focus is dropped rather than resolving to some other field.
#[test]
fn text_input_focus_is_dropped_when_its_field_stops_being_emitted() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(test_caret_motion());
    let survivor = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let removed = Rc::new(RefCell::new(TextSelectionSlot::default()));
    seed_text_input_targets(
        &mut renderer,
        [("survivor", &survivor), ("removed", &removed)]
            .iter()
            .map(|(value, selection)| {
                text_input_target(text_field_model(value, None), Rc::clone(selection))
            })
            .collect(),
    );
    assert!(renderer.set_focused_text_input(Some(1)));

    seed_text_input_targets(
        &mut renderer,
        vec![text_input_target(
            text_field_model("survivor", None),
            Rc::clone(&survivor),
        )],
    );
    renderer.validate_focused_text_input_after_flush();

    assert!(
        !renderer.text_editing.has_focus(),
        "focus must be cleared when its field leaves the tree"
    );
    assert!(renderer.focused_text_target_data().is_none());
}

#[test]
fn text_selection_pointer_update_uses_transient_redraw_path() {
    let mut renderer = test_renderer();
    let selection = Rc::new(RefCell::new(TextSelectionSlot::default()));
    seed_text_input_targets(
        &mut renderer,
        vec![text_input_target(
            text_field_model("selection", None),
            Rc::clone(&selection),
        )],
    );

    assert!(renderer.update_text_selection_from_pointer(0, Point::ZERO, false));
    assert_eq!(
        renderer.root_marks(),
        crate::renderer::Dirty::NONE,
        "text selection changes are rendered by the transient overlay instead of a structural mark"
    );
    assert!(!renderer.update_text_selection_from_pointer(0, Point::ZERO, false));
    assert_eq!(
        renderer.root_marks(),
        crate::renderer::Dirty::NONE,
        "unchanged text selection must not mark structure"
    );
}

/// Seeds `targets` as the frame's emitted text inputs through the retained
/// registry — the path a real emit takes, owner bucket and all — then
/// materializes, so staged and retained readers both see them exactly like
/// a flush's. The owner is the root cell, the owner a window-level
/// registration carries.
fn seed_text_input_targets<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    targets: Vec<TextInputTarget<E>>,
) {
    let anchor = renderer.window_placement();
    let owner = Rc::clone(renderer.root_cell());
    renderer.purge_registrations(&owner);
    renderer.retained.enlist(&owner);
    for target in targets {
        let bounds = target.bounds;
        let entry = crate::renderer::mount::RetainedEntry::at(
            target,
            bounds,
            &anchor,
            &owner,
            renderer.retained.next_seq(),
        );
        let mut slot = owner.registrations.borrow_mut();
        slot.get_or_insert_with(|| Box::new(crate::renderer::mount::OwnerRegistrations::default()))
            .text_input_targets
            .push(entry);
    }
    renderer.retained.stale.set(true);
    renderer.registries();
}

/// A text-input target with real bounds and a real shaped layout, so click
/// gestures resolve to actual caret/word/line ranges instead of an empty
/// layout's index 0.
fn shaped_text_input_target(
    value: &str,
    selection: &Rc<RefCell<TextSelectionSlot>>,
    env: &Environment,
) -> TextInputTarget<crate::text::ParleyEngine> {
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let layout = HydrolysisRenderer::build_text_layout(
        &mut state,
        StyledStr::plain(value.to_owned()),
        HorizontalAlignment::Leading,
        env,
        Some(200.0),
    );
    let mut target = text_input_target(text_field_model(value, None), Rc::clone(selection));
    target.bounds = Rect::new(0.0, 0.0, 200.0, 60.0);
    target.frame = target.bounds;
    target.text_bounds = Rect::new(0.0, 0.0, 200.0, 60.0);
    target.text_clip_bounds = target.text_bounds;
    target.cursor_area = target.text_bounds;
    target.layout = layout;
    target.env = env.clone();
    target
}

/// The window point at the center of the caret geometry for `byte_index`, used
/// to aim synthetic clicks inside a specific word.
fn caret_point_in_target<E: crate::text::TextEngine>(
    target: &TextInputTarget<E>,
    byte_index: usize,
) -> Point {
    let geometry = target.layout.caret_rect(TextPosition {
        index: byte_index,
        affinity: Affinity::Downstream,
    });
    Point::new(
        target.text_bounds.x0 + f64::midpoint(geometry.x0, geometry.x1),
        target.text_bounds.y0 + f64::midpoint(geometry.y0, geometry.y1),
    )
}

/// The release event re-runs the drag-extension path before the drag is
/// cleared. A double-click drag must keep its word granularity there — and for
/// any jitter between down and up — instead of collapsing the word the gesture
/// selected back to the caret under the pointer.
#[test]
fn double_click_word_selection_survives_pointer_release() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(test_caret_motion());
    let env = test_environment();
    let selection = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let target = shaped_text_input_target("hello world", &selection, &env);
    let point = caret_point_in_target(&target, 8);
    seed_text_input_targets(&mut renderer, vec![target]);

    renderer.handle_pointer_down(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
        PointerButton::Primary,
        &env,
    );
    renderer.handle_pointer_up(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
        PointerButton::Primary,
        &env,
    );
    renderer.handle_pointer_down(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
        PointerButton::Primary,
        &env,
    );
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        6..11,
        "double-click must select the whole word under the pointer"
    );

    renderer.handle_pointer_up(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
        PointerButton::Primary,
        &env,
    );
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        6..11,
        "releasing a double-click must not collapse the word selection to a caret"
    );
}

/// Holding the button after a double-click extends the selection word by word,
/// anchored on the word the gesture snapped to — matching platform text-field
/// behavior.
#[test]
fn double_click_drag_extends_selection_by_words() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(test_caret_motion());
    let env = test_environment();
    let selection = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let target = shaped_text_input_target("hello world", &selection, &env);
    let world_point = caret_point_in_target(&target, 8);
    let hello_point = caret_point_in_target(&target, 2);
    seed_text_input_targets(&mut renderer, vec![target]);

    renderer.handle_pointer_down(
        crate::num_cast::f64_as_f32(world_point.x),
        crate::num_cast::f64_as_f32(world_point.y),
        PointerButton::Primary,
        &env,
    );
    renderer.handle_pointer_up(
        crate::num_cast::f64_as_f32(world_point.x),
        crate::num_cast::f64_as_f32(world_point.y),
        PointerButton::Primary,
        &env,
    );
    // Second click stays held: this is a double-click-drag, not a third click.
    renderer.handle_pointer_down(
        crate::num_cast::f64_as_f32(world_point.x),
        crate::num_cast::f64_as_f32(world_point.y),
        PointerButton::Primary,
        &env,
    );
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        6..11
    );

    // Still holding the second click's button, drag back across "hello": the
    // selection grows to cover both whole words, not a caret at the pointer.
    renderer.handle_pointer_move(
        crate::num_cast::f64_as_f32(hello_point.x),
        crate::num_cast::f64_as_f32(hello_point.y),
        &env,
    );
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        0..11,
        "double-click drag must extend the selection word by word"
    );
    renderer.handle_pointer_up(
        crate::num_cast::f64_as_f32(hello_point.x),
        crate::num_cast::f64_as_f32(hello_point.y),
        PointerButton::Primary,
        &env,
    );
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        0..11
    );
}

#[test]
fn secure_text_context_menu_excludes_copy_and_cut() {
    let selection = Rc::new(RefCell::new(TextSelectionSlot {
        anchor: 0,
        focus: 3,
        initialized: true,
    }));
    let target = text_input_target(secure_field_model("abc"), selection);

    let mut env = test_environment();
    crate::localization::install(&mut env);
    let nodes = SemanticCore::build_text_context_menu_nodes(&target, &env, true);
    let labels = nodes
        .iter()
        .filter_map(|node| match node {
            PopupMenuNode::Command { plain_label, .. } => Some(plain_label.as_str()),
            PopupMenuNode::Menu { .. } | PopupMenuNode::Divider => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(labels, vec!["Paste", "Select All"]);
}

#[test]
#[cfg(hydrolysis_gpu_painter)]
fn bare_text_at_window_root_renders_into_page() {
    let mut renderer = test_renderer();
    let env = test_environment();

    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(waterui_text::text("probe")),
        &env,
        Rect::new(0.0, 0.0, 160.0, 160.0),
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    assert!(
        renderer.commit_mirror().created > 0,
        "a bare text view at the window root must draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
#[cfg(hydrolysis_gpu_painter)]
fn bare_str_at_window_root_renders_into_page() {
    let mut renderer = test_renderer();
    let env = test_environment();

    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(Str::from("probe")),
        &env,
        Rect::new(0.0, 0.0, 160.0, 160.0),
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    assert!(
        renderer.commit_mirror().created > 0,
        "a bare string view at the window root must draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn text_shaping_produces_nonzero_intrinsic_in_tests() {
    let env = test_environment();
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let size = HydrolysisRenderer::measure_text_intrinsic_size(
        &mut state,
        waterui_text::styled::StyledStr::plain("probe"),
        &env,
    );
    assert!(
        size.width > 0.0 && size.height > 0.0,
        "text shaping must produce a non-zero intrinsic size, got {size:?}"
    );
}

#[test]
#[cfg(hydrolysis_gpu_painter)]
fn bare_str_renders_into_page() {
    let mut renderer = test_renderer();
    let env = test_environment();

    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(Str::from("probe")),
        &env,
        bounds,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    assert!(
        renderer.commit_mirror().created > 0,
        "a bare string must build a text node and draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn render_path_text_layout_has_lines() {
    let env = test_environment();
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let layout = HydrolysisRenderer::build_text_layout(
        &mut state,
        waterui_text::styled::StyledStr::plain("probe"),
        HorizontalAlignment::Leading,
        &env,
        Some(160.0),
    );
    assert!(
        layout.line_count() > 0,
        "render-path text layout must not be empty"
    );
}

/// The three-point probe contract every `SubView` owes the layout algorithm.
///
/// Stacks derive how far a child may shrink, and how far it wants to grow, by
/// measuring it at zero, at nothing, and at infinity. Those answers are only
/// useful if `min <= ideal <= max` holds for every view, so this pins the
/// contract across a representative spread of leaves and containers rather than
/// leaving each to be discovered when a layout looks wrong.
///
/// This covers the Rust-side realization. The native backends answer the same
/// three proposals through their own `sizeThatFits`/`onMeasure`, and owe the
/// same invariant; verifying that needs a test in each of those languages.
#[test]
fn every_view_answers_the_three_point_probe_consistently() {
    let env = test_environment();

    let cases: Vec<(&str, AnyView)> = vec![
        ("text", AnyView::new(text("probe"))),
        ("empty", AnyView::new(())),
        ("spacer", AnyView::new(waterui_layout::spacer())),
        ("divider", AnyView::new(Divider)),
        ("fixed frame", AnyView::new(().size(40.0, 20.0))),
        (
            "min-width frame",
            AnyView::new(().size(40.0, 20.0).min_width(60.0)),
        ),
        (
            "max-width frame",
            AnyView::new(().size(40.0, 20.0).max_width(100.0)),
        ),
        (
            "greedy frame",
            AnyView::new(().size(40.0, 20.0).max_width(f32::INFINITY)),
        ),
        ("button", AnyView::new(button("Tap").action(|| {}))),
        (
            "hstack",
            AnyView::new(hstack((text("a"), text("bb"), text("ccc")))),
        ),
        (
            "vstack",
            AnyView::new(vstack((text("a"), text("bb"), text("ccc")))),
        ),
        ("zstack", AnyView::new(zstack((text("a"), text("bb"))))),
    ];

    for (name, view) in cases {
        let mut subview = Realized::new(view, &env);

        let ideal = subview.measure(ProposalSize::UNSPECIFIED).size;

        // Probe one axis at a time, leaving the other unspecified. This is the
        // shape the window uses to derive its resize limits, and it is stricter
        // than probing both at once: constraining only one axis must not make the
        // other axis's answer travel backwards.
        let min_width = subview
            .measure(ProposalSize::new(Some(0.0), None))
            .size
            .width;
        let min_height = subview
            .measure(ProposalSize::new(None, Some(0.0)))
            .size
            .height;
        let max_width = subview
            .measure(ProposalSize::new(Some(f32::INFINITY), None))
            .size
            .width;
        let max_height = subview
            .measure(ProposalSize::new(None, Some(f32::INFINITY)))
            .size
            .height;

        for (axis, min, ideal, max) in [
            ("width", min_width, ideal.width, max_width),
            ("height", min_height, ideal.height, max_height),
        ] {
            assert!(
                min.is_finite() && min >= 0.0,
                "{name}: the {axis} minimum must be a finite, non-negative extent, got {min}"
            );
            assert!(
                ideal.is_finite() && ideal >= 0.0,
                "{name}: the {axis} ideal must be a finite, non-negative extent, got {ideal}"
            );
            assert!(
                !max.is_nan() && max >= 0.0,
                "{name}: the {axis} maximum must be non-negative (infinity means unbounded), \
                 got {max}"
            );
            assert!(
                min <= ideal + 0.001,
                "{name}: the {axis} minimum ({min}) must not exceed the ideal ({ideal}); a \
                 stack would compress it below a size it cannot take"
            );
            assert!(
                ideal <= max + 0.001,
                "{name}: the {axis} ideal ({ideal}) must not exceed the maximum ({max}); a \
                 stack would grow it past a size it cannot take"
            );
        }
    }
}

/// `BadgeMetrics` offsets are anchored to the content's trailing edge — the
/// badge's leading edge sits `offset_x` inside it, mirrored to the leading
/// edge in RTL — and its bottom edge overlaps the top edge by `offset_y`,
/// matching `BadgedBox` in Compose. They are not center offsets.
#[test]
fn badge_indicator_anchors_to_the_content_trailing_edge() {
    use waterui::component::badge::{Badge, BadgeValue};
    use waterui_core::layout::LayoutDirection;

    /// `Badge` requires `Clone` content and `Frame` is not `Clone`, so the
    /// anchor is a sized view produced from a `Clone` shell.
    #[derive(Clone)]
    struct FillAnchor;

    impl View for FillAnchor {
        fn body(self, _env: &Environment) -> impl View {
            ().size(160.0, 160.0)
        }
    }

    let mut env = test_environment();
    let log = env
        .get::<BadgeDrawLog>()
        .expect("badge draw log is installed")
        .clone();
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);
    let anchor = FillAnchor;

    // The renderer retains badge state by node, so each variant captures on a
    // fresh renderer.
    let capture = |view: Badge<FillAnchor>, env: &Environment| {
        log.0.borrow_mut().clear();
        let mut renderer = test_renderer_with_theme(MinimalTestTheme {
            badge_draws: Rc::clone(&log.0),
            ..Default::default()
        });
        capture_root_window(&mut renderer, view, env, bounds);
        log.0.borrow().clone()
    };

    // Zero count (0_u32 / Hidden / Count(0)) draws nothing on either Badge or Tab::badge.
    assert_eq!(capture(Badge::new(0_u32, anchor.clone()), &env), []);
    assert_eq!(
        capture(Badge::new(BadgeValue::Hidden, anchor.clone()), &env),
        []
    );
    assert_eq!(
        capture(Badge::new(BadgeValue::Count(0), anchor.clone()), &env),
        []
    );

    // Dot: leading edge 6 inside the trailing edge, top edge flush —
    // the dot fills the content's top-trailing 6×6 corner.
    assert_eq!(
        capture(Badge::new(BadgeValue::Dot, anchor.clone()), &env).as_slice(),
        &[Rect::new(154.0, 0.0, 160.0, 6.0)]
    );

    // Count and text badges: leading edge 12 inside the trailing edge regardless
    // of their width, bottom edge 14 below the content's top (their 16-high pill
    // tops out 2 above the anchor).
    let draws = capture(Badge::new(5_u32, anchor.clone()), &env);
    assert_eq!(draws.len(), 1, "one badge indicator draw, got {draws:?}");
    approx::assert_relative_eq!(draws[0].x0, 148.0);
    approx::assert_relative_eq!(draws[0].y0, -2.0);

    let text_draws = capture(Badge::new(BadgeValue::text("New"), anchor.clone()), &env);
    assert_eq!(
        text_draws.len(),
        1,
        "one text badge draw, got {text_draws:?}"
    );
    approx::assert_relative_eq!(text_draws[0].x0, 148.0);
    approx::assert_relative_eq!(text_draws[0].y0, -2.0);

    // RTL mirrors the anchor to the leading edge.
    env.insert(LayoutDirection::RightToLeft);
    assert_eq!(
        capture(Badge::new(BadgeValue::Dot, anchor.clone()), &env).as_slice(),
        &[Rect::new(0.0, 0.0, 6.0, 6.0)]
    );

    let draws = capture(Badge::new(5_u32, anchor), &env);
    assert_eq!(draws.len(), 1, "one badge indicator draw, got {draws:?}");
    approx::assert_relative_eq!(draws[0].x1, 12.0);
    approx::assert_relative_eq!(draws[0].y0, -2.0);
}

/// A tab's `badge` draws dot, count, and text badges on the item; no badge,
/// `BadgeValue::Hidden`, `Count(0)`, and `0_u32` draw none.
#[test]
fn tab_badge_draws_only_for_a_nonzero_count() {
    use waterui::component::badge::BadgeValue;
    let env = test_environment();
    let log = env
        .get::<BadgeDrawLog>()
        .expect("badge draw log is installed")
        .clone();
    let bounds = Rect::new(0.0, 0.0, 480.0, 480.0);
    // The renderer retains tab state by node, so each variant captures on a
    // fresh renderer.
    let capture = |badge: Option<BadgeValue>| {
        log.0.borrow_mut().clear();
        let inbox = Tab::new("Inbox", || NavigationView::new("Inbox", text("One")));
        let inbox = match badge {
            Some(value) => inbox.badge(value),
            None => inbox,
        };
        let tabs = Tabs::new(&Binding::new(0usize), vec![inbox]);
        let mut renderer = test_renderer_with_theme(MinimalTestTheme {
            badge_draws: Rc::clone(&log.0),
            ..Default::default()
        });
        capture_root_window(&mut renderer, tabs, &env, bounds);
        log.0.borrow().len()
    };

    assert_eq!(capture(None), 0, "an unbadged tab draws no badge");
    assert_eq!(
        capture(Some(BadgeValue::from(0_u32))),
        0,
        "a zero count draws no badge"
    );
    assert_eq!(
        capture(Some(BadgeValue::Hidden)),
        0,
        "an explicitly hidden badge draws nothing"
    );
    assert_eq!(
        capture(Some(BadgeValue::Count(0))),
        0,
        "Count(0) draws nothing"
    );
    assert_eq!(capture(Some(BadgeValue::Dot)), 1, "Dot draws one dot badge");
    assert_eq!(
        capture(Some(BadgeValue::Count(3))),
        1,
        "a counted tab draws one count badge"
    );
    assert_eq!(
        capture(Some(BadgeValue::text("New"))),
        1,
        "a text tab badge draws one pill badge"
    );
}

/// water-rs/hydrolysis#51: after the single-child collapse the surviving
/// element reports the labelled container's resolved extent — the size it
/// answered to the placement proposal, centred on the assigned frame — not
/// the child's assigned frame. A root `button.padding(8)` is assigned the
/// whole window while the padding answers only the button's fit plus its
/// insets, so announcing the child's (8, 8, 144, 144) frame announces the
/// button without its padding.
#[cfg(feature = "accessibility")]
#[test]
fn a_collapsed_naming_scope_reports_the_containers_resolved_extent() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let measured = Realized::with_theme(
        AnyView::new(button("OK").action(|| {}).padding_with(8.0)),
        &env,
        &theme,
    )
    .measure(ProposalSize::new(Some(160.0), Some(160.0)))
    .size;
    let mut renderer = test_renderer();
    let view = button("OK")
        .action(|| {})
        .padding_with(8.0)
        .a11y_label("Named");
    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a labelled padding container must publish an accessibility tree");
    let (_, node) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Named"))
        .expect("the labelled element must exist");
    let bounds = node.bounds().expect("the element must carry bounds");
    assert!(
        (bounds.width() - f64::from(measured.width)).abs() < 0.5
            && (bounds.height() - f64::from(measured.height)).abs() < 0.5,
        "the element must report the padding's resolved {}x{}, got {}x{}",
        measured.width,
        measured.height,
        bounds.width(),
        bounds.height(),
    );
    // Symmetric padding's envelope and the assigned frame share the window's
    // centre — either anchor gives (80, 80) here.
    let center_x = f64::midpoint(bounds.x0, bounds.x1);
    let center_y = f64::midpoint(bounds.y0, bounds.y1);
    assert!(
        (center_x - 80.0).abs() < 0.5 && (center_y - 80.0).abs() < 0.5,
        "the resolved extent must centre on the assigned frame, got centre ({center_x}, {center_y})",
    );
}

/// water-rs/hydrolysis#51: the resolved extent is centred on the assigned
/// frame — asymmetric insets shift the placed envelope off the window's
/// centre, but the view's own answer to its proposal is positioned within the
/// assigned bounds, so the reported bounds must not follow the content.
#[cfg(feature = "accessibility")]
#[test]
fn a_collapsed_naming_scope_centres_the_resolved_extent_on_the_assigned_frame() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let measured = Realized::with_theme(
        AnyView::new(
            button("OK")
                .action(|| {})
                .padding_with([0.0, 20.0, 0.0, 0.0]),
        ),
        &env,
        &theme,
    )
    .measure(ProposalSize::new(Some(160.0), Some(160.0)))
    .size;
    let mut renderer = test_renderer();
    let view = button("OK")
        .action(|| {})
        .padding_with([0.0, 20.0, 0.0, 0.0])
        .a11y_label("Named");
    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a labelled padding container must publish an accessibility tree");
    let (_, node) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Named"))
        .expect("the labelled element must exist");
    let bounds = node.bounds().expect("the element must carry bounds");
    assert!(
        (bounds.width() - f64::from(measured.width)).abs() < 0.5
            && (bounds.height() - f64::from(measured.height)).abs() < 0.5,
        "the element must report the padding's resolved {}x{}, got {}x{}",
        measured.width,
        measured.height,
        bounds.width(),
        bounds.height(),
    );
    // Leading-only insets place the content envelope at (20, 0, 140, 160),
    // centred at (90, 80) — the resolved extent must NOT follow it: the
    // padded view's answer centres on the assigned frame (80, 80), and every
    // edge of the reported bounds must stay inside the 160x160 window.
    let center_x = f64::midpoint(bounds.x0, bounds.x1);
    let center_y = f64::midpoint(bounds.y0, bounds.y1);
    assert!(
        (center_x - 80.0).abs() < 0.5 && (center_y - 80.0).abs() < 0.5,
        "the resolved extent must centre on the assigned frame (80, 80), got ({center_x}, {center_y})",
    );
    assert!(
        bounds.x0 >= -0.5 && bounds.y0 >= -0.5 && bounds.x1 <= 160.5 && bounds.y1 <= 160.5,
        "a view entirely inside the window must report bounds inside it, got {bounds:?}",
    );
}

/// A labelled container with several semantic children stands as a `Group`
/// and reports its own response rectangle — the stack's answer centred in the
/// frame it was assigned — never the descendants' union and never the whole
/// assigned frame (`docs/layout-spec.md` §7.2).
#[cfg(feature = "accessibility")]
#[test]
fn a_labelled_container_that_stands_reports_its_response_rectangle() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let view = vstack((text("A"), text("B"))).a11y_label("Named");
    capture_root_window(&mut renderer, view, &env, Rect::new(0.0, 0.0, 160.0, 160.0));

    let update = renderer
        .take_accessibility_tree_update()
        .expect("a labelled stack must publish an accessibility tree");
    let (_, node) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Named"))
        .expect("the labelled element must exist");
    assert_eq!(node.role(), AccessibilityNodeRole::Group);
    let bounds = node.bounds().expect("the element must carry bounds");
    let (center_x, center_y) = (
        f64::midpoint(bounds.x0, bounds.x1),
        f64::midpoint(bounds.y0, bounds.y1),
    );
    assert!(
        bounds.width() < 80.0
            && bounds.height() < 80.0
            && (center_x - 80.0).abs() < 0.5
            && (center_y - 80.0).abs() < 0.5,
        "a standing container reports its answer centred in the assigned frame, got {bounds:?}",
    );
}

/// A GPU context whose generations' engines report their composition
/// decisions into the returned sink.
#[cfg(hydrolysis_gpu_painter)]
fn diagnostic_gpu() -> (
    crate::platform::OffscreenGpuContext,
    cherenkov_gpu::composition::Sink,
) {
    let diagnostics = cherenkov_gpu::composition::Sink::new();
    let gpu = crate::platform::OffscreenGpuContext::for_tests_with_engine_config(
        cherenkov_gpu::GpuConfig {
            composition_diag: Some(diagnostics.clone()),
            ..cherenkov_gpu::GpuConfig::default()
        },
    );
    (gpu, diagnostics)
}

/// A bar item's icon-only default reaches through an environment snapshot
/// wrapping its content, which replaces the environment wholesale — as the
/// navigation stack's retained environment wraps every bar slot.
#[cfg(feature = "navigation")]
#[test]
fn a_toolbar_item_keeps_icon_only_labels_inside_an_environment_snapshot() {
    use crate::widgets::nav::navigation::scoped_toolbar_item_env;

    let env = test_environment();
    let mut renderer = test_renderer();
    let item = AnyView::new(Metadata::new(text("Share"), Environment::new()));
    let node = RenderNode::build(
        Metadata::new(item, scoped_toolbar_item_env(&env)),
        &env,
        &mut renderer,
    );

    let scope = node
        .leading_scope_env()
        .expect("the item opens with its environment scope");
    assert!(matches!(
        scope.get::<LabelDisplayMode>(),
        Some(LabelDisplayMode::IconOnly)
    ));
}
