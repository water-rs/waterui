//! Gradient Example - Demonstrates `WaterUI`'s gradient system
//!
//! This example showcases the gradient views:
//! - A mesh gradient animated with a repeating hue rotation
//! - An animated mesh gradient driven by the canvas clock
//! - A GPU flowing shader gradient
//! - Linear, radial, angular gradients
//! - Mesh gradients with per-vertex colors
//! - Shape fills with gradients

use waterui::animation::Repeat;
use waterui::app::{App, Windows};
use waterui::color::Working;
use waterui::color::working::from_linear_srgb;
use waterui::layout::UnitPoint;
use waterui::prelude::*;
use waterui::preview;
use waterui::shape::CornerRadius;

use core::time::Duration;

/// A colour from linear sRGB components. Components above 1.0 are brighter
/// than SDR white and need an HDR display to show.
fn color(r: f32, g: f32, b: f32) -> Color {
    Color::new(Working(from_linear_srgb([r, g, b], 1.0)))
}

/// Base 3×3 mesh gradient palette (deep blues, purples, teals).
fn initial_mesh_colors() -> [Color; 9] {
    [
        color(0.1, 0.1, 0.3),   // Top-left: dark blue
        color(0.2, 0.1, 0.4),   // Top-center: purple
        color(0.3, 0.1, 0.3),   // Top-right: magenta
        color(0.1, 0.2, 0.4),   // Mid-left: blue
        color(0.2, 0.3, 0.5),   // Center: lighter blue
        color(0.3, 0.2, 0.4),   // Mid-right: purple
        color(0.05, 0.15, 0.3), // Bottom-left: dark teal
        color(0.1, 0.2, 0.35),  // Bottom-center: blue
        color(0.15, 0.1, 0.25), // Bottom-right: dark purple
    ]
}

/// Demo: mesh gradient whose hue cycles under a repeating `Animation`
fn animated_background_section() -> impl View {
    let active = Binding::new(false);
    let hue = active.select(Angle::degrees(180.0), Angle::ZERO).with(
        Animation::ease_in_out(Duration::from_secs(4))
            .repeat(Repeat::Forever { autoreverse: true }),
    );

    vstack((
        text("Animated Mesh Gradient").font_size(20.0),
        "Colors flow under a repeating animation without a manual clock loop",
        zstack((
            MeshGradient::new(3, 3, initial_mesh_colors())
                .size(300.0, 200.0)
                .hue_rotation(hue),
            vstack((
                text("Fluid Background")
                    .font_size(24.0)
                    .foreground(Color::srgb(255, 255, 255)),
                text("Colors flow over time").foreground(Color::srgb(200, 220, 255)),
            ))
            .padding(),
        ))
        .size(300.0, 200.0),
    ))
    .spacing(12.0)
    .padding()
    .on_appear(move || active.set(true))
}

/// Demo: mesh gradient animated by the canvas clock (no per-frame CPU updates)
fn gpu_animated_mesh_gradient_section() -> impl View {
    let animating = Binding::new(true);
    // The configuration is a signal: pausing sets the speed to zero, which
    // holds the gradient still and lets the engine idle.
    let config = animating.map(|animating| {
        let config = animated_mesh_config();
        if animating { config } else { config.speed(0.0) }
    });

    vstack((
        text("GPU Animated Mesh Gradient").font_size(20.0),
        "Speed, warp and palette follow a signal",
        zstack((
            AnimatedMeshGradient::new(config).size(300.0, 200.0),
            vstack((
                text("Mesh Gradient")
                    .font_size(24.0)
                    .foreground(Color::srgb(255, 255, 255)),
                text("No per-frame CPU updates").foreground(Color::srgb(200, 220, 255)),
            ))
            .padding(),
        ))
        .size(300.0, 200.0),
        toggle("Animate", &animating),
    ))
    .spacing(12.0)
    .padding()
}

fn animated_mesh_config() -> AnimatedMeshGradientConfig {
    AnimatedMeshGradientConfig::aqua_bloom()
}

#[preview]
fn animated_mesh_gradient_preview() -> impl View {
    AnimatedMeshGradient::new(animated_mesh_config()).size(640.0, 360.0)
}

/// Demo: GPU flowing shader gradient
fn flowing_gradient_section() -> impl View {
    vstack((
        text("Flowing Gradient").font_size(20.0),
        "Noise-driven colour bands drifting on the GPU",
        FlowingGradient::new().size(300.0, 200.0),
    ))
    .spacing(12.0)
    .padding()
}

