//! Scroll-machinery contracts: the §6 measure rules on a scroll's two axes,
//! `report_offset` binding writes, controller request/animation handling,
//! scrollbar-thumb geometry, and the retained-context legs that keep them
//! working inside lazy and navigation content. These are the backend's own
//! contracts, so they stay here under water-rs/waterui#1673, mounted on
//! `HeadlessRuntime`/`SemanticRuntime` under `TestStyle` — extents the
//! assertions need are derived from the mounted nodes' own bounds, never
//! pinned to a style package's metrics.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); every case names its origin file and asserts what
//! it asserted there.

mod support;

use accesskit::Role;
use support::{
    f64_as_f32, mount_rendered, mount_semantic, node, node_bounds, node_bounds_with_role, node_id,
    node_label_exists, nodes,
};
use waterui::View;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::id::SelfId;
use waterui::navigation::{NavigationStack, NavigationView};
use waterui::prelude::*;
use waterui_core::Environment;
use waterui_layout::scroll;
use waterui_layout::scroll::{ScrollView, scroll_both, scroll_horizontal};

fn visual_shell<V: View>(content: V) -> impl View {
    content.padding_with(20.0).background(Srgb::BLACK)
}

fn labeled_card(label: &'static str, width: f32, height: f32, color: Srgb) -> impl View {
    text(label)
        .body()
        .foreground(Srgb::WHITE)
        .size(width, height)
        .background(color)
}

fn scroll_content_view() -> impl View {
    visual_shell(
        scroll(
            vstack((
                labeled_card("First item", 120.0, 48.0, Srgb::new(1.0, 0.1, 0.1)),
                labeled_card("Second item", 120.0, 48.0, Srgb::new(0.1, 1.0, 0.1)),
                labeled_card("Third item", 120.0, 48.0, Srgb::new(0.1, 0.1, 1.0)),
                labeled_card("Fourth item", 120.0, 48.0, Srgb::new(1.0, 0.8, 0.1)),
            ))
            .spacing(12.0),
        )
        .size(120.0, 120.0)
        .a11y_label("scroll-layout"),
    )
}

// Origin: waterui `testing/tests/layout.rs`.
#[test]
fn scroll_view_scroll_down_changes_content() {
    let mut app = mount_rendered(Environment::new(), scroll_content_view, 180, 180);
    app.settle();
    let second_before = node_bounds(app.tree(), "Second item");
    let rail = node_id(app.tree(), "scroll-layout");
    app.scroll_down(rail);

    let second_after = node_bounds(app.tree(), "Second item");
    assert!(
        node_label_exists(app.tree(), "Third item"),
        "the third item must be reachable after scrolling"
    );
    assert!(
        second_after.y0 < second_before.y0,
        "scrolling down should move earlier content upward: before={second_before:?} after={second_after:?}"
    );
}

// Defect reproduction: a chip row clipped by its scroll rail inside a 340 pt
// sidebar ("Archive" rendered "Archiv" while ~40 pt of free space remains
// before the trailing icon). The rail is a `scroll()` — default
// `Axis::Vertical` — wrapping the horizontal chip row next to a trailing
// icon button.
//
// Root cause is in `RenderNode::Scroll::measure`: it answered
// `proposal.unwrap_or(0)` on both axes, so the `0` probe on the scroll's
// non-scrolling axis reported `0` instead of the content's intrinsic extent —
// layout-spec.md §6 ("only a `0` proposal measures the content, answering its
// intrinsic extent on the non-scrolling axis and `0` on the scrolling
// axis"). The row's negotiator (§4.2) then treated the rail as a 0-minimum
// flexible member: the icon kept its target and the rail absorbed the whole
// deficit, clipping the last chip's label inside the viewport — unreachable,
// since a vertical scroll cannot scroll horizontally.
//
// The chips are plain buttons here: the contract is the rail's measure
// answer, and every extent it needs is read off the mounted tree rather than
// a style package's metrics.
fn chips() -> impl View {
    hstack((
        button("All").action(|| {}),
        button("Work").action(|| {}),
        button("Personal").action(|| {}),
        button("Archive").action(|| {}),
    ))
    .spacing(8.0)
}

fn chip_row(rail: ScrollView) -> impl View {
    vstack((hstack((
        rail.a11y_label("chip-rail"),
        button("F").action(|| {}).a11y_label("More filters"),
    ))
    .spacing(8.0),))
}

