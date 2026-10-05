//! System back against a rendered navigation stack.
//!
//! The semantic runtime never registers back targets — they are recorded
//! while a stack paints — so these tests drive [`crate::HeadlessRuntime`] and
//! read the accessibility tree the rendered frame emits.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, NodeId, Role, TreeId, TreeUpdate};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::stack::vstack;
use waterui_navigation::{NavigationLink, NavigationStack, NavigationView};
use waterui_text::text;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{BackEdge, BackNavigation, InputEvent};

const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;
const FRAME: Duration = Duration::from_millis(16);
/// Longer than the test theme's 450ms navigation transition, so one pump past
/// this covers a push, a pop, and an interactive completion from progress 0.
const TRANSITION: Duration = Duration::from_millis(700);

struct Clock {
    now: Instant,
}

fn mount(view: AnyView) -> (HeadlessRuntime, Clock) {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    let runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    (
        runtime,
        Clock {
            now: Instant::now(),
        },
    )
}

fn pump(runtime: &mut HeadlessRuntime, clock: &mut Clock) {
    clock.now += FRAME;
    let _ = runtime.pump_at(false, clock.now);
}

fn pump_for(runtime: &mut HeadlessRuntime, clock: &mut Clock, duration: Duration) {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration {
        pump(runtime, clock);
        elapsed += FRAME;
    }
}

fn pump_until_label(
    runtime: &mut HeadlessRuntime,
    clock: &mut Clock,
    role: Role,
    label: &str,
) -> TreeUpdate {
    for _ in 0..80 {
        pump(runtime, clock);
        if let Some(update) = runtime.accessibility_tree()
            && find_by_label(&update, role, label).is_some()
        {
            return update;
        }
    }
    panic!("navigation back test: {role:?} labelled {label:?} never appeared");
}

fn tree(runtime: &mut HeadlessRuntime) -> TreeUpdate {
    runtime
        .accessibility_tree()
        .expect("the rendered frame produced no accessibility tree")
}

fn click(runtime: &mut HeadlessRuntime, target: NodeId) {
    assert!(
        runtime.perform_accessibility_action(ActionRequest {
            action: Action::Click,
            target_node: target,
            target_tree: TreeId::ROOT,
            data: None,
        }),
        "the accessibility click changed nothing"
    );
}

fn back(runtime: &mut HeadlessRuntime, clock: &mut Clock, event: BackNavigation) {
    runtime.push_input_event(InputEvent::BackNavigation(event));
    pump(runtime, clock);
}

fn two_level_stack() -> AnyView {
    AnyView::new(NavigationStack::new(NavigationView::new(
        "Root",
        vstack((NavigationLink::new("Open Detail", || {
            NavigationView::new("Detail", text("detail content"))
        }),)),
    )))
}

fn open_detail(runtime: &mut HeadlessRuntime, clock: &mut Clock) {
    let update = pump_until_label(runtime, clock, Role::Button, "Open Detail");
    let (open, _) =
        find_by_label(&update, Role::Button, "Open Detail").expect("the link is missing");
    click(runtime, open);
    let _ = pump_until_label(runtime, clock, Role::Header, "Detail");
}

fn shows(update: &TreeUpdate, role: Role, label: &str) -> bool {
    find_by_label(update, role, label).is_some()
}

#[test]
fn navigation_back_gesture_pops_on_invoke() {
    let (mut runtime, mut clock) = mount(two_level_stack());
    let _ = pump_until_label(&mut runtime, &mut clock, Role::Header, "Root");
    assert!(
        !runtime.renderer().has_back_navigation_target(),
        "the root registers no back target"
    );

    open_detail(&mut runtime, &mut clock);
    assert!(
        runtime.renderer().has_back_navigation_target(),
        "a pushed page registers a back target"
    );

    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Started {
            edge: BackEdge::Left,
        },
    );
    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Progressed { progress: 0.35 },
    );
    let mid = tree(&mut runtime);
    assert!(
        shows(&mid, Role::Header, "Detail"),
        "progress alone must not pop"
    );

    back(&mut runtime, &mut clock, BackNavigation::Invoked);
    pump_for(&mut runtime, &mut clock, TRANSITION);
    let update = tree(&mut runtime);
    assert!(
        shows(&update, Role::Header, "Root"),
        "invoking the gesture must reveal the root"
    );
    assert!(
        !shows(&update, Role::Label, "detail content"),
        "invoking the gesture must remove the detail page"
    );
    assert!(
        !runtime.renderer().has_back_navigation_target(),
        "the root after the pop registers no back target"
    );
}

