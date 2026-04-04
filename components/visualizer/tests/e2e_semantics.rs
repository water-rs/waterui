use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_testing::{MountedApp, Role};
use waterui_visualizer::{AudioCapture, Waveform};

fn waveform_view() -> impl waterui::View {
    Waveform::new(AudioCapture::default())
        .width(180.0)
        .height(120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Waveform")
}

#[waterui::test(waveform_view)]
fn waveform_exposes_accessibility_image(app: &mut MountedApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Waveform")
        .assert_exists();
}