/// The chips' intrinsic row extent, measured on the mounted tree: the
/// leading chip's leading edge to the trailing chip's trailing edge.
fn chip_row_extent(app: &mut support::RenderedApp) -> f64 {
    let all = node_bounds_with_role(app.tree(), Role::Button, "All");
    let archive = node_bounds_with_role(app.tree(), Role::Button, "Archive");
    archive.x0 + archive.width() - all.x0
}

/// §6 scroll contract on the non-scrolling axis: the vertical rail's minimum
/// is the chips' intrinsic width, so the rail keeps the row's extent and the
/// row overflows past the trailing icon instead of clipping "Archive".
#[test]
fn vertical_scroll_rail_reports_content_minimum_on_non_scrolling_axis() {
    let mut app = mount_rendered(Environment::new(), || chip_row(scroll(chips())), 340, 300);
    app.settle();
    let extent = chip_row_extent(&mut app);
    let rail = node_bounds(app.tree(), "chip-rail");
    assert!(
        (rail.width() - extent).abs() < 1.0,
        "vertical rail should report the content's intrinsic minimum on its \
         non-scrolling axis (§6): rail={rail:?} chip extent={extent}"
    );
    let archive = node_bounds_with_role(app.tree(), Role::Button, "Archive");
    assert!(
        archive.x0 + archive.width() <= rail.x0 + rail.width() + 0.1,
        "Archive chip should lie fully inside the rail: archive={archive:?} rail={rail:?}"
    );
    let icon = node_bounds_with_role(app.tree(), Role::Button, "More filters");
    assert!(
        icon.x0 >= rail.width() - 0.1,
        "the trailing icon should be pushed past the rail by the overflow: {icon:?}"
    );
}

/// The scrolling axis is the one that takes `0`: a horizontal rail
/// legitimately negotiates down and clips its overflow — scrollable, by
/// design. This pins the spec-correct side of the same rule.
#[test]
fn horizontal_scroll_rail_keeps_zero_minimum_on_scrolling_axis() {
    let mut app = mount_rendered(
        Environment::new(),
        || chip_row(scroll_horizontal(chips())),
        340,
        300,
    );
    app.settle();
    let icon = node_bounds_with_role(app.tree(), Role::Button, "More filters");
    let rail = node_bounds(app.tree(), "chip-rail");
    // The rail's slot when it absorbs the row deficit: everything before the
    // icon, less the 8-pt gap the row puts between them.
    let squeezed = icon.x0 - rail.x0 - 8.0;
    assert!(
        (rail.width() - squeezed).abs() < 1.0,
        "horizontal rail should absorb the row deficit (§6: 0 on the \
         scrolling axis): rail={rail:?} expected slot={squeezed}"
    );
}

// Defect reproduction: water-rs/hydrolysis#182 — watergram's folder rail is a
// `scroll_horizontal` wrapping `Lazy::hstack(ForEach)` caption chips beside a
// trailing icon button, as `water-rs/watergram` `sidebar_view` built it
// (src/views.rs at 937ea83). The last chip rendered "Archiv" in a 340 pt
// sidebar even though the rail had room to scroll to it.
//
// Root cause is the same `RenderNode::Scroll::measure` arm one layer deeper:
// `Lazy::hstack` wraps its stack in a vertical `scroll()`, so the horizontal
// rail's content measure proposes `(None, Some(height))` to a *scroll* — and
// the arm answered `proposal.unwrap_or(0)`, i.e. `0`, where §2 asks for the
// ideal (intrinsic) extent. The outer scroll's `content_size` collapsed to
// the viewport, so the rail could never scroll and the viewport clip cut the
// last chip mid-glyph. The arm now measures the content's intrinsic extent on
// a `None` axis — the same extent the layout pass then measures the content
// at — while the `0` probe keeps the contract above.
//
// The caption chips below are the ones the app draws itself (`text(label)
// .caption()` over `SurfaceVariant`), not a style package's chip — watergram
// does not use the m3 chip for this row.

/// One watergram folder chip: caption text over a rounded `SurfaceVariant`
/// when active.
fn folder_chip(name: &'static str, active: bool) -> impl View {
    use waterui::shape::{RoundedRectangle, ShapeExt as _};
    use waterui::theme::color::{Accent, SurfaceVariant};

    let label_view: waterui::AnyView = if active {
        text(name).caption().bold().foreground(Accent).anyview()
    } else {
        text(name).caption().muted().anyview()
    };
    label_view.padding_with((3.0, 10.0)).background(if active {
        RoundedRectangle::new(0.5).fill(SurfaceVariant).anyview()
    } else {
        waterui::AnyView::default()
    })
}

