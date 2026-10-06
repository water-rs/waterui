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
use nami::collection::List as Membership;
use nami::collection::SignalCollection;
use waterui::animation::Animation;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::prelude::ContextMenu;
use waterui::{AnyView, ViewExt as _};
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::Native;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::{Id, SelfId};
use waterui_layout::collection_transition::collection_transition;
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{VStack, vstack};
use waterui_navigation::tab::{Tab, TabsLayout};
use waterui_navigation::{NavigationLink, NavigationStack, NavigationView};

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind, WindowSafeArea};
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

/// The gesture regions covering a window-space point, top-down by `y0`.
fn regions_at(runtime: &HeadlessRuntime, x: f64) -> Vec<kurbo::Rect> {
    let mut regions: Vec<kurbo::Rect> = runtime
        .renderer()
        .hit_test
        .gesture_regions
        .iter()
        .map(|region| region.bounds)
        .filter(|bounds| bounds.x0 <= x && x <= bounds.x1)
        .collect();
    regions.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    regions
}

fn click(runtime: &mut HeadlessRuntime, x: f64, y: f64) {
    for event in primary_click(
        crate::num_cast::f64_as_f32(x),
        crate::num_cast::f64_as_f32(y),
    ) {
        runtime.push_input_event(event);
    }
    pump_until_settled(runtime);
}

/// The scroll content the display-scale case lays out: a reference button
/// above a scroll whose content puts a 50pt tap target beneath a 100pt
/// spacer.
fn reference_over_scrolled_target(fired: &Rc<Cell<bool>>) -> AnyView {
    let fired = fired.clone();
    AnyView::new(vstack((
        button("Reference"),
        scroll(vstack((
            vstack((text("Spacer"),)).size(f32::INFINITY, 100.0),
            vstack((text("Target"),))
                .size(f32::INFINITY, 50.0)
                .on_tap(move || fired.set(true)),
        ))),
    )))
}

/// A placement scope's transform is a record-space delta, never the paint
/// transform: at display scale 2 the paint transform carries the scale while
/// hit space stays logical, so a scroll's clip scope must not scale the
/// content's registrations. The target is hit at the same logical window
/// rect a scale-1 window resolves it at.
#[test]
fn a_scroll_content_button_at_display_scale_two_is_hit_at_its_window_position() {
    const TARGET: f64 = 50.0;
    let truth_fired = Rc::new(Cell::new(false));
    let builder =
        AnyViewBuilder::<AnyView>::new(move || reference_over_scrolled_target(&truth_fired));
    let mut truth = runtime(builder);
    let x = midpoint(bounds_of(&mut truth, "Reference")).0;
    let expected = regions_at(&truth, f64::from(x))
        .into_iter()
        .find(|bounds| (bounds.height() - TARGET).abs() < 1.0)
        .expect("the scroll content's target registers a tap region at scale 1");

    let fired = Rc::new(Cell::new(false));
    let builder = {
        let fired = fired.clone();
        AnyViewBuilder::<AnyView>::new(move || reference_over_scrolled_target(&fired))
    };
    let mut runtime = runtime(builder).with_scale_factor(2.0);
    let x = f64::from(midpoint(bounds_of(&mut runtime, "Reference")).0);
    let regions = regions_at(&runtime, x);
    assert!(
        regions
            .iter()
            .any(|bounds| (bounds.y0 - expected.y0).abs() < 1.0
                && (bounds.height() - TARGET).abs() < 1.0),
        "the scroll content's tap region must resolve at its logical window rect \
         y {}..{} at display scale 2, got {regions:?}",
        expected.y0,
        expected.y1
    );
    click(&mut runtime, x, f64::midpoint(expected.y0, expected.y1));
    assert!(
        fired.get(),
        "a click at the button's true window position must fire it at display scale 2"
    );
}

/// More 44pt rows than the test window holds, so the List scrolls.
const OVERFLOWING_ROWS: usize = 30;

