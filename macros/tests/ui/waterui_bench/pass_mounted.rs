fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view)]
fn steady(perf: &mut waterui_testing::PerfApp) {
    perf.measure("steady-redraw", |run| {
        run.redraw();
    });
}

fn main() {}
