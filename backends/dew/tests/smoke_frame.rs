//! End-to-end smoke test: a UI-like scene through the full dew pipeline
//! (display list → band scheduler → painter → display flush) at the
//! canonical 320×240 embedded panel size.
//!
//! Run with `--nocapture` to export `/tmp/waterui_dew_smoke.png` for visual
//! review.

use kurbo::{Affine, Circle, Rect, RoundedRect, Stroke};
use peniko::Color;
use waterui_dew::{BandScheduler, BufferDisplay, DisplayList, Painter, render_frame};

fn demo_scene() -> DisplayList {
    let mut list = DisplayList::new();
    // Background.
    list.fill(
        &Rect::new(0.0, 0.0, 320.0, 240.0),
        Affine::IDENTITY,
        Color::from_rgb8(24, 28, 38),
    );
    // A card.
    list.fill(
        &RoundedRect::new(20.0, 20.0, 300.0, 130.0, 12.0),
        Affine::IDENTITY,
        Color::from_rgb8(44, 52, 70),
    );
    list.stroke(
        &RoundedRect::new(20.0, 20.0, 300.0, 130.0, 12.0),
        Affine::IDENTITY,
        Stroke::new(2.0),
        Color::from_rgb8(90, 110, 150),
    );
    // A "button" pill on the card.
    list.fill(
        &RoundedRect::new(40.0, 84.0, 160.0, 114.0, 15.0),
        Affine::IDENTITY,
        Color::from_rgb8(70, 130, 240),
    );
    // An accent dot, deliberately crossing band boundaries.
    list.fill(
        &Circle::new((250.0, 180.0), 36.0),
        Affine::IDENTITY,
        Color::from_rgb8(240, 180, 60),
    );
    list
}

#[test]
fn full_frame_renders_through_bands() {
    let scheduler = BandScheduler::new(320, 240, 16);
    let mut painter = Painter::default();
    let mut display = BufferDisplay::new(320, 240);
    let list = demo_scene();

    render_frame(
        &mut painter,
        &list,
        &scheduler,
        &[Rect::new(0.0, 0.0, 320.0, 240.0)],
        &mut display,
    );

    // Spot checks: background, card body, button pill, accent dot.
    assert_eq!(display.pixel(5, 5), [24, 28, 38, 255]);
    assert_eq!(display.pixel(160, 60), [44, 52, 70, 255]);
    assert_eq!(display.pixel(100, 99), [70, 130, 240, 255]);
    assert_eq!(display.pixel(250, 180), [240, 180, 60, 255]);

    std::fs::write("/tmp/waterui_dew_smoke.png", display.to_png())
        .expect("failed to export smoke-frame PNG");
}

/// A partial update must only repaint the dirty region: pixels outside it
/// keep whatever the display already shows.
#[test]
fn partial_update_touches_only_dirty_regions() {
    let scheduler = BandScheduler::new(320, 240, 16);
    let mut painter = Painter::default();
    let mut display = BufferDisplay::new(320, 240);

    // The display still shows nothing (transparent black) outside the
    // dirty rect, even though the scene covers the whole screen.
    let list = demo_scene();
    render_frame(
        &mut painter,
        &list,
        &scheduler,
        &[Rect::new(40.0, 84.0, 160.0, 114.0)],
        &mut display,
    );

    assert_eq!(display.pixel(100, 99), [70, 130, 240, 255]);
    assert_eq!(display.pixel(5, 5), [0, 0, 0, 0]);
    assert_eq!(display.pixel(250, 180), [0, 0, 0, 0]);
}
