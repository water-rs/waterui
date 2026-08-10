use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use crate::driver::{A11yDriver, DriverPumpResult};
use accesskit::{ActionRequest as AccessibilityActionRequest, NodeId as AccessibilityNodeId};
use hydrolysis::{HydrolysisRenderer, OffscreenWindow, PlatformWindow};
use hydrolysis_m3::install as install_m3;
use vello::kurbo::Shape;
use waterui::Computed;
use waterui::View as _;
use waterui::ViewExt as _;
use waterui::color::ResolvedColor;
use waterui::component::{text, vstack};
use waterui::graphics::SceneViewMergeToParent;
use waterui::graphics::color::Srgb;
use waterui::graphics::{Scene2D, SceneContent, SceneView};
use waterui::theme;
use waterui_canvas::Canvas;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::{Point, Rect, Size};
use waterui_core::{AnyView, Environment, Native};

use crate::snapshot::readback_texture_rgba8;

#[derive(Debug)]
struct NoopDriver;

impl A11yDriver for NoopDriver {
    fn pump(
        &mut self,
        _content: &AnyViewBuilder<AnyView>,
        _env: &Environment,
        _capture_snapshot: bool,
    ) -> DriverPumpResult {
        DriverPumpResult {
            rebuilt: false,
            tree_update: None,
            snapshot: None,
            ui_focus: None,
        }
    }

    fn perform_action(&mut self, _request: AccessibilityActionRequest, _env: &Environment) -> bool {
        false
    }

    fn hover_at(&mut self, _x: f32, _y: f32, _env: &Environment) -> bool {
        false
    }

    fn pointer_down(&mut self, _x: f32, _y: f32, _env: &Environment) -> bool {
        false
    }

    fn pointer_move(&mut self, _x: f32, _y: f32, _env: &Environment) -> bool {
        false
    }

    fn pointer_up(&mut self, _x: f32, _y: f32, _env: &Environment) -> bool {
        false
    }

    fn scroll_at(
        &mut self,
        _x: f32,
        _y: f32,
        _dx: f32,
        _dy: f32,
        _is_line_delta: bool,
        _env: &Environment,
    ) -> bool {
        false
    }

    fn text_input(&mut self, _text: String, _env: &Environment) -> bool {
        false
    }

    fn key_press(
        &mut self,
        _key: hydrolysis::KeyCode,
        _modifiers: hydrolysis::Modifiers,
        _env: &Environment,
    ) -> bool {
        false
    }

    fn magnify_at(&mut self, _x: f32, _y: f32, _factor: f32, _env: &Environment) -> bool {
        false
    }

    fn clear_ui_focus(&mut self, _env: &Environment) -> bool {
        false
    }

    fn request_redraw(&mut self, _content: &AnyViewBuilder<AnyView>, _env: &Environment) -> bool {
        false
    }

    fn pump_frame(
        &mut self,
        _content: &AnyViewBuilder<AnyView>,
        _env: &Environment,
    ) -> crate::driver::FrameTiming {
        crate::driver::FrameTiming::default()
    }
}

fn node_id(raw: u64) -> NodeId {
    NodeId::from(AccessibilityNodeId(raw))
}

fn node(
    id: u64,
    role: Role,
    label: Option<&str>,
    value: Option<&str>,
    enabled: bool,
) -> NodeSnapshot {
    NodeSnapshot {
        id: node_id(id),
        role,
        label: label.map(ToOwned::to_owned),
        value: value.map(ToOwned::to_owned),
        bounds: None,
        enabled,
        selected: false,
        checked: None,
        expanded: None,
        hidden: false,
        children: Vec::new(),
    }
}

fn tree(nodes: Vec<NodeSnapshot>) -> TreeSnapshot {
    let Some(root) = nodes.first().map(NodeSnapshot::id) else {
        panic!("test tree helper requires at least one node");
    };
    let nodes = nodes.into_iter().map(|node| (node.id(), node)).collect();
    TreeSnapshot {
        revision: 1,
        root,
        focus: root,
        nodes,
    }
}

