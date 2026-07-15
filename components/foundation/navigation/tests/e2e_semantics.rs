//! End-to-end semantic tests for navigation components.

use core::convert::TryFrom;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use waterui::ViewExt as _;
use waterui::id::Id;
use waterui::layout::stack::vstack;
use waterui::text::Text;
use waterui::{Binding, Environment, View};
use waterui_navigation::tab::{Tab, Tabs};
use waterui_navigation::{
    NavigationLink, NavigationPath, NavigationSplitView, NavigationStack, NavigationView,
};
use waterui_testing::{Role, Selector, SemanticApp, ui};

#[derive(Clone, Debug, PartialEq, Eq)]
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
            Tab::new(home_tab_id(), "Home Tab", || {
                NavigationView::new("Home", Text::new("home content"))
            }),
            Tab::new(settings_tab_id(), "Settings Tab", || {
                NavigationView::new("Settings", Text::new("settings content"))
            }),
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
    path_stack_nested_value_link_view_with_path(NavigationPath::new())
}

fn path_stack_nested_value_link_view_with_path(path: NavigationPath<TestRoute>) -> impl View {
    NavigationStack::with_path(
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

fn path_root_lifecycle_view(appeared: Rc<Cell<u32>>, disappeared: Rc<Cell<u32>>) -> impl View {
    NavigationStack::with_path(
        NavigationPath::<TestRoute>::new(),
        NavigationView::new(
            "Path Root",
            NavigationLink::value("Open Path Detail", TestRoute::First),
        )
        .on_navigation_appear(move || appeared.set(appeared.get() + 1))
        .on_navigation_disappear(move || disappeared.set(disappeared.get() + 1)),
    )
    .destination(|route| match route {
        TestRoute::First => NavigationView::new("Path Detail", Text::new("path detail content")),
        TestRoute::Second => unreachable!("this test never pushes the second route"),
    })
    .transition(waterui_navigation::navigation_transition::none())
}

fn restored_path_lifecycle_view(
    path: NavigationPath<TestRoute>,
    root_appeared: Rc<Cell<u32>>,
    destination_appeared: Rc<Cell<u32>>,
    destination_disappeared: Rc<Cell<u32>>,
    destination_popped: Rc<Cell<u32>>,
) -> impl View {
    NavigationStack::with_path(
        path,
        NavigationView::new("Restored Root", Text::new("restored root content"))
            .on_navigation_appear(move || root_appeared.set(root_appeared.get() + 1)),
    )
    .destination(move |route| {
        let appeared = Rc::clone(&destination_appeared);
        let disappeared = Rc::clone(&destination_disappeared);
        let popped = Rc::clone(&destination_popped);
        let (title, content) = match route {
            TestRoute::First => ("Restored First", "restored first content"),
            TestRoute::Second => ("Restored Second", "restored second content"),
        };
        NavigationView::new(title, Text::new(content))
            .on_navigation_appear(move || appeared.set(appeared.get() + 1))
            .on_navigation_disappear(move || disappeared.set(disappeared.get() + 1))
            .on_navigation_pop(move || popped.set(popped.get() + 1))
    })
    .transition(waterui_navigation::navigation_transition::none())
}

fn denied_pop_view(attempts: Rc<Cell<u32>>) -> impl View {
    NavigationStack::new(NavigationView::new(
        "Root",
        NavigationLink::new("Open Locked", move || {
            let attempts = Rc::clone(&attempts);
            NavigationView::new("Locked", Text::new("locked content"))
                .navigation_pop_enabled(false)
                .on_navigation_pop_attempted(move || attempts.set(attempts.get() + 1))
        }),
    ))
    .transition(waterui_navigation::navigation_transition::none())
}

fn lifecycle_view(
    appeared: Rc<Cell<u32>>,
    disappeared: Rc<Cell<u32>>,
    popped: Rc<Cell<u32>>,
) -> impl View {
    NavigationStack::new(NavigationView::new(
        "Root",
        NavigationLink::new("Open Lifecycle", move || {
            let appeared = Rc::clone(&appeared);
            let disappeared = Rc::clone(&disappeared);
            let popped = Rc::clone(&popped);
            NavigationView::new("Lifecycle", Text::new("lifecycle content"))
                .on_navigation_appear(move || appeared.set(appeared.get() + 1))
                .on_navigation_disappear(move || disappeared.set(disappeared.get() + 1))
                .on_navigation_pop(move || popped.set(popped.get() + 1))
        }),
    ))
    .transition(waterui_navigation::navigation_transition::none())
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
fn native_back_updates_the_explicit_navigation_path() {
    let path = NavigationPath::<TestRoute>::new();
    let mounted_path = path.clone();
    let mut app =
        mount_view(move || path_stack_nested_value_link_view_with_path(mounted_path.clone()));

    assert!(app.query().role(Role::BUTTON).label("Open First").tap());
    assert!(app.query().role(Role::BUTTON).label("Open Second").tap());
    assert_eq!(path.snapshot(), vec![TestRoute::First, TestRoute::Second]);

    assert!(app.query().role(Role::BUTTON).label("Back").tap());
    assert_eq!(path.snapshot(), vec![TestRoute::First]);
    app.query()
        .role(Role::BUTTON)
        .label("Open Second")
        .assert_exists();
}

#[test]
fn explicit_path_root_keeps_native_chrome_and_lifecycle() {
    let appeared = Rc::new(Cell::new(0));
    let disappeared = Rc::new(Cell::new(0));
    let mounted_appeared = Rc::clone(&appeared);
    let mounted_disappeared = Rc::clone(&disappeared);
    let mut app = mount_view(move || {
        path_root_lifecycle_view(
            Rc::clone(&mounted_appeared),
            Rc::clone(&mounted_disappeared),
        )
    });

    app.query().label("Path Root").assert_exists();
    assert_eq!(appeared.get(), 1);
    assert_eq!(disappeared.get(), 0);

    assert!(
        app.query()
            .role(Role::BUTTON)
            .label("Open Path Detail")
            .tap()
    );
    assert_eq!(appeared.get(), 1);
    assert_eq!(disappeared.get(), 1);
    app.query()
        .role(Role::LABEL)
        .label("path detail content")
        .assert_exists();

    assert!(app.query().role(Role::BUTTON).label("Back").tap());
    assert_eq!(appeared.get(), 2);
    assert_eq!(disappeared.get(), 1);
    app.query().label("Path Root").assert_exists();
}

#[test]
fn restored_path_and_atomic_multi_pop_have_exact_lifecycle() {
    let path = NavigationPath::from_iter([TestRoute::First, TestRoute::Second]);
    let mounted_path = path.clone();
    let root_appeared = Rc::new(Cell::new(0));
    let destination_appeared = Rc::new(Cell::new(0));
    let destination_disappeared = Rc::new(Cell::new(0));
    let destination_popped = Rc::new(Cell::new(0));
    let mounted_root_appeared = Rc::clone(&root_appeared);
    let mounted_destination_appeared = Rc::clone(&destination_appeared);
    let mounted_destination_disappeared = Rc::clone(&destination_disappeared);
    let mounted_destination_popped = Rc::clone(&destination_popped);
    let mut app = mount_view(move || {
        restored_path_lifecycle_view(
            mounted_path.clone(),
            Rc::clone(&mounted_root_appeared),
            Rc::clone(&mounted_destination_appeared),
            Rc::clone(&mounted_destination_disappeared),
            Rc::clone(&mounted_destination_popped),
        )
    });

    app.query()
        .role(Role::LABEL)
        .label("restored second content")
        .assert_exists();
    assert_eq!(root_appeared.get(), 0);
    assert_eq!(destination_appeared.get(), 1);
    assert_eq!(destination_disappeared.get(), 0);
    assert_eq!(destination_popped.get(), 0);

    path.clear();
    assert_eq!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::LABEL)
                    .label("restored root content"),
            )],
            waterui_testing::WaitOptions::new(Duration::from_millis(200)),
        ),
        waterui_testing::WaitResult::Completed,
        "atomic path removal should return to the root"
    );
    assert_eq!(root_appeared.get(), 1);
    assert_eq!(destination_appeared.get(), 1);
    assert_eq!(destination_disappeared.get(), 1);
    assert_eq!(destination_popped.get(), 2);
}

