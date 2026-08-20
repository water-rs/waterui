fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view)]
fn wrong_param_count() {}

fn main() {}
