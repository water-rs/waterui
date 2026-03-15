use waterui::View;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui_layout::scroll;
use waterui_layout::stack::{hstack, vstack, zstack};
use waterui_testing::{MountedApp, Snapshot, UiTest};

const VIEWPORT_WIDTH: u32 = 180;
const VIEWPORT_HEIGHT: u32 = 180;
const SUITE: &str = "layout/visual";

fn mount_view<V, F>(build: F) -> MountedApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    UiTest::new()
        .viewport(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
        .mount(build)
}

fn visual_shell<V: View>(content: V) -> impl View {
    content.padding_with(20.0).background(Srgb::BLACK)
}

fn pixel_at(snapshot: &Snapshot, x: u32, y: u32) -> [u8; 4] {
    assert!(x < snapshot.width, "x coordinate out of bounds");
    assert!(y < snapshot.height, "y coordinate out of bounds");
    let offset = ((y * snapshot.width + x) * 4) as usize;
    [
        snapshot.rgba8[offset],
        snapshot.rgba8[offset + 1],
        snapshot.rgba8[offset + 2],
        snapshot.rgba8[offset + 3],
    ]
}

fn visible_pixels(snapshot: &Snapshot) -> usize {
    snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0)
        .count()
}

fn assert_is_red(pixel: [u8; 4], context: &str) {
    assert!(
        pixel[3] > 0 && pixel[0] > 150 && pixel[1] < 120 && pixel[2] < 120,
        "{context}: expected red-dominant pixel, got {pixel:?}"
    );
}

fn assert_is_green(pixel: [u8; 4], context: &str) {
    assert!(
        pixel[3] > 0 && pixel[1] > 150 && pixel[0] < 120 && pixel[2] < 120,
        "{context}: expected green-dominant pixel, got {pixel:?}"
    );
}

#[test]
fn vstack_renders_children_vertically() {
    let mut app = mount_view(|| {
        visual_shell(
            vstack((
                Srgb::new(1.0, 0.1, 0.1).size(80.0, 40.0),
                Srgb::new(0.1, 1.0, 0.1).size(80.0, 40.0),
            ))
            .spacing(12.0),
        )
    });

    let captured = app.capture_snapshot(SUITE, "vstack-renders-children-vertically", "00_initial");
    assert!(
        captured.path().is_file(),
        "vstack-renders-children-vertically: snapshot missing"
    );
    assert!(
        visible_pixels(captured.snapshot()) > 0,
        "vstack-renders-children-vertically: expected visible pixels"
    );
    assert_is_red(pixel_at(captured.snapshot(), 30, 30), "vstack first child");
    assert_is_green(pixel_at(captured.snapshot(), 30, 82), "vstack second child");
}

#[test]
fn hstack_renders_children_horizontally() {
    let mut app = mount_view(|| {
        visual_shell(
            hstack((
                Srgb::new(1.0, 0.1, 0.1).size(40.0, 80.0),
                Srgb::new(0.1, 1.0, 0.1).size(40.0, 80.0),
            ))
            .spacing(12.0),
        )
    });

    let captured =
        app.capture_snapshot(SUITE, "hstack-renders-children-horizontally", "00_initial");
    assert!(
        captured.path().is_file(),
        "hstack-renders-children-horizontally: snapshot missing"
    );
    assert!(
        visible_pixels(captured.snapshot()) > 0,
        "hstack-renders-children-horizontally: expected visible pixels"
    );
    assert_is_red(pixel_at(captured.snapshot(), 30, 30), "hstack left child");
    assert_is_green(pixel_at(captured.snapshot(), 82, 30), "hstack right child");
}

#[test]
fn zstack_overlays_children() {
    let mut app = mount_view(|| {
        visual_shell(zstack((
            Srgb::new(1.0, 0.1, 0.1).size(120.0, 120.0),
            Srgb::new(0.1, 1.0, 0.1).size(60.0, 60.0),
        )))
    });

    let captured = app.capture_snapshot(SUITE, "zstack-overlays-children", "00_initial");
    assert!(
        captured.path().is_file(),
        "zstack-overlays-children: snapshot missing"
    );
    assert_is_red(
        pixel_at(captured.snapshot(), 30, 30),
        "zstack background child",
    );
    assert_is_green(
        pixel_at(captured.snapshot(), 80, 80),
        "zstack overlay child",
    );
}

#[test]
fn scroll_view_scroll_down_changes_content() {
    let mut app = mount_view(|| {
        visual_shell(
            scroll(
                vstack((
                    Srgb::new(1.0, 0.1, 0.1).size(120.0, 48.0),
                    Srgb::new(0.1, 1.0, 0.1).size(120.0, 48.0),
                    Srgb::new(0.1, 0.1, 1.0).size(120.0, 48.0),
                    Srgb::new(1.0, 0.8, 0.1).size(120.0, 48.0),
                ))
                .spacing(12.0),
            )
            .size(120.0, 120.0)
            .a11y_label("scroll-layout"),
        )
    });

    let initial = app.capture_snapshot(
        SUITE,
        "scroll-view-scroll-down-changes-content",
        "00_initial",
    );
    assert!(
        initial.path().is_file(),
        "scroll-view-scroll-down-changes-content: initial snapshot missing"
    );

    assert!(
        app.query().label("scroll-layout").scroll_down(),
        "scroll view scroll_down should succeed"
    );

    let scrolled = app.capture_snapshot(
        SUITE,
        "scroll-view-scroll-down-changes-content",
        "01_scrolled",
    );
    assert!(
        scrolled.path().is_file(),
        "scroll-view-scroll-down-changes-content: scrolled snapshot missing"
    );
    assert!(
        initial.snapshot().changed_pixels(scrolled.snapshot()) > 0,
        "scroll view scroll_down should change rendered pixels"
    );
}
