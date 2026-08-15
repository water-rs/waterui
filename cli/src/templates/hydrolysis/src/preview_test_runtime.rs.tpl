//! Hydrolysis preview test runtime for {{ ctx.app_display_name }}.

use std::{fs, io::Write as _, path::PathBuf};

use crate::preview_test;
use waterui_preview_protocol::hydrolysis::{
    PERF_REPORT_LINE_PREFIX, PREVIEW_RUN_CONFIG_ENV, PerfRunConfig, PreviewRunConfig,
    PreviewRunMode,
};
use waterui_testing::{PerfConfig, ui};

pub(crate) fn run() {
    let config = load_run_config();
    match config.mode {
        PreviewRunMode::Semantic => run_semantic(config.width, config.height),
        PreviewRunMode::Perf(ref perf) => run_perf(perf, config.width, config.height),
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

fn run_perf(perf: &PerfRunConfig, width: f32, height: f32) {
    let config = PerfConfig {
        warmups: perf.warmups,
        samples: perf.samples,
        repetitions: perf.repetitions,
    };
    let builder = ui()
        .viewport(dimension_to_u32(width), dimension_to_u32(height))
        .theme(preview_test::install_preview_theme as fn(&mut waterui::env::Environment))
        .perf_config(config);

    let report = match &perf.flamegraph {
        Some(path) => profile_perf(path, perf.flamegraph_frequency, builder),
        None => builder.perf_with(preview_test::load_preview_view, |perf| {
            preview_test::run_perf_automation(perf);
            if perf.measurement_count() == 0 {
                perf.measure("steady-redraw", |run| {
                    run.redraw();
                });
            }
        }),
    };

    let measurements = waterui_testing::protocol::perf_report_to_protocol(&report);
    let json = serde_json::to_string(&measurements).unwrap_or_else(|error| {
        panic!("hydrolysis preview perf: failed to serialize report: {error}")
    });
    write_status(&format!("{PERF_REPORT_LINE_PREFIX}{json}"));
}

fn profile_perf(
    path: &std::path::Path,
    frequency: i32,
    builder: waterui_testing::UiBuilder,
) -> waterui_testing::PerfReport {
    let guard = pprof::ProfilerGuard::new(frequency)
        .unwrap_or_else(|error| panic!("hydrolysis preview perf: failed to start profiler: {error}"));
    let report = builder.perf_with(preview_test::load_preview_view, |perf| {
        preview_test::run_perf_automation(perf);
        if perf.measurement_count() == 0 {
            perf.measure("steady-redraw", |run| {
                run.redraw();
            });
        }
    });
    let profile = guard
        .report()
        .build()
        .unwrap_or_else(|error| panic!("hydrolysis preview perf: failed to build profiler report: {error}"));
    let file = fs::File::create(path).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview perf: failed to create flamegraph `{}`: {error}",
            path.display()
        )
    });
    profile
        .flamegraph(file)
        .unwrap_or_else(|error| panic!("hydrolysis preview perf: failed to write flamegraph: {error}"));
    report
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
