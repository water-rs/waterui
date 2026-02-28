use std::thread;
use std::time::Duration;

use hydrolysis::run;
use waterui::Environment;
use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{RoundedRectangle, ShapeExt};

fn main_view() -> impl View {
    let toggle_value = binding(true);
    let slider_value = binding(0.4_f64);
    let stepper_value = binding(3_i32);

    scroll(vstack((
        text("Hydrolysis Wayland Smoke").size(28.0),
        text("Direct self-drawn window via winit + Vello").size(16.0),
        RoundedRectangle::new(0.2)
            .fill(Color::srgb_hex("#2563EB"))
            .size(560.0, 180.0),
        hstack((
            Toggle::new(&toggle_value).label("Toggle"),
            Slider::new(0.0..=1.0, &slider_value).label("Slider"),
            Stepper::new(&stepper_value).range(0..=10).label("Stepper"),
        ))
        .spacing(16.0),
        text("Scroll to verify wheel routing").size(14.0),
        spacer(),
    ))
    .spacing(20.0)
    .padding())
}

fn app() -> App {
    let env = Environment::new();
    App::new(main_view, env).title("Hydrolysis Wayland Smoke")
}

fn main() {
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(4));
        std::process::exit(0);
    });

    run(app());
}
