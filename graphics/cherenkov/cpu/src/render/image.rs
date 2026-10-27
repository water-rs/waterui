//! Registered CPU images, shared by retained paint operands. A replacement
//! of the same dimensions decodes over the pixels once the operands that
//! shared them are discarded.
//!
//! The image paint's span kernel evaluates sixteen lanes of device
//! columns at a time through the device→image map, the per-axis extend
//! modes and the sampling — the same expressions the scalar path
//! evaluates per pixel, so a lane never depends on its position in the
//! span and agrees with `PaintData::eval` to within the level's
//! rounding. The texel index math runs in u32 lanes — nearest's
//! `floor(d + 0.49999997)` is `f32::round` exactly, and bilinear's
//! clamped coordinate, neighbour bound and three-`mul_add` mix are
//! lane-wise — while the gathers stage each texel's sixteen bytes and
//! deinterleave them through the block store's inverse.
//!
//! The lane helpers, extend modes, stores and the block driver are
//! `super::span`'s — the kernel plugs in through [`SpanKernel`] — and
//! the same rules apply: `#[inline(always)]` on the kernel path and no
//! closures there.

use std::mem::MaybeUninit;

use cherenkov::{
    Extend, ImageColorSpace, ImageFormat, ImageUpload, ResourceError, Sampling, srgb_decode,
};
use fearless_simd::{Select, Simd, SimdBase, SimdFloat, SimdInterleaved, f32x4, f32x16, u32x16};

use super::paint::{ImagePaintData, PaintData};
use super::span::{
    ExtendSpan, NoneSpan, PadSpan, ReflectSpan, RepeatSpan, SpanKernel, apply_span, clamp_span,
    extend_t_span,
};

/// A registered image decoded into working-space premultiplied texels.
#[derive(Debug)]
pub struct CpuImage {
    /// Texel columns.
    pub width: u32,
    /// Texel rows.
    pub height: u32,
    /// `width * height` premultiplied linear Display P3 texels in row order.
    pub pixels: Box<[[f32; 4]]>,
}

impl CpuImage {
    /// The decoded texels' heap bytes.
    ///
    /// # Panics
    /// When the texel count does not fit `u64` — unreachable on any
    /// supported target.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        u64::try_from(size_of_val(&*self.pixels)).expect("image allocation fits u64")
    }

    /// Decode once on registration. Alpha is unpremultiplied in the encoded
    /// domain before transfer/primary conversion, then premultiplied in P3.
    /// `Rgba16F` texels convert f16 -> f64 directly — never through 8 bits —
    /// and keep values outside `[0, 1]`.
    ///
    /// # Errors
    /// [`ResourceError::Image`] when `image` is empty, in a format the CPU
    /// cannot decode, or its byte length disagrees with its dimensions.
    pub fn decode(image: &ImageUpload) -> Result<Self, ResourceError> {
        let count = Self::validate(image)?;
        let mut pixels = vec![[0.0; 4]; count].into_boxed_slice();
        convert(image, &mut pixels);
        Ok(Self {
            width: image.width,
            height: image.height,
            pixels,
        })
    }

    /// Decodes a validated `image` of this image's dimensions over the
    /// existing pixels, reusing their storage.
    pub fn overwrite(&mut self, image: &ImageUpload) {
        convert(image, &mut self.pixels);
    }

    /// Checks that `image` can be decoded: nonempty, a CPU-decodable
    /// format, and a byte length matching its dimensions. Returns its
    /// texel count.
    ///
    /// # Errors
    /// [`ResourceError::Image`] when `image` is empty, in a format the CPU
    /// cannot decode, or its byte length disagrees with its dimensions.
    pub fn validate(image: &ImageUpload) -> Result<usize, ResourceError> {
        if image.width == 0 || image.height == 0 {
            return Err(ResourceError::Image(
                "CPU uploads require nonempty images".into(),
            ));
        }
        let bytes_per_texel = match image.format {
            ImageFormat::Rgba8 => 4,
            ImageFormat::Rgba16F => 8,
            format => {
                return Err(ResourceError::Image(format!(
                    "unsupported image format {format:?}"
                )));
            }
        };
        usize::try_from(image.width)
            .ok()
            .zip(usize::try_from(image.height).ok())
            .and_then(|(width, height)| width.checked_mul(height))
            .filter(|count| count.checked_mul(bytes_per_texel) == Some(image.data.len()))
            .ok_or_else(|| ResourceError::Image("image dimensions and byte length disagree".into()))
    }
}

