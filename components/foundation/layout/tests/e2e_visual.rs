//! End-to-end visual-rendering tests for the `layout` component.

use hydrolysis_m3::install as install_m3;
use waterui::View;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::text::text;
use waterui_layout::scroll;
use waterui_layout::stack::{hstack, vstack, zstack};
use waterui_testing::{Role, SemanticApp, ui};

const VIEWPORT_WIDTH: u32 = 180;
const VIEWPORT_HEIGHT: u32 = 180;

fn mount_view<V, F>(build: F) -> SemanticApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
        ui().theme(install_m3)
        .viewport(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
        .mount(build)
}

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

#[test]
fn vstack_renders_children_vertically() {
    let mut app = mount_view(|| {
        visual_shell(
            vstack((
                labeled_card("Upper card", 80.0, 40.0, Srgb::new(1.0, 0.1, 0.1)),
                labeled_card("Lower card", 80.0, 40.0, Srgb::new(0.1, 1.0, 0.1)),
            ))
            .spacing(12.0),
        )
    });

    let upper = app.query().role(Role::LABEL).label("Upper card").single();
    let lower = app.query().role(Role::LABEL).label("Lower card").single();
    assert!(
        upper.bounds().y() + upper.bounds().height() <= lower.bounds().y(),
        "vstack should place the upper card above the lower card: upper={:?} lower={:?}",
        upper.bounds(),
        lower.bounds()
    );
}

#[test]
fn hstack_renders_children_horizontally() {
    let mut app = mount_view(|| {
        visual_shell(
            hstack((
                labeled_card("Left card", 40.0, 80.0, Srgb::new(1.0, 0.1, 0.1)),
                labeled_card("Right card", 40.0, 80.0, Srgb::new(0.1, 1.0, 0.1)),
            ))
            .spacing(12.0),
        )
    });

    let left = app.query().role(Role::LABEL).label("Left card").single();
    let right = app.query().role(Role::LABEL).label("Right card").single();
    assert!(
        left.bounds().x() + left.bounds().width() <= right.bounds().x(),
        "hstack should place the left card before the right card: left={:?} right={:?}",
        left.bounds(),
        right.bounds()
    );
}

#[test]
fn zstack_overlays_children() {
    let mut app = mount_view(|| {
        visual_shell(zstack((
            labeled_card("Background layer", 120.0, 120.0, Srgb::new(1.0, 0.1, 0.1)),
            labeled_card("Overlay layer", 60.0, 60.0, Srgb::new(0.1, 1.0, 0.1)),
        )))
    });

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

#[test]
fn scroll_view_scroll_down_changes_content() {
    let mut app = mount_view(|| {
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
    });

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
