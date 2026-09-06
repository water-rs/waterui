//! Navigation rendered end to end: chrome, pushing, going back, and the
//! contract that presentation is instantaneous.
//!
//! Run with `--no-capture` to export `/tmp/waterui_dew_navigation.png` for
//! visual review.

use core::cell::Cell;
use std::rc::Rc;

use kurbo::Rect;
use nami::binding;
use peniko::Brush;
use waterui::Plugin as _;
use waterui::color::{ResolvedColor, Srgb};
use waterui::prelude::*;
use waterui::theme::{ColorSettings, Theme};
use waterui_backend_core::input::TouchPhase;
use waterui_controls::toggle::toggle;
use waterui_core::Str;
use waterui_dew::{DewRuntime, DisplayList, DrawCommand, HostBoard, PlacedCommand, PointerSample};
use waterui_navigation::{
    NavigationLink, NavigationPath, NavigationSplitView, NavigationStack,
    NavigationTitleDisplayMode, NavigationToolbar, NavigationToolbarItem,
    NavigationToolbarPlacement, NavigationView,
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

fn tap_labeled(runtime: &mut DewRuntime<HostBoard>, label: &str) -> Option<waterui_dew::Frame> {
    let bounds = runtime
        .board()
        .accessibility_tree()
        .expect("a rendered host frame publishes accessibility bounds")
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .unwrap_or_else(|| panic!("the visible control `{label}` has accessibility bounds"));
    tap(
        runtime,
        f64::midpoint(bounds.x0, bounds.x1),
        f64::midpoint(bounds.y0, bounds.y1),
    )
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

const fn solid_color(command: &PlacedCommand) -> Option<peniko::Color> {
    match command.command() {
        DrawCommand::FillPath {
            brush: Brush::Solid(color),
            ..
        } => Some(*color),
        _ => None,
    }
}

const fn exact_f64(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

const fn exact_f32(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

fn assert_exact_f64(left: f64, right: f64) {
    assert_eq!(left.to_bits(), right.to_bits());
}

fn solid_fill_bounds(list: &DisplayList, color: peniko::Color) -> Vec<Rect> {
    list.commands()
        .iter()
        .filter(|command| solid_color(command) == Some(color))
        .map(PlacedCommand::bounds)
        .collect()
}

fn display_srgb(red: u8, green: u8, blue: u8) -> peniko::Color {
    let resolved = ResolvedColor::from_srgb(Srgb::new_u8(red, green, blue));
    let srgb = resolved.to_srgb_with_headroom();
    peniko::Color::new([srgb.red, srgb.green, srgb.blue, resolved.opacity])
}

fn only_solid_fill(list: &DisplayList, color: peniko::Color) -> Rect {
    let bounds = solid_fill_bounds(list, color);
    assert_eq!(bounds.len(), 1, "expected exactly one {color:?} fill");
    bounds[0]
}

fn assert_root_background(list: &DisplayList) {
    let first = list
        .commands()
        .first()
        .expect("every Dew frame begins with its root background");
    assert!(solid_color(first).is_some());
    assert_eq!(
        first.bounds(),
        Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT))
    );
}

fn full_width_solid_fills(list: &DisplayList, width: f64) -> Vec<Rect> {
    list.commands()
        .iter()
        .skip(1)
        .filter_map(|command| {
            let bounds = command.bounds();
            (solid_color(command).is_some()
                && exact_f64(bounds.x0, 0.0)
                && exact_f64(bounds.x1, width))
            .then_some(bounds)
        })
        .collect()
}

/// A stack renders its root destination's content under the bar, and the bar
/// is drawn as chrome above it.
#[test]
fn a_stack_draws_its_root_under_a_bar() {
    let mut renderer = support::test_renderer();
    let list = renderer.render_tree(
        AnyView::new(NavigationStack::new(NavigationView::new(
            "Settings",
            Color::srgb(255, 0, 0),
        ))),
        &support::test_environment(),
        f64::from(WIDTH),
        f64::from(HEIGHT),
    );
    assert_root_background(&list);
    let content = only_solid_fill(&list, display_srgb(255, 0, 0));
    let bars = full_width_solid_fills(&list, f64::from(WIDTH))
        .into_iter()
        .filter(|bounds| bounds.height() > 1.0 && *bounds != content)
        .collect::<Vec<_>>();
    let [bar] = bars.as_slice() else {
        panic!("navigation emits exactly one top surface")
    };
    assert_exact_f64(bar.x0, 0.0);
    assert_exact_f64(bar.y0, 0.0);
    assert_exact_f64(bar.x1, f64::from(WIDTH));
    assert_eq!(
        content,
        Rect::new(0.0, bar.y1, f64::from(WIDTH), f64::from(HEIGHT))
    );
}

/// The backend owns the window background, and the retained root observes the
/// theme signal without rebuilding its view tree.
#[test]
fn root_background_tracks_the_dynamic_theme() {
    let background = binding(ResolvedColor::from_srgb(Srgb::new(1.0, 0.0, 0.0)));
    let mut environment = support::test_environment();
    Theme::new()
        .colors(ColorSettings::new().background(background.clone()))
        .install(&mut environment);
    let mut runtime = DewRuntime::new(HostBoard::new(WIDTH, HEIGHT), environment, 16, || {
        AnyView::new(())
    });
    runtime.pump().expect("the red background renders");
    assert_eq!(runtime.board().framebuffer().pixel(0, 0), [255, 0, 0, 255]);
    assert_eq!(
        runtime.board().framebuffer().pixel(WIDTH - 1, HEIGHT - 1),
        [255, 0, 0, 255]
    );

    background.set(ResolvedColor::from_srgb(Srgb::new(0.0, 0.0, 1.0)));
    runtime
        .pump()
        .expect("changing the background signal renders one new frame");
    assert_eq!(runtime.board().framebuffer().pixel(0, 0), [0, 0, 255, 255]);
    assert_eq!(
        runtime.board().framebuffer().pixel(WIDTH - 1, HEIGHT - 1),
        [0, 0, 255, 255]
    );
    assert!(
        runtime.pump().is_none(),
        "the theme update settles in one frame"
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
    tap_labeled(&mut runtime, "Remembered");
    assert!(observed.get(), "the toggle flips on the root screen");
    tap_labeled(&mut runtime, "Open");
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
    let mut renderer = support::test_renderer();
    let list = renderer.render_tree(
        AnyView::new(NavigationStack::new(
            NavigationView::new("Rooms", Color::srgb(0, 255, 255))
                .searchable(&query, "Find a room")
                .navigation_toolbar(NavigationToolbar::new(vec![
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarTrailing,
                        Color::srgb(255, 0, 255).width(12.0).height(8.0),
                    ),
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::BottomBar,
                        Color::srgb(255, 255, 0).width(14.0).height(6.0),
                    ),
                ]))
                .navigation_title_display_mode(NavigationTitleDisplayMode::Large),
        )),
        &support::test_environment(),
        f64::from(WIDTH),
        f64::from(HEIGHT),
    );
    let height = f64::from(HEIGHT);
    let width = f64::from(WIDTH);
    assert_root_background(&list);

    let content = only_solid_fill(&list, display_srgb(0, 255, 255));
    let mut bars: Vec<Rect> = full_width_solid_fills(&list, width)
        .into_iter()
        .filter(|bounds| bounds.height() > 1.0 && *bounds != content)
        .collect();
    bars.sort_by(|left, right| left.y0.total_cmp(&right.y0));
    let [top_bar, bottom_bar] = bars.as_slice() else {
        panic!("navigation emits exactly one top and one bottom surface")
    };
    assert_exact_f64(top_bar.y0, 0.0);
    assert_exact_f64(bottom_bar.y1, height);

    assert_eq!(content, Rect::new(0.0, top_bar.y1, width, bottom_bar.y0));

    let trailing = only_solid_fill(&list, display_srgb(255, 0, 255));
    assert_exact_f64(trailing.width(), 12.0);
    assert_exact_f64(trailing.height(), 8.0);
    assert_exact_f64(trailing.x1, width - 8.0);
    assert_eq!(top_bar.intersect(trailing), trailing);

    let bottom_item = only_solid_fill(&list, display_srgb(255, 255, 0));
    assert_exact_f64(bottom_item.width(), 14.0);
    assert_exact_f64(bottom_item.height(), 6.0);
    assert_exact_f64(bottom_item.center().x, bottom_bar.center().x);
    assert_eq!(bottom_bar.intersect(bottom_item), bottom_item);

    let border: Vec<Rect> = full_width_solid_fills(&list, width)
        .into_iter()
        .filter(|bounds| exact_f64(bounds.height(), 1.0))
        .collect();
    assert_eq!(
        border,
        vec![
            Rect::new(0.0, top_bar.y1 - 1.0, width, top_bar.y1),
            Rect::new(0.0, bottom_bar.y0, width, bottom_bar.y0 + 1.0),
        ]
    );

    let title_sizes: Vec<f32> = list
        .commands()
        .iter()
        .filter_map(|placed| match placed.command() {
            DrawCommand::GlyphRun { font_size, .. } => Some(*font_size),
            _ => None,
        })
        .collect();
    let title = list
        .commands()
        .iter()
        .find(|placed| {
            matches!(placed.command(), DrawCommand::GlyphRun { font_size, .. } if exact_f32(*font_size, 24.0))
        })
        .unwrap_or_else(|| panic!("the large title uses the configured title font: {title_sizes:?}"));
    assert_eq!(top_bar.intersect(title.bounds()), title.bounds());

    let search = list
        .commands()
        .iter()
        .filter(|command| solid_color(command).is_some())
        .map(PlacedCommand::bounds)
        .find(|bounds| exact_f64(bounds.x0, 8.0) && exact_f64(bounds.x1, width - 8.0))
        .expect("the search field owns the exact inset bar width");
    assert_eq!(top_bar.intersect(search), search);
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
        support::export_path("waterui_dew_navigation_root.png"),
        runtime.board().framebuffer().to_png(),
    )
    .expect("export the root screen");
    tap_labeled(&mut runtime, "Schedule").expect("following the link renders a frame");
    std::fs::write(
        support::export_path("waterui_dew_navigation_pushed.png"),
        runtime.board().framebuffer().to_png(),
    )
    .expect("export the pushed screen");
}