/// The app's trailing icon button (`folder_plus()` in watergram): a plain
/// icon-only button.
fn folder_icon_button() -> impl View {
    waterui_controls::button(label("New folder").icon(text("+")))
        .label_style(waterui_controls::label::LabelDisplayMode::IconOnly)
        .plain()
        .action(|| {})
}

/// The dogfood row: `hstack((scroll_horizontal(lazy chips), icon_button))`
/// padded `(4, 8)` — the same nesting the app ships.
fn folder_rail_row(
    chips: &[(&'static str, i32)],
    controller: &waterui_layout::scroll::ScrollController<waterui::layout::Point>,
) -> impl View {
    use waterui::component::lazy::Lazy;
    use waterui::id::SelfId;
    use waterui::views::ForEach;

    hstack((
        scroll_horizontal(Lazy::hstack(ForEach::new(
            chips.iter().copied().map(SelfId::new).collect::<Vec<_>>(),
            |tab| {
                let (name, id) = tab.into_inner();
                folder_chip(name, id == 0).anyview()
            },
        )))
        .scroll_controller(controller)
        .a11y_label("chip-rail"),
        folder_icon_button().a11y_label("New folder"),
    ))
    .padding_with((4.0, 8.0))
}

/// A scroll whose content is reached through another scroll must keep a real
/// content extent: the rail's viewport takes the row's leftover width, the
/// chips' row stays wider than it, and scrolling brings the last chip's whole
/// label inside the viewport.
///
/// The row is pinned at 292 pt so the leftover rail is narrower than the
/// chip content — the geometry the defect clipped at. Before the fix the
/// inner scroll answered `0` on the rail's `None` measure, the content
/// collapsed to the viewport, and the last chip could never be scrolled in.
#[test]
fn a_nested_scroll_reports_its_content_extent_and_scrolls_the_last_chip_in() {
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let chips = vec![("All", 0i32), ("Work", 1), ("Personal", 2), ("Archive", -1)];
    let mut app = mount_rendered(
        Environment::new(),
        {
            let controller = controller.clone();
            move || {
                scroll(vstack((
                    folder_rail_row(&chips, &controller).size(292.0, 56.0),
                )))
            }
        },
        340,
        640,
    );
    // `Lazy::hstack` keeps a small perpetual measure queue under this
    // runtime, so the test pumps a bounded window instead of settling; the
    // geometry the assertions read is stable well inside it.
    app.pump_for(std::time::Duration::from_secs(1));

    let rail_ids: Vec<_> = nodes(app.tree())
        .filter(|(_, node)| node.label() == Some("chip-rail"))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        rail_ids.len(),
        2,
        "expected outer rail and nested scroll nodes"
    );
    let viewport = node(app.tree(), rail_ids[0])
        .bounds()
        .expect("the rail viewport carries bounds");
    let content = node(app.tree(), rail_ids[1])
        .bounds()
        .expect("the rail content carries bounds");
    // The rail takes the row's leftover: it ends where the icon begins, the
    // icon keeping its own measured extent beside it.
    let icon = node_bounds_with_role(app.tree(), Role::Button, "New folder");
    assert!(
        viewport.x0 + viewport.width() <= icon.x0 && icon.width() > 0.0 && viewport.width() > 0.0,
        "the rail's viewport should take the row's leftover before the icon: \
         viewport={viewport:?} icon={icon:?}"
    );
    assert!(
        content.width() > viewport.width() + 1.0,
        "the horizontal scroll's content must keep the chips' real extent \
         and overflow its viewport; before the fix the nested scroll answered \
         `0` on the `None` measure and the content collapsed to the viewport: \
         viewport={viewport:?} content={content:?}"
    );

    controller.scroll_to(waterui::layout::Point::new(10_000.0, 0.0));
    app.pump_for(std::time::Duration::from_secs(1));
    let archive = node_bounds(app.tree(), "Archive");
    let viewport = nodes(app.tree())
        .find_map(|(_, node)| (node.label() == Some("chip-rail")).then_some(node))
        .and_then(accesskit::Node::bounds)
        .expect("the rail viewport carries bounds");
    assert!(
        archive.x0 >= viewport.x0 - 0.1
            && archive.x0 + archive.width() <= viewport.x0 + viewport.width() + 0.1,
        "the whole Archive label must lie inside the rail's viewport once \
         scrolled to the end: archive={archive:?} viewport={viewport:?}"
    );
}

/// With an offer wider than the chip row, every chip is whole on both
/// variants. The horizontal rail claims the row's width — its scrolling
/// axis — so the icon tiles out to the trailing edge. The vertical rail
/// hugs the chips' width instead: the row's main axis is the scroll's
/// non-scrolling axis, which follows the content (layout-spec.md §6,
/// water-rs/waterui#1208), leaving the row content-sized and centred.
#[test]
fn chip_rail_fills_offer_when_row_fits() {
    for horizontal in [false, true] {
        let mut app = mount_rendered(
            Environment::new(),
            move || {
                chip_row(if horizontal {
                    scroll_horizontal(chips())
                } else {
                    scroll(chips())
                })
            },
            900,
            300,
        );
        app.settle();
        let extent = chip_row_extent(&mut app);
        let rail = node_bounds(app.tree(), "chip-rail");
        assert!(
            extent <= rail.width() + 0.1,
            "the rail has room for the whole chip row: extent={extent} rail={rail:?}"
        );
        let icon = node_bounds_with_role(app.tree(), Role::Button, "More filters");
        if horizontal {
            // The rail claims the offer on its scrolling axis: icon, gap
            // and rail tile the full row.
            assert!(
                (icon.x0 - rail.width() - 8.0).abs() < 1.0
                    && (icon.x0 + icon.width() - 900.0).abs() < 1.0,
                "rail and icon fill the 900-pt row: icon={icon:?} rail={rail:?}"
            );
        } else {
            // The rail hugs the content on its non-scrolling axis and the
            // icon sits right after it — the row no longer tiles to 900.
            assert!(
                (rail.width() - extent).abs() < 1.0
                    && (icon.x0 - (rail.x0 + rail.width() + 8.0)).abs() < 1.0,
                "vertical rail hugs the chip extent, icon follows: icon={icon:?} \
                 rail={rail:?} extent={extent}"
            );
        }
    }
}

/// `ScrollView::report_offset` (water-rs/waterui#1296): the backend writes
/// the content offset, in points, into the binding whenever it changes —
/// whether the user or a controller scrolled it, every frame of a glide —
/// and stays silent while the view is idle.
///
/// A scroll rail with a `report_offset` binding and an accessibility label
/// for the query surface.
#[allow(clippy::needless_pass_by_value)]
fn offset_rail(
    offset: Binding<waterui::layout::Point>,
    controller: Option<waterui_layout::scroll::ScrollController<waterui::layout::Point>>,
) -> impl View {
    let rail = scroll(
        vstack((
            labeled_card("First item", 120.0, 48.0, Srgb::new(1.0, 0.1, 0.1)),
            labeled_card("Second item", 120.0, 48.0, Srgb::new(0.1, 1.0, 0.1)),
            labeled_card("Third item", 120.0, 48.0, Srgb::new(0.1, 0.1, 1.0)),
            labeled_card("Fourth item", 120.0, 48.0, Srgb::new(1.0, 0.8, 0.1)),
        ))
        .spacing(12.0),
    )
    .report_offset(&offset);
    let rail = match &controller {
        Some(controller) => rail.scroll_controller(controller),
        None => rail,
    };
    rail.size(120.0, 120.0).a11y_label("offset-rail")
}

/// A `report_offset` sink that counts every write: the binding is written,
/// never read, so writes must arrive only when the offset changes.
fn counting_offset() -> (
    Binding<waterui::layout::Point>,
    std::rc::Rc<std::cell::Cell<usize>>,
) {
    let writes = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let counted = waterui::binding(waterui::layout::Point::zero()).filter({
        let writes = std::rc::Rc::clone(&writes);
        move |_| {
            writes.set(writes.get() + 1);
            true
        }
    });
    (counted, writes)
}

// Wheel scrolling writes the offset: each `ScrollDown` step lands in the
// binding once the glide settles.
#[test]
fn report_offset_writes_the_offset_as_the_view_scrolls() {
    let (offset, writes) = counting_offset();
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            move || offset_rail(offset.clone(), None)
        },
        180,
        180,
    );
    app.settle();
    assert_eq!(
        offset.snapshot(),
        waterui::layout::Point::zero(),
        "attaching writes the current offset at once"
    );
    let after_attach = writes.get();

    let rail = node_id(app.tree(), "offset-rail");
    app.scroll_down(rail);
    assert!(
        offset.snapshot().y > 0.0,
        "scrolling must write the new offset, got {:?}",
        offset.snapshot()
    );
    assert!(
        writes.get() > after_attach,
        "scrolling wrote {after_attach} -> {} offsets",
        writes.get()
    );
}

