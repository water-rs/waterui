//! The procedural test picture, written through the plane layouts gralloc
//! reports — no media files, no assets.
//!
//! The picture is four colour quadrants, a white bar sweeping right and a
//! strip holding the frame counter in binary, so a torn or stale buffer
//! is visible immediately. Every write honours the plane's own
//! `rowStride`/`pixelStride` and `data` pointer, so the same code covers
//! planar 4:2:0 and the NV12-as-three-planes layout grallocs commonly
//! report for `Y8Cb8Cr8_420`.

use std::ops::Range;

/// Width of every video frame.
pub const WIDTH: u32 = 1920;
/// Height of every video frame.
pub const HEIGHT: u32 = 1080;

/// The sweep bar's width in pixels.
pub const BAR: u32 = 24;
/// The binary frame-counter strip's height in pixels.
pub const STRIP: u32 = 40;
/// A counter cell's width in pixels — one bit of the frame index.
pub const CELL: u32 = 24;

/// Chroma samples per row: one (Cb, Cr) pair per 2x2 luma block.
const CW: usize = (WIDTH / 2) as usize;
/// Chroma rows.
const CH: usize = (HEIGHT / 2) as usize;

/// One mapped plane region, as `AHardwareBuffer_lockPlanes` reports it.
pub struct Plane {
    /// The plane's first byte.
    pub data: *mut u8,
    /// Bytes from one row to the next.
    pub row_stride: usize,
    /// Bytes from one sample to the next within a row.
    pub pixel_stride: usize,
}

/// The quadrant colours as linear 0..1 RGB: top-left, top-right,
/// bottom-left, bottom-right.
const QUADRANTS: [(f64, f64, f64); 4] = [
    (0.85, 0.08, 0.08),
    (0.08, 0.70, 0.08),
    (0.08, 0.15, 0.85),
    (0.55, 0.55, 0.55),
];

/// BT.709 video-range `Y'CbCr` codes for `rgb` (0..1).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "clamped video-range codes; the textbook coefficient order stays readable"
)]
fn code709(rgb: (f64, f64, f64)) -> (u8, u8, u8) {
    let (r, g, b) = rgb;
    let clamp = |v: f64, lo: f64, hi: f64| -> u8 { v.clamp(lo, hi).round() as u8 };
    (
        clamp(16.0 + 65.481 * r + 128.553 * g + 24.966 * b, 16.0, 235.0),
        clamp(128.0 - 37.797 * r - 74.203 * g + 112.0 * b, 16.0, 240.0),
        clamp(128.0 + 112.0 * r - 93.786 * g - 18.214 * b, 16.0, 240.0),
    )
}

/// BT.2020 video-range `Y'CbCr` codes for `rgb` (0..1), 10-bit left-aligned
/// in u16 as P010 stores them.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "clamped video-range codes; the textbook coefficient order stays readable"
)]
fn code2020(rgb: (f64, f64, f64)) -> (u16, u16, u16) {
    let (r, g, b) = rgb;
    let yn = 0.2627 * r + 0.6780 * g + 0.0593 * b;
    let clamp = |v: f64, lo: f64, hi: f64| -> u16 { (v.clamp(lo, hi).round() as u16) << 6 };
    (
        clamp(64.0 + 876.0 * yn, 64.0, 940.0),
        clamp(
            512.0 + 896.0 * (b - yn) / (2.0 * (1.0 - 0.0593)),
            64.0,
            960.0,
        ),
        clamp(
            512.0 + 896.0 * (r - yn) / (2.0 * (1.0 - 0.2627)),
            64.0,
            960.0,
        ),
    )
}

/// The white bar's left edge on frame `frame`.
fn bar_x(frame: u64) -> usize {
    usize::try_from(frame * 8 % u64::from(WIDTH - BAR)).expect("the bar stays inside the frame")
}

/// The chroma runs one row carries: the two quadrant halves and the
/// neutral override under the bar and in the counter strip (covering the
/// whole row when it is a strip row).
struct ChromaRuns {
    left: (u8, u8),
    right: (u8, u8),
    neutral: Range<usize>,
}

fn chroma_runs(luma_row: usize, frame: u64, chroma: [(u8, u8); 4]) -> ChromaRuns {
    let strip = luma_row >= (HEIGHT - STRIP) as usize;
    if strip {
        return ChromaRuns {
            left: (128, 128),
            right: (128, 128),
            neutral: 0..CW,
        };
    }
    let top = luma_row < (HEIGHT / 2) as usize;
    let (left, right) = if top {
        (chroma[0], chroma[1])
    } else {
        (chroma[2], chroma[3])
    };
    let bar = bar_x(frame) / 2;
    ChromaRuns {
        left,
        right,
        neutral: bar..(bar + (BAR / 2) as usize),
    }
}

