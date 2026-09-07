//! End-to-end accessibility tree publication and action routing.

use core::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Role, TreeId};
use nami::{SignalExt as _, binding};
use waterui::prelude::Color;
use waterui::view::ViewExt as _;
use waterui_controls::button::button;
use waterui_controls::toggle::toggle;
use waterui_core::{AnyView, Str};
use waterui_dew::{DewRuntime, HostBoard};
use waterui_math::ast::MathStyle;
use waterui_math::view::Math;
use waterui_math::{latex, mathml};
use waterui_navigation::{NavigationStack, NavigationView, Tab, Tabs};
use waterui_text::text;

mod support;

#[test]
fn button_is_published_and_accessibility_click_invokes_it() {
    let invocations = Rc::new(Cell::new(0));
    let action_invocations = Rc::clone(&invocations);
    let mut runtime = DewRuntime::new(
        HostBoard::new(200, 48),
        support::test_environment(),
        16,
        move || {
            let action_invocations = Rc::clone(&action_invocations);
            AnyView::new(button("Save").action(move || {
                action_invocations.set(action_invocations.get() + 1);
            }))
        },
    );
    runtime.pump().expect("the first frame renders");

    let update = runtime
        .board()
        .accessibility_tree()
        .expect("Dew must publish an accessibility tree");
    let (button_id, button_node) = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Button)
        .expect("the semantic button must be present");
    assert_eq!(button_node.label(), Some("Save"));
    assert!(button_node.supports_action(Action::Click));
    let button_id = *button_id;

    runtime
        .board_mut()
        .push_accessibility_action(ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: button_id,
            data: None,
        });
    runtime
        .pump()
        .expect("an accessibility click must drive one retained refresh");
    assert_eq!(invocations.get(), 1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Screen {
    Now,
    Later,
}

#[test]
fn tabs_publish_selection_and_accessibility_click_selects_a_page() {
    let selection = binding(Screen::Now);
    let observed = selection.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(240, 240),
        support::test_environment(),
        16,
        move || {
            AnyView::new(Tabs::new(
                &selection,
                vec![
                    Tab::new(Screen::Now, "Now", || {
                        NavigationView::new("Now", Color::red())
                    }),
                    Tab::new(Screen::Later, "Later", || {
                        NavigationView::new("Later", Color::blue())
                    }),
                ],
            ))
        },
    );
    runtime.pump().expect("the first tab frame renders");

    let update = runtime
        .board()
        .accessibility_tree()
        .expect("tabs publish an accessibility tree");
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == Role::TabList)
            .count(),
        1
    );
    let (later_id, later) = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Tab && node.label() == Some("Later"))
        .expect("the Later tab is semantic");
    assert_eq!(later.is_selected(), Some(false));
    assert!(later.supports_action(Action::Click));
    let later_id = *later_id;

    runtime
        .board_mut()
        .push_accessibility_action(ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: later_id,
            data: None,
        });
    runtime
        .pump()
        .expect("selecting a tab through accessibility renders its page");
    assert_eq!(observed.get(), Screen::Later);
    let later = runtime
        .board()
        .accessibility_tree()
        .expect("the updated tab tree is published")
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.role() == Role::Tab && node.label() == Some("Later")).then_some(node)
        })
        .expect("the Later tab remains semantic");
    assert_eq!(later.is_selected(), Some(true));
}

#[test]
fn navigation_publishes_a_group_with_its_visible_title() {
    let mut runtime = DewRuntime::new(
        HostBoard::new(240, 240),
        support::test_environment(),
        16,
        || {
            AnyView::new(NavigationStack::new(NavigationView::new(
                "Rooms",
                Color::red(),
            )))
        },
    );
    runtime.pump().expect("the navigation frame renders");

    let update = runtime
        .board()
        .accessibility_tree()
        .expect("navigation publishes an accessibility tree");
    let navigation = update
        .nodes
        .iter()
        .find_map(|(_, node)| (node.role() == Role::Navigation).then_some(node))
        .expect("the visible destination is a navigation group");
    assert!(!navigation.children().is_empty());
    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| { node.role() == Role::Label && node.value() == Some("Rooms") })
    );
}

/// The application's own name for a control wins over the control's title.
///
/// A toggle names its node after its visible title, which is right until the
/// application says otherwise: `.a11y_label(..)` exists precisely because the
/// visible text is not always what a screen reader should hear. Dew consumed no
/// naming metadata at all, so the override reached nothing and every node on
/// this backend answered to whatever it had generated for itself.
#[test]
fn an_application_label_replaces_a_controls_own_title() {
    let ready = binding(false);
    let mut runtime = DewRuntime::new(
        HostBoard::new(240, 64),
        support::test_environment(),
        16,
        move || AnyView::new(toggle("Wi-Fi", &ready).a11y_label("Wireless network")),
    );
    runtime.pump().expect("the toggle frame renders");

    let update = runtime
        .board()
        .accessibility_tree()
        .expect("dew publishes an accessibility tree");
    let switch = update
        .nodes
        .iter()
        .find_map(|(_, node)| (node.role() == Role::Switch).then_some(node))
        .expect("the toggle publishes a switch node");
    assert_eq!(switch.label(), Some("Wireless network"));
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("Wi-Fi")),
        "the overridden title must not survive anywhere in the tree"
    );
}

