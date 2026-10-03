//! Interactive probe: reports the window's live size as it changes, for
//! checking the platform runner's resize plumbing by hand.

use std::thread;
use std::time::Duration;

use hydrolysis::run;
use waterui::Environment;
use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{RoundedRectangle, ShapeExt};
use waterui::window::{Window, WindowState};

/// Bounded-content probe for window size limits: no scroll, so the vstack's
/// intrinsic minimum (two text lines + a 320x120 fixed chip + padding) is the
/// smallest the window may shrink to, and its height is also the maximum the
/// window may grow to — nothing in the tree stretches.
fn main_view() -> impl View {
    vstack((
        text("Window size limits probe").size(24.0),
        text("Shrink and grow the window; it should clamp to content.").size(14.0),
        RoundedRectangle::new(0.2)
            .fill(Color::srgb_hex("#2563EB"))
            .size(320.0, 120.0),
    ))
    .spacing(16.0)
    .padding()
    .background(Color::srgb_hex("#EEF2FF"))
    .foreground(Color::srgb_hex("#0F172A"))
}

fn main() {
    let lifetime = std::env::var("HYDROLYSIS_WAYLAND_SECONDS").map_or_else(
        |_| Duration::from_secs(3600),
        |value| {
            Duration::from_secs(
                value
                    .parse::<u64>()
                    .expect("HYDROLYSIS_WAYLAND_SECONDS must be an unsigned integer"),
            )
        },
    );
    thread::spawn(move || {
        thread::sleep(lifetime);
        std::process::exit(0);
    });

    run(
        App::new_with_windows(
            [Window::new(
                "size-probe",
                binding(WindowState::Normal),
                main_view,
            )],
            Environment::new(),
        ),
        hydrolysis_m3::Material3::defaults(),
    );
}
