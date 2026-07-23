use super::*;
use std::borrow::Cow;
use std::rc::Rc;

mod perf_full_rebuild;
mod perf_scroll;
mod retained_scene;
mod tree;
use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
use waterui::gesture::{DragGesture, GestureObserver, MagnificationGesture};
use waterui::{Binding, Color, Computed, SignalExt as _, ViewExt as _};
use waterui_canvas::Canvas;
use waterui_controls::button::{ButtonStyle, button};
use waterui_controls::slider::slider;
use waterui_controls::toggle::ToggleStyle;
use waterui_form::picker::PickerStyle;
use waterui_layout::scroll;
use waterui_layout::stack::{VStackLayout, hstack, vstack};

use crate::engine::{Brush, DrawContext, WidgetTheme};
use crate::platform::PlatformWindow as _;
use crate::widgets::util::widget_theme;
use waterui_backend_core::widget::{
    BadgeMetrics, ButtonMetrics, DividerMetrics, InputFieldMetrics, InteractionMotion, ListMetrics,
    NavigationMetrics, NavigationMotion, PickerMetrics, ProgressIndicatorStyle, ProgressMetrics,
    ProgressMotion, RadioIndicatorState, RadioSelectionMotion, SliderMetrics, StepperMetrics,
    TableMetrics, TabsMetrics, TextCaretMotion, TextContextMenuMetrics, ToggleMetrics,
    WidgetInteractionState,
};

fn test_renderer() -> HydrolysisRenderer {
    let mut platform =
        crate::platform::OffscreenWindow::new_for_tests(160, 160, wgpu::TextureFormat::Rgba8Unorm);
    let surface = platform.surface();
    let mut renderer = HydrolysisRenderer::new(surface.device());
    renderer.set_frame_resources(surface.adapter(), surface.device(), surface.queue());
    renderer
}

fn test_environment() -> Environment {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        native_executor::NativeExecutor::new(),
    ));
    let mut env = Environment::new();
    crate::testing::install_theme(&mut env);
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    env
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

    fn get(&self) -> Self::Output {
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
    TextInputTarget {
        bounds: Rect::ZERO,
        cursor_area: Rect::ZERO,
        text_bounds: Rect::ZERO,
        text_clip_bounds: Rect::ZERO,
        content_alpha: 1.0,
        layout: parley::Layout::default(),
        purpose: TextInputPurpose::Normal,
        depth: 0,
        order: 0,
        model,
        selection,
        focus_binding: None,
        #[cfg(feature = "accessibility")]
        accessibility_node_id: None,
    }
}

#[test]
fn measure_layout_dimensions_collects_alignment_keys_from_wrapper_layouts() {
    let env = test_environment();
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
    );

    assert_eq!(
        dimensions.explicit_horizontal(HorizontalAlignment::Leading),
        Some(10.0)
    );
}

#[test]
fn scale_metadata_is_layout_transparent() {
    let env = test_environment();
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
    let initial = measure_view_dimensions(&view, &mut state, &env).size;

    scale.set(2.0);
    let mut state = HydroState::default();
    let scaled = measure_view_dimensions(&view, &mut state, &env).size;

    assert_eq!(initial, LayoutSize::new(80.0, 120.0));
    assert_eq!(scaled, initial);
}

#[test]
fn hydro_subview_preserves_stretch_control_minimum_under_zero_width_proposal() {
    let env = test_environment();
    let value = Binding::f64(0.5);
    let view = normalize_layout_view(
        AnyView::new(slider("Playback position", &value).hide_label()),
        &env,
    );
    let mut state = HydroState::default();
    let state = RefCell::new(&mut state);
    let subview = HydroSubview::from_view(&view, &state, &env);

    let measured = subview.measure(ProposalSize::new(Some(0.0), None));

    assert!(
        measured.size.width > 0.0,
        "Hydrolysis stretch controls must preserve their intrinsic minimum under constrained measurement"
    );
}

