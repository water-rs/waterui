mod collection_update;
use super::*;
use std::borrow::Cow;
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use executor_core::LocalExecutor;
use executor_core::async_task::{self, AsyncTask, Runnable};

#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod context_menu_presentation;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod dynamic_remeasure;
#[cfg(not(target_arch = "wasm32"))]
mod emoji_atlas;
mod gesture_capture;
mod gesture_env;
mod gesture_retention;
mod gesture_surface;
mod gpu_surface_direct;
mod gpu_surface_idle;
mod gpu_surface_input;
mod image_ingest;
mod ime;
mod layout_contract;
mod lazy_cross;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod list_remeasure;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod list_row_focus;
mod list_row_hit;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod list_row_metrics;
mod perf_full_rebuild;
mod perf_scroll;
#[cfg(not(target_arch = "wasm32"))]
mod popup_frame;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod popup_windows;
mod retained_scene;
mod scene_offer;
#[cfg(feature = "accessibility")]
mod scroll_frames;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod semantic_runtime;
mod shadow;
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
mod teardown_order;
mod tree;
#[cfg(not(target_arch = "wasm32"))]
mod window_background;
#[cfg(not(target_arch = "wasm32"))]
mod window_mount;
use vello::kurbo::{Affine, BezPath, Point, Rect};
use waterui::gesture::{DragGesture, GestureObserver, MagnificationGesture};
use waterui::prelude::text;
use waterui::style::FloatingStyle;
use waterui::{Binding, Color, Computed, Signal, SignalExt as _, ViewExt as _};
use waterui_canvas::Canvas;
use waterui_controls::button::{ButtonSize, ButtonStyle, button};
use waterui_controls::label::{LabelDisplayMode, label};
use waterui_controls::slider::slider;
#[cfg(feature = "accessibility")]
use waterui_controls::text_field::field;
use waterui_controls::toggle::{ToggleStyle, toggle};
use waterui_form::picker::PickerStyle;
#[cfg(feature = "accessibility")]
use waterui_form::picker::{Picker, color::ColorPicker, date::DatePicker};
use waterui_layout::stack::{VStackLayout, hstack, vstack, zstack};
use waterui_layout::{Divider, scroll};
#[cfg(feature = "accessibility")]
use waterui_navigation::NavigationView;
#[cfg(feature = "accessibility")]
use waterui_navigation::tab::{Tab, TabsLayout};

use crate::engine::{Brush, DrawContext, WidgetTheme};
use crate::platform::PlatformWindow as _;
use waterui_backend_core::widget::{
    BadgeMetrics, ButtonMetrics, DividerMetrics, InputFieldMetrics, InteractionFocusBinding,
    InteractionMotion, ListMetrics, ModalInteraction, NavigationMetrics, NavigationMotion,
    PickerMetrics, ProgressIndicatorStyle, ProgressMetrics, ProgressMotion, RadioIndicatorState,
    RadioSelectionMotion, SliderMetrics, StepperEnd, StepperMetrics, TableMetrics, TabsMetrics,
    TextCaretMotion, TextContextMenuMetrics, ToggleMetrics, WidgetInteractionState,
};
use waterui_core::EasingCurve;
use waterui_core::handler::SharedAction;

fn test_renderer() -> HydrolysisRenderer {
    test_renderer_with_theme(MinimalTestTheme::default())
}

fn test_renderer_with_theme(theme: MinimalTestTheme) -> HydrolysisRenderer {
    let mut platform =
        crate::platform::OffscreenWindow::new_for_tests(160, 160, wgpu::TextureFormat::Rgba8Unorm);
    let surface = platform.surface();
    let mut renderer = HydrolysisRenderer::new(surface.adapter(), surface.device(), Rc::new(theme));
    renderer.set_frame_resources(
        surface.adapter(),
        surface.device(),
        surface.queue(),
        surface.device_loss(),
    );
    renderer
}

/// Emits the semantic node a real widget emits for an interaction identity:
/// it advertises `Focus` (and `Click` when it activates), its action target is
/// registered, and the interaction key is linked so the pointer machinery —
/// and keyboard traversal, which reads the semantic tree — resolves the node.
#[cfg(feature = "accessibility")]
fn emit_focusable_node(
    renderer: &mut HydrolysisRenderer,
    key: &InteractionKey,
    bounds: Rect,
    env: &Environment,
    activation: Option<AccessibilityActivation>,
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
/// future is simply never polled. Runnables are therefore parked in a
/// thread-local queue and dropped when the thread ends. Do not run them inline —
/// these futures re-enter the renderer and its GPU work, which deadlocks when
/// polled in the middle of the render call that spawned them.
#[derive(Clone, Copy, Debug, Default)]
struct TestLocalExecutor;

thread_local! {
    /// Parks runnables so dropping them (which would cancel the task) is deferred
    /// to thread teardown rather than happening inside `schedule`.
    static PARKED_RUNNABLES: RefCell<Vec<Runnable>> = const { RefCell::new(Vec::new()) };
}

impl LocalExecutor for TestLocalExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let (runnable, task) = async_task::spawn_local(fut, |runnable: Runnable| {
            PARKED_RUNNABLES.with(|parked| parked.borrow_mut().push(runnable));
        });
        runnable.schedule();
        task
    }
}

pub(crate) fn test_environment() -> Environment {
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        TestLocalExecutor,
        waterui::task::RefreshRate::HEADLESS,
    ));
    themed_test_environment()
}

/// The same environment, but without pinning this thread's local executor, so
/// that a [`HeadlessRuntime`](crate::HeadlessRuntime) built with it installs its
/// own draining one instead.
///
/// The first install wins, and building the environment happens before the
/// runtime that would install a real executor exists, so a test that needs
/// spawned work to actually *run* — a `GpuView`'s async `setup`, which never
/// completes on the parking [`TestLocalExecutor`] — has to leave the slot open.
pub(crate) fn pumped_test_environment() -> Environment {
    themed_test_environment()
}

fn themed_test_environment() -> Environment {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let mut env = Environment::new();
    crate::testing::install_theme(&mut env);
    crate::localization::install(&mut env);
    env.insert(BadgeDrawLog(Rc::new(RefCell::new(Vec::new()))));
    env
}

/// Every badge indicator rect the test theme was asked to draw, in window
/// coordinates. Tests read it back via `env.get::<BadgeDrawLog>()`.
#[derive(Clone, Default)]
pub(crate) struct BadgeDrawLog(pub Rc<RefCell<Vec<Rect>>>);

#[derive(Clone, Copy)]
struct RecursivelyErasedView;

impl View for RecursivelyErasedView {
    fn body(self, _env: &Environment) -> impl View {
        AnyView::new(self)
    }
}

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
fn animated_scalar_subscribes_before_reading_its_snapshot() {
    let signal = registration_signal(0.25, nami::watcher::Context::from(0.25));
    let mut renderer = test_renderer();

    let resolved = renderer.resolve_animated_scalar_with_discriminator(&signal, usize::MAX);

    assert_eq!(resolved, 0.25);
    assert!(signal.subscribed.get());
}

