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
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::Binding;

fn set_f64_button(label: &'static str, value: f64, binding: &Binding<f64>) -> impl View {
    button(label)
        .action(move |State(current): State<Binding<f64>>| current.set(value))
        .state(binding)
}

/// Sample content for demonstrating filters
fn sample_content() -> impl View {
    vstack((
        hstack((
            Red.size(40.0, 40.0),
            Green.size(40.0, 40.0),
            Blue.size(40.0, 40.0),
        ))
        .spacing(0.0),
        hstack((
            Yellow.size(40.0, 40.0),
            Purple.size(40.0, 40.0),
            Cyan.size(40.0, 40.0),
        ))
        .spacing(0.0),
    ))
    .spacing(0.0)
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
            set_f64_button("0", 0.0, blur_radius),
            set_f64_button("5", 5.0, blur_radius),
            set_f64_button("10", 10.0, blur_radius),
            set_f64_button("20", 20.0, blur_radius),
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
        sample_content()
            .brightness(animated_brightness)
            .min_height(100.0),
        Slider::new(-1.0..=1.0, brightness),
        hstack((
            set_f64_button("-1", -1.0, brightness),
            set_f64_button("0", 0.0, brightness),
            set_f64_button("0.5", 0.5, brightness),
            set_f64_button("1", 1.0, brightness),
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
        sample_content()
            .saturation(animated_saturation)
            .min_height(100.0),
        Slider::new(0.0..=2.0, saturation),
        hstack((
            set_f64_button("0", 0.0, saturation),
            set_f64_button("1", 1.0, saturation),
            set_f64_button("1.5", 1.5, saturation),
            set_f64_button("2", 2.0, saturation),
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
        sample_content()
            .contrast(animated_contrast)
            .min_height(100.0),
        Slider::new(0.0..=2.0, contrast),
        hstack((
            set_f64_button("0", 0.0, contrast),
            set_f64_button("1", 1.0, contrast),
            set_f64_button("1.5", 1.5, contrast),
            set_f64_button("2", 2.0, contrast),
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
        sample_content()
            .hue_rotation(animated_hue)
            .min_height(100.0),
        Slider::new(0.0..=360.0, hue),
        hstack((
            set_f64_button("0", 0.0, hue),
            set_f64_button("90", 90.0, hue),
            set_f64_button("180", 180.0, hue),
            set_f64_button("270", 270.0, hue),
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
        sample_content()
            .grayscale(animated_grayscale)
            .min_height(100.0),
        Slider::new(0.0..=1.0, grayscale),
        hstack((
            set_f64_button("0", 0.0, grayscale),
            set_f64_button("0.5", 0.5, grayscale),
            set_f64_button("1", 1.0, grayscale),
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
            set_f64_button("0", 0.0, opacity),
            set_f64_button("0.5", 0.5, opacity),
            set_f64_button("1", 1.0, opacity),
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
                .action(
                    |State(b): State<Binding<f64>>,
                     State(s): State<Binding<f64>>,
                     State(h): State<Binding<f64>>| {
                        b.set(0.0);
                        s.set(1.0);
                        h.set(0.0);
                    },
                )
                .state(combined_blur)
                .state(combined_saturation)
                .state(combined_hue),
            button("Dreamy")
                .action(
                    |State(b): State<Binding<f64>>, State(s): State<Binding<f64>>| {
                        b.set(3.0);
                        s.set(0.7);
                    },
                )
                .state(combined_blur)
                .state(combined_saturation),
            button("Vibrant")
                .action(
                    |State(h): State<Binding<f64>>, State(s): State<Binding<f64>>| {
                        h.set(180.0);
                        s.set(1.8);
                    },
                )
                .state(combined_hue)
                .state(combined_saturation),
            button("Vintage")
                .action(
                    |State(b): State<Binding<f64>>,
                     State(s): State<Binding<f64>>,
                     State(h): State<Binding<f64>>| {
                        b.set(1.0);
                        s.set(0.5);
                        h.set(30.0);
                    },
                )
                .state(combined_blur)
                .state(combined_saturation)
                .state(combined_hue),
        )),
    ))
    .padding()
}

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

#[preview]
fn filter_preview() -> impl View {
    vstack((
        text("Filter Smoke").headline(),
        sample_content()
            .blur(4.0)
            .saturation(1.3)
            .hue_rotation(36.0)
            .contrast(1.1)
            .size(220.0, 140.0),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
