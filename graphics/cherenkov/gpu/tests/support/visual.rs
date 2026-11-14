//! Helpers that turn readbacks into images for visual-equivalence checks.

use std::ops::Range;

use cherenkov::Readback;
use cherenkov_oracle::F32Image;

/// The `x` by `y` window of `readback` as an image.
pub fn crop(readback: &Readback, x: Range<usize>, y: Range<usize>) -> F32Image {
    let width = readback.width as usize;
    F32Image {
        width: u32::try_from(x.len()).expect("window width fits u32"),
        height: u32::try_from(y.len()).expect("window height fits u32"),
        pixels: y
            .flat_map(|row| x.clone().map(move |col| (col, row)))
            .map(|(col, row)| readback.pixels[row * width + col])
            .collect(),
    }
}
