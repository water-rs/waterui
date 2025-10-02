use waterui_color::Color;

#[test]
fn test_srgb_to_linear() {
    // Test black
    let black = Color::srgb(0, 0, 0, 1.0).resolve(&Default::default());
    assert!((black.red - 0.0).abs() < 1e-6);
    assert!((black.green - 0.0).abs() < 1e-6);
    assert!((black.blue - 0.0).abs() < 1e-6);

    // Test white
    let white = Color::srgb(255, 255, 255, 1.0).resolve(&Default::default());
    assert!((white.red - 1.0).abs() < 1e-6);
    assert!((white.green - 1.0).abs() < 1e-6);
    assert!((white.blue - 1.0).abs() < 1e-6);

    // Test 50% gray (128/255)
    let gray = Color::srgb(128, 128, 128, 1.0).resolve(&Default::default());
    // The linear value for sRGB 128/255 is approx 0.21586.
    let mid_gray_val = 0.2158608;
    assert!((gray.red - mid_gray_val).abs() < 1e-6);
    assert!((gray.green - mid_gray_val).abs() < 1e-6);
    assert!((gray.blue - mid_gray_val).abs() < 1e-6);
}

#[test]
fn test_p3_to_linear_srgb() {
    // Test P3 red, which is outside sRGB gamut
    let p3_red = Color::p3(1.0, 0.0, 0.0, 1.0).resolve(&Default::default());
    assert!((p3_red.red - 1.2249401).abs() < 1e-6);
    assert!((p3_red.green - -0.0420301).abs() < 1e-6);
    assert!((p3_red.blue - -0.0197211).abs() < 1e-6);

    // Test P3 green
    let p3_green = Color::p3(0.0, 1.0, 0.0, 1.0).resolve(&Default::default());
    assert!((p3_green.red - -0.2249401).abs() < 1e-6);
    assert!((p3_green.green - 1.0420301).abs() < 1e-6);
    assert!((p3_green.blue - -0.0786361).abs() < 1e-6);

    // Test a gray value, which should be the same in both spaces
    // The linear value for a gamma-corrected 0.5 is ~0.2140411
    let gray = Color::p3(0.5, 0.5, 0.5, 1.0).resolve(&Default::default());
    let linear_gray_val = 0.2140411;
    assert!((gray.red - linear_gray_val).abs() < 1e-5);
    assert!((gray.green - linear_gray_val).abs() < 1e-5);
    assert!((gray.blue - linear_gray_val).abs() < 1e-5);
}
