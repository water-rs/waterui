//! Shape Example - Demonstrates WaterUI's Shape system
//!
//! This example showcases:
//! - FilledShape: Shapes filled with color using `.fill()` method
//! - ClipShape: Clipping views to shapes using `.clip()` method
//! - Built-in shapes: Circle, Ellipse, Capsule, Rectangle, RoundedRectangle
//! - Custom paths: Using Path builder for arbitrary shapes
//!
//! Shapes in WaterUI have the same layout behavior as SwiftUI:
//! - They fill all available space (like Color)
//! - Use `.frame()` or container constraints to size them

use core::f32::consts::{FRAC_PI_2, PI};
use waterui::app::App;
use waterui::metadata::secure::ColorSpace;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{
    Capsule, Circle, Ellipse, Path, Rectangle, RoundedRectangle, ShapeExt, UnevenRoundedRectangle,
};
use waterui::widget::condition::when;

/// Demo: Circle shape
fn circle_demo() -> impl View {
    vstack((
        text("Circle").size(18.0),
        "Inscribed in the view bounds",
        hstack((
            // Filled circle
            Circle.fill(Color::srgb_hex("#3B82F6")).size(80.0, 80.0),
            // Clipped image (simulated with color)
            zstack((
                Color::srgb_hex("#10B981"),
                text("Clipped").foreground(Color::srgb(255, 255, 255)),
            ))
            .size(80.0, 80.0)
            .clip(Circle),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Ellipse shape
fn ellipse_demo() -> impl View {
    vstack((
        text("Ellipse").size(18.0),
        "Fills the view bounds as an ellipse",
        hstack((
            // Filled ellipse
            Ellipse.fill(Color::srgb_hex("#8B5CF6")).size(120.0, 60.0),
            // Clipped content
            zstack((
                Color::srgb_hex("#F59E0B"),
                text("Clipped").foreground(Color::srgb(255, 255, 255)),
            ))
            .size(120.0, 60.0)
            .clip(Ellipse),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Capsule shape
fn capsule_demo() -> impl View {
    vstack((
        text("Capsule").size(18.0),
        "Pill shape with fully rounded ends",
        hstack((
            // Horizontal capsule
            Capsule.fill(Color::srgb_hex("#EC4899")).size(120.0, 50.0),
            // Vertical capsule
            Capsule.fill(Color::srgb_hex("#06B6D4")).size(50.0, 100.0),
        ))
        .spacing(16.0),
        // Capsule-clipped button-like view
        zstack((
            Capsule.fill(Color::srgb_hex("#3B82F6")),
            text("Capsule Button")
                .foreground(Color::srgb(255, 255, 255))
                .padding(),
        ))
        .size(150.0, 44.0),
    ))
    .padding()
}

/// Demo: Rectangle shapes
fn rectangle_demo() -> impl View {
    vstack((
        text("Rectangle").size(18.0),
        "Sharp corners",
        hstack((
            Rectangle.fill(Color::srgb_hex("#EF4444")).size(80.0, 60.0),
            zstack((
                Color::srgb_hex("#84CC16"),
                text("Rect").foreground(Color::srgb(255, 255, 255)),
            ))
            .size(80.0, 60.0)
            .clip(Rectangle),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Rounded rectangle
fn rounded_rectangle_demo() -> impl View {
    vstack((
        text("Rounded Rectangle").size(18.0),
        "Uniform corner radius (0.0-0.5)",
        hstack((
            // Small radius
            RoundedRectangle::new(0.1)
                .fill(Color::srgb_hex("#14B8A6"))
                .size(80.0, 60.0),
            // Medium radius
            RoundedRectangle::new(0.2)
                .fill(Color::srgb_hex("#F97316"))
                .size(80.0, 60.0),
            // Large radius (approaches capsule)
            RoundedRectangle::new(0.4)
                .fill(Color::srgb_hex("#A855F7"))
                .size(80.0, 60.0),
        ))
        .spacing(12.0),
    ))
    .padding()
}

/// Demo: Uneven rounded rectangle
fn uneven_rounded_rectangle_demo() -> impl View {
    vstack((
        text("Uneven Rounded Rectangle").size(18.0),
        "Independent corner radii",
        hstack((
            // Top rounded only
            UnevenRoundedRectangle::new(0.3, 0.3, 0.0, 0.0)
                .fill(Color::srgb_hex("#0EA5E9"))
                .size(80.0, 60.0),
            // Diagonal corners
            UnevenRoundedRectangle::new(0.3, 0.0, 0.3, 0.0)
                .fill(Color::srgb_hex("#F43F5E"))
                .size(80.0, 60.0),
            // Bottom rounded only
            UnevenRoundedRectangle::new(0.0, 0.0, 0.3, 0.3)
                .fill(Color::srgb_hex("#22C55E"))
                .size(80.0, 60.0),
        ))
        .spacing(12.0),
    ))
    .padding()
}

/// Demo: Custom path shapes
fn custom_path_demo() -> impl View {
    // Triangle
    let triangle = Path::new()
        .move_to(0.5, 0.0)
        .line_to(1.0, 1.0)
        .line_to(0.0, 1.0)
        .close();

    // Star (5-pointed)
    let star = create_star(5, 0.4);

    // Heart
    let heart = Path::new()
        .move_to(0.5, 0.2)
        .cubic_to(0.5, 0.0, 0.0, 0.0, 0.0, 0.3)
        .cubic_to(0.0, 0.6, 0.5, 0.8, 0.5, 1.0)
        .cubic_to(0.5, 0.8, 1.0, 0.6, 1.0, 0.3)
        .cubic_to(1.0, 0.0, 0.5, 0.0, 0.5, 0.2)
        .close();

    // Arrow
    let arrow = Path::new()
        .move_to(0.5, 0.0)
        .line_to(1.0, 0.4)
        .line_to(0.7, 0.4)
        .line_to(0.7, 1.0)
        .line_to(0.3, 1.0)
        .line_to(0.3, 0.4)
        .line_to(0.0, 0.4)
        .close();

    vstack((
        text("Custom Paths").size(18.0),
        "Build arbitrary shapes with Path",
        hstack((
            triangle.fill(Color::srgb_hex("#FBBF24")).size(70.0, 70.0),
            star.fill(Color::srgb_hex("#F472B6")).size(70.0, 70.0),
            heart.fill(Color::srgb_hex("#EF4444")).size(70.0, 70.0),
            arrow.fill(Color::srgb_hex("#6366F1")).size(70.0, 70.0),
        ))
        .spacing(12.0),
    ))
    .padding()
}

/// Demo: HDR shapes
fn hdr_shapes() -> impl View {
    let sdr = Color::srgb_f32(0.9, 0.2, 0.35);
    let hdr = Color::srgb_f32(0.9, 0.2, 0.35).with_headroom(1.5);
    let hdr_blue = Color::srgb_f32(0.2, 0.6, 1.0).with_headroom(1.0);

    hstack((
        zstack((
            RoundedRectangle::new(0.18).fill(sdr),
            text("SDR").foreground(Color::srgb(255, 255, 255)),
        ))
        .size(100.0, 60.0),
        zstack((
            RoundedRectangle::new(0.18).fill(hdr),
            text("HDR").foreground(Color::srgb(255, 255, 255)),
        ))
        .size(100.0, 60.0),
        Circle.fill(hdr_blue).size(60.0, 60.0),
    ))
    .spacing(12.0)
}

/// Demo: HDR shapes
fn hdr_shape_demo(show_hdr: &Binding<bool>) -> impl View {
    let show_hdr = show_hdr.clone();
    vstack((
        text("HDR Shapes").size(18.0),
        "Extended range colors via headroom",
        Toggle::new(&show_hdr).label("Show HDR"),
        when(show_hdr.clone(), || {
            hdr_shapes().color_space(ColorSpace::Hdr)
        })
        .otherwise(|| hdr_shapes().color_space(ColorSpace::Sdr)),
    ))
    .padding()
}

/// Helper: Create a star path
fn create_star(points: usize, inner_ratio: f32) -> Path {
    let mut path = Path::new();
    let outer_radius = 0.5;
    let inner_radius = outer_radius * inner_ratio;
    let center = 0.5;

    for i in 0..(points * 2) {
        let angle = (i as f32) * PI / (points as f32) - FRAC_PI_2;
        let radius = if i % 2 == 0 {
            outer_radius
        } else {
            inner_radius
        };
        let x = center + radius * angle.cos();
        let y = center + radius * angle.sin();

        if i == 0 {
            path = path.move_to(x, y);
        } else {
            path = path.line_to(x, y);
        }
    }
    path.close()
}

/// Demo: Clipping showcase
fn clip_showcase() -> impl View {
    vstack((
        text("Clipping Views").size(18.0),
        "Apply shapes as masks to any view",
        hstack((
            // Circle clipped gradient
            zstack((
                Color::srgb_hex("#818CF8"),
                vstack((
                    text("Avatar").foreground(Color::srgb(255, 255, 255)),
                    text("Image").foreground(Color::srgb(200, 200, 200)),
                )),
            ))
            .size(80.0, 80.0)
            .clip(Circle),
            // Rounded rect clipped card
            zstack((
                Color::srgb_hex("#1E293B"),
                vstack((
                    text("Card").foreground(Color::srgb(255, 255, 255)),
                    text("Content").foreground(Color::srgb(156, 163, 175)),
                )),
            ))
            .size(100.0, 80.0)
            .clip(RoundedRectangle::new(0.15)),
            // Custom shape clip
            {
                let hexagon = create_hexagon();
                zstack((
                    Color::srgb_hex("#059669"),
                    text("Hex").foreground(Color::srgb(255, 255, 255)),
                ))
                .size(80.0, 80.0)
                .clip(hexagon)
            },
        ))
        .spacing(12.0),
    ))
    .padding()
}

/// Helper: Create a hexagon path
fn create_hexagon() -> Path {
    let mut path = Path::new();
    let center = 0.5;
    let radius = 0.5;

    for i in 0..6 {
        let angle = (i as f32) * PI / 3.0 - FRAC_PI_2;
        let x = center + radius * angle.cos();
        let y = center + radius * angle.sin();

        if i == 0 {
            path = path.move_to(x, y);
        } else {
            path = path.line_to(x, y);
        }
    }
    path.close()
}

/// Demo: Shape layout behavior
fn layout_demo() -> impl View {
    vstack((
        text("Layout Behavior").size(18.0),
        "Shapes fill available space like Color",
        hstack((
            // Shape fills container
            zstack((
                Color::srgb_hex("#E5E7EB"),
                Circle.fill(Color::srgb_hex("#3B82F6")),
            ))
            .size(100.0, 100.0),
            // Shape in flexible layout
            vstack((
                text("Above").foreground(Color::srgb(107, 114, 128)),
                RoundedRectangle::new(0.2).fill(Color::srgb_hex("#10B981")),
                text("Below").foreground(Color::srgb(107, 114, 128)),
            ))
            .size(100.0, 120.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

/// Demo: Morphing animation between built-in shapes.
fn morph_demo() -> impl View {
    vstack((
        text("Morph Animation").size(18.0),
        "SDF-based shape morphing for built-in shapes",
        hstack((
            Circle
                .morph_to(RoundedRectangle::new(0.22), Color::srgb_hex("#3B82F6"))
                .size(90.0, 90.0),
            RoundedRectangle::new(0.08)
                .morph_to(Capsule, Color::srgb_hex("#10B981"))
                .duration(core::time::Duration::from_millis(1200))
                .size(120.0, 70.0),
            Rectangle
                .morph_to(Ellipse, Color::srgb_hex("#F97316"))
                .duration(core::time::Duration::from_millis(800))
                .autoreverse(true)
                .size(110.0, 70.0),
        ))
        .spacing(12.0),
    ))
    .padding()
}

fn main() -> impl View {
    let show_hdr = binding(true);

    //panic!("Shape example app requires WaterUI runtime.");
    scroll(
        vstack((
            // Header
            text("WaterUI Shape Examples").size(28.0),
            "Shapes and clipping demonstrations",
            Divider,
            // Basic shapes group
            vstack((
                circle_demo(),
                Divider,
                ellipse_demo(),
                Divider,
                capsule_demo(),
                Divider,
                rectangle_demo(),
            )),
            // Advanced shapes group
            vstack((
                Divider,
                rounded_rectangle_demo(),
                Divider,
                uneven_rounded_rectangle_demo(),
                Divider,
                custom_path_demo(),
                Divider,
                hdr_shape_demo(&show_hdr),
            )),
            // Showcase group
            vstack((
                Divider,
                clip_showcase(),
                Divider,
                morph_demo(),
                Divider,
                layout_demo(),
                spacer().min_height(32.0),
            )),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
