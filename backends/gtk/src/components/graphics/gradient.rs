//! GTK `ResolvedGradient` component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_graphics::color::ResolvedColor;
use waterui_graphics::{GradientType, ResolvedGradient, ResolvedGradientStop};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::resolved_color_to_srgba_f64;

const ANGULAR_SEGMENTS: usize = 720;

impl GtkComponent for Native<ResolvedGradient> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let resolved = self.into_inner();
        let mut stops = resolved.stops.clone();
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        assert!(
            !stops.is_empty(),
            "resolved gradient must contain at least one stop"
        );

        let area = gtk4::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);

        area.set_draw_func(move |_area, cr, width, height| {
            let width = f64::from(width);
            let height = f64::from(height);

            match resolved.gradient_type {
                GradientType::Linear => {
                    let gradient = gtk4::cairo::LinearGradient::new(
                        f64::from(resolved.start_point[0]) * width,
                        f64::from(resolved.start_point[1]) * height,
                        f64::from(resolved.end_point[0]) * width,
                        f64::from(resolved.end_point[1]) * height,
                    );
                    for stop in &stops {
                        let (red, green, blue, alpha) = to_rgba(stop.color);
                        gradient.add_color_stop_rgba(
                            f64::from(stop.position),
                            red,
                            green,
                            blue,
                            alpha,
                        );
                    }
                    cr.rectangle(0.0, 0.0, width, height);
                    cr.set_source(&gradient)
                        .expect("failed to bind linear gradient source");
                    cr.fill().expect("failed to draw linear gradient");
                }
                GradientType::Radial => {
                    let cx = f64::from(resolved.start_point[0]) * width;
                    let cy = f64::from(resolved.start_point[1]) * height;
                    let scale = width.min(height);
                    let start_radius = f64::from(resolved.start_value) * scale;
                    let end_radius = f64::from(resolved.end_value) * scale;
                    assert!(end_radius > 0.0, "radial gradient end radius must be > 0");

                    let gradient =
                        gtk4::cairo::RadialGradient::new(cx, cy, start_radius, cx, cy, end_radius);
                    for stop in &stops {
                        let normalized = (end_radius - start_radius)
                            .mul_add(f64::from(stop.position), start_radius)
                            / end_radius;
                        let (red, green, blue, alpha) = to_rgba(stop.color);
                        gradient.add_color_stop_rgba(normalized, red, green, blue, alpha);
                    }
                    cr.rectangle(0.0, 0.0, width, height);
                    cr.set_source(&gradient)
                        .expect("failed to bind radial gradient source");
                    cr.fill().expect("failed to draw radial gradient");
                }
                GradientType::Angular => {
                    draw_angular_gradient(cr, &resolved, stops.as_slice(), width, height);
                }
                GradientType::Mesh => {
                    panic!("ResolvedGradient with mesh type is invalid")
                }
            }
        });

        area.upcast()
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
)]
fn draw_angular_gradient(
    cr: &gtk4::cairo::Context,
    resolved: &ResolvedGradient,
    stops: &[ResolvedGradientStop],
    width: f64,
    height: f64,
) {
    let sweep = f64::from(resolved.end_value - resolved.start_value);
    assert!(sweep > 0.0, "angular gradient sweep must be positive");
    assert!(
        sweep <= core::f64::consts::TAU,
        "angular gradient sweep must be <= TAU"
    );

    let cx = f64::from(resolved.start_point[0]) * width;
    let cy = f64::from(resolved.start_point[1]) * height;
    let radius = width.hypot(height);
    let start = f64::from(resolved.start_value);
    let sweep_fraction = sweep / core::f64::consts::TAU;

    for segment in 0..ANGULAR_SEGMENTS {
        let t0 = segment as f64 / ANGULAR_SEGMENTS as f64;
        let t1 = (segment + 1) as f64 / ANGULAR_SEGMENTS as f64;
        let tm = f64::midpoint(t0, t1);
        let mapped = if tm <= sweep_fraction {
            (tm / sweep_fraction) as f32
        } else {
            1.0
        };

        let color = sample_stop_color(stops, mapped);
        let (red, green, blue, alpha) = to_rgba(color);
        let angle0 = t0.mul_add(core::f64::consts::TAU, start);
        let angle1 = t1.mul_add(core::f64::consts::TAU, start);

        cr.new_sub_path();
        cr.move_to(cx, cy);
        cr.line_to(
            radius.mul_add(angle0.cos(), cx),
            radius.mul_add(angle0.sin(), cy),
        );
        cr.arc(cx, cy, radius, angle0, angle1);
        cr.close_path();
        cr.set_source_rgba(red, green, blue, alpha);
        cr.fill().expect("failed to draw angular gradient segment");
    }
}

fn sample_stop_color(stops: &[ResolvedGradientStop], t: f32) -> ResolvedColor {
    assert!(
        (0.0..=1.0).contains(&t),
        "gradient sampling position must be within [0, 1]"
    );
    if t <= stops[0].position {
        return stops[0].color;
    }

    for window in stops.windows(2) {
        let left = window[0];
        let right = window[1];
        if t <= right.position {
            let span = right.position - left.position;
            assert!(
                span > 0.0,
                "gradient stop positions must be strictly increasing"
            );
            let local_t = (t - left.position) / span;
            return lerp_color(left.color, right.color, local_t);
        }
    }

    stops
        .last()
        .expect("resolved gradient must contain at least one stop")
        .color
}

fn lerp_color(a: ResolvedColor, b: ResolvedColor, t: f32) -> ResolvedColor {
    ResolvedColor {
        red: (b.red - a.red).mul_add(t, a.red),
        green: (b.green - a.green).mul_add(t, a.green),
        blue: (b.blue - a.blue).mul_add(t, a.blue),
        headroom: (b.headroom - a.headroom).mul_add(t, a.headroom),
        opacity: (b.opacity - a.opacity).mul_add(t, a.opacity),
    }
}

fn to_rgba(color: ResolvedColor) -> (f64, f64, f64, f64) {
    resolved_color_to_srgba_f64(color)
}
