use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use crate::driver::{A11yDriver, DriverPumpResult};
use accesskit::{ActionRequest as AccessibilityActionRequest, NodeId as AccessibilityNodeId};
use hydrolysis::{HydrolysisRenderer, OffscreenWindow, PlatformWindow};
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

    fn magnify_at(&mut self, _x: f32, _y: f32, _factor: f32, _env: &Environment) -> bool {
        false
    }

    fn clear_ui_focus(&mut self, _env: &Environment) -> bool {
        false
    }

    fn ui_focus(&self) -> Option<NodeId> {
        None
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
        ui_focus: None,
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
fn smoke_theme_foreground_slot_snapshot_contains_visible_pixels() {
    let mut env = Environment::new();
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
    let host = TestHost::new(env, 240, 120);
    let snapshot = host.render(
        vstack((text("Theme slot").body(), text("Theme slot").body())).background(Srgb::BLACK),
    );
    let white_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0 && px[0] > 180 && px[1] > 180 && px[2] > 180)
        .count();
    assert!(
        white_pixels > 0,
        "expected theme foreground slot render to produce visible bright pixels (white_pixels={white_pixels})"
    );
}

#[test]
fn smoke_text_color_snapshot_contains_visible_pixels() {
    let host = TestHost::new(Environment::new(), 240, 120);
    let snapshot = host.render(
        vstack((
            text("Explicit color").body().color(Srgb::WHITE),
            text("Explicit color").body().color(Srgb::WHITE),
        ))
        .background(Srgb::BLACK),
    );
    let white_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0 && px[0] > 180 && px[1] > 180 && px[2] > 180)
        .count();
    assert!(
        white_pixels > 0,
        "expected explicit text color render to produce visible bright pixels (white_pixels={white_pixels})"
    );
}

#[test]
fn smoke_text_snapshot_contains_visible_pixels() {
    let host = TestHost::new(Environment::new(), 240, 120);
    let snapshot = host.render(
        vstack((
            text("Focused datum").body().foreground(Srgb::WHITE),
            text("Selected datum").body().foreground(Srgb::WHITE),
        ))
        .background(Srgb::BLACK),
    );
    let white_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0 && px[0] > 180 && px[1] > 180 && px[2] > 180)
        .count();
    assert!(
        white_pixels > 0,
        "expected text render to produce visible bright pixels (white_pixels={white_pixels})"
    );
}

#[test]
fn ui_test_snapshot_renders_text_after_canvas() {
    let mut app = UiTest::new().viewport(320, 320).mount(|| {
        vstack((
            Canvas::new(|ctx| {
                ctx.set_fill_style(Srgb::new(0.0, 0.85, 0.65));
                ctx.fill_rect(Rect::new(Point::new(0.0, 0.0), Size::new(240.0, 180.0)));
            })
            .size(240.0, 180.0),
            text("W")
                .size(48.0)
                .color(Srgb::WHITE)
                .body()
                .padding_with(6.0),
        ))
        .spacing(6.0)
        .background(Srgb::BLACK)
    });
    let snapshot = app.snapshot();
    let non_black_pixels = snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0))
        .count();
    assert!(
        non_black_pixels > 240 * 180,
        "expected mounted snapshot to preserve text after canvas (non_black_pixels={non_black_pixels})"
    );
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

    let _ = app.snapshot();
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
}

#[test]
fn ui_test_drains_local_tasks_through_headless_runtime() {
    use waterui::task::spawn_local;
    use waterui::{Binding, ViewExt as _};

    let status = Binding::container(String::from("idle"));
    let status_for_view = status.clone();

    let mut app = UiTest::new().mount(move || {
        waterui::text!("{status_for_view}")
            .with_state(&status_for_view)
            .on_appear(|status: Binding<String>| {
                spawn_local(async move {
                    status.set(String::from("ready"));
                })
                .detach();
            })
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
    let username_for_view = username.clone();
    let password_for_view = password.clone();

    let mut app = UiTest::new().mount(move || {
        vstack((
            TextField::new(&username_for_view)
                .label(text("Username"))
                .focused(&focus_for_view, Field::Username),
            SecureField::new(text("Password"), &password_for_view)
                .focused(&focus_for_view, Field::Password),
            button("Submit"),
        ))
    });

    let username_selector = Selector::default().role(Role::TEXT_INPUT).label("Username");
    let password_selector = Selector::default()
        .role(Role::PASSWORD_INPUT)
        .label("Password");

    assert!(
        app.wait_for_ui_focus(username_selector.clone(), Duration::from_millis(200)),
        "expected initial FocusState to focus the username field"
    );
    app.assert_ui_focus(username_selector.clone());
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
    app.assert_ui_focus(password_selector.clone());
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
