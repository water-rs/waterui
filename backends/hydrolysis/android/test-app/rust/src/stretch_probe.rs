//! The `#3752` acceptance scene: three scroll containers whose overscroll
//! stretch the two painters realize, so the APKs can be compared visually
//! on-device. A vertical scroll below a heading (a nonzero viewport origin),
//! a horizontal scroll, and a vertical scroll under a rotated, scaled and
//! therefore skewed-to-the-axes ancestor.

use waterui::graphics::color::Srgb;
use waterui::layout::frame::Frame;
use waterui::prelude::*;

/// Distinct hues, so a warp reads against the stripes it moves.
const PALETTE: [&str; 6] = [
    "#E53935", "#FB8C00", "#FDD835", "#43A047", "#1E88E5", "#8E24AA",
];

/// Content bands in both scroll directions, enough to overflow the viewports.
const BANDS: usize = 12;

fn row(index: usize) -> impl View {
    Frame::new(text!("Row {index}", index = index.to_string()))
        .max_width(f32::INFINITY)
        .height(64.0)
        .background(Srgb::from_hex(PALETTE[index % PALETTE.len()]))
}

fn column(index: usize) -> impl View {
    Frame::new(text!("{index}", index = index.to_string()))
        .width(96.0)
        .max_height(f32::INFINITY)
        .background(Srgb::from_hex(PALETTE[index % PALETTE.len()]))
}

fn vertical() -> impl View {
    scroll(vstack((0..BANDS).map(row).collect::<Vec<_>>()).spacing(0.0)).height(220.0)
}

fn horizontal() -> impl View {
    scroll_horizontal(hstack((0..BANDS).map(column).collect::<Vec<_>>()).spacing(0.0)).height(100.0)
}

pub fn view() -> impl View {
    vstack((
        text("Stretch probe").title(),
        vertical(),
        text("horizontal"),
        horizontal(),
        text("transformed"),
        vertical().rotation(10.0).scale(0.9, 0.7),
    ))
    .padding()
}
