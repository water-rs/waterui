use super::*;
use std::borrow::Cow;

mod retained_scene;
use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
use waterui::gesture::{DragGesture, GestureObserver, MagnificationGesture};
use waterui::shape::{Circle, RoundedRectangle, ShapeExt};
use waterui::{Binding, Color, SignalExt as _, ViewExt as _};
use waterui_canvas::Canvas;
use waterui_controls::button::{ButtonStyle, button};
use waterui_controls::slider::slider;
use waterui_controls::toggle::ToggleStyle;
use waterui_core::dynamic::{Dynamic, DynamicInitialContent};
use waterui_core::id::SelfId;
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
    renderer.set_frame_resources(surface.device(), surface.queue());
    renderer
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
    let env = Environment::default();
    let child = normalize_layout_view(
        AnyView::new(().size(20.0, 10.0).horizontal_alignment_guide(
            HorizontalAlignment::Leading,
            |dimensions: &ViewDimensions| dimensions.size.width * 0.5,
        )),
        &env,
    );
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 0.0,
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
    let env = Environment::default();
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
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        native_executor::NativeExecutor::new(),
    ));
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
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
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        native_executor::NativeExecutor::new(),
    ));
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
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
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        native_executor::NativeExecutor::new(),
    ));
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
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
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        native_executor::NativeExecutor::new(),
    ));
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
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

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.dispatch(view, &env, bounds);
    renderer.finish_rebuild_frame();

    let target = renderer
        .hit_test
        .pointer_targets
        .iter()
        .next()
        .expect("state-wrapped button should register a pointer target");
    assert!(
        target.bounds.width() < 200.0,
        "state-wrapped button hit bounds must stay intrinsic, got width {}",
        target.bounds.width()
    );
}