/// Decodes a validated `image` into `pixels`, one working-space texel per
/// encoded texel.
#[expect(
    clippy::cast_possible_truncation,
    reason = "decoded working pixels use f32"
)]
fn convert(image: &ImageUpload, pixels: &mut [[f32; 4]]) {
    // u8 data can be premultiplied-encodable only up to 1.0; f16 texels
    // keep extended values, so their un-premultiply is unbounded.
    let unpremul_max = match image.format {
        ImageFormat::Rgba8 => 1.0,
        _ => f64::INFINITY,
    };
    let texel = |encoded: [f64; 4]| {
        let alpha = encoded[3];
        if alpha == 0.0 {
            return [0.0; 4];
        }
        let linear = std::array::from_fn(|channel| {
            let straight = if image.premultiplied {
                (encoded[channel] / alpha).min(unpremul_max)
            } else {
                encoded[channel]
            };
            if image.color_space == ImageColorSpace::LinearSrgb
                || image.color_space == ImageColorSpace::LinearP3
            {
                straight
            } else {
                srgb_decode(straight)
            }
        });
        let working = if image.color_space == ImageColorSpace::DisplayP3
            || image.color_space == ImageColorSpace::LinearP3
        {
            linear
        } else {
            mul(&XYZ_TO_P3, mul(&SRGB_TO_XYZ, linear))
        };
        [
            (working[0] * alpha) as f32,
            (working[1] * alpha) as f32,
            (working[2] * alpha) as f32,
            alpha as f32,
        ]
    };
    match image.format {
        ImageFormat::Rgba8 => {
            for (out, pixel) in pixels.iter_mut().zip(image.data.as_chunks::<4>().0) {
                *out = texel(pixel.map(|v| f64::from(v) / 255.0));
            }
        }
        ImageFormat::Rgba16F => {
            for (out, pixel) in pixels.iter_mut().zip(image.data.as_chunks::<8>().0) {
                *out = texel(std::array::from_fn(|channel| {
                    f64::from(half::f16::from_le_bytes([
                        pixel[2 * channel],
                        pixel[2 * channel + 1],
                    ]))
                }));
            }
        }
        _ => unreachable!("format validated before conversion"),
    }
}

const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_390_799_265_959_5,
        0.357_584_339_383_878,
        0.180_480_788_401_834_3,
    ],
    [
        0.212_639_005_871_510_4,
        0.715_168_678_767_756,
        0.072_192_315_360_733_7,
    ],
    [
        0.019_330_818_715_591_9,
        0.119_194_779_794_626,
        0.950_532_152_249_660_7,
    ],
];
const XYZ_TO_P3: [[f64; 3]; 3] = [
    [
        2.493_496_911_941_425,
        -0.931_383_617_919_123_9,
        -0.402_710_784_450_716_8,
    ],
    [
        -0.829_488_969_561_574_7,
        1.762_664_060_318_346_3,
        0.023_624_685_841_943_6,
    ],
    [
        0.035_845_830_243_784_5,
        -0.076_172_389_268_041_8,
        0.956_884_524_007_687_2,
    ],
];
fn mul(matrix: &[[f64; 3]; 3], value: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row[2].mul_add(value[2], row[1].mul_add(value[1], row[0] * value[0])))
}

