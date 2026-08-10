//! End-to-end view dispatch tests: real `WaterUI` view trees rendered
//! through layout, text shaping, and the banded flush pipeline.
//!
//! Run with `--nocapture` to export `/tmp/waterui_dew_views.png` for visual
//! review.

use core::cell::Cell;
use std::rc::Rc;

use nami::binding;
use waterui::prelude::{Color, text, vstack};
use waterui_backend_core::input::TouchPhase;
use waterui_controls::button::button;
use waterui_controls::slider::slider;
use waterui_controls::stepper::stepper;
use waterui_controls::toggle::toggle;
use waterui_core::interaction::Disabled;
use waterui_core::{AnyView, Environment, Metadata, env::use_env};
use waterui_dew::{DewRuntime, HostBoard, PointerSample, render_view_png};

mod support;

fn click(runtime: &mut DewRuntime<HostBoard>, x: f64, y: f64) {
    runtime.board_mut().push_pointer(PointerSample {
        x,
        y,
        phase: TouchPhase::Started,
    });
    runtime.board_mut().push_pointer(PointerSample {
        x,
        y,
        phase: TouchPhase::Ended,
    });
    runtime
        .pump()
        .expect("control input must drive one retained refresh");
}

/// Two stacked colors must split the screen, proving measure → place →
/// render flows through a real `VStack` layout.
#[test]
fn vstack_of_colors_splits_the_screen() {
    let png = render_view_png(
        || vstack((Color::red(), Color::blue())),
        support::test_environment(),
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
    let env = support::test_environment();
    let mut runtime = DewRuntime::new(HostBoard::new(240, 80), env, 16, || {
        AnyView::new(text("Hello, dew!"))
    });
    runtime.pump().expect("first frame renders");
    let dark_pixels = runtime
        .board()
        .framebuffer()
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] == 255 && px[0] < 128)
        .count();
    assert!(
        dark_pixels > 20,
        "expected visible glyph pixels, found {dark_pixels}"
    );
}

/// A `Binding` change must trigger exactly one retained refresh whose dirty region
/// stays local to the text that changed — the flush-economy contract.
#[test]
fn binding_change_dirties_only_the_text_region() {
    let count = binding(1);
    let count_for_root = count.clone();
    let env = support::test_environment();
    let mut runtime = DewRuntime::new(HostBoard::new(240, 240), env, 16, move || {
        let count = count_for_root.clone();
        AnyView::new(text!("Count: {count}"))
    });

    let first = runtime.pump().expect("initial frame renders");
    assert_eq!(first.len(), 1, "first frame is one full-screen rect");
    assert!(runtime.pump().is_none(), "clean frame must not render");

    count.set(2);
    let dirty = runtime
        .pump()
        .expect("binding change must request a retained refresh");
    assert!(!dirty.is_empty(), "changed text must produce dirty rects");
    for rect in &dirty {
        assert!(
            rect.width() < 240.0 && rect.height() < 120.0,
            "dirty rect should stay local to the text, got {rect:?}"
        );
    }
}

#[test]
fn button_click_invokes_action_once() {
    let invocations = Rc::new(Cell::new(0));
    let action_invocations = Rc::clone(&invocations);
    let mut runtime = DewRuntime::new(
        HostBoard::new(200, 48),
        support::test_environment(),
        16,
        move || {
            let action_invocations = Rc::clone(&action_invocations);
            AnyView::new(button("Purchase").action(move || {
                action_invocations.set(action_invocations.get() + 1);
            }))
        },
    );
    runtime.pump().expect("initial frame renders");

    click(&mut runtime, 100.0, 24.0);

    assert_eq!(invocations.get(), 1);
}

#[test]
fn disabled_button_ignores_pointer_input() {
    let invocations = Rc::new(Cell::new(0));
    let action_invocations = Rc::clone(&invocations);
    let mut runtime = DewRuntime::new(
        HostBoard::new(200, 48),
        support::test_environment(),
        16,
        move || {
            let action_invocations = Rc::clone(&action_invocations);
            AnyView::new(use_env(move |mut env: Environment| {
                Disabled::install(&mut env, true);
                Metadata::new(
                    button("Unavailable").action(move || {
                        action_invocations.set(action_invocations.get() + 1);
                    }),
                    env,
                )
            }))
        },
    );
    runtime.pump().expect("initial frame renders");
    runtime.board_mut().push_pointer(PointerSample {
        x: 100.0,
        y: 24.0,
        phase: TouchPhase::Started,
    });
    runtime.board_mut().push_pointer(PointerSample {
        x: 100.0,
        y: 24.0,
        phase: TouchPhase::Ended,
    });

    assert!(
        runtime.pump().is_none(),
        "disabled control must not request a refresh"
    );
    assert_eq!(invocations.get(), 0);
}

#[test]
fn toggle_click_updates_binding() {
    let enabled = binding(false);
    let enabled_for_root = enabled.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(200, 48),
        support::test_environment(),
        16,
        move || AnyView::new(toggle("Ready", &enabled_for_root)),
    );
    runtime.pump().expect("initial frame renders");

    click(&mut runtime, 180.0, 24.0);

    assert!(enabled.get());
}

#[test]
fn slider_drag_maps_pointer_to_range() {
    let amount = binding(0.0);
    let amount_for_root = amount.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(200, 60),
        support::test_environment(),
        16,
        move || AnyView::new(slider("Amount", &amount_for_root).range(0.0..=100.0)),
    );
    runtime.pump().expect("initial frame renders");

    click(&mut runtime, 145.0, 42.0);

    assert!(
        (amount.get() - 75.0).abs() < 0.01,
        "slider should map the track position to its configured range"
    );
}

#[test]
fn stepper_buttons_apply_step_and_range() {
    let quantity = binding(3);
    let quantity_for_root = quantity.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(200, 60),
        support::test_environment(),
        16,
        move || AnyView::new(stepper("Quantity", &quantity_for_root).range(1..=5).step(2)),
    );
    runtime.pump().expect("initial frame renders");

    click(&mut runtime, 186.0, 30.0);
    assert_eq!(quantity.get(), 5);

    click(&mut runtime, 150.0, 30.0);
    assert_eq!(quantity.get(), 3);
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
        support::test_environment(),
        320,
        160,
    );
    std::fs::write("/tmp/waterui_dew_views.png", png).expect("export visual review PNG");
}