/// The page content shared by the navigation-offset case and its ground
/// truth: a reference button that registers outside any placement scope,
/// with an overflowing List — whose surface pushes a scroll clip scope —
/// directly beneath it.
fn reference_over_rows(tapped: &Rc<Cell<Option<usize>>>) -> AnyView {
    let tapped = tapped.clone();
    AnyView::new(vstack((
        button("Reference"),
        List::for_each(
            SignalCollection::new((0..OVERFLOWING_ROWS).map(SelfId::new).collect::<Vec<_>>()),
            move |row| {
                let index = row.into_inner();
                let tapped = tapped.clone();
                ListItem::new(
                    vstack((text(format!("Row {index}")),))
                        .size(360.0, 44.0)
                        .on_tap(move || tapped.set(Some(index))),
                )
            },
        ),
    )))
}

/// A pushed navigation page sits at the stack's window position (below a
/// header, the safe-area top and the bar). Its List overflows the page, so
/// the List's surface pushes a scroll clip scope; that scope's delta is the
/// caller's record-space delta, so it keeps the stack's position instead of
/// cancelling it against the paint transform. The first row is hit where it
/// sits relative to the page's reference button — the same offset a plain
/// window lays it at.
#[test]
fn a_list_row_inside_a_pushed_navigation_page_is_hit_below_the_stack_offset() {
    // Ground truth: the row's offset beneath the reference in a plain window.
    let truth_tapped = Rc::new(Cell::new(None));
    let builder = AnyViewBuilder::<AnyView>::new(move || reference_over_rows(&truth_tapped));
    let mut truth = runtime(builder);
    let reference = bounds_of(&mut truth, "Reference");
    let x = f64::midpoint(reference.x0, reference.x1);
    let row = regions_at(&truth, x)
        .into_iter()
        .find(|bounds| bounds.y0 >= reference.y1 - 1.0)
        .expect("the first list row registers a tap region below the reference");
    let (offset, height) = (row.y0 - reference.y1, row.height());

    let tapped = Rc::new(Cell::new(None));
    let builder = {
        let tapped = tapped.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let tapped = tapped.clone();
            // The header puts the stack itself at a non-zero window offset:
            // a scope that cancelled the stack's window position would land
            // the rows above where they draw.
            AnyView::new(vstack((
                text("Stack header").padding(),
                NavigationStack::new(NavigationView::new(
                    "Root",
                    NavigationLink::new("Open Detail", move || {
                        NavigationView::new("Detail", reference_over_rows(&tapped))
                    }),
                )),
            )))
        })
    };
    let mut env = test_environment();
    env.insert(WindowSafeArea(nami::binding(EdgeInsets::new(
        48.0, 0.0, 0.0, 0.0,
    ))));
    let mut runtime = HeadlessRuntime::new_for_tests(
        env,
        builder,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    pump_until_settled(&mut runtime);
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
    let mut at = Instant::now();
    let mut elapsed = Duration::ZERO;
    while elapsed < TRANSITION {
        at += FRAME;
        let _ = runtime.pump_at(false, at);
        elapsed += FRAME;
    }
    let reference = bounds_of(&mut runtime, "Reference");
    assert!(
        reference.y0 > 48.0,
        "the pushed page sits below the safe-area top, got {reference:?}"
    );
    let top = reference.y1 + offset;
    let regions = regions_at(&runtime, x);
    assert!(
        regions
            .iter()
            .any(|bounds| (bounds.y0 - top).abs() < 1.0 && (bounds.height() - height).abs() < 1.0),
        "the first row's tap region must resolve at y {top}..{} under the stack \
         offset, got {regions:?}",
        top + height
    );
    click(&mut runtime, x, top + height / 2.0);
    assert_eq!(
        tapped.get(),
        Some(0),
        "a click on the first row's true window position must fire that row"
    );
}

