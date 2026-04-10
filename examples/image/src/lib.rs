//! Image Example - Demonstrates GPU-accelerated image processing
//!
//! This example showcases:
//! - `Image` - GPU-accelerated image display from pixel data
//! - `Photo` - Async image loading from URL with GPU filters
//! - `filtrate` - GPU filter effects (blur, brightness, saturation, etc.)
//! - Interactive blur control with Slider (real-time updates)
//! - Custom URL loading with TextField and Button

use waterui::app::App;
use waterui::media::photo::Event as PhotoEvent;
use waterui::media::{Image, Photo};
use waterui::prelude::*;

// Note: filtrate is re-exported through waterui::media as Filter

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

/// Section showing Photo loading from URL with interactive blur control
fn photo_section() -> impl View {
    let blur_value = Binding::f64(2.0);
    let status_message: Binding<String> = Binding::container(String::from("Loading..."));

    // Photo with reactive blur - blur updates in real-time as slider moves
    let photo = Photo::new("https://picsum.photos/200/150")
        .on_event({
            let status = status_message.clone();
            move |event| match event {
                PhotoEvent::Loaded => status.set(String::from("Loaded successfully")),
                PhotoEvent::Error(msg) => status.set(format!("Error: {msg}")),
            }
        })
        .blur(blur_value.clone()); // Pass binding for reactive blur

    vstack((
        text("Photo from URL").headline(),
        text!("{status_message}").sub_headline(),
        photo,
        hstack((
            text("Blur:"),
            Slider::new(0.0..=10.0, &blur_value),
            text!("{blur_value:.1}"),
        )),
    ))
    .padding()
}

/// Section with custom URL input and load button
fn custom_url_section() -> impl View {
    let url_input: Binding<Str> = Binding::container(Str::from("https://picsum.photos/300/200"));
    let blur_value = Binding::f64(0.0);
    let status_message: Binding<String> =
        Binding::container(String::from("Enter a URL and click Load"));
    let (handler, photo_view) = Dynamic::new();

    // Set initial placeholder
    handler.set(text("No image loaded yet").foreground(Grey));

    vstack((
        text("Custom URL Loader").headline(),
        "Load any image from the web (blur updates in real-time)",
        hstack((
            TextField::new(&url_input).prompt("Enter image URL"),
            button("Load")
                .bordered_prominent()
                .action(
                    |State(url): State<Binding<Str>>,
                     State(blur): State<Binding<f64>>,
                     State(status): State<Binding<String>>,
                     State(handler): State<DynamicHandler>| {
                        let url_str = url.get();
                        if url_str.is_empty() {
                            status.set(String::from("Please enter a URL"));
                            return;
                        }
                        status.set(String::from("Loading..."));
                        // Pass blur binding for reactive updates
                        let photo = Photo::new(url_str)
                            .on_event({
                                let status = status.clone();
                                move |event| match event {
                                    PhotoEvent::Loaded => {
                                        status.set(String::from("Loaded successfully"))
                                    }
                                    PhotoEvent::Error(msg) => status.set(format!("Error: {msg}")),
                                }
                            })
                            .blur(blur.clone());
                        handler.set(photo);
                    },
                )
                .state(&url_input)
                .state(&blur_value)
                .state(&status_message)
                .state(&handler),
        )),
        text!("{status_message}").sub_headline(),
        photo_view,
        hstack((
            text("Blur:"),
            Slider::new(0.0..=20.0, &blur_value),
            text!("{blur_value:.1}"),
        )),
    ))
    .padding()
}

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
            Divider,
            // Photo test with interactive blur
            text("Photo Test").headline(),
            photo_section(),
            Divider,
            // Custom URL loader
            custom_url_section(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

