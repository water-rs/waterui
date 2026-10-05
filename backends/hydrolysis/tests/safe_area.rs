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
use waterui::layout::safe_area::{EdgeSet, SafeAreaRegions};
use waterui::layout::scroll::ScrollView;
use waterui::navigation::NavigationView;
use waterui::prelude::*;
use waterui::{AnyView, Binding, Color, View};
use waterui_testing::{DragOptions, Role, Styled, UiBuilder};

/// The window insets these tests publish — a status bar at the top, a
/// navigation bar at the bottom and a small cutout on the leading edge.
const SAFE_INSETS: EdgeInsets = EdgeInsets::new(48.0, 34.0, 8.0, 8.0);

/// A keyboard region on the bottom edge, published separately from the
/// container insets the way an Android host reports `ime` next to `system_bars`.
const KEYBOARD_INSETS: EdgeInsets = EdgeInsets::new(0.0, 336.0, 0.0, 0.0);

/// The window-space y where the keyboard band starts for the test viewport.
const KEYBOARD_TOP: f32 = 508.0;

/// The window-space y where the container (navigation bar) band starts.
const CONTAINER_TOP: f32 = 844.0 - 34.0;

fn card(label: &'static str) -> impl View {
    text(label).body().foreground(Srgb::WHITE)
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
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn root_content_stays_inside_the_safe_area(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || vstack((card("inbox"),)).alignment(HorizontalAlignment::Leading));

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("inbox")
        .single()
        .bounds();
    assert!(
        (bounds.x() - SAFE_INSETS.leading()).abs() <= 1.0
            && (bounds.y() - SAFE_INSETS.top()).abs() <= 1.0,
        "root content should start at the safe-area origin \
         ({}, {}), got {bounds:?}",
        SAFE_INSETS.leading(),
        SAFE_INSETS.top()
    );
}

/// `.ignore_safe_area(ALL)` lays the root content out edge-to-edge.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn ignore_safe_area_all_reaches_the_window_origin(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || {
            vstack((card("chrome"),))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(EdgeSet::ALL)
        });

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("chrome")
        .single()
        .bounds();
    assert!(
        bounds.x().abs() <= 1.0 && bounds.y().abs() <= 1.0,
        "ignore_safe_area(ALL) content should start at the window origin, got {bounds:?}"
    );
}

/// `.ignore_safe_area(TOP)` releases only the top inset: the content slides
/// under the status bar but keeps the leading safe edge.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn ignore_safe_area_releases_only_the_flagged_edges(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || {
            vstack((card("header"),))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(EdgeSet::TOP)
        });

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("header")
        .single()
        .bounds();
    assert!(
        bounds.y().abs() <= 1.0,
        "a TOP-released view should reach the window top, got {bounds:?}"
    );
    assert!(
        (bounds.x() - SAFE_INSETS.leading()).abs() <= 1.0,
        "the leading edge stays inside the safe area, got {bounds:?}"
    );
}

/// §7.1 "touches": a release applies only on an edge the wrapper's laid-out
/// frame ends on. The first child of a vstack ends on the top boundary and
/// its TOP declaration releases it to the window edge; the second child ends
/// mid-surface — the same declaration releases nothing for it.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn an_ignore_releases_only_on_an_edge_the_frame_touches(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || {
            vstack((
                card("top-touches").ignore_safe_area(EdgeSet::TOP),
                card("below").ignore_safe_area(EdgeSet::TOP),
            ))
            .alignment(HorizontalAlignment::Leading)
        });

    let top = app
        .query()
        .role(Role::LABEL)
        .label("top-touches")
        .single()
        .bounds();
    assert!(
        top.y().abs() <= 1.0,
        "the touching edge releases to the window origin, got {top:?}"
    );

    let below = app
        .query()
        .role(Role::LABEL)
        .label("below")
        .single()
        .bounds();
    // "below" sits one spacing under the first child's released height. A
    // release would have shifted its origin up by the container depth; its
    // frame does not touch the top boundary, so nothing releases.
    assert!(
        (below.y() - (top.height() + 10.0)).abs() <= 1.0,
        "a non-touching edge releases nothing, got {below:?}"
    );
}