#[test]
fn hydro_subview_preserves_non_stretch_button_intrinsic_under_zero_width_proposal() {
    let env = test_environment();
    let view = normalize_layout_view(AnyView::new(button("Medium (0.7)").action(|| {})), &env);
    let mut state = HydroState::default();
    let state = RefCell::new(&mut state);
    let subview = HydroSubview::from_view(&view, &state, &env);

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
    let expanded = Binding::bool(false);
    let view = normalize_layout_view(
        AnyView::new(
            button("Toggle Bars")
                .action(|waterui::State(value): waterui::State<Binding<bool>>| {
                    value.set(!value.get());
                })
                .state(&expanded),
        ),
        &env,
    );
    let mut state = HydroState::default();
    let state = RefCell::new(&mut state);
    let subview = HydroSubview::from_view(&view, &state, &env);

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
                value.set(!value.get());
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
        HydrolysisRenderer::new(surface.device())
    };
    let env = test_environment();
    let bounds = vello::kurbo::Rect::new(0.0, 0.0, 160.0, 160.0);
    let surface = platform.surface();
    renderer.set_frame_resources(surface.adapter(), surface.device(), surface.queue());
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
    let mut state = HydroState::default();
    let proposal = ProposalSize::UNSPECIFIED;

    let raw = measure_view_dimensions_with_proposal(
        &AnyView::new(Str::from("Hydrolysis")),
        proposal,
        &mut state,
        &env,
    );
    let borrowed = measure_view_dimensions_with_proposal(
        &AnyView::new("Hydrolysis"),
        proposal,
        &mut state,
        &env,
    );
    let owned = measure_view_dimensions_with_proposal(
        &AnyView::new(String::from("Hydrolysis")),
        proposal,
        &mut state,
        &env,
    );
    let cow = measure_view_dimensions_with_proposal(
        &AnyView::new(Cow::Borrowed("Hydrolysis")),
        proposal,
        &mut state,
        &env,
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
    renderer.set_window_bounds(bounds);
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
    renderer.hit_test.interaction.begin_press(
        &slot,
        Point::new(20.0, 20.0),
        renderer.frame_instant(),
    );
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
    renderer.hit_test.interaction.begin_press(
        &slot,
        Point::new(20.0, 20.0),
        renderer.frame_instant(),
    );
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
    let (state, _, _) =
        renderer.bind_focused_interaction_target(key, Rect::new(0.0, 0.0, 80.0, 80.0), &env, true);

    assert!(state.focus_visible);
    assert_eq!(state.focus_progress, 1.0);
}

#[derive(Default)]
struct NoopDrawContext;

impl DrawContext for NoopDrawContext {
    fn fill_rect(&mut self, _rect: Rect, _brush: &Brush) {}
    fn fill_rounded_rect(&mut self, _rect: Rect, _radii: RoundedRectRadii, _brush: &Brush) {}
    fn stroke_rect(&mut self, _rect: Rect, _brush: &Brush, _width: f64) {}
    fn stroke_rounded_rect(
        &mut self,
        _rect: Rect,
        _radii: RoundedRectRadii,
        _brush: &Brush,
        _width: f64,
    ) {
    }
    fn stroke_line(&mut self, _from: Point, _to: Point, _brush: &Brush, _width: f64) {}
    fn stroke_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush, _width: f64) {}
    fn fill_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush) {}
    fn fill_path(&mut self, _path: &BezPath, _brush: &Brush) {}
    fn stroke_path(&mut self, _path: &BezPath, _brush: &Brush, _width: f64) {}
    fn push_layer(&mut self, _alpha: f32, _clip: Option<&Rect>) {}
    fn pop_layer(&mut self) {}
    fn push_transform(&mut self, _affine: Affine) {}
    fn pop_transform(&mut self) {}
}

