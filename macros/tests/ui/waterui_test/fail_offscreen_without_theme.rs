fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::test(sample_view, offscreen)]
fn offscreen_without_theme(_app: &mut waterui_testing::OffscreenApp) {}

fn main() {}
