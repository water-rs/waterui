//! End-to-end form rendering: the form-showcase view vocabulary (derive
//! forms, manual controls, styled text, dividers, scroll, progress) rendered
//! through the full dew pipeline.
//!
//! Exports `/tmp/dew_form_render.png` for visual review.

use waterui::prelude::*;
use waterui::prelude::{slider::slider, stepper::stepper};
use waterui::reactive::binding;
use waterui_core::AnyView;
use waterui_dew::{DrawCommand, render_view_png, theme};

mod support;

// The prelude glob also exports `waterui::test`; pull the built-in test
// attribute back into scope explicitly.
use core::prelude::v1::test;

/// Representative auto-generated form: one field per supported type.
#[form]
struct DeviceSettings {
    /// Device display name
    name: String,
    /// Enable the wireless radio
    wireless: bool,
    /// Transmit power level
    power: f64,
    /// Heartbeat interval in seconds
    heartbeat: i32,
}

const WIDTH: u32 = 410;
const HEIGHT: u32 = 600;

// The bindings must be owned for the returned view's lifetime even though
// the control constructors borrow them — taking `&Binding` would dangle.
#[expect(
    clippy::needless_pass_by_value,
    reason = "owned bindings outlive the view tree they back; borrowing would dangle"
)]
fn form_screen(
    settings: Binding<DeviceSettings>,
    hostname: Binding<Str>,
    enabled: Binding<bool>,
    retries: Binding<i32>,
    signal: Binding<f64>,
) -> impl View {
    let screen = scroll(
        vstack((
            text("Device Setup").title(),
            "Configure the device before deployment",
            Divider,
            vstack((
                text("Generated Form").sub_headline(),
                form(&settings),
                text!("Name: {name}", name = settings.project().name),
            )),
            Divider,
            vstack((
                text("Manual Controls").sub_headline(),
                TextField::new("Hostname", &hostname).prompt("device-01"),
                Toggle::new("Enable Feature", &enabled),
                stepper("Retry Count", &retries).range(0..=10).step(1),
                slider("Signal", &signal),
                progress(signal.clone()),
            )),
            Divider,
            hstack((
                text("Status:").bold(),
                text!("{enabled}", enabled = enabled),
            )),
        ))
        .padding(),
    );
    zstack((Color::srgb(255, 255, 255), screen))
}

fn build_screen() -> impl View {
    form_screen(
        DeviceSettings::binding(),
        binding(""),
        binding(true),
        binding(3),
        binding(0.5),
    )
}

/// Renders the form UI to a PNG, with deterministic spot checks on the
/// white background fill, and exports it for visual review.
#[test]
fn form_ui_renders_to_png() {
    let png = render_view_png(build_screen, support::test_environment(), WIDTH, HEIGHT);
    std::fs::write(support::export_path("form", "render"), &png).expect("export form render PNG");

    let pixmap = vello_cpu::Pixmap::from_png(std::io::Cursor::new(png.as_slice()))
        .expect("png decodes back");
    let pixel = |x: usize, y: usize| {
        let p = pixmap.data()[y * WIDTH as usize + x];
        [p.r, p.g, p.b, p.a]
    };
    // The white background fill stretches the full window; the padded
    // corners contain no widget pixels.
    assert_eq!(pixel(2, 2), [255, 255, 255, 255]);
    assert_eq!(pixel(WIDTH as usize - 3, 2), [255, 255, 255, 255]);
    assert_eq!(pixel(2, HEIGHT as usize - 3), [255, 255, 255, 255]);
}

/// The retained scene for the form must contain the expected widget
/// structure: styled title glyphs, accent fills for the on-toggle, slider
/// fill, and progress fill, plus the viewport clip from the scroll view.
#[test]
fn form_scene_contains_expected_widget_commands() {
    let mut renderer = support::test_renderer();
    let list = renderer.render_tree(
        AnyView::new(build_screen()),
        &support::test_environment(),
        f64::from(WIDTH),
        f64::from(HEIGHT),
    );
    let commands = list.commands();

    let title_runs = commands
        .iter()
        .filter(|placed| {
            matches!(
                placed.command(),
                DrawCommand::GlyphRun { font_size, .. } if (font_size - 24.0).abs() < f32::EPSILON
            )
        })
        .count();
    assert!(title_runs >= 1, "the .title() text must shape at 24px");

    let subheadline_runs = commands
        .iter()
        .filter(|placed| {
            matches!(
                placed.command(),
                DrawCommand::GlyphRun { font_size, .. } if (font_size - 20.0).abs() < f32::EPSILON
            )
        })
        .count();
    assert!(
        subheadline_runs >= 2,
        "both .sub_headline() sections must shape at 20px"
    );

    let accent_fills = commands
        .iter()
        .filter(|placed| {
            matches!(
                placed.command(),
                DrawCommand::FillPath { brush: peniko::Brush::Solid(color), .. }
                    if color.components.iter().zip(theme::ACCENT.components)
                        .all(|(actual, expected)| (actual - expected).abs() < 1.0e-6)
            )
        })
        .count();
    assert!(
        accent_fills >= 3,
        "expected accent fills from the on-toggle, slider, and progress, got {accent_fills}"
    );

    let clipped = commands
        .iter()
        .filter(|placed| placed.command().clip().is_some())
        .count();
    assert!(
        clipped > 0,
        "scroll content must record the viewport clip on its commands"
    );
}