struct MinimalTestTheme;

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
            transition_duration: Duration::from_millis(250),
            pushpop_parallax_factor: 0.35,
        }
    }

    fn button_metrics(&self, _style: ButtonStyle) -> ButtonMetrics {
        ButtonMetrics {
            padding_x: 1.0,
            padding_y: 2.0,
            min_width: 123.0,
            min_height: 45.0,
        }
    }

    fn draw_button_chrome(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _style: ButtonStyle,
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

    fn draw_stepper_button(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
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

    fn draw_text_context_menu_panel(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

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
            thumb_radius: 9.0,
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
            ProgressIndicatorStyle::Linear => ProgressMetrics {
                label_height: 18.0,
                bar_top_offset: 10.0,
                bar_height: 8.0,
                bar_horizontal_inset: 8.0,
                value_label_top_spacing: 6.0,
                min_track_width: 72.0,
                circular_diameter: 0.0,
                circular_stroke_width: 0.0,
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
            },
        }
    }

    fn draw_progress_linear_track(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
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
    ) {
    }
    fn draw_progress_circular_fill(
        &self,
        _draw: &mut dyn DrawContext,
        _path: &BezPath,
        _width: f64,
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
            large_bar_height: 152.0,
            inline_title_height: 28.0,
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
            small_offset_y: 4.0,
            large_offset_x: 2.0,
            large_offset_y: 1.0,
        }
    }

    fn badge_label_color(&self) -> Color {
        Color::srgb(255, 255, 255)
    }

    fn badge_label_font(&self) -> waterui_text::font::Font {
        waterui_text::font::Font::default()
    }

    fn draw_badge_small(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_badge_large(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

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
fn widget_theme_can_be_replaced_in_environment() {
    let env = test_environment();

    let metrics = widget_theme(&env).button_metrics(ButtonStyle::Plain);
    assert_eq!(metrics.min_width, 123.0);
    assert_eq!(metrics.min_height, 45.0);

    let mut draw = NoopDrawContext;
    widget_theme(&env).draw_button_chrome(
        &mut draw,
        Rect::new(0.0, 0.0, 10.0, 10.0),
        ButtonStyle::Plain,
        WidgetInteractionState::NONE,
    );
}

#[test]
fn ime_preedit_commit_and_disable_update_focused_text_target() {
    let mut renderer = test_renderer();
    renderer.set_text_caret_motion(MinimalTestTheme.text_caret_motion());
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
    assert!(renderer.handle_ime_preedit("拼音"));
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

    assert!(renderer.handle_ime_preedit("候选"));
    assert!(renderer.handle_ime_disabled());
    assert_eq!(renderer.text_editing.ime_preedit, None);
    assert_eq!(
        renderer.text_editing.text_input_targets[0]
            .model
            .plain_text(),
        "中"
    );
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

#[test]
fn secure_text_context_menu_excludes_copy_and_cut() {
    let selection = Rc::new(RefCell::new(TextSelectionSlot {
        anchor: 0,
        focus: 3,
        initialized: true,
    }));
    let target = text_input_target(secure_field_model("abc"), selection);

    let entries = HydrolysisRenderer::build_text_context_menu_entries(&target);
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
    renderer.set_window_bounds(Rect::new(0.0, 0.0, 160.0, 160.0));
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
    renderer.set_window_bounds(Rect::new(0.0, 0.0, 160.0, 160.0));
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
fn text_leaves_measure_off_thread_and_match_main_thread() {
    use core::cell::RefCell;

    let env = test_environment();
    let proposal = ProposalSize::UNSPECIFIED;

    // Text is the only leaf whose measurement is heavy enough to offload: it
    // resolves to a `Send` layout input on the main thread and shapes on any
    // thread. Covers a bare `Str` and a string-like (`String`) that is bodied
    // into text and recursed.
    let str_view = AnyView::new(Str::from("probe"));
    let string_view = AnyView::new(String::from("hello world"));
    let text_views: [&AnyView; 2] = [&str_view, &string_view];

    for view in text_views {
        let mut state = HydroState::default();
        let state_cell = RefCell::new(&mut state);
        let subview = HydroSubview::from_view(view, &state_cell, &env);
        assert!(
            !subview.require_main_thread(),
            "text leaf {} must be measurable off the main thread",
            view.name()
        );

        // Measuring on a worker thread must touch no `MainThreadBound` state
        // (which would panic) and must reproduce the main-thread measurement.
        let off_thread = std::thread::scope(|scope| {
            scope
                .spawn(|| subview.measure(proposal))
                .join()
                .expect("worker measurement thread panicked")
        });
        let main_thread = {
            let mut state = state_cell.borrow_mut();
            measure_view_dimensions_with_proposal(view, proposal, &mut state, &env)
        };
        assert_eq!(
            off_thread.size,
            main_thread.size,
            "off-thread measure of {} must equal the main-thread measure",
            view.name()
        );
        assert!(
            off_thread.size.width > 0.0 && off_thread.size.height > 0.0,
            "text shaping must produce a non-zero size for {}",
            view.name()
        );
    }

    // Non-text leaves measure in O(1); they are not offloaded and stay
    // main-thread-bound, as does any container that recurses into its children.
    {
        let unit_view = AnyView::new(());
        let container_view = AnyView::new(vstack((Str::from("a"), Str::from("b"))));
        let mut state = HydroState::default();
        let state_cell = RefCell::new(&mut state);
        assert!(
            HydroSubview::from_view(&unit_view, &state_cell, &env).require_main_thread(),
            "a non-text leaf must measure on the main thread"
        );
        assert!(
            HydroSubview::from_view(&container_view, &state_cell, &env).require_main_thread(),
            "a container that recurses into children must measure on the main thread"
        );
    }
}

#[test]
fn bare_str_renders_into_scene() {
    let mut renderer = test_renderer();
    let env = test_environment();

    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);
    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(bounds);
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
