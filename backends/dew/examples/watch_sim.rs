//! The Waveshare 2.06" watch panel (410×502), simulated on the desktop.
//!
//! Runs dew's complete embedded rendering flow natively — no
//! cross-compilation — with a reactive per-second counter exercising the
//! rebuild → diff → dirty-band flush economy.
//!
//! Run with: `cargo run -p waterui-dew --example watch_sim --features simulator`

use std::time::Instant;

use nami::binding;
use waterui::prelude::{Color, text, vstack};
use waterui_core::{AnyView, Environment};

fn main() {
    let seconds = binding(0i64);
    let started = Instant::now();

    let seconds_for_root = seconds.clone();
    let mut last = 0;
    waterui_dew::simulator::run(
        410,
        502,
        "dew — ESP32-S3 watch panel simulator",
        Environment::new(),
        move || {
            let seconds = seconds_for_root.clone();
            AnyView::new(vstack((
                text("Dew on watch"),
                Color::cyan(),
                text!("uptime: {seconds} s"),
            )))
        },
        move || {
            let elapsed = started.elapsed().as_secs().cast_signed();
            if elapsed != last {
                last = elapsed;
                seconds.set(elapsed);
            }
        },
    );
}