/// The span-invariant image evaluation, decided once per span: the
/// paint the `Transformed` chain walk starts from (lowering folds an
/// image pattern's transform into the mapping, so the walk finds no
/// links today, as in `super::gradient::linear_span`), the
/// device→image-pixel map, the per-axis extend modes, the sampling and
/// the image dimensions as f32.
#[derive(Clone, Copy)]
struct ImageEval<'a> {
    /// The paint the chain walk starts from.
    outer: &'a PaintData,
    /// Device-to-image-pixel transform.
    inv: [f32; 6],
    /// The registered image's premultiplied texels.
    img: &'a CpuImage,
    /// Horizontal continuation.
    extend_x: Extend,
    /// Vertical continuation.
    extend_y: Extend,
    /// Sampling.
    sampling: Sampling,
    /// `img.width` as f32.
    iw: f32,
    /// `img.height` as f32.
    ih: f32,
}

impl<'a> ImageEval<'a> {
    /// Copies the span's invariants out of the paint data once.
    #[expect(
        clippy::cast_precision_loss,
        reason = "image dimensions are bounded by memory budget"
    )]
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level"
    )]
    #[inline(always)]
    fn new(outer: &'a PaintData, params: &'a ImagePaintData) -> Self {
        Self {
            outer,
            inv: params.inv,
            img: &params.image,
            extend_x: params.extend_x,
            extend_y: params.extend_y,
            sampling: params.sampling,
            iw: params.image.width as f32,
            ih: params.image.height as f32,
        }
    }
}

/// Nearest's texel index on one axis: `(d − 0.5)` clamped into the
/// texel-centre domain `[0, dim − 1]`, then `f32::round`. The clamp's
/// integer bounds let it commute with the round, and on the clamped
/// non-negative coordinate `floor(d + 0.49999997)` is `round` exactly —
/// the level's own lowering — so the scalar path and the block lanes
/// compute the same index.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "texel coordinates are clamped before indexing; image dimensions are bounded by memory budget"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
pub(super) fn nearest_texel(d: f32, dim: usize) -> usize {
    ((d - 0.5).clamp(0.0, dim as f32 - 1.0) + 0.499_999_97).floor() as usize
}

/// Bilinear's tap pair on one axis: the clamped texel-centre
/// coordinate's lower texel, the right/top neighbour bound to the
/// edge, and the blend fraction — `sample_image`'s scalar expression,
/// the same the block lanes compute in vector.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "texel coordinates are clamped before indexing; image dimensions are bounded by memory budget"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
pub(super) fn linear_taps(d: f32, dim: usize) -> (usize, usize, f32) {
    let f = (d - 0.5).clamp(0.0, dim as f32 - 1.0);
    let lo = f.floor() as usize;
    (lo, (lo + 1).min(dim - 1), f - lo as f32)
}

/// The inverse of `super::span`'s `block_group`/`store_block`: the
/// `stage` pixels' `[r, g, b, a]` quarters deinterleaved into
/// per-channel vectors, one 128-bit group load per four lanes.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn deinterleave_block<S: Simd>(simd: S, stage: &[f32; 64]) -> [f32x16<S>; 4] {
    let mut quarters = [[simd.splat_f32x4(0.0); 4]; 4];
    for (k, group) in stage.as_chunks::<16>().0.iter().enumerate() {
        quarters[k] = f32x4::load_four_interleaved(simd, group);
    }
    let mut px = [simd.splat_f32x16(0.0); 4];
    for (ch, out) in px.iter_mut().enumerate() {
        *out = simd.combine_f32x8(
            simd.combine_f32x4(quarters[0][ch], quarters[1][ch]),
            simd.combine_f32x4(quarters[2][ch], quarters[3][ch]),
        );
    }
    px
}

/// `img.pixels[idx]` per lane into per-channel vectors — the block's
/// four taps for [`taps_linear_block`], its one for
/// [`taps_nearest_block`]: each texel's sixteen bytes staged, then
/// [`deinterleave_block`].
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn gather_block<S: Simd>(simd: S, img: &CpuImage, idx: u32x16<S>) -> [f32x16<S>; 4] {
    let mut stage = [0.0f32; 64];
    for (slot, i) in stage.as_chunks_mut::<4>().0.iter_mut().zip(idx.to_array()) {
        *slot = img.pixels[i as usize];
    }
    deinterleave_block(simd, &stage)
}

