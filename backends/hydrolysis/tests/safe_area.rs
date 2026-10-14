//! Host↔renderer boundary tests for §7.1's window safe-area regions: an
//! environment carrying [`hydrolysis::WindowSafeArea`] /
//! [`hydrolysis::WindowKeyboardArea`] places the root content inside the
//! deepest region it does not ignore, `.ignore_safe_area` releases only the
//! edges the wrapper's laid-out frame touches, a background fill extends its
//! paint through both regions on every edge it touches, and a scroll surface
//! extends under the regions on the edges it touches, insets its content by
//! them and scrolls a focused field the minimum distance that brings it clear.
//!
//! Layout facts are read from accessibility-tree bounds. Paint facts —
//! whether a fill actually extended into a band — have no a11y or layout
//! signal by design (extension never changes layout, hit or a11y bounds), so
//! they are captured as snapshots and read visually.

use std::time::Duration;

use waterui::component::list::{List, ListItem};
use waterui::graphics::color::Srgb;
use waterui::id::SelfId;
use waterui::layout::frame::Frame;
use waterui::layout::padding::EdgeInsets;
use waterui::layout::safe_area::{EdgeSet, IgnoreSafeArea, SafeAreaRegions};
use waterui::layout::scroll::ScrollView;
use waterui::navigation::NavigationView;
use waterui::prelude::*;
use waterui::{AnyView, Binding, Color, View};
mod support;

use accesskit::Role;
use support::{
    f64_as_f32, mount_rendered, node_bounds, node_bounds_only_role, node_bounds_with_role,
    node_exists, node_id, node_id_with_role,
};

/// The window insets these tests publish — a status bar at the top, a
/// navigation bar at the bottom and a small cutout on the leading edge.
const SAFE_INSETS: EdgeInsets = EdgeInsets::new(48.0, 34.0, 8.0, 8.0);

/// A keyboard region on the bottom edge, published separately from the
/// container insets the way an Android host reports `ime` next to `system_bars`.
const KEYBOARD_INSETS: EdgeInsets = EdgeInsets::new(0.0, 336.0, 0.0, 0.0);

/// The window-space y where the keyboard band starts for the test viewport.
const KEYBOARD_TOP: f64 = 508.0;

/// The window-space y where the container (navigation bar) band starts.
const CONTAINER_TOP: f64 = 844.0 - 34.0;

fn card(label: &'static str) -> impl View {
    text(label).body().foreground(Srgb::WHITE)
}

/// A labelled probe carrying an `.ignore_safe_area` release: the label's
/// bounds land on the released boundary only when the edge its frame
/// touches is reachable — a covered edge releases nothing, so the frame
/// discriminates the two.
fn edge_probe(
    view: impl View,
    label: impl signal::IntoComputed<Str>,
    ignore: impl Into<IgnoreSafeArea>,
) -> impl View {
    view.ignore_safe_area(ignore).a11y_label(label)
}

fn env_with_insets(insets: &Binding<EdgeInsets>) -> Environment {
    let mut env = Environment::new();
    env.insert(hydrolysis::WindowSafeArea(insets.clone()));
    env
}

fn env_with_keyboard(
    container: &Binding<EdgeInsets>,
    keyboard: &Binding<EdgeInsets>,
) -> Environment {
    let mut env = env_with_insets(container);
    env.insert(hydrolysis::WindowKeyboardArea(keyboard.clone()));
    env
}

/// §7.1 "Layout avoids the regions": the root lays out inside the deepest
/// region on every edge.
#[test]
fn root_content_stays_inside_the_safe_area() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || vstack((card("inbox"),)).alignment(HorizontalAlignment::Leading),
        390,
        844,
    );

    let bounds = node_bounds_with_role(app.tree(), Role::Label, "inbox");
    assert!(
        (bounds.x0 - f64::from(SAFE_INSETS.leading())).abs() <= 1.0
            && (bounds.y0 - f64::from(SAFE_INSETS.top())).abs() <= 1.0,
        "root content should start at the safe-area origin \
         ({}, {}), got {bounds:?}",
        f64::from(SAFE_INSETS.leading()),
        f64::from(SAFE_INSETS.top())
    );
}

/// `.ignore_safe_area(ALL)` lays the root content out edge-to-edge.
#[test]
fn ignore_safe_area_all_reaches_the_window_origin() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            vstack((card("chrome"),))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(EdgeSet::ALL)
        },
        390,
        844,
    );

    let bounds = node_bounds_with_role(app.tree(), Role::Label, "chrome");
    assert!(
        bounds.x0.abs() <= 1.0 && bounds.y0.abs() <= 1.0,
        "ignore_safe_area(ALL) content should start at the window origin, got {bounds:?}"
    );
}

/// `.ignore_safe_area(TOP)` releases only the top inset: the content slides
/// under the status bar but keeps the leading safe edge.
#[test]
fn ignore_safe_area_releases_only_the_flagged_edges() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            vstack((card("header"),))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(EdgeSet::TOP)
        },
        390,
        844,
    );

    let bounds = node_bounds_with_role(app.tree(), Role::Label, "header");
    assert!(
        bounds.y0.abs() <= 1.0,
        "a TOP-released view should reach the window top, got {bounds:?}"
    );
    assert!(
        (bounds.x0 - f64::from(SAFE_INSETS.leading())).abs() <= 1.0,
        "the leading edge stays inside the safe area, got {bounds:?}"
    );
}

/// §7.1 "touches": a release applies only on an edge the wrapper's laid-out
/// frame ends on. The first child of a vstack ends on the top boundary and
/// its TOP declaration releases it to the window edge; the second child ends
/// mid-surface — the same declaration releases nothing for it.
#[test]
fn an_ignore_releases_only_on_an_edge_the_frame_touches() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            vstack((
                card("top-touches").ignore_safe_area(EdgeSet::TOP),
                card("below").ignore_safe_area(EdgeSet::TOP),
            ))
            .alignment(HorizontalAlignment::Leading)
        },
        390,
        844,
    );

    let top = node_bounds_with_role(app.tree(), Role::Label, "top-touches");
    assert!(
        top.y0.abs() <= 1.0,
        "the touching edge releases to the window origin, got {top:?}"
    );

    let below = node_bounds_with_role(app.tree(), Role::Label, "below");
    // "below" sits one spacing under the first child's released height. A
    // release would have shifted its origin up by the container depth; its
    // frame does not touch the top boundary, so nothing releases.
    assert!(
        (below.y0 - (top.height() + 10.0)).abs() <= 1.0,
        "a non-touching edge releases nothing, got {below:?}"
    );
}

/// A later insets write — rotation, split-screen or the IME arriving —
/// re-lays the window out against the new value.
#[test]
fn a_safe_area_change_relays_out_the_root() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || vstack((card("inbox"),)).alignment(HorizontalAlignment::Leading),
        390,
        844,
    );
    let first = node_bounds(app.tree(), "inbox");

    insets.set(EdgeInsets::new(80.0, 34.0, 24.0, 8.0));
    app.settle();

    let moved = node_bounds(app.tree(), "inbox");
    assert!(
        (moved.y0 - 80.0).abs() <= 1.0 && (moved.x0 - 24.0).abs() <= 1.0,
        "the root should track the updated insets: was {first:?}, now {moved:?}"
    );
}

