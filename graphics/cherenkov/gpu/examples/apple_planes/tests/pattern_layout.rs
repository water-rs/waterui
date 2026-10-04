//! `fill_bgra`'s geometry: the quadrants land where the picture puts
//! them, the sweep bar moves with the frame counter, and the strip's
//! cells carry the counter bits — the checks the YUV layout test does
//! for Android, on the BGRA layout Apple planes read.

use apple_planes::pattern::{self, BAR, CELL, HEIGHT, STRIP, WIDTH};

/// Fills a tight `32BGRA` image and returns it as packed pixels.
fn image(frame: u64) -> Vec<u32> {
    let mut bytes = vec![0u8; WIDTH as usize * HEIGHT as usize * 4];
    unsafe {
        pattern::fill_bgra(bytes.as_mut_ptr(), WIDTH as usize * 4, frame);
    }
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b))
        .collect()
}

/// `px` as (r, g, b) bytes.
const fn rgb(px: u32) -> (u8, u8, u8) {
    let [b, g, r, _] = px.to_le_bytes();
    (r, g, b)
}

/// Whether `px` is (near-)white.
const fn white(px: u32) -> bool {
    let (r, g, b) = rgb(px);
    r > 240 && g > 240 && b > 240
}

/// Whether `px` is near-black.
const fn dark(px: u32) -> bool {
    let (r, g, b) = rgb(px);
    r < 100 && g < 100 && b < 100
}

#[test]
fn quadrants_hold_the_named_hues() {
    let img = image(0);
    let at = |x: u32, y: u32| rgb(img[(y * WIDTH + x) as usize]);
    // The pattern's quadrants: red, green, blue, gray — bottom rows
    // sampled above the counter strip.
    let bottom = HEIGHT - STRIP - HEIGHT / 8;
    let (r, g, b) = at(WIDTH / 4, HEIGHT / 4);
    assert!(r > g + 60 && r > b + 60, "top-left red-ish: {r} {g} {b}");
    let (r, g, b) = at(WIDTH * 3 / 4, HEIGHT / 4);
    assert!(g > r + 60 && g > b + 60, "top-right green-ish: {r} {g} {b}");
    let (r, g, b) = at(WIDTH / 4, bottom);
    assert!(
        b > r + 60 && b > g + 60,
        "bottom-left blue-ish: {r} {g} {b}"
    );
    let (r, g, b) = at(WIDTH * 3 / 4, bottom);
    assert!(
        r.abs_diff(g) < 8 && g.abs_diff(b) < 8 && r > 60,
        "bottom-right gray: {r} {g} {b}"
    );
}

#[test]
fn every_pixel_is_opaque() {
    let img = image(0);
    assert!(img.iter().all(|px| px >> 24 == 0xFF));
}

#[test]
fn the_sweep_bar_moves_with_the_frame() {
    let first = image(0);
    let later = image(60);
    // Row 10 is inside the top half, where the bar sweeps.
    let bar_at = |img: &[u32]| -> usize {
        (0..WIDTH as usize)
            .find(|&x| white(img[10 * WIDTH as usize + x]))
            .expect("a bar on the row")
    };
    // The bar sweeps `frame * 8` pixels, wrapping across the width.
    assert_eq!(bar_at(&first), 0);
    let row = |img: &[u32], x: usize| img[10 * WIDTH as usize + x];
    assert!(white(row(&first, BAR as usize - 1)), "bar right edge");
    assert!(
        !white(row(&first, BAR as usize)),
        "the bar is BAR pixels wide"
    );
    assert_eq!(bar_at(&later), 480);
    assert_ne!(bar_at(&first), bar_at(&later));
}

#[test]
fn the_strip_carries_the_frame_counter() {
    // Frame 5 = 0b101: lit cells at cell 0 and cell 2.
    let img = image(5);
    let row = (HEIGHT - STRIP / 2) as usize;
    let cell = |n: usize| img[row * WIDTH as usize + n * CELL as usize + CELL as usize / 2];
    assert!(white(cell(0)), "bit 0 lit: {:#x}", cell(0));
    assert!(dark(cell(1)), "bit 1 dark: {:#x}", cell(1));
    assert!(white(cell(2)), "bit 2 lit: {:#x}", cell(2));
    assert!(dark(cell(3)), "bit 3 dark: {:#x}", cell(3));
    // A later frame's high bit lands in the same strip.
    let img = image(1 << 40);
    let cell40 = img[row * WIDTH as usize + 40 * CELL as usize + CELL as usize / 2];
    assert!(white(cell40), "bit 40 lit: {cell40:#x}");
}
