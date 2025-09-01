//! Comprehensive example showing all registered components using WaterUI layout system.

use gtk4::prelude::*;
use waterui::{Binding, Environment, Str, component::*};
use waterui_gtk4::renderer::render;

fn main() {
    // Create the application
    let app = gtk4::Application::new(
        Some("com.example.waterui-all-components"),
        gtk4::gio::ApplicationFlags::empty(),
    );

    app.connect_activate(|app| {
        // Create the main window
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("WaterUI - All Components"));
        window.set_default_size(600, 500);

        // Environment for rendering
        let env = Environment::new();

        // Create bindings for form components
        let text_value = Binding::container(Str::from("Initial text"));
        let toggle_value = Binding::bool(false);
        let slider_value = Binding::container(50.0);
        let stepper_value = Binding::int(5);

        // Create fancy static canvas with complex graphics
        let static_canvas = canvas::CanvasContent::new()
            // Background gradient effect using overlapping shapes
            .circle(
                150.0,
                80.0,
                60.0,
                vello::peniko::Color::rgba(0.2, 0.4, 0.8, 0.3),
            )
            .circle(
                180.0,
                100.0,
                45.0,
                vello::peniko::Color::rgba(0.8, 0.2, 0.6, 0.3),
            )
            .circle(
                120.0,
                110.0,
                35.0,
                vello::peniko::Color::rgba(0.6, 0.8, 0.2, 0.3),
            )
            // Geometric pattern
            .rect(
                20.0,
                20.0,
                40.0,
                40.0,
                vello::peniko::Color::rgb(0.9, 0.1, 0.1),
            )
            .rect(
                30.0,
                30.0,
                40.0,
                40.0,
                vello::peniko::Color::rgba(0.1, 0.9, 0.1, 0.7),
            )
            .rect(
                40.0,
                40.0,
                40.0,
                40.0,
                vello::peniko::Color::rgba(0.1, 0.1, 0.9, 0.5),
            )
            // Complex star pattern (6 parameters needed for line: x1, y1, x2, y2, color, width)
            .line(
                250.0,
                30.0,
                280.0,
                60.0,
                vello::peniko::Color::rgb(1.0, 0.8, 0.0),
                2.0,
            )
            .line(
                280.0,
                30.0,
                250.0,
                60.0,
                vello::peniko::Color::rgb(1.0, 0.8, 0.0),
                2.0,
            )
            .line(
                265.0,
                20.0,
                265.0,
                70.0,
                vello::peniko::Color::rgb(1.0, 0.8, 0.0),
                2.0,
            )
            .line(
                235.0,
                45.0,
                295.0,
                45.0,
                vello::peniko::Color::rgb(1.0, 0.8, 0.0),
                2.0,
            )
            // Decorative border elements
            .circle(10.0, 10.0, 8.0, vello::peniko::Color::rgb(0.8, 0.0, 0.8))
            .circle(340.0, 10.0, 8.0, vello::peniko::Color::rgb(0.8, 0.0, 0.8))
            .circle(10.0, 140.0, 8.0, vello::peniko::Color::rgb(0.8, 0.0, 0.8))
            .circle(340.0, 140.0, 8.0, vello::peniko::Color::rgb(0.8, 0.0, 0.8));

        // Create dynamic interactive canvas with animation-like effects
        let dynamic_canvas = canvas::canvas_with_context(400.0, 200.0, |ctx| {
            use vello::kurbo::Point;

            // Animated-looking wave pattern
            for i in 0..20 {
                let x = i as f64 * 20.0;
                let y1 = 100.0 + (i as f64 * 0.5).sin() * 30.0;
                let y2 = 100.0 + ((i as f64 * 0.5) + 1.0).sin() * 30.0;

                let color = vello::peniko::Color::rgb(
                    0.5 + (i as f64 * 0.3).cos() * 0.5,
                    0.3 + (i as f64 * 0.2).sin() * 0.3,
                    0.7 + (i as f64 * 0.4).cos() * 0.3,
                );

                ctx.stroke_line(Point::new(x, y1), Point::new(x + 20.0, y2), color, 2.0);
            }

            // Spiral pattern
            for i in 0..50 {
                let angle = i as f64 * 0.3;
                let radius = i as f64 * 2.0;
                let x = 200.0 + angle.cos() * radius;
                let y = 100.0 + angle.sin() * radius;

                let color = vello::peniko::Color::rgb(
                    0.8 - (i as f64 / 50.0) * 0.3,
                    0.2 + (i as f64 / 50.0) * 0.6,
                    0.9 - (i as f64 / 50.0) * 0.4,
                );

                ctx.fill_circle(Point::new(x, y), 3.0, color);
            }

            // Complex geometric mandala-like pattern
            ctx.with_save(|ctx| {
                ctx.translate(100.0, 100.0);

                for layer in 0..5 {
                    let radius = 20.0 + layer as f64 * 15.0;
                    let count = 6 + layer * 2;

                    for i in 0..count {
                        let angle = (i as f64 / count as f64) * 2.0 * std::f64::consts::PI;
                        let x = angle.cos() * radius;
                        let y = angle.sin() * radius;

                        let color = vello::peniko::Color::rgba(
                            0.9 - layer as f64 * 0.1,
                            0.3 + layer as f64 * 0.15,
                            0.6 + layer as f64 * 0.1,
                            0.8 - layer as f64 * 0.1,
                        );

                        ctx.fill_rect(
                            vello::kurbo::Rect::new(x - 3.0, y - 3.0, x + 3.0, y + 3.0),
                            color,
                        );
                    }
                }
            });

            // Gradient-like effect using multiple overlapping circles
            for i in 0..15 {
                let alpha = 0.1 - (i as f64 / 15.0) * 0.08;
                let size = 5.0 + i as f64 * 3.0;
                ctx.fill_circle(
                    Point::new(320.0, 50.0),
                    size,
                    vello::peniko::Color::rgba(1.0, 0.5, 0.0, alpha),
                );
            }
        });

        // Create sections to reduce tuple size
        let header_section = layout::stack::VStack::new((
            text("Welcome to WaterUI Components Demo"),
            divder::Divider,
        ));

        let progress_section = layout::stack::VStack::new((
            text("Progress Components:"),
            progress(0.75),
            loading(),
            divder::Divider,
        ));

        let form_section = layout::stack::VStack::new((
            text("Form Components:"),
            form::field("Text Field:", &text_value),
            form::toggle("Toggle Switch", &toggle_value),
            form::Slider::new(0.0..=100.0, &slider_value),
            form::stepper(&stepper_value),
        ));

        let canvas_section = layout::stack::VStack::new((
            text("Canvas Components:"),
            text("Static Canvas with Geometric Patterns:"),
            canvas::canvas(static_canvas, 350.0, 150.0),
            text("Dynamic Canvas with Complex Graphics:"),
            dynamic_canvas,
            divder::Divider,
        ));

        // Use WaterUI's layout system with VStack
        let content = layout::stack::VStack::new((
            header_section,
            progress_section,
            form_section,
            canvas_section,
            text("All components registered successfully!"),
        ));

        // Render the entire layout using WaterUI
        let widget = render(content, &env);

        // Add scrolling support
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&widget));

        window.set_child(Some(&scrolled));
        window.present();

        println!("All WaterUI components displayed using native WaterUI layout!");
    });

    app.run();
}
