use image::ImageEncoder as _;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_image::Image;
use waterui_testing::{Role, SemanticApp};

fn gpu_image_view() -> impl waterui::View {
    Image::new(
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
        2,
        2,
    )
    .size(120.0, 120.0)
    .a11y_role(AccessibilityRole::Image)
    .a11y_label("GPU image")
}

fn decoded_image_view() -> impl waterui::View {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
            2,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .expect("sample PNG should encode");
    Image::from_encoded(&png)
        .expect("sample PNG should decode")
        .size(120.0, 120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Decoded image")
}

#[waterui::test(gpu_image_view)]
fn gpu_image_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("GPU image")
        .assert_exists();
}

#[waterui::test(decoded_image_view)]
fn decoded_image_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Decoded image")
        .assert_exists();
}
