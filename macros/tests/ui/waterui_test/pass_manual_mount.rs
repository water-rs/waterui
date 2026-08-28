fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::test]
fn manual_mount(ui: waterui_testing::UiBuilder) {
    let mut app = ui.mount(sample_view);
    let _ = app.tree();
}

fn main() {}