/// A two-column split retains each opened detail rather than rebuilding it
/// after the selection passes through the placeholder.
#[test]
fn a_split_retains_details_across_selection_round_trips() {
    let selection = binding(None::<u32>);
    let observed = selection.clone();
    let builds = Rc::new(Cell::new(0));
    let counted = Rc::clone(&builds);
    let mut runtime = DewRuntime::new(
        HostBoard::new(600, HEIGHT),
        support::test_environment(),
        16,
        move || {
            let counted = Rc::clone(&counted);
            AnyView::new(NavigationSplitView::new(
                &selection,
                Color::red(),
                move |room: u32| {
                    counted.set(counted.get() + 1);
                    NavigationView::new(text!("Room {room}"), Color::blue())
                },
            ))
        },
    );
    runtime.pump().expect("the split's first frame renders");

    observed.set(Some(7));
    runtime
        .pump()
        .expect("selecting a detail renders one frame");
    observed.set(None);
    runtime
        .pump()
        .expect("clearing selection shows the placeholder");
    observed.set(Some(7));
    runtime
        .pump()
        .expect("reselecting the retained detail renders one frame");

    assert_eq!(builds.get(), 1, "the detail node is retained by its id");
    assert!(
        runtime.pump().is_none(),
        "split selection changes are instantaneous"
    );
}