/// A navigation stack records each page in the page's own space and presents
/// it at the stack's place in the window. The page's hit targets and
/// accessibility bounds have to follow it there: under a top inset the link a
/// finger lands on is the one drawn below the status bar, not one inset above
/// it. Tapping the reported bounds must also activate the link.
#[test]
fn navigation_page_targets_follow_the_safe_area() {
    use waterui::navigation::{NavigationLink, NavigationStack, NavigationView};

    fn stack() -> impl View {
        NavigationStack::new(NavigationView::new(
            "Root",
            vstack((NavigationLink::new("Open Detail", || {
                NavigationView::new("Detail", text("detail"))
            }),)),
        ))
    }

    let no_insets = waterui::binding(EdgeInsets::default());
    let mut bare = mount_rendered(env_with_insets(&no_insets), stack, 390, 844);
    let unshifted = node_bounds_with_role(bare.tree(), Role::Button, "Open Detail");

    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(env_with_insets(&insets), stack, 390, 844);
    let shifted = node_bounds_with_role(app.tree(), Role::Button, "Open Detail");
    // The link is centred in the page, so the horizontal insets move it by
    // half their difference; the top inset moves it down whole.
    let expected_x =
        unshifted.x0 + (f64::from(SAFE_INSETS.leading()) - f64::from(SAFE_INSETS.trailing())) / 2.0;
    let expected_y = unshifted.y0 + f64::from(SAFE_INSETS.top());
    assert!(
        (shifted.x0 - expected_x).abs() <= 1.0 && (shifted.y0 - expected_y).abs() <= 1.0,
        "the page's link should sit inside the safe area at ({expected_x}, {expected_y}): \
         {unshifted:?} without insets, {shifted:?} with {SAFE_INSETS:?}"
    );

    let link_id = node_id_with_role(app.tree(), Role::Button, "Open Detail");
    app.tap_node(link_id, 0.5, 0.5);
    app.settle();
    assert!(
        node_exists(app.tree(), Role::Button, "Back"),
        "a tap on the link's reported bounds must push the detail page"
    );
}

/// The context a retained sub-view's nodes were laid out against lasts
/// the node's lifetime, not the frame's: nodes inside an unchanged
/// retained sub-view are not re-laid out on steady frames. The
/// navigation chrome that hosts a page lives inside the stack's own
/// retained subtree — if its context expired with the frame, the next
/// steady-frame flush would hand the page content `None`, the page would
/// re-lay out context-free and the probe's `.ignore_safe_area` release
/// would silently release nothing, ending at the keyboard boundary
/// instead of the released one.
#[test]
fn a_retained_navigation_page_keeps_its_context_on_steady_frames() {
    use waterui::navigation::NavigationStack;

    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let draft = Binding::container(waterui::Str::from(""));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            NavigationStack::new(NavigationView::new(
                "Compose",
                vstack((
                    field("composer", &draft),
                    edge_probe(
                        vstack((card("compose"), spacer())),
                        "compose-area",
                        SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                    ),
                )),
            ))
        },
        390,
        844,
    );
    app.settle();
    // Focusing the field invalidates a frame without moving the page's
    // layout: the retained subtree flushes on a steady frame, where a
    // frame-scoped context would already be gone.
    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "composer");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(250));

    let probe = node_bounds(app.tree(), "compose-area");
    assert!(
        (probe.y0 + probe.height() - CONTAINER_TOP).abs() <= 1.0,
        "the released page should still end at {CONTAINER_TOP} on a steady \
         frame, got {probe:?}"
    );
}

/// Focused-field clearance through a retained page survives steady frames:
/// the scroll surface's facts are laid out only while the hosting widget
/// still has a context, so a second field focused under the keyboard band
/// must still scroll clear inside a `NavigationStack`.
#[test]
fn a_second_field_in_a_navigation_stack_still_clears_the_keyboard() {
    use waterui::navigation::NavigationStack;

    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let first = Binding::container(waterui::Str::from(""));
    let second = Binding::container(waterui::Str::from(""));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            NavigationStack::new(NavigationView::new(
                "Compose",
                ScrollView::vertical(vstack((
                    field("First", &first).size(350.0, 44.0),
                    spacer().size(390.0, 400.0),
                    field("Second", &second).size(350.0, 44.0),
                    spacer().size(390.0, 300.0),
                ))),
            ))
        },
        390,
        844,
    );
    app.settle();

    let covered = node_bounds_with_role(app.tree(), Role::TextInput, "Second");
    assert!(
        covered.y0 + covered.height() > KEYBOARD_TOP,
        "precondition: the second field starts under the keyboard band, \
         got {covered:?}"
    );

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "First");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));
    app.pump_for(Duration::from_secs(1));

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "Second");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));
    app.pump_for(Duration::from_secs(1));

    let cleared = node_bounds_with_role(app.tree(), Role::TextInput, "Second");
    assert!(
        (cleared.y0 + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the second field scrolls clear of the keyboard, got {cleared:?}"
    );
}

/// A fill's extension reaches through wrappers that never read bounds: an
/// `Opacity` over the color is still a fill, so the translucent paint runs
/// on through the keyboard band to the window edge exactly as the opaque
/// case does (snapshot).
#[test]
fn a_fill_inside_opacity_still_extends() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((card("content"), spacer()))
                .alignment(HorizontalAlignment::Leading)
                .background(Color::new(Srgb::new(0.2, 0.5, 0.9)).opacity(0.6))
        },
        390,
        844,
    );

    let content = node_bounds_with_role(app.tree(), Role::Label, "content");
    assert!(
        (content.y0 - f64::from(SAFE_INSETS.top())).abs() <= 1.0,
        "the content frame keeps the boundary, got {content:?}"
    );
    app.capture_snapshot("safe-area", "fill-inside-opacity", "keyboard");
}

/// `SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM)` lays the view out under the
/// keyboard but still clear of the container (navigation-bar) inset: the
/// bottom edge slides under the keyboard band and stops at
/// `window height - container bottom`.
#[test]
fn ignoring_only_the_keyboard_region_keeps_the_container_inset() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((spacer(), card("composer")))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM))
        },
        390,
        844,
    );

    let bounds = node_bounds_with_role(app.tree(), Role::Label, "composer");
    assert!(
        (bounds.y0 + bounds.height() - CONTAINER_TOP).abs() <= 1.0,
        "the keyboard region is released but the container inset still binds: \
         expected bottom {CONTAINER_TOP}, got {bounds:?}"
    );
}

/// `SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM)` with the keyboard up
/// releases nothing: the keyboard region is the deeper one on that edge and
/// still binds, so the frame ends on the keyboard boundary exactly as if no
/// declaration were present.
#[test]
fn ignoring_only_the_container_region_releases_nothing_under_the_keyboard() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((spacer(), card("composer")))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM))
        },
        390,
        844,
    );

    let bounds = node_bounds_with_role(app.tree(), Role::Label, "composer");
    assert!(
        (bounds.y0 + bounds.height() - KEYBOARD_TOP).abs() <= 1.0,
        "the keyboard region still binds: expected bottom {KEYBOARD_TOP}, got {bounds:?}"
    );

    // With the keyboard down the container region is the deepest one on the
    // bottom edge, and the declaration releases it: the frame slides under
    // the navigation bar by the container inset and ends on the window edge.
    keyboard.set(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));
    app.settle();

    let released = node_bounds_with_role(app.tree(), Role::Label, "composer");
    assert!(
        (released.y0 + released.height() - 844.0).abs() <= 1.0,
        "the CONTAINER release ends on the window edge — the bottom grew by \
         the container inset (34) once the keyboard left, got {released:?}"
    );
}

/// §7.1 "Paint extends for fills": a background fill whose frame touches the
/// bottom boundary paints through the keyboard and container bands to the
/// window edge. The paint extension is a snapshot-only fact — the fill's own
/// frame (and every a11y bound) stays on the boundary.
#[test]
fn a_fill_background_paints_under_the_keyboard_region() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                spacer(),
                text("composer")
                    .body()
                    .foreground(Srgb::WHITE)
                    .padding_with(8.0)
                    .background(Srgb::new(0.0, 0.35, 0.85)),
            ))
            .alignment(HorizontalAlignment::Leading)
        },
        390,
        844,
    );

    let bounds = node_bounds_with_role(app.tree(), Role::Label, "composer");
    // The text sits 8 pt inside its `padding_with(8.0)` chrome; the padded
    // frame, not the label, is what ends on the keyboard boundary.
    assert!(
        (bounds.y0 + bounds.height() - (KEYBOARD_TOP - 8.0)).abs() <= 1.0,
        "the content's frame ends on the keyboard boundary, got {bounds:?}"
    );
    app.capture_snapshot("safe-area", "fill-under-keyboard", "keyboard");
}