/// A later insets write — rotation, split-screen or the IME arriving —
/// re-lays the window out against the new value.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_safe_area_change_relays_out_the_root(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || vstack((card("inbox"),)).alignment(HorizontalAlignment::Leading));
    let first = app.query().label("inbox").single().bounds();

    insets.set(EdgeInsets::new(80.0, 34.0, 24.0, 8.0));
    app.settle();

    let moved = app.query().label("inbox").single().bounds();
    assert!(
        (moved.y() - 80.0).abs() <= 1.0 && (moved.x() - 24.0).abs() <= 1.0,
        "the root should track the updated insets: was {first:?}, now {moved:?}"
    );
}

/// A navigation stack records each page in the page's own space and presents
/// it at the stack's place in the window. The page's hit targets and
/// accessibility bounds have to follow it there: under a top inset the link a
/// finger lands on is the one drawn below the status bar, not one inset above
/// it. Tapping the reported bounds must also activate the link.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn navigation_page_targets_follow_the_safe_area(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
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
    let mut bare = ui
        .clone()
        .environment(env_with_insets(&no_insets))
        .mount_offscreen(stack);
    let unshifted = bare
        .query()
        .role(Role::BUTTON)
        .label("Open Detail")
        .single()
        .bounds();

    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(stack);
    let link = app.query().role(Role::BUTTON).label("Open Detail").single();
    let shifted = link.bounds();
    // The link is centred in the page, so the horizontal insets move it by
    // half their difference; the top inset moves it down whole.
    let expected_x = unshifted.x() + (SAFE_INSETS.leading() - SAFE_INSETS.trailing()) / 2.0;
    let expected_y = unshifted.y() + SAFE_INSETS.top();
    assert!(
        (shifted.x() - expected_x).abs() <= 1.0 && (shifted.y() - expected_y).abs() <= 1.0,
        "the page's link should sit inside the safe area at ({expected_x}, {expected_y}): \
         {unshifted:?} without insets, {shifted:?} with {SAFE_INSETS:?}"
    );

    link.tap(&mut app);
    app.settle();
    assert!(
        app.query().role(Role::BUTTON).label("Back").exists(),
        "a tap on the link's reported bounds must push the detail page"
    );
}

/// A fill's extension reaches through wrappers that never read bounds: an
/// `Opacity` over the color is still a fill, so the translucent paint runs
/// on through the keyboard band to the window edge exactly as the opaque
/// case does (snapshot).
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_fill_inside_opacity_still_extends(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((card("content"), spacer()))
                .alignment(HorizontalAlignment::Leading)
                .background(Color::new(Srgb::new(0.2, 0.5, 0.9)).opacity(0.6))
        });

    let content = app
        .query()
        .role(Role::LABEL)
        .label("content")
        .single()
        .bounds();
    assert!(
        (content.y() - SAFE_INSETS.top()).abs() <= 1.0,
        "the content frame keeps the boundary, got {content:?}"
    );
    app.capture_snapshot("safe-area", "fill-inside-opacity", "keyboard");
}

/// `SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM)` lays the view out under the
/// keyboard but still clear of the container (navigation-bar) inset: the
/// bottom edge slides under the keyboard band and stops at
/// `window height - container bottom`.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn ignoring_only_the_keyboard_region_keeps_the_container_inset(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((spacer(), card("composer")))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM))
        });

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    assert!(
        (bounds.y() + bounds.height() - CONTAINER_TOP).abs() <= 1.0,
        "the keyboard region is released but the container inset still binds: \
         expected bottom {CONTAINER_TOP}, got {bounds:?}"
    );
}