/// Demo: Shape filled with gradient
fn shape_fill_section() -> impl View {
    vstack((
        text("Shape + Gradient Fill").font_size(20.0),
        "Gradients clipped to shapes on the GPU",
        hstack((
            // Linear gradient clipped to rounded rectangle
            vstack((
                Gradient::linear(
                    [color(1.0, 0.3, 0.5), color(0.3, 0.5, 1.0)],
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                )
                .size(100.0, 100.0)
                .clip(RoundedRectangle::new(CornerRadius::fraction(0.18))),
                "Linear + Rounded",
            )),
            // Radial gradient clipped to circle
            vstack((
                Gradient::radial(
                    vec![
                        (0.0, color(1.0, 1.0, 0.8)),
                        (0.5, color(1.0, 0.6, 0.2)),
                        (1.0, color(0.6, 0.2, 0.1)),
                    ],
                    UnitPoint::CENTER,
                    0.0,
                    0.7,
                )
                .size(100.0, 100.0)
                .clip(Circle),
                "Radial + Circle",
            )),
            // Mesh gradient clipped to rounded rectangle
            vstack((
                MeshGradient::new(
                    2,
                    2,
                    [
                        color(0.0, 0.8, 0.4),
                        color(0.0, 0.4, 0.8),
                        color(0.8, 0.4, 0.0),
                        color(0.8, 0.0, 0.4),
                    ],
                )
                .size(100.0, 100.0)
                .clip(RoundedRectangle::new(CornerRadius::fraction(0.22))),
                "Mesh + Rounded",
            )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Linear gradient - horizontal, vertical, and diagonal
fn linear_gradient_section() -> impl View {
    vstack((
        text("Linear Gradients").font_size(20.0),
        "Gradients along a line from start to end point",
        hstack((
            // Horizontal: left to right
            vstack((
                Gradient::linear(
                    vec![
                        (0.0, color(1.0, 0.0, 0.0)), // Red
                        (0.5, color(1.0, 1.0, 0.0)), // Yellow
                        (1.0, color(0.0, 1.0, 0.0)), // Green
                    ],
                    UnitPoint::LEADING,
                    UnitPoint::TRAILING,
                )
                .size(120.0, 80.0),
                "Horizontal",
            )),
            // Vertical: top to bottom
            vstack((
                Gradient::linear(
                    [
                        color(0.0, 0.5, 1.0), // Sky blue
                        color(0.0, 0.0, 0.5), // Dark blue
                    ],
                    UnitPoint::TOP,
                    UnitPoint::BOTTOM,
                )
                .size(120.0, 80.0),
                "Vertical",
            )),
            // Diagonal
            vstack((
                Gradient::linear(
                    [
                        color(1.0, 0.0, 1.0), // Magenta
                        color(0.0, 1.0, 1.0), // Cyan
                    ],
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                )
                .size(120.0, 80.0),
                "Diagonal",
            )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Radial gradient - expanding from center
fn radial_gradient_section() -> impl View {
    vstack((
        text("Radial Gradients").font_size(20.0),
        "Gradients expanding outward from a center point",
        hstack((
            // Centered radial
            vstack((
                Gradient::radial(
                    vec![
                        (0.0, color(1.0, 1.0, 1.0)), // White center
                        (0.5, color(1.0, 0.8, 0.0)), // Yellow
                        (1.0, color(1.0, 0.3, 0.0)), // Orange-red
                    ],
                    UnitPoint::CENTER,
                    0.0, // Start radius (point)
                    0.7, // End radius
                )
                .size(120.0, 120.0),
                "Centered",
            )),
            // Off-center radial
            vstack((
                Gradient::radial(
                    [
                        color(1.0, 1.0, 1.0), // White
                        color(0.2, 0.4, 1.0), // Blue
                    ],
                    UnitPoint::new(0.3, 0.3), // Off-center
                    0.0,
                    0.8,
                )
                .size(120.0, 120.0),
                "Off-center",
            )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Mesh gradient - bilinear interpolation between vertex colors
fn mesh_gradient_section() -> impl View {
    vstack((
        text("Static Mesh Gradients").font_size(20.0),
        "Gradients with per-vertex colors interpolated across a grid",
        hstack((
            // 2x2 mesh (4 corners)
            vstack((
                MeshGradient::new(
                    2,
                    2,
                    [
                        color(1.0, 0.0, 0.0), // Top-left: Red
                        color(0.0, 1.0, 0.0), // Top-right: Green
                        color(0.0, 0.0, 1.0), // Bottom-left: Blue
                        color(1.0, 1.0, 0.0), // Bottom-right: Yellow
                    ],
                )
                .size(120.0, 120.0),
                "2x2 Corners",
            )),
            // 3x3 mesh with center highlight
            vstack((
                MeshGradient::new(
                    3,
                    3,
                    [
                        color(0.2, 0.2, 0.4),
                        color(0.3, 0.3, 0.5),
                        color(0.2, 0.2, 0.4),
                        color(0.3, 0.3, 0.5),
                        color(1.0, 1.0, 1.0), // Bright center
                        color(0.3, 0.3, 0.5),
                        color(0.2, 0.2, 0.4),
                        color(0.3, 0.3, 0.5),
                        color(0.2, 0.2, 0.4),
                    ],
                )
                .size(120.0, 120.0),
                "3x3 Highlight",
            )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: HDR gradients - extended dynamic range colors
fn hdr_gradient_section() -> impl View {
    vstack((
        text("HDR Gradients").font_size(20.0),
        "Extended brightness beyond SDR (requires HDR display)",
        hstack((
            // SDR comparison (baseline)
            vstack((
                Gradient::radial(
                    [
                        color(1.0, 1.0, 1.0), // SDR white
                        color(0.1, 0.1, 0.2), // Dark edge
                    ],
                    UnitPoint::CENTER,
                    0.0,
                    0.7,
                )
                .size(120.0, 120.0),
                "SDR White",
            )),
            // HDR bright white (1.5x brightness)
            vstack((
                Gradient::radial(
                    [
                        color(1.5, 1.5, 1.5), // HDR bright
                        color(0.1, 0.1, 0.2), // Dark edge
                    ],
                    UnitPoint::CENTER,
                    0.0,
                    0.7,
                )
                .size(120.0, 120.0),
                "HDR 1.5x",
            )),
            // HDR super bright (2x brightness)
            vstack((
                Gradient::radial(
                    [
                        color(2.0, 2.0, 2.0), // Very bright HDR
                        color(0.1, 0.1, 0.2), // Dark edge
                    ],
                    UnitPoint::CENTER,
                    0.0,
                    0.7,
                )
                .size(120.0, 120.0),
                "HDR 2x",
            )),
        ))
        .spacing(16.0),
        // HDR color highlights
        hstack((
            // HDR red highlight
            vstack((
                Gradient::linear(
                    [
                        color(2.0, 0.3, 0.3), // Bright HDR red
                        color(0.3, 0.0, 0.0), // Dark red
                    ],
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                )
                .size(120.0, 80.0),
                "HDR Red",
            )),
            // HDR green highlight
            vstack((
                Gradient::linear(
                    [
                        color(0.3, 2.0, 0.3), // Bright HDR green
                        color(0.0, 0.3, 0.0), // Dark green
                    ],
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                )
                .size(120.0, 80.0),
                "HDR Green",
            )),
            // HDR blue highlight
            vstack((
                Gradient::linear(
                    [
                        color(0.3, 0.3, 2.0), // Bright HDR blue
                        color(0.0, 0.0, 0.3), // Dark blue
                    ],
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                )
                .size(120.0, 80.0),
                "HDR Blue",
            )),
        ))
        .spacing(16.0),
    ))
    .spacing(12.0)
    .padding()
}

/// Root view: full gradient showcase.
pub fn demo() -> impl View {
    scroll(
        vstack((
            // Header
            text("WaterUI Gradient Examples").font_size(28.0),
            "GPU-rendered gradients with animation support",
            Divider,
            vstack((
                // Animated mesh gradient background
                animated_background_section(),
                Divider,
                // GPU animated mesh gradient
                gpu_animated_mesh_gradient_section(),
                Divider,
                // GPU flowing shader gradient
                flowing_gradient_section(),
            )),
            Divider,
            // Shape + gradient fill
            shape_fill_section(),
            Divider,
            // Linear gradients
            linear_gradient_section(),
            Divider,
            // Radial gradients
            radial_gradient_section(),
            Divider,
            vstack((
                // Mesh gradients
                mesh_gradient_section(),
                Divider,
                // HDR gradients
                hdr_gradient_section(),
            )),
        ))
        .padding_with(16.0),
    )
}

/// Builds the gradient example app around [`demo`].
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(env).window(Window::new("", demo))
}