/// An `.ignore_safe_area` on an ancestor does not replace the fill's own
/// default: a TOP declaration on the tree leaves a bottom background fill
/// extending under the keyboard region.
#[test]
fn a_fill_under_an_ancestor_top_ignore_still_extends_at_the_bottom() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                card("header"),
                spacer(),
                text("composer")
                    .body()
                    .foreground(Srgb::WHITE)
                    .padding_with(8.0)
                    .background(Srgb::new(0.0, 0.35, 0.85)),
            ))
            .alignment(HorizontalAlignment::Leading)
            .ignore_safe_area(EdgeSet::TOP)
        },
        390,
        844,
    );

    let header = node_bounds_with_role(app.tree(), Role::Label, "header");
    assert!(
        header.y0.abs() <= 1.0,
        "the ancestor's TOP ignore released the header under the status bar, got {header:?}"
    );
    let composer = node_bounds_with_role(app.tree(), Role::Label, "composer");
    assert!(
        (composer.y0 + composer.height() - (KEYBOARD_TOP - 8.0)).abs() <= 1.0,
        "the composer frame still ends on the keyboard boundary, got {composer:?}"
    );
    app.capture_snapshot("safe-area", "fill-under-ancestor-top-ignore", "keyboard");
}

/// A background fill inside a `KEYBOARD.on(BOTTOM)` subtree ignores the
/// keyboard region (its frame ends on the container boundary) but still
/// extends through the container band it does not ignore.
#[test]
fn a_fill_under_a_keyboard_ignore_extends_through_the_container_band() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                spacer(),
                text("composer")
                    .body()
                    .foreground(Srgb::WHITE)
                    .padding_with(8.0)
                    .background(Srgb::new(0.0, 0.35, 0.85)),
            ))
            .alignment(HorizontalAlignment::Leading)
            .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM))
        },
        390,
        844,
    );

    let composer = node_bounds_with_role(app.tree(), Role::Label, "composer");
    assert!(
        (composer.y0 + composer.height() - (CONTAINER_TOP - 8.0)).abs() <= 1.0,
        "the composer frame ends on the container boundary, got {composer:?}"
    );
    app.capture_snapshot("safe-area", "fill-under-keyboard-ignore", "keyboard");
}

/// §7.1's fill rule names background fills only: a `Divider` at the bottom
/// edge and a bare `Color` strip at the top edge keep their own frames — no
/// paint reaches into a region for them (snapshot read).
#[test]
fn non_fill_leaves_at_an_edge_do_not_extend() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                Color::new(Srgb::new(0.8, 0.1, 0.1)).size(390.0, 4.0),
                card("content"),
                spacer(),
                Divider,
            ))
            .alignment(HorizontalAlignment::Leading)
        },
        390,
        844,
    );

    let content = node_bounds_with_role(app.tree(), Role::Label, "content");
    assert!(
        (content.y0 - (f64::from(SAFE_INSETS.top()) + 4.0 + 10.0)).abs() <= 1.0,
        "the color strip's laid-out frame stays inside the container inset, \
         got {content:?}"
    );
    app.capture_snapshot("safe-area", "non-fill-leaves-at-edges", "keyboard");
}

/// A non-fill background — `.background` of a view rather than a color —
/// extends only where it ignores the safe area, so with no declaration it
/// paints no further than its laid-out frame even at the bottom edge. The
/// paint extension never changes a layout bound, so the snapshot is the
/// check: the band stays the window background.
#[test]
fn a_non_fill_background_stays_inside_the_keyboard_region() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                spacer(),
                text("composer")
                    .body()
                    .foreground(Srgb::WHITE)
                    .padding_with(8.0)
                    .background(text("backdrop").foreground(Srgb::new(0.0, 0.5, 0.0))),
            ))
            .alignment(HorizontalAlignment::Leading)
        },
        390,
        844,
    );

    app.capture_snapshot("safe-area", "non-fill-under-keyboard", "keyboard");
}

/// §7.1 "Scroll surfaces": a scroll surface covering the keyboard band
/// scrolls a focused text field the minimum distance that brings it clear —
/// its bottom lands exactly on the keyboard boundary — on focus, and again
/// while the keyboard inset grows.
#[test]
fn a_scroll_surface_scrolls_the_focused_field_clear_of_the_keyboard() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            ScrollView::vertical(vstack((
                spacer().size(390.0, 560.0),
                field("Message", &value).size(350.0, 44.0),
                spacer().size(390.0, 380.0),
            )))
        },
        390,
        844,
    );

    // The surface extends under the keyboard, so the field at the bottom of the
    // content is painted (and reported) inside the band it must escape.
    let covered = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        covered.y0 + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition: the field starts under the keyboard band, got {covered:?}"
    );

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "Message");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));

    let cleared = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (cleared.y0 + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the field's bottom lands exactly on the keyboard boundary, got {cleared:?}"
    );

    keyboard.set(EdgeInsets::new(0.0, 400.0, 0.0, 0.0));
    app.pump_for(Duration::from_millis(500));

    let tracked = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (tracked.y0 + tracked.height() - (844.0 - 400.0)).abs() <= 0.5,
        "the surface tracks the growing keyboard inset, got {tracked:?}"
    );
}

/// The clearance bound is `min(keyboard top, the surface's own frame bottom)`:
/// a `vstack((scroll(form), toolbar))` whose scroll frame ends above the
/// keyboard scrolls the field to the frame's bottom, not into the toolbar.
#[test]
fn a_focused_field_clears_to_the_surface_frame_above_a_toolbar() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                ScrollView::vertical(vstack((
                    spacer().size(390.0, 560.0),
                    field("Message", &value).size(350.0, 44.0),
                    spacer().size(390.0, 380.0),
                ))),
                card("toolbar").size(390.0, 56.0),
            ))
        },
        390,
        844,
    );

    // Scroll frame: root content (48..508) minus the toolbar's 56 and the
    // vstack's 10 spacing → 48..442.
    let surface_bottom = KEYBOARD_TOP - 56.0 - 10.0;

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "Message");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));
    // The clearance glides; pump it fully past the settle cap so the
    // assertion reads the converged position.
    app.pump_for(Duration::from_secs(1));

    let cleared = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (cleared.y0 + cleared.height() - surface_bottom).abs() <= 0.5,
        "the field clears to the scroll surface's own frame bottom \
         ({surface_bottom}), got {cleared:?}"
    );
}

/// A `List` is a scroll surface: a text field in a row under the keyboard
/// band is scrolled clear to the list's frame bottom on focus.
#[test]
fn a_list_row_field_scrolls_clear_of_the_keyboard() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let field_row = 10;
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            List::for_each((0..40).map(SelfId::new).collect::<Vec<_>>(), {
                let value = value.clone();
                move |item| {
                    if *item == field_row {
                        ListItem::new(AnyView::new(field("Message", &value)))
                    } else {
                        ListItem::new(AnyView::new(text(format!("row-{}", *item))))
                    }
                }
            })
        },
        390,
        844,
    );

    let covered = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        covered.y0 + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition: the row field starts under the keyboard band, got {covered:?}"
    );

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "Message");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));
    app.pump_for(Duration::from_secs(1));

    let cleared = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (cleared.y0 + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the list scrolls the field's bottom onto the keyboard boundary, got {cleared:?}"
    );
}

/// The scroll surface's lazy viewport covers the extended frame: a lazy stack
/// builds the rows laid out under the keyboard band, not just the avoided
/// window.
#[test]
fn a_lazy_stack_in_a_scroll_builds_rows_under_the_keyboard_band() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            ScrollView::vertical(VStack::for_each(
                (0..40).map(SelfId::new).collect::<Vec<_>>(),
                |item| Frame::new(text(format!("row-{}", *item))).size(390.0, 30.0),
            ))
        },
        390,
        844,
    );

    // Row 15 sits at window y ≈ 48 + 15 * (30 + 10) = 648 — inside the
    // keyboard band. Without the surface's extension the lazy viewport ends
    // at the avoided bottom (508) and the row is never built.
    let row = node_bounds_with_role(app.tree(), Role::Label, "row-15");
    assert!(
        row.y0 > KEYBOARD_TOP && row.y0 < 844.0,
        "the row under the keyboard band is built and placed, got {row:?}"
    );
}

