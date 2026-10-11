fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::bench(sample_view, theme = some_style)]
fn wrong_param_count() {}

fn main() {}
