fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view, max_p95_us = 1_000, max_p95_us = 2_000)]
fn duplicate_budget(perf: &mut waterui_testing::PerfApp) {
    let _ = perf;
}

fn main() {}
