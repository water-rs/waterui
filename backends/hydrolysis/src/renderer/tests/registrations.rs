//! Retained-registration regressions from the per-owner registry review.
//!
//! Each registration's lifetime is its recording owner's: a record that
//! leaves one of its sub-views unplaced retires that subtree's targets and
//! occluders, and a frame's materialized lists carry nothing a hidden tab,
//! a closed overlay or an inactive navigation page once registered. The
//! last test pins the materialized paint order to the ranking dev's single
//! per-frame counter produced.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::Role;
use nami::Binding;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::prelude::ContextMenu;
use waterui::{AnyView, ViewExt as _};
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::Native;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::{Id, SelfId};
use waterui_layout::frame::Frame;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::vstack;
use waterui_navigation::tab::{Tab, TabsLayout};
use waterui_navigation::{NavigationLink, NavigationStack, NavigationView};

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::platform_view::{PlatformView, PlatformViewSink};

const WINDOW: (u32, u32) = (400, 700);
const FRAME: Duration = Duration::from_millis(16);
/// Longer than the test theme's 450ms navigation transition.
const TRANSITION: Duration = Duration::from_millis(700);

fn primary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    ]
}

fn secondary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
    ]
}

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    )
}

fn pump_until_settled(runtime: &mut HeadlessRuntime) {
    let mut at = Instant::now();
    for _ in 0..128 {
        at += FRAME;
        let _ = runtime.pump_at(false, at);
        if runtime.is_settled() && !runtime.has_pending_semantic_update() {
            break;
        }
    }
}

fn bounds_of(runtime: &mut HeadlessRuntime, label: &str) -> accesskit::Rect {
    pump_until_settled(runtime);
    runtime
        .accessibility_tree()
        .as_ref()
        .and_then(|update| find_by_label(update, Role::Button, label))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("{label} must emit a button with bounds"))
}

