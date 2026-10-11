//! `fill_nv12` writes the pattern where a reader honouring gralloc's
//! `lockPlanes` layout finds it — the regression this guards is the
//! contiguous chroma write that an NV12-as-three-planes report (stride-2
//! Cb/Cr views into one interleaved region) scattered across the frame.

use android_planes::pattern::{self, HEIGHT, Plane, WIDTH};

/// The quadrant chroma codes `code709` produces: (Cb, Cr) for
/// top-left, top-right, bottom-left, bottom-right.
const CHROMA: [(u8, u8); 4] = [(99, 214), (82, 70), (209, 107), (128, 128)];

/// One mapped 1920x1080 4:2:0 buffer: tight luma rows then a tight UV
/// region, the layout every gralloc agrees on at this size.
struct Mapped {
    bytes: Vec<u8>,
    /// Luma region bytes.
    luma_len: usize,
}

impl Mapped {
    /// A zeroed mapping with `uv_stride` bytes per chroma row.
    fn new(y_stride: usize, uv_stride: usize) -> Self {
        let luma_len = y_stride * HEIGHT as usize;
        let uv_len = uv_stride * (HEIGHT / 2) as usize;
        Self {
            bytes: vec![0; luma_len + uv_len],
            luma_len,
        }
    }

    fn uv_at(&self, row: usize, byte: usize, uv_stride: usize) -> u8 {
        self.bytes[self.luma_len + row * uv_stride + byte]
    }

    /// Reads the (Cb, Cr) pair a semi-planar reader sees at chroma
    /// position `(row, col)` of an `uv_stride`-row region.
    fn pair_at(&self, row: usize, col: usize, uv_stride: usize) -> (u8, u8) {
        (
            self.uv_at(row, col * 2, uv_stride),
            self.uv_at(row, col * 2 + 1, uv_stride),
        )
    }

    /// Asserts frame 0's quadrants at the corners of each quadrant.
    /// Rows are chroma rows — half the luma row.
    fn assert_quadrants(&self, uv_stride: usize) {
        let mid_row = HEIGHT as usize / 8;
        let low_row = HEIGHT as usize / 8 * 3;
        for (row, (l, r)) in [(mid_row, (0, 1)), (low_row, (2, 3))] {
            // Past the sweep bar (12 chroma columns on frame 0).
            assert_eq!(
                self.pair_at(row, 20, uv_stride),
                CHROMA[l],
                "row {row} left"
            );
            assert_eq!(
                self.pair_at(row, WIDTH as usize / 2 - 20, uv_stride),
                CHROMA[r],
                "row {row} right"
            );
        }
        // The bar overrides its columns with neutral chroma.
        assert_eq!(self.pair_at(mid_row, 5, uv_stride), (128, 128), "bar");
        // The counter strip is neutral (chroma rows under the strip's
        // luma rows).
        let strip = (HEIGHT as usize - 40) / 2;
        assert_eq!(self.pair_at(strip, 20, uv_stride), (128, 128), "strip");
    }
}

/// Semi-planar NV12 reported as two planes: one interleaved UV plane.
#[test]
fn nv12_two_planes() {
    let mut map = Mapped::new(WIDTH as usize, WIDTH as usize);
    unsafe {
        pattern::fill_nv12(
            &[
                Plane {
                    data: map.bytes.as_mut_ptr(),
                    row_stride: WIDTH as usize,
                    pixel_stride: 1,
                },
                Plane {
                    data: map.bytes.as_mut_ptr().add(map.luma_len),
                    row_stride: WIDTH as usize,
                    pixel_stride: 2,
                },
            ],
            0,
        );
    }
    map.assert_quadrants(WIDTH as usize);
}

/// NV12 reported as three planes: stride-2 Cb and Cr views into one
/// interleaved region, one byte apart — what Pixel's gralloc reports for
/// `Y8Cb8Cr8_420`.
#[test]
fn nv12_as_three_planes() {
    let mut map = Mapped::new(WIDTH as usize, WIDTH as usize);
    let uv = unsafe { map.bytes.as_mut_ptr().add(map.luma_len) };
    unsafe {
        pattern::fill_nv12(
            &[
                Plane {
                    data: map.bytes.as_mut_ptr(),
                    row_stride: WIDTH as usize,
                    pixel_stride: 1,
                },
                Plane {
                    data: uv,
                    row_stride: WIDTH as usize,
                    pixel_stride: 2,
                },
                Plane {
                    data: uv.add(1),
                    row_stride: WIDTH as usize,
                    pixel_stride: 2,
                },
            ],
            0,
        );
    }
    map.assert_quadrants(WIDTH as usize);
}

/// Tri-planar 4:2:0: separate tight Cb and Cr planes.
#[test]
fn planar_i420() {
    let uv_stride = WIDTH as usize / 2;
    let mut map = Mapped::new(WIDTH as usize, uv_stride * 2);
    // Cb plane then Cr plane, each `uv_stride` per row, packed.
    let cb = unsafe { map.bytes.as_mut_ptr().add(map.luma_len) };
    let cr = unsafe { cb.add(uv_stride * HEIGHT as usize / 2) };
    unsafe {
        pattern::fill_nv12(
            &[
                Plane {
                    data: map.bytes.as_mut_ptr(),
                    row_stride: WIDTH as usize,
                    pixel_stride: 1,
                },
                Plane {
                    data: cb,
                    row_stride: uv_stride,
                    pixel_stride: 1,
                },
                Plane {
                    data: cr,
                    row_stride: uv_stride,
                    pixel_stride: 1,
                },
            ],
            0,
        );
    }
    let mid_row = HEIGHT as usize / 8;
    let low_row = HEIGHT as usize / 8 * 3;
    let at = |row: usize, col: usize, plane: usize| {
        map.bytes[map.luma_len + plane * uv_stride * HEIGHT as usize / 2 + row * uv_stride + col]
    };
    for (row, (l, r)) in [(mid_row, (0, 1)), (low_row, (2, 3))] {
        assert_eq!(at(row, 20, 0), CHROMA[l].0, "Cb row {row} left");
        assert_eq!(at(row, 20, 1), CHROMA[l].1, "Cr row {row} left");
        assert_eq!(
            at(row, WIDTH as usize / 2 - 20, 0),
            CHROMA[r].0,
            "Cb row {row} right"
        );
        assert_eq!(
            at(row, WIDTH as usize / 2 - 20, 1),
            CHROMA[r].1,
            "Cr row {row} right"
        );
    }
}
