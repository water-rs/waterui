//! Host wall-clock estimate of `fill_p010`'s per-frame cost — the
//! producer-bound number the `hdr` heartbeat's `fill=` reports. Host
//! timing only: the mapped buffer here is ordinary RAM, not the
//! gralloc-mapped AHB the Pixel writes.

use std::time::Instant;

use android_planes::pattern::{self, HEIGHT, WIDTH};

const FRAMES: u64 = 240;

#[test]
fn p010_fill_cost() {
    let stride = WIDTH as usize;
    let mut mapped = vec![0u8; stride * 2 * (HEIGHT as usize + HEIGHT as usize / 2)];
    // Warm up: first frame also pays the row-pattern allocations.
    // SAFETY: `mapped` is a writable `stride * 2 * (HEIGHT + HEIGHT / 2)`
    // byte buffer, covering the P010 luma and chroma planes.
    unsafe { pattern::fill_p010(mapped.as_mut_ptr(), stride, 0) };
    let start = Instant::now();
    for frame in 1..=FRAMES {
        // SAFETY: as above.
        unsafe { pattern::fill_p010(mapped.as_mut_ptr(), stride, frame) };
        std::hint::black_box(mapped[0]);
    }
    let per_frame = start.elapsed() / u32::try_from(FRAMES).unwrap();
    eprintln!("fill_p010: {per_frame:?}/frame over {FRAMES} frames (host estimate)");
}
