fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view)]
async fn async_bench(perf: &mut waterui_testing::PerfApp) {
    let _ = perf;
}

fn main() {}