fn scoped_tree() -> TreeSnapshot {
    let mut root = node(1, Role::LIST, Some("root"), None, true);
    root.children = vec![node_id(2), node_id(3)];

    let mut alpha = node(2, Role::LIST_ITEM, Some("Alpha card"), None, true);
    alpha.children = vec![node_id(4), node_id(5)];

    let mut beta = node(3, Role::LIST_ITEM, Some("Beta card"), None, true);
    beta.children = vec![node_id(6), node_id(7)];

    let edit_alpha = node(4, Role::BUTTON, Some("Edit"), None, true);
    let email_alpha = node(
        5,
        Role::TEXT_INPUT,
        Some("Email"),
        Some("alpha@example.com"),
        true,
    );
    let edit_beta = node(6, Role::BUTTON, Some("Edit"), None, true);
    let email_beta = node(
        7,
        Role::TEXT_INPUT,
        Some("Email"),
        Some("beta@example.com"),
        true,
    );

    tree(vec![
        root,
        alpha,
        beta,
        edit_alpha,
        email_alpha,
        edit_beta,
        email_beta,
    ])
}

fn mounted(tree: TreeSnapshot) -> SemanticApp {
    SemanticApp {
        env: Environment::new(),
        content: AnyViewBuilder::new(|| AnyView::new(())),
        driver: Box::new(NoopDriver),
        tree,
        ui_focus: None,
        revision: 2,
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    String::from("<non-string panic>")
}

fn themed_environment() -> Environment {
    let mut env = Environment::new();
    install_m3(&mut env);
    env
}

#[test]
fn smoke_snapshot_size_matches_target() {
    let host = TestHost::new(Environment::new(), 64, 48);
    let snapshot = host.render(());
    assert_eq!(snapshot.width, 64);
    assert_eq!(snapshot.height, 48);
    assert_eq!(snapshot.rgba8.len(), 64 * 48 * 4);
}

#[test]
fn smoke_theme_foreground_slot_snapshot_preserves_semantic_labels() {
    let mut env = themed_environment();
    theme::install_color_signal::<theme::color::Foreground>(
        &mut env,
        Computed::constant(ResolvedColor {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            opacity: 1.0,
            headroom: 1.0,
        }),
    );
    let mut app = ui()
        .environment(env)
        .viewport(240, 120)
        .theme(|_: &mut Environment| {})
        .mount(|| {
            vstack((text("Theme slot").body(), text("Theme slot").body())).background(Srgb::BLACK)
        });
    assert_eq!(
        app.query()
            .role(Role::LABEL)
            .label("Theme slot")
            .all()
            .len(),
        2,
        "theme slot text should stay queryable under custom theme environment"
    );
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 240);
    assert_eq!(snapshot.height, 120);
    assert_eq!(snapshot.rgba8.len(), 240 * 120 * 4);
}

#[test]
fn semantic_builder_does_not_require_theme_package() {
    let mut app = ui()
        .viewport(180, 80)
        .mount(|| text("Semantic only").body());
    let _ = app
        .query()
        .role(Role::LABEL)
        .label("Semantic only")
        .single();
}

#[test]
fn themed_builder_exposes_offscreen_perf_closure_api() {
    let report = ui()
        .viewport(96, 72)
        .theme(install_m3)
        .perf_config(PerfConfig {
            warmups: 1,
            samples: 3,
            repetitions: 1,
        })
        .perf_with(
            || text("Measured").body(),
            |perf| {
                perf.measure("steady", |run| {
                    let _ = run
                        .app()
                        .query()
                        .role(Role::LABEL)
                        .label("Measured")
                        .single();
                });
            },
        );

    let measurements = report.measurements();
    assert_eq!(measurements.len(), 1);
    assert_eq!(measurements[0].name, "steady");
    let stats = measurements[0].stats();
    assert_eq!(stats.samples, 3);
}