// A controller `scroll_to` writes the offset the same way user input does.
#[test]
fn report_offset_writes_the_offset_on_controller_scroll_to() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            let controller = controller.clone();
            move || offset_rail(offset.clone(), Some(controller.clone()))
        },
        180,
        180,
    );
    app.settle();

    controller.scroll_to(waterui::layout::Point::new(0.0, 60.0));
    app.settle();
    assert_eq!(
        offset.snapshot(),
        waterui::layout::Point::new(0.0, 60.0),
        "a controller scroll_to must land in the report binding"
    );
}

/// A rail tall enough to take `scroll_to(0, 300)` without clamping — the
/// navigation-content tests below need real travel, not the edge.
fn nav_rail(
    offset: &Binding<waterui::layout::Point>,
    controller: &waterui_layout::scroll::ScrollController<waterui::layout::Point>,
) -> impl View {
    scroll(
        vstack((
            labeled_card("Item 1", 120.0, 60.0, Srgb::new(1.0, 0.1, 0.1)),
            labeled_card("Item 2", 120.0, 60.0, Srgb::new(0.1, 1.0, 0.1)),
            labeled_card("Item 3", 120.0, 60.0, Srgb::new(0.1, 0.1, 1.0)),
            labeled_card("Item 4", 120.0, 60.0, Srgb::new(1.0, 0.8, 0.1)),
            labeled_card("Item 5", 120.0, 60.0, Srgb::new(0.8, 0.1, 1.0)),
            labeled_card("Item 6", 120.0, 60.0, Srgb::new(0.1, 0.8, 1.0)),
            labeled_card("Item 7", 120.0, 60.0, Srgb::new(1.0, 0.5, 0.1)),
            labeled_card("Item 8", 120.0, 60.0, Srgb::new(0.5, 0.1, 1.0)),
        ))
        .spacing(12.0),
    )
    .report_offset(offset)
    .scroll_controller(controller)
    .size(120.0, 120.0)
    .a11y_label("nav-rail")
}

