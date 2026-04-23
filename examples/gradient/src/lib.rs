//! Gradient Example - Demonstrates WaterUI's gradient system
//!
//! This example showcases GPU-rendered gradients:
//! - Animated fluid mesh gradient background (time-based, automatic)
//! - GPU flowing shader gradient (fully GPU-animated)
//! - Linear, radial, angular gradients
//! - Mesh gradients with per-vertex colors
//! - Shape fills with gradients

use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::shape::{Circle, RoundedRectangle};
use waterui::task::sleep;
use waterui_graphics::{
    AnimatedMeshGradient, AnimatedMeshGradientConfig, Gradient, MeshGradient, ResolvedColor,
};

use core::time::Duration;

/// Helper to create a resolved color (SDR)
fn color(r: f32, g: f32, b: f32) -> ResolvedColor {
    ResolvedColor {
        red: r,
        green: g,
        blue: b,
        opacity: 1.0,
        headroom: 0.0,
    }
}

/// Helper to create an HDR color with extended brightness
/// Values can exceed 1.0 for bright highlights on HDR displays
fn hdr_color(r: f32, g: f32, b: f32, headroom: f32) -> ResolvedColor {
    ResolvedColor {
        red: r,
        green: g,
        blue: b,
        opacity: 1.0,
        headroom,
    }
}

/// Computes animated mesh colors based on elapsed time.
/// Uses sine waves with different phases and frequencies for a dreamy fluid effect.
fn compute_animated_colors(time: f32) -> [ResolvedColor; 9] {
    // Base palette - deep blues, purples, teals
    let base_colors: [[f32; 3]; 9] = [
        [0.1, 0.1, 0.3],   // Top-left: dark blue
        [0.2, 0.1, 0.4],   // Top-center: purple
        [0.3, 0.1, 0.3],   // Top-right: magenta
        [0.1, 0.2, 0.4],   // Mid-left: blue
        [0.2, 0.3, 0.5],   // Center: lighter blue
        [0.3, 0.2, 0.4],   // Mid-right: purple
        [0.05, 0.15, 0.3], // Bottom-left: dark teal
        [0.1, 0.2, 0.35],  // Bottom-center: blue
        [0.15, 0.1, 0.25], // Bottom-right: dark purple
    ];

    let mut colors = [ResolvedColor::default(); 9];

    for (i, base) in base_colors.iter().enumerate() {
        // Each vertex has unique phase offsets for organic movement
        let phase = (i as f32) * 0.7;
        let x = (i % 3) as f32;
        let y = (i / 3) as f32;

        // Multiple sine waves at different frequencies for complex motion
        let wave1 = (time * 0.5 + phase).sin() * 0.15;
        let wave2 = (time * 0.3 + x * 0.5).sin() * 0.1;
        let wave3 = (time * 0.7 + y * 0.8).cos() * 0.08;

        // Apply waves to each color channel with different phases
        colors[i] = ResolvedColor {
            red: (base[0] + wave1 + wave2 * 0.5).clamp(0.0, 1.0),
            green: (base[1] + wave2 + wave3 * 0.5).clamp(0.0, 1.0),
            blue: (base[2] + wave3 + wave1 * 0.5).clamp(0.0, 1.0),
            opacity: 1.0,
            headroom: 0.0,
        };
    }

    colors
}

async fn animate_mesh_colors(colors: Binding<[ResolvedColor; 9]>) {
    let start = std::time::Instant::now();
    loop {
        let elapsed = start.elapsed().as_secs_f32();
        colors.set(compute_animated_colors(elapsed));
        sleep(Duration::from_millis(16)).await;
    }
}

/// Demo: Animated fluid mesh gradient background (automatic time-based animation)
fn animated_background_section() -> impl View {
    // Create binding for animated colors
    let colors: Binding<[ResolvedColor; 9]> = Binding::container(compute_animated_colors(0.0));

    vstack((
        text("Animated Mesh Gradient").size(20.0),
        "Automatic time-based fluid animation",
        // The animated mesh gradient
        zstack((
            // Background: MeshGradient accepts Signal directly!
            MeshGradient::new(3, 3, colors.clone()).size(300.0, 200.0),
            // Overlay content
            vstack((
                text("Fluid Background")
                    .size(24.0)
                    .foreground(Color::srgb(255, 255, 255)),
                text("Colors flow over time").foreground(Color::srgb(200, 200, 255)),
            ))
            .padding(),
        ))
        .size(300.0, 200.0),
    ))
    .spacing(12.0)
    .padding()
    .task(animate_mesh_colors(colors.clone()))
}

