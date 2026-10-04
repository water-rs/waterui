//! Hydrolysis preview test runtime for {{ ctx.app_display_name }}.

use std::io::Write as _;

use crate::preview_test;
use waterui_preview_protocol::hydrolysis::{
    PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode,
};
use waterui_testing::ui;

pub(crate) fn run() {
    let config = crate::run_config::load_run_config::<PreviewRunConfig>(
        PREVIEW_RUN_CONFIG_ENV,
        "preview test",
    );
    match config.mode {
        PreviewRunMode::Semantic => run_semantic(config.width, config.height),
        PreviewRunMode::Image { .. } | PreviewRunMode::Scenario { .. } => panic!(
            "hydrolysis preview test: render runs require the preview binary (waterui-preview-mode)"
        ),
    }
}

fn run_semantic(width: f32, height: f32) {
    // The environment is the application's own composition root: `app(env)`
    // installs the realizations and options the application configures, and
    // the styled mount applies the preview style's tokens exactly as `main`'s
    // `hydrolysis::run` style does — component bodies that read Material
    // tokens resolve them.
    let mut app = ui()
        .environment(preview_test::app_environment())
        .theme(preview_test::preview_style())
        .viewport(dimension_to_u32(width), dimension_to_u32(height))
        .mount(preview_test::load_preview_view);
    preview_test::run_semantic_automation(&mut app);
    write_status("semantic ok");
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