/// `SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM)` with the keyboard up
/// releases nothing: the keyboard region is the deeper one on that edge and
/// still binds, so the frame ends on the keyboard boundary exactly as if no
/// declaration were present.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn ignoring_only_the_container_region_releases_nothing_under_the_keyboard(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((spacer(), card("composer")))
                .alignment(HorizontalAlignment::Leading)
                .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM))
        });

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    assert!(
        (bounds.y() + bounds.height() - KEYBOARD_TOP).abs() <= 1.0,
        "the keyboard region still binds: expected bottom {KEYBOARD_TOP}, got {bounds:?}"
    );

    // With the keyboard down the container region is the deepest one on the
    // bottom edge, and the declaration releases it: the frame slides under
    // the navigation bar by the container inset and ends on the window edge.
    keyboard.set(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));
    app.settle();

    let released = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    assert!(
        (released.y() + released.height() - 844.0).abs() <= 1.0,
        "the CONTAINER release ends on the window edge — the bottom grew by \
         the container inset (34) once the keyboard left, got {released:?}"
    );
}

/// §7.1 "Paint extends for fills": a background fill whose frame touches the
/// bottom boundary paints through the keyboard and container bands to the
/// window edge. The paint extension is a snapshot-only fact — the fill's own
/// frame (and every a11y bound) stays on the boundary.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_fill_background_paints_under_the_keyboard_region(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((
                spacer(),
                text("composer")
                    .body()
                    .foreground(Srgb::WHITE)
                    .padding_with(8.0)
                    .background(Srgb::new(0.0, 0.35, 0.85)),
            ))
            .alignment(HorizontalAlignment::Leading)
        });

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    // The text sits 8 pt inside its `padding_with(8.0)` chrome; the padded
    // frame, not the label, is what ends on the keyboard boundary.
    assert!(
        (bounds.y() + bounds.height() - (KEYBOARD_TOP - 8.0)).abs() <= 1.0,
        "the content's frame ends on the keyboard boundary, got {bounds:?}"
    );
    app.capture_snapshot("safe-area", "fill-under-keyboard", "keyboard");
}

/// An `.ignore_safe_area` on an ancestor does not replace the fill's own
/// default: a TOP declaration on the tree leaves a bottom background fill
/// extending under the keyboard region.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_fill_under_an_ancestor_top_ignore_still_extends_at_the_bottom(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
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
        });

    let header = app
        .query()
        .role(Role::LABEL)
        .label("header")
        .single()
        .bounds();
    assert!(
        header.y().abs() <= 1.0,
        "the ancestor's TOP ignore released the header under the status bar, got {header:?}"
    );
    let composer = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    assert!(
        (composer.y() + composer.height() - (KEYBOARD_TOP - 8.0)).abs() <= 1.0,
        "the composer frame still ends on the keyboard boundary, got {composer:?}"
    );
    app.capture_snapshot("safe-area", "fill-under-ancestor-top-ignore", "keyboard");
}

/// A background fill inside a `KEYBOARD.on(BOTTOM)` subtree ignores the
/// keyboard region (its frame ends on the container boundary) but still
/// extends through the container band it does not ignore.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_fill_under_a_keyboard_ignore_extends_through_the_container_band(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
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
        });

    let composer = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    assert!(
        (composer.y() + composer.height() - (CONTAINER_TOP - 8.0)).abs() <= 1.0,
        "the composer frame ends on the container boundary, got {composer:?}"
    );
    app.capture_snapshot("safe-area", "fill-under-keyboard-ignore", "keyboard");
}

