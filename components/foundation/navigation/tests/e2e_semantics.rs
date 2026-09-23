//! End-to-end semantic tests for navigation components.

use core::convert::TryFrom;
use std::time::Duration;

use waterui::ViewExt as _;
use waterui::id::{Id, TaggedView};
use waterui::layout::stack::vstack;
use waterui::text::Text;
use waterui::{AnyView, Binding, Environment, View};
use waterui_navigation::tab::{Tab, Tabs};
use waterui_navigation::{
    NavigationLink, NavigationPath, NavigationSplitView, NavigationStack, NavigationView,
};
use waterui_testing::{Role, Selector, SemanticApp, ui};

#[derive(Clone, PartialEq, Eq)]
enum TestRoute {
    First,
    Second,
}

fn mount_view<V, F>(build: F) -> SemanticApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let mut env = Environment::new();
    hydrolysis_m3::install(&mut env);
    ui().environment(env).mount(build)
}

fn home_tab_id() -> Id {
    Id::try_from(1).expect("test tab id must be non-zero")
}

fn settings_tab_id() -> Id {
    Id::try_from(2).expect("test tab id must be non-zero")
}

fn tabs_view() -> impl View {
    let selection = Binding::container(home_tab_id());

    Tabs::new(
        selection,
        vec![
            Tab::new(
                TaggedView::new(home_tab_id(), AnyView::new("Home Tab")),
                || NavigationView::new("Home", Text::new("home content")),
            ),
            Tab::new(
                TaggedView::new(settings_tab_id(), AnyView::new("Settings Tab")),
                || NavigationView::new("Settings", Text::new("settings content")),
            ),
        ],
    )
}

fn stack_view() -> impl View {
    NavigationStack::new(NavigationView::new(
        "Root",
        vstack((NavigationLink::new("Open Detail", || {
            NavigationView::new("Detail", Text::new("detail content"))
        }),)),
    ))
}

fn path_stack_nested_value_link_view() -> impl View {
    let path = NavigationPath::new();

    NavigationStack::with(
        path,
        NavigationView::new(
            "Root",
            vstack((NavigationLink::value("Open First", TestRoute::First),)),
        ),
    )
    .destination(|route| match route {
        TestRoute::First => NavigationView::new(
            "First",
            vstack((NavigationLink::value("Open Second", TestRoute::Second),)),
        ),
        TestRoute::Second => NavigationView::new("Second", Text::new("second content")),
    })
}

fn split_view() -> impl View {
    let selection = Binding::container(None::<i32>);

    NavigationSplitView::new(
        &selection,
        {
            let selection = selection.clone();
            move || {
                vstack((waterui::component::button("Select Detail")
                    .action(
                        |waterui::State(selection): waterui::State<Binding<Option<i32>>>| {
                            selection.set(Some(7));
                        },
                    )
                    .state(&selection),))
            }
        },
        |value| NavigationView::new("Detail", Text::new(format!("detail:{value}"))),
    )
    .placeholder(|| Text::new("placeholder content"))
}

#[test]
fn tabs_tap_switches_selection_and_content() {
    let mut app = mount_view(tabs_view);
    app.query().role(Role::TAB_LIST).assert_exists();
    app.query()
        .role(Role::TAB)
        .label("Home Tab")
        .assert_exists();
    app.query()
        .role(Role::TAB)
        .label("Settings Tab")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("home content")
        .assert_exists();
    assert!(
        app.query().role(Role::TAB).label("Settings Tab").tap(),
        "settings tab tap should succeed"
    );
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::TAB)
                    .label("Settings Tab")
                    .selected(true),
            )],
            waterui_testing::WaitOptions::new(Duration::from_millis(200)),
        ) == waterui_testing::WaitResult::Completed,
        "settings tab should become selected"
    );
    app.query()
        .role(Role::LABEL)
        .label("settings content")
        .assert_exists();
}

#[test]
fn navigation_link_push_and_back_pop_update_content() {
    let mut app = mount_view(stack_view);
    app.query()
        .role(Role::BUTTON)
        .label("Open Detail")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Open Detail").tap(),
        "navigation link tap should succeed"
    );
    app.query().role(Role::BUTTON).label("Back").assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("detail content")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Back").tap(),
        "back button tap should succeed"
    );
    assert!(
        app.wait_for(
            &[app.expect_exists(Selector::default().role(Role::BUTTON).label("Open Detail"),)],
            waterui_testing::WaitOptions::new(Duration::from_millis(200)),
        ) == waterui_testing::WaitResult::Completed,
        "root navigation content should return after back"
    );
}

#[test]
fn path_stack_keeps_value_links_active_inside_destination() {
    let mut app = mount_view(path_stack_nested_value_link_view);
    assert!(
        app.query().role(Role::BUTTON).label("Open First").tap(),
        "root value link tap should succeed"
    );
    app.query()
        .role(Role::BUTTON)
        .label("Open Second")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Open Second").tap(),
        "destination value link tap should succeed"
    );
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::LABEL)
                    .label("second content")
            )],
            waterui_testing::WaitOptions::new(Duration::from_millis(200)),
        ) == waterui_testing::WaitResult::Completed,
        "second destination content should appear after nested value link tap"
    );
}

#[test]
fn split_view_selection_switches_placeholder_to_detail() {
    let mut app = mount_view(split_view);
    app.query()
        .role(Role::BUTTON)
        .label("Select Detail")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("placeholder content")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Select Detail").tap(),
        "split sidebar action should succeed"
    );
    assert!(
        app.wait_for(
            &[app.expect_exists(Selector::default().role(Role::LABEL).label("detail:7"))],
            waterui_testing::WaitOptions::new(Duration::from_millis(200)),
        ) == waterui_testing::WaitResult::Completed,
        "split detail content should appear after selection"
    );
}
