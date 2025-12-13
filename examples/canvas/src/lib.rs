use waterui::app::App;
use waterui::graphics::Canvas;
use waterui::graphics::kurbo::{Circle, Line, Point, Rect};
use waterui::graphics::peniko::Color;
use waterui::prelude::*;

fn main() -> impl View {
    vstack((
        text("H₂O Molecule").size(24),
        text("Simple 2D molecular visualization").size(14),
        Canvas::new(|ctx| {
            let size = ctx.size();
            let center = ctx.center();

            // ----------------------------
            // Background
            // ----------------------------
            ctx.fill(
                Rect::from_origin_size(Point::ZERO, size),
                Color::new([0.08, 0.1, 0.14, 1.0]),
            );

            // ----------------------------
            // Molecule geometry
            // ----------------------------
            let oxygen_radius = 40.0;
            let hydrogen_radius = 22.0;
            let bond_length = 90.0;

            // Water bond angle ~104.5°
            let angle = 104.5_f64.to_radians() / 2.0;

            let hx1 = center.x - bond_length * angle.sin();
            let hy1 = center.y + bond_length * angle.cos();

            let hx2 = center.x + bond_length * angle.sin();
            let hy2 = center.y + bond_length * angle.cos();

            let oxygen = center;
            let hydrogen1 = Point::new(hx1, hy1);
            let hydrogen2 = Point::new(hx2, hy2);

            // ----------------------------
            // Bonds
            // ----------------------------
            ctx.stroke(
                Line::new(oxygen, hydrogen1),
                Color::new([0.9, 0.9, 0.9, 0.8]),
                4.0,
            );

            ctx.stroke(
                Line::new(oxygen, hydrogen2),
                Color::new([0.9, 0.9, 0.9, 0.8]),
                4.0,
            );

            // ----------------------------
            // Atoms
            // ----------------------------
            // Oxygen (O)
            ctx.fill(
                Circle::new(oxygen, oxygen_radius),
                Color::new([0.85, 0.2, 0.25, 1.0]),
            );

            // Hydrogens (H)
            ctx.fill(
                Circle::new(hydrogen1, hydrogen_radius),
                Color::new([0.95, 0.95, 0.95, 1.0]),
            );

            ctx.fill(
                Circle::new(hydrogen2, hydrogen_radius),
                Color::new([0.95, 0.95, 0.95, 1.0]),
            );
        }),
        text("Bond angle ≈ 104.5°").size(12),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