/// `sample_image`'s nearest arm lane-wise: [`nearest_texel`]'s index
/// math in vector — the clamped coordinate's `floor(d + 0.49999997)`
/// and `y·w + x` in u32 lanes — with the gather the only per-lane work.
#[expect(
    clippy::cast_precision_loss,
    reason = "image dimensions are bounded by memory budget"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn taps_nearest_block<S: Simd>(
    simd: S,
    img: &CpuImage,
    u: f32x16<S>,
    v: f32x16<S>,
) -> [f32x16<S>; 4] {
    let half = simd.splat_f32x16(0.5);
    let round = simd.splat_f32x16(0.499_999_97);
    let fx = (clamp_span(simd, u - half, 0.0, img.width as f32 - 1.0) + round).floor();
    let fy = (clamp_span(simd, v - half, 0.0, img.height as f32 - 1.0) + round).floor();
    let xs = simd.cvt_u32_f32x16(fx);
    let ys = simd.cvt_u32_f32x16(fy);
    gather_block(
        simd,
        img,
        simd.add_u32x16(simd.mul_u32x16(ys, simd.splat_u32x16(img.width)), xs),
    )
}

/// `sample_image`'s bilinear arm lane-wise: [`linear_taps`]' index
/// math in vector — the clamped texel-centre coordinate, its
/// `floor`/`fract` split and the `min(x0 + 1, w − 1)` neighbour bound —
/// then the scalar sampler's three `mul_add` lerps per channel over
/// four per-lane gathers.
#[expect(
    clippy::cast_precision_loss,
    reason = "image dimensions are bounded by memory budget"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn taps_linear_block<S: Simd>(
    simd: S,
    img: &CpuImage,
    u: f32x16<S>,
    v: f32x16<S>,
) -> [f32x16<S>; 4] {
    let half = simd.splat_f32x16(0.5);
    // Clamp the *sample coordinate* into texel-centre space before
    // taking the fraction, as the scalar sampler does.
    let fx = clamp_span(simd, u - half, 0.0, img.width as f32 - 1.0);
    let fy = clamp_span(simd, v - half, 0.0, img.height as f32 - 1.0);
    let (xf, yf) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - xf, fy - yf);
    let one = simd.splat_u32x16(1);
    let x0 = simd.cvt_u32_f32x16(xf);
    let y0 = simd.cvt_u32_f32x16(yf);
    let x1 = simd.min_u32x16(simd.add_u32x16(x0, one), simd.splat_u32x16(img.width - 1));
    let y1 = simd.min_u32x16(simd.add_u32x16(y0, one), simd.splat_u32x16(img.height - 1));
    let wv = simd.splat_u32x16(img.width);
    // One gather body for the four taps: a loop keeps a single inlined
    // copy of the staging and deinterleave temporaries, which in a
    // debug build all live in the caller's frame.
    let mut taps = [[simd.splat_f32x16(0.0); 4]; 4];
    for (tap, (y, x)) in taps
        .iter_mut()
        .zip([(y0, x0), (y0, x1), (y1, x0), (y1, x1)])
    {
        *tap = gather_block(simd, img, simd.add_u32x16(simd.mul_u32x16(y, wv), x));
    }
    let [c00, c10, c01, c11] = taps;
    let mut px = [simd.splat_f32x16(0.0); 4];
    for ch in 0..4 {
        let top = tx.mul_add(c10[ch] - c00[ch], c00[ch]);
        let bot = tx.mul_add(c11[ch] - c01[ch], c01[ch]);
        px[ch] = ty.mul_add(bot - top, top);
    }
    px
}

