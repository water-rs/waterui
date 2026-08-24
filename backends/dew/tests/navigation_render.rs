//! Navigation rendered end to end: chrome, pushing, going back, and the
//! contract that presentation is instantaneous.
//!
//! Run with `--no-capture` to export `/tmp/waterui_dew_navigation.png` for
//! visual review.

use core::cell::Cell;
use std::rc::Rc;

use nami::binding;
use waterui::prelude::*;
use waterui_backend_core::input::TouchPhase;
use waterui_controls::button::button;
use waterui_controls::toggle::toggle;
use waterui_core::Str;
use waterui_dew::{DewRenderer, DewRuntime, HostBoard, PointerSample, render_view_png};
use waterui_navigation::{
    NavigationLink, NavigationPath, NavigationStack, NavigationTitleDisplayMode, NavigationToolbar,
    NavigationToolbarItem, NavigationToolbarPlacement, NavigationView,
};

mod support;

const WIDTH: u32 = 240;
const HEIGHT: u32 = 240;

/// Taps `(x, y)` and pumps the frame the tap produced, if any.
fn tap(runtime: &mut DewRuntime<HostBoard>, x: f64, y: f64) -> Option<waterui_dew::Frame> {
    runtime.board_mut().push_pointer(PointerSample {
        x,
        y,
        phase: TouchPhase::Started,
    });
    runtime.board_mut().push_pointer(PointerSample {
        x,
        y,
        phase: TouchPhase::Ended,
    });
    runtime.pump()
}

fn run(build: impl Fn() -> NavigationStack<(), ()> + 'static) -> DewRuntime<HostBoard> {
    let mut runtime = DewRuntime::new(
        HostBoard::new(WIDTH, HEIGHT),
        support::test_environment(),
        16,
        move || AnyView::new(build()),
    );
    runtime.pump().expect("the first frame renders");
    runtime
}

/// A stack renders its root destination's content under the bar, and the bar
/// is drawn as chrome above it.
#[test]
fn a_stack_draws_its_root_under_a_bar() {
    let png = render_view_png(
        || NavigationStack::new(NavigationView::new("Settings", Color::red())),
        support::test_environment(),
        WIDTH,
        HEIGHT,
    );
    let pixmap =
        vello_cpu::Pixmap::from_png(std::io::Cursor::new(png.as_slice())).expect("png decodes");
    let pixel = |x: usize, y: usize| {
        let p = pixmap.data()[y * WIDTH as usize + x];
        [p.r, p.g, p.b]
    };
    let content = pixel(120, 200);
    assert!(
        content[0] > 150 && content[2] < 100,
        "the destination's content fills the area under the bar, got {content:?}"
    );
    let bar = pixel(120, 4);
    assert!(
        !(bar[0] > 150 && bar[2] < 100),
        "the bar covers the top strip rather than the content, got {bar:?}"
    );
}

/// Following a link pushes: the destination replaces the root's content, the
/// push costs exactly one frame, and no animation frame follows it.
#[test]
fn a_push_presents_in_one_frame_and_nothing_follows() {
    let mut runtime = run(|| {
        NavigationStack::new(NavigationView::new(
            "Root",
            NavigationLink::new("Open", || NavigationView::new("Detail", Color::blue())),
        ))
    });
    let frame = tap(&mut runtime, 120.0, 160.0).expect("following a link renders a frame");
    assert!(
        !frame.dirty.is_empty(),
        "the pushed destination has to reach the panel"
    );
    assert!(
        runtime.pump().is_none(),
        "presentation is instantaneous: a push leaves no animation running"
    );
}

