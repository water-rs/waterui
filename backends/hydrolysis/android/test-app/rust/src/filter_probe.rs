//! The `#1750` acceptance scene: every linear filter HWUI maps plus blur,
//! driven through the same `Filtered`/`FilterViewExt` calls the GPU painter
//! realizes through Filtrate, so the two APKs render identical frames that
//! can be compared visually on-device. `Brightness` runs a `Binding` animated
//! with a 400 ms ease-in-out to prove live reactivity; the rest are static.

use std::time::Duration;

use waterui::animation::Animation;
use waterui::graphics::color::{Blue, Color, Green, Grey, Red};
use waterui::graphics::filtrate::filters;
use waterui::layout::frame::Frame;
use waterui::prelude::*;
use waterui::reactive::binding;

/// The swatch the filters act on: saturated primaries over a neutral row.
fn sample() -> impl View {
    vstack((
        hstack((
            Frame::new(Red).width(24.0).height(16.0),
            Frame::new(Green).width(24.0).height(16.0),
            Frame::new(Blue).width(24.0).height(16.0),
        ))
        .spacing(0.0),
        hstack((
            Frame::new(Color::srgb_u32(0x000_000))
                .width(24.0)
                .height(16.0),
            Frame::new(Grey).width(24.0).height(16.0),
            Frame::new(Color::srgb_u32(0xFF_FFFF))
                .width(24.0)
                .height(16.0),
        ))
        .spacing(0.0),
    ))
    .spacing(0.0)
}

/// A labelled tile at a fixed width so the grid's columns stay aligned.
fn tile(label: &'static str, content: impl View) -> impl View {
    Frame::new(vstack((text(label).caption(), content)).spacing(4.0)).width(96.0)
}

pub fn view() -> impl View {
    let brightness: Binding<f32> = binding(0.0f32);
    let animated = brightness.with(Animation::ease_in_out(Duration::from_millis(400)));
    vstack((
        text("Filter validation").title(),
        text!("Brightness: {value}", value = brightness),
        hstack((
            button("Brighten")
                .action(|State(value): State<Binding<f32>>| value.set(0.25f32))
                .state(&brightness),
            button("Reset")
                .action(|State(value): State<Binding<f32>>| value.set(0.0f32))
                .state(&brightness),
        )),
        hstack((
            tile("Original", sample()),
            tile("Brightness", sample().brightness(animated)),
            tile("Blur", sample().blur(2.0f32)),
        )),
        hstack((
            tile("Contrast", sample().contrast(1.5f32)),
            tile("Exposure", sample().exposure(1.0f32)),
            tile(
                "White point",
                sample().filter(filters::WhitePoint(1.0f32, 0.8f32, 0.6f32)),
            ),
        )),
        hstack((
            tile("Saturation", sample().saturation(0.25f32)),
            tile("Hue rotation", sample().hue_rotation(90.0f32)),
            tile("Grayscale", sample().grayscale(0.6f32)),
        )),
        hstack((
            tile("Sepia", sample().sepia(0.8f32)),
            tile("Invert", sample().invert()),
            tile(
                "Color matrix",
                sample().color_matrix([
                    [0.0, 1.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ]),
            ),
        )),
        hstack((
            tile("Mono", sample().filter(filters::PhotoEffectMono)),
            tile("Tonal", sample().filter(filters::PhotoEffectTonal)),
            tile(
                "Color / blur",
                sample().brightness(0.1f32).blur(2.0f32).saturation(0.5f32),
            ),
        )),
    ))
    .spacing(12.0)
    .padding()
}
