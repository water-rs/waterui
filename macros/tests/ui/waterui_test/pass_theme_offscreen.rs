fn sample_view() -> impl waterui::View {
    ()
}

fn no_extra_theme(_env: &mut waterui::env::Environment) {}

#[waterui::test(sample_view, theme = no_extra_theme, viewport = (240, 120), offscreen)]
fn themed_offscreen(app: &mut waterui_testing::OffscreenApp) {
    let _ = app.tree();
}

fn main() {}