/// Going back restores the previous destination — including the state it
/// owned, which is what retaining the covered entry is for.
#[test]
fn going_back_restores_the_covered_destination_with_its_state() {
    let flag = binding(false);
    let observed = flag.clone();
    let mut runtime = run(move || {
        let flag = flag.clone();
        NavigationStack::new(NavigationView::new(
            "Root",
            vstack((
                toggle("Remembered", &flag),
                NavigationLink::new("Open", || NavigationView::new("Detail", Color::blue())),
            )),
        ))
    });
    // The toggle is the first row under the bar, the link the row below it.
    tap(&mut runtime, 210.0, 43.0);
    assert!(observed.get(), "the toggle flips on the root screen");
    tap(&mut runtime, 120.0, 84.0);
    // Back is the leading item of the pushed destination's bar.
    let frame = tap(&mut runtime, 30.0, 14.0).expect("going back renders a frame");
    assert!(
        observed.get(),
        "the root's state survives being covered and uncovered"
    );
    // The root was retained, not rebuilt: a rebuilt one would have to shape
    // "Remembered" and "Open" again, and this frame shapes nothing.
    assert_eq!(
        frame.work.text_layouts_shaped, 0,
        "the uncovered root re-uses the text it shaped before it was covered"
    );
    assert!(runtime.pump().is_none(), "a pop is instantaneous too");
}

/// A destination that refuses to be popped stays put, and still hears about
/// the attempt.
#[test]
fn a_destination_can_refuse_a_pop() {
    let attempts = Rc::new(Cell::new(0));
    let counted = Rc::clone(&attempts);
    let mut runtime = run(move || {
        let counted = Rc::clone(&counted);
        NavigationStack::new(NavigationView::new(
            "Root",
            NavigationLink::new("Open", move || {
                let counted = Rc::clone(&counted);
                NavigationView::new("Detail", Color::blue())
                    .navigation_pop_enabled(false)
                    .on_navigation_pop_attempted(move || counted.set(counted.get() + 1))
            }),
        ))
    });
    tap(&mut runtime, 120.0, 160.0);
    tap(&mut runtime, 30.0, 14.0);
    assert_eq!(attempts.get(), 1, "the destination hears the attempt");
    // Still on the detail screen: its own back affordance is still there to
    // press, and pressing it reports another attempt.
    tap(&mut runtime, 30.0, 14.0);
    assert_eq!(attempts.get(), 2, "the refused pop left the stack alone");
}

/// A path-backed stack is driven by its path, not by the controller alone:
/// pushing a route presents its destination, and the back affordance has to
/// shorten the path itself — popping only the controller's own count would
/// leave the path claiming a screen that is no longer on the panel.
#[test]
fn a_path_backed_stack_pushes_and_pops_through_its_path() {
    let path: NavigationPath<u32> = NavigationPath::new();
    let observed = path.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(WIDTH, HEIGHT),
        support::test_environment(),
        16,
        {
            let path = path.clone();
            move || {
                AnyView::new(
                    NavigationStack::with_path(
                        path.clone(),
                        NavigationView::new("Rooms", Color::red()),
                    )
                    .destination(|room: u32| {
                        NavigationView::new(text!("Room {room}"), Color::blue())
                    }),
                )
            }
        },
    );
    runtime.pump().expect("the first frame renders");

    path.push(7);
    assert!(
        runtime.pump().is_some(),
        "pushing a route presents its destination"
    );
    assert_eq!(observed.len(), 1, "the path holds the pushed route");
    assert!(
        runtime.pump().is_none(),
        "and presenting it leaves nothing running"
    );

    // Back is the leading item of the pushed destination's bar.
    tap(&mut runtime, 30.0, 14.0);
    assert_eq!(
        observed.len(),
        0,
        "going back shortens the path that owns the entry"
    );
}

