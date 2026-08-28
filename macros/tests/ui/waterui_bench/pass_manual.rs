fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench]
fn manual(ui: waterui_testing::UiBuilder) -> waterui_testing::PerfReport {
    ui.perf_with(sample_view, |perf| {
        perf.measure("steady-redraw", |run| {
            run.redraw();
        });
    })
}

fn main() {}