/// §7.1 "never fights a user scroll": a multi-frame touch drag on a surface
/// under non-zero regions keeps scrolling — every frame's layout rebind keeps
/// the live drag handle valid instead of dropping the gesture after the
/// first frame.
#[test]
fn a_touch_drag_over_several_frames_keeps_scrolling_under_nonzero_insets() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            ScrollView::vertical(vstack(
                (0..30)
                    .map(|i| Frame::new(text(format!("row-{i}"))).size(390.0, 30.0))
                    .collect::<Vec<_>>(),
            ))
        },
        390,
        844,
    );

    // A touch drag is what claims a scroll view on a real device; the host
    // reports its gesture constants through `PlatformWindow::touch_scroll_config`.
    app.set_touch_scroll_config(hydrolysis::TouchScrollConfig::android_default());

    let before = node_bounds_with_role(app.tree(), Role::Label, "row-8");

    // A 250 pt drag sampled over five frames: each move lands on its own
    // pump, so a stale scroll handle would drop everything after frame one.
    app.queue_drag_from_to_with(
        195.0,
        400.0,
        195.0,
        150.0,
        5,
        true,
        hydrolysis::PointerKind::Touch,
    );
    app.pump_for(Duration::from_millis(500));
    app.settle();

    let after = node_bounds_with_role(app.tree(), Role::Label, "row-8");
    assert!(
        before.y0 - after.y0 >= 100.0,
        "the drag scrolled the content substantially across frames: \
         was {before:?}, now {after:?}"
    );
}

/// Fractional geometry (§7.1's "touches" resolved at display resolution):
/// Android-density insets and a text-height composer leave the f32-placed
/// frame a sub-pixel off the boundary. A touch within half a physical
/// pixel at the window's scale is still a touch — the nested declaration
/// on the grown frame edge releases to the window edge.
#[test]
fn a_fractional_geometry_still_touches_the_boundary() {
    // 2.75x-density container insets: nothing in this geometry is an
    // integer — not the boundary (844 - 24.38), not the release amounts,
    // not the composer's measured text height.
    const FRACTIONAL_INSETS: EdgeInsets = EdgeInsets::new(47.24, 24.38, 8.2, 8.6);
    let container = waterui::binding(FRACTIONAL_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = {
        let mut app = mount_rendered(
            env_with_keyboard(&container, &keyboard),
            move || {
                vstack((
                    spacer(),
                    vstack((spacer(), card("composer")))
                        .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM)),
                ))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM))
            },
            390,
            844,
        );
        app.set_scale_factor(2.75);
        app
    };

    // The outer release moves the bottom boundary to the container edge
    // (819.62); the inner stack's f32-grown frame edge lands a sub-pixel
    // off it. Its CONTAINER declaration still touches and releases the
    // composer through the navigation-bar band to the window edge.
    let composer = node_bounds_with_role(app.tree(), Role::Label, "composer");
    assert!(
        (composer.y0 + composer.height() - 844.0).abs() <= 1.0,
        "the nested declaration touches within half a physical pixel and \
         releases to the window edge, got {composer:?}"
    );
    app.capture_snapshot("safe-area", "fractional-geometry-touch", "keyboard");
}

/// A declaration ON the fill replaces the default extension — including an
/// empty one: a background `Color` carrying `.ignore_safe_area(EdgeSet::NONE)`
/// names no edge, so it releases nothing and no default extension applies.
/// The snapshot is the check: the bands stay the window background.
#[test]
fn a_fill_with_an_empty_ignore_extends_nowhere() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                spacer(),
                text("composer")
                    .body()
                    .foreground(Srgb::WHITE)
                    .padding_with(8.0)
                    .background(
                        Color::new(Srgb::new(0.0, 0.35, 0.85)).ignore_safe_area(EdgeSet::NONE),
                    ),
            ))
            .alignment(HorizontalAlignment::Leading)
        },
        390,
        844,
    );

    let bounds = node_bounds_with_role(app.tree(), Role::Label, "composer");
    // The declaration names no edges, so the padded frame still ends on the
    // keyboard boundary — the layout precondition a default extension
    // would share.
    assert!(
        (bounds.y0 + bounds.height() - (KEYBOARD_TOP - 8.0)).abs() <= 1.0,
        "the frame still ends on the boundary, got {bounds:?}"
    );
    app.capture_snapshot("safe-area", "fill-declared-none", "keyboard");
}

/// `.ignore_safe_area(EdgeSet::BOTTOM)` on a fill replaces the default on
/// every edge: a full-height background touches both the top and bottom
/// boundaries, and only the named edge reaches the window edge — the
/// status-bar band keeps the window background (snapshot).
#[test]
fn a_fill_ignore_names_the_only_edge_that_extends() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((card("content"), spacer()))
                .alignment(HorizontalAlignment::Leading)
                .background(Color::new(Srgb::new(0.75, 0.4, 0.0)).ignore_safe_area(EdgeSet::BOTTOM))
        },
        390,
        844,
    );

    // The named bottom edge's release is the fill's whole extension. The
    // assertion is a precondition — the card keeps the container inset —
    // and the snapshot is the check: the background paints to the window
    // bottom edge and nowhere else.
    let content = node_bounds_with_role(app.tree(), Role::Label, "content");
    assert!(
        (content.y0 - f64::from(SAFE_INSETS.top())).abs() <= 1.0,
        "the content frame keeps the container inset, got {content:?}"
    );
    app.capture_snapshot("safe-area", "fill-ignore-bottom-edge-only", "keyboard");
}

/// §7.1 chrome containers: a form inside `NavigationView` content inherits
/// the host's boundaries — it extends under the keyboard and clears a
/// focused field exactly as a root-level surface does. Runs at scale 1 and
/// at 2.625, a real device density: the hosted frame maps from the layout
/// record, so the clearance lands on the boundary in logical units
/// whatever the window's scale.
fn navigation_hosted_form_clears_the_focused_field(scale: f64) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = {
        let mut app = mount_rendered(
            env_with_keyboard(&container, &keyboard),
            move || {
                NavigationView::new(
                    "Compose",
                    ScrollView::vertical(vstack((
                        spacer().size(390.0, 560.0),
                        field("Message", &value).size(350.0, 44.0),
                        spacer().size(390.0, 380.0),
                    ))),
                )
            },
            390,
            844,
        );
        app.set_scale_factor(scale);
        app
    };

    let covered = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        covered.y0 + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition at scale {scale}: the field starts under the keyboard \
         band, got {covered:?}"
    );

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "Message");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));
    app.pump_for(Duration::from_secs(1));

    let cleared = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (cleared.y0 + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "at scale {scale} the navigation-hosted surface clears the field to \
         the keyboard boundary, got {cleared:?}"
    );
}

#[test]
fn a_form_inside_navigation_content_clears_the_focused_field() {
    navigation_hosted_form_clears_the_focused_field(1.0);
}

#[test]
fn a_form_inside_navigation_content_clears_the_focused_field_at_device_scale() {
    navigation_hosted_form_clears_the_focused_field(2.625);
}

