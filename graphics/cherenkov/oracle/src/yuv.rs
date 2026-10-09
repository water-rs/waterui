//! Reference decode of biplanar 4:2:0 `Y'CbCr` frames — NV12 (8-bit) and
//! P010 (10-bit) — into the working space, extended linear Display P3
//! with `1.0` at reference white.
//!
//! [`decode`] evaluates every luma pixel of the frame, at the frame's own
//! resolution, in `f64`:
//!
//! 1. **Code normalization** ([`Range`]). Luma becomes `Y'` on `[0, 1]`
//!    across the nominal range and chroma becomes `Cb`, `Cr` on
//!    `[-0.5, 0.5]`. At bit depth `N`, video range is BT.709 / BT.2020
//!    quantization, `Y' = (D / 2^(N-8) - 16) / 219` and
//!    `C = (D / 2^(N-8) - 128) / 224`; full range is
//!    `Y' = D / (2^N - 1)` and `C = (D - 2^(N-1)) / (2^N - 1)`. A P010 word
//!    carries its code in the high ten bits ([`P010`]).
//! 2. **Chroma reconstruction** ([`ChromaSiting`]), bilinear at the sited
//!    position. Number luma samples `0..w` and chroma samples `0..cw` on
//!    one axis (`cw = ⌈w / 2⌉`), and measure positions in luma samples,
//!    luma sample `x` sitting at `x`. Chroma sample `k` sits at:
//!
//!    - `2k` when that axis is [`ChromaOffset::Cosited`] — on top of luma
//!      sample `2k`;
//!    - `2k + 0.5` when it is [`ChromaOffset::Centered`] — midway between
//!      luma samples `2k` and `2k + 1`.
//!
//!    Luma sample `x` therefore lies at chroma coordinate `u = (x - s) / 2`,
//!    with `s` the offset above (`0` or `0.5`). With `k0 = ⌊u⌋` and
//!    `f = u - k0`, its chroma is `(1 - f) · C[k0] + f · C[k0 + 1]`; the
//!    two axes combine separably, which is bilinear interpolation of the
//!    four nearest chroma samples. The weights are exact: a cosited axis
//!    gives `f = 0` on even `x` (the luma sample is on a chroma site and
//!    takes it alone) and `f = 1/2` on odd `x`; a centred axis gives
//!    `f = 3/4` on even `x` (`C[x/2 - 1]` weighted `1/4`, `C[x/2]`
//!    weighted `3/4`) and `f = 1/4` on odd `x`. Interpolation runs on the
//!    normalized components; normalization is affine, so this is the
//!    same as interpolating codes.
//!
//!    Edges clamp: a sample index below `0` reads sample `0`, and one at or
//!    beyond `cw` reads sample `cw - 1`. Outside the outermost chroma sites
//!    the edge sample's value holds, and the weights still sum to one.
//!    Clamping is reached on a centred axis at luma sample `0`
//!    (`k0 = -1`) and, for either siting, at the last luma sample of an
//!    even-length axis, which lies past the last chroma site.
//! 3. **Matrix** ([`Matrix`]): the non-constant-luminance inverse
//!    `R' = Y' + 2(1 - Kr) Cr`, `B' = Y' + 2(1 - Kb) Cb`,
//!    `G' = (Y' - Kr R' - Kb B') / Kg`, with the recommendation's published
//!    `Kr`, `Kg`, `Kb` (`Kg = 1 - Kr - Kb`).
//! 4. **Transfer** ([`Transfer`]) to white-relative linear light, per
//!    channel. A signal below `0` decodes as `0` for every non-linear
//!    transfer: each curve is defined from black up. Above `1` the
//!    relative curves and HLG continue their formula (super-white codes
//!    stay brighter than white), and PQ clamps to `1`, its 10 000-nit
//!    ceiling. [`Transfer::Linear`] passes the signal through unchanged.
//! 5. **Primaries** ([`Primaries`]): the source RGB maps through XYZ into
//!    linear Display P3, unclamped, so colours outside P3 keep their
//!    negative channels. The result is opaque (alpha `1.0`).

use std::fmt;

