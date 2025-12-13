use waterui::app::App;
use waterui::color::Srgb;
use waterui::graphics::Canvas;
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
            ctx.set_fill_style(Srgb::new(0.08, 0.1, 0.14));
            ctx.fill_rect(Rect::new(Point::zero(), size));

            // ----------------------------
            // Molecule geometry
            // ----------------------------
            let oxygen_radius = 40.0;
            let hydrogen_radius = 22.0;
            let bond_length = 90.0;

            // Water bond angle ~104.5°
            let angle = 104.5_f32.to_radians() / 2.0;

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
            let bond_color =
                waterui::color::ResolvedColor::from(Srgb::new(0.9, 0.9, 0.9)).with_opacity(0.8);
            ctx.set_stroke_style(bond_color);
            ctx.set_line_width(4.0);
            ctx.stroke_line(oxygen, hydrogen1);
            ctx.stroke_line(oxygen, hydrogen2);

            // ----------------------------
            // Atoms
            // ----------------------------
            // Oxygen (O)
            ctx.set_fill_style(Srgb::new(0.85, 0.2, 0.25));
            ctx.fill_circle(oxygen, oxygen_radius);

            // Hydrogens (H)
            ctx.set_fill_style(Srgb::new(0.95, 0.95, 0.95));
            ctx.fill_circle(hydrogen1, hydrogen_radius);
            ctx.fill_circle(hydrogen2, hydrogen_radius);
        }),
        text("Bond angle ≈ 104.5°").size(12),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
