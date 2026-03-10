use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use crate::driver::{A11yDriver, DriverPumpResult};
use accesskit::{ActionRequest as AccessibilityActionRequest, NodeId as AccessibilityNodeId};
use hydrolysis::{HydrolysisRenderer, OffscreenWindow, PlatformWindow};
use vello::kurbo::Shape;
use waterui::View as _;
use waterui::graphics::SceneViewMergeToParent;
use waterui::graphics::color::Srgb;
use waterui::graphics::{Scene2D, SceneContent, SceneView};
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
        _content: AnyView,
        _env: &Environment,
        _capture_snapshot: bool,
    ) -> DriverPumpResult {
        DriverPumpResult {
            rebuilt: false,
            tree_update: None,
            snapshot: None,
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

    fn magnify_at(&mut self, _x: f32, _y: f32, _factor: f32, _env: &Environment) -> bool {
        false
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

fn mounted(tree: TreeSnapshot) -> MountedApp {
    MountedApp {
        env: Environment::new(),
        content: AnyViewBuilder::new(|| AnyView::new(())),
        driver: Box::new(NoopDriver),
        tree,
        revision: 2,
    }
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
fn smoke_canvas_snapshot_contains_visible_pixels() {
    let host = TestHost::new(Environment::new(), 96, 72);
    let snapshot = host.render(Canvas::new(|ctx| {
        ctx.set_fill_style(Srgb::new(1.0, 0.0, 0.0));
        ctx.fill_rect(Rect::new(Point::new(8.0, 8.0), Size::new(40.0, 24.0)));
    }));
    let colored_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
        .count();
    let opaque_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0)
        .count();
    assert!(
        opaque_pixels > 0,
        "expected canvas render to produce visible pixels (colored_pixels={colored_pixels}, opaque_pixels={opaque_pixels})"
    );
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
fn smoke_scene_view_snapshot_contains_visible_pixels() {
    let build_called = Rc::new(Cell::new(false));
    let mut platform = OffscreenWindow::new(96, 72, wgpu::TextureFormat::Rgba8Unorm);
    let mut renderer = {
        let surface = platform.surface();
        HydrolysisRenderer::new(surface.device())
    };
    let bounds = vello::kurbo::Rect::new(0.0, 0.0, 96.0, 72.0);
    let env = Environment::new().extending(SceneViewMergeToParent);

    let surface = platform.surface();
    renderer.set_frame_resources(surface.device(), surface.queue());
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.dispatch(
        SceneView::new(TestSceneContent(Rc::clone(&build_called))),
        &env,
        bounds,
    );
    renderer.finish_rebuild_frame();
    assert!(build_called.get(), "expected scene view build_scene to run");

    let frame = surface
        .acquire()
        .expect("waterui-testing failed to acquire offscreen frame");
    renderer.render_scene_to_texture(
        surface.device(),
        surface.queue(),
        frame.view(),
        surface.format(),
        96,
        72,
        vello::peniko::Color::TRANSPARENT,
    );
    let rgba8 = readback_texture_rgba8(surface.device(), surface.queue(), frame.texture(), 96, 72);
    renderer.clear_frame_resources();
    surface.present(frame);

    let snapshot = Snapshot {
        width: 96,
        height: 72,
        rgba8,
    };
    let colored_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
        .count();
    let opaque_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0)
        .count();
    assert!(
        opaque_pixels > 0,
        "expected scene view render to produce visible pixels (colored_pixels={colored_pixels}, opaque_pixels={opaque_pixels})"
    );
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
fn wait_for_existence_and_nonexistence_complete_immediately() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::LABEL, Some("status"), Some("ready"), true),
    ]));

    assert!(app.wait_for_existence(
        Selector::default().role(Role::LABEL).label("status"),
        Duration::from_millis(50),
    ));
    assert!(app.wait_for_nonexistence(
        Selector::default().role(Role::BUTTON).label("missing"),
        Duration::from_millis(50),
    ));
    assert!(app.wait_for_value_eq(
        Selector::default().role(Role::LABEL).label("status"),
        "ready",
        Duration::from_millis(50),
    ));
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
fn snapshot_changed_pixels_reports_differences() {
    let host = TestHost::new(Environment::new(), 48, 48);
    let before = host.render(Canvas::new(|ctx| {
        ctx.set_fill_style(Srgb::new(1.0, 0.0, 0.0));
        ctx.fill_rect(Rect::new(Point::new(4.0, 4.0), Size::new(16.0, 16.0)));
    }));
    let after = host.render(Canvas::new(|ctx| {
        ctx.set_fill_style(Srgb::new(0.0, 1.0, 0.0));
        ctx.fill_rect(Rect::new(Point::new(4.0, 4.0), Size::new(16.0, 16.0)));
    }));
    assert!(before.changed_pixels(&after) > 0);
    assert!(before.changed_ratio(&after) > 0.0);
}

