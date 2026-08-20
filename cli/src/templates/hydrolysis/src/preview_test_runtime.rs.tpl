//! Hydrolysis preview test runtime for {{ ctx.app_display_name }}.

use std::{fs, io::Write as _, path::PathBuf};

use crate::preview_test;
use waterui_preview_protocol::hydrolysis::{
    PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode,
};
use waterui_testing::ui;

pub(crate) fn run() {
    let config = load_run_config();
    match config.mode {
        PreviewRunMode::Semantic => run_semantic(config.width, config.height),
        PreviewRunMode::Image { .. } | PreviewRunMode::Scenario { .. } => panic!(
            "hydrolysis preview test: render runs require the preview binary (waterui-preview-mode)"
        ),
    }
}

fn load_run_config() -> PreviewRunConfig {
    let path = std::env::var_os(PREVIEW_RUN_CONFIG_ENV).unwrap_or_else(|| {
        panic!("hydrolysis preview test: missing environment variable `{PREVIEW_RUN_CONFIG_ENV}`")
    });
    let raw = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview test: failed to read run config `{}`: {error}",
            PathBuf::from(&path).display()
        )
    });
    serde_json::from_slice(&raw).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview test: failed to parse run config `{}`: {error}",
            PathBuf::from(&path).display()
        )
    })
}

fn run_semantic(width: f32, height: f32) {
    let mut env = waterui::env::Environment::new();
    preview_test::install_preview_theme(&mut env);
    let mut app = ui()
        .environment(env)
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