/// water-rs/waterui#1915: a `scroll_to` is applied while Hydrolysis lays out
/// the scroll node, and a navigation page's content is a retained sub-view
/// whose layout is cached (`BuiltSubview::layout_if_needed` skips it once the
/// rect is stable). On the broken build a controller request only pruned the
/// controller's frame-registry watch — the cached layout never re-ran, the
/// request was never applied, and the rail stayed at offset 0.
#[test]
fn scroll_controller_applies_requests_inside_navigation_content() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            let controller = controller.clone();
            move || {
                NavigationStack::new(NavigationView::new("Inbox", nav_rail(&offset, &controller)))
            }
        },
        320,
        240,
    );
    app.settle();

    controller.scroll_to(waterui::layout::Point::new(0.0, 300.0));
    app.settle();
    assert_eq!(
        offset.snapshot(),
        waterui::layout::Point::new(0.0, 300.0),
        "a controller scroll_to inside navigation content must land"
    );

    // A repeat request to the same target after a user scroll is honoured:
    // the generation bump is what makes it observable.
    let rail = node_id(app.tree(), "nav-rail");
    app.scroll_down(rail);
    assert_ne!(
        offset.snapshot(),
        waterui::layout::Point::new(0.0, 300.0),
        "the user scroll should move the rail off the requested offset"
    );
    controller.scroll_to(waterui::layout::Point::new(0.0, 300.0));
    app.settle();
    assert_eq!(
        offset.snapshot(),
        waterui::layout::Point::new(0.0, 300.0),
        "a repeat scroll_to to the same target must land after a user scroll"
    );
}

/// water-rs/waterui#1915, the measure-memo half: a lazy row's host measures
/// it (`patch_and_measure`) before the flush lays it out (`flush_in_rect`),
/// and inside the row's dependency pass the scroll's child measure answers
/// from the per-frame memo — skipping the reads beneath it, so the pass's
/// dependency sweep prunes a subscription the cached layout still needs.
/// The first growth still lays the row out (the memo answered with fresh
/// dimensions); the pruned dependency only shows on the second, which must
/// still re-lay the text out.
#[test]
fn scroll_child_measure_memo_cannot_hide_a_layout_dependency() {
    let value = Binding::container(String::from("9"));
    let mut app = mount_rendered(
        Environment::new(),
        {
            let value = value.clone();
            move || {
                let value = value.clone();
                scroll(VStack::for_each(vec![SelfId::new(0usize)], move |_item| {
                    let value = value.clone();
                    scroll_both(waterui::text!("{value}").a11y_label("counter")).max_height(24.0)
                }))
            }
        },
        320,
        240,
    );
    app.settle();
    let counter = |app: &mut support::RenderedApp| node_bounds(app.tree(), "counter");

    // The first update re-runs the row's pass: on the broken build the
    // pass's own probes hit memos filled by `patch_and_measure`, so the
    // text's content read is never re-recorded and `finish_pass` prunes it
    // — the dependency is lost while the fresh geometry still lands.
    value.set(String::from("10000"));
    app.settle();
    let counter_after_first = counter(&mut app);

    // The second update is what the lost dependency must still deliver: a
    // string wider than the row's viewport makes the laid-out extent grow
    // visibly past the first update's answer.
    value.set(String::from(
        "100000000000000000000000000000000000000000000000000",
    ));
    app.settle();
    let counter_after_second = counter(&mut app);
    assert!(
        counter_after_second.width() > counter_after_first.width(),
        "the reactive text's laid-out frame did not grow a second time: \
         {counter_after_first:?} -> {counter_after_second:?}"
    );
}