#[test]
fn ui_test_hover_drag_and_magnify_change_snapshot() {
    use waterui::accessibility::AccessibilityRole;
    use waterui::gesture::{
        DragEvent, DragGesture, GestureObserver, MagnificationEvent, MagnificationGesture,
    };
    use waterui::{Binding, SignalExt as _, ViewExt as _};
    use waterui_core::Metadata;

    let offset = Binding::f32(0.0);
    let scale = Binding::f32(1.0);
    let hovered = Binding::bool(false);

    let mut app = UiTest::new().viewport(160, 160).mount({
        let offset = offset.clone();
        let scale = scale.clone();
        let hovered = hovered.clone();
        move || {
            let canvas = Canvas::with_signal(
                offset.zip(&scale).zip(&hovered),
                |ctx: &mut waterui_canvas::DrawingContext<'_>, ((offset, scale), hovered)| {
                    let background = if hovered {
                        Srgb::new(0.15, 0.18, 0.22)
                    } else {
                        Srgb::new(0.04, 0.05, 0.06)
                    };
                    ctx.set_fill_style(background);
                    ctx.fill_rect(Rect::new(
                        Point::new(0.0, 0.0),
                        Size::new(ctx.width, ctx.height),
                    ));
                    ctx.set_fill_style(Srgb::new(0.95, 0.32, 0.18));
                    let marker_size = 28.0 * scale;
                    ctx.fill_rect(Rect::new(
                        Point::new(22.0 + offset, 48.0),
                        Size::new(marker_size, marker_size),
                    ));
                },
            )
            .size(120.0, 120.0);
            let canvas = Metadata::new(
                canvas,
                GestureObserver::new(DragGesture::new(0.0))
                    .with_state(&offset)
                    .action_with_env(|offset: Binding<f32>, env| {
                        let drag = env
                            .get::<DragEvent>()
                            .expect("test drag gesture missing DragEvent");
                        offset.set(drag.translation.x);
                    }),
            );
            let canvas = Metadata::new(
                canvas,
                GestureObserver::new(MagnificationGesture::new(1.0))
                    .with_state(&scale)
                    .action_with_env(|scale: Binding<f32>, env| {
                        let magnification = env
                            .get::<MagnificationEvent>()
                            .expect("test magnification gesture missing MagnificationEvent");
                        scale.set(magnification.scale);
                    }),
            );
            canvas
                .with_state(&hovered)
                .on_hover_enter(|hovered: Binding<bool>| hovered.set(true))
                .on_hover_exit(|hovered: Binding<bool>| hovered.set(false))
                .a11y_label("interactive canvas")
                .a11y_role(AccessibilityRole::Button)
        }
    });

    let bounds = app.query().label("interactive canvas").single().bounds();
    assert!(bounds.width() > 0.0 && bounds.height() > 0.0);

    let base = app.snapshot();
    assert!(app.query().label("interactive canvas").hover());
    let hovered_frame = app.snapshot();
    assert!(base.changed_pixels(&hovered_frame) > 0);

    let center_before_drag = app.query().label("interactive canvas").single().center();
    assert!(app.magnify_at(center_before_drag.0, center_before_drag.1, 1.2));
    let magnified_before_drag = app.snapshot();
    assert!(hovered_frame.changed_pixels(&magnified_before_drag) > 0);

    assert!(app.query().label("interactive canvas").drag_by(24.0, 0.0));
    let dragged_frame = app.snapshot();
    assert!(magnified_before_drag.changed_pixels(&dragged_frame) > 0);

    let center_after_drag = app.query().label("interactive canvas").single().center();
    assert!(
        app.magnify_at(center_after_drag.0, center_after_drag.1, 1.4),
        "magnify after drag should still target stacked gesture observers"
    );
    let magnified_frame = app.snapshot();
    assert!(dragged_frame.changed_pixels(&magnified_frame) > 0);
}