#[test]
fn toggle_progress_subscribes_before_reading_its_snapshot() {
    let signal = registration_signal(false, nami::watcher::Context::from(false));
    let mut renderer = test_renderer();

    let (progress, selected) =
        renderer.resolve_toggle_progress(&signal, Animation::linear(Duration::ZERO));

    assert_eq!(progress, 0.0);
    assert!(!selected);
    assert!(signal.subscribed.get());
}

#[test]
fn labeled_toggle_keeps_label_activation_out_of_switch_visual_interaction() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let enabled = Binding::bool(false);
    let bounds = Rect::new(0.0, 0.0, 160.0, 40.0);

    capture_root_window(
        &mut renderer,
        toggle("Enable Feature", &enabled),
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
            label_point.x as f32,
            label_point.y as f32,
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
            label_point.x as f32,
            label_point.y as f32,
            PointerButton::Primary,
            &env,
        );
        assert_eq!(enabled.snapshot(), expected);
    }

    let switch_point = Point::new(
        (switch_target.bounds.x0 + switch_target.bounds.x1) * 0.5,
        (switch_target.bounds.y0 + switch_target.bounds.y1) * 0.5,
    );
    let _ = renderer.handle_pointer_down(
        switch_point.x as f32,
        switch_point.y as f32,
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
            .latest()
            .and_then(|wave| wave.origin),
        Some(switch_point),
        "switch ripple must originate inside the switch control"
    );
}

fn empty_selection_menu() -> nami::Computed<Vec<ResolvedMenuItem>> {
    nami::Computed::new(Vec::new())
}

fn text_field_model(value: &str, line_limit: Option<usize>) -> TextInputModel {
    TextInputModel::TextField {
        value: Binding::container(StyledStr::plain(value.to_owned())),
        line_limit,
        selection_menu: empty_selection_menu(),
    }
}

fn secure_field_model(value: &str) -> TextInputModel {
    let mut secure = FormSecure::default();
    secure.set(value.to_owned());
    TextInputModel::SecureField {
        value: Binding::container(secure),
    }
}

fn text_input_target(
    model: TextInputModel,
    selection: Rc<RefCell<TextSelectionSlot>>,
) -> TextInputTarget {
    let interaction_key = InteractionKey::for_rc(&selection, 0);
    TextInputTarget {
        interaction_key,
        modal: false,
        bounds: Rect::ZERO,
        cursor_area: Rect::ZERO,
        text_bounds: Rect::ZERO,
        text_clip_bounds: Rect::ZERO,
        content_alpha: 1.0,
        layout: std::sync::Arc::new(parley::Layout::default()),
        purpose: TextInputPurpose::Normal,
        depth: 0,
        order: 0,
        model,
        selection,
        env: test_environment(),
        focus_binding: None,
        #[cfg(feature = "accessibility")]
        accessibility_node_id: None,
    }
}

#[test]
fn measure_layout_dimensions_collects_alignment_keys_from_wrapper_layouts() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let child = normalize_layout_view(
        AnyView::new(().size(20.0, 10.0).horizontal_alignment_guide(
            HorizontalAlignment::Leading,
            |dimensions: &ViewDimensions| dimensions.size.width * 0.5,
        )),
        &env,
    );
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: Computed::constant(0.0),
    };
    let mut state = HydroState::default();
    let dimensions = measure_layout_dimensions(
        &layout,
        [&child],
        ProposalSize::UNSPECIFIED,
        &mut state,
        &env,
        &theme,
    );

    assert_eq!(
        dimensions.explicit_horizontal(HorizontalAlignment::Leading),
        Some(10.0)
    );
}

#[test]
fn layout_normalization_does_not_charge_metadata_depth_to_component_recursion() {
    let env = test_environment();
    let mut view = AnyView::new(text("Leaf"));
    for _ in 0..80 {
        view = AnyView::new(view.opacity(1.0));
    }

    let normalized = normalize_layout_view(view, &env);

    assert!(normalized.is::<Metadata<Opacity>>());
}

#[test]
#[should_panic(expected = "hydrolysis layout normalization exceeded recursion budget")]
fn layout_normalization_still_rejects_recursive_component_bodies() {
    let env = test_environment();

    let _ = normalize_layout_view(AnyView::new(RecursivelyErasedView), &env);
}

#[test]
fn scale_metadata_is_layout_transparent() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let scale = Binding::f32(1.0);
    let view = normalize_layout_view(
        AnyView::new(
            ().size(80.0, 80.0)
                .scale(scale.clone(), scale.clone())
                .min_height(120.0),
        ),
        &env,
    );

    let mut state = HydroState::default();
    let initial = measure_view_dimensions(&view, &mut state, &env, &theme).size;

    scale.set(2.0);
    let mut state = HydroState::default();
    let scaled = measure_view_dimensions(&view, &mut state, &env, &theme).size;

    assert_eq!(initial, LayoutSize::new(80.0, 120.0));
    assert_eq!(scaled, initial);
}

#[test]
fn hydro_subview_preserves_stretch_control_minimum_under_zero_width_proposal() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let value = Binding::f64(0.5);
    let view = normalize_layout_view(
        AnyView::new(slider("Playback position", &value).hide_label()),
        &env,
    );
    let mut state = HydroState::default();
    let state = RefCell::new(&mut state);
    let subview = HydroSubview::from_view(&view, &state, &env, &theme);

    let measured = subview.measure(ProposalSize::new(Some(0.0), None));

    assert!(
        measured.size.width > 0.0,
        "Hydrolysis stretch controls must preserve their intrinsic minimum under constrained measurement"
    );
}

#[test]
fn hydro_subview_preserves_non_stretch_button_intrinsic_under_zero_width_proposal() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let view = normalize_layout_view(AnyView::new(button("Medium (0.7)").action(|| {})), &env);
    let mut state = HydroState::default();
    let state = RefCell::new(&mut state);
    let subview = HydroSubview::from_view(&view, &state, &env, &theme);

    let intrinsic = subview.measure(ProposalSize::UNSPECIFIED);
    let constrained = subview.measure(ProposalSize::new(Some(0.0), None));

    assert_eq!(
        constrained.size.width, intrinsic.size.width,
        "Hydrolysis non-stretch controls must not be compressed below their intrinsic text width"
    );
}