#[test]
fn navigation_back_gesture_cancel_keeps_the_page() {
    let (mut runtime, mut clock) = mount(two_level_stack());
    open_detail(&mut runtime, &mut clock);

    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Started {
            edge: BackEdge::Right,
        },
    );
    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Progressed { progress: 0.8 },
    );
    back(&mut runtime, &mut clock, BackNavigation::Cancelled);
    pump_for(&mut runtime, &mut clock, TRANSITION);

    let update = tree(&mut runtime);
    assert!(
        shows(&update, Role::Header, "Detail"),
        "a cancelled gesture must keep the detail page"
    );
    assert!(
        !shows(&update, Role::Button, "Open Detail"),
        "a cancelled gesture must not reveal the root"
    );
    assert!(runtime.renderer().has_back_navigation_target());
}

#[test]
fn navigation_back_button_predictive_start_pops() {
    let (mut runtime, mut clock) = mount(two_level_stack());
    open_detail(&mut runtime, &mut clock);

    // A back button's predictive animation reports no swipe edge, then commits.
    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Started {
            edge: BackEdge::None,
        },
    );
    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Progressed { progress: 0.2 },
    );
    back(&mut runtime, &mut clock, BackNavigation::Invoked);
    pump_for(&mut runtime, &mut clock, TRANSITION);

    let update = tree(&mut runtime);
    assert!(
        shows(&update, Role::Header, "Root"),
        "a back button's predictive gesture must pop to the root"
    );
    assert!(!runtime.renderer().has_back_navigation_target());
}

#[test]
fn navigation_back_invoke_alone_pops() {
    let (mut runtime, mut clock) = mount(two_level_stack());
    open_detail(&mut runtime, &mut clock);

    back(&mut runtime, &mut clock, BackNavigation::Invoked);
    pump_for(&mut runtime, &mut clock, TRANSITION);

    let update = tree(&mut runtime);
    assert!(
        shows(&update, Role::Header, "Root"),
        "a back button must pop to the root"
    );
    assert!(
        shows(&update, Role::Button, "Open Detail"),
        "the root page must return after the back button"
    );
    assert!(!runtime.renderer().has_back_navigation_target());
}

#[test]
fn navigation_back_refused_pop_keeps_the_page() {
    let attempts = Rc::new(Cell::new(0u32));
    let mounted = Rc::clone(&attempts);
    let (mut runtime, mut clock) = mount(AnyView::new(NavigationStack::new(NavigationView::new(
        "Root",
        vstack((NavigationLink::new("Open Locked", move || {
            let attempts = Rc::clone(&mounted);
            NavigationView::new("Locked", text("locked content"))
                .navigation_pop_enabled(false)
                .on_navigation_pop_attempted(move || attempts.set(attempts.get() + 1))
        }),)),
    ))));
    let update = pump_until_label(&mut runtime, &mut clock, Role::Button, "Open Locked");
    let (open, _) =
        find_by_label(&update, Role::Button, "Open Locked").expect("the link is missing");
    click(&mut runtime, open);
    let _ = pump_until_label(&mut runtime, &mut clock, Role::Header, "Locked");

    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Started {
            edge: BackEdge::Left,
        },
    );
    assert_eq!(
        attempts.get(),
        1,
        "a refused start still runs the pop policy"
    );
    back(
        &mut runtime,
        &mut clock,
        BackNavigation::Progressed { progress: 0.9 },
    );
    back(&mut runtime, &mut clock, BackNavigation::Invoked);
    pump_for(&mut runtime, &mut clock, TRANSITION);
    assert_eq!(
        attempts.get(),
        1,
        "the rest of a refused gesture must not pop"
    );
    let update = tree(&mut runtime);
    assert!(
        shows(&update, Role::Label, "locked content"),
        "a refused gesture must keep the page"
    );

    back(&mut runtime, &mut clock, BackNavigation::Invoked);
    pump_for(&mut runtime, &mut clock, TRANSITION);
    assert_eq!(
        attempts.get(),
        2,
        "a later back button runs the pop policy again"
    );
    let update = tree(&mut runtime);
    assert!(
        shows(&update, Role::Label, "locked content"),
        "a refused back button must keep the page"
    );
    assert!(
        !shows(&update, Role::Button, "Open Locked"),
        "a refused pop must not reveal the root"
    );
}

#[test]
fn navigation_back_available_only_above_the_root() {
    let (mut runtime, mut clock) = mount(two_level_stack());
    let _ = pump_until_label(&mut runtime, &mut clock, Role::Header, "Root");
    assert!(!runtime.renderer().has_back_navigation_target());

    open_detail(&mut runtime, &mut clock);
    assert!(runtime.renderer().has_back_navigation_target());

    back(&mut runtime, &mut clock, BackNavigation::Invoked);
    pump_for(&mut runtime, &mut clock, TRANSITION);
    let update = tree(&mut runtime);
    assert!(shows(&update, Role::Header, "Root"));
    assert!(!runtime.renderer().has_back_navigation_target());
}
