//! Gradient Example - Demonstrates WaterUI's gradient system
//!
//! This example showcases GPU-rendered gradients:
//! - Linear gradients with configurable direction
//! - Radial gradients from center outward
//! - Angular (conic) gradients
//! - Mesh gradients with per-vertex colors
//!
//! Gradients are rendered using GpuSurface for consistent cross-platform
//! appearance and high performance.

use waterui::app::App;
use waterui::prelude::*;
use waterui_color::ResolvedColor;
use waterui_graphics::GradientView;

/// Helper to create a resolved color
fn color(r: f32, g: f32, b: f32) -> ResolvedColor {
    ResolvedColor {
        red: r,
        green: g,
        blue: b,
        opacity: 1.0,
        headroom: 0.0,
    }
}

/// Demo: Linear gradient - horizontal, vertical, and diagonal
fn linear_gradient_section() -> impl View {
    vstack((
        text("Linear Gradients").size(20.0),
        "Gradients along a line from start to end point",
        hstack((
            // Horizontal: left to right
            vstack((
                GradientView::linear(
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
                GradientView::linear(
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
                GradientView::linear(
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
                GradientView::radial(
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
                GradientView::radial(
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
            // Ring gradient
            vstack((
                GradientView::radial(
                    vec![
                        (0.0, color(0.1, 0.1, 0.1)), // Dark center
                        (0.4, color(0.1, 0.1, 0.1)), // Still dark
                        (0.5, color(0.0, 1.0, 0.5)), // Bright ring
                        (0.6, color(0.1, 0.1, 0.1)), // Dark again
                        (1.0, color(0.1, 0.1, 0.1)), // Dark edge
                    ],
                    [0.5, 0.5],
                    0.0,
                    0.7,
                )
                .size(120.0, 120.0),
                "Ring",
            )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Angular (conic) gradient - rotating around center
fn angular_gradient_section() -> impl View {
    use core::f32::consts::PI;

    vstack((
        text("Angular (Conic) Gradients").size(20.0),
        "Gradients rotating around a center point",
        hstack((
            // Rainbow wheel
            vstack((
                GradientView::angular(
                    vec![
                        (0.0, color(1.0, 0.0, 0.0)),  // Red
                        (0.17, color(1.0, 0.5, 0.0)), // Orange
                        (0.33, color(1.0, 1.0, 0.0)), // Yellow
                        (0.5, color(0.0, 1.0, 0.0)),  // Green
                        (0.67, color(0.0, 0.5, 1.0)), // Blue
                        (0.83, color(0.5, 0.0, 1.0)), // Purple
                        (1.0, color(1.0, 0.0, 0.0)),  // Back to red
                    ],
                    [0.5, 0.5],   // Center
                    -PI,         // Start angle
                    PI,          // End angle
                )
                .size(120.0, 120.0),
                "Rainbow",
            )),
            // Pie chart style
            vstack((
                GradientView::angular(
                    vec![
                        (0.0, color(0.2, 0.6, 1.0)),  // Blue
                        (0.25, color(0.2, 0.6, 1.0)), // Blue
                        (0.25, color(0.0, 0.8, 0.4)), // Green
                        (0.5, color(0.0, 0.8, 0.4)),  // Green
                        (0.5, color(1.0, 0.6, 0.0)),  // Orange
                        (0.75, color(1.0, 0.6, 0.0)), // Orange
                        (0.75, color(0.8, 0.2, 0.2)), // Red
                        (1.0, color(0.8, 0.2, 0.2)),  // Red
                    ],
                    [0.5, 0.5],
                    -PI,
                    PI,
                )
                .size(120.0, 120.0),
                "Pie Chart",
            )),
            // Sweep
            vstack((
                GradientView::angular(
                    vec![
                        (0.0, color(0.0, 0.0, 0.0)), // Black
                        (1.0, color(1.0, 1.0, 1.0)), // White
                    ],
                    [0.5, 0.5],
                    0.0,
                    2.0 * PI,
                )
                .size(120.0, 120.0),
                "Sweep",
            )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Mesh gradient - bilinear interpolation between vertex colors
fn mesh_gradient_section() -> impl View {
    vstack((
        text("Mesh Gradients").size(20.0),
        "Gradients with per-vertex colors interpolated across a grid",
        hstack((
            // 2x2 mesh (4 corners)
            vstack((
                GradientView::mesh(
                    2,
                    2,
                    vec![
                        // Row 0 (top): left, right
                        ([0.0, 0.0], color(1.0, 0.0, 0.0)), // Top-left: Red
                        ([1.0, 0.0], color(0.0, 1.0, 0.0)), // Top-right: Green
                        // Row 1 (bottom): left, right
                        ([0.0, 1.0], color(0.0, 0.0, 1.0)), // Bottom-left: Blue
                        ([1.0, 1.0], color(1.0, 1.0, 0.0)), // Bottom-right: Yellow
                    ],
                    true, // Smooth colors
                )
                .size(120.0, 120.0),
                "2x2 Corners",
            )),
            // 3x3 mesh with center highlight
            vstack((
                GradientView::mesh(
                    3,
                    3,
                    vec![
                        // Row 0 (top)
                        ([0.0, 0.0], color(0.2, 0.2, 0.4)),
                        ([0.5, 0.0], color(0.3, 0.3, 0.5)),
                        ([1.0, 0.0], color(0.2, 0.2, 0.4)),
                        // Row 1 (middle)
                        ([0.0, 0.5], color(0.3, 0.3, 0.5)),
                        ([0.5, 0.5], color(1.0, 1.0, 1.0)), // Bright center
                        ([1.0, 0.5], color(0.3, 0.3, 0.5)),
                        // Row 2 (bottom)
                        ([0.0, 1.0], color(0.2, 0.2, 0.4)),
                        ([0.5, 1.0], color(0.3, 0.3, 0.5)),
                        ([1.0, 1.0], color(0.2, 0.2, 0.4)),
                    ],
                    true,
                )
                .size(120.0, 120.0),
                "3x3 Highlight",
            )),
            // Warm to cool gradient
            vstack((
                GradientView::mesh(
                    3,
                    2,
                    vec![
                        // Row 0 (top): warm colors
                        ([0.0, 0.0], color(1.0, 0.3, 0.0)), // Orange
                        ([0.5, 0.0], color(1.0, 0.6, 0.0)), // Yellow-orange
                        ([1.0, 0.0], color(1.0, 0.8, 0.2)), // Light yellow
                        // Row 1 (bottom): cool colors
                        ([0.0, 1.0], color(0.0, 0.3, 0.8)), // Blue
                        ([0.5, 1.0], color(0.2, 0.5, 0.8)), // Lighter blue
                        ([1.0, 1.0], color(0.4, 0.7, 1.0)), // Sky blue
                    ],
                    true,
                )
                .size(120.0, 80.0),
                "Warm to Cool",
            )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Using gradients as backgrounds
fn gradient_background_section() -> impl View {
    vstack((
        text("Gradient Backgrounds").size(20.0),
        "Using .background() with gradient views",
        hstack((
            text("Linear BG")
                .size(16.0)
                .foreground(Color::srgb(255, 255, 255))
                .padding()
                .background(GradientView::linear(
                    vec![
                        (0.0, color(0.2, 0.4, 0.8)),
                        (1.0, color(0.1, 0.2, 0.4)),
                    ],
                    [0.0, 0.0],
                    [1.0, 1.0],
                )),
            text("Radial BG")
                .size(16.0)
                .foreground(Color::srgb(255, 255, 255))
                .padding()
                .background(GradientView::radial(
                    vec![
                        (0.0, color(0.8, 0.2, 0.4)),
                        (1.0, color(0.3, 0.1, 0.2)),
                    ],
                    [0.5, 0.5],
                    0.0,
                    0.8,
                )),
            text("Button")
                .size(16.0)
                .foreground(Color::srgb(255, 255, 255))
                .padding()
                .background(GradientView::linear(
                    vec![
                        (0.0, color(0.0, 0.7, 0.4)),
                        (1.0, color(0.0, 0.5, 0.3)),
                    ],
                    [0.5, 0.0],
                    [0.5, 1.0],
                )),
        ))
        .spacing(16.0),
    ))
    .padding()
}

#[cfg_attr(debug_assertions, hot_reload)]
fn main() -> impl View {
    scroll(
        vstack((
            // Header
            text("WaterUI Gradient Examples").size(28.0),
            "GPU-rendered gradients using GpuSurface",
            Divider,
            // Linear gradients
            linear_gradient_section(),
            Divider,
            // Radial gradients
            radial_gradient_section(),
            Divider,
            // Angular gradients
            angular_gradient_section(),
            Divider,
            // Mesh gradients
            mesh_gradient_section(),
            Divider,
            // Gradient backgrounds
            gradient_background_section(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