/// §7.1's fill rule names background fills only: a `Divider` at the bottom
/// edge and a bare `Color` strip at the top edge keep their own frames — no
/// paint reaches into a region for them (snapshot read).
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn non_fill_leaves_at_an_edge_do_not_extend(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((
                Color::new(Srgb::new(0.8, 0.1, 0.1)).size(390.0, 4.0),
                card("content"),
                spacer(),
                Divider,
            ))
            .alignment(HorizontalAlignment::Leading)
        });

    let content = app
        .query()
        .role(Role::LABEL)
        .label("content")
        .single()
        .bounds();
    assert!(
        (content.y() - (SAFE_INSETS.top() + 4.0 + 10.0)).abs() <= 1.0,
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
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_non_fill_background_stays_inside_the_keyboard_region(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((
                spacer(),
                text("composer")
                    .body()
                    .foreground(Srgb::WHITE)
                    .padding_with(8.0)
                    .background(text("backdrop").foreground(Srgb::new(0.0, 0.5, 0.0))),
            ))
            .alignment(HorizontalAlignment::Leading)
        });

    app.capture_snapshot("safe-area", "non-fill-under-keyboard", "keyboard");
}

/// §7.1 "Scroll surfaces": a scroll surface covering the keyboard band
/// scrolls a focused text field the minimum distance that brings it clear —
/// its bottom lands exactly on the keyboard boundary — on focus, and again
/// while the keyboard inset grows.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_scroll_surface_scrolls_the_focused_field_clear_of_the_keyboard(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            ScrollView::vertical(vstack((
                spacer().size(390.0, 560.0),
                field("Message", &value).size(350.0, 44.0),
                spacer().size(390.0, 380.0),
            )))
        });

    // The surface extends under the keyboard, so the field at the bottom of the
    // content is painted (and reported) inside the band it must escape.
    let covered = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        covered.y() + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition: the field starts under the keyboard band, got {covered:?}"
    );

    app.query().role(Role::TEXT_INPUT).label("Message").focus();
    app.settle();

    let cleared = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        (cleared.y() + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the field's bottom lands exactly on the keyboard boundary, got {cleared:?}"
    );

    keyboard.set(EdgeInsets::new(0.0, 400.0, 0.0, 0.0));
    app.settle();

    let tracked = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        (tracked.y() + tracked.height() - (844.0 - 400.0)).abs() <= 0.5,
        "the surface tracks the growing keyboard inset, got {tracked:?}"
    );
}

/// The clearance bound is `min(keyboard top, the surface's own frame bottom)`:
/// a `vstack((scroll(form), toolbar))` whose scroll frame ends above the
/// keyboard scrolls the field to the frame's bottom, not into the toolbar.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_focused_field_clears_to_the_surface_frame_above_a_toolbar(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((
                ScrollView::vertical(vstack((
                    spacer().size(390.0, 560.0),
                    field("Message", &value).size(350.0, 44.0),
                    spacer().size(390.0, 380.0),
                ))),
                card("toolbar").size(390.0, 56.0),
            ))
        });

    // Scroll frame: root content (48..508) minus the toolbar's 56 and the
    // vstack's 10 spacing → 48..442.
    let surface_bottom = KEYBOARD_TOP - 56.0 - 10.0;

    app.query().role(Role::TEXT_INPUT).label("Message").focus();
    app.settle();
    // The clearance glides; pump it fully past the settle cap so the
    // assertion reads the converged position.
    app.pump_for(Duration::from_secs(1));

    let cleared = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        (cleared.y() + cleared.height() - surface_bottom).abs() <= 0.5,
        "the field clears to the scroll surface's own frame bottom \
         ({surface_bottom}), got {cleared:?}"
    );
}

/// A `List` is a scroll surface: a text field in a row under the keyboard
/// band is scrolled clear to the list's frame bottom on focus.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_list_row_field_scrolls_clear_of_the_keyboard(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let field_row = 10;
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
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
        });

    let covered = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        covered.y() + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition: the row field starts under the keyboard band, got {covered:?}"
    );

    app.query().role(Role::TEXT_INPUT).label("Message").focus();
    app.settle();
    app.pump_for(Duration::from_secs(1));

    let cleared = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        (cleared.y() + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the list scrolls the field's bottom onto the keyboard boundary, got {cleared:?}"
    );
}

