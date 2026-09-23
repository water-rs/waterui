//! End-to-end view dispatch tests: real `WaterUI` view trees rendered
//! through layout, text shaping, and the banded flush pipeline.
//!
//! Run with `--nocapture` to export `/tmp/waterui_dew_views.png` for visual
//! review.

use nami::binding;
use waterui::prelude::{Color, text, vstack};
use waterui_core::{AnyView, Environment};
use waterui_dew::{BufferDisplay, DewRuntime, render_view_png};

/// Two stacked colors must split the screen, proving measure → place →
/// render flows through a real `VStack` layout.
#[test]
fn vstack_of_colors_splits_the_screen() {
    let png = render_view_png(
        || vstack((Color::red(), Color::blue())),
        Environment::new(),
        128,
        128,
    );
    let pixmap = vello_cpu::Pixmap::from_png(std::io::Cursor::new(png.as_slice()))
        .expect("png decodes back");
    let pixel = |x: usize, y: usize| {
        let p = pixmap.data()[y * 128 + x];
        [p.r, p.g, p.b]
    };
    let top = pixel(64, 20);
    let bottom = pixel(64, 108);
    assert!(
        top[0] > 150 && top[2] < 100,
        "top half should be red, got {top:?}"
    );
    assert!(
        bottom[2] > 150 && bottom[0] < 100,
        "bottom half should be blue, got {bottom:?}"
    );
}

/// Text must produce visible glyphs: dark pixels somewhere in the layout.
#[test]
fn text_renders_visible_glyphs() {
    let env = Environment::new();
    let display = BufferDisplay::new(240, 80);
    let mut runtime = DewRuntime::new(display, env, 16, || AnyView::new(text("Hello, dew!")));
    runtime.pump().expect("first frame renders");
    let dark_pixels = runtime
        .display()
        .pixels()
        .chunks_exact(4)
        .filter(|px| px[3] == 255 && px[0] < 128)
        .count();
    assert!(
        dark_pixels > 20,
        "expected visible glyph pixels, found {dark_pixels}"
    );
}

/// A `Binding` change must trigger exactly one rebuild whose dirty region
/// stays local to the text that changed — the flush-economy contract.
#[test]
fn binding_change_dirties_only_the_text_region() {
    let count = binding(1);
    let count_for_root = count.clone();
    let env = Environment::new();
    let display = BufferDisplay::new(240, 240);
    let mut runtime = DewRuntime::new(display, env, 16, move || {
        let count = count_for_root.clone();
        AnyView::new(text!("Count: {count}"))
    });

    let first = runtime.pump().expect("initial frame renders");
    assert_eq!(first.len(), 1, "first frame is one full-screen rect");
    assert!(runtime.pump().is_none(), "clean frame must not render");

    count.set(2);
    let dirty = runtime
        .pump()
        .expect("binding change must request a rebuild");
    assert!(!dirty.is_empty(), "changed text must produce dirty rects");
    for rect in &dirty {
        assert!(
            rect.width() < 240.0 && rect.height() < 120.0,
            "dirty rect should stay local to the text, got {rect:?}"
        );
    }
}

/// Visual review artifact: a small composed UI.
#[test]
fn export_composed_ui_for_visual_review() {
    let png = render_view_png(
        || {
            vstack((
                text("Dew renders WaterUI"),
                Color::cyan(),
                text("vello_cpu + banded flush"),
            ))
        },
        Environment::new(),
        320,
        160,
    );
    std::fs::write("/tmp/waterui_dew_views.png", png).expect("export visual review PNG");
}