#[test]
fn themed_builder_default_perf_requests_redraw() {
    let report = ui()
        .viewport(96, 72)
        .theme(install_m3)
        .perf_config(PerfConfig {
            warmups: 1,
            samples: 3,
            repetitions: 1,
        })
        .perf(|| text("Redraw measured").body());

    let measurements = report.measurements();
    assert_eq!(measurements.len(), 1);
    assert_eq!(measurements[0].name, "steady-redraw");
    let stats = measurements[0].stats();
    assert_eq!(stats.samples, 3);
    assert_eq!(stats.rebuilt_frames, 0);
    assert!(
        stats.phases.render.p95 > Duration::ZERO,
        "default perf should measure real redraw frames"
    );
}

#[test]
fn ui_test_environment_builder_preserves_custom_theme() {
    let mut env = themed_environment();
    theme::install_color_signal::<theme::color::Foreground>(
        &mut env,
        Computed::constant(ResolvedColor {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            opacity: 1.0,
            headroom: 1.0,
        }),
    );
    let mut app = ui()
        .environment(env)
        .viewport(240, 120)
        .theme(|_: &mut Environment| {})
        .mount(|| {
            vstack((text("Mounted theme").body(), text("Mounted theme").body()))
                .background(Srgb::BLACK)
        });
    assert_eq!(
        app.query()
            .role(Role::LABEL)
            .label("Mounted theme")
            .all()
            .len(),
        2,
        "UiBuilder environment builder should keep mounted text semantics intact"
    );
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 240);
    assert_eq!(snapshot.height, 120);
    assert_eq!(snapshot.rgba8.len(), 240 * 120 * 4);
}

#[test]
fn smoke_text_color_snapshot_preserves_semantic_labels() {
    let mut app = ui().viewport(240, 120).theme(install_m3).mount(|| {
        vstack((
            text("Explicit color").body().color(Srgb::WHITE),
            text("Explicit color").body().color(Srgb::WHITE),
        ))
        .background(Srgb::BLACK)
    });
    assert_eq!(
        app.query()
            .role(Role::LABEL)
            .label("Explicit color")
            .all()
            .len(),
        2,
        "explicit text color should not break semantic text exposure"
    );
}

#[test]
fn smoke_text_snapshot_preserves_semantic_labels() {
    let mut app = ui().viewport(240, 120).mount(|| {
        vstack((
            text("Focused datum").body().foreground(Srgb::WHITE),
            text("Selected datum").body().foreground(Srgb::WHITE),
        ))
        .background(Srgb::BLACK)
    });
    app.query()
        .role(Role::LABEL)
        .label("Focused datum")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Selected datum")
        .assert_exists();
}

#[test]
fn tappable_composed_view_exposes_clickable_accessibility_node() {
    let tapped = Rc::new(Cell::new(false));
    let tapped_for_view = Rc::clone(&tapped);
    let mut app = ui().viewport(160, 96).mount(move || {
        text("Assist")
            .body()
            .padding_with(6.0)
            .on_tap({
                let tapped_for_view = Rc::clone(&tapped_for_view);
                move || tapped_for_view.set(true)
            })
            .a11y_label("Assist")
            .a11y_role(waterui::accessibility::AccessibilityRole::Button)
            .a11y_children(waterui::accessibility::AccessibilityChildren::ExcludeDescendants)
    });

    app.query()
        .role(Role::BUTTON)
        .label("Assist")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Assist").tap(),
        "tap gesture accessibility node should route click actions through Hydrolysis gestures"
    );
    assert!(
        tapped.get(),
        "accessibility click should trigger tap gesture"
    );
    app.query()
        .role(Role::LABEL)
        .label("Assist")
        .assert_not_exists();
}

#[test]
fn ui_test_snapshot_renders_text_after_canvas() {
    let mut app = ui().viewport(320, 320).theme(install_m3).mount(|| {
        vstack((
            Canvas::new(|ctx| {
                ctx.set_fill_style(Srgb::new(0.0, 0.85, 0.65));
                ctx.fill_rect(Rect::new(Point::new(0.0, 0.0), Size::new(240.0, 180.0)));
            })
            .size(240.0, 180.0)
            .a11y_role(waterui::accessibility::AccessibilityRole::Image)
            .a11y_label("Canvas layer"),
            text("W")
                .size(48.0)
                .color(Srgb::WHITE)
                .body()
                .padding_with(6.0)
                .a11y_label("Letter W"),
        ))
        .spacing(6.0)
        .background(Srgb::BLACK)
    });
    app.query()
        .role(Role::IMAGE)
        .label("Canvas layer")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Letter W")
        .assert_exists();
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 320);
    assert_eq!(snapshot.height, 320);
}