/// The scroll surface's lazy viewport covers the extended frame: a lazy stack
/// builds the rows laid out under the keyboard band, not just the avoided
/// window.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_lazy_stack_in_a_scroll_builds_rows_under_the_keyboard_band(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            ScrollView::vertical(VStack::for_each(
                (0..40).map(SelfId::new).collect::<Vec<_>>(),
                |item| Frame::new(text(format!("row-{}", *item))).size(390.0, 30.0),
            ))
        });

    // Row 15 sits at window y ≈ 48 + 15 * (30 + 10) = 648 — inside the
    // keyboard band. Without the surface's extension the lazy viewport ends
    // at the avoided bottom (508) and the row is never built.
    let row = app
        .query()
        .role(Role::LABEL)
        .label("row-15")
        .single()
        .bounds();
    assert!(
        row.y() > KEYBOARD_TOP && row.y() < 844.0,
        "the row under the keyboard band is built and placed, got {row:?}"
    );
}

/// §7.1 "never fights a user scroll": a multi-frame touch drag on a surface
/// under non-zero regions keeps scrolling — every frame's layout rebind keeps
/// the live drag handle valid instead of dropping the gesture after the
/// first frame.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_touch_drag_over_several_frames_keeps_scrolling_under_nonzero_insets(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            ScrollView::vertical(vstack(
                (0..30)
                    .map(|i| Frame::new(text(format!("row-{i}"))).size(390.0, 30.0))
                    .collect::<Vec<_>>(),
            ))
        });

    // A touch drag is what claims a scroll view on a real device; the host
    // reports its gesture constants through `PlatformWindow::touch_scroll_config`.
    app.set_touch_scroll_config(hydrolysis::TouchScrollConfig::android_default());

    let before = app
        .query()
        .role(Role::LABEL)
        .label("row-8")
        .single()
        .bounds();

    // A 250 pt drag sampled over five frames: each move lands on its own
    // pump, so a stale scroll handle would drop everything after frame one.
    app.queue_drag_from_to_with(
        195.0,
        400.0,
        195.0,
        150.0,
        DragOptions {
            steps: 5,
            frame_per_step: true,
            pointer: hydrolysis::PointerKind::Touch,
        },
    );
    app.pump_for(Duration::from_millis(500));
    app.settle();

    let after = app
        .query()
        .role(Role::LABEL)
        .label("row-8")
        .single()
        .bounds();
    assert!(
        before.y() - after.y() >= 100.0,
        "the drag scrolled the content substantially across frames: \
         was {before:?}, now {after:?}"
    );
}

/// Fractional geometry (§7.1's "touches" resolved at display resolution):
/// Android-density insets and a text-height composer leave the f32-placed
/// frame a sub-pixel off the boundary. A touch within half a physical
/// pixel at the window's scale is still a touch — the nested declaration
/// on the grown frame edge releases to the window edge.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_fractional_geometry_still_touches_the_boundary(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    // 2.75x-density container insets: nothing in this geometry is an
    // integer — not the boundary (844 - 24.38), not the release amounts,
    // not the composer's measured text height.
    const FRACTIONAL_INSETS: EdgeInsets = EdgeInsets::new(47.24, 24.38, 8.2, 8.6);
    let container = waterui::binding(FRACTIONAL_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .scale_factor(2.75)
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((
                spacer(),
                vstack((spacer(), card("composer")))
                    .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM)),
            ))
            .alignment(HorizontalAlignment::Leading)
            .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM))
        });

    // The outer release moves the bottom boundary to the container edge
    // (819.62); the inner stack's f32-grown frame edge lands a sub-pixel
    // off it. Its CONTAINER declaration still touches and releases the
    // composer through the navigation-bar band to the window edge.
    let composer = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    assert!(
        (composer.y() + composer.height() - 844.0).abs() <= 1.0,
        "the nested declaration touches within half a physical pixel and \
         releases to the window edge, got {composer:?}"
    );
    app.capture_snapshot("safe-area", "fractional-geometry-touch", "keyboard");
}