/// One chroma component's placement inside the mapped planes: `comp` 0 is
/// Cb, 1 is Cr.
#[derive(Clone, Copy)]
struct Channel {
    base: *mut u8,
    row_stride: usize,
    pixel_stride: usize,
    comp: usize,
}

/// Writes `value` over `cols` of `row` in `channel`, honouring the
/// channel's pixel stride.
///
/// # Safety
/// `channel.base` has `row_stride`-spaced writable rows covering `cols`.
unsafe fn channel_run(channel: Channel, row: usize, cols: Range<usize>, value: u8) {
    let row_base = unsafe { channel.base.add(row * channel.row_stride) };
    if channel.pixel_stride == 1 {
        unsafe { row_base.add(cols.start).write_bytes(value, cols.len()) };
        return;
    }
    for col in cols {
        unsafe { *row_base.add(col * channel.pixel_stride) = value };
    }
}

/// Writes one chroma row of `frame` into `channels`.
///
/// # Safety
/// As [`fill_nv12`].
#[expect(
    clippy::cast_ptr_alignment,
    reason = "an interleaved plane's byte-adjacent base is u16-aligned (gralloc rows are)"
)]
unsafe fn chroma_row8(channels: [Channel; 2], row: usize, frame: u64, chroma: [(u8, u8); 4]) {
    let runs = chroma_runs(row * 2, frame, chroma);
    // When the two components sit byte-adjacent at stride 2 (NV12,
    // however many lockPlanes entries describe it), one u16 write covers
    // the pair.
    let [cb, cr] = channels;
    if cb.pixel_stride == 2
        && cr.pixel_stride == 2
        && cb.row_stride == cr.row_stride
        && cb.base.wrapping_add(1) == cr.base
    {
        let pair_at = |row_base: *mut u16, cols: Range<usize>, cbv: u8, crv: u8| unsafe {
            std::slice::from_raw_parts_mut(row_base.add(cols.start), cols.len())
                .fill(u16::from_le_bytes([cbv, crv]));
        };
        let row_base = unsafe { cb.base.add(row * cb.row_stride) }.cast::<u16>();
        pair_at(row_base, 0..CW / 2, runs.left.0, runs.left.1);
        pair_at(row_base, CW / 2..CW, runs.right.0, runs.right.1);
        pair_at(row_base, runs.neutral, 128, 128);
        return;
    }
    for channel in channels {
        let value = |pair: (u8, u8)| <[u8; 2]>::from(pair)[channel.comp];
        unsafe {
            channel_run(channel, row, 0..CW / 2, value(runs.left));
            channel_run(channel, row, CW / 2..CW, value(runs.right));
            channel_run(channel, row, runs.neutral.clone(), 128);
        }
    }
}

/// Writes one luma row (u8 codes) of `frame` at `row` into `base`.
///
/// # Safety
/// `base` has room for `WIDTH` bytes.
unsafe fn luma_row8(base: *mut u8, row: usize, frame: u64, luma: [u8; 4]) {
    let width = WIDTH as usize;
    let strip = row >= (HEIGHT - STRIP) as usize;
    if strip {
        // Counter strip: `CELL`-wide cells, bit i of the frame index at
        // cell i — lit white when set.
        let mut x = 0usize;
        while x < width {
            let bit = frame >> ((x / CELL as usize) % 64) & 1;
            let cell = (CELL as usize).min(width - x);
            unsafe {
                base.add(x)
                    .write_bytes(if bit == 1 { 235 } else { 24 }, cell);
            };
            x += cell;
        }
        return;
    }
    let top = row < (HEIGHT / 2) as usize;
    let (left, right) = if top {
        (luma[0], luma[1])
    } else {
        (luma[2], luma[3])
    };
    unsafe {
        base.write_bytes(left, width / 2);
        base.add(width / 2).write_bytes(right, width / 2);
        let bar = bar_x(frame);
        base.add(bar).write_bytes(235, BAR as usize);
    }
}

/// The strip's luma row: `CELL`-wide cells, bit i of `frame` at cell i.
fn strip_row10(frame: u64) -> Vec<u16> {
    let width = WIDTH as usize;
    let mut row = vec![0u16; width];
    let mut x = 0usize;
    while x < width {
        let bit = frame >> ((x / CELL as usize) % 64) & 1;
        let cell = (CELL as usize).min(width - x);
        row[x..x + cell].fill(if bit == 1 { 940 << 6 } else { 80 << 6 });
        x += cell;
    }
    row
}

