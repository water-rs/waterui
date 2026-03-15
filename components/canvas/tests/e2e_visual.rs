use waterui::Environment;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::layout::{Point, Rect, Size};
use waterui_canvas::Canvas;
use waterui_testing::{Snapshot, TestHost};

const SUITE: &str = "canvas/visual";

fn host() -> TestHost {
    TestHost::new(Environment::new(), 180, 120)
}

fn colored_pixels(snapshot: &Snapshot) -> usize {
    snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0))
        .count()
}

#[test]
fn canvas_fill_rect_produces_colored_region() {
    let captured = host().capture_snapshot(
        Canvas::new(|ctx| {
            ctx.set_fill_style(Srgb::new(1.0, 0.25, 0.1));
            ctx.fill_rect(Rect::new(Point::new(20.0, 16.0), Size::new(100.0, 60.0)));
        })
        .size(160.0, 100.0)
        .background(Srgb::BLACK),
        SUITE,
        "canvas-fill-rect-produces-colored-region",
        "00_initial",
    );

    assert!(
        captured.path().is_file(),
        "canvas-fill-rect-produces-colored-region: snapshot missing"
    );
    let colored = colored_pixels(captured.snapshot());
    assert!(
        colored > 0,
        "canvas-fill-rect-produces-colored-region: expected colored pixels (colored={colored})"
    );
}