#[test]
fn denied_pop_reports_attempt_and_keeps_destination_active() {
    let attempts = Rc::new(Cell::new(0));
    let mounted_attempts = Rc::clone(&attempts);
    let mut app = mount_view(move || denied_pop_view(Rc::clone(&mounted_attempts)));

    assert!(app.query().role(Role::BUTTON).label("Open Locked").tap());
    assert!(
        app.query().role(Role::BUTTON).label("Back").tap(),
        "the native back control should consume a denied attempt"
    );

    assert_eq!(attempts.get(), 1);
    app.query()
        .role(Role::LABEL)
        .label("locked content")
        .assert_exists();
}

#[test]
fn destination_lifecycle_distinguishes_disappear_from_completed_pop() {
    let appeared = Rc::new(Cell::new(0));
    let disappeared = Rc::new(Cell::new(0));
    let popped = Rc::new(Cell::new(0));
    let mounted_appeared = Rc::clone(&appeared);
    let mounted_disappeared = Rc::clone(&disappeared);
    let mounted_popped = Rc::clone(&popped);
    let mut app = mount_view(move || {
        lifecycle_view(
            Rc::clone(&mounted_appeared),
            Rc::clone(&mounted_disappeared),
            Rc::clone(&mounted_popped),
        )
    });

    assert!(app.query().role(Role::BUTTON).label("Open Lifecycle").tap());
    assert_eq!(appeared.get(), 1);
    assert_eq!(disappeared.get(), 0);
    assert_eq!(popped.get(), 0);

    assert!(app.query().role(Role::BUTTON).label("Back").tap());
    assert_eq!(disappeared.get(), 1);
    assert_eq!(popped.get(), 1);
}

#[test]
fn split_view_selection_switches_placeholder_to_detail() {
    let mut env = Environment::new();
    hydrolysis_m3::install(&mut env);
    let mut app = ui().environment(env).viewport(1_000, 844).mount(split_view);
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
