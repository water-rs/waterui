fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::test(sample_view)]
async fn sample_async_test(app: &mut waterui_testing::MountedApp) {
    let _ = app.tree();
}

fn main() {}