#[test]
fn render_layout_container_places_non_stretch_layout_at_intrinsic_size() {
    #[derive(Debug)]
    struct RecordingLayout {
        placed_bounds: Rc<RefCell<Option<LayoutRect>>>,
    }

    impl Layout for RecordingLayout {
        fn size_that_fits(
            &self,
            _proposal: ProposalSize,
            _children: &[&dyn SubView],
        ) -> LayoutSize {
            LayoutSize::new(80.0, 40.0)
        }

        fn place(&self, bounds: LayoutRect, _children: &[&dyn SubView]) -> Vec<LayoutRect> {
            *self.placed_bounds.borrow_mut() = Some(bounds);
            Vec::new()
        }
    }

    let placed_bounds = Rc::new(RefCell::new(None));
    let layout = RecordingLayout {
        placed_bounds: Rc::clone(&placed_bounds),
    };
    let env = Environment::new();
    let mut renderer = test_renderer();
    let ctx = RenderContext::with_transforms(
        Rect::new(0.0, 0.0, 300.0, 300.0),
        Affine::IDENTITY,
        Affine::IDENTITY,
    );

    HydrolysisRenderer::render_layout_container(
        &mut renderer,
        ctx,
        Box::new(layout),
        Vec::new(),
        &env,
    );

    assert_eq!(
        placed_bounds
            .borrow()
            .as_ref()
            .map(LayoutRect::size)
            .copied(),
        Some(LayoutSize::new(80.0, 40.0))
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
    let env = Environment::new();
    let bounds = Rect::new(0.0, 0.0, 160.0, 80.0);

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.dispatch(view, &env, bounds);
    renderer.finish_rebuild_frame();

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
    let env = Environment::new();
    let bounds = vello::kurbo::Rect::new(0.0, 0.0, 160.0, 160.0);
    let surface = platform.surface();
    renderer.set_frame_resources(surface.device(), surface.queue());
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.dispatch(view, &env, bounds);
    renderer.finish_rebuild_frame();

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
    let env = Environment::default();
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
fn dynamic_initial_content_builds_real_subtree_before_second_rebuild() {
    let env = Environment::new();
    let (handler, dynamic) = Dynamic::new();
    handler.set("Hydrolysis dynamic");
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.dispatch(dynamic, &env, bounds);
    renderer.finish_rebuild_frame();

    let node = renderer
        .lifecycle
        .dynamic_nodes
        .values()
        .next()
        .expect("initial Dynamic render must register a lifecycle node");
    let subtree = node
        .cached_subtree
        .as_ref()
        .expect("initial Dynamic render must cache a subtree");

    #[cfg(feature = "accessibility")]
    assert!(
        !subtree.accessibility.root_children.is_empty(),
        "initial Dynamic render must cache the real content subtree, not an empty placeholder"
    );
    assert!(
        !renderer.take_rebuild_request(),
        "initial Dynamic content must not force a rebuild before a previous layout exists"
    );
}

#[test]
fn dynamic_body_snapshot_after_render_does_not_schedule_rebuild() {
    let env = Environment::new();
    let (handler, dynamic) = Dynamic::new();
    handler.set_with_metadata(
        "Hydrolysis dynamic",
        nami::watcher::Metadata::new().with(DynamicInitialContent),
    );
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.dispatch(dynamic, &env, bounds);

    handler.set_with_metadata(
        "Hydrolysis dynamic snapshot",
        nami::watcher::Metadata::new().with(DynamicInitialContent),
    );
    assert!(
        !renderer.take_rebuild_request(),
        "body-time Dynamic snapshots after the node rendered must not schedule rebuild loops"
    );
    let pending_is_empty = renderer
        .lifecycle
        .dynamic_nodes
        .values()
        .next()
        .expect("rendered Dynamic must register a lifecycle node")
        .pending_view
        .borrow()
        .is_none();
    assert!(
        pending_is_empty,
        "body-time Dynamic snapshots after render must not leave stale pending content"
    );

    handler.set("Hydrolysis dynamic update");
    assert!(
        renderer.take_patch_request(),
        "real Dynamic updates after render must schedule a fine-grained reactive patch"
    );
    assert!(
        !renderer.take_rebuild_request(),
        "an isolated Dynamic content change must not force a full structural rebuild"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn fixed_scroll_content_keeps_offscreen_children_registered() {
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
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
    renderer.capture_window_scene(view, env, bounds, Affine::IDENTITY, Affine::IDENTITY);
    renderer.finish_rebuild_frame();
}

fn root_scroll_cache_key(renderer: &HydrolysisRenderer) -> usize {
    renderer
        .retained_window_frame
        .as_ref()
        .expect("root scroll must retain a window frame")
        .subtree
        .scroll_draws()
        .next()
        .expect("root scroll must record a scroll draw")
        .cache_key
}

fn root_scroll_handle(renderer: &HydrolysisRenderer) -> crate::scroll::ScrollHandle {
    renderer
        .retained_window_frame
        .as_ref()
        .expect("root scroll must retain a window frame")
        .subtree
        .scroll_draws()
        .next()
        .expect("root scroll must record a scroll draw")
        .handle
        .clone()
}

#[test]
fn fixed_scroll_content_refreshes_window_frame_after_offset_change() {
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let view = scroll(vstack((
        ().size(120.0, 600.0),
        button("Offscreen").action(|| {}),
    )));
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    capture_root_window(&mut renderer, view, &env, bounds);

    let handle = root_scroll_handle(&renderer);
    assert!(handle.apply_scroll_delta(0.0, -48.0, false));

    assert!(
        renderer.window_scroll_draws_reusable(),
        "fixed scroll content is viewport-independent and must re-composite by translation"
    );
    assert!(
        renderer.refresh_window_frame(&env),
        "the window frame must refresh by translating retained scroll content"
    );
}

#[test]
fn scroll_content_cache_replay_preserves_dynamic_morphs() {
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    let make_view = || {
        scroll(vstack((
            Circle
                .morph_to(RoundedRectangle::new(0.22), Color::srgb_hex("#3B82F6"))
                .duration(Duration::from_millis(1_100))
                .size(60.0, 60.0),
            ().size(120.0, 600.0),
        )))
    };

    capture_root_window(&mut renderer, make_view(), &env, bounds);

    let cache_key = root_scroll_cache_key(&renderer);
    assert!(
        !renderer
            .scroll_content_caches
            .get(&cache_key)
            .expect("scroll content cache must be stored")
            .dynamic_morphs
            .is_empty(),
        "scroll content cache must retain renderer-side dynamic morph draws"
    );

    renderer.set_scroll_content_cache_reuse(true);
    capture_root_window(&mut renderer, make_view(), &env, bounds);

    assert!(
        !renderer
            .retained_window_frame
            .as_ref()
            .expect("cache replay must retain a window frame")
            .subtree
            .scroll_draws()
            .next()
            .expect("cache replay must record a scroll draw")
            .content_morphs
            .is_empty(),
        "scroll draw must carry cached dynamic morphs for replay"
    );
}

#[test]
fn animated_transform_scroll_content_cache_replays_while_animation_is_active() {
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let scale = Binding::f32(1.0);
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    let make_view = || {
        let animated_scale = scale
            .clone()
            .with(Animation::linear(Duration::from_millis(1_000)));
        scroll(vstack((
            ().size(60.0, 60.0)
                .scale(animated_scale.clone(), animated_scale),
            ().size(120.0, 600.0),
        )))
    };

    capture_root_window(&mut renderer, make_view(), &env, bounds);

    let cache_key = root_scroll_cache_key(&renderer);
    let lazy_viewport = {
        let cache = renderer
            .scroll_content_caches
            .get(&cache_key)
            .expect("scroll content cache must be stored");
        assert!(
            !cache.animation_dependent,
            "captured dynamic transform metadata must not poison scroll content cache reuse"
        );
        assert!(
            cache.subtree.transform_draws().next().is_some(),
            "animated transform metadata must be captured as replayable dynamic transform draw"
        );
        cache.lazy_viewport
    };

    scale.set(2.0);
    assert!(
        renderer.animations_active(),
        "changing an animated transform signal must leave the animation controller active"
    );
    assert!(
        renderer.retained_window_can_drive_active_animations(),
        "the window frame must redraw captured transform animations in scroll content without dispatch"
    );
    renderer.animation_controller.bind_scalar_target(
        AnimationKey::renderer_local_scalar(usize::MAX - 1),
        1.0,
        Animation::linear(Duration::from_millis(250)),
        renderer.frame_instant(),
    );
    assert!(
        renderer.retained_window_can_drive_active_animations(),
        "renderer-local interaction animations must not force captured transform animations back through scene dispatch"
    );
    let cache = renderer
        .scroll_content_caches
        .get(&cache_key)
        .expect("scroll content cache must remain stored");
    assert!(
        renderer.can_reuse_scroll_content_cache(cache, lazy_viewport),
        "active transform animation should reuse cached scroll content through dynamic transform replay"
    );
}

#[test]
fn animated_opacity_scroll_content_cache_replays_while_animation_is_active() {
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let opacity = Binding::f32(1.0);
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    let make_view = || {
        let animated_opacity = opacity
            .clone()
            .with(Animation::linear(Duration::from_millis(1_000)));
        scroll(vstack((
            ().size(60.0, 60.0).opacity(animated_opacity),
            ().size(120.0, 600.0),
        )))
    };

    capture_root_window(&mut renderer, make_view(), &env, bounds);

    let cache_key = root_scroll_cache_key(&renderer);
    let lazy_viewport = {
        let cache = renderer
            .scroll_content_caches
            .get(&cache_key)
            .expect("scroll content cache must be stored");
        assert!(
            !cache.animation_dependent,
            "animated opacity is captured as a replayable dynamic opacity layer and must not poison cache reuse"
        );
        assert!(
            cache.subtree.opacity_draws().next().is_some(),
            "animated opacity must be captured as a replayable dynamic opacity draw"
        );
        cache.lazy_viewport
    };

    opacity.set(0.35);
    assert!(
        renderer.animations_active(),
        "changing animated opacity must leave the animation controller active"
    );
    assert!(
        renderer.retained_window_can_drive_active_animations(),
        "the window frame must redraw captured opacity animations in scroll content without dispatch"
    );
    let cache = renderer
        .scroll_content_caches
        .get(&cache_key)
        .expect("scroll content cache must remain stored");
    assert!(
        renderer.can_reuse_scroll_content_cache(cache, lazy_viewport),
        "active opacity animation should reuse cached scroll content through dynamic opacity replay"
    );
}

#[test]
fn lazy_scroll_content_rejects_window_refresh_after_offset_change() {
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let rows = (0..40).map(SelfId::new).collect::<Vec<_>>();
    let view = scroll(waterui_layout::stack::VStack::for_each(rows, |_| {
        ().size(120.0, 40.0)
    }));
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    capture_root_window(&mut renderer, view, &env, bounds);

    let cache_key = root_scroll_cache_key(&renderer);
    assert!(
        renderer
            .scroll_content_caches
            .get(&cache_key)
            .expect("lazy scroll content cache must be stored")
            .viewport_dependent
    );
    let handle = root_scroll_handle(&renderer);
    assert!(handle.apply_scroll_delta(0.0, -48.0, false));

    assert!(
        !renderer.window_scroll_draws_reusable(),
        "lazy scroll content depends on viewport materialization and must not reuse a stale retained window frame"
    );
}

#[test]
fn fixed_vstack_inside_scroll_does_not_use_lazy_stack_slots() {
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
    let mut renderer = test_renderer();
    let view = scroll(vstack((
        waterui_controls::button::button("Short"),
        waterui_controls::button::button("A much longer button label"),
        waterui_controls::button::button("Short again"),
    )));
    let bounds = Rect::new(0.0, 0.0, 160.0, 160.0);

    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.dispatch(view, &env, bounds);
    renderer.finish_rebuild_frame();

    assert!(
        renderer.lazy.lazy_stack_controller.slots.is_empty(),
        "FixedContainer layout must not be virtualized by Hydrolysis lazy stack slots"
    );
}

#[test]
fn interaction_press_origin_is_converted_to_widget_local_space() {
    let state = WidgetInteractionState {
        press_origin: Some(Point::new(125.0, 84.0)),
        press_progress: 0.5,
        press_layer_opacity: 0.12,
        ..WidgetInteractionState::NONE
    };

    let local = crate::renderer::local_interaction_state(state, Affine::translate((100.0, 80.0)));

    assert_eq!(local.press_origin, Some(Point::new(25.0, 4.0)));
    assert_eq!(local.press_progress, state.press_progress);
    assert_eq!(local.press_layer_opacity, state.press_layer_opacity);
}

#[test]
fn interaction_press_slot_does_not_migrate_to_unrelated_bounds() {
    let mut renderer = test_renderer();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);

    renderer.begin_rebuild_frame();
    let (_, slot, _) = renderer.bind_interaction_target(Rect::new(0.0, 0.0, 80.0, 80.0), &env);
    renderer.hit_test.interaction.begin_press(
        slot,
        Point::new(20.0, 20.0),
        renderer.frame_instant(),
    );
    renderer.finish_rebuild_frame();

    renderer.begin_rebuild_frame();
    let (state, _, _) =
        renderer.bind_interaction_target(Rect::new(100.0, 100.0, 180.0, 180.0), &env);

    assert!(!state.pressed);
    assert_eq!(state.press_origin, None);
}

#[test]
fn interaction_engine_resolves_focus_state() {
    let mut renderer = test_renderer();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);

    renderer.begin_rebuild_frame();
    let (state, _, _) =
        renderer.bind_focused_interaction_target(Rect::new(0.0, 0.0, 80.0, 80.0), &env, true);

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

    fn draw_button_chrome(&self, _draw: &mut dyn DrawContext, _bounds: Rect, _style: ButtonStyle) {}

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
        _state: WidgetInteractionState,
    ) {
    }

    fn draw_toggle_checkbox(&self, _draw: &mut dyn DrawContext, _bounds: Rect, _progress: f32) {}

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

    fn draw_slider_track(&self, _draw: &mut dyn DrawContext, _track_rect: Rect, _fill_rect: Rect) {}

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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);

    let metrics = widget_theme(&env).button_metrics(ButtonStyle::Plain);
    assert_eq!(metrics.min_width, 123.0);
    assert_eq!(metrics.min_height, 45.0);

    let mut draw = NoopDrawContext;
    widget_theme(&env).draw_button_chrome(
        &mut draw,
        Rect::new(0.0, 0.0, 10.0, 10.0),
        ButtonStyle::Plain,
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
        renderer.take_rebuild_request(),
        "text input focus changes must rebuild immediately so focus animations start on click"
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);

    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(Rect::new(0.0, 0.0, 160.0, 160.0));
    renderer.capture_window_scene(
        waterui_text::text("probe"),
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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);

    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(Rect::new(0.0, 0.0, 160.0, 160.0));
    renderer.capture_window_scene(
        Str::from("probe"),
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
    let env = Environment::default();
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
fn bare_str_direct_dispatch_renders_into_scene() {
    let mut renderer = test_renderer();
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);

    renderer.begin_rebuild_frame();
    renderer.set_window_bounds(Rect::new(0.0, 0.0, 160.0, 160.0));
    renderer.dispatch(
        Str::from("probe"),
        &env,
        Rect::new(0.0, 0.0, 160.0, 160.0),
    );
    assert!(
        !renderer.scene_is_empty(),
        "directly dispatched string must draw glyphs"
    );
    renderer.finish_rebuild_frame();
}

#[test]
fn render_path_text_layout_has_lines() {
    let env = Environment::default();
    let mut state = HydroState::default();
    let layout = HydrolysisRenderer::build_text_layout(
        &mut state,
        waterui_text::styled::StyledStr::plain("probe"),
        HorizontalAlignment::Leading,
        &env,
        Some(160.0),
    );
    assert!(!layout.is_empty(), "render-path text layout must not be empty");
    assert!(layout.lines().next().is_some(), "layout must have lines");
}