fn midpoint(bounds: accesskit::Rect) -> (f32, f32) {
    (
        crate::num_cast::f64_as_f32(f64::midpoint(bounds.x0, bounds.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(bounds.y0, bounds.y1)),
    )
}

/// The pointer targets and gesture regions covering a window-space point —
/// the lists a pointer press at that point would walk.
fn hits_at(runtime: &HeadlessRuntime, x: f32, y: f32) -> (usize, usize) {
    let point = kurbo::Point::new(f64::from(x), f64::from(y));
    let renderer = runtime.renderer();
    let targets = renderer
        .hit_test
        .pointer_targets
        .iter()
        .filter(|target| target.bounds.contains(point))
        .count();
    let regions = renderer
        .hit_test
        .gesture_regions
        .iter()
        .filter(|region| region.bounds.contains(point))
        .count();
    (targets, regions)
}

/// A hidden tab's content is a sub-view the tabs record no longer places:
/// its button's registrations retire with it. The selected pane's button
/// paints at the same spot, so the spot keeps exactly the live pane's one
/// target — never both — and a tap fires only that pane's action.
#[test]
fn a_hidden_tabs_button_is_not_hittable_after_the_switch() {
    let fired1 = Rc::new(Cell::new(false));
    let fired2 = Rc::new(Cell::new(false));
    let selection = Binding::container(Id::try_from(1).expect("non-zero tab id"));
    let builder = {
        let selection = selection.clone();
        let (fired1, fired2) = (fired1.clone(), fired2.clone());
        AnyViewBuilder::<AnyView>::new(move || {
            let tabs = (1_i32..=2)
                .map(|i| {
                    let id = Id::try_from(i).expect("non-zero tab id");
                    let fired = if i == 1 {
                        fired1.clone()
                    } else {
                        fired2.clone()
                    };
                    Tab::new(id, format!("Tab {i}"), move || {
                        let fired = fired.clone();
                        NavigationView::new(
                            format!("Pane {i}"),
                            button(format!("Fire {i}")).action(move || fired.set(true)),
                        )
                    })
                })
                .collect();
            AnyView::new(TabsLayout::new(selection.clone(), tabs))
        })
    };
    let mut runtime = runtime(builder);
    let (x, y) = midpoint(bounds_of(&mut runtime, "Fire 1"));

    selection.set(Id::try_from(2).expect("non-zero tab id"));
    pump_until_settled(&mut runtime);

    assert_eq!(
        hits_at(&runtime, x, y),
        (1, 0),
        "exactly the selected pane's target may cover the spot — a stale \
         target from the hidden pane would double it"
    );
    for event in primary_click(x, y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    assert!(
        fired2.get(),
        "the tap must fire the selected pane's button at that spot"
    );
    assert!(
        !fired1.get(),
        "a tap where the hidden tab's button painted must never fire it"
    );
}

/// A drawn context menu's painted bounds carry an occluder that eats the
/// presses beneath it; closing the menu retires the host's registrations,
/// so the row the occluder covered takes its taps again.
#[test]
fn a_closed_menus_occluder_leaves_the_row_beneath_hittable() {
    const ROWS: usize = 5;
    const MENU_ROW: usize = ROWS - 2;
    const ROW_H: f32 = 48.0;
    const TAP_ROW_H: f32 = 400.0;

    let command = Rc::new(Cell::new(false));
    let taps = Rc::new(RefCell::new(0_u32));
    let builder = {
        let taps = taps.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            let taps = taps.clone();
            let menu = {
                let command = command.clone();
                move || {
                    let menu_command = command.clone();
                    ContextMenu::new(vec!["Star".action(move || menu_command.set(true))])
                        .accessory(Frame::new(button("React").action(|| {})))
                }
            };
            AnyView::new(scroll(List::for_each(
                SignalCollection::new(rows),
                move |row| {
                    let index = row.into_inner();
                    let content = if index == MENU_ROW {
                        AnyView::new(
                            Frame::new(button("Menu row").action(|| {}).context_menu(menu()))
                                .height(ROW_H)
                                .max_width(f32::INFINITY),
                        )
                    } else if index == ROWS - 1 {
                        let taps = taps.clone();
                        AnyView::new(
                            vstack((text("Tail row"),))
                                .size(f32::INFINITY, TAP_ROW_H)
                                .on_tap(move || *taps.borrow_mut() += 1),
                        )
                    } else {
                        AnyView::new(
                            vstack((text(format!("Row {index}")),)).size(f32::INFINITY, ROW_H),
                        )
                    };
                    ListItem::new(content)
                },
            )))
        })
    };
    let mut runtime = runtime(builder);

    // Open the drawn presentation over the tail row, then choose the item —
    // the press belongs to the menu, and choosing it closes the host.
    let (row_x, row_y) = midpoint(bounds_of(&mut runtime, "Menu row"));
    for event in secondary_click(row_x, row_y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    let (_, accessory) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn presentation mounts");
    assert!(accessory.is_some(), "an accessory forces the drawn menu");
    let item = runtime
        .context_menu_row_frames()
        .first()
        .copied()
        .expect("the drawn menu emits a row per item");
    for event in primary_click(
        crate::num_cast::f64_as_f32(item.center().x),
        crate::num_cast::f64_as_f32(item.center().y),
    ) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    assert!(
        runtime.context_menu_presentation_frames().is_none(),
        "an item choice closes the menu"
    );

    // The occluder retired with the host: a tap where the menu painted —
    // dead while the occluder stood — now reaches the row's gesture.
    for event in primary_click(
        crate::num_cast::f64_as_f32(item.center().x),
        crate::num_cast::f64_as_f32(item.center().y),
    ) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    assert_eq!(
        *taps.borrow(),
        1,
        "the tap must reach the row once the closed menu's occluder is gone"
    );
}

/// A pushed page leaves the earlier page mounted under an unhittable scope:
/// its targets retire out of the hit lists, so the spot its button painted
/// at stops being hittable and a tap there fires nothing.
#[test]
fn an_inactive_navigation_page_is_not_hittable() {
    let fired = Rc::new(Cell::new(false));
    let builder = {
        let fired = fired.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let fired = fired.clone();
            AnyView::new(NavigationStack::new(NavigationView::new(
                "Root",
                vstack((
                    vstack((text("Root area"),))
                        .size(f32::INFINITY, 500.0)
                        .on_tap(move || fired.set(true)),
                    NavigationLink::new("Open Detail", || {
                        NavigationView::new("Detail", text("detail content"))
                    }),
                )),
            )))
        })
    };
    let mut runtime = runtime(builder);
    let mut at = Instant::now();
    pump_until_settled(&mut runtime);

    // The root page's counted region sits low in the content, clear of the
    // detail page's chrome — probe its centre.
    let (probe_x, probe_y) = (crate::num_cast::u32_as_f32(WINDOW.0) / 2.0, 400.0);
    assert!(
        hits_at(&runtime, probe_x, probe_y).1 > 0,
        "the root page's tap region is hittable before the push"
    );

    // Push the detail page through the link's accessibility click, then let
    // the transition run out.
    let update = runtime
        .accessibility_tree()
        .expect("the root page emits an accessibility tree");
    let (open, _) = find_by_label(&update, Role::Button, "Open Detail")
        .expect("the navigation link is missing");
    assert!(
        runtime.perform_accessibility_action(accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_node: open,
            target_tree: accesskit::TreeId::ROOT,
            data: None,
        }),
        "the link click changed nothing"
    );
    let mut elapsed = Duration::ZERO;
    while elapsed < TRANSITION {
        at += FRAME;
        let _ = runtime.pump_at(false, at);
        elapsed += FRAME;
    }
    pump_until_settled(&mut runtime);

    let (targets, regions) = hits_at(&runtime, probe_x, probe_y);
    assert_eq!(
        (targets, regions),
        (0, 0),
        "the inactive page's tap region must leave the hit lists"
    );
    for event in primary_click(probe_x, probe_y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    assert!(
        !fired.get(),
        "a tap where the inactive page's region painted must fire nothing"
    );
}

