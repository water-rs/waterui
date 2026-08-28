fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::test(sample_view)]
fn wrong_return(_app: &mut waterui_testing::SemanticApp) -> bool {
    true
}

fn main() {}