/// A navigation page's background fill stays inside the content frame: the
/// bar's band is not a boundary the content inherited, so the fill never
/// paints over the bar or the status-bar band, and rows scrolled to the
/// top stop at the bar's inner edge. Paint is the check — the snapshot
/// shows the fill beginning at the bar and the scrolled rows clipping at
/// the bar's inner edge, never covering the title.
#[test]
fn a_navigation_page_background_stays_below_the_bar() {
    let insets = waterui::binding(SAFE_INSETS);
    let offset = waterui::binding(waterui::layout::Point::zero());
    let mut app = mount_rendered(
        env_with_insets(&insets),
        {
            let offset = offset.clone();
            move || {
                NavigationView::new(
                    "Home",
                    ScrollView::vertical(vstack((
                        card("row one"),
                        card("row two"),
                        card("row three"),
                        spacer().size(390.0, 1400.0),
                    )))
                    .report_offset(&offset)
                    .background(Color::new(Srgb::new(0.85, 0.2, 0.3)))
                    .a11y_label("page-scroll"),
                )
            }
        },
        390,
        844,
    );
    app.settle();

    // Read the bar's own bounds — the navigation-bar a11y node carries the
    // laid-out bar rect, so the test does not restate the theme metric.
    let bar = node_bounds_only_role(app.tree(), Role::Navigation);
    let scroll = node_bounds(app.tree(), "page-scroll");
    assert!(
        (scroll.y0 - (bar.y0 + bar.height())).abs() <= 1.0,
        "the page's scroll surface starts at the bar's inner edge, \
         got scroll {scroll:?} with bar {bar:?}"
    );

    {
        let id = node_id(app.tree(), "page-scroll");
        app.scroll_down(id);
    }
    app.settle();
    assert!(
        offset.snapshot().y > 1.0,
        "the page scrolls under the bar: the offset must move, got {:?}",
        offset.snapshot()
    );
    app.capture_snapshot("safe-area", "nav-page-background", "insets");
}

/// `.ignore_safe_area(EdgeSet::TOP)` inside navigation content releases
/// nothing it does not touch: the page's top edge is not on the host's top
/// boundary — the bar's band covers it — so the hero stays at the content
/// top, below the bar (48 inset + the bar's height), never at the window
/// edge.
#[test]
fn an_ignore_top_inside_navigation_content_stays_below_the_bar() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            NavigationView::new(
                "Hero",
                vstack((card("hero").ignore_safe_area(EdgeSet::TOP), card("below"))),
            )
        },
        390,
        844,
    );

    let bar = node_bounds_only_role(app.tree(), Role::Navigation);
    let hero = node_bounds_with_role(app.tree(), Role::Label, "hero");
    assert!(
        (hero.y0 - (bar.y0 + bar.height())).abs() <= 1.0,
        "the covered top edge releases nothing: the hero stays below the \
         bar, got hero {hero:?} with bar {bar:?}"
    );
}

/// The edge a hosted subtree still touches stays reachable: a navigation
/// page ending on the bottom boundary extends its background fill through
/// the keyboard band to the window edge — the snapshot shows the fill
/// painting the band, the page's frame ending on the boundary. A release
/// probe at the page's bottom edge discriminates reachability: released
/// past the keyboard, it lands on the container boundary.
#[test]
fn a_navigation_page_touching_the_bottom_still_extends_its_fill() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            NavigationView::new(
                "Home",
                vstack((
                    card("content"),
                    spacer(),
                    edge_probe(
                        card("docked"),
                        "docked",
                        SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                    ),
                ))
                .background(Color::new(Srgb::new(0.2, 0.6, 0.35)))
                .a11y_label("page"),
            )
        },
        390,
        844,
    );

    let page = node_bounds(app.tree(), "page");
    assert!(
        (page.y0 + page.height() - KEYBOARD_TOP).abs() <= 1.0,
        "the page's frame still ends on the keyboard boundary, got {page:?}"
    );
    let docked = node_bounds(app.tree(), "docked");
    assert!(
        (docked.y0 + docked.height() - CONTAINER_TOP).abs() <= 1.0,
        "the bottom edge is reachable: the release lands the probe at \
         {CONTAINER_TOP}, got {docked:?}"
    );
    app.capture_snapshot("safe-area", "nav-bottom-fill", "keyboard");
}

/// An anchored overlay's background fill stays at the overlay's own frame:
/// the overlay inherits the window's boundaries, and a tooltip mid-window
/// touches none of them, so its fill extends nowhere — the snapshot shows
/// a small fill under the anchor, not a window-flooding one.
#[test]
fn an_anchored_overlay_fill_stays_at_its_own_frame() {
    use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay};

    let insets = waterui::binding(SAFE_INSETS);
    let open = waterui::binding(true);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            vstack((
                spacer().size(390.0, 200.0),
                text("anchor").body().anchored_overlay(
                    AnchoredOverlay::new(&open, move || {
                        text("tip")
                            .caption()
                            .padding_with(6.0)
                            .background(Color::new(Srgb::new(0.2, 0.7, 0.9)))
                    })
                    .edge(AnchorEdge::Bottom),
                ),
                spacer(),
            ))
            .alignment(HorizontalAlignment::Leading)
        },
        390,
        844,
    );

    let anchor = node_bounds_with_role(app.tree(), Role::Label, "anchor");
    let tip = node_bounds_with_role(app.tree(), Role::Label, "tip");
    assert!(
        tip.y0 >= anchor.y0 + anchor.height() - 1.0,
        "the tip presents below its anchor, anchor {anchor:?} tip {tip:?}"
    );
    assert!(
        tip.width() <= 120.0 && tip.height() <= 80.0,
        "the tip keeps its own small frame rather than flooding the window, \
         got {tip:?}"
    );
    app.capture_snapshot("safe-area", "overlay-fill", "insets");
}

/// A split view's detail background does not cross the sidebar: the
/// detail column's leading edge is not on the host's leading boundary, so
/// it is covered — the snapshot shows the fill starting at the column's
/// leading edge with the sidebar unpainted-over beside it.
#[test]
fn a_split_detail_background_does_not_cross_the_sidebar() {
    use waterui::navigation::NavigationSplitView;

    let insets = waterui::binding(SAFE_INSETS);
    let selection = Binding::container(Some(0i32));
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            NavigationSplitView::new(
                &selection,
                || {
                    List::for_each((0..8).map(SelfId::new).collect::<Vec<_>>(), |item| {
                        ListItem::new(text(format!("side {}", *item)))
                    })
                },
                |id| {
                    NavigationView::new(
                        format!("Detail {id}"),
                        vstack((card("detail-fill"), spacer()))
                            .background(Color::new(Srgb::new(0.85, 0.45, 0.1))),
                    )
                },
            )
        },
        1400,
        900,
    );
    app.settle();

    let sidebar = node_bounds_only_role(app.tree(), Role::List);
    let detail = node_bounds_with_role(app.tree(), Role::Label, "detail-fill");
    assert!(
        detail.x0 >= sidebar.x0 + sidebar.width() - 1.0,
        "the detail column starts at or past the sidebar's trailing edge, \
         sidebar {sidebar:?} detail {detail:?}"
    );
    app.capture_snapshot("safe-area", "split-detail-background", "insets");
}

/// Nested declarations across chrome content never move a boundary inward:
/// hosted content inherits the host's boundary and released regions, so an
/// inner `.ignore_safe_area` can only release further out — never re-cover
/// a band the outer declaration already released. Both directions run:
/// KEYBOARD under the navigation view with CONTAINER inside, and CONTAINER
/// under the navigation view with KEYBOARD inside.
#[test]
fn nested_releases_never_move_the_navigation_boundary_inward() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);

    // Outer releases the keyboard region, inner names the container: both
    // regions end up released, so the inner declaration extends the
    // content outward to the window edge — never inward to a re-covered
    // boundary.
    let mut keyboard_first = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            NavigationView::new(
                "Outer",
                vstack((spacer(), card("inner")))
                    .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM)),
            )
            .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM))
        },
        390,
        844,
    );
    let inner = node_bounds_with_role(keyboard_first.tree(), Role::Label, "inner");
    assert!(
        (inner.y0 + inner.height() - 844.0).abs() <= 1.0,
        "KEYBOARD under nav + CONTAINER inside: the bottom edge lands on the \
         window edge, got {inner:?}"
    );

    // Outer releases the container region, inner names the keyboard: the
    // inner declaration releases outward past the container band to the
    // window edge.
    let mut container_first = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            NavigationView::new(
                "Outer",
                vstack((spacer(), card("inner")))
                    .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM)),
            )
            .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM))
        },
        390,
        844,
    );
    let inner = node_bounds_with_role(container_first.tree(), Role::Label, "inner");
    assert!(
        (inner.y0 + inner.height() - 844.0).abs() <= 1.0,
        "CONTAINER under nav + KEYBOARD inside: the bottom edge lands on the \
         window edge, got {inner:?}"
    );
}

