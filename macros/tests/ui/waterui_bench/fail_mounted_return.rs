fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view)]
fn returns_report(perf: &mut waterui_testing::PerfApp) -> u32 {
    let _ = perf;
    0
}

fn main() {}