/// The pattern's colours as sRGB-encoded `BGRA` bytes packed in a u32
/// (little-endian: B in the low byte), for RGB-plane producers such as
/// Apple's `32BGRA` `IOSurface` path.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "clamped sRGB codes; the textbook encode order stays readable"
)]
fn bgra8(rgb: (f64, f64, f64)) -> u32 {
    let enc = |v: f64| -> u32 {
        let s = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        u32::from((s * 255.0).clamp(0.0, 255.0).round() as u8)
    };
    let (r, g, b) = rgb;
    enc(b) | enc(g) << 8 | enc(r) << 16 | 0xFF00_0000
}

/// Writes frame `frame` into a `32BGRA` buffer mapped at `base`.
///
/// The same quadrants, sweep bar and counter strip the YUV fills
/// produce, in sRGB so an sRGB-declared frame decodes to the same
/// picture. `row_bytes` is the buffer's byte stride (`bytesPerRow`),
/// which the write honours exactly; a row occupies `4 * WIDTH` bytes
/// of it.
///
/// # Safety
/// `base` is a writable mapping of at least `HEIGHT` rows of
/// `row_bytes`, aligned to 4 bytes (any `IOSurface` or `CVPixelBuffer`
/// base is).
#[expect(
    clippy::cast_ptr_alignment,
    reason = "a CVPixelBuffer base is 64-byte aligned; BGRA rows are 4-byte aligned"
)]
pub unsafe fn fill_bgra(base: *mut u8, row_bytes: usize, frame: u64) {
    let width = WIDTH as usize;
    let quadrants = QUADRANTS.map(bgra8);
    let (mut top, mut bottom) = (vec![quadrants[0]; width], vec![quadrants[2]; width]);
    top[width / 2..].fill(quadrants[1]);
    bottom[width / 2..].fill(quadrants[3]);
    let mut strip = vec![0u32; width];
    let (lit, dark) = (bgra8((1.0, 1.0, 1.0)), bgra8((0.08, 0.08, 0.08)));
    let mut x = 0usize;
    while x < width {
        let bit = frame >> ((x / CELL as usize) % 64) & 1;
        let cell = (CELL as usize).min(width - x);
        strip[x..x + cell].fill(if bit == 1 { lit } else { dark });
        x += cell;
    }
    let bar = bar_x(frame);
    for row in 0..HEIGHT as usize {
        // SAFETY: the caller guarantees `row_bytes` per row; a row needs
        // `4 * WIDTH` of it.
        let dst = unsafe {
            std::slice::from_raw_parts_mut(base.add(row * row_bytes).cast::<u32>(), width)
        };
        let (src, patch_bar) = if row >= (HEIGHT - STRIP) as usize {
            (strip.as_slice(), false)
        } else if row < (HEIGHT / 2) as usize {
            (top.as_slice(), true)
        } else {
            (bottom.as_slice(), true)
        };
        dst.copy_from_slice(src);
        if patch_bar {
            dst[bar..bar + BAR as usize].fill(lit);
        }
    }
}

/// `pair` interleaved over `dst` — one P010 (Cb, Cr) run.
fn interleave10(dst: &mut [u16], pair: (u16, u16)) {
    let (samples, _) = dst.as_chunks_mut::<2>();
    for sample in samples {
        *sample = pair.into();
    }
}