/// A stack built over a path that already holds routes — a session restored
/// at boot — presents the top one on its first frame.
#[test]
fn a_stack_opens_on_the_route_its_path_already_holds() {
    let path: NavigationPath<u32> = NavigationPath::new();
    path.push(4);
    let observed = path.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(WIDTH, HEIGHT),
        support::test_environment(),
        16,
        move || {
            AnyView::new(
                NavigationStack::with_path(
                    path.clone(),
                    NavigationView::new("Rooms", Color::red()),
                )
                .destination(|room: u32| NavigationView::new(text!("Room {room}"), Color::blue())),
            )
        },
    );
    runtime.pump().expect("the first frame renders");
    assert!(
        runtime.pump().is_none(),
        "the restored route is already on screen after the first frame"
    );
    // The restored destination is a pushed one, so it carries a back
    // affordance that shortens the path.
    tap(&mut runtime, 30.0, 14.0);
    assert_eq!(observed.len(), 0, "going back leaves the root");
}

/// A destination's whole bar reaches the panel: a trailing item on the item
/// row, a large title on its own row beneath it, a search field under that,
/// and a bottom-bar item at the foot of the screen.
#[test]
fn a_bar_places_every_part_it_declares() {
    let query = binding(Str::from("Bed"));
    let mut renderer = DewRenderer::default();
    let list = renderer.render_tree(
        AnyView::new(NavigationStack::new(
            NavigationView::new("Rooms", vstack((text("Kitchen"), text("Bedroom"))))
                .searchable(&query, "Find a room")
                .navigation_toolbar(NavigationToolbar::new(vec![
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarTrailing,
                        button("Add").action(|| {}),
                    ),
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::BottomBar,
                        button("Edit").action(|| {}),
                    ),
                ]))
                .navigation_title_display_mode(NavigationTitleDisplayMode::Large),
        )),
        &support::test_environment(),
        f64::from(WIDTH),
        f64::from(HEIGHT),
    );
    let commands = list.commands();
    let height = f64::from(HEIGHT);
    let width = f64::from(WIDTH);

    assert!(
        commands.iter().any(|placed| {
            let bounds = placed.bounds();
            bounds.y0 > height - 48.0 && bounds.width() > width - 4.0
        }),
        "the bottom bar covers the foot of the screen"
    );
    assert!(
        commands.iter().any(|placed| {
            let bounds = placed.bounds();
            bounds.y0 > height - 44.0 && bounds.x0 > 40.0 && bounds.x1 < width - 40.0
        }),
        "the bottom-bar item is drawn inside it"
    );
    assert!(
        commands.iter().any(|placed| {
            let bounds = placed.bounds();
            bounds.y0 < 36.0 && bounds.x1 > width - 12.0 && bounds.width() < width / 2.0
        }),
        "the trailing toolbar item sits at the right of the item row"
    );
    assert!(
        commands.iter().any(|placed| {
            let bounds = placed.bounds();
            bounds.y0 > 36.0 && bounds.y1 < 80.0 && bounds.x0 < 20.0
        }),
        "a large title takes its own row beneath the items, aligned to the leading edge"
    );
    assert!(
        commands.iter().any(|placed| {
            let bounds = placed.bounds();
            bounds.y0 > 60.0 && bounds.y1 < 110.0 && bounds.width() > width - 24.0
        }),
        "the search field spans the bar beneath the title"
    );
}

/// Visual review artifact: a destination with a bar and content, then the
/// screen a link pushes — the one that carries the back affordance.
#[test]
fn export_navigation_for_visual_review() {
    let mut runtime = run(|| {
        NavigationStack::new(NavigationView::new(
            "Thermostat",
            vstack((
                text("Living room"),
                text("21.5 °C"),
                NavigationLink::new("Schedule", || {
                    NavigationView::new("Schedule", vstack((text("Weekdays"), text("06:30"))))
                }),
            )),
        ))
    });
    std::fs::write(
        "/tmp/waterui_dew_navigation_root.png",
        runtime.board().framebuffer().to_png(),
    )
    .expect("export the root screen");
    tap(&mut runtime, 120.0, 100.0).expect("following the link renders a frame");
    std::fs::write(
        "/tmp/waterui_dew_navigation_pushed.png",
        runtime.board().framebuffer().to_png(),
    )
    .expect("export the pushed screen");
}
