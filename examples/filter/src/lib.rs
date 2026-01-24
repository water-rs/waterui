//! Filter Example - Demonstrates WaterUI's visual filter system
//!
//! This example showcases visual filters that can be applied to any view:
//! - Blur - Gaussian blur effect
//! - Brightness - Lighten or darken content
//! - Saturation - Adjust color intensity
//! - Contrast - Adjust color contrast
//! - Hue Rotation - Shift colors around the color wheel
//! - Grayscale - Convert to grayscale
//! - Opacity - Adjust transparency
//!
//! All filters support reactive values and can be animated using
//! the `.with(Animation::...)` modifier.

use core::time::Duration;
use waterui::animation::Animation;
use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::Binding;

/// Sample content for demonstrating filters
fn sample_content() -> impl View {
    zstack((
        Blue,
        vstack((
            text("WaterUI").title().foreground(Srgb::WHITE),
            text("Filters").body().foreground(Srgb::WHITE),
        )),
    ))
    .size(120.0, 80.0)
}

/// Demo: Blur filter - Gaussian blur effect
fn blur_section(blur_radius: &Binding<f64>) -> impl View {
    let animated_blur = blur_radius
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Blur").headline(),
        "Apply Gaussian blur to content",
        sample_content().blur(animated_blur).min_height(100.0),
        Slider::new(0.0..=20.0, blur_radius),
        hstack((
            button("0").with_state(blur_radius).action(|b| b.set(0.0)),
            button("5").with_state(blur_radius).action(|b| b.set(5.0)),
            button("10").with_state(blur_radius).action(|b| b.set(10.0)),
            button("20").with_state(blur_radius).action(|b| b.set(20.0)),
        )),
    ))
    .padding()
}

/// Demo: Brightness filter - lighten or darken
fn brightness_section(brightness: &Binding<f64>) -> impl View {
    let animated_brightness = brightness
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Brightness").headline(),
        "Adjust brightness (-1 to 1)",
        sample_content().brightness(animated_brightness).min_height(100.0),
        Slider::new(-1.0..=1.0, brightness),
        hstack((
            button("-1").with_state(brightness).action(|b| b.set(-1.0)),
            button("0").with_state(brightness).action(|b| b.set(0.0)),
            button("0.5").with_state(brightness).action(|b| b.set(0.5)),
            button("1").with_state(brightness).action(|b| b.set(1.0)),
        )),
    ))
    .padding()
}

/// Demo: Saturation filter - color intensity
fn saturation_section(saturation: &Binding<f64>) -> impl View {
    let animated_saturation = saturation
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Saturation").headline(),
        "Adjust color saturation (0 = grayscale)",
        sample_content().saturation(animated_saturation).min_height(100.0),
        Slider::new(0.0..=2.0, saturation),
        hstack((
            button("0").with_state(saturation).action(|s| s.set(0.0)),
            button("1").with_state(saturation).action(|s| s.set(1.0)),
            button("1.5").with_state(saturation).action(|s| s.set(1.5)),
            button("2").with_state(saturation).action(|s| s.set(2.0)),
        )),
    ))
    .padding()
}

/// Demo: Contrast filter
fn contrast_section(contrast: &Binding<f64>) -> impl View {
    let animated_contrast = contrast
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Contrast").headline(),
        "Adjust color contrast",
        sample_content().contrast(animated_contrast).min_height(100.0),
        Slider::new(0.0..=2.0, contrast),
        hstack((
            button("0").with_state(contrast).action(|c| c.set(0.0)),
            button("1").with_state(contrast).action(|c| c.set(1.0)),
            button("1.5").with_state(contrast).action(|c| c.set(1.5)),
            button("2").with_state(contrast).action(|c| c.set(2.0)),
        )),
    ))
    .padding()
}

/// Demo: Hue rotation filter - shift colors
fn hue_rotation_section(hue: &Binding<f64>) -> impl View {
    let animated_hue = hue
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(400)));

    vstack((
        text("Hue Rotation").headline(),
        "Rotate colors around the color wheel (0-360 degrees)",
        sample_content().hue_rotation(animated_hue).min_height(100.0),
        Slider::new(0.0..=360.0, hue),
        hstack((
            button("0").with_state(hue).action(|h| h.set(0.0)),
            button("90").with_state(hue).action(|h| h.set(90.0)),
            button("180").with_state(hue).action(|h| h.set(180.0)),
            button("270").with_state(hue).action(|h| h.set(270.0)),
        )),
    ))
    .padding()
}

