fn sample_view() -> impl waterui::View {
    ()
}

fn sample_theme(_env: &mut waterui::env::Environment) {}

#[waterui::bench(
    sample_view,
    theme = sample_theme,
    viewport = (390, 844),
    max_p95_us = 8_000,
    max_mean_us = 6_000,
    max_rebuild_ratio = 0.25,
    max_scene_layers = 64,
    max_gpu_surface_layers = 4,
    max_clip_layers = 128,
)]
fn budgeted(perf: &mut waterui_testing::PerfApp) {
    perf.measure("steady-redraw", |run| {
        run.redraw();
    });
}

fn main() {}
