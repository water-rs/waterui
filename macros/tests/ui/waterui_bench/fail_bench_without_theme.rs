fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view)]
fn steady(perf: &mut waterui_testing::PerfApp) {
    let _ = perf;
}

fn main() {}