/// Demo: Grayscale filter
fn grayscale_section(grayscale: &Binding<f64>) -> impl View {
    let animated_grayscale = grayscale
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Grayscale").headline(),
        "Convert to grayscale (0 = color, 1 = grayscale)",
        sample_content().grayscale(animated_grayscale).min_height(100.0),
        Slider::new(0.0..=1.0, grayscale),
        hstack((
            button("0").with_state(grayscale).action(|g| g.set(0.0)),
            button("0.5").with_state(grayscale).action(|g| g.set(0.5)),
            button("1").with_state(grayscale).action(|g| g.set(1.0)),
        )),
    ))
    .padding()
}

/// Demo: Opacity filter
fn opacity_section(opacity: &Binding<f64>) -> impl View {
    let animated_opacity = opacity
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Opacity").headline(),
        "Adjust transparency (0 = invisible, 1 = opaque)",
        sample_content().opacity(animated_opacity).min_height(100.0),
        Slider::new(0.0..=1.0, opacity),
        hstack((
            button("0").with_state(opacity).action(|o| o.set(0.0)),
            button("0.5").with_state(opacity).action(|o| o.set(0.5)),
            button("1").with_state(opacity).action(|o| o.set(1.0)),
        )),
    ))
    .padding()
}

/// Demo: Combined filters with animation
fn combined_section(
    combined_blur: &Binding<f64>,
    combined_saturation: &Binding<f64>,
    combined_hue: &Binding<f64>,
) -> impl View {
    let animated_blur = combined_blur
        .clone()
        .map(|v| v as f32)
        .with(Animation::spring(200.0, 15.0));
    let animated_saturation = combined_saturation
        .clone()
        .map(|v| v as f32)
        .with(Animation::spring(200.0, 15.0));
    let animated_hue = combined_hue
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(500)));

    vstack((
        text("Combined Filters").headline(),
        "Apply multiple filters with spring animations",
        sample_content()
            .blur(animated_blur)
            .saturation(animated_saturation)
            .hue_rotation(animated_hue)
            .min_height(100.0),
        hstack((
            button("Reset")
                .with_state(combined_blur)
                .with_state(combined_saturation)
                .with_state(combined_hue)
                .action(|((b, s), h)| {
                    b.set(0.0);
                    s.set(1.0);
                    h.set(0.0);
                }),
            button("Dreamy")
                .with_state(combined_blur)
                .with_state(combined_saturation)
                .action(|(b, s)| {
                    b.set(3.0);
                    s.set(0.7);
                }),
            button("Vibrant")
                .with_state(combined_hue)
                .with_state(combined_saturation)
                .action(|(h, s)| {
                    h.set(180.0);
                    s.set(1.8);
                }),
            button("Vintage")
                .with_state(combined_blur)
                .with_state(combined_saturation)
                .with_state(combined_hue)
                .action(|((b, s), h)| {
                    b.set(1.0);
                    s.set(0.5);
                    h.set(30.0);
                }),
        )),
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    // State for individual filter sections (using f64 for Slider compatibility)
    let blur_radius = Binding::f64(0.0);
    let brightness = Binding::f64(0.0);
    let saturation = Binding::f64(1.0);
    let contrast = Binding::f64(1.0);
    let hue = Binding::f64(0.0);
    let grayscale = Binding::f64(0.0);
    let opacity = Binding::f64(1.0);

    // State for combined filter section
    let combined_blur = Binding::f64(0.0);
    let combined_saturation = Binding::f64(1.0);
    let combined_hue = Binding::f64(0.0);

    scroll(
        vstack((
            // Header
            text("WaterUI Filter Examples").title(),
            "Visual demonstrations of the filter system",
            Divider,
            // First group of filters
            vstack((
                blur_section(&blur_radius),
                Divider,
                brightness_section(&brightness),
                Divider,
                saturation_section(&saturation),
                Divider,
                contrast_section(&contrast),
            )),
            // Second group of filters
            vstack((
                Divider,
                hue_rotation_section(&hue),
                Divider,
                grayscale_section(&grayscale),
                Divider,
                opacity_section(&opacity),
                Divider,
                combined_section(&combined_blur, &combined_saturation, &combined_hue),
            )),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