#[test]
fn state_wrapped_button_remains_non_stretch_for_layout() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let expanded = Binding::bool(false);
    let view = normalize_layout_view(
        AnyView::new(
            button("Toggle Bars")
                .action(|waterui::State(value): waterui::State<Binding<bool>>| {
                    value.toggle();
                })
                .state(&expanded),
        ),
        &env,
    );
    let mut state = HydroState::default();
    let state = RefCell::new(&mut state);
    let subview = HydroSubview::from_view(&view, &state, &env, &theme);

    let intrinsic = subview.measure(ProposalSize::UNSPECIFIED);
    let proposed = subview.measure(ProposalSize::new(Some(720.0), None));

    assert_eq!(subview.stretch_axis(), StretchAxis::None);
    assert_eq!(
        proposed.size.width, intrinsic.size.width,
        "environment state metadata must not make a button stretch across its VStack row"
    );
}

#[test]
fn vstack_places_state_wrapped_button_at_intrinsic_width() {
    let env = test_environment();
    let expanded = Binding::bool(false);
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
            .floating_with(floating_style.clone()),)),
        &env,
        Rect::new(0.0, 0.0, 160.0, 160.0),
    );
    let intrinsic_bounds = renderer
        .hit_test
        .pointer_targets
        .first()
        .expect("floating button must register an intrinsic pointer target")
        .bounds;
    assert_eq!(intrinsic_bounds.width(), 37.0);
    assert_eq!(intrinsic_bounds.height(), 41.0);
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
            .plain(),))
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
            GestureObserver::new(DragGesture::new(0.0), || {}),
        ),
        GestureObserver::new(MagnificationGesture::new(1.0), || {}),
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
        (zoom_in.bounds.x0 + zoom_in.bounds.x1) * 0.5,
        (zoom_in.bounds.y0 + zoom_in.bounds.y1) * 0.5,
    );
    let _ =
        renderer.handle_pointer_down(point.x as f32, point.y as f32, PointerButton::Primary, &env);
    assert!(
        renderer.flush_window_tree(
            &env,
            Rect::new(0.0, 0.0, 160.0, 160.0),
            Affine::IDENTITY,
            Affine::IDENTITY,
        ),
        "pressed controls must survive a retained redraw before pointer release"
    );
    assert!(renderer.handle_pointer_up(
        point.x as f32,
        point.y as f32,
        PointerButton::Primary,
        &env,
    ));

    assert_eq!(zoom.snapshot(), 0.5);
    assert!(
        renderer.take_patch_request(),
        "a synchronous button action must schedule a retained-tree refresh"
    );
}

#[test]
fn gpu_surface_external_redraw_is_consumed_during_continuous_frames() {
    use waterui_graphics::RedrawHandle;

    let redraw_handle = RedrawHandle::new();
    redraw_handle.request_redraw();

    assert!(super::render::take_gpu_surface_redraw_request(
        true,
        &redraw_handle
    ));
    assert!(
        !redraw_handle.is_dirty(),
        "a continuous inner frame must not leave the external wake coalesced forever"
    );
}

