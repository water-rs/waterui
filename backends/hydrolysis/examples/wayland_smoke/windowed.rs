//! The example program, compiled where `hydrolysis::run` exists.

use std::thread;
use std::time::Duration;

use hydrolysis::run;
use waterui::Environment;
use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{RoundedRectangle, ShapeExt};
use waterui::window::{Window, WindowState};
use waterui_controls::{slider::slider, stepper::stepper};

fn main_view() -> impl View {
    let toggle_value = binding(true);
    let slider_value = binding(0.4_f64);
    let stepper_value = binding(3_i32);

    scroll(
        vstack((
            text("Hydrolysis Wayland Smoke").size(28.0),
            text("Direct self-drawn window via winit + Cherenkov").size(16.0),
            RoundedRectangle::new(0.2)
                .fill(Color::srgb_hex("#2563EB"))
                .size(560.0, 180.0),
            hstack((
                Toggle::new("Toggle", &toggle_value),
                slider("Slider", &slider_value),
                stepper("Stepper", &stepper_value).range(0..=10),
            ))
            .spacing(16.0),
            text("Scroll to verify wheel routing").size(14.0),
            spacer(),
        ))
        .spacing(20.0)
        .padding(),
    )
    .background(Color::srgb_hex("#EEF2FF"))
    .foreground(Color::srgb_hex("#0F172A"))
}

fn app(env: Environment) -> App {
    App::new_with_windows(
        [Window::new(
            "Hydrolysis Wayland Smoke",
            binding(WindowState::Normal),
            main_view,
        )],
        env,
    )
}

fn smoke_lifetime() -> Duration {
    let seconds = std::env::var("HYDROLYSIS_WAYLAND_SECONDS").map_or(10, |value| {
        value
            .parse::<u64>()
            .expect("HYDROLYSIS_WAYLAND_SECONDS must be an unsigned integer")
    });
    Duration::from_secs(seconds)
}

pub fn main() {
    thread::spawn(|| {
        thread::sleep(smoke_lifetime());
        std::process::exit(0);
    });

    // `run` installs the style's tokens (fonts, colors) over the framework
    // defaults before the first frame; the app-level environment carries none.
    run(
        app(Environment::new()),
        hydrolysis_m3::Material3::defaults(),
    );
}
