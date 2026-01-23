//! Image Example - Demonstrates GPU-accelerated image processing
//!
//! This example showcases:
//! - `Image` - GPU-accelerated image display from pixel data
//! - `Photo` - Async image loading from URL with GPU filters
//! - `filtrate` - GPU filter effects (blur, brightness, saturation, etc.)

use waterui::app::App;
use waterui::prelude::*;
use waterui_media::{Image, Photo};

// Note: filtrate is re-exported through waterui_media as Filter

/// Generate a colorful test pattern (RGBA pixels)
fn generate_test_pattern(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);

    for y in 0..height {
        for x in 0..width {
            // Create a gradient pattern
            let r = ((x as f32 / width as f32) * 255.0) as u8;
            let g = ((y as f32 / height as f32) * 255.0) as u8;
            let b = (((x + y) as f32 / (width + height) as f32) * 255.0) as u8;
            let a = 255u8;

            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(a);
        }
    }

    pixels
}

/// Generate a checkerboard pattern
fn generate_checkerboard(width: u32, height: u32, square_size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);

    for y in 0..height {
        for x in 0..width {
            let is_white = ((x / square_size) + (y / square_size)) % 2 == 0;
            let color = if is_white { 255u8 } else { 50u8 };

            pixels.push(color); // R
            pixels.push(color); // G
            pixels.push(color); // B
            pixels.push(255);   // A
        }
    }

    pixels
}

/// Section showing Image with no filters
fn original_image_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Original Image").headline(),
        "No filters applied",
        Image::new(pixels, 200, 150),
    ))
    .padding()
}

/// Section showing Image with blur filter
fn blur_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Blur Filter").headline(),
        "Gaussian blur (radius: 3.0)",
        Image::new(pixels, 200, 150).blur(3.0_f32),
    ))
    .padding()
}

/// Section showing Image with brightness filter
fn brightness_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Brightness Filter").headline(),
        "Increased brightness (+0.3)",
        Image::new(pixels, 200, 150).brightness(0.3_f32),
    ))
    .padding()
}

/// Section showing Image with saturation filter
fn saturation_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Saturation Filter").headline(),
        "Increased saturation (1.5x)",
        Image::new(pixels, 200, 150).saturation(1.5_f32),
    ))
    .padding()
}

/// Section showing Image with grayscale filter
fn grayscale_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Grayscale Filter").headline(),
        "Full grayscale conversion",
        Image::new(pixels, 200, 150).grayscale(1.0_f32),
    ))
    .padding()
}

/// Section showing Image with sepia filter
fn sepia_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Sepia Filter").headline(),
        "Vintage sepia tone",
        Image::new(pixels, 200, 150).sepia(1.0_f32),
    ))
    .padding()
}

/// Section showing Image with invert filter
fn invert_section() -> impl View {
    let pixels = generate_checkerboard(200, 150, 20);

    vstack((
        text("Invert Filter").headline(),
        "Inverted colors",
        Image::new(pixels, 200, 150).invert(),
    ))
    .padding()
}

/// Section showing Image with hue rotation
fn hue_rotation_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Hue Rotation").headline(),
        "Colors rotated 180 degrees",
        Image::new(pixels, 200, 150).hue_rotation(180.0_f32),
    ))
    .padding()
}

/// Section showing Image with vignette filter
fn vignette_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Vignette Filter").headline(),
        "Darkened corners",
        Image::new(pixels, 200, 150).vignette(0.5_f32, 0.5_f32),
    ))
    .padding()
}

/// Section showing Image with multiple combined filters
fn combined_filters_section() -> impl View {
    let pixels = generate_test_pattern(200, 150);

    vstack((
        text("Combined Filters").headline(),
        "Blur + Saturation + Vignette",
        Image::new(pixels, 200, 150)
            .blur(2.0_f32)
            .saturation(1.3_f32)
            .vignette(0.6_f32, 0.4_f32),
    ))
    .padding()
}

/// Section showing Photo loading from URL
fn photo_section() -> impl View {
    vstack((
        text("Photo from URL").headline(),
        "Async loading with blur filter",
        Photo::new("https://picsum.photos/200/150")
            .blur(2.0_f32),
    ))
    .padding()
}

/// Section showing Photo with vintage filter preset
fn photo_vintage_section() -> impl View {
    vstack((
        text("Photo - Vintage Style").headline(),
        "Sepia + Vignette + Contrast",
        Photo::new("https://picsum.photos/200/151")
            .sepia(0.7_f32)
            .contrast(1.2_f32)
            .vignette(0.5_f32, 0.5_f32),
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    scroll(
        vstack((
            // Header
            text("GPU Image Processing").title(),
            "Demonstrating filtrate GPU filters on Image and Photo",
            Divider,

            // Simple test - just one Image
            text("Image Test").headline(),
            original_image_section(),

            // Photo test
            text("Photo Test").headline(),
            photo_section(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
