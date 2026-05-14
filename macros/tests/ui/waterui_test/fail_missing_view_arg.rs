fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::test]
fn missing_arg(app: &mut waterui_testing::SemanticApp) {
    let _ = (sample_view, app);
}

fn main() {}
