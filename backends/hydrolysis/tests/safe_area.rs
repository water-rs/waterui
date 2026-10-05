//! Host↔renderer boundary tests for the window safe area: an environment
//! carrying [`hydrolysis::WindowSafeArea`] places the root content inside the
//! host-published insets, and `.ignore_safe_area` releases its flagged edges —
//! verified through the rendered runtime's accessibility-tree bounds, the same
//! way an Android host's insets reach the pipeline.

use waterui::Binding;
use waterui::View;
use waterui::graphics::color::Srgb;
use waterui::layout::padding::EdgeInsets;
use waterui::layout::safe_area::{EdgeSet, SafeAreaRegions};
use waterui::layout::scroll::ScrollView;
use waterui::prelude::*;
use waterui_testing::{Role, Styled, UiBuilder};

/// The window insets these tests publish — a status bar at the top, a
/// navigation bar at the bottom and a small cutout on the leading edge.
const SAFE_INSETS: EdgeInsets = EdgeInsets::new(48.0, 34.0, 8.0, 8.0);

/// A keyboard region on the bottom edge, published separately from the
/// container insets the way an Android host reports `ime` next to `system_bars`.
const KEYBOARD_INSETS: EdgeInsets = EdgeInsets::new(0.0, 336.0, 0.0, 0.0);

/// The window-space y where the keyboard band starts for the test viewport.
const KEYBOARD_TOP: f32 = 508.0;

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

/// A fill background under a translucent keyboard region is painted by the
/// fill: the content's frame stops on the avoided edge, but the fill's paint
/// runs on through the keyboard band to the window edge (§7.1 "Paint extends
/// for fills"). The extension is verified by reading the snapshot PNG — below
/// the avoided edge the fill covers the band where no paint would otherwise
/// reach.
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
    assert!(
        bounds.y() + bounds.height() <= KEYBOARD_TOP + 1.0,
        "the content itself stays clear of the keyboard region, got {bounds:?}"
    );
    app.capture_snapshot("safe-area", "fill-under-keyboard", "keyboard");
}

/// A non-fill background is not extended: the backdrop keeps its own frame and
/// does not paint into the keyboard band.
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

    let bounds = app
        .query()
        .role(Role::LABEL)
        .label("backdrop")
        .single()
        .bounds();
    assert!(
        bounds.y() + bounds.height() <= KEYBOARD_TOP + 1.0,
        "a non-fill background must not extend into the keyboard band, got {bounds:?}"
    );
    app.capture_snapshot("safe-area", "non-fill-under-keyboard", "keyboard");
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
    let expected_bottom = 844.0 - SAFE_INSETS.bottom();
    assert!(
        (bounds.y() + bounds.height() - expected_bottom).abs() <= 1.0,
        "the keyboard region is released but the container inset still binds: \
         expected bottom {expected_bottom}, got {bounds:?}"
    );
}

/// A scroll surface covering the keyboard band scrolls a focused text field
/// the minimum distance that brings it clear — on focus, and again while the
/// keyboard inset grows (§7.1 "Scroll surfaces").
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
        cleared.y() + cleared.height() <= KEYBOARD_TOP + 1.0,
        "focusing the field scrolls it clear of the keyboard, got {cleared:?}"
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
        tracked.y() + tracked.height() <= 844.0 - 400.0 + 1.0,
        "the surface tracks the growing keyboard inset, got {tracked:?}"
    );
}