use crate::color::{
    BT2100_LUMA, Mat3, P3_TO_XYZ, REC2020_TO_XYZ, SRGB_TO_XYZ, hlg_inverse_oetf, mat3_inv,
    mat3_mul, mat3_product, pq_decode, srgb_decode,
};
use crate::image::Image;

/// The `Y'CbCr` matrix of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    /// BT.601: `Kr = 0.299`, `Kb = 0.114`.
    Bt601,
    /// BT.709: `Kr = 0.2126`, `Kb = 0.0722`.
    Bt709,
    /// BT.2020 non-constant luminance: `Kr = 0.2627`, `Kb = 0.0593`.
    Bt2020,
}

impl Matrix {
    /// `[Kr, Kg, Kb]` as the recommendation publishes them: BT.601's luma
    /// equation, BT.709-6 item 3.2, BT.2020-2 Table 4.
    const fn weights(self) -> [f64; 3] {
        match self {
            Self::Bt601 => [0.299, 0.587, 0.114],
            Self::Bt709 => [0.2126, 0.7152, 0.0722],
            Self::Bt2020 => BT2100_LUMA,
        }
    }

    /// `R'G'B'` of a normalized `Y'`, `Cb`, `Cr`.
    const fn to_rgb(self, luma: f64, [cb, cr]: [f64; 2]) -> [f64; 3] {
        let [kr, kg, kb] = self.weights();
        let red = (2.0 * (1.0 - kr)).mul_add(cr, luma);
        let blue = (2.0 * (1.0 - kb)).mul_add(cb, luma);
        let green = (luma - kr.mul_add(red, kb * blue)) / kg;
        [red, green, blue]
    }
}

/// The code range of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    /// Studio range: at 8 bits, luma `16..=235` and chroma `16..=240`
    /// span the nominal range, scaled by `2^(N-8)` at `N` bits.
    Video,
    /// Full range: `0..=2^N - 1`, chroma centred on `2^(N-1)`.
    Full,
}

impl Range {
    /// `Y'` of a luma code at `bits` depth.
    fn luma(self, code: u32, bits: u32) -> f64 {
        let code = f64::from(code);
        match self {
            Self::Video => (code / f64::from(1u32 << (bits - 8)) - 16.0) / 219.0,
            Self::Full => code / f64::from((1u32 << bits) - 1),
        }
    }

    /// `Cb` or `Cr` of a chroma code at `bits` depth.
    fn chroma(self, code: u32, bits: u32) -> f64 {
        let code = f64::from(code);
        match self {
            Self::Video => (code / f64::from(1u32 << (bits - 8)) - 128.0) / 224.0,
            Self::Full => (code - f64::from(1u32 << (bits - 1))) / f64::from((1u32 << bits) - 1),
        }
    }
}

/// Where chroma samples sit on one axis, relative to luma samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromaOffset {
    /// Chroma sample `k` sits on luma sample `2k`.
    Cosited,
    /// Chroma sample `k` sits midway between luma samples `2k` and
    /// `2k + 1`.
    Centered,
}

impl ChromaOffset {
    /// The offset `s` doubled, `2s`: luma sample `x` lies at chroma
    /// coordinate `u` with `4u = 2x - 2s`.
    const fn doubled(self) -> u32 {
        match self {
            Self::Cosited => 0,
            Self::Centered => 1,
        }
    }

    /// The chroma samples bracketing luma sample `luma` on this axis,
    /// clamped to `0..samples`, and the weight of the second one.
    ///
    /// `q = 2·luma + 4 - 2s` is `4·(u + 1)`, so `q / 4` is `⌊u⌋ + 1` and
    /// `q % 4` is `4·(u - ⌊u⌋)`: the arithmetic is exact.
    fn taps(self, luma: u32, samples: u32) -> (usize, usize, f64) {
        let q = 2 * luma + 4 - self.doubled();
        let next = q / 4;
        let last = samples - 1;
        // `next - 1` is `⌊u⌋`, which is `-1` only on a centred axis at
        // luma sample 0: saturating clamps it to sample 0.
        let near = next.saturating_sub(1).min(last);
        let far = next.min(last);
        (near as usize, far as usize, f64::from(q % 4) / 4.0)
    }
}