/// A split narrower than two declared minimum columns becomes a hierarchy;
/// its contextual back action clears selection and returns to the primary.
#[test]
fn a_compact_split_navigates_back_to_its_primary() {
    let selection = binding(None::<u32>);
    let observed = selection.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(200, HEIGHT),
        support::test_environment(),
        16,
        move || {
            AnyView::new(NavigationSplitView::new(
                &selection,
                Color::red(),
                |room: u32| NavigationView::new(text!("Room {room}"), Color::blue()),
            ))
        },
    );
    runtime.pump().expect("the compact primary renders");
    observed.set(Some(3));
    runtime.pump().expect("the compact detail renders");

    tap(&mut runtime, 24.0, 14.0).expect("the contextual back action renders the primary");
    assert_eq!(
        observed.get(),
        None,
        "back clears the selection that presented the detail"
    );
}

/// Three-column splits retain middle and detail destinations independently.
#[test]
fn a_three_column_split_retains_both_destination_levels() {
    let primary = binding(Some(1_u32));
    let secondary = binding(Some(10_u32));
    let observed_primary = primary.clone();
    let observed_secondary = secondary.clone();
    let content_builds = Rc::new(Cell::new(0));
    let detail_builds = Rc::new(Cell::new(0));
    let counted_content = Rc::clone(&content_builds);
    let counted_detail = Rc::clone(&detail_builds);
    let mut runtime = DewRuntime::new(
        HostBoard::new(900, HEIGHT),
        support::test_environment(),
        16,
        move || {
            let counted_content = Rc::clone(&counted_content);
            let counted_detail = Rc::clone(&counted_detail);
            AnyView::new(NavigationSplitView::three_column(
                &primary,
                &secondary,
                Color::red(),
                move |section: u32| {
                    counted_content.set(counted_content.get() + 1);
                    NavigationView::new(text!("Section {section}"), Color::green())
                },
                move |item: u32| {
                    counted_detail.set(counted_detail.get() + 1);
                    NavigationView::new(text!("Item {item}"), Color::blue())
                },
            ))
        },
    );
    runtime.pump().expect("all three columns render");
    std::fs::write(
        support::export_path("waterui_dew_navigation_split.png"),
        runtime.board().framebuffer().to_png(),
    )
    .expect("export split visual review PNG");
    assert_eq!(content_builds.get(), 1);
    assert_eq!(detail_builds.get(), 1);

    observed_primary.set(Some(2));
    runtime.pump().expect("the new middle destination renders");
    observed_primary.set(Some(1));
    runtime
        .pump()
        .expect("the retained middle destination returns");
    observed_secondary.set(None);
    runtime.pump().expect("the detail placeholder renders");
    observed_secondary.set(Some(10));
    runtime.pump().expect("the retained detail returns");

    assert_eq!(content_builds.get(), 2, "each middle id is built once");
    assert_eq!(detail_builds.get(), 1, "the detail id is built once");
}