/// A declaration ON the fill replaces the default extension — including an
/// empty one: a background `Color` carrying `.ignore_safe_area(EdgeSet::NONE)`
/// names no edge, so it releases nothing and no default extension applies.
/// The snapshot is the check: the bands stay the window background.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_fill_with_an_empty_ignore_extends_nowhere(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
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
        });

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("composer")
        .single()
        .bounds();
    // The declaration names no edges, so the padded frame still ends on the
    // keyboard boundary — the layout precondition a default extension
    // would share.
    assert!(
        (bounds.y() + bounds.height() - (KEYBOARD_TOP - 8.0)).abs() <= 1.0,
        "the frame still ends on the boundary, got {bounds:?}"
    );
    app.capture_snapshot("safe-area", "fill-declared-none", "keyboard");
}

/// `.ignore_safe_area(EdgeSet::BOTTOM)` on a fill replaces the default on
/// every edge: a full-height background touches both the top and bottom
/// boundaries, and only the named edge reaches the window edge — the
/// status-bar band keeps the window background (snapshot).
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_fill_ignore_names_the_only_edge_that_extends(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            vstack((card("content"), spacer()))
                .alignment(HorizontalAlignment::Leading)
                .background(Color::new(Srgb::new(0.75, 0.4, 0.0)).ignore_safe_area(EdgeSet::BOTTOM))
        });

    // The named bottom edge's release is the fill's whole extension. The
    // assertion is a precondition — the card keeps the container inset —
    // and the snapshot is the check: the background paints to the window
    // bottom edge and nowhere else.
    let content = app
        .query()
        .role(Role::LABEL)
        .label("content")
        .single()
        .bounds();
    assert!(
        (content.y() - SAFE_INSETS.top()).abs() <= 1.0,
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
fn navigation_hosted_form_clears_the_focused_field(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
    scale: f64,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = ui
        .scale_factor(scale)
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            NavigationView::new(
                "Compose",
                ScrollView::vertical(vstack((
                    spacer().size(390.0, 560.0),
                    field("Message", &value).size(350.0, 44.0),
                    spacer().size(390.0, 380.0),
                ))),
            )
        });

    let covered = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        covered.y() + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition at scale {scale}: the field starts under the keyboard \
         band, got {covered:?}"
    );

    app.query().role(Role::TEXT_INPUT).label("Message").focus();
    app.settle();
    app.pump_for(Duration::from_secs(1));

    let cleared = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        (cleared.y() + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "at scale {scale} the navigation-hosted surface clears the field to \
         the keyboard boundary, got {cleared:?}"
    );
}

#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_form_inside_navigation_content_clears_the_focused_field(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    navigation_hosted_form_clears_the_focused_field(ui, 1.0);
}

#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_form_inside_navigation_content_clears_the_focused_field_at_device_scale(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    navigation_hosted_form_clears_the_focused_field(ui, 2.625);
}

/// A navigation page's background fill stays inside the content frame: the
/// bar's band is not a boundary the content inherited, so the fill never
/// paints over the bar or the status-bar band, and rows scrolled to the
/// top stop at the bar's inner edge. Paint is the check — the snapshot
/// shows the fill beginning at the bar, never covering the title.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_navigation_page_background_stays_below_the_bar(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || {
            NavigationView::new(
                "Home",
                ScrollView::vertical(vstack((
                    card("row one"),
                    card("row two"),
                    card("row three"),
                    spacer().size(390.0, 480.0),
                )))
                .background(Color::new(Srgb::new(0.85, 0.2, 0.3)))
                .a11y_label("page-scroll"),
            )
        });
    app.settle();
    app.query().label("page-scroll").scroll_down();
    app.settle();
    app.capture_snapshot("safe-area", "nav-page-background", "insets");
}

