//! Hydrolysis preview test runtime for {{ ctx.app_display_name }}.

use std::io::Write as _;

use crate::preview_test;
use waterui_preview_protocol::run::{PreviewColorScheme, PreviewRunConfig, PreviewRunMode};

pub(crate) fn run() {
    // `app(env)` runs inside `app_environment` below; the subscriber must be
    // installed first or events the application emits while building its
    // `App` are dropped (water-rs/waterui#2513).
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    hydrolysis::init_tracing();
    let config = PreviewRunConfig::load_from_env()
        .unwrap_or_else(|error| panic!("hydrolysis preview test: {error}"));
    let color_scheme = config.color_scheme().map(|scheme| match scheme {
        PreviewColorScheme::Light => waterui::theme::ColorScheme::Light,
        PreviewColorScheme::Dark => waterui::theme::ColorScheme::Dark,
    });
    match config.mode() {
        PreviewRunMode::Semantic { video } => {
            preview_test::run_preview_test(
                dimension_to_u32(config.width()),
                dimension_to_u32(config.height()),
                f64::from(config.scale()),
                color_scheme,
                video.as_ref(),
            );
            write_status("semantic ok");
        }
        PreviewRunMode::Image { .. } | PreviewRunMode::Scenario { .. } => panic!(
            "hydrolysis preview test: render runs require the preview binary (waterui-preview-mode)"
        ),
    }
}

fn dimension_to_u32(value: f32) -> u32 {
    assert!(
        value.is_finite() && value > 0.0,
        "hydrolysis preview test dimension must be finite and positive"
    );
    value.round() as u32
}

fn write_status(message: &str) {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(message.as_bytes())
        .and_then(|()| stdout.write_all(b"\n"))
        .unwrap_or_else(|error| panic!("hydrolysis preview test: failed to write status: {error}"));
}