/// Writes frame `frame` into a `Y8Cb8Cr8_420` buffer mapped as `planes`.
///
/// Covers both the semi-planar (NV12, 2 planes) and tri-planar (3
/// planes) layouts the flexible format resolves to. Chroma lands where a
/// reader honouring `data`/`rowStride`/`pixelStride` finds it —
/// including the NV12-as-three-planes report where planes 1 and 2 are
/// stride-2 views into one interleaved region.
///
/// # Safety
/// Every entry of `planes` is a writable mapping of the buffer's plane.
///
/// # Panics
/// On a plane count that is neither 2 nor 3.
pub unsafe fn fill_nv12(planes: &[Plane], frame: u64) {
    assert!(
        planes.len() == 2 || planes.len() == 3,
        "Y8Cb8Cr8_420 resolved to {} planes",
        planes.len()
    );
    let luma: [u8; 4] = QUADRANTS.map(|rgb| code709(rgb).0);
    let chroma: [(u8, u8); 4] = QUADRANTS.map(|rgb| {
        let (_, cb, cr) = code709(rgb);
        (cb, cr)
    });
    let y = &planes[0];
    unsafe {
        let y_base = y.data;
        for row in 0..HEIGHT as usize {
            luma_row8(y_base.add(row * y.row_stride), row, frame, luma);
        }
    }
    // The two chroma components as channels. Two planes carry the UV
    // pairs at `pixelStride` on one plane; three planes carry one
    // component each — stride-2 views into the same interleaved bytes on
    // an NV12 gralloc, real planes on an I420 one.
    let channels: [Channel; 2] = if planes.len() == 2 {
        let uv = &planes[1];
        [
            Channel {
                base: uv.data,
                row_stride: uv.row_stride,
                pixel_stride: uv.pixel_stride,
                comp: 0,
            },
            Channel {
                base: unsafe { uv.data.add(1) },
                row_stride: uv.row_stride,
                pixel_stride: uv.pixel_stride,
                comp: 1,
            },
        ]
    } else {
        [
            Channel {
                base: planes[1].data,
                row_stride: planes[1].row_stride,
                pixel_stride: planes[1].pixel_stride,
                comp: 0,
            },
            Channel {
                base: planes[2].data,
                row_stride: planes[2].row_stride,
                pixel_stride: planes[2].pixel_stride,
                comp: 1,
            },
        ]
    };
    for row in 0..CH {
        unsafe { chroma_row8(channels, row, frame, chroma) };
    }
}

/// Writes frame `frame` into a `YCbCr_P010` buffer mapped at `base`.
///
/// `stride` is the luma samples per row. P010 is a fixed format, so the
/// layout is the semiplanar one every P010 gralloc produces — a u16 luma
/// plane followed by an interleaved (Cb, Cr) u16 plane.
///
/// # Safety
/// `base` is a writable mapping of the buffer's `stride * HEIGHT` u16
/// luma and chroma plane.
#[expect(
    clippy::cast_ptr_alignment,
    reason = "gralloc's mapped address is page-aligned and the UV offset is even"
)]
pub unsafe fn fill_p010(base: *mut u8, stride: usize, frame: u64) {
    let luma: [u16; 4] = QUADRANTS.map(|rgb| code2020(rgb).0);
    let chroma: [(u16, u16); 4] = QUADRANTS.map(|rgb| {
        let (_, cb, cr) = code2020(rgb);
        (cb, cr)
    });
    let width = WIDTH as usize;
    // The frame is a handful of row patterns repeated hundreds of times:
    // build each once, then copy one row at a time into the mapped
    // buffer and patch the moving bar's run — memcpy instead of
    // per-sample writes.
    let (mut y_top, mut y_bot) = (vec![luma[0]; width], vec![luma[2]; width]);
    y_top[width / 2..].fill(luma[1]);
    y_bot[width / 2..].fill(luma[3]);
    let y_strip = strip_row10(frame);
    let (mut uv_top, mut uv_bot) = (vec![0u16; CW * 2], vec![0u16; CW * 2]);
    interleave10(&mut uv_top[..CW], chroma[0]);
    interleave10(&mut uv_top[CW..], chroma[1]);
    interleave10(&mut uv_bot[..CW], chroma[2]);
    interleave10(&mut uv_bot[CW..], chroma[3]);
    let bar = bar_x(frame);
    let bar_cols = (bar / 2)..(bar / 2 + (BAR / 2) as usize);
    unsafe {
        let y_base = base.cast::<u16>();
        for row in 0..HEIGHT as usize {
            let dst = std::slice::from_raw_parts_mut(y_base.add(row * stride), width);
            let (src, patch_bar) = if row >= (HEIGHT - STRIP) as usize {
                (y_strip.as_slice(), false)
            } else if row < (HEIGHT / 2) as usize {
                (y_top.as_slice(), true)
            } else {
                (y_bot.as_slice(), true)
            };
            dst.copy_from_slice(src);
            if patch_bar {
                dst[bar..bar + BAR as usize].fill(940 << 6);
            }
        }
        // The interleaved chroma plane follows the luma plane.
        let uv_base = base.add(stride * 2 * HEIGHT as usize).cast::<u16>();
        for row in 0..CH {
            let luma_row = row * 2;
            let dst = std::slice::from_raw_parts_mut(uv_base.add(row * stride), CW * 2);
            if luma_row >= (HEIGHT - STRIP) as usize {
                // Both codes equal — one flat run.
                dst.fill(512 << 6);
                continue;
            }
            dst.copy_from_slice(if luma_row < (HEIGHT / 2) as usize {
                uv_top.as_slice()
            } else {
                uv_bot.as_slice()
            });
            dst[bar_cols.start * 2..bar_cols.end * 2].fill(512 << 6);
        }
    }
}
