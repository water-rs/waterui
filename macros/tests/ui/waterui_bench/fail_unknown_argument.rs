fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view, max_bogus = 3)]
fn unknown_argument(perf: &mut waterui_testing::PerfApp) {
    let _ = perf;
}

fn main() {}
