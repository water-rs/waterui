//! End-to-end visual-rendering tests for the `layout` component.

use waterui::View;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::text::text;
use waterui_layout::scroll;
use waterui_layout::stack::{hstack, vstack, zstack};
use waterui_testing::{Role, SemanticApp};

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

fn vstack_view() -> impl View {
    visual_shell(
        vstack((
            labeled_card("Upper card", 80.0, 40.0, Srgb::new(1.0, 0.1, 0.1)),
            labeled_card("Lower card", 80.0, 40.0, Srgb::new(0.1, 1.0, 0.1)),
        ))
        .spacing(12.0),
    )
}

#[waterui::test(vstack_view, theme = hydrolysis_m3::install, viewport = (180, 180))]
fn vstack_renders_children_vertically(app: &mut SemanticApp) {
    let upper = app.query().role(Role::LABEL).label("Upper card").single();
    let lower = app.query().role(Role::LABEL).label("Lower card").single();
    assert!(
        upper.bounds().y() + upper.bounds().height() <= lower.bounds().y(),
        "vstack should place the upper card above the lower card: upper={:?} lower={:?}",
        upper.bounds(),
        lower.bounds()
    );
}

fn hstack_view() -> impl View {
    visual_shell(
        hstack((
            labeled_card("Left card", 40.0, 80.0, Srgb::new(1.0, 0.1, 0.1)),
            labeled_card("Right card", 40.0, 80.0, Srgb::new(0.1, 1.0, 0.1)),
        ))
        .spacing(12.0),
    )
}

#[waterui::test(hstack_view, theme = hydrolysis_m3::install, viewport = (180, 180))]
fn hstack_renders_children_horizontally(app: &mut SemanticApp) {
    let left = app.query().role(Role::LABEL).label("Left card").single();
    let right = app.query().role(Role::LABEL).label("Right card").single();
    assert!(
        left.bounds().x() + left.bounds().width() <= right.bounds().x(),
        "hstack should place the left card before the right card: left={:?} right={:?}",
        left.bounds(),
        right.bounds()
    );
}

fn zstack_view() -> impl View {
    visual_shell(zstack((
        labeled_card("Background layer", 120.0, 120.0, Srgb::new(1.0, 0.1, 0.1)),
        labeled_card("Overlay layer", 60.0, 60.0, Srgb::new(0.1, 1.0, 0.1)),
    )))
}

#[waterui::test(zstack_view, theme = hydrolysis_m3::install, viewport = (180, 180))]
fn zstack_overlays_children(app: &mut SemanticApp) {
    let background = app
        .query()
        .role(Role::LABEL)
        .label("Background layer")
        .single();
    let overlay = app
        .query()
        .role(Role::LABEL)
        .label("Overlay layer")
        .single();
    let background_center = background.center();
    let overlay_center = overlay.center();
    assert!(
        (background_center.0 - overlay_center.0).abs() <= 1.0
            && (background_center.1 - overlay_center.1).abs() <= 1.0,
        "zstack should center overlay and background together: background={:?} overlay={:?}",
        background.bounds(),
        overlay.bounds()
    );
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

#[waterui::test(scroll_content_view, theme = hydrolysis_m3::install, viewport = (180, 180))]
fn scroll_view_scroll_down_changes_content(app: &mut SemanticApp) {
    let second_before = app
        .query()
        .role(Role::LABEL)
        .label("Second item")
        .single()
        .bounds();
    app.query().label("scroll-layout").scroll_down();

    let second_after = app
        .query()
        .role(Role::LABEL)
        .label("Second item")
        .single()
        .bounds();
    app.query()
        .role(Role::LABEL)
        .label("Third item")
        .assert_exists();
    assert!(
        second_after.y() < second_before.y(),
        "scrolling down should move earlier content upward: before={second_before:?} after={second_after:?}"
    );
}

/// Two texts too wide for the row. Without a priority they share the shortfall;
/// the prioritized one should keep its width while the other gives way.
fn layout_priority_view() -> impl View {
    visual_shell(
        hstack((
            text("Keep me whole")
                .body()
                .foreground(Srgb::WHITE)
                .background(Srgb::new(1.0, 0.1, 0.1))
                .layout_priority(1),
            text("I can shrink")
                .body()
                .foreground(Srgb::WHITE)
                .background(Srgb::new(0.1, 1.0, 0.1)),
        ))
        .spacing(0.0),
    )
}

#[waterui::test(layout_priority_view, theme = hydrolysis_m3::install, viewport = (160, 120))]
fn layout_priority_protects_the_prioritized_child(app: &mut SemanticApp) {
    let kept = app
        .query()
        .role(Role::LABEL)
        .label("Keep me whole")
        .single();
    let yielded = app.query().role(Role::LABEL).label("I can shrink").single();

    assert!(
        kept.bounds().width() > yielded.bounds().width(),
        "the prioritized child must keep more width than the one that gives way: \
         kept={:?} yielded={:?}",
        kept.bounds(),
        yielded.bounds()
    );
}
