//! End-to-end accessibility tree publication and action routing.

use core::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Role, TreeId};
use nami::binding;
use waterui::prelude::Color;
use waterui_controls::button::button;
use waterui_core::AnyView;
use waterui_dew::{DewRuntime, HostBoard};
use waterui_navigation::{NavigationStack, NavigationView, Tab, Tabs};

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
