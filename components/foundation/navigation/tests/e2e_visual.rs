//! Offscreen visual stages for retained navigation motion and native-style chrome.

use hydrolysis_m3::install as install_m3;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use waterui::layout::stack::vstack;
use waterui::text::Text;
use waterui_navigation::{NavigationLink, NavigationStack, NavigationView};
use waterui_testing::{Role, ui};

const VIEWPORT_WIDTH: u16 = 390;
const VIEWPORT_HEIGHT: u16 = 844;

fn visual_stack(destination_appeared: Rc<Cell<bool>>) -> impl waterui::View {
    NavigationStack::new(NavigationView::new(
        "Library",
        vstack((
            Text::new("Recently viewed"),
            NavigationLink::new("Open Atlas", move || {
                let destination_appeared = Rc::clone(&destination_appeared);
                NavigationView::new("Atlas", Text::new("Atlas destination"))
                    .on_navigation_appear(move || destination_appeared.set(true))
            }),
        )),
    ))
}

#[test]
fn navigation_stack_exports_root_destination_and_interactive_pop_stages() {
    let destination_appeared = Rc::new(Cell::new(false));
    let destination_appeared_for_view = Rc::clone(&destination_appeared);
    let mut app = ui()
        .viewport(VIEWPORT_WIDTH.into(), VIEWPORT_HEIGHT.into())
        .theme(install_m3)
        .mount(move || visual_stack(Rc::clone(&destination_appeared_for_view)));

    app.query().label("Library").assert_exists();
    let _root = app.capture_snapshot("navigation", "stack", "root");

    assert!(
        app.query().role(Role::BUTTON).label("Open Atlas").tap(),
        "navigation link should open the visual destination"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while !destination_appeared.get() {
        assert!(
            Instant::now() < deadline,
            "navigation destination did not finish appearing"
        );
        let _ = app.snapshot();
    }
    let _destination = app.capture_snapshot("navigation", "stack", "destination");

    app.queue_pointer_down(2.0, f32::from(VIEWPORT_HEIGHT) * 0.5);
    let _ = app.snapshot();
    app.queue_pointer_move(
        f32::from(VIEWPORT_WIDTH) * 0.45,
        f32::from(VIEWPORT_HEIGHT) * 0.5,
    );
    let _interactive = app.capture_snapshot("navigation", "stack", "interactive-pop-progress");
    app.queue_pointer_up(
        f32::from(VIEWPORT_WIDTH) * 0.45,
        f32::from(VIEWPORT_HEIGHT) * 0.5,
    );
    let _ = app.snapshot();
}