/// A collection entry mid-transition flushes under its clip scope: the scope
/// carries the entry's delta and the entry links with identity, so the
/// delta applies once. The entering row's tap region starts where the row
/// above it ends, not one more row-offset further down.
#[test]
fn a_collection_entry_mid_transition_is_hit_at_its_single_offset_position() {
    let tapped = Rc::new(Cell::new(None));
    let list: Membership<SelfId<u64>> = Membership::from(vec![SelfId::new(0), SelfId::new(1)]);
    let builder = {
        let list = list.clone();
        let tapped = tapped.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let tapped = tapped.clone();
            let collection = VStack::for_each(list.clone(), move |item: SelfId<u64>| {
                let id = *item;
                let tapped = tapped.clone();
                AnyView::new(
                    vstack((text(format!("Item {id}")),))
                        .size(120.0, 40.0)
                        .on_tap(move || tapped.set(Some(id))),
                )
            });
            AnyView::new(collection_transition(
                collection,
                Animation::linear(Duration::from_millis(1_000)),
            ))
        })
    };
    let mut runtime = runtime(builder);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    list.push(SelfId::new(2));
    let _ = runtime.pump_at(false, start + FRAME);
    let _ = runtime.pump_at(false, start + Duration::from_millis(516));

    let x = f64::from(WINDOW.0) / 2.0;
    let regions = regions_at(&runtime, x);
    assert_eq!(
        regions.len(),
        3,
        "both settled rows and the entering row register tap regions, got {regions:?}"
    );
    let settled = regions[1];
    let entering = regions[2];
    // Settled, the entering row starts one row pitch below the row above:
    // mid-transition it lies between that row's end and its settled top. A
    // delta applied twice lands it a whole offset further down.
    let settled_top = settled.y0 + (settled.y0 - regions[0].y0);
    assert!(
        entering.y0 >= settled.y1 - 1.0 && entering.y0 <= settled_top + 1.0,
        "the entering row's region must start between the row above's end \
         (y {}) and its settled top (y {settled_top}), not one more row offset \
         down: {entering:?}",
        settled.y1
    );
    for event in primary_click(
        crate::num_cast::f64_as_f32(x),
        crate::num_cast::f64_as_f32(f64::midpoint(entering.y0, entering.y1)),
    ) {
        runtime.push_input_event(event);
    }
    let _ = runtime.pump_at(false, start + Duration::from_millis(532));
    assert_eq!(
        tapped.get(),
        Some(2),
        "a click inside the entering row's region must land on the entering row"
    );
}

/// A hovered target that turns unhittable leaves hover, as dev's
/// `Hittable(false)` truncation cleared it: the slot and the state-layer
/// handles both report not hovering once the subtree is unhittable.
#[test]
fn a_hovered_target_that_turns_unhittable_clears_its_hover() {
    let enabled = Binding::container(true);
    let builder = {
        let enabled = enabled.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(vstack((button("Hover me").hittable(enabled.clone()),)))
        })
    };
    let mut runtime = runtime(builder);
    let (x, y) = midpoint(bounds_of(&mut runtime, "Hover me"));
    runtime.push_input_event(InputEvent::PointerMove {
        id: 2,
        kind: PointerKind::Mouse,
        x,
        y,
    });
    pump_until_settled(&mut runtime);

    let point = kurbo::Point::new(f64::from(x), f64::from(y));
    let (slot, handles) = runtime
        .renderer()
        .hit_test
        .hover_targets
        .iter()
        .find(|target| target.bounds.contains(point))
        .map(|target| (target.slot.clone(), target.handles.clone()))
        .expect("the button registers a hover target under the pointer");
    let handles = handles.expect("a button's hover target carries state-layer handles");
    assert!(
        runtime.renderer().hit_test.interaction.hovering(&slot) && handles.hovering(),
        "the pointer resting on the button hovers it"
    );

    // Each record rebinds the widget's state-layer handles, so the hover
    // check reads the handles bound by the unhittable record.
    let press = runtime
        .renderer()
        .hit_test
        .pointer_targets
        .iter()
        .find(|target| target.bounds.contains(point))
        .and_then(|target| target.press_slot.clone())
        .expect("the button registers a press target under the pointer");

    enabled.set(false);
    pump_until_settled(&mut runtime);
    assert!(
        !runtime.renderer().hit_test.interaction.hovering(&slot),
        "the unhittable button's hover slot must clear"
    );
    let handles = runtime
        .renderer()
        .hit_test
        .interaction
        .handles_for(&press)
        .expect("the unhittable button still binds its state-layer handles");
    assert!(
        !handles.hovering(),
        "the unhittable button's state-layer handles must leave hover"
    );
}
