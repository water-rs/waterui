//! Conversion of perf measurements into the shared bench wire types.
//!
//! `#[waterui::bench]` full runs serialize the converted report as JSON into
//! the bench report directory; the `water bench` command deserializes the same
//! types, so field renames break the build instead of silently
//! desynchronizing.

use std::time::Duration;

use waterui_preview_protocol::bench::{PerfFrame, PerfMeasurement, PerfPhases};

use crate::perf::PerfReport;

/// Converts a perf report into the shared bench wire measurements.
#[must_use]
pub fn perf_report_to_protocol(report: &PerfReport) -> Vec<PerfMeasurement> {
    report
        .measurements()
        .iter()
        .map(|measurement| {
            let stats = measurement.stats();
            PerfMeasurement {
                name: measurement.name.clone(),
                samples: stats.samples as u64,
                mean_us: micros(stats.mean),
                median_us: micros(stats.median),
                p95_us: micros(stats.p95),
                min_us: micros(stats.min),
                max_us: micros(stats.max),
                rebuilt_frames: stats.rebuilt_frames as u64,
                rendered_frames: stats.rendered_frames as u64,
                idle_frames: stats.idle_frames as u64,
                rendered_mean_us: micros(stats.rendered_total.mean),
                rendered_p95_us: micros(stats.rendered_total.p95),
                rendered_max_us: micros(stats.rendered_total.max),
                missed_120fps_frames: stats.missed_120fps_frames as u64,
                missed_60fps_frames: stats.missed_60fps_frames as u64,
                measurement_cache_hits: stats.measurement_cache_hits,
                measurement_cache_misses: stats.measurement_cache_misses,
                scene_layers: stats.scene_layers,
                vello_scene_layers: stats.vello_scene_layers,
                gpu_surface_layers: stats.gpu_surface_layers,
                clip_layers: stats.clip_layers,
                max_clip_depth: stats.max_clip_depth,
                applied_filter_count: stats.applied_filter_count,
                applied_filter_capture_us: stats.applied_filter_capture_us,
                applied_filter_effect_us: stats.applied_filter_effect_us,
                phases: PerfPhases {
                    rebuild_mean_us: micros(stats.phases.rebuild.mean),
                    rebuild_p95_us: micros(stats.phases.rebuild.p95),
                    build_content_mean_us: micros(stats.phases.build_content.mean),
                    build_content_p95_us: micros(stats.phases.build_content.p95),
                    scene_dispatch_mean_us: micros(stats.phases.scene_dispatch.mean),
                    scene_dispatch_p95_us: micros(stats.phases.scene_dispatch.p95),
                    scene_finish_mean_us: micros(stats.phases.scene_finish.mean),
                    scene_finish_p95_us: micros(stats.phases.scene_finish.p95),
                    render_mean_us: micros(stats.phases.render.mean),
                    render_p95_us: micros(stats.phases.render.p95),
                    animation_mean_us: micros(stats.phases.animation.mean),
                    input_mean_us: micros(stats.phases.input.mean),
                },
                frames: measurement
                    .frames
                    .iter()
                    .enumerate()
                    .map(|(index, frame)| PerfFrame {
                        index: index as u64,
                        total_us: micros(frame.total),
                        rebuild_us: micros(frame.profile.phases.rebuild),
                        build_content_us: micros(frame.profile.phases.build_content),
                        scene_dispatch_us: micros(frame.profile.phases.scene_dispatch),
                        scene_finish_us: micros(frame.profile.phases.scene_finish),
                        render_us: micros(frame.profile.phases.render),
                        acquire_us: micros(frame.profile.phases.acquire),
                        present_us: micros(frame.profile.phases.present),
                        animation_us: micros(frame.profile.phases.animation),
                        input_us: micros(frame.profile.phases.input),
                        executor_before_us: micros(frame.profile.phases.executor_before),
                        executor_after_us: micros(frame.profile.phases.executor_after),
                        rebuilt: frame.rebuilt,
                        rendered: frame.profile.counters.rendered,
                        captured_snapshot: frame.profile.counters.captured_snapshot,
                        cpu_percent: f64::from(frame.resources.cpu_percent),
                        memory_bytes: frame.resources.memory_bytes,
                        gpu_frame_us: micros(
                            frame.profile.phases.acquire
                                + frame.profile.phases.render
                                + frame.profile.phases.present,
                        ),
                        measurement_cache_hits: u64::from(
                            frame.profile.counters.measurement_cache_hits,
                        ),
                        measurement_cache_misses: u64::from(
                            frame.profile.counters.measurement_cache_misses,
                        ),
                        scene_layers: u64::from(frame.profile.counters.scene_layers),
                        vello_scene_layers: u64::from(frame.profile.counters.vello_scene_layers),
                        gpu_surface_layers: u64::from(frame.profile.counters.gpu_surface_layers),
                        clip_layers: u64::from(frame.profile.counters.clip_layers),
                        max_clip_depth: u64::from(frame.profile.counters.max_clip_depth),
                        applied_filter_count: u64::from(
                            frame.profile.counters.applied_filter_count,
                        ),
                        applied_filter_capture_us: frame.profile.counters.applied_filter_capture_us,
                        applied_filter_effect_us: frame.profile.counters.applied_filter_effect_us,
                    })
                    .collect(),
            }
        })
        .collect()
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