// A controller `animate_to` rides the request's curve: mid-duration the
// reported offset is strictly between start and target — a jump would
// already sit at the target — and the run lands exactly on it once the
// duration ends (water-rs/waterui#1901).
#[test]
fn animate_to_glides_along_its_curve_and_lands_on_the_target() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            let controller = controller.clone();
            move || offset_rail(offset.clone(), Some(controller.clone()))
        },
        180,
        180,
    );
    app.settle();

    controller.animate_to(
        waterui::layout::Point::new(0.0, 108.0),
        waterui::animation::Animation::ease_in_out(std::time::Duration::from_millis(500)),
    );
    app.pump_for(std::time::Duration::from_millis(200));
    let midway = offset.snapshot().y;
    assert!(
        midway > 0.0 && midway < 108.0,
        "an animated scroll should be in flight mid-duration, offset {midway}"
    );

    app.pump_for(std::time::Duration::from_millis(500));
    app.settle();
    assert_eq!(
        offset.snapshot(),
        waterui::layout::Point::new(0.0, 108.0),
        "the animation must land exactly on the target"
    );
}

// A spring request settles on the target once its duration ends, however far
// it overshoots in between.
#[test]
fn animate_to_with_a_spring_settles_on_the_target() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            let controller = controller.clone();
            move || offset_rail(offset.clone(), Some(controller.clone()))
        },
        180,
        180,
    );
    app.settle();

    controller.animate_to(
        waterui::layout::Point::new(0.0, 60.0),
        waterui::animation::Animation::spring(100.0, 10.0),
    );
    app.settle();
    assert_eq!(
        offset.snapshot(),
        waterui::layout::Point::new(0.0, 60.0),
        "a spring scroll must settle exactly on the target"
    );
}

// A `scroll_to` request carries no animation: it still lands in one frame.
#[test]
fn scroll_to_lands_in_one_frame() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            let controller = controller.clone();
            move || offset_rail(offset.clone(), Some(controller.clone()))
        },
        180,
        180,
    );
    app.settle();

    controller.scroll_to(waterui::layout::Point::new(0.0, 60.0));
    app.pump_for(std::time::Duration::from_millis(16));
    assert_eq!(
        offset.snapshot(),
        waterui::layout::Point::new(0.0, 60.0),
        "a jump must land within a single frame"
    );
}

// The user scrolling takes over from an animation in flight: a trackpad
// pixel delta cancels the run and moves the offset directly, so pumping the
// rest of the duration moves nothing.
#[test]
fn a_user_scroll_during_animate_to_cancels_the_animation() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            let controller = controller.clone();
            move || offset_rail(offset.clone(), Some(controller.clone()))
        },
        180,
        180,
    );
    app.settle();

    controller.animate_to(
        waterui::layout::Point::new(0.0, 108.0),
        waterui::animation::Animation::ease_in_out(std::time::Duration::from_millis(500)),
    );
    app.pump_for(std::time::Duration::from_millis(200));
    let midway = offset.snapshot().y;
    assert!(
        midway > 0.0 && midway < 108.0,
        "expected an in-flight offset, got {midway}"
    );

    let rail = node_bounds(app.tree(), "offset-rail");
    app.scroll_at(
        f64_as_f32(rail.x0) + 60.0,
        f64_as_f32(rail.y0) + 60.0,
        0.0,
        -20.0,
        false,
    );
    let taken = offset.snapshot().y;
    assert!(
        (taken - (midway + 20.0)).abs() < 0.5,
        "the pixel delta should move the offset directly: {midway} -> {taken}"
    );

    app.pump_for(std::time::Duration::from_millis(600));
    assert!(
        (offset.snapshot().y - taken).abs() < 0.5,
        "a cancelled animation must not resume toward the target: {:?}",
        offset.snapshot()
    );
}