/// `extend_t` on one lane for the one-pixel path: the mode's scalar
/// [`ExtendSpan::lane`], so a lone pixel gets the block lane's
/// rounding.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn extend_lane<S: Simd>(simd: S, t: f32, extend: Extend) -> (f32, bool) {
    match extend {
        Extend::Pad => PadSpan::lane(simd, t),
        Extend::Repeat => RepeatSpan::lane(simd, t),
        Extend::Reflect => ReflectSpan::lane(simd, t),
        Extend::None => NoneSpan::lane(simd, t),
    }
}

/// `sample_image` on one pixel in channel-vector form: the shared
/// scalar index math and the bilinear mix through the level's
/// `mul_add`, the same arithmetic the block lanes get.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn sample_pixel<S: Simd>(simd: S, img: &CpuImage, u: f32, v: f32, sampling: Sampling) -> f32x4<S> {
    let (w, h) = (img.width as usize, img.height as usize);
    match sampling {
        Sampling::Nearest => f32x4::load_array(
            simd,
            img.pixels[nearest_texel(v, h) * w + nearest_texel(u, w)],
        ),
        Sampling::Linear => {
            let (x0, x1, fx) = linear_taps(u, w);
            let (y0, y1, fy) = linear_taps(v, h);
            let tx = simd.splat_f32x4(fx);
            let ty = simd.splat_f32x4(fy);
            let c00 = f32x4::load_array(simd, img.pixels[y0 * w + x0]);
            let c10 = f32x4::load_array(simd, img.pixels[y0 * w + x1]);
            let c01 = f32x4::load_array(simd, img.pixels[y1 * w + x0]);
            let c11 = f32x4::load_array(simd, img.pixels[y1 * w + x1]);
            let top = tx.mul_add(c10 - c00, c00);
            let bot = tx.mul_add(c11 - c01, c01);
            ty.mul_add(bot - top, top)
        }
    }
}

/// The image kernel [`super::span::span`] drives: `block` maps the
/// sixteen lanes into image space and gathers taps; `run` is the
/// one-pixel path — `eval_pixel`'s channel-vector form of the same
/// expression.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
// SAFETY: run writes every output pixel before returning.
unsafe impl<'a, S: Simd> SpanKernel<'a, S> for ImageEval<'a> {
    /// One `LANES`-wide block from device x-centres `xs` on row `dy`'s
    /// centre: `outer`'s chain inverses and `inv` map each lane into
    /// image pixel space, the per-axis extend modes decide the covered
    /// lanes — `Extend::None` lanes outside `[0, 1]` come out
    /// transparent — and the sampling gathers premultiplied taps.
    #[inline(always)]
    fn block(&self, simd: S, xs: f32x16<S>, dy: f32x16<S>) -> [f32x16<S>; 4] {
        let (mut cx, mut cy) = (xs, dy);
        let mut link = self.outer;
        while let PaintData::Transformed(inner) = link {
            (cx, cy) = apply_span(simd, inner.inverse_f32, cx, cy);
            link = &inner.paint;
        }
        let (cx, cy) = apply_span(simd, self.inv, cx, cy);
        let iw = simd.splat_f32x16(self.iw);
        let ih = simd.splat_f32x16(self.ih);
        let (tx, in_x) = extend_t_span(simd, cx / iw, self.extend_x);
        let (ty, in_y) = extend_t_span(simd, cy / ih, self.extend_y);
        let (u, v) = (tx * iw, ty * ih);
        let mut px = match self.sampling {
            Sampling::Nearest => taps_nearest_block(simd, self.img, u, v),
            Sampling::Linear => taps_linear_block(simd, self.img, u, v),
        };
        let zero = simd.splat_f32x16(0.0);
        let active = in_x & in_y;
        for c in &mut px {
            *c = active.select(*c, zero);
        }
        px
    }

    /// The one-pixel path over a whole run: the `Transformed` test is
    /// decided once, outside the loop, so the common leaf case carries
    /// no chain walk.
    #[inline(always)]
    fn run(&self, simd: S, x: f32, y: f32, out: &mut [MaybeUninit<[f32; 4]>]) {
        if matches!(self.outer, PaintData::Transformed(..)) {
            eval_pixels::<S, true>(simd, *self, x, y, out);
        } else {
            eval_pixels::<S, false>(simd, *self, x, y, out);
        }
    }
}