/// The two-axis chroma location of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromaSiting {
    /// Horizontal.
    pub x: ChromaOffset,
    /// Vertical.
    pub y: ChromaOffset,
}

impl ChromaSiting {
    /// Centred on both axes (JPEG, H.273 chroma location type 1).
    pub const CENTERED: Self = Self {
        x: ChromaOffset::Centered,
        y: ChromaOffset::Centered,
    };
    /// Cosited horizontally, centred vertically (MPEG-2, H.273 type 0).
    pub const LEFT: Self = Self {
        x: ChromaOffset::Cosited,
        y: ChromaOffset::Centered,
    };
    /// Cosited on both axes (H.273 chroma location type 2, the siting
    /// HDR10 specifies for its 4:2:0 BT.2020 video; BT.2020 itself does not
    /// define 4:2:0 siting).
    pub const TOP_LEFT: Self = Self {
        x: ChromaOffset::Cosited,
        y: ChromaOffset::Cosited,
    };
}

/// The additive primaries a frame's signal is expressed on; all use the
/// D65 white point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primaries {
    /// BT.709 / sRGB.
    Bt709,
    /// Display P3.
    DisplayP3,
    /// BT.2020.
    Bt2020,
}

impl Primaries {
    /// Linear RGB on these primaries to CIE XYZ.
    const fn to_xyz(self) -> &'static Mat3 {
        match self {
            Self::Bt709 => &SRGB_TO_XYZ,
            Self::DisplayP3 => &P3_TO_XYZ,
            Self::Bt2020 => &REC2020_TO_XYZ,
        }
    }

    /// The luminance weights `[Kr, Kg, Kb]` of linear RGB on these
    /// primaries, as HLG's OOTF takes them. BT.2020 uses BT.2100 Table 5's
    /// published `0.2627`, `0.6780`, `0.0593` and BT.709 its
    /// recommendation's `0.2126`, `0.7152`, `0.0722` — in both cases the
    /// same coefficients as the matching `Y'CbCr` matrix. Display P3 has no
    /// published rounded set, so it takes the `Y` row of its RGB-to-XYZ
    /// matrix.
    const fn luminance(self) -> [f64; 3] {
        match self {
            Self::Bt709 => Matrix::Bt709.weights(),
            Self::DisplayP3 => P3_TO_XYZ[1],
            Self::Bt2020 => Matrix::Bt2020.weights(),
        }
    }
}

/// The transfer a frame's `R'G'B'` signal is encoded with, and the levels
/// an absolute transfer needs to land white-relative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transfer {
    /// Linear light; the signal passes through unchanged.
    Linear,
    /// The IEC 61966-2-1 sRGB curve.
    Srgb,
    /// BT.709-encoded video through the BT.1886 reference EOTF with
    /// `Lw = 1`, `Lb = 0`: `max(E', 0)^2.4`, reference white at `1.0`.
    Bt709,
    /// SMPTE ST 2084: the signal decodes to absolute luminance, which is
    /// divided by `reference_white` nits.
    Pq {
        /// The nits reference white occupies.
        reference_white: f64,
    },
    /// BT.2100 hybrid log-gamma. The inverse OETF gives scene light `E`;
    /// the OOTF `Fd = peak · Ys^(γ - 1) · E` gives display light, with
    /// `Ys` the luminance of `E` on the frame's primaries (for BT.2020,
    /// BT.2100's published `0.2627 R + 0.6780 G + 0.0593 B`) and
    /// `γ = 1.2 + 0.42 · log10(peak / 1000)`; `Fd` is divided by
    /// `reference_white`. Scene black (`Ys = 0`) decodes to black.
    ///
    /// BT.2100 and BT.2390 specify that system-gamma formula for nominal
    /// peaks of roughly 400 to 2000 nits; a `peak` outside that span
    /// evaluates the same formula. The display black level is taken as
    /// zero (`Lb = 0`), so the OOTF carries no black lift `β`.
    Hlg {
        /// The nits reference white occupies.
        reference_white: f64,
        /// The nominal peak display luminance `Lw` the OOTF targets, in
        /// nits.
        peak: f64,
    },
}