// Dragging the horizontal scrollbar's thumb moves `offset_x`: the gutter
// target maps the pointer through the thumb geometry of the *horizontal*
// axis, so the content width — not the height — sizes the thumb and its
// travel.
#[test]
fn horizontal_scrollbar_drag_moves_the_offset() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            move || {
                scroll_horizontal(
                    hstack((
                        labeled_card("First chip", 120.0, 48.0, Srgb::new(1.0, 0.1, 0.1)),
                        labeled_card("Second chip", 120.0, 48.0, Srgb::new(0.1, 1.0, 0.1)),
                        labeled_card("Third chip", 120.0, 48.0, Srgb::new(0.1, 0.1, 1.0)),
                        labeled_card("Fourth chip", 120.0, 48.0, Srgb::new(1.0, 0.8, 0.1)),
                    ))
                    .spacing(12.0),
                )
                .report_offset(&offset)
                // The label lands on the scroll leaf itself, not the frame
                // slot: under §6 a horizontal scroll hugs its content's
                // height, so the gutter the drag targets lives in the
                // hugged bounds, not the 120x120 frame (water-rs/waterui#1208).
                .a11y_label("h-rail")
                .size(120.0, 120.0)
            }
        },
        180,
        180,
    );
    app.settle();
    let rail = node_bounds(app.tree(), "h-rail");
    assert!(
        offset.snapshot().x.abs() <= 0.5,
        "the rail starts at offset zero: {:?}",
        offset.snapshot()
    );
    // The horizontal gutter is the bottom 12pt of the rail's viewport; at
    // offset zero the thumb starts at the gutter's leading edge, so a press
    // near that edge grabs the thumb and a drag across it scrolls.
    let gutter_y = f64_as_f32(rail.y0 + rail.height() - 6.0);
    app.drag_from_to(
        f64_as_f32(rail.x0) + 10.0,
        gutter_y,
        f64_as_f32(rail.x0) + 70.0,
        gutter_y,
        10,
    );
    let x = offset.snapshot().x;
    // The thumb's extent is track * viewport / content (120 * 120 / 516 ≈
    // 27.9), so its travel is ≈ 92.1. The press at 10pt grabs the thumb 10pt
    // in; dragging to 70pt maps 60pt of pointer travel onto the 396pt
    // content extent: (70 - 10) / 92.1 * 396 ≈ 258.
    let expected = (70.0 - 10.0) / (120.0 - 120.0 * 120.0 / 516.0) * 396.0;
    assert!(
        (x - expected).abs() <= 2.0,
        "the thumb maps pointer travel through its geometry: expected \
         ≈{expected}, got {x} (rail={rail:?})"
    );
    assert!(
        x <= 396.0,
        "offset_x cannot pass the content extent (4 x 120 + 3 x 12 - 120): {x}"
    );
}

// Idle is silent: frames pump but the offset never changes, so nothing is
// written past the attach write.
#[test]
fn report_offset_writes_nothing_while_idle() {
    let (offset, writes) = counting_offset();
    let mut app = mount_rendered(
        Environment::new(),
        {
            let offset = offset.clone();
            move || offset_rail(offset.clone(), None)
        },
        180,
        180,
    );
    app.settle();
    let baseline = writes.get();

    app.pump_for(std::time::Duration::from_millis(500));
    app.settle();
    assert_eq!(
        writes.get(),
        baseline,
        "an idle scroll view must not write the offset binding"
    );
    assert_eq!(offset.snapshot(), waterui::layout::Point::zero());
}

// The semantic runtime never runs layout — `emit_accessibility` births the
// scroll handle — so the report binding must be attached there too: an
// accessibility ScrollDown still moves the offset.
#[test]
fn report_offset_writes_under_the_semantic_runtime() {
    let (offset, writes) = counting_offset();
    let mut app = mount_semantic(
        Environment::new(),
        {
            let offset = offset.clone();
            move || offset_rail(offset.clone(), None)
        },
        180,
        180,
    );
    app.settle();
    let baseline = writes.get();

    let rail = node_id(app.tree(), "offset-rail");
    app.scroll_down(rail);
    assert!(
        offset.snapshot().y > 0.0,
        "a semantic-runtime scroll must write the offset, got {:?}",
        offset.snapshot()
    );
    assert!(
        writes.get() > baseline,
        "semantic scrolling wrote {baseline} -> {} offsets",
        writes.get()
    );
}

// water-rs/waterui#1208: a scroll answers the content's own answer on its
// non-scrolling axis (docs/layout-spec.md §6) — a horizontal scroll hugs
// the chip row's height instead of claiming the stack's whole offer, so a
// horizontal `ScrollView` inside a `VStack` needs no height literal.