/// With nothing said about it, the control keeps naming itself: the override is
/// a replacement for the default, not a requirement for having one.
#[test]
fn a_control_keeps_its_own_title_when_the_application_names_nothing() {
    let ready = binding(false);
    let mut runtime = DewRuntime::new(
        HostBoard::new(240, 64),
        support::test_environment(),
        16,
        move || AnyView::new(toggle("Wi-Fi", &ready)),
    );
    runtime.pump().expect("the toggle frame renders");

    let update = runtime
        .board()
        .accessibility_tree()
        .expect("dew publishes an accessibility tree");
    let switch = update
        .nodes
        .iter()
        .find_map(|(_, node)| (node.role() == Role::Switch).then_some(node))
        .expect("the toggle publishes a switch node");
    assert_eq!(switch.label(), Some("Wi-Fi"));
}

/// A scene leaf takes the application's name over the description its content
/// generated for itself.
///
/// A formula answers `SceneContent::accessibility_label` with its `MathML`, which
/// is the best a drawing can say about itself and still worse than a caption
/// the application wrote. The two must not both be announced, so the override
/// replaces it rather than adding to it.
#[test]
fn an_application_label_replaces_a_scene_leaf_default_description() {
    const FORMULA: &str = r"\frac{a}{b}";

    let mut runtime = DewRuntime::new(
        HostBoard::new(160, 160),
        support::test_environment(),
        16,
        || AnyView::new(Math::new(FORMULA).a11y_label("a over b")),
    );
    runtime.pump().expect("the formula frame renders");

    let mathml = mathml::to_mathml(
        &latex::parse(FORMULA).expect("the fixture formula parses"),
        MathStyle::Text,
    );
    let update = runtime
        .board()
        .accessibility_tree()
        .expect("dew publishes an accessibility tree");
    let image = update
        .nodes
        .iter()
        .find_map(|(_, node)| (node.role() == Role::Image).then_some(node))
        .expect("the formula publishes an image node");
    assert_eq!(image.label(), Some("a over b"));
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some(mathml.as_str())),
        "the content's own description is replaced, not announced alongside"
    );
}

/// A name on a parent scope belongs to the node that represents the wrapped
/// view, and is said exactly once.
///
/// Padding, a frame, a background: the wrappers between `.a11y_label(..)` and
/// the control it names publish nothing themselves, so the control is what the
/// name is about. Hydrolysis expresses this as the naming scope being claimed
/// by the node that represents the view, with everything below it silent; dew
/// claims the same scope at the one funnel every node registers through.
#[test]
fn a_label_on_a_parent_scope_names_the_control_it_wraps_once() {
    let mut runtime = DewRuntime::new(
        HostBoard::new(240, 96),
        support::test_environment(),
        16,
        || {
            AnyView::new(
                button("Save")
                    .action(|| {})
                    .padding()
                    .a11y_label("Save draft"),
            )
        },
    );
    runtime.pump().expect("the button frame renders");

    let update = runtime
        .board()
        .accessibility_tree()
        .expect("dew publishes an accessibility tree");
    let named = update
        .nodes
        .iter()
        .filter(|(_, node)| node.label() == Some("Save draft"))
        .collect::<Vec<_>>();
    assert_eq!(
        named.len(),
        1,
        "the scope names one node, not every node under it"
    );
    assert_eq!(named[0].1.role(), Role::Button);
}

/// A reactive label follows its signal without rebuilding the subtree.
///
/// The label is captured once, when the node is built, and re-read at each
/// flush — the same contract every other dew signal holds — so a name derived
/// from application state stays current on a backend whose whole point is not
/// rebuilding what did not change.
#[test]
fn a_reactive_label_republishes_when_its_signal_changes() {
    let unread = binding(3_i32);
    let observed = unread.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(240, 64),
        support::test_environment(),
        16,
        move || {
            let label = unread.map(|count| Str::from(format!("{count} unread")));
            AnyView::new(text("Inbox").a11y_label(label))
        },
    );
    runtime.pump().expect("the first frame renders");
    assert!(
        runtime
            .board()
            .accessibility_tree()
            .expect("dew publishes an accessibility tree")
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("3 unread"))
    );

    observed.set(4);
    runtime
        .pump()
        .expect("a label change asks for one more frame");
    assert!(
        runtime
            .board()
            .accessibility_tree()
            .expect("dew publishes an accessibility tree")
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("4 unread")),
        "the retained node re-reads the label it captured"
    );
}

/// An automation identifier reaches the same node the label does.
///
/// `.a11y_id(..)` is what `waterui-testing` selectors and the native automation
/// frameworks match on; it is never spoken, so it fills in beside the name
/// rather than replacing it.
#[test]
fn an_automation_identifier_lands_on_the_named_node() {
    let ready = binding(false);
    let mut runtime = DewRuntime::new(
        HostBoard::new(240, 64),
        support::test_environment(),
        16,
        move || AnyView::new(toggle("Wi-Fi", &ready).a11y_id("settings.wifi")),
    );
    runtime.pump().expect("the toggle frame renders");

    let update = runtime
        .board()
        .accessibility_tree()
        .expect("dew publishes an accessibility tree");
    let switch = update
        .nodes
        .iter()
        .find_map(|(_, node)| (node.role() == Role::Switch).then_some(node))
        .expect("the toggle publishes a switch node");
    assert_eq!(switch.author_id(), Some("settings.wifi"));
    assert_eq!(
        switch.label(),
        Some("Wi-Fi"),
        "an identifier is not a name: the control keeps the one it had"
    );
}