/// `.ignore_safe_area(EdgeSet::TOP)` inside navigation content releases
/// nothing it does not touch: the page's top edge is not on the host's top
/// boundary — the bar's band covers it — so the hero stays at the content
/// top, below the bar (48 inset + the 64pt bar), never at the window edge.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn an_ignore_top_inside_navigation_content_stays_below_the_bar(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let insets = waterui::binding(SAFE_INSETS);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || {
            NavigationView::new(
                "Hero",
                vstack((card("hero").ignore_safe_area(EdgeSet::TOP), card("below"))),
            )
        });

    let hero = app
        .query()
        .role(Role::LABEL)
        .label("hero")
        .single()
        .bounds();
    assert!(
        (hero.y() - (SAFE_INSETS.top() + 64.0)).abs() <= 1.0,
        "the covered top edge releases nothing: the hero stays below the \
         bar, got {hero:?}"
    );
}

/// The edge a hosted subtree still touches stays reachable: a navigation
/// page ending on the bottom boundary extends its background fill through
/// the keyboard band to the window edge — the snapshot shows the fill
/// painting the band, the content frame ending on the boundary.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_navigation_page_touching_the_bottom_still_extends_its_fill(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            NavigationView::new(
                "Home",
                vstack((card("content"), spacer()))
                    .background(Color::new(Srgb::new(0.2, 0.6, 0.35))),
            )
        });

    let content = app
        .query()
        .role(Role::LABEL)
        .label("content")
        .single()
        .bounds();
    assert!(
        content.y() + content.height() <= KEYBOARD_TOP + 1.0,
        "the content frame still ends on the boundary, got {content:?}"
    );
    app.capture_snapshot("safe-area", "nav-bottom-fill", "keyboard");
}

/// An anchored overlay's background fill stays at the overlay's own frame:
/// the overlay inherits the window's boundaries, and a tooltip mid-window
/// touches none of them, so its fill extends nowhere — the snapshot shows
/// a small fill under the anchor, not a window-flooding one.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn an_anchored_overlay_fill_stays_at_its_own_frame(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay};

    let insets = waterui::binding(SAFE_INSETS);
    let open = waterui::binding(true);
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || {
            vstack((
                spacer().size(390.0, 200.0),
                text("anchor").body().anchored_overlay(
                    AnchoredOverlay::new(
                        &open,
                        text("tip")
                            .caption()
                            .padding_with(6.0)
                            .background(Color::new(Srgb::new(0.2, 0.7, 0.9))),
                    )
                    .edge(AnchorEdge::Bottom),
                ),
                spacer(),
            ))
            .alignment(HorizontalAlignment::Leading)
        });

    assert!(
        app.query().role(Role::LABEL).label("tip").exists(),
        "precondition: the presented overlay content is in the tree"
    );
    app.capture_snapshot("safe-area", "overlay-fill", "insets");
}

/// A split view's detail background does not cross the sidebar: the
/// detail column's leading edge is not on the host's leading boundary, so
/// it is covered — the snapshot shows the fill starting at the column's
/// leading edge with the sidebar unpainted-over beside it.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (1400, 900))]
fn a_split_detail_background_does_not_cross_the_sidebar(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    use waterui::navigation::NavigationSplitView;

    let insets = waterui::binding(SAFE_INSETS);
    let selection = Binding::container(Some(0i32));
    let mut app = ui
        .environment(env_with_insets(&insets))
        .mount_offscreen(move || {
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
        });
    app.settle();

    assert!(
        app.query().role(Role::LABEL).label("detail-fill").exists(),
        "precondition: the detail column rendered"
    );
    app.capture_snapshot("safe-area", "split-detail-background", "insets");
}