/// The issue scenario: `VStack { ScrollView(.horizontal) { chips } }` sizes
/// the scroll to the chips' height.
#[test]
fn horizontal_scrollview_hugs_content_height_in_a_vstack() {
    let mut app = mount_rendered(
        Environment::new(),
        || vstack((scroll_horizontal(chips()).a11y_label("chip-rail"),)),
        340,
        300,
    );
    app.settle();
    let chip_height = node_bounds_with_role(app.tree(), Role::Button, "All").height();
    let rail = node_bounds(app.tree(), "chip-rail");
    assert!(
        (rail.height() - chip_height).abs() < 1.0,
        "horizontal scroll should hug the chips' height instead of claiming \
         the stack's offer (#1208): rail={rail:?} chip height={chip_height}"
    );
}

/// The vertical symmetric case: a vertical scroll in an `HStack` hugs the
/// content's width.
#[test]
fn vertical_scrollview_hugs_content_width_in_an_hstack() {
    let mut app = mount_rendered(
        Environment::new(),
        || {
            hstack((scroll(
                vstack((
                    button("All").action(|| {}),
                    button("Personal").action(|| {}),
                    button("Archive").action(|| {}),
                ))
                .spacing(8.0),
            )
            .a11y_label("chip-rail"),))
        },
        340,
        300,
    );
    app.settle();
    let chip_width = node_bounds_with_role(app.tree(), Role::Button, "All")
        .width()
        .max(node_bounds_with_role(app.tree(), Role::Button, "Personal").width())
        .max(node_bounds_with_role(app.tree(), Role::Button, "Archive").width());
    let rail = node_bounds(app.tree(), "chip-rail");
    assert!(
        (rail.width() - chip_width).abs() < 1.0,
        "vertical scroll should hug the content's width instead of claiming \
         the stack's offer (#1208): rail={rail:?} content width={chip_width}"
    );
}

/// Content that itself fills the non-scrolling axis still fills it through
/// the scroll: a horizontal scroll of `max_height(INFINITY)` content in a
/// `VStack` claims the whole height offer.
#[test]
fn scrollview_fills_the_non_scrolling_axis_with_filling_content() {
    let mut app = mount_rendered(
        Environment::new(),
        || vstack((scroll_horizontal(chips().max_height(f32::INFINITY)).a11y_label("chip-rail"),)),
        340,
        300,
    );
    app.settle();
    let chip_height = node_bounds_with_role(app.tree(), Role::Button, "All").height();
    let rail = node_bounds(app.tree(), "chip-rail");
    assert!(
        rail.height() > 3.0 * chip_height,
        "a scroll of height-filling content fills the stack's offer (#1208): \
         rail={rail:?} chip height={chip_height}"
    );
}

// Issue #1259: the controller's `position()` reports the last measured
// actual position — user scrolls and applied requests — and never a
// pending request's target.
#[test]
fn scroll_position_reports_actual_geometry_not_pending_requests() {
    let offset = waterui::binding(waterui::layout::Point::zero());
    let controller = waterui_layout::scroll::ScrollController::new(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        Environment::new(),
        {
            let controller = controller.clone();
            move || offset_rail(offset.clone(), Some(controller.clone()))
        },
        180,
        180,
    );
    app.settle();

    // The first real layout reports the viewport's actual geometry.
    let position = controller
        .position()
        .snapshot()
        .expect("the first painted layout reports a position");
    assert_eq!(position.offset, waterui::layout::Point::zero());
    assert!(position.at_start && !position.at_end);
    assert_eq!(
        position.viewport_size,
        waterui::layout::Size::new(120.0, 120.0)
    );
    assert!(position.content_size.height > position.viewport_size.height);

    // A user scroll moves the reported offset.
    let rail = node_id(app.tree(), "offset-rail");
    app.scroll_down(rail);
    let scrolled = controller.position().snapshot().unwrap();
    assert!(
        scrolled.offset.y > 0.0,
        "a user scroll must report the new offset, got {:?}",
        scrolled.offset
    );

    // A request not yet applied stays pending: position() still reports the
    // last actual offset, not the requested target.
    controller.scroll_to(waterui::layout::Point::new(0.0, 200.0));
    assert_eq!(
        controller.position().snapshot().unwrap().offset,
        scrolled.offset,
        "a pending request must never pose as the actual position"
    );

    // The next frame applies the request, and the actual offset reports it.
    app.pump_for(std::time::Duration::from_millis(16));
    let applied = controller.position().snapshot().unwrap();
    assert!(
        applied.offset.y > scrolled.offset.y && applied.at_end,
        "the applied request reports the landed offset, clamped at the extent: {applied:?}"
    );
}