impl Transfer {
    /// A positive, finite level, or the error naming it.
    const fn level(what: &'static str, nits: f64) -> Result<(), YuvError> {
        if nits.is_finite() && nits > 0.0 {
            Ok(())
        } else {
            Err(YuvError::Level { what, nits })
        }
    }

    /// Whether the transfer's levels are usable.
    fn validate(self) -> Result<(), YuvError> {
        match self {
            Self::Linear | Self::Srgb | Self::Bt709 => Ok(()),
            Self::Pq { reference_white } => Self::level("PQ reference white", reference_white),
            Self::Hlg {
                reference_white,
                peak,
            } => {
                Self::level("HLG reference white", reference_white)?;
                Self::level("HLG peak", peak)
            }
        }
    }

    /// White-relative linear light of an `R'G'B'` signal on `primaries`.
    fn to_linear(self, signal: [f64; 3], primaries: Primaries) -> [f64; 3] {
        match self {
            Self::Linear => signal,
            Self::Srgb => signal.map(|c| srgb_decode(c.max(0.0))),
            Self::Bt709 => signal.map(|c| c.max(0.0).powf(2.4)),
            Self::Pq { reference_white } => {
                signal.map(|c| pq_decode(c) * (10_000.0 / reference_white))
            }
            Self::Hlg {
                reference_white,
                peak,
            } => {
                let scene = signal.map(|c| hlg_inverse_oetf(c.max(0.0)));
                let [kr, kg, kb] = primaries.luminance();
                let ys = kb.mul_add(scene[2], kr.mul_add(scene[0], kg * scene[1]));
                if ys <= 0.0 {
                    return [0.0; 3];
                }
                let gamma = 0.42f64.mul_add((peak / 1000.0).log10(), 1.2);
                let gain = ys.powf(gamma - 1.0) * (peak / reference_white);
                scene.map(|c| c * gain)
            }
        }
    }
}

/// How a frame's planes decode into the working space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YuvColor {
    /// The `Y'CbCr` matrix.
    pub matrix: Matrix,
    /// The code range.
    pub range: Range,
    /// Where chroma samples sit.
    pub siting: ChromaSiting,
    /// The primaries of the decoded signal.
    pub primaries: Primaries,
    /// The transfer of the decoded signal.
    pub transfer: Transfer,
}

/// The word packing of a biplanar 4:2:0 frame's planes.
pub trait PlaneLayout {
    /// One stored sample.
    type Word: Copy + fmt::Debug;
    /// The code's bit depth.
    const BITS: u32;

    /// The code a stored word carries.
    ///
    /// # Errors
    /// [`YuvError::Padding`] when the word is not a valid sample of the
    /// layout.
    fn code(word: Self::Word) -> Result<u32, YuvError>;
}

/// NV12: 8-bit codes, one byte per sample.
#[derive(Clone, Copy, Debug)]
pub struct Nv12;

impl PlaneLayout for Nv12 {
    type Word = u8;
    const BITS: u32 = 8;

    fn code(word: u8) -> Result<u32, YuvError> {
        Ok(u32::from(word))
    }
}

/// P010: 10-bit codes in the high bits of 16-bit words; the low six bits
/// are padding and must be zero.
#[derive(Clone, Copy, Debug)]
pub struct P010;

impl PlaneLayout for P010 {
    type Word = u16;
    const BITS: u32 = 10;

    fn code(word: u16) -> Result<u32, YuvError> {
        if word.trailing_zeros() >= 6 {
            Ok(u32::from(word >> 6))
        } else {
            Err(YuvError::Padding { word })
        }
    }
}

/// A biplanar 4:2:0 frame: a luma plane and an interleaved `CbCr` plane at
/// half resolution on both axes (rounded up), both tightly packed in row
/// order.
#[derive(Debug)]
pub struct YuvFrame<'a, L: PlaneLayout> {
    /// Luma width in pixels.
    pub width: u32,
    /// Luma height in pixels.
    pub height: u32,
    /// `width × height` luma words.
    pub luma: &'a [L::Word],
    /// `⌈width / 2⌉ × ⌈height / 2⌉` `(Cb, Cr)` word pairs.
    pub chroma: &'a [[L::Word; 2]],
}