/// Nested declarations across chrome content never move a boundary inward:
/// hosted content inherits the host's boundary and released regions, so an
/// inner `.ignore_safe_area` can only release further out — never re-cover
/// a band the outer declaration already released. Both directions run:
/// KEYBOARD under the navigation view with CONTAINER inside, and CONTAINER
/// under the navigation view with KEYBOARD inside.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn nested_releases_never_move_the_navigation_boundary_inward(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);

    // Outer releases the keyboard region, inner names the container: both
    // regions end up released, so the inner declaration extends the
    // content outward to the window edge — never inward to a re-covered
    // boundary.
    let mut keyboard_first = ui
        .clone()
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            NavigationView::new(
                "Outer",
                vstack((spacer(), card("inner")))
                    .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM)),
            )
            .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM))
        });
    let inner = keyboard_first
        .query()
        .role(Role::LABEL)
        .label("inner")
        .single()
        .bounds();
    assert!(
        (inner.y() + inner.height() - 844.0).abs() <= 1.0,
        "KEYBOARD under nav + CONTAINER inside: the bottom edge lands on the \
         window edge, got {inner:?}"
    );

    // Outer releases the container region, inner names the keyboard: the
    // inner declaration releases outward past the container band to the
    // window edge.
    let mut container_first = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            NavigationView::new(
                "Outer",
                vstack((spacer(), card("inner")))
                    .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM)),
            )
            .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM))
        });
    let inner = container_first
        .query()
        .role(Role::LABEL)
        .label("inner")
        .single()
        .bounds();
    assert!(
        (inner.y() + inner.height() - 844.0).abs() <= 1.0,
        "CONTAINER under nav + KEYBOARD inside: the bottom edge lands on the \
         window edge, got {inner:?}"
    );
}

/// `ScrollNode` reads its metrics after `begin_flush`: a keyboard inset
/// change scrolls the field clear inside the early pass, so the very frame
/// the inset arrives already paints the field above the new keyboard top —
/// not one frame later.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_keyboard_inset_change_clears_the_field_on_that_frame(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
            ScrollView::vertical(vstack((
                spacer().size(390.0, 560.0),
                field("Message", &value).size(350.0, 44.0),
                spacer().size(390.0, 380.0),
            )))
        });

    app.query().role(Role::TEXT_INPUT).label("Message").focus();
    app.settle();
    app.pump_for(Duration::from_secs(1));

    keyboard.set(EdgeInsets::new(0.0, 400.0, 0.0, 0.0));
    // One frame — not a settle. The clearance must already be painted.
    app.pump_for(Duration::from_millis(16));

    let tracked = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        (tracked.y() + tracked.height() - (844.0 - 400.0)).abs() <= 0.5,
        "the field is already clear on the frame the inset arrived, got {tracked:?}"
    );
}

/// A field inside a scroll nested in a scroll is claimed once by the
/// surface that can clear it: the inner surface's content lays out without
/// a safe-area context, so the outer surface owns the clearance — the
/// field lands on the keyboard boundary, never over-scrolled by a second
/// surface reading the field's pre-clearance rect.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844))]
fn a_field_in_a_nested_scroll_is_cleared_once(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let container = waterui::binding(SAFE_INSETS);
    let keyboard = waterui::binding(KEYBOARD_INSETS);
    let value = Binding::container(waterui::Str::from(""));
    let mut app = ui
        .environment(env_with_keyboard(&container, &keyboard))
        .mount_offscreen(move || {
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
        });

    // The inner surface is context-free (§7.1: a scroll surface's content
    // stays context-free): its field at 510..554 sits under the keyboard
    // band inside the outer surface's extended window.
    let covered = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        covered.y() + covered.height() > KEYBOARD_TOP + 1.0,
        "precondition: the nested field starts under the keyboard band, got {covered:?}"
    );

    app.query().role(Role::TEXT_INPUT).label("Message").focus();
    app.settle();
    app.pump_for(Duration::from_secs(1));

    let cleared = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Message")
        .single()
        .bounds();
    assert!(
        (cleared.y() + cleared.height() - KEYBOARD_TOP).abs() <= 0.5,
        "the owning surface clears the nested field exactly once, got {cleared:?}"
    );
}
