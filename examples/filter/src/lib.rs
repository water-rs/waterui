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
use waterui::reactive::Binding;

/// Sample content for demonstrating filters
fn sample_content() -> impl View {
    zstack((
        Color::srgb_hex("#3B82F6"),
        vstack((
            text("WaterUI")
                .size(24.0)
                .foreground(Color::srgb(255, 255, 255)),
            text("Filters")
                .size(16.0)
                .foreground(Color::srgb(255, 255, 255)),
        )),
    ))
    .size(120.0, 80.0)
}

/// Demo: Blur filter - Gaussian blur effect
fn blur_section(blur_radius: Binding<f64>) -> impl View {
    let animated_blur = blur_radius
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Blur").size(20.0),
        "Apply Gaussian blur to content",
        zstack((sample_content().blur(animated_blur),)).min_height(100.0),
        Slider::new(0.0..=20.0, &blur_radius),
        hstack((
            {
                let b = blur_radius.clone();
                button("0").action(move || b.set(0.0))
            },
            {
                let b = blur_radius.clone();
                button("5").action(move || b.set(5.0))
            },
            {
                let b = blur_radius.clone();
                button("10").action(move || b.set(10.0))
            },
            {
                let b = blur_radius.clone();
                button("20").action(move || b.set(20.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Brightness filter - lighten or darken
fn brightness_section(brightness: Binding<f64>) -> impl View {
    let animated_brightness = brightness
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Brightness").size(20.0),
        "Adjust brightness (-1 to 1)",
        zstack((sample_content().brightness(animated_brightness),)).min_height(100.0),
        Slider::new(-1.0..=1.0, &brightness),
        hstack((
            {
                let b = brightness.clone();
                button("-1").action(move || b.set(-1.0))
            },
            {
                let b = brightness.clone();
                button("0").action(move || b.set(0.0))
            },
            {
                let b = brightness.clone();
                button("0.5").action(move || b.set(0.5))
            },
            {
                let b = brightness.clone();
                button("1").action(move || b.set(1.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Saturation filter - color intensity
fn saturation_section(saturation: Binding<f64>) -> impl View {
    let animated_saturation = saturation
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Saturation").size(20.0),
        "Adjust color saturation (0 = grayscale)",
        zstack((sample_content().saturation(animated_saturation),)).min_height(100.0),
        Slider::new(0.0..=2.0, &saturation),
        hstack((
            {
                let s = saturation.clone();
                button("0").action(move || s.set(0.0))
            },
            {
                let s = saturation.clone();
                button("1").action(move || s.set(1.0))
            },
            {
                let s = saturation.clone();
                button("1.5").action(move || s.set(1.5))
            },
            {
                let s = saturation.clone();
                button("2").action(move || s.set(2.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Contrast filter
fn contrast_section(contrast: Binding<f64>) -> impl View {
    let animated_contrast = contrast
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Contrast").size(20.0),
        "Adjust color contrast",
        zstack((sample_content().contrast(animated_contrast),)).min_height(100.0),
        Slider::new(0.0..=2.0, &contrast),
        hstack((
            {
                let c = contrast.clone();
                button("0").action(move || c.set(0.0))
            },
            {
                let c = contrast.clone();
                button("1").action(move || c.set(1.0))
            },
            {
                let c = contrast.clone();
                button("1.5").action(move || c.set(1.5))
            },
            {
                let c = contrast.clone();
                button("2").action(move || c.set(2.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Hue rotation filter - shift colors
fn hue_rotation_section(hue: Binding<f64>) -> impl View {
    let animated_hue = hue
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(400)));

    vstack((
        text("Hue Rotation").size(20.0),
        "Rotate colors around the color wheel (0-360 degrees)",
        zstack((sample_content().hue_rotation(animated_hue),)).min_height(100.0),
        Slider::new(0.0..=360.0, &hue),
        hstack((
            {
                let h = hue.clone();
                button("0").action(move || h.set(0.0))
            },
            {
                let h = hue.clone();
                button("90").action(move || h.set(90.0))
            },
            {
                let h = hue.clone();
                button("180").action(move || h.set(180.0))
            },
            {
                let h = hue.clone();
                button("270").action(move || h.set(270.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Grayscale filter
fn grayscale_section(grayscale: Binding<f64>) -> impl View {
    let animated_grayscale = grayscale
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Grayscale").size(20.0),
        "Convert to grayscale (0 = color, 1 = grayscale)",
        zstack((sample_content().grayscale(animated_grayscale),)).min_height(100.0),
        Slider::new(0.0..=1.0, &grayscale),
        hstack((
            {
                let g = grayscale.clone();
                button("0").action(move || g.set(0.0))
            },
            {
                let g = grayscale.clone();
                button("0.5").action(move || g.set(0.5))
            },
            {
                let g = grayscale.clone();
                button("1").action(move || g.set(1.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Opacity filter
fn opacity_section(opacity: Binding<f64>) -> impl View {
    let animated_opacity = opacity
        .clone()
        .map(|v| v as f32)
        .with(Animation::ease_in_out(Duration::from_millis(300)));

    vstack((
        text("Opacity").size(20.0),
        "Adjust transparency (0 = invisible, 1 = opaque)",
        zstack((
            Color::srgb_hex("#EF4444").size(120.0, 80.0),
            sample_content().opacity(animated_opacity),
        ))
        .min_height(100.0),
        Slider::new(0.0..=1.0, &opacity),
        hstack((
            {
                let o = opacity.clone();
                button("0").action(move || o.set(0.0))
            },
            {
                let o = opacity.clone();
                button("0.5").action(move || o.set(0.5))
            },
            {
                let o = opacity.clone();
                button("1").action(move || o.set(1.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Combined filters with animation
fn combined_section(
    combined_blur: Binding<f64>,
    combined_saturation: Binding<f64>,
    combined_hue: Binding<f64>,
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
        text("Combined Filters").size(20.0),
        "Apply multiple filters with spring animations",
        zstack((sample_content()
            .blur(animated_blur)
            .saturation(animated_saturation)
            .hue_rotation(animated_hue),))
        .min_height(100.0),
        hstack((
            {
                let b = combined_blur.clone();
                let s = combined_saturation.clone();
                let h = combined_hue.clone();
                button("Reset").action(move || {
                    b.set(0.0);
                    s.set(1.0);
                    h.set(0.0);
                })
            },
            {
                let b = combined_blur.clone();
                let s = combined_saturation.clone();
                button("Dreamy").action(move || {
                    b.set(3.0);
                    s.set(0.7);
                })
            },
            {
                let h = combined_hue.clone();
                let s = combined_saturation.clone();
                button("Vibrant").action(move || {
                    h.set(180.0);
                    s.set(1.8);
                })
            },
            {
                let b = combined_blur.clone();
                let s = combined_saturation.clone();
                let h = combined_hue.clone();
                button("Vintage").action(move || {
                    b.set(1.0);
                    s.set(0.5);
                    h.set(30.0);
                })
            },
        )),
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    // State for individual filter sections (using f64 for Slider compatibility)
    let blur_radius = Binding::container(0.0_f64);
    let brightness = Binding::container(0.0_f64);
    let saturation = Binding::container(1.0_f64);
    let contrast = Binding::container(1.0_f64);
    let hue = Binding::container(0.0_f64);
    let grayscale = Binding::container(0.0_f64);
    let opacity = Binding::container(1.0_f64);

    // State for combined filter section
    let combined_blur = Binding::container(0.0_f64);
    let combined_saturation = Binding::container(1.0_f64);
    let combined_hue = Binding::container(0.0_f64);

    scroll(
        vstack((
            // Header
            text("WaterUI Filter Examples").size(28.0),
            "Visual demonstrations of the filter system",
            Divider,
            // First group of filters
            vstack((
                blur_section(blur_radius),
                Divider,
                brightness_section(brightness),
                Divider,
                saturation_section(saturation),
                Divider,
                contrast_section(contrast),
            )),
            // Second group of filters
            vstack((
                Divider,
                hue_rotation_section(hue),
                Divider,
                grayscale_section(grayscale),
                Divider,
                opacity_section(opacity),
                Divider,
                combined_section(combined_blur, combined_saturation, combined_hue),
            )),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