/// Platform-view placements stage at materialization and write to the sink
/// table once, at frame end: however often the frame re-materializes, the
/// table never sees the same view twice.
#[test]
fn the_platform_view_table_holds_each_view_once_after_n_frames() {
    let sink = PlatformViewSink::new();
    let mut env = test_environment();
    env.insert(sink.clone());
    let builder = AnyViewBuilder::<AnyView>::new(|| {
        AnyView::new(vstack((
            Frame::new(AnyView::new(Native::new(PlatformView::new("probe-a")))).height(48.0),
            Frame::new(AnyView::new(Native::new(PlatformView::new("probe-b")))).height(48.0),
        )))
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        env,
        builder,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    let mut at = Instant::now();
    for _ in 0..6 {
        at += FRAME;
        // A snapshot pump renders the frame like a real host's encoded
        // frame; publish runs once per encoded frame, as the host does.
        let pump = runtime.pump_at(true, at);
        assert!(pump.profile.counters.rendered);
        sink.table().borrow_mut().publish();
    }

    let table = sink.table().borrow();
    let placements = table.placements();
    assert_eq!(
        placements.len(),
        2,
        "each platform view holds its placement once, got {placements:?}"
    );
    let mut ids: Vec<u64> = placements.iter().map(|placement| placement.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2, "no view may record twice in one frame");
}

/// The materialized paint order keeps dev's per-frame registration ranking:
/// the scroll gutter — registered after the scroll's content — outranks
/// every content target it can overlap, and a `.on_tap` inside a List row
/// ranks above the row's own press target, so the innermost gesture claims
/// the press exactly as it did under the flat per-frame lists.
#[test]
fn materialized_paint_order_keeps_devs_ranking() {
    const ROWS: usize = 40;
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let selection = Binding::container(Option::<usize>::None);
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        AnyView::new(scroll(
            List::for_each(SignalCollection::new(rows), |row| {
                let index = row.into_inner();
                ListItem::new(
                    vstack((text(format!("Row {index}")),))
                        .size(360.0, 44.0)
                        .on_tap(|| {}),
                )
            })
            .selection(&selection),
        ))
    });
    let mut runtime = runtime(builder);
    pump_until_settled(&mut runtime);

    let point = kurbo::Point::new(200.0, 300.0);
    let renderer = runtime.renderer();
    let presses: Vec<usize> = renderer
        .hit_test
        .pointer_targets
        .iter()
        .filter(|target| target.press_slot.is_some() && target.bounds.contains(point))
        .map(|target| target.order)
        .collect();
    let claimed_regions: Vec<&crate::renderer::GestureRegion> = renderer
        .hit_test
        .gesture_regions
        .iter()
        .filter(|region| region.bounds.contains(point))
        .collect();

    // The gutter drag target outranks every press and gesture the scroll
    // content registered — it registers after the content its bar overlays.
    let gutter_orders: Vec<usize> = renderer
        .hit_test
        .pointer_targets
        .iter()
        .filter(|target| target.captures_drag)
        .map(|target| target.order)
        .collect();
    assert!(
        !gutter_orders.is_empty(),
        "an overflowing scroll must register its gutter drag target"
    );
    let content_max = presses
        .iter()
        .chain(claimed_regions.iter().map(|region| &region.order))
        .copied()
        .max()
        .expect("the rows register presses and gestures");
    for order in &gutter_orders {
        assert!(
            *order > content_max,
            "the gutter's order {order} must rank above every content order {content_max}"
        );
    }

    // A gesture region nested inside a row's press bounds outranks that
    // press — on dev the content's `.on_tap` minted its order after the row
    // bound its press slot.
    assert!(
        !presses.is_empty(),
        "a selectable row registers a press target at the point"
    );
    assert!(
        !claimed_regions.is_empty(),
        "the row's on_tap registers a gesture region at the point"
    );
    for region in &claimed_regions {
        for (index, target) in renderer.hit_test.pointer_targets.iter().enumerate() {
            if target.press_slot.is_none() || !target.bounds.contains(point) {
                continue;
            }
            let strictly_inside = region.bounds.x0 > target.bounds.x0
                && region.bounds.y0 > target.bounds.y0
                && region.bounds.x1 < target.bounds.x1
                && region.bounds.y1 < target.bounds.y1;
            if strictly_inside {
                assert!(
                    region.order > target.order,
                    "a gesture region nested in a row's press must outrank it \
                     (region #{index} order {} vs press order {})",
                    region.order,
                    target.order
                );
            }
        }
    }
}