/// Why a frame cannot be decoded.
#[derive(Clone, Debug, PartialEq)]
pub enum YuvError {
    /// The frame has no pixels.
    Empty,
    /// A plane's sample count does not match the frame size.
    PlaneSize {
        /// `"luma"` or `"chroma"`.
        plane: &'static str,
        /// The count the frame size requires.
        expected: usize,
        /// The count supplied.
        actual: usize,
    },
    /// A P010 word with non-zero padding bits.
    Padding {
        /// The offending word.
        word: u16,
    },
    /// A transfer level that is not a positive, finite number of nits.
    Level {
        /// Which level.
        what: &'static str,
        /// The value supplied.
        nits: f64,
    },
}

impl fmt::Display for YuvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the frame has no pixels"),
            Self::PlaneSize {
                plane,
                expected,
                actual,
            } => write!(
                f,
                "the {plane} plane holds {actual} samples; the frame size needs {expected}"
            ),
            Self::Padding { word } => write!(
                f,
                "P010 word {word:#06x} has non-zero padding in its low six bits"
            ),
            Self::Level { what, nits } => {
                write!(
                    f,
                    "{what} must be a positive, finite number of nits, not {nits}"
                )
            }
        }
    }
}

impl std::error::Error for YuvError {}

/// A plane's sample count against the count the frame size needs.
const fn check_plane(plane: &'static str, expected: usize, actual: usize) -> Result<(), YuvError> {
    if expected == actual {
        Ok(())
    } else {
        Err(YuvError::PlaneSize {
            plane,
            expected,
            actual,
        })
    }
}

/// The reference decode of `frame` under `color`: one working-space pixel
/// per luma sample, premultiplied and opaque, as the module documentation
/// defines it.
///
/// # Errors
/// [`YuvError`] when the frame is empty, a plane's length does not match
/// the frame size, a word is not a valid sample of its layout, or a
/// transfer level is not a positive, finite number of nits.
pub fn decode<L: PlaneLayout>(
    frame: &YuvFrame<'_, L>,
    color: &YuvColor,
) -> Result<Image, YuvError> {
    color.transfer.validate()?;
    let (width, height) = (frame.width, frame.height);
    if width == 0 || height == 0 {
        return Err(YuvError::Empty);
    }
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    check_plane("luma", width as usize * height as usize, frame.luma.len())?;
    check_plane(
        "chroma",
        chroma_width as usize * chroma_height as usize,
        frame.chroma.len(),
    )?;
    let luma = frame
        .luma
        .iter()
        .map(|&word| L::code(word).map(|code| color.range.luma(code, L::BITS)))
        .collect::<Result<Vec<f64>, YuvError>>()?;
    let chroma = frame
        .chroma
        .iter()
        .map(|&[cb, cr]| -> Result<[f64; 2], YuvError> {
            Ok([
                color.range.chroma(L::code(cb)?, L::BITS),
                color.range.chroma(L::code(cr)?, L::BITS),
            ])
        })
        .collect::<Result<Vec<[f64; 2]>, YuvError>>()?;

    let to_p3 = mat3_product(&mat3_inv(&P3_TO_XYZ), color.primaries.to_xyz());
    let stride = chroma_width as usize;
    let at = |across: usize, down: usize| chroma[down * stride + across];
    let lerp = |from: [f64; 2], to: [f64; 2], weight: f64| {
        [
            (to[0] - from[0]).mul_add(weight, from[0]),
            (to[1] - from[1]).mul_add(weight, from[1]),
        ]
    };
    let mut image = Image::new(width as usize, height as usize);
    for row in 0..height {
        let (j0, j1, fy) = color.siting.y.taps(row, chroma_height);
        for col in 0..width {
            let (i0, i1, fx) = color.siting.x.taps(col, chroma_width);
            let sited = lerp(
                lerp(at(i0, j0), at(i1, j0), fx),
                lerp(at(i0, j1), at(i1, j1), fx),
                fy,
            );
            let index = row as usize * width as usize + col as usize;
            let signal = color.matrix.to_rgb(luma[index], sited);
            let light = color.transfer.to_linear(signal, color.primaries);
            let p3 = mat3_mul(&to_p3, light);
            image.pixels[index] = [p3[0], p3[1], p3[2], 1.0];
        }
    }
    Ok(image)
}