/// `ScrollNode` reads its metrics after `begin_flush`: a keyboard inset
/// change scrolls the field clear inside the early pass, so the very frame
/// the inset arrives already paints the field above the new keyboard top —
/// not one frame later.
#[test]
fn a_keyboard_inset_change_clears_the_field_on_that_frame() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            ScrollView::vertical(vstack((
                spacer().size(390.0, 560.0),
                field("Message", &value).size(350.0, 44.0),
                spacer().size(390.0, 380.0),
            )))
        },
        390,
        844,
    );

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "Message");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));
    app.pump_for(Duration::from_secs(1));

    keyboard.set(EdgeInsets::new(0.0, 400.0, 0.0, 0.0));
    // One frame — not a settle. The clearance must already be painted.
    app.pump_for(Duration::from_millis(16));

    let tracked = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (tracked.y0 + tracked.height() - (844.0 - 400.0)).abs() <= 0.5,
        "the field is already clear on the frame the inset arrived, got {tracked:?}"
    );
}

/// A field inside a scroll nested in a scroll is claimed once by the
/// surface that can clear it: the inner surface's content lays out without
/// a safe-area context, so the outer surface owns the clearance — the
/// field lands on the keyboard boundary, never over-scrolled by a second
/// surface reading the field's pre-clearance rect.
#[test]
fn a_field_in_a_nested_scroll_is_cleared_once() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            ScrollView::vertical(vstack((
                spacer().size(390.0, 430.0),
                ScrollView::vertical(vstack((
                    spacer().size(350.0, 80.0),
                    field("Message", &value).size(350.0, 44.0),
                    spacer().size(350.0, 300.0),
                )))
                .size(350.0, 200.0),
                spacer().size(390.0, 380.0),
            )))
        },
        390,
        844,
    );

    // The inner surface is context-free (§7.1: a scroll surface's content
    // stays context-free): its field at 510..554 sits under the keyboard
    // band inside the outer surface's extended window.
    let covered = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        covered.y0 + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition: the nested field starts under the keyboard band, got {covered:?}"
    );

    {
        let id = node_id_with_role(app.tree(), Role::TextInput, "Message");
        app.focus_quiet(id);
    }
    app.pump_for(Duration::from_millis(500));
    app.pump_for(Duration::from_secs(1));

    let cleared = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (cleared.y0 + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the owning surface clears the nested field exactly once, got {cleared:?}"
    );
}

/// Tab content above the bottom bar is `Covered` on its bottom edge: the
/// page's frame ends at the bar's top, the bar's bottom edge sits on the
/// container boundary, and a background fill inside extends nowhere
/// through the bar — the snapshot shows the fill stopping at the tab
/// bar's top edge.
#[test]
fn a_tab_page_background_stops_at_the_tab_bar() {
    use waterui::navigation::{Tab, Tabs};

    let insets = waterui::binding(SAFE_INSETS);
    let selection = Binding::container(0i32);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            Tabs::new(
                &selection,
                vec![
                    Tab::new(0i32, "Messages", move || {
                        NavigationView::new(
                            "Messages",
                            vstack((card("tab content"), spacer()))
                                .background(Color::new(Srgb::new(0.55, 0.3, 0.75)))
                                .a11y_label("tab-page"),
                        )
                    }),
                    Tab::new(1i32, "Settings", move || {
                        NavigationView::new("Settings", text("settings"))
                    }),
                ],
            )
        },
        390,
        844,
    );
    app.settle();

    let page = node_bounds(app.tree(), "tab-page");
    let bar = node_bounds_only_role(app.tree(), Role::TabList);
    assert!(
        (bar.y0 + bar.height() - CONTAINER_TOP).abs() <= 1.0
            && (page.y0 + page.height() - bar.y0).abs() <= 1.0,
        "with the keyboard down the bar ends on the container boundary and \
         the page's bottom edge sits at the bar's top edge, page {page:?} \
         bar {bar:?}"
    );
    app.capture_snapshot("safe-area", "tab-page-background", "insets");
}

/// §7.1's chrome rule: a bottom tab bar stays docked to its edge — laid out
/// clear of the container region only — so a keyboard deeper than the
/// bottom container inset covers the bar instead of lifting it, whether the
/// keyboard stops partway up the band or covers it outright. The tab's
/// hosted content keeps the deeper boundary: a focused field inside it
/// still clears the keyboard.
#[test]
fn a_tab_bar_stays_docked_under_the_keyboard() {
    use waterui::navigation::{Tab, Tabs};

    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));
    let selection = Binding::container(0i32);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            Tabs::new(
                &selection,
                vec![
                    Tab::new(0i32, "Messages", {
                        let value = value.clone();
                        move || {
                            NavigationView::new(
                                "Messages",
                                zstack((
                                    ScrollView::vertical(vstack((
                                        spacer().size(390.0, 560.0),
                                        field("Message", &value).size(350.0, 44.0),
                                        spacer().size(390.0, 380.0),
                                    )))
                                    .a11y_label("tab-page"),
                                    vstack((
                                        spacer(),
                                        card("edge")
                                            .ignore_safe_area(
                                                SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                                            )
                                            .a11y_label("tab-content-edge"),
                                    )),
                                )),
                            )
                        }
                    }),
                    Tab::new(1i32, "Settings", move || {
                        NavigationView::new("Settings", text("settings"))
                    }),
                ],
            )
        },
        390,
        844,
    );
    app.settle();

    // Two keyboard depths on the bottom edge: one that only partly covers
    // the band — deeper than the container inset, shallower than inset
    // plus bar height — and one that covers it outright. The band ends on
    // the container boundary either way.
    for (bottom_inset, keyboard_top, covers_band) in
        [(60.0_f32, 784.0, false), (336.0, KEYBOARD_TOP, true)]
    {
        keyboard.set(EdgeInsets::new(0.0, bottom_inset, 0.0, 0.0));
        app.settle();

        let bar = node_bounds_only_role(app.tree(), Role::TabList);
        assert!(
            (bar.y0 + bar.height() - CONTAINER_TOP).abs() <= 1.0,
            "bottom_inset={bottom_inset}: the tab bar docks at the \
             container boundary under the keyboard, got {bar:?}"
        );
        assert!(
            if covers_band {
                bar.y0 > keyboard_top + 1.0
            } else {
                bar.y0 < keyboard_top
            },
            "bottom_inset={bottom_inset}: the band {} the keyboard's top, \
             got {bar:?}",
            if covers_band {
                "sits wholly under"
            } else {
                "pokes above"
            },
        );

        // The hosted content keeps the deeper boundary — it stops at the
        // higher of the band's top edge and the keyboard's top: the bar's
        // top under the partway keyboard, the keyboard boundary under the
        // deep one, where the covered band stays out of reach even for
        // the card's `.ignore_safe_area(.keyboard)` release.
        let content_bottom = bar.y0.min(keyboard_top);
        let content = node_bounds(app.tree(), "tab-content-edge");
        assert!(
            (content.y0 + content.height() - content_bottom).abs() <= 1.0,
            "bottom_inset={bottom_inset}: the hosted content ends at the \
             deeper boundary, got {content:?}"
        );
        let page = node_bounds(app.tree(), "tab-page");
        assert!(
            (page.y0 + page.height() - content_bottom).abs() <= 1.0,
            "bottom_inset={bottom_inset}: the page surface stops at the \
             deeper boundary under the docked bar, got {page:?}"
        );
    }

    // The loop's last leg left the deep keyboard up: a focused field in
    // the hosted content still clears to the keyboard boundary.
    let covered = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        covered.y0 + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition: the field starts under the keyboard band, got {covered:?}"
    );
    let field = node_id_with_role(app.tree(), Role::TextInput, "Message");
    app.focus_quiet(field);
    app.pump_for(Duration::from_millis(500));
    app.pump_for(Duration::from_secs(1));

    let cleared = node_bounds_with_role(app.tree(), Role::TextInput, "Message");
    assert!(
        (cleared.y0 + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the hosted surface clears the field to the keyboard boundary, \
         got {cleared:?}"
    );
}