#[test]
fn smoke_canvas_snapshot_preserves_accessibility_metadata() {
    let mut app = ui().viewport(96, 72).theme(install_m3).mount(|| {
        Canvas::new(|ctx| {
            ctx.set_fill_style(Srgb::new(1.0, 0.0, 0.0));
            ctx.fill_rect(Rect::new(Point::new(8.0, 8.0), Size::new(40.0, 24.0)));
        })
        .a11y_role(waterui::accessibility::AccessibilityRole::Image)
        .a11y_label("Canvas smoke")
    });
    app.query()
        .role(Role::IMAGE)
        .label("Canvas smoke")
        .assert_exists();
    let snapshot = app.snapshot();
    assert_eq!(snapshot.width, 96);
    assert_eq!(snapshot.height, 72);
}

struct TestSceneContent(Rc<Cell<bool>>);

impl SceneContent for TestSceneContent {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        self.0.set(true);
        let rect = vello::kurbo::Rect::from_origin_size(
            vello::kurbo::Point::new(8.0, 8.0),
            vello::kurbo::Size::new(f64::from(width.min(40.0)), f64::from(height.min(24.0))),
        )
        .to_path(0.1);
        let brush: vello::peniko::Brush = vello::peniko::Color::new([1.0, 0.0, 0.0, 1.0]).into();
        scene.fill(
            vello::peniko::Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            &brush,
            &rect,
        );
        false
    }
}

#[test]
fn scene_view_body_merges_to_native_when_marker_is_present() {
    let env = Environment::new().extending(SceneViewMergeToParent);
    let body = SceneView::new(TestSceneContent(Rc::new(Cell::new(false)))).body(&env);
    let any = AnyView::new(body);
    assert!(
        any.is::<Native<SceneView>>(),
        "expected SceneView body to resolve to Native<SceneView> when merge marker is present"
    );
}

#[test]
fn smoke_scene_view_snapshot_runs_build_scene_and_returns_buffer() {
    let build_called = Rc::new(Cell::new(false));
    let mut platform = OffscreenWindow::new_for_tests(96, 72, wgpu::TextureFormat::Rgba8Unorm);
    let mut renderer = {
        let surface = platform.surface();
        HydrolysisRenderer::new(surface.device())
    };
    let bounds = vello::kurbo::Rect::new(0.0, 0.0, 96.0, 72.0);
    let env = Environment::new().extending(SceneViewMergeToParent);

    let surface = platform.surface();
    renderer.set_frame_resources(surface.adapter(), surface.device(), surface.queue());
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(SceneView::new(TestSceneContent(Rc::clone(&build_called)))),
        &env,
        bounds,
        vello::kurbo::Affine::IDENTITY,
        vello::kurbo::Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    assert!(build_called.get(), "expected scene view build_scene to run");

    let frame = surface
        .acquire()
        .expect("waterui-testing failed to acquire offscreen frame");
    renderer.render_scene_to_texture(hydrolysis::HydrolysisRenderTarget {
        adapter: surface.adapter(),
        device: surface.device(),
        queue: surface.queue(),
        texture: Some(frame.texture()),
        view: frame.view(),
        format: surface.format(),
        width: 96,
        height: 72,
        base_color: vello::peniko::Color::TRANSPARENT,
    });
    let rgba8 = readback_texture_rgba8(surface.device(), surface.queue(), frame.texture(), 96, 72);
    renderer.clear_frame_resources();
    surface.present(frame);

    let snapshot = Snapshot {
        width: 96,
        height: 72,
        rgba8,
    };
    assert_eq!(snapshot.width, 96);
    assert_eq!(snapshot.height, 72);
    assert_eq!(snapshot.rgba8.len(), 96 * 72 * 4);
}

