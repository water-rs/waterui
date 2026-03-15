use waterui::Environment;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui_barcode::Barcode;
use waterui_testing::{Snapshot, TestHost};

const SUITE: &str = "barcode/visual";

fn host() -> TestHost {
    TestHost::new(Environment::new(), 220, 220)
}

fn opaque_pixels(snapshot: &Snapshot) -> usize {
    snapshot
        .rgba8
        .chunks_exact(4)
        .filter(|px| px[3] > 0)
        .count()
}

fn has_variation(snapshot: &Snapshot) -> bool {
    let mut pixels = snapshot.rgba8.chunks_exact(4);
    let Some(first) = pixels.next() else {
        return false;
    };
    pixels.any(|px| px != first)
}

#[test]
fn qr_code_renders_visible_pattern() {
    let captured = host().capture_snapshot(
        Barcode::qr("https://waterui.dev/testing")
            .size(180.0, 180.0)
            .padding_with(20.0)
            .background(Srgb::WHITE),
        SUITE,
        "qr-code-renders-visible-pattern",
        "00_initial",
    );

    assert!(
        captured.path().is_file(),
        "qr-code-renders-visible-pattern: snapshot missing"
    );
    let opaque = opaque_pixels(captured.snapshot());
    assert!(
        opaque > 0,
        "qr-code-renders-visible-pattern: expected visible pixels (opaque={opaque})"
    );
    assert!(
        has_variation(captured.snapshot()),
        "qr-code-renders-visible-pattern: expected non-uniform output"
    );
}
