//! Host↔renderer boundary tests for the window safe area: an environment
//! carrying [`hydrolysis::WindowSafeArea`] places the root content inside the
//! host-published insets, and `.ignore_safe_area` releases its flagged edges —
//! verified through the rendered runtime's accessibility-tree bounds, the same
//! way an Android host's insets reach the pipeline.

use waterui::Binding;
use waterui::View;
use waterui::graphics::color::Srgb;
use waterui::layout::padding::EdgeInsets;
use waterui::layout::safe_area::EdgeSet;
use waterui::prelude::*;
use waterui_testing::{Role, Styled, UiBuilder};

/// The window insets these tests publish — a status bar at the top, a
/// navigation bar at the bottom and a small cutout on the leading edge.
const SAFE_INSETS: EdgeInsets = EdgeInsets::new(48.0, 34.0, 8.0, 8.0);

fn card(label: &'static str) -> impl View {
    text(label).body().foreground(Srgb::WHITE)
}

fn env_with_insets(insets: &Binding<EdgeInsets>) -> Environment {
    let mut env = Environment::new();
    env.insert(hydrolysis::WindowSafeArea(insets.clone()));
    env
}

/// The root content lays out inside the published insets.
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
