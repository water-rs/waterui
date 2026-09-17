//! The rendered pair `docs/jsx.md` embeds under "Attribute order is
//! semantic".
//!
//! JSX `<Text padding={16} background={…}>` and
//! `<Text background={…} padding={16}>` are the same attributes in two orders
//! and they mean two different views; the guide shows both as rendered
//! frames rather than describing the difference in prose alone.

use std::path::PathBuf;

use waterui::ViewExt as _;
use waterui::graphics::color::Color;
use waterui::text::text;
use waterui_testing::ui;

/// Regenerates the guide's two illustrations.
///
/// This is a documentation generator, not an assertion: it renders real
/// frames through the offscreen backend and writes PNGs, so it stays
/// ignored in the suite and runs on demand with
///
/// ```bash
/// cargo nextest run -p waterui-ts -E 'test(export_jsx_order_illustrations)' --run-ignored all
/// ```
///
/// Binary images are not tracked in this repository; they are served from
/// `assets.waterui.dev`. The frames are written under the target directory
/// (`CARGO_TARGET_TMPDIR/jsx-illustrations`, printed by the test), and the
/// guide references them at `https://assets.waterui.dev/docs/jsx/<name>`,
/// where a regenerated pair is uploaded to replace the old one.
///
/// The subject is the text the guide shows, on a fixed 240×120 viewport at
/// scale 1, in the same blue for both orders so only the order changes.
#[test]
#[ignore = "documentation generator: writes the guide's frames; run with --run-ignored all"]
fn export_jsx_order_illustrations() {
    let illustrations = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("jsx-illustrations");
    std::fs::create_dir_all(&illustrations).expect("creating the illustrations directory");
    tracing::info!(directory = %illustrations.display(), "writing the guide's frames");

    // `<Text padding={16} background={…}>`: the padding sits inside the
    // background, so the colour covers the inset too.
    let mut padding_then_background = ui().viewport(240, 120).mount_offscreen(|| {
        text("Attribute order")
            .padding_with(16.0)
            .background(Color::srgb(64, 128, 216))
    });
    padding_then_background
        .snapshot()
        .save_png(illustrations.join("jsx-order-padding-then-background.png"))
        .expect("writing jsx-order-padding-then-background.png");

    // `<Text background={…} padding={16}>`: the background hugs the text and
    // the padding is the transparent ring around it.
    let mut background_then_padding = ui().viewport(240, 120).mount_offscreen(|| {
        text("Attribute order")
            .background(Color::srgb(64, 128, 216))
            .padding_with(16.0)
    });
    background_then_padding
        .snapshot()
        .save_png(illustrations.join("jsx-order-background-then-padding.png"))
        .expect("writing jsx-order-background-then-padding.png");
}
