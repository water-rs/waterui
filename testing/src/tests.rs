use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use crate::driver::{DriverPumpResult, ResourceSampler};
use accesskit::{ActionRequest as AccessibilityActionRequest, NodeId as AccessibilityNodeId};
use hydrolysis::InputEvent;
use vello::kurbo::Shape;
use waterui::ViewExt as _;
use waterui::component::{text, vstack};
use waterui::graphics::SceneViewMergeToParent;
use waterui::graphics::color::Srgb;
use waterui::graphics::{Scene2D, SceneContent, SceneView};
use waterui::layout::scroll::ScrollView;
use waterui::text::Text;
use waterui_canvas::Canvas;
use waterui_core::layout::{Point, Rect, Size};
use waterui_core::{AnyView, Native, View};

#[derive(Debug)]
struct NoopDriver;

impl RuntimeDriver for NoopDriver {
    fn pump_at(&mut self, _at: std::time::Instant, _capture_snapshot: bool) -> DriverPumpResult {
        DriverPumpResult {
            rebuilt: false,
            profile: hydrolysis::FrameProfile::default(),
            tree_update: None,
            snapshot: None,
            ui_focus: None,
        }
    }

    fn is_settled(&self) -> bool {
        true
    }

    fn has_pending_semantic_update(&self) -> bool {
        false
    }

    fn perform_accessibility_action(&mut self, _request: AccessibilityActionRequest) -> bool {
        false
    }

    fn push_input_event(&mut self, _event: InputEvent) {}

    fn request_redraw(&mut self) {}