#[test]
fn query_chain_and_index_are_type_safe() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Save changes"), None, true),
        node(3, Role::BUTTON, Some("Save draft"), None, false),
    ]));

    let results = app
        .query()
        .role(Role::BUTTON)
        .label_contains("Save")
        .enabled(true)
        .all();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id().as_u64(), 2);
    assert_eq!(
        results[results[0].id()].node().label(),
        Some("Save changes")
    );
    assert_eq!(app.tree()[node_id(2)].label(), Some("Save changes"));
}

#[test]
fn hidden_nodes_are_excluded_unless_requested() {
    let mut hidden = node(3, Role::BUTTON, Some("Hidden action"), None, true);
    hidden.hidden = true;

    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Visible action"), None, true),
        hidden,
    ]));

    app.query()
        .role(Role::BUTTON)
        .label("Visible action")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Hidden action")
        .assert_not_exists();

    let hidden_match = app
        .query()
        .role(Role::BUTTON)
        .label("Hidden action")
        .hidden(true)
        .single();
    assert_eq!(hidden_match.id().as_u64(), 3);
}

#[test]
fn relative_queries_scope_by_semantic_handle() {
    let mut app = mounted(scoped_tree());

    let alpha = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Alpha card")
        .single();
    let beta = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Beta card")
        .single();

    let alpha_button = app
        .query()
        .within(&alpha)
        .role(Role::BUTTON)
        .label("Edit")
        .single();
    let alpha_input = app
        .query()
        .children_of(&alpha)
        .role(Role::TEXT_INPUT)
        .label("Email")
        .single();
    let beta_button = app
        .query()
        .within(&beta)
        .role(Role::BUTTON)
        .label("Edit")
        .single();

    assert_eq!(alpha_button.id().as_u64(), 4);
    assert_eq!(alpha_input.id().as_u64(), 5);
    assert_eq!(beta_button.id().as_u64(), 6);
}

#[test]
fn value_contains_matches_semantic_values() {
    let mut app = mounted(scoped_tree());

    let alpha_email = app
        .query()
        .role(Role::TEXT_INPUT)
        .value_contains("alpha@")
        .single();

    assert_eq!(alpha_email.id().as_u64(), 5);
}

#[test]
fn wait_for_existence_and_nonexistence_complete_immediately() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::LABEL, Some("status"), Some("ready"), true),
    ]));

    let status_selector = Selector::default().role(Role::LABEL).label("status");
    let missing_button_selector = Selector::default().role(Role::BUTTON).label("missing");
    assert!(app.wait_for_existence(&status_selector, Duration::from_millis(50),));
    assert!(app.wait_for_nonexistence(&missing_button_selector, Duration::from_millis(50),));
    assert!(app.wait_for_value_eq(&status_selector, "ready", Duration::from_millis(50),));
}

#[test]
fn wait_for_inverted_reports_fulfillment() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Delete"), None, true),
    ]));

    let expectation = app
        .expect_exists(Selector::default().role(Role::BUTTON).label("Delete"))
        .inverted();
    let result = app.wait_for(&[expectation], WaitOptions::new(Duration::from_millis(10)));
    assert_eq!(result, WaitResult::InvertedFulfillment);
}

#[test]
fn wait_for_times_out_when_condition_never_matches() {
    let mut app = mounted(tree(vec![node(1, Role::LIST, Some("root"), None, true)]));

    let expectation = app.expect_exists(Selector::default().role(Role::BUTTON).label("never"));
    let result = app.wait_for(&[expectation], WaitOptions::new(Duration::from_millis(10)));
    assert_eq!(result, WaitResult::TimedOut);
}

#[test]
fn wait_for_panics_on_empty_expectations() {
    let mut app = mounted(tree(vec![node(1, Role::LIST, Some("root"), None, true)]));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.wait_for(&[], WaitOptions::default());
    }));
    assert!(outcome.is_err());
}

