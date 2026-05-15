//! Hydrolysis preview test runtime for {{ ctx.app_display_name }}.

use std::{env, fs::File, io::Write as _, path::PathBuf, time::Duration};

use crate::preview_test;
use waterui_testing::{PerfConfig, ui};

pub(crate) fn run() {
    let width = parse_dimension(preview_test::PREVIEW_WIDTH_ENV);
    let height = parse_dimension(preview_test::PREVIEW_HEIGHT_ENV);
    match preview_test::PREVIEW_TEST_MODE {
        "semantic" => run_semantic(width, height),
        "perf" => run_perf(width, height),
        other => panic!("hydrolysis preview test: unsupported mode `{other}`"),
    }
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

fn run_perf(width: f32, height: f32) {
    let config = PerfConfig {
        warmups: parse_optional_u32(preview_test::PERF_WARMUPS_ENV).unwrap_or(10),
        samples: parse_optional_u32(preview_test::PERF_SAMPLES_ENV).unwrap_or(120),
        repetitions: parse_optional_u32(preview_test::PERF_REPETITIONS_ENV).unwrap_or(7),
    };
    let builder = ui()
        .viewport(dimension_to_u32(width), dimension_to_u32(height))
        .theme(preview_test::install_preview_theme as fn(&mut waterui::env::Environment))
        .perf_config(config);

    let report = match env::var_os(preview_test::FLAMEGRAPH_ENV) {
        Some(path) => profile_perf(PathBuf::from(path), builder),
        None => builder.perf_with(preview_test::load_preview_view, |perf| {
            preview_test::run_perf_automation(perf);
            if perf.measurement_count() == 0 {
                perf.measure("steady", |_| {});
            }
        }),
    };

    for measurement in report.measurements() {
        let stats = measurement.stats();
        write_status(&format!(
            "perf {} samples={} mean={}us median={}us p95={}us min={}us max={}us rebuilt={}",
            measurement.name,
            stats.samples,
            duration_micros(stats.mean),
            duration_micros(stats.median),
            duration_micros(stats.p95),
            duration_micros(stats.min),
            duration_micros(stats.max),
            stats.rebuilt_frames
        ));
        write_status(&format!(
            "perf-phases {} rebuild_mean={}us rebuild_p95={}us render_mean={}us render_p95={}us animation_mean={}us input_mean={}us missed_120fps={} missed_60fps={} measurement_cache_hits={} measurement_cache_misses={} scene_layers={} vello_scene_layers={} gpu_surface_layers={} clip_layers={} max_clip_depth={}",
            measurement.name,
            duration_micros(stats.phases.rebuild.mean),
            duration_micros(stats.phases.rebuild.p95),
            duration_micros(stats.phases.render.mean),
            duration_micros(stats.phases.render.p95),
            duration_micros(stats.phases.animation.mean),
            duration_micros(stats.phases.input.mean),
            stats.missed_120fps_frames,
            stats.missed_60fps_frames,
            stats.measurement_cache_hits,
            stats.measurement_cache_misses,
            stats.scene_layers,
            stats.vello_scene_layers,
            stats.gpu_surface_layers,
            stats.clip_layers,
            stats.max_clip_depth
        ));
        for (index, frame) in measurement.frames.iter().enumerate() {
            write_status(&format!(
                "perf-sample {} index={} total={}us rebuild={}us render={}us acquire={}us present={}us animation={}us input={}us executor_before={}us executor_after={}us rebuilt={} rendered={} captured_snapshot={} cpu_percent_milli={} memory_bytes={} gpu_frame={}us measurement_cache_hits={} measurement_cache_misses={} scene_layers={} vello_scene_layers={} gpu_surface_layers={} clip_layers={} max_clip_depth={}",
                measurement.name,
                index,
                duration_micros(frame.total),
                duration_micros(frame.profile.phases.rebuild),
                duration_micros(frame.profile.phases.render),
                duration_micros(frame.profile.phases.acquire),
                duration_micros(frame.profile.phases.present),
                duration_micros(frame.profile.phases.animation),
                duration_micros(frame.profile.phases.input),
                duration_micros(frame.profile.phases.executor_before),
                duration_micros(frame.profile.phases.executor_after),
                u8::from(frame.rebuilt),
                u8::from(frame.profile.counters.rendered),
                u8::from(frame.profile.counters.captured_snapshot),
                cpu_percent_milli(frame.resources.cpu_percent),
                frame.resources.memory_bytes,
                duration_micros(
                    frame.profile.phases.acquire + frame.profile.phases.render + frame.profile.phases.present
                ),
                frame.profile.counters.measurement_cache_hits,
                frame.profile.counters.measurement_cache_misses,
                frame.profile.counters.scene_layers,
                frame.profile.counters.vello_scene_layers,
                frame.profile.counters.gpu_surface_layers,
                frame.profile.counters.clip_layers,
                frame.profile.counters.max_clip_depth
            ));
        }
    }
}

fn profile_perf(
    path: PathBuf,
    builder: waterui_testing::UiBuilder<fn(&mut waterui::env::Environment)>,
) -> waterui_testing::PerfReport {
    let frequency = parse_optional_i32(preview_test::FLAMEGRAPH_FREQUENCY_ENV).unwrap_or(100);
    let guard = pprof::ProfilerGuard::new(frequency)
        .unwrap_or_else(|error| panic!("hydrolysis preview perf: failed to start profiler: {error}"));
    let report = builder.perf_with(preview_test::load_preview_view, |perf| {
        preview_test::run_perf_automation(perf);
        if perf.measurement_count() == 0 {
            perf.measure("steady", |_| {});
        }
    });
    let profile = guard
        .report()
        .build()
        .unwrap_or_else(|error| panic!("hydrolysis preview perf: failed to build profiler report: {error}"));
    let file = File::create(&path).unwrap_or_else(|error| {
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

fn required_env(name: &str) -> String {
    env::var(name)
        .unwrap_or_else(|error| panic!("hydrolysis preview test: missing environment variable `{name}`: {error}"))
}

fn parse_dimension(name: &str) -> f32 {
    let raw = required_env(name);
    raw.parse::<f32>()
        .unwrap_or_else(|error| panic!("hydrolysis preview test: invalid `{name}` value `{raw}`: {error}"))
}

fn parse_optional_u32(name: &str) -> Option<u32> {
    let raw = env::var(name).ok()?;
    Some(raw.parse::<u32>().unwrap_or_else(|error| {
        panic!("hydrolysis preview test: invalid `{name}` value `{raw}`: {error}")
    }))
}

fn parse_optional_i32(name: &str) -> Option<i32> {
    let raw = env::var(name).ok()?;
    Some(raw.parse::<i32>().unwrap_or_else(|error| {
        panic!("hydrolysis preview test: invalid `{name}` value `{raw}`: {error}")
    }))
}

fn dimension_to_u32(value: f32) -> u32 {
    assert!(
        value.is_finite() && value > 0.0,
        "hydrolysis preview test dimension must be finite and positive"
    );
    value.round() as u32
}

fn duration_micros(duration: Duration) -> u128 {
    duration.as_micros()
}

fn cpu_percent_milli(value: f32) -> u32 {
    assert!(
        value.is_finite() && value >= 0.0,
        "hydrolysis preview perf: cpu percent must be finite and non-negative"
    );
    (value * 1000.0).round() as u32
}

fn write_status(message: &str) {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(message.as_bytes())
        .and_then(|()| stdout.write_all(b"\n"))
        .unwrap_or_else(|error| panic!("hydrolysis preview test: failed to write status: {error}"));
}