    fn clear_ui_focus(&mut self) -> bool {
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
        identifier: None,
        value: value.map(ToOwned::to_owned),
        bounds: None,
        enabled,
        selected: false,
        checked: None,
        expanded: None,
        busy: false,
        hidden: false,
        children: Vec::new(),
        actions: Vec::new(),
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

fn mounted(tree: TreeSnapshot) -> SemanticApp<NoopDriver> {
    SemanticApp {
        runtime: NoopDriver,
        tree,
        ui_focus: None,
        revision: 2,
        viewport: (0, 0),
        clock: None,
        resources: ResourceSampler::new(),
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

/// A view that draws widget chrome mounts on the semantic pipeline: the scroll
/// view's container and the button's node are products of the view tree and
/// the widgets' semantics — no style package is installed (#290).
#[test]
fn scroll_view_and_button_structure_mounts_without_a_style() {
    let mut app = ui().viewport(240, 160).mount(|| {
        ScrollView::vertical(vstack((
            waterui::component::button("Submit"),
            text("Scrolled content").body(),
        )))
    });
    assert_eq!(app.query().role(Role::BUTTON).all().len(), 1);
    app.query()
        .role(Role::LABEL)
        .label("Scrolled content")
        .assert_exists();
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
fn a11y_identifier_flows_from_modifier_to_selector() {
    let mut app = ui().mount(|| {
        vstack((
            waterui::component::button("Submit").a11y_id("login.submit"),
            waterui::component::button("Submit"),
        ))
    });
    let element = app.query().identifier("login.submit").single();
    assert_eq!(element.node().identifier(), Some("login.submit"));
    assert_eq!(element.node().label(), Some("Submit"));
    // The identifier is nearest-consumer metadata: the second, unadorned
    // button must not inherit it.
    assert_eq!(
        app.query().role(Role::BUTTON).all().len(),
        2,
        "both buttons stay queryable by role"
    );
    app.query().identifier("login.submit").tap();
}

/// Explicit `.color(...)` is a view-tree attribute: on the semantic pipeline
/// the two labels stay queryable with it set.
#[test]
fn explicit_text_color_preserves_semantic_labels() {
    let mut app = ui().viewport(240, 120).mount(|| {
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
fn smoke_text_preserves_semantic_labels() {
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
    app.query().role(Role::BUTTON).label("Assist").tap();
    assert!(
        tapped.get(),
        "accessibility click should trigger tap gesture"
    );
    app.query()
        .role(Role::LABEL)
        .label("Assist")
        .assert_not_exists();
}

/// A `Canvas` composes to `SceneView`, whose accessibility metadata is a
/// product of the view tree — the IMAGE role and its label exist on the
/// semantic pipeline with nothing rendered.
#[test]
fn canvas_and_text_expose_accessibility_nodes_semantically() {
    let mut app = ui().viewport(320, 320).mount(|| {
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
}

/// A `SceneView` marked to merge exposes its accessibility node on the
/// semantic pipeline: the role and label are view-tree metadata, the scene's
/// pixels are the rendered pipeline's concern.
#[test]
fn scene_view_exposes_accessibility_node_semantically() {
    let mut app = ui().viewport(96, 72).mount(|| {
        SceneView::new(TestSceneContent(Rc::new(Cell::new(false))))
            .a11y_role(waterui::accessibility::AccessibilityRole::Image)
            .a11y_label("Scene layer")
    });
    app.query()
        .role(Role::IMAGE)
        .label("Scene layer")
        .assert_exists();
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
            None,
            &rect,
        );
        false
    }
}

#[test]
fn scene_view_body_merges_to_native_when_marker_is_present() {
    let env = waterui_core::Environment::new().extending(SceneViewMergeToParent);
    let body = SceneView::new(TestSceneContent(Rc::new(Cell::new(false)))).body(&env);
    let any = AnyView::new(body);
    assert!(
        any.is::<Native<SceneView>>(),
        "expected SceneView body to resolve to Native<SceneView> when merge marker is present"
    );
}

/// `spawn_local` work scheduled from `on_appear` drains on the semantic
/// pipeline too — the parked-task executor is runtime-agnostic.
#[test]
fn semantic_mount_drains_spawned_local_work() {
    use waterui::task::spawn_local;
    use waterui::{Binding, ViewExt as _};

    let status = Binding::container(String::from("idle"));
    let status_for_view = status.clone();

    let mut app = ui().mount(move || {
        waterui::text!("{status_for_view}")
            .on_appear(|status: waterui::State<Binding<String>>| {
                spawn_local(async move {
                    status.set(String::from("ready"));
                })
                .detach();
            })
            .state(&status_for_view)
    });

    let status_selector = Selector::default().role(Role::LABEL).label("ready");
    assert!(
        app.wait_for_existence(&status_selector, Duration::from_millis(500)),
        "expected the semantic runtime to drain spawn_local task and update the binding"
    );
    assert_eq!(status.get().as_str(), "ready");
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
fn mixed_and_busy_selectors_preserve_complete_accessibility_state() {
    let mut state = node(2, Role::CHECKBOX, Some("Sync all"), None, true);
    state.checked = Some(CheckedState::Mixed);
    state.busy = true;
    let mut app = mounted(tree(vec![
        node(1, Role::GROUP, Some("root"), None, true),
        state,
    ]));

    let element = app.query().role(Role::CHECKBOX).mixed().busy(true).single();

    assert_eq!(element.node().checked_state(), Some(CheckedState::Mixed));
    assert!(element.node().busy());
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
fn wait_for_ordered_expectations_skip_inverted_positions() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::LABEL, Some("Ready"), None, true),
        node(3, Role::BUTTON, Some("Continue"), None, true),
    ]));

    // An inverted expectation holds no position in the required order, so the
    // two present elements fulfill in list order and the wait completes.
    let expectations = [
        app.expect_exists(Selector::default().role(Role::BUTTON).label("Delete"))
            .inverted(),
        app.expect_exists(Selector::default().role(Role::LABEL).label("Ready")),
        app.expect_exists(Selector::default().role(Role::BUTTON).label("Continue")),
    ];
    let result = app.wait_for(
        &expectations,
        WaitOptions::new(Duration::from_millis(10)).enforce_order(true),
    );
    assert_eq!(result, WaitResult::Completed);
}

#[test]
fn query_exists_is_true_for_multiple_matches() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("A"), None, true),
        node(3, Role::BUTTON, Some("A"), None, true),
    ]));