/// A `NavigationView` bottom toolbar is the same docked chrome: under a
/// keyboard deeper than the bottom inset, its items sit in the band under
/// the keyboard region instead of riding on top of it.
#[test]
fn a_navigation_bottom_toolbar_stays_docked_under_the_keyboard() {
    use waterui::navigation::{
        NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
    };

    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            NavigationView::new("Home", vstack((card("content"), spacer()))).navigation_toolbar(
                NavigationToolbar::new(vec![NavigationToolbarItem::action(
                    NavigationToolbarPlacement::BottomBar,
                    "New",
                    || {},
                )]),
            )
        },
        390,
        844,
    );
    app.settle();

    let item = node_bounds_with_role(app.tree(), Role::Button, "New");
    assert!(
        item.y0 > KEYBOARD_TOP + 1.0 && item.y0 + item.height() <= CONTAINER_TOP + 1.0,
        "the bottom toolbar item sits in the container band under the \
         keyboard, got {item:?}"
    );
}

/// The canonical `Tab::container` shape — hidden-bar `NavigationView` →
/// `NavigationStack` → page `NavigationView` carrying a bottom toolbar:
/// the dock the tab bar establishes has to reach the page's chrome
/// through every hosted/`content_area_for` hop, so the page toolbar
/// stacks on the docked tab bar with the keyboard down and up, never
/// riding onto the keyboard while the tab bar sits under it.
#[test]
fn a_stack_page_toolbar_inside_a_tab_stacks_on_the_docked_tab_bar() {
    use waterui::navigation::{
        NavigationStack, NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement, Tab,
        Tabs,
    };

    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));
    let selection = Binding::container(0i32);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            Tabs::new(
                &selection,
                vec![
                    Tab::container(0i32, "Messages", move || {
                        NavigationStack::new(
                            NavigationView::new("Root", vstack((card("content"), spacer())))
                                .navigation_toolbar(NavigationToolbar::new(vec![
                                    NavigationToolbarItem::action(
                                        NavigationToolbarPlacement::BottomBar,
                                        "New",
                                        || {},
                                    ),
                                ])),
                        )
                    }),
                    Tab::container(1i32, "Settings", move || {
                        NavigationStack::new(NavigationView::new("Settings", text("settings")))
                    }),
                ],
            )
        },
        390,
        844,
    );
    app.settle();

    // The stacking gap and the item's `y` are measured with the keyboard
    // down; the keyboard covering the tab bar's band must leave both
    // identical — the dock lands on the same edge either way.
    let item_down = node_bounds_with_role(app.tree(), Role::Button, "New");
    let bar_down = node_bounds_only_role(app.tree(), Role::TabList);
    let gap_down = bar_down.y0 - item_down.y0 - item_down.height();

    keyboard.set(KEYBOARD_INSETS);
    app.settle();

    let item = node_bounds_with_role(app.tree(), Role::Button, "New");
    let bar = node_bounds_only_role(app.tree(), Role::TabList);
    assert!(
        (item.y0 - item_down.y0).abs() <= 0.5
            && (bar.y0 - item.y0 - item.height() - gap_down).abs() <= 0.5,
        "with the keyboard covering the tab bar the toolbar item keeps \
         its y ({:.1} -> {:.1}) and its stacking gap ({:.1} -> {:.1}), \
         item {item:?} tab bar {bar:?}",
        item_down.y0,
        item.y0,
        gap_down,
        bar.y0 - item.y0 - item.height(),
    );
}

/// A `Tabs` inside navigation content that does not reach the bottom edge
/// docks its own bar at its own frame's bottom — the dock the outer bar
/// establishes applies only to content whose edge lands on it, not to
/// every descendant carrying the boundary down a `child()` chain.
#[test]
fn a_fixed_height_tabs_inside_tab_content_docks_its_own_bar() {
    use waterui::navigation::{
        NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement, Tab, Tabs,
    };

    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));
    let selection = Binding::container(0i32);
    let inner = Binding::container(0i32);
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            Tabs::new(
                &selection,
                vec![
                    Tab::new(0i32, "Outer", {
                        let inner = inner.clone();
                        move || {
                            NavigationView::new(
                                "Outer",
                                vstack((
                                    Tabs::new(
                                        &inner,
                                        vec![
                                            Tab::new(0i32, "Inner A", move || {
                                                NavigationView::new("A", text("a"))
                                            }),
                                            Tab::new(1i32, "Inner B", move || {
                                                NavigationView::new("B", text("b"))
                                            }),
                                        ],
                                    )
                                    .size(370.0, 200.0),
                                    card("below").a11y_label("below-inner"),
                                    spacer(),
                                )),
                            )
                            // The outer bar's bottom dock is what the inner
                            // tabs must not inherit: it lands on the
                            // container boundary, far past the nested
                            // frame's own edge.
                            .navigation_toolbar(
                                NavigationToolbar::new(vec![NavigationToolbarItem::action(
                                    NavigationToolbarPlacement::BottomBar,
                                    "New",
                                    || {},
                                )]),
                            )
                        }
                    }),
                    Tab::new(1i32, "Other", move || {
                        NavigationView::new("Other", text("other"))
                    }),
                ],
            )
        },
        390,
        844,
    );
    app.settle();

    for keyboard_up in [false, true] {
        if keyboard_up {
            keyboard.set(KEYBOARD_INSETS);
            app.settle();
        }
        let mut bars: Vec<_> = support::nodes(app.tree())
            .filter(|(_, node)| node.role() == Role::TabList)
            .filter_map(|(_, node)| node.bounds())
            .collect();
        bars.sort_by(|a, b| a.y0.partial_cmp(&b.y0).unwrap());
        assert_eq!(
            bars.len(),
            2,
            "expected exactly two tab lists, got {bars:?}"
        );
        let [inner_bar, outer_bar] = [bars[0], bars[1]];
        let below = node_bounds(app.tree(), "below-inner");
        assert!(
            inner_bar.y0 + inner_bar.height() <= below.y0 + 1.0,
            "keyboard_up={keyboard_up}: a nested tab bar that does not reach \
             the dock stays inside its own frame, above the next sibling, \
             inner bar {inner_bar:?} sibling {below:?}"
        );
        assert!(
            (outer_bar.y0 + outer_bar.height() - CONTAINER_TOP).abs() <= 1.0,
            "keyboard_up={keyboard_up}: the outer bar stays docked on the \
             container boundary, got {outer_bar:?}"
        );
    }
}

/// A drawn context-menu panel hosts its accessory under the panel's own
/// frame (`with_frame` on the window root context): the accessory's
/// background stays inside the panel — the snapshot shows a small tinted
/// accessory row, not a window-wide fill.
#[test]
fn a_context_menu_panel_background_stays_at_the_panel() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            Frame::new(
                button("host").action(|| {}).context_menu(
                    ContextMenu::new(vec!["Copy".action(|| {})]).accessory(
                        text("preview")
                            .padding_with(6.0)
                            .background(Color::new(Srgb::new(0.75, 0.25, 0.55)))
                            .a11y_label("preview-panel"),
                    ),
                ),
            )
            .width(160.0)
            .height(80.0)
        },
        390,
        844,
    );
    app.settle();

    let host = node_bounds_with_role(app.tree(), Role::Button, "host");
    // A secondary click opens the menu; the accessory forces the drawn
    // presentation so the panel renders inside this window's scene.
    app.secondary_click_at(
        f64_as_f32(host.x0 + host.width() / 2.0),
        f64_as_f32(host.y0 + host.height() / 2.0),
    );
    app.settle();

    let preview = node_bounds(app.tree(), "preview-panel");
    assert!(
        preview.width() <= 200.0 && preview.height() <= 120.0,
        "the panel keeps its own small frame rather than flooding the \
         window, got {preview:?}"
    );
    app.capture_snapshot("safe-area", "context-menu-panel", "insets");
}