#[test]
fn query_optional_panics_on_multiple_matches() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("A"), None, true),
        node(3, Role::BUTTON, Some("A"), None, true),
    ]));

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = app.query().role(Role::BUTTON).label("A").optional();
    }));
    assert!(outcome.is_err());
}

#[test]
fn element_set_index_by_node_id_panics_when_missing() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("A"), None, true),
    ]));
    let set = app.query().role(Role::BUTTON).all();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = &set[node_id(99)];
    }));
    assert!(outcome.is_err());
}

#[test]
fn stale_handle_panics_for_interaction_and_relative_query() {
    let mut app = mounted(scoped_tree());

    let alpha = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Alpha card")
        .single();
    let edit = app
        .query()
        .within(&alpha)
        .role(Role::BUTTON)
        .label("Edit")
        .single();
    app.tree.revision = 99;

    let interaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = edit.tap(&mut app);
    }));
    let interaction_payload = interaction.expect_err("stale handle should panic");
    let interaction_message = panic_message(&*interaction_payload);
    assert!(
        interaction_message.contains("stale element handle"),
        "unexpected stale interaction panic: {interaction_message}"
    );

    let scoped_query = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = app
            .query()
            .within(&alpha)
            .role(Role::BUTTON)
            .label("Edit")
            .single();
    }));
    let scoped_query_payload = scoped_query.expect_err("stale scoped query should panic");
    let scoped_query_message = panic_message(&*scoped_query_payload);
    assert!(
        scoped_query_message.contains("stale element handle"),
        "unexpected stale scoped query panic: {scoped_query_message}"
    );
}

#[test]
fn ui_test_hover_drag_and_magnify_update_semantic_bounds() {
    use waterui::gesture::{
        DragEvent, DragGesture, GestureObserver, MagnificationEvent, MagnificationGesture,
    };
    use waterui::prelude::text;
    use waterui::{Binding, SignalExt as _, State, ViewExt as _};
    use waterui_core::extract::Use;

    #[derive(Clone)]
    struct DragOffset(Binding<f32>);

    #[derive(Clone)]
    struct ZoomScale(Binding<f32>);

    #[derive(Clone)]
    struct HoverState(Binding<bool>);

    let offset = Binding::f32(0.0);
    let scale = Binding::f32(1.0);
    let hovered = Binding::bool(false);

    let mut app = ui().viewport(160, 160).mount({
        let offset = offset.clone();
        let scale = scale.clone();
        let hovered = hovered.clone();
        move || {
            let drag_offset_state = DragOffset(offset.clone());
            let zoom_scale_state = ZoomScale(scale.clone());
            let hover_state = HoverState(hovered.clone());
            let hovered_opacity = hovered
                .clone()
                .map(|hovered| if hovered { 1.0 } else { 0.68 });
            let surface = text("interactive canvas")
                .padding()
                .size(120.0, 120.0)
                .offset(offset.clone(), 0.0)
                .scale(scale.clone(), scale.clone())
                .opacity(hovered_opacity);
            surface
                .gesture_observer(GestureObserver::new(
                    DragGesture::new(0.0),
                    |State(DragOffset(offset)): State<DragOffset>, drag: Use<DragEvent>| {
                        offset.set(drag.translation.x);
                    },
                ))
                .state(&drag_offset_state)
                .gesture_observer(GestureObserver::new(
                    MagnificationGesture::new(1.0),
                    |State(ZoomScale(scale)): State<ZoomScale>,
                     magnification: Use<MagnificationEvent>| {
                        scale.set(magnification.scale);
                    },
                ))
                .state(&zoom_scale_state)
                .on_hover_enter(|State(HoverState(hovered)): State<HoverState>| hovered.set(true))
                .on_hover_exit(|State(HoverState(hovered)): State<HoverState>| hovered.set(false))
                .state(&hover_state)
        }
    });

    let initial_bounds = app.query().label("interactive canvas").single().bounds();
    assert!(initial_bounds.width() > 0.0 && initial_bounds.height() > 0.0);

    assert!(app.query().label("interactive canvas").hover());
    assert!(hovered.get(), "hover should update the tracked binding");

    let center_before_drag = app.query().label("interactive canvas").single().center();
    assert!(app.magnify_at(center_before_drag.0, center_before_drag.1, 1.2));
    assert!(
        (scale.get() - 1.2).abs() < 0.001,
        "magnify should update the tracked scale binding"
    );

    assert!(app.query().label("interactive canvas").drag_by(24.0, 0.0));
    assert!(
        (offset.get() - 24.0).abs() < 0.001,
        "drag should update the tracked offset binding"
    );

    let center_after_drag = app.query().label("interactive canvas").single().center();
    assert!(
        app.magnify_at(center_after_drag.0, center_after_drag.1, 1.4),
        "magnify after drag should still target stacked gesture observers"
    );
    assert!(
        (scale.get() - 1.4).abs() < 0.001,
        "second magnify should update the tracked scale binding"
    );

    let updated_bounds = app.query().label("interactive canvas").single().bounds();
    assert!(
        updated_bounds.width() > initial_bounds.width(),
        "magnify should grow the accessible bounds width"
    );
    assert!(
        updated_bounds.height() > initial_bounds.height(),
        "magnify should grow the accessible bounds height"
    );
    assert!(
        updated_bounds.x() > initial_bounds.x(),
        "drag should move the accessible bounds horizontally"
    );
}

