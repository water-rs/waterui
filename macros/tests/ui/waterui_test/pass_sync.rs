fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::test(sample_view)]
fn sample_sync_test(app: &mut waterui_testing::SemanticApp) {
    let _ = app.tree();
}

fn main() {}