/// One pixel of the block expression in channel-vector form: `outer`'s
/// chain — present only in the `CHAINED` instantiation — and `inv`
/// applied with the level's own `mul_add` on lane 0 of an `f32x4`, the
/// same arithmetic, fused or not, the block lanes get; the scalar
/// extends and index math, and the level's `mul_add` for the bilinear
/// mix. Zero where either `Extend::None` leaves the domain.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_pixel<S: Simd, const CHAINED: bool>(
    simd: S,
    eval: ImageEval<'_>,
    x: f32,
    y: f32,
) -> f32x4<S> {
    let (mut cx, mut cy) = (simd.splat_f32x4(x), simd.splat_f32x4(y));
    if CHAINED {
        let mut link = eval.outer;
        while let PaintData::Transformed(inner) = link {
            (cx, cy) = apply_span(simd, inner.inverse_f32, cx, cy);
            link = &inner.paint;
        }
    }
    let (cx, cy) = apply_span(simd, eval.inv, cx, cy);
    let (tx, in_x) = extend_lane(simd, cx.to_array()[0] / eval.iw, eval.extend_x);
    let (ty, in_y) = extend_lane(simd, cy.to_array()[0] / eval.ih, eval.extend_y);
    if !(in_x && in_y) {
        return simd.splat_f32x4(0.0);
    }
    sample_pixel(simd, eval.img, tx * eval.iw, ty * eval.ih, eval.sampling)
}

/// `eval_pixel` over a run of `out`, `x` stepping the device pixel
/// centres by `1.0` — the same value the block lanes form.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn eval_pixels<S: Simd, const CHAINED: bool>(
    simd: S,
    eval: ImageEval<'_>,
    mut x: f32,
    y: f32,
    out: &mut [MaybeUninit<[f32; 4]>],
) {
    for px in out.iter_mut() {
        // SAFETY: the store writes all four lanes of `px`.
        eval_pixel::<S, CHAINED>(simd, eval, x, y).store_array(unsafe { &mut *px.as_mut_ptr() });
        x += 1.0;
    }
}

/// `PaintData::Image` over `out` through `super::span`'s driver: the
/// span's invariants decided once by [`ImageEval`], blocks and the
/// one-pixel path shared with every other kind. Entered through
/// `vectorize` so it compiles under the level's target features even
/// though it is not inlined — `#[inline(never)]` keeps the kernel's
/// body out of `PaintData::eval_span`'s frame, where inlined next to
/// every other kind it overflowed the debug test stack.
#[inline(never)]
pub(super) fn image_span<'a, S: Simd>(
    simd: S,
    outer: &PaintData,
    params: &ImagePaintData,
    x0: usize,
    py: usize,
    out: &'a mut [MaybeUninit<[f32; 4]>],
) -> &'a mut [[f32; 4]] {
    simd.vectorize(
        #[inline(always)]
        || super::span::span(simd, &ImageEval::new(outer, params), x0, py, out),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov_oracle::metrics::assert_close;

    /// A `LinearP3` upload is the working space already: no transfer
    /// function and no primaries matrix — the byte value passes through.
    #[test]
    fn linear_p3_upload_decodes_as_identity() {
        let image = ImageUpload {
            width: 1,
            height: 1,
            data: vec![255, 0, 0, 255].into(),
            color_space: ImageColorSpace::LinearP3,
            premultiplied: false,
            format: ImageFormat::Rgba8,
        };
        let decoded = CpuImage::decode(&image).expect("decode");
        // 255/255 and 0/255 decode to one and zero under the identity transfer.
        assert_close(&decoded.pixels[0], &[1.0, 0.0, 0.0, 1.0]);
    }
}