#[test]
fn ui_test_drains_local_tasks_through_headless_runtime() {
    use waterui::task::spawn_local;
    use waterui::{Binding, ViewExt as _};

    let status = Binding::container(String::from("idle"));
    let status_for_view = status.clone();

    let mut app = ui().theme(install_m3).mount(move || {
        waterui::text!("{status_for_view}")
            .on_appear(|status: waterui::State<Binding<String>>| {
                spawn_local(async move {
                    status.set(String::from("ready"));
                })
                .detach();
            })
            .state(&status_for_view)
    });

    let deadline = std::time::Instant::now() + Duration::from_millis(200);
    while status.get() != "ready" && std::time::Instant::now() < deadline {
        let _ = app.snapshot();
    }
    assert_eq!(
        status.get().as_str(),
        "ready",
        "expected headless runtime to drain spawn_local task and update the binding"
    );
}

#[test]
fn ui_focus_is_separate_from_accessibility_focus() {
    use waterui::form::secure::Secure;
    use waterui::prelude::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Username,
        Password,
    }

    let focus = Binding::container(Some(Field::Username));
    let username = Binding::container(Str::from(""));
    let password = Binding::container(Secure::default());
    let focus_for_view = focus.clone();
    let mut env = Environment::new();
    hydrolysis_m3::install(&mut env);
    let mut app = ui().environment(env).mount(move || {
        vstack((
            TextField::new(&username)
                .label(text("Username"))
                .focused(&focus_for_view, Field::Username),
            SecureField::new(text("Password"), &password).focused(&focus_for_view, Field::Password),
            button("Submit"),
        ))
    });

    let username_selector = Selector::default().role(Role::TEXT_INPUT).label("Username");
    let password_selector = Selector::default()
        .role(Role::PASSWORD_INPUT)
        .label("Password");

    assert!(
        app.wait_for_ui_focus(&username_selector, Duration::from_millis(200)),
        "expected initial FocusState to focus the username field"
    );
    app.assert_ui_focus(&username_selector);
    assert_eq!(focus.get(), Some(Field::Username));

    let username_id = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Username")
        .single()
        .id();
    assert_eq!(app.ui_focus(), Some(username_id));

    assert!(
        app.query()
            .role(Role::PASSWORD_INPUT)
            .label("Password")
            .focus(),
        "expected password field focus action to succeed"
    );
    let password_id = app
        .query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .single()
        .id();
    app.assert_ui_focus(&password_selector);
    assert_eq!(app.ui_focus(), Some(password_id));
    assert_eq!(focus.get(), Some(Field::Password));

    assert!(
        app.query().role(Role::BUTTON).label("Submit").focus(),
        "expected button accessibility focus action to succeed"
    );
    let submit_id = app.query().role(Role::BUTTON).label("Submit").single().id();
    assert_eq!(submit_id, app.tree().focus());
    assert_eq!(app.ui_focus(), Some(password_id));
    assert_eq!(focus.get(), Some(Field::Password));

    assert!(
        app.clear_ui_focus(),
        "expected clear_ui_focus to clear the active FocusState target"
    );
    assert_eq!(app.ui_focus(), None);
    assert_eq!(focus.get(), None);
    assert_eq!(app.tree().focus(), submit_id);
}