/// Demo: GPU flowing gradient (fully shader animated, no CPU updates)
fn gpu_animated_mesh_gradient_section() -> impl View {
    let gradient = AnimatedMeshGradient::new(animated_mesh_config());

    vstack((
        text("GPU Animated Mesh Gradient").size(20.0),
        "Speed + palette configured at creation time",
        zstack((
            gradient.size(300.0, 200.0),
            vstack((
                text("Mesh Gradient")
                    .size(24.0)
                    .foreground(Color::srgb(255, 255, 255)),
                text("No per-frame CPU updates").foreground(Color::srgb(200, 220, 255)),
            ))
            .padding(),
        ))
        .size(300.0, 200.0),
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

/// Demo: Shape filled with gradient
fn shape_fill_section() -> impl View {
    vstack((
        text("Shape + Gradient Fill").size(20.0),
        "Gradients clipped to shapes on the GPU",
        hstack((
            // Linear gradient clipped to rounded rectangle
            vstack((
                Gradient::linear(
                    vec![(0.0, color(1.0, 0.3, 0.5)), (1.0, color(0.3, 0.5, 1.0))],
                    [0.0, 0.0],
                    [1.0, 1.0],
                )
                .size(100.0, 100.0)
                .clip(RoundedRectangle::new(0.18)),
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
                    [0.5, 0.5],
                    0.0,
                    0.7,
                )
                .size(100.0, 100.0)
                .clip(Circle),
                "Radial + Circle",
            )),
            // Mesh gradient clipped to rounded rectangle
            vstack((
                Gradient::mesh(
                    2,
                    2,
                    vec![
                        ([0.0, 0.0], color(0.0, 0.8, 0.4)),
                        ([1.0, 0.0], color(0.0, 0.4, 0.8)),
                        ([0.0, 1.0], color(0.8, 0.4, 0.0)),
                        ([1.0, 1.0], color(0.8, 0.0, 0.4)),
                    ],
                    true,
                )
                .size(100.0, 100.0)
                .clip(RoundedRectangle::new(0.22)),
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
        text("Linear Gradients").size(20.0),
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
                    [0.0, 0.5], // Start: left center
                    [1.0, 0.5], // End: right center
                )
                .size(120.0, 80.0),
                "Horizontal",
            )),
            // Vertical: top to bottom
            vstack((
                Gradient::linear(
                    vec![
                        (0.0, color(0.0, 0.5, 1.0)), // Sky blue
                        (1.0, color(0.0, 0.0, 0.5)), // Dark blue
                    ],
                    [0.5, 0.0], // Start: top center
                    [0.5, 1.0], // End: bottom center
                )
                .size(120.0, 80.0),
                "Vertical",
            )),
            // Diagonal
            vstack((
                Gradient::linear(
                    vec![
                        (0.0, color(1.0, 0.0, 1.0)), // Magenta
                        (1.0, color(0.0, 1.0, 1.0)), // Cyan
                    ],
                    [0.0, 0.0], // Start: top-left
                    [1.0, 1.0], // End: bottom-right
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
        text("Radial Gradients").size(20.0),
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
                    [0.5, 0.5], // Center
                    0.0,        // Start radius (point)
                    0.7,        // End radius
                )
                .size(120.0, 120.0),
                "Centered",
            )),
            // Off-center radial
            vstack((
                Gradient::radial(
                    vec![
                        (0.0, color(1.0, 1.0, 1.0)), // White
                        (1.0, color(0.2, 0.4, 1.0)), // Blue
                    ],
                    [0.3, 0.3], // Off-center
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
        text("Static Mesh Gradients").size(20.0),
        "Gradients with per-vertex colors interpolated across a grid",
        hstack((
            // 2x2 mesh (4 corners)
            vstack((
                Gradient::mesh(
                    2,
                    2,
                    vec![
                        ([0.0, 0.0], color(1.0, 0.0, 0.0)), // Top-left: Red
                        ([1.0, 0.0], color(0.0, 1.0, 0.0)), // Top-right: Green
                        ([0.0, 1.0], color(0.0, 0.0, 1.0)), // Bottom-left: Blue
                        ([1.0, 1.0], color(1.0, 1.0, 0.0)), // Bottom-right: Yellow
                    ],
                    true,
                )
                .size(120.0, 120.0),
                "2x2 Corners",
            )),
            // 3x3 mesh with center highlight
            vstack((
                Gradient::mesh(
                    3,
                    3,
                    vec![
                        ([0.0, 0.0], color(0.2, 0.2, 0.4)),
                        ([0.5, 0.0], color(0.3, 0.3, 0.5)),
                        ([1.0, 0.0], color(0.2, 0.2, 0.4)),
                        ([0.0, 0.5], color(0.3, 0.3, 0.5)),
                        ([0.5, 0.5], color(1.0, 1.0, 1.0)), // Bright center
                        ([1.0, 0.5], color(0.3, 0.3, 0.5)),
                        ([0.0, 1.0], color(0.2, 0.2, 0.4)),
                        ([0.5, 1.0], color(0.3, 0.3, 0.5)),
                        ([1.0, 1.0], color(0.2, 0.2, 0.4)),
                    ],
                    true,
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
        text("HDR Gradients").size(20.0),
        "Extended brightness beyond SDR (requires HDR display)",
        hstack((
            // SDR comparison (baseline)
            vstack((
                Gradient::radial(
                    vec![
                        (0.0, color(1.0, 1.0, 1.0)), // SDR white
                        (1.0, color(0.1, 0.1, 0.2)), // Dark edge
                    ],
                    [0.5, 0.5],
                    0.0,
                    0.7,
                )
                .size(120.0, 120.0),
                "SDR White",
            )),
            // HDR bright white (1.5x brightness)
            vstack((
                Gradient::radial(
                    vec![
                        (0.0, hdr_color(1.5, 1.5, 1.5, 0.5)), // HDR bright
                        (1.0, color(0.1, 0.1, 0.2)),          // Dark edge
                    ],
                    [0.5, 0.5],
                    0.0,
                    0.7,
                )
                .size(120.0, 120.0),
                "HDR 1.5x",
            )),
            // HDR super bright (2x brightness)
            vstack((
                Gradient::radial(
                    vec![
                        (0.0, hdr_color(2.0, 2.0, 2.0, 1.0)), // Very bright HDR
                        (1.0, color(0.1, 0.1, 0.2)),          // Dark edge
                    ],
                    [0.5, 0.5],
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
                    vec![
                        (0.0, hdr_color(2.0, 0.3, 0.3, 1.0)), // Bright HDR red
                        (1.0, color(0.3, 0.0, 0.0)),          // Dark red
                    ],
                    [0.0, 0.0],
                    [1.0, 1.0],
                )
                .size(120.0, 80.0),
                "HDR Red",
            )),
            // HDR green highlight
            vstack((
                Gradient::linear(
                    vec![
                        (0.0, hdr_color(0.3, 2.0, 0.3, 1.0)), // Bright HDR green
                        (1.0, color(0.0, 0.3, 0.0)),          // Dark green
                    ],
                    [0.0, 0.0],
                    [1.0, 1.0],
                )
                .size(120.0, 80.0),
                "HDR Green",
            )),
            // HDR blue highlight
            vstack((
                Gradient::linear(
                    vec![
                        (0.0, hdr_color(0.3, 0.3, 2.0, 1.0)), // Bright HDR blue
                        (1.0, color(0.0, 0.0, 0.3)),          // Dark blue
                    ],
                    [0.0, 0.0],
                    [1.0, 1.0],
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

fn main() -> impl View {
    scroll(
        vstack((
            // Header
            text("WaterUI Gradient Examples").size(28.0),
            "GPU-rendered gradients with animation support",
            Divider,
            // Animated mesh gradient background
            animated_background_section(),
            Divider,
            // GPU flowing shader gradient
            gpu_animated_mesh_gradient_section(),
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
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
