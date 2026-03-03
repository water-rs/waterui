fn sample_view() -> impl waterui::View {
    ()
}

#[waterui::test(sample_view)]
fn wrong_param_type(_app: &waterui_testing::MountedApp) {}

fn main() {}
