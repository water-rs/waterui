use waterui::Environment;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::text::{styled, text};
use waterui_testing::{Snapshot, TestHost};

const SUITE: &str = "text/visual";

fn host() -> TestHost {
    TestHost::new(Environment::new(), 280, 120)
}

fn bright_pixels(snapshot: &Snapshot) -> usize {
    snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0 && px[0] > 180 && px[1] > 180 && px[2] > 180)
        .count()
}

#[test]
fn text_renders_visible_content() {
    let captured = host().capture_snapshot(
        text("Visible content")
            .body()
            .foreground(Srgb::WHITE)
            .padding_with(16.0)
            .background(Srgb::BLACK),
        SUITE,
        "text-renders-visible-content",
        "00_initial",
    );

    assert!(
        captured.path().is_file(),
        "text-renders-visible-content: snapshot missing"
    );
    let bright = bright_pixels(captured.snapshot());
    assert!(
        bright > 0,
        "text-renders-visible-content: expected visible bright pixels (bright={bright})"
    );
}

#[test]
fn styled_text_renders_multiple_styles() {
    let captured = host().capture_snapshot(
        text(styled::StyledStr::from_markdown(
            "Plain *italic* **bold** `code`",
        ))
        .body()
        .foreground(Srgb::WHITE)
        .padding_with(16.0)
        .background(Srgb::BLACK),
        SUITE,
        "styled-text-renders-multiple-styles",
        "00_initial",
    );

    assert!(
        captured.path().is_file(),
        "styled-text-renders-multiple-styles: snapshot missing"
    );
    let bright = bright_pixels(captured.snapshot());
    assert!(
        bright > 0,
        "styled-text-renders-multiple-styles: expected visible bright pixels (bright={bright})"
    );
}