    assert!(app.query().role(Role::BUTTON).label("A").exists());
    assert!(!app.query().role(Role::BUTTON).label("B").exists());
}

#[test]
fn assert_ui_focus_failure_names_the_actual_focus_target() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Save"), None, true),
        node(3, Role::BUTTON, Some("Cancel"), None, true),
    ]));
    app.ui_focus = Some(node_id(3));

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.assert_ui_focus(&Selector::default().role(Role::BUTTON).label("Save"));
    }));
    let message = panic_message(&*outcome.expect_err("assertion must fail"));
    assert!(
        message.contains("Cancel"),
        "failure must name the actual focus target: {message}"
    );
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
        edit.tap(&mut app);
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
    let mut app = ui().mount(move || {
        vstack((
            TextField::new(text("Username"), &username).focused(&focus_for_view, Field::Username),
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

    app.query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .focus();
    let password_id = app
        .query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .single()
        .id();
    app.assert_ui_focus(&password_selector);
    assert_eq!(app.ui_focus(), Some(password_id));
    assert_eq!(focus.get(), Some(Field::Password));

    app.query().role(Role::BUTTON).label("Submit").focus();
    let submit_id = app.query().role(Role::BUTTON).label("Submit").single().id();
    assert_eq!(submit_id, app.tree().focus());
    assert_eq!(app.ui_focus(), Some(password_id));
    assert_eq!(focus.get(), Some(Field::Password));

    app.clear_ui_focus();
    assert_eq!(app.ui_focus(), None);
    assert_eq!(focus.get(), None);
    assert_eq!(app.tree().focus(), submit_id);
}

#[test]
fn runtime_focus_writes_move_and_clear_ui_focus() {
    use waterui::form::secure::Secure;
    use waterui::prelude::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Username,
        Password,
    }

    let focus = Binding::container(None::<Field>);
    let username = Binding::container(Str::from(""));
    let password = Binding::container(Secure::default());
    let focus_for_view = focus.clone();
    let mut app = ui().mount(move || {
        vstack((
            TextField::new(text("Username"), &username).focused(&focus_for_view, Field::Username),
            SecureField::new(text("Password"), &password).focused(&focus_for_view, Field::Password),
        ))
    });

    let username_selector = Selector::default().role(Role::TEXT_INPUT).label("Username");
    let password_selector = Selector::default()
        .role(Role::PASSWORD_INPUT)
        .label("Password");

    assert_eq!(app.ui_focus(), None);

    focus.set(Some(Field::Password));
    assert!(
        app.wait_for_ui_focus(&password_selector, Duration::from_millis(200)),
        "a runtime write to the focus binding must move UI focus to the password field"
    );
    app.assert_ui_focus(&password_selector);

    focus.set(Some(Field::Username));
    assert!(
        app.wait_for_ui_focus(&username_selector, Duration::from_millis(200)),
        "a later write must move UI focus back to the username field"
    );

    focus.set(None);
    app.settle();
    assert_eq!(app.ui_focus(), None);
}

#[test]
fn ui_focus_accepts_a_new_target_after_being_cleared() {
    use waterui::prelude::*;

    let focus = Binding::container(None::<i32>);
    let value = Binding::container(Str::from(""));
    let focus_for_view = focus.clone();
    let mut app =
        ui().mount(move || TextField::new(text("Field"), &value).focused(&focus_for_view, 0));

    let selector = Selector::default().role(Role::TEXT_INPUT).label("Field");

    focus.set(Some(0));
    assert!(app.wait_for_ui_focus(&selector, Duration::from_millis(200)));

    app.clear_ui_focus();
    assert_eq!(app.ui_focus(), None);
    assert_eq!(focus.get(), None);

    app.query().role(Role::TEXT_INPUT).label("Field").focus();
    app.assert_ui_focus(&selector);
    assert_eq!(focus.get(), Some(0));
}

#[test]
#[should_panic(expected = "requires exactly one TextField or SecureField")]
fn focused_modifier_without_a_text_anchor_panics() {
    use waterui::prelude::*;

    let focus = Binding::container(None::<i32>);
    let _app = ui().mount(move || button("No anchor").focused(&focus, 0));
}

#[test]
#[should_panic(expected = "found 2")]
fn focused_modifier_with_two_text_anchors_panics() {
    use waterui::prelude::*;

    let focus = Binding::container(None::<i32>);
    let first = Binding::container(Str::from(""));
    let second = Binding::container(Str::from(""));
    let _app = ui().mount(move || {
        vstack((
            TextField::new(text("First"), &first),
            TextField::new(text("Second"), &second),
        ))
        .focused(&focus, 0)
    });
}

#[test]
#[should_panic(expected = "multiple .focused()")]
fn focused_modifier_twice_on_the_same_control_panics() {
    use waterui::prelude::*;

    let focus_a = Binding::container(None::<i32>);
    let focus_b = Binding::container(None::<i32>);
    let value = Binding::container(Str::from(""));
    let _app = ui().mount(move || {
        TextField::new(text("Field"), &value)
            .focused(&focus_a, 0)
            .focused(&focus_b, 1)
    });
}

#[test]
fn committed_text_keeps_the_caret_at_the_end_across_retained_refreshes() {
    use waterui::prelude::*;

    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();
    let mut app = ui().mount(move || TextField::new(text("Full Name"), &value_for_view));

    app.query()
        .role(Role::TEXT_INPUT)
        .label("Full Name")
        .focus();

    let mut expected = String::new();
    for character in "Lexo Liu".chars() {
        expected.push(character);
        app.text_input(character.to_string());
        assert_eq!(
            value.get().as_str(),
            expected,
            "each retained refresh must preserve the caret after the committed prefix"
        );
    }
}

/// A query answers about the app's state now, not as of the last interaction.
///
/// Every input path settles after dispatching, so a tap's consequences are in
/// the tree by the time the call returns. State a test changes directly — a
/// `Binding` it owns, set the way app code would — goes through no such path.
/// Without the sync on read, the next query answered from the tree as it stood
/// before the change: it reported the old label, and an assertion that should
/// have failed passed.
#[test]
fn a_query_sees_state_changed_since_the_last_pump() {
    let label = waterui::reactive::binding(waterui::Str::from("before"));
    let probe = label.clone();
    let mut app = crate::ui().mount(move || vstack((Text::computed(label.clone()),)));

    app.query()
        .role(crate::Role::LABEL)
        .label("before")
        .assert_exists();

    probe.set(waterui::Str::from("after"));

    app.query()
        .role(crate::Role::LABEL)
        .label("after")
        .assert_exists();
    app.query()
        .role(crate::Role::LABEL)
        .label("before")
        .assert_not_exists();
}

/// An app whose only activity is visual-only repaint still answers queries
/// promptly.
///
/// An indeterminate indicator repaints forever on a rendered runtime, but that
/// work never moves semantic state — so the semantic runtime settles with the
/// indicator on screen, and a query never waits on a pump budget it cannot
/// satisfy. What queries wait on is *unapplied* work, which is the state the
/// app leaves whenever a binding changes.
#[test]
fn a_visually_animating_app_still_settles_and_stays_current() {
    let label = waterui::reactive::binding(waterui::Str::from("before"));
    let probe = label.clone();
    let mut app = crate::ui().mount(move || {
        vstack((
            waterui::component::progress::loading().label("Loading"),
            Text::computed(label.clone()),
        ))
    });

    assert!(
        app.runtime.is_settled(),
        "an indeterminate indicator repaints forever but moves no semantic state — the semantic runtime settles"
    );
    assert!(
        !app.runtime.has_pending_semantic_update(),
        "with nothing unapplied the tree is current"
    );

    probe.set(waterui::Str::from("after"));
    assert!(
        app.runtime.has_pending_semantic_update(),
        "a signal change leaves an update the last flush did not apply"
    );

    app.query()
        .role(crate::Role::LABEL)
        .label("after")
        .assert_exists();
    assert!(
        !app.runtime.has_pending_semantic_update(),
        "reading the tree must have applied the update, not merely waited for it"
    );
}