#[test]
fn draggable_metadata_delivers_drag_data_to_drop_destination() {
    use std::{cell::RefCell, rc::Rc};
    use waterui::drag_drop::DragData;
    use waterui::prelude::hstack;

    let dropped = Rc::new(RefCell::new(None::<String>));
    let dropped_target = Rc::clone(&dropped);
    let view = hstack((
        ().size(60.0, 60.0).draggable(DragData::text("🍎 Apple")),
        ().size(60.0, 60.0).drop_destination(move |data: DragData| {
            *dropped_target.borrow_mut() = Some(data.as_str().to_owned());
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
fn gesture_group_identity_collapses_nested_gesture_observers_on_same_view() {
    let view = AnyView::new(Metadata::new(
        Metadata::new(
            ().size(20.0, 10.0),
            GestureObserver::new(DragGesture::new(8.0), || {}),
        ),
        GestureObserver::new(MagnificationGesture::new(1.0), || {}),
    ));
    let outer = view
        .downcast_ref::<Metadata<GestureObserver>>()
        .expect("expected outer gesture observer metadata");
    let inner = outer
        .content
        .downcast_ref::<Metadata<GestureObserver>>()
        .expect("expected inner gesture observer metadata");

    assert_eq!(
        gesture_group_identity(&outer.content),
        gesture_group_identity(&inner.content)
    );
}

#[test]
fn renderer_magnification_targets_outer_observer_in_stacked_gesture_chain() {
    use std::{cell::Cell, rc::Rc};
    use waterui_core::Metadata;

    let offset = Binding::f32(0.0);
    let scale = Binding::f32(1.0);
    let drag_hits = Rc::new(Cell::new(0u32));
    let magnify_hits = Rc::new(Cell::new(0u32));
    let view = {
        let canvas = Canvas::with_signal(offset.zip(&scale), |_ctx, (_offset, _scale)| {})
            .size(120.0, 120.0);
        let canvas = {
            let drag_hits = Rc::clone(&drag_hits);
            Metadata::new(
                canvas,
                GestureObserver::new(DragGesture::new(0.0), move || {
                    drag_hits.set(drag_hits.get() + 1);
                }),
            )
        };
        {
            let magnify_hits = Rc::clone(&magnify_hits);
            Metadata::new(
                canvas,
                GestureObserver::new(MagnificationGesture::new(1.0), move || {
                    magnify_hits.set(magnify_hits.get() + 1);
                }),
            )
        }
    };

    let mut platform =
        crate::platform::OffscreenWindow::new_for_tests(160, 160, wgpu::TextureFormat::Rgba8Unorm);
    let mut renderer = {
        let surface = platform.surface();
        HydrolysisRenderer::new(
            surface.adapter(),
            surface.device(),
            Rc::new(MinimalTestTheme::default()),
        )
    };
    let env = test_environment();
    let bounds = vello::kurbo::Rect::new(0.0, 0.0, 160.0, 160.0);
    let surface = platform.surface();
    renderer.set_frame_resources(
        surface.adapter(),
        surface.device(),
        surface.queue(),
        surface.device_loss(),
    );
    capture_root_window(&mut renderer, view, &env, bounds);

    let point = vello::kurbo::Point::new(60.0, 60.0);
    let debug_targets = renderer.gesture_engine.debug_targets_at(point);
    assert_eq!(
        debug_targets.len(),
        2,
        "expected stacked drag+magnification gesture targets at point, got {:?}",
        debug_targets
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
    let mut state = HydroState::default();
    let proposal = ProposalSize::UNSPECIFIED;

    let raw = measure_view_dimensions_with_proposal(
        &AnyView::new(Str::from("Hydrolysis")),
        proposal,
        &mut state,
        &env,
        &theme,
    );
    let borrowed = measure_view_dimensions_with_proposal(
        &AnyView::new("Hydrolysis"),
        proposal,
        &mut state,
        &env,
        &theme,
    );
    let owned = measure_view_dimensions_with_proposal(
        &AnyView::new(String::from("Hydrolysis")),
        proposal,
        &mut state,
        &env,
        &theme,
    );
    let cow = measure_view_dimensions_with_proposal(
        &AnyView::new(Cow::Borrowed("Hydrolysis")),
        proposal,
        &mut state,
        &env,
        &theme,
    );

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

fn capture_root_window<V: waterui_core::View>(
    renderer: &mut HydrolysisRenderer,
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
    let env = test_environment().extending(waterui_graphics::SceneViewMergeToParent);
    let mut renderer = test_renderer();
    let recording = waterui_graphics::Picture::record(|_scene| {});
    let picture = waterui_graphics::Picture::new(
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
    let field_value = Binding::container(Str::from(""));
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
/// called with, and layout normalization resolves that body before the naming
/// scope exists — so the snapshot carries no label, and flattening it replaces
/// the scoped environment the build installed. Every `Map` in a `map-gpu` build
/// and every `Video` in a `video-gpu` one sits under such a hook, and
/// `.a11y_label()` on them named nothing at all: the engine, finding no ambient
/// label, announced its own generated description instead.
#[cfg(feature = "accessibility")]
#[test]
fn a_label_survives_the_environment_snapshot_a_view_hook_takes() {
    use waterui_core::AnyView;
    use waterui_map::{Coordinate, Map, MapConfig, Region};

    let mut env = test_environment();
    // The hooked body stands in for a real map realization — `Native<MapConfig>`
    // is unreachable because the backend has no map engine and panics on it.
    env.insert_hook::<MapConfig, AnyView>(|_env, _config| {
        AnyView::new(text("map").a11y_role(AccessibilityRole::Image))
    });
    let mut renderer = test_renderer();
    // The frame matters: a layout container normalizes its children, and it is
    // normalization that resolves the hooked body — one level above the naming
    // scope the build installs.
    let view = Map::new(Region::new(Coordinate::default(), 1.0, 1.0))
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

/// A `WebView` created under an application-provided controller exists — the
/// controller permits it — but on a build bridging no engine there is nothing
/// to draw it with, and the backend fails rather than occupying a layout slot
/// with no page behind it.
#[cfg(not(hydrolysis_macos_system_webview))]
#[test]
#[should_panic(expected = "no web engine is bridged")]
fn a_webview_with_no_engine_to_draw_it_panics() {
    use std::future::{Future, ready};

    use waterui_core::{Signal, Str};
    use waterui_webview::{
        BackendEvent, Cookie, CustomWebViewController, OriginPolicy, ScriptInjectionTime,
        ScriptMessageHandler, WatcherGuard, WatcherSet, WebView, WebViewConfig, WebViewController,
        WebViewHandle,
    };

    /// The smallest controller that can still create a `WebView`: the handle
    /// answers what construction asks and discards the rest, because the
    /// assertion only needs the view to exist long enough for the backend to
    /// refuse it.
    struct TestWebViewController;

    impl CustomWebViewController for TestWebViewController {
        fn open(&self, _config: WebViewConfig) -> impl WebViewHandle {
            TestWebViewHandle {
                watchers: WatcherSet::new(),
            }
        }
    }

    struct TestWebViewHandle {
        watchers: WatcherSet<BackendEvent>,
    }

    impl WebViewHandle for TestWebViewHandle {
        fn go_back(&self) {}
        fn go_forward(&self) {}
        fn go_to(&self, _url: &waterui_webview::Url) {}
        fn stop(&self) {}
        fn refresh(&self) {}
        fn set_user_agent(&self, _user_agent: &str) {}
        fn can_go_back(&self) -> bool {
            false
        }
        fn can_go_forward(&self) -> bool {
            false
        }
        fn inject_script(&self, _key: &str, _script: &str, _time: ScriptInjectionTime) {}
        fn add_handler(&self, _name: &str, _handler: Box<ScriptMessageHandler>) {}
        fn remove_handler(&self, _name: &str) {}
        fn set_bridge_origins(&self, _policy: OriginPolicy) {}
        // No interception facility: the double has no engine to route an asset
        // origin through, and the assertion it serves never opens assets.
        fn asset_origin(&self) -> Option<waterui_webview::Url> {
            None
        }
        fn set_cookie(&self, _cookie: Cookie<'static>) {}
        fn set_redirects_enabled(&self, _enabled: impl Signal<Output = bool>) {}
        fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
            self.watchers.insert(f)
        }
        fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> {
            ready(Vec::new())
        }
        fn run_javascript(&self, _script: &str) -> impl Future<Output = Result<Str, Str>> {
            ready(Err(Str::from_static("no page")))
        }
        fn call_async_javascript(&self, _body: &str) -> impl Future<Output = Result<Str, Str>> {
            ready(Err(Str::from_static("no page")))
        }
    }

    let mut env = test_environment();
    env.insert(WebViewController::new(TestWebViewController));
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
    use nami::collection::List;
    use waterui_core::id::SelfId;
    use waterui_layout::stack::VStack;

    let env = test_environment();
    let mut renderer = test_renderer();
    let rows: List<SelfId<u64>> = List::from(vec![SelfId::new(0), SelfId::new(1)]);
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
    use nami::collection::List;
    use waterui_core::id::SelfId;
    use waterui_layout::stack::ZStack;

    let env = test_environment();
    let mut renderer = test_renderer();
    let items: List<SelfId<u64>> = List::from(vec![SelfId::new(0), SelfId::new(1)]);
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
    let visible = Binding::bool(false);
    let state = visible.map(|visible| AccessibilityState::new().hidden(!visible));
    let view = vstack((text("Hidden child"),))
        .a11y_label("Hidden group")
        .a11y_role(AccessibilityRole::Group)
        .a11y_state_signal(state);

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
    assert!(group.children().is_empty());
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
    let busy = Binding::bool(false);
    let state = busy.map(|busy| AccessibilityState::new().mixed().busy(busy));
    let view = text("Tri-state")
        .a11y_role(AccessibilityRole::Checkbox)
        .a11y_state_signal(state);
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

#[cfg(feature = "accessibility")]
#[test]
fn disabled_picker_family_and_tabs_expose_no_mutating_actions() {
    use core::convert::TryFrom as _;
    use jiff::civil::Date;
    use waterui_core::id::Id;

    let env = test_environment();
    let mut renderer = test_renderer();
    let picker_selection = Binding::container("Alpha");
    let date = Binding::container(Date::new(2025, 1, 10).expect("valid test date"));
    let color = Binding::container(Color::srgb(0, 0, 0));
    let first = Id::try_from(1).expect("non-zero tab id");
    let second = Id::try_from(2).expect("non-zero tab id");
    let tabs = TabsLayout::new(
        Binding::container(first),
        vec![
            Tab::new(first, "First tab", || {
                NavigationView::new("First", text("One"))
            }),
            Tab::new(second, "Second tab", || {
                NavigationView::new("Second", text("Two"))
            }),
        ],
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
fn interaction_press_origin_is_converted_to_widget_local_space() {
    let mut press_waves = waterui_backend_core::widget::PressWaves::EMPTY;
    press_waves.push(waterui_backend_core::widget::PressWave {
        origin: Some(Point::new(125.0, 84.0)),
        progress: 0.5,
        opacity: 0.12,
    });
    let state = WidgetInteractionState {
        press_waves,
        ..WidgetInteractionState::NONE
    };

    let local = crate::renderer::local_interaction_state(state, Affine::translate((100.0, 80.0)));

    let wave = local
        .press_waves
        .latest()
        .expect("wave must survive the local-space mapping");
    assert_eq!(wave.origin, Some(Point::new(25.0, 4.0)));
    assert_eq!(wave.progress, 0.5);
    assert_eq!(wave.opacity, 0.12);
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
        .begin_press(&slot, Point::new(20.0, 20.0), at);
    renderer.finish_rebuild_frame();

    renderer.begin_rebuild_frame();
    let (state, _, _) =
        renderer.bind_interaction_target(second_key, Rect::new(100.0, 100.0, 180.0, 180.0), &env);

    assert!(!state.pressed);
    assert!(state.press_waves.is_empty());
}

/// Regression guard for the retained-path state-layer loss: a began press must
/// surface through the re-bound `WidgetInteractionState` as a visible press
/// layer once the fade-in has run, because widgets draw their themed state
/// layers from exactly this sampled state on every flush.
#[test]
fn began_press_samples_a_visible_press_layer_after_fade_in() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 80.0, 80.0);
    let owner = Rc::new(());
    let key = InteractionKey::for_rc(&owner, 0);

    renderer.begin_rebuild_frame();
    let (_, slot, _) = renderer.bind_interaction_target(key.clone(), bounds, &env);
    let at = renderer.frame_instant();
    renderer
        .hit_test
        .interaction
        .begin_press(&slot, Point::new(20.0, 20.0), at);
    renderer.finish_rebuild_frame();

    // Advance past the press fade-in (105ms in MinimalTestTheme) and re-bind:
    // the sampled state must carry a visible press layer, its origin, and
    // non-zero grow progress.
    let later = renderer
        .frame_instant()
        .checked_add(Duration::from_millis(200))
        .expect("test press deadline overflow");
    renderer.set_frame_instant(later);
    renderer.begin_rebuild_frame();
    let (state, _, _) = renderer.bind_interaction_target(key, bounds, &env);
    assert!(state.pressed, "held press must stay visually pressed");
    let wave = state
        .press_waves
        .latest()
        .expect("held press must sample a visible wave");
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
    );

    assert!(state.focus_visible);
    assert_eq!(state.focus_progress, 1.0);
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
    let (_, press_slot, handles) = renderer.bind_interaction_target(key, bounds, &env);
    renderer.register_interactive_pointer_target(bounds, press_slot, move |_, _, _| {
        action_activations.set(action_activations.get() + 1);
        true
    });
    let started = renderer.frame_instant();

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
        let activation: AccessibilityActivation = Rc::new(RefCell::new(
            move |_: &mut SemanticCore, _: &Environment| {
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
    let focused = Binding::bool(false);
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
    let mut dialog_env = env.clone();
    let active = Binding::bool(false);
    dialog_env.insert(
        ModalInteraction::new(false, SharedAction::new(|_: Environment| {})).active(active.clone()),
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

#[derive(Default)]
pub(crate) struct MinimalTestTheme {
    badge_draws: Rc<RefCell<Vec<Rect>>>,
}

impl crate::Style for MinimalTestTheme {
    /// The minimal theme installs no tokens of its own — the runtime's
    /// framework defaults (`install_theme_tokens`) are all a test needs.
    fn install_tokens(&self, _env: &mut Environment) {}
}

impl WidgetTheme for MinimalTestTheme {
    fn interaction_motion(&self) -> InteractionMotion {
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
            minimum_press_duration: Duration::from_millis(225),
            touch_delay: Duration::from_millis(150),
        }
    }

    fn progress_motion(&self) -> ProgressMotion {
        ProgressMotion {
            linear_determinate: Animation::bezier(Duration::from_millis(250), 0.4, 0.0, 0.6, 1.0),
            circular_determinate: Animation::bezier(Duration::from_millis(500), 0.0, 0.0, 0.2, 1.0),
            linear_indeterminate_cycle: Duration::from_millis(2_000),
            loading_cycle: Duration::from_millis(4_666),
            circular_indeterminate_cycle: Duration::from_millis(5_332),
        }
    }

    fn text_caret_motion(&self) -> TextCaretMotion {
        TextCaretMotion {
            fade_cycle_duration: Duration::from_millis(1_060),
            frame_interval: Duration::from_millis(530),
            min_opacity: 0.2,
        }
    }

    fn navigation_motion(&self) -> NavigationMotion {
        NavigationMotion {
            transition_duration: Duration::from_millis(450),
            transition_easing: EasingCurve::bezier(0.2, 0.0, 0.0, 1.0),
            shared_axis_slide_distance: 30.0,
            fade_through_threshold: 0.35,
        }
    }

    fn button_metrics(&self, _style: ButtonStyle, _size: ButtonSize) -> ButtonMetrics {
        ButtonMetrics {
            padding_x: 1.0,
            padding_y: 2.0,
            min_width: 123.0,
            min_height: 45.0,
        }
    }

    fn icon_button_metrics(&self, _style: ButtonStyle, _size: ButtonSize) -> ButtonMetrics {
        ButtonMetrics::new(0.0, 0.0, 41.0, 43.0)
    }

    fn draw_button_chrome(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _style: ButtonStyle,
        _icon_only: bool,
        _state: WidgetInteractionState,
    ) {
    }

    fn toggle_metrics(&self, _style: ToggleStyle) -> ToggleMetrics {
        ToggleMetrics {
            width: 10.0,
            height: 20.0,
            label_spacing: 3.0,
        }
    }

    fn toggle_value_animation(&self) -> Animation {
        Animation::linear(Duration::from_millis(100))
    }

    fn draw_toggle_switch(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f32,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }

    fn draw_toggle_checkbox(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f32,
        _state: WidgetInteractionState,
    ) {
    }

    fn stepper_metrics(&self) -> StepperMetrics {
        StepperMetrics {
            button_min_size: 12.0,
            button_max_size: 18.0,
            button_intrinsic_size: 14.0,
            button_spacing: 4.0,
            label_spacing: 8.0,
        }
    }

    fn draw_stepper_button(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _end: StepperEnd,
        _state: WidgetInteractionState,
    ) {
    }
    fn draw_stepper_decrement_icon(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_stepper_increment_icon(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn input_field_metrics(&self) -> InputFieldMetrics {
        InputFieldMetrics {
            label_height: 14.0,
            min_width: 100.0,
            min_height: 32.0,
            horizontal_inset: 8.0,
            vertical_inset: 6.0,
        }
    }

    fn input_placeholder_color(&self) -> waterui_graphics::color::Color {
        waterui_graphics::color::Color::srgb(0, 0, 0)
    }

    fn input_selection_brush(&self) -> Brush {
        Brush::from(vello::peniko::Color::new([0.20, 0.45, 0.90, 0.28]))
    }

    fn input_caret_brush(&self, opacity: f32) -> Brush {
        Brush::from(vello::peniko::Color::new([0.12, 0.14, 0.18, opacity]))
    }

    fn draw_input_field(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }

    fn text_context_menu_metrics(&self) -> TextContextMenuMetrics {
        TextContextMenuMetrics {
            row_height: 56.0,
            horizontal_padding: 16.0,
            vertical_padding: 12.0,
            min_width: 112.0,
            max_width: 320.0,
            width_per_char: 8.5,
            corner_radius: 4.0,
            separator_horizontal_inset: 16.0,
            separator_thickness: 1.0,
        }
    }

    fn draw_text_context_menu_panel(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        let radii = vello::kurbo::RoundedRectRadii::from_single_radius(
            self.text_context_menu_metrics().corner_radius,
        );
        // A level-2-like shadow under the panel, deep enough for tests to
        // distinguish it from the scrim's uniform dim.
        draw.draw_shadow(
            bounds,
            radii,
            vello::kurbo::Vec2::new(0.0, 3.0),
            6.0,
            vello::peniko::Color::new([0.0, 0.0, 0.0, 0.35]),
        );
        draw.fill_rounded_rect(
            bounds,
            radii,
            &Brush::Solid(vello::peniko::Color::new([0.96, 0.94, 0.97, 1.0])),
        );
    }

    fn draw_text_context_menu_separator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn picker_metrics(&self, _style: PickerStyle) -> PickerMetrics {
        PickerMetrics {
            min_width: 72.0,
            min_height: 28.0,
            horizontal_inset: 8.0,
            vertical_inset: 6.0,
            label_spacing: 8.0,
            indicator_space: 18.0,
            radio_indicator_size: 16.0,
            radio_label_spacing: 8.0,
            radio_row_spacing: 8.0,
            popup_top_spacing: 4.0,
            popup_row_height: 48.0,
            popup_corner_radius: 6.0,
            segment_min_width: 58.0,
        }
    }

    fn radio_selection_motion(&self) -> RadioSelectionMotion {
        RadioSelectionMotion {
            inner_grow: Animation::linear(Duration::from_millis(1)),
            inner_opacity: Animation::linear(Duration::from_millis(1)),
            outer_color: Animation::linear(Duration::from_millis(1)),
        }
    }

    fn draw_picker_indicator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn draw_picker_popup(&self, _draw: &mut dyn DrawContext, _popup_rect: Rect) {}

    fn draw_picker_popup_row_background(
        &self,
        _draw: &mut dyn DrawContext,
        _row_rect: Rect,
        _selected: bool,
    ) {
    }

    fn draw_picker_separator(&self, _draw: &mut dyn DrawContext, _separator: Rect) {}

    fn draw_radio_indicator(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _state: RadioIndicatorState,
    ) {
    }

    fn slider_metrics(&self) -> SliderMetrics {
        SliderMetrics {
            horizontal_inset: 12.0,
            horizontal_spacing: 8.0,
            vertical_spacing: 6.0,
            min_track_width: 72.0,
            track_height: 6.0,
            handle_width: 4.0,
            handle_height: 44.0,
        }
    }

    fn draw_slider_track(
        &self,
        _draw: &mut dyn DrawContext,
        _track_rect: Rect,
        _fill_rect: Rect,
        _state: WidgetInteractionState,
    ) {
    }

    fn draw_slider_thumb(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _state: WidgetInteractionState,
    ) {
    }

    fn progress_metrics(&self, style: ProgressIndicatorStyle) -> ProgressMetrics {
        match style {
            ProgressIndicatorStyle::Loading => ProgressMetrics::loading(48.0, 38.0),
            ProgressIndicatorStyle::Linear => ProgressMetrics {
                label_height: 18.0,
                bar_top_offset: 10.0,
                bar_height: 8.0,
                bar_horizontal_inset: 8.0,
                value_label_top_spacing: 6.0,
                min_track_width: 72.0,
                circular_diameter: 0.0,
                circular_stroke_width: 0.0,
                loading_indicator_size: 0.0,
            },
            ProgressIndicatorStyle::Circular => ProgressMetrics {
                label_height: 0.0,
                bar_top_offset: 0.0,
                bar_height: 0.0,
                bar_horizontal_inset: 0.0,
                value_label_top_spacing: 0.0,
                min_track_width: 0.0,
                circular_diameter: 32.0,
                circular_stroke_width: 5.0,
                loading_indicator_size: 0.0,
            },
        }
    }

    fn draw_progress_linear_track(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _active_end: Option<f64>,
    ) {
    }
    fn draw_progress_linear_fill(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_progress_linear_indeterminate(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _elapsed: Duration,
        _four_color: bool,
    ) {
    }
    fn draw_progress_circular_track(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _width: f64,
        _active_turns: Option<f64>,
    ) {
    }
    fn draw_progress_circular_fill(
        &self,
        _draw: &mut dyn DrawContext,
        _path: &BezPath,
        _width: f64,
    ) {
    }
    fn draw_progress_loading(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _elapsed: Duration,
        _four_color: bool,
    ) {
    }

    fn draw_progress_circular_indeterminate(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _width: f64,
        _elapsed: Duration,
        _four_color: bool,
    ) {
    }

    fn navigation_metrics(&self) -> NavigationMetrics {
        NavigationMetrics {
            automatic_bar_height: 64.0,
            inline_bar_height: 64.0,
            medium_bar_height: 112.0,
            large_bar_height: 152.0,
            inline_title_height: 28.0,
            medium_title_height: 36.0,
            large_title_height: 36.0,
            title_leading_inset: 16.0,
            title_trailing_inset: 16.0,
            large_title_bottom_inset: 28.0,
            horizontal_inset: 4.0,
            item_spacing: 0.0,
            search_height: 56.0,
            search_vertical_inset: 4.0,
            back_button_size: 40.0,
            back_button_leading_inset: 4.0,
            back_button_top_inset: 12.0,
        }
    }

    fn draw_navigation_bar(&self, _draw: &mut dyn DrawContext, _bounds: Rect, _background: &Brush) {
    }

    fn draw_navigation_bar_separator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_navigation_back_button(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn tabs_metrics(&self) -> TabsMetrics {
        TabsMetrics {
            bar_height: 48.0,
            button_min_width: 48.0,
            button_horizontal_inset: 16.0,
            active_indicator_height: 3.0,
            active_indicator_radius: 3.0,
        }
    }
    fn draw_tabs_bar(&self, _draw: &mut dyn DrawContext, _bounds: Rect, _top_edge: bool) {}
    fn draw_tabs_highlight(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_scroll_indicator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn divider_metrics(&self) -> DividerMetrics {
        DividerMetrics { thickness: 1.0 }
    }

    fn draw_divider(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn badge_metrics(&self) -> BadgeMetrics {
        BadgeMetrics {
            small_size: 6.0,
            large_size: 16.0,
            large_horizontal_padding: 4.0,
            small_offset_x: 6.0,
            small_offset_y: 6.0,
            large_offset_x: 12.0,
            large_offset_y: 14.0,
        }
    }

    fn badge_label_color(&self) -> Color {
        Color::srgb(255, 255, 255)
    }

    fn badge_label_font(&self) -> waterui_text::font::Font {
        waterui_text::font::Font::default()
    }

    fn draw_badge_small(&self, _draw: &mut dyn DrawContext, bounds: Rect) {
        self.badge_draws.borrow_mut().push(bounds);
    }
    fn draw_badge_large(&self, _draw: &mut dyn DrawContext, bounds: Rect) {
        self.badge_draws.borrow_mut().push(bounds);
    }

    fn list_metrics(&self) -> ListMetrics {
        ListMetrics {
            one_line_row_height: 56.0,
            horizontal_inset: 16.0,
            vertical_inset: 10.0,
            divider_leading_inset: 16.0,
            divider_trailing_inset: 16.0,
            move_control_width: 20.0,
            delete_control_width: 26.0,
            trailing_control_spacing: 6.0,
            trailing_control_vertical_inset: 6.0,
            section_header_height: 48.0,
            section_footer_height: 40.0,
        }
    }

    fn draw_list_row_background(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _alternate: bool,
    ) {
    }
    fn draw_list_move_control(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_list_delete_control(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_list_separator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn table_metrics(&self) -> TableMetrics {
        TableMetrics {
            min_column_width: 72.0,
            cell_horizontal_padding: 32.0,
            cell_vertical_inset: 16.0,
            header_height: 56.0,
            row_height: 52.0,
            outline_width: 1.0,
        }
    }

    fn draw_table_background(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_table_header_background(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_table_cell_border(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_table_column_separator(&self, _draw: &mut dyn DrawContext, _from: Point, _to: Point) {}
}

#[test]
fn ime_preedit_commit_and_disable_update_focused_text_target() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(MinimalTestTheme::default().text_caret_motion());
    let selection = Rc::new(RefCell::new(TextSelectionSlot {
        anchor: 0,
        focus: 0,
        initialized: true,
    }));
    renderer
        .text_editing
        .text_input_targets
        .push(text_input_target(
            text_field_model("", None),
            Rc::clone(&selection),
        ));

    assert!(renderer.set_focused_text_input(Some(0)));
    assert!(
        renderer.take_patch_request(),
        "text input focus changes must refresh the retained tree so focus animations start on click"
    );
    assert!(
        !renderer.take_rebuild_request(),
        "text input focus changes must not rebuild the view body"
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
    renderer.set_text_caret_motion(MinimalTestTheme::default().text_caret_motion());
    let first = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let focused = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let emit = |renderer: &mut HydrolysisRenderer,
                targets: &[(&str, &Rc<RefCell<TextSelectionSlot>>)]| {
        renderer.text_editing.text_input_targets.clear();
        for (value, selection) in targets {
            renderer
                .text_editing
                .text_input_targets
                .push(text_input_target(
                    text_field_model(value, None),
                    Rc::clone(selection),
                ));
        }
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
    renderer.set_text_caret_motion(MinimalTestTheme::default().text_caret_motion());
    let survivor = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let removed = Rc::new(RefCell::new(TextSelectionSlot::default()));
    for (value, selection) in [("survivor", &survivor), ("removed", &removed)] {
        renderer
            .text_editing
            .text_input_targets
            .push(text_input_target(
                text_field_model(value, None),
                Rc::clone(selection),
            ));
    }
    assert!(renderer.set_focused_text_input(Some(1)));

    renderer.text_editing.text_input_targets.clear();
    renderer
        .text_editing
        .text_input_targets
        .push(text_input_target(
            text_field_model("survivor", None),
            Rc::clone(&survivor),
        ));
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
    renderer
        .text_editing
        .text_input_targets
        .push(text_input_target(
            text_field_model("selection", None),
            Rc::clone(&selection),
        ));

    assert!(renderer.update_text_selection_from_pointer(0, Point::ZERO, false));
    assert!(
        !renderer.take_rebuild_request(),
        "text selection changes are rendered by the transient overlay instead of a full scene rebuild"
    );
    assert!(!renderer.update_text_selection_from_pointer(0, Point::ZERO, false));
    assert!(
        !renderer.take_rebuild_request(),
        "unchanged text selection must not schedule redundant rebuilds"
    );
}

/// A text-input target with real bounds and a real shaped layout, so click
/// gestures resolve to actual caret/word/line ranges instead of an empty
/// layout's index 0.
fn shaped_text_input_target(
    value: &str,
    selection: &Rc<RefCell<TextSelectionSlot>>,
    env: &Environment,
) -> TextInputTarget {
    let mut state = HydroState::default();
    let layout = HydrolysisRenderer::build_text_layout(
        &mut state,
        StyledStr::plain(value.to_owned()),
        HorizontalAlignment::Leading,
        env,
        Some(200.0),
    );
    let mut target = text_input_target(text_field_model(value, None), Rc::clone(selection));
    target.bounds = Rect::new(0.0, 0.0, 200.0, 60.0);
    target.text_bounds = Rect::new(0.0, 0.0, 200.0, 60.0);
    target.text_clip_bounds = target.text_bounds;
    target.cursor_area = target.text_bounds;
    target.layout = layout;
    target.env = env.clone();
    target
}

/// The window point at the center of the caret geometry for `byte_index`, used
/// to aim synthetic clicks inside a specific word.
fn caret_point_in_target(target: &TextInputTarget, byte_index: usize) -> Point {
    let cursor =
        parley::Cursor::from_byte_index(&target.layout, byte_index, parley::Affinity::Downstream);
    let geometry = cursor.geometry(&target.layout, 1.0);
    Point::new(
        target.text_bounds.x0 + (geometry.x0 + geometry.x1) * 0.5,
        target.text_bounds.y0 + (geometry.y0 + geometry.y1) * 0.5,
    )
}

/// The release event re-runs the drag-extension path before the drag is
/// cleared. A double-click drag must keep its word granularity there — and for
/// any jitter between down and up — instead of collapsing the word the gesture
/// selected back to the caret under the pointer.
#[test]
fn double_click_word_selection_survives_pointer_release() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(MinimalTestTheme::default().text_caret_motion());
    let env = test_environment();
    let selection = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let target = shaped_text_input_target("hello world", &selection, &env);
    let point = caret_point_in_target(&target, 8);
    renderer.text_editing.text_input_targets.push(target);

    renderer.handle_pointer_down(point.x as f32, point.y as f32, PointerButton::Primary, &env);
    renderer.handle_pointer_up(point.x as f32, point.y as f32, PointerButton::Primary, &env);
    renderer.handle_pointer_down(point.x as f32, point.y as f32, PointerButton::Primary, &env);
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        6..11,
        "double-click must select the whole word under the pointer"
    );

    renderer.handle_pointer_up(point.x as f32, point.y as f32, PointerButton::Primary, &env);
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
    renderer.set_text_caret_motion(MinimalTestTheme::default().text_caret_motion());
    let env = test_environment();
    let selection = Rc::new(RefCell::new(TextSelectionSlot::default()));
    let target = shaped_text_input_target("hello world", &selection, &env);
    let world_point = caret_point_in_target(&target, 8);
    let hello_point = caret_point_in_target(&target, 2);
    renderer.text_editing.text_input_targets.push(target);

    renderer.handle_pointer_down(
        world_point.x as f32,
        world_point.y as f32,
        PointerButton::Primary,
        &env,
    );
    renderer.handle_pointer_up(
        world_point.x as f32,
        world_point.y as f32,
        PointerButton::Primary,
        &env,
    );
    // Second click stays held: this is a double-click-drag, not a third click.
    renderer.handle_pointer_down(
        world_point.x as f32,
        world_point.y as f32,
        PointerButton::Primary,
        &env,
    );
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        6..11
    );

    // Still holding the second click's button, drag back across "hello": the
    // selection grows to cover both whole words, not a caret at the pointer.
    renderer.handle_pointer_move(hello_point.x as f32, hello_point.y as f32, &env);
    assert_eq!(
        normalized_selection_range(selection.borrow().anchor, selection.borrow().focus),
        0..11,
        "double-click drag must extend the selection word by word"
    );
    renderer.handle_pointer_up(
        hello_point.x as f32,
        hello_point.y as f32,
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
    let entries = SemanticCore::build_text_context_menu_entries(&target, &env);
    let labels = entries
        .iter()
        .filter_map(|entry| match entry {
            TextContextMenuEntry::Command { label, .. } => Some(label.as_str()),
            TextContextMenuEntry::Divider => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(labels, vec!["Paste", "Select All"]);
}

#[test]
fn bare_text_at_window_root_renders_into_scene() {
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
        !renderer.scene_is_empty(),
        "a bare text view at the window root must draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn bare_str_at_window_root_renders_into_scene() {
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
        !renderer.scene_is_empty(),
        "a bare string view at the window root must draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn text_shaping_produces_nonzero_intrinsic_in_tests() {
    let env = test_environment();
    let mut state = HydroState::default();
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
fn resolved_text_fast_path_matches_the_recursive_measure() {
    use core::cell::RefCell;

    let env = test_environment();
    let proposal = ProposalSize::UNSPECIFIED;

    // A text leaf resolves once into a layout input and is then shaped through the
    // text service directly, skipping the general recursion through `HydroState`.
    // That shortcut is only sound while it reports what the path it bypasses would.
    // Covers a bare `Str` and a string-like (`String`) bodied into text and recursed.
    let str_view = AnyView::new(Str::from("probe"));
    let string_view = AnyView::new(String::from("hello world"));

    for view in [&str_view, &string_view] {
        let mut state = HydroState::default();
        let state_cell = RefCell::new(&mut state);
        let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
        let fast_path = HydroSubview::from_view(view, &state_cell, &env, &theme).measure(proposal);
        let recursive = {
            let mut state = state_cell.borrow_mut();
            measure_view_dimensions_with_proposal(view, proposal, &mut state, &env, &theme)
        };

        assert_eq!(
            fast_path.size,
            recursive.size,
            "the resolved-text fast path for {} must equal the recursive measure",
            view.name()
        );
        assert!(
            fast_path.size.width > 0.0 && fast_path.size.height > 0.0,
            "text shaping must produce a non-zero size for {}",
            view.name()
        );
    }
}

#[test]
fn bare_str_renders_into_scene() {
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
        !renderer.scene_is_empty(),
        "a bare string must build a text node and draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn render_path_text_layout_has_lines() {
    let env = test_environment();
    let mut state = HydroState::default();
    let layout = HydrolysisRenderer::build_text_layout(
        &mut state,
        waterui_text::styled::StyledStr::plain("probe"),
        HorizontalAlignment::Leading,
        &env,
        Some(160.0),
    );
    assert!(
        !layout.is_empty(),
        "render-path text layout must not be empty"
    );
    assert!(layout.lines().next().is_some(), "layout must have lines");
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
    use core::cell::RefCell;

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
        ("button", AnyView::new(button("Tap"))),
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
        // The layout path measures normalized views, so the contract is about
        // those, not about raw bodies.
        let view = normalize_layout_view(view, &env);
        let mut state = HydroState::default();
        let cell = RefCell::new(&mut state);
        let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
        let subview = HydroSubview::from_view(&view, &cell, &env, &theme);

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
    use waterui::component::badge::Badge;
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
    let capture = |view: Badge, env: &Environment| {
        log.0.borrow_mut().clear();
        let mut renderer = test_renderer_with_theme(MinimalTestTheme {
            badge_draws: Rc::clone(&log.0),
        });
        capture_root_window(&mut renderer, view, env, bounds);
        log.0.borrow().clone()
    };

    // Dot (value 0): leading edge 6 inside the trailing edge, top edge flush —
    // the dot fills the content's top-trailing 6×6 corner.
    assert_eq!(
        capture(Badge::new(0, anchor.clone()), &env).as_slice(),
        &[Rect::new(154.0, 0.0, 160.0, 6.0)]
    );

    // Count badge: leading edge 12 inside the trailing edge regardless of its
    // width, bottom edge 14 below the content's top (its 16-high pill tops out
    // 2 above the anchor).
    let draws = capture(Badge::new(5, anchor.clone()), &env);
    assert_eq!(draws.len(), 1, "one badge indicator draw, got {draws:?}");
    assert_eq!(draws[0].x0, 148.0);
    assert_eq!(draws[0].y0, -2.0);

    // RTL mirrors the anchor to the leading edge.
    env.insert(LayoutDirection::RightToLeft);
    assert_eq!(
        capture(Badge::new(0, anchor.clone()), &env).as_slice(),
        &[Rect::new(0.0, 0.0, 6.0, 6.0)]
    );

    let draws = capture(Badge::new(5, anchor.clone()), &env);
    assert_eq!(draws.len(), 1, "one badge indicator draw, got {draws:?}");
    assert_eq!(draws[0].x1, 12.0);
    assert_eq!(draws[0].y0, -2.0);
}