/// A lazy stack outside a scroll surface records its context for lazily
/// materialized items — the `LazyStack` layout arm used to leave it unset,
/// so items always saw `None`. Items inherit the window's boundaries like
/// static siblings: the last row's bottom edge is reachable, so the release
/// probe inside it lands on the container boundary.
///
/// The fixture cannot assume where item edges land — placement depends on
/// style metrics — so it publishes no keyboard at first, reads where the
/// bottom-anchored stack put the last row's bottom edge, and then
/// publishes a keyboard region whose boundary lies exactly there. The
/// probe is the row itself, so the released measure grows the row's top
/// edge upward while its bottom edge stays anchored on the boundary —
/// the release cannot shift the edge it released.
#[test]
fn a_lazy_stack_outside_a_scroll_gives_items_a_context() {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));
    let mut app = mount_rendered(
        env_with_keyboard(&container, &keyboard),
        move || {
            vstack((
                spacer(),
                VStack::for_each((0..4).map(SelfId::new).collect::<Vec<_>>(), |item| {
                    let color = if *item == 3 {
                        Color::new(Srgb::new(0.2, 0.5, 0.8))
                    } else {
                        Color::new(Srgb::new(0.92, 0.92, 0.92))
                    };
                    if *item == 3 {
                        AnyView::new(
                            // The probe is the whole row: its bottom edge
                            // coincides with the stack's anchored bottom
                            // edge, which a bottom-anchored stack cannot
                            // shift when the released measure grows the
                            // row — the growth slides the row's top up,
                            // not its bottom off the boundary.
                            edge_probe(
                                Frame::new(text("lazy 3"))
                                    .width(390.0)
                                    .height(40.0)
                                    .background(color),
                                "lazy-3",
                                SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                            ),
                        )
                    } else {
                        AnyView::new(
                            Frame::new(text(format!("lazy {}", *item)))
                                .width(390.0)
                                .height(40.0)
                                .background(color),
                        )
                    }
                }),
            ))
        },
        390,
        844,
    );
    // A lazy stack keeps a small perpetual measure queue under this runtime,
    // so the test pumps a bounded window instead of settling.
    app.pump_for(Duration::from_millis(500));

    // Wherever the row's probe landed, a keyboard band whose top edge sits
    // exactly on its bottom edge makes that edge reachable; the release
    // then extends the probe's bounds down to the container boundary.
    let laid = node_bounds(app.tree(), "lazy-3");
    keyboard.set(EdgeInsets::new(
        0.0,
        f64_as_f32(844.0 - (laid.y0 + laid.height())),
        0.0,
        0.0,
    ));
    app.pump_for(Duration::from_millis(500));

    let last = node_bounds(app.tree(), "lazy-3");
    assert!(
        (last.y0 + last.height() - CONTAINER_TOP).abs() <= 1.0,
        "the last lazy item's bottom edge is reachable: the release lands \
         it at {CONTAINER_TOP}, got {last:?} (laid {laid:?})"
    );
    app.capture_snapshot("safe-area", "lazy-stack-item-fill", "keyboard");
}

/// A navigation view without a visible bar on an edge passes that edge
/// through: with the bar hidden the page's top edge touches the status
/// boundary, so the page is laid out against it and its background fill
/// extends through the status band — no chrome covers the band. The hero
/// probe on the page's top edge discriminates reachability: released, it
/// lands on the window edge itself.
#[test]
fn a_navigation_page_without_a_bar_passes_the_edge_through() {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            NavigationView::new(
                "Home",
                vstack((
                    edge_probe(card("hero"), "hero", EdgeSet::TOP),
                    card("content"),
                    spacer(),
                ))
                .background(Color::new(Srgb::new(0.7, 0.55, 0.1)))
                .a11y_label("page"),
            )
            .navigation_bar_visibility(false)
        },
        390,
        844,
    );
    app.settle();

    let page = node_bounds(app.tree(), "page");
    assert!(
        (page.y0 - f64::from(SAFE_INSETS.top())).abs() <= 1.0,
        "with no bar the page starts at the status boundary, got {page:?}"
    );
    let hero = node_bounds(app.tree(), "hero");
    assert!(
        hero.y0.abs() <= 1.0,
        "the top edge is reachable: the release lands the hero at the \
         window edge, got {hero:?}"
    );
    app.capture_snapshot("safe-area", "nav-barless-pass-through", "insets");
}

/// §7.1 "Chrome" capture vehicle — this test exists to export the offscreen
/// PNG captures a human reviews; its assertions only gate the captures on the
/// layout being what it should be. The bars' paint extension has no a11y or
/// layout signal by design, so the snapshots are the evidence:
/// `navigation-chrome-extension/{insets,transition}` show a navigation
/// stack's bar surfaces covering the status band and the bottom inset —
/// settled and mid-push — and `tab-bar-extension/insets` the bottom-docked
/// tab bar's surface at the window edge.
#[test]
fn chrome_surface_extension_captures() {
    use waterui::navigation::{
        NavigationLink, NavigationStack, NavigationToolbar, NavigationToolbarItem,
        NavigationToolbarPlacement, Tab, Tabs,
    };
    use waterui::{Str, ViewExt};

    let insets = waterui::binding(SAFE_INSETS);
    let query = waterui::binding(Str::from(""));
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            NavigationStack::new(
                NavigationView::new(
                    "Inbox",
                    vstack((
                        card("mail"),
                        NavigationLink::new("Open Detail", || {
                            NavigationView::new("Detail", text("detail body"))
                                .navigation_bar_color(Color::new(Srgb::new(0.55, 0.2, 0.7)))
                        }),
                        spacer(),
                    ))
                    .background(Color::new(Srgb::new(0.08, 0.1, 0.2))),
                )
                .navigation_bar_color(Color::new(Srgb::new(0.55, 0.2, 0.7)))
                .navigation_toolbar(
                    NavigationToolbar::default()
                        .item(NavigationToolbarItem::new(
                            NavigationToolbarPlacement::TopBarTrailing,
                            button("Add").action(|| {}),
                        ))
                        .item(NavigationToolbarItem::new(
                            NavigationToolbarPlacement::BottomBar,
                            button("Mark All").action(|| {}),
                        )),
                )
                .searchable(&query, "Search mail"),
            )
        },
        390,
        844,
    );
    app.settle();
    app.capture_snapshot("safe-area", "navigation-chrome-extension", "insets");

    // Mid-push: the queued pointer events arm the transition and one pumped
    // clock window lands the capture inside the fade-through.
    let link = node_bounds_with_role(app.tree(), Role::Button, "Open Detail");
    let (x, y) = (link.x0 + link.width() / 2.0, link.y0 + link.height() / 2.0);
    app.queue_pointer_down(f64_as_f32(x), f64_as_f32(y));
    app.queue_pointer_up(f64_as_f32(x), f64_as_f32(y));
    app.pump_for(Duration::from_millis(180));
    app.capture_snapshot("safe-area", "navigation-chrome-extension", "transition");

    let selection = Binding::container(0i32);
    let mut app = mount_rendered(
        env_with_insets(&insets),
        move || {
            Tabs::new(
                &selection,
                vec![
                    Tab::new(0i32, "Messages", move || {
                        NavigationView::new(
                            "Messages",
                            vstack((card("chat list"), spacer()))
                                .background(Color::new(Srgb::new(0.08, 0.1, 0.2)))
                                .a11y_label("tab-page"),
                        )
                    }),
                    Tab::new(1i32, "Settings", move || {
                        NavigationView::new("Settings", text("settings"))
                    }),
                ],
            )
        },
        390,
        844,
    );
    app.settle();

    let bar = node_bounds_only_role(app.tree(), Role::TabList);
    assert!(
        (bar.y0 + bar.height() - CONTAINER_TOP).abs() <= 1.0,
        "the tab bar's frame keeps the navigation-band boundary, got {bar:?}"
    );
    let title = node_bounds_with_role(app.tree(), Role::Header, "Messages");
    assert!(
        title.y0 >= f64::from(SAFE_INSETS.top()) - 1.0,
        "the nested page's title stays inside the safe area, got {title:?}"
    );
    app.capture_snapshot("safe-area", "tab-bar-extension", "insets");
}