#[test]
fn committed_text_keeps_the_caret_at_the_end_across_retained_refreshes() {
    use waterui::prelude::*;

    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();
    let mut env = Environment::new();
    hydrolysis_m3::install(&mut env);
    let mut app = ui()
        .environment(env)
        .mount(move || TextField::new(&value_for_view).label(text("Full Name")));

    assert!(
        app.query()
            .role(Role::TEXT_INPUT)
            .label("Full Name")
            .focus(),
        "expected the text field focus action to succeed"
    );

    let mut expected = String::new();
    for character in "Lexo Liu".chars() {
        expected.push(character);
        assert!(app.text_input(character.to_string()));
        assert_eq!(
            value.get().as_str(),
            expected,
            "each retained refresh must preserve the caret after the committed prefix"
        );
    }
}

// ============================================================================
// Async GPU setup must complete before a frame is captured (issue #149)
// ============================================================================

use waterui::graphics::{GpuContext, GpuFrame, GpuSurface, GpuView, wgpu};

/// A `GpuView` whose `setup` yields before it is ready, the way a real one does
/// while it builds pipelines. It draws nothing until setup has completed, so a
/// capture taken before the executor has driven that future sees only the
/// window background.
#[derive(Debug)]
struct DeferredClearRenderer {
    color: wgpu::Color,
    ready: Rc<Cell<bool>>,
}

impl GpuView for DeferredClearRenderer {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        YieldOnce::default().await;
        self.ready.set(true);
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        if !self.ready.get() {
            return;
        }
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_deferred_gpu_surface_encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hydrolysis_deferred_gpu_surface_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        frame.queue.submit([encoder.finish()]);
    }
}

/// Returns `Pending` exactly once, so a future awaiting it needs a second poll.
#[derive(Default)]
struct YieldOnce {
    polled: bool,
}

impl std::future::Future for YieldOnce {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        if self.polled {
            return std::task::Poll::Ready(());
        }
        self.polled = true;
        cx.waker().wake_by_ref();
        std::task::Poll::Pending
    }
}

/// A `GpuSurface` must reach the captured frame even though its `setup` is
/// async. Capturing the very first pumped frame photographs the surface before
/// any GPU content exists — the regression that made every GPU preview in the
/// book render as a flat background.
#[test]
fn headless_capture_waits_for_async_gpu_setup() {
    let ready = Rc::new(Cell::new(false));
    let ready_for_view = Rc::clone(&ready);
    let content = AnyViewBuilder::new(move || {
        AnyView::new(GpuSurface::new(DeferredClearRenderer {
            color: wgpu::Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            ready: Rc::clone(&ready_for_view),
        }))
    });

    let mut env = Environment::new();
    hydrolysis::testing::install_theme(&mut env);
    install_m3(&mut env);
    let mut runtime = hydrolysis::HeadlessRuntime::new_for_tests(env, content, 64, 64);

    // Pump until the frame settles, exactly as the preview runtime does.
    let mut settled = false;
    for _ in 0..64 {
        if !runtime.pump_at(false, std::time::Instant::now()).rebuilt {
            settled = true;
            break;
        }
    }
    assert!(settled, "frame never settled");
    assert!(
        ready.get(),
        "the async GpuView::setup must have been driven to completion"
    );

    let snapshot = runtime
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("capture must produce a snapshot");
    let center = ((snapshot.width as usize / 2)
        + (snapshot.height as usize / 2) * snapshot.width as usize)
        * 4;
    assert_eq!(
        &snapshot.rgba8[center..center + 3],
        &[255, 0, 0],
        "the GpuSurface content must be present in the captured frame"
    );
}

