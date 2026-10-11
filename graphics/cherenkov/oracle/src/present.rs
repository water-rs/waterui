//! Presentation of the working-space image into an output encoding — the
//! oracle twin of `cherenkov-gpu`'s `render/present.wgsl`, evaluated in
//! `f64` on the oracle's [`Image`].
//!
//! The bench's `render --present` compares an engine image that went
//! through the real presentation pass against the same reference image run
//! through these functions.

use crate::Image;
use crate::color::{
    hlg_encode_channel, hlg_inverse_ootf, linear_bt2020_to_linear_p3, linear_srgb_to_linear_p3,
    mat3_mul, pq_encode, srgb_decode, srgb_decode_extended, srgb_encode, srgb_encode_extended,
};

/// Linear Display P3 → linear sRGB — the Bradford-adapted D65 matrix of
/// `present.wgsl`, the same constants in `f64`.
const P3_TO_LINEAR_SRGB: [[f64; 3]; 3] = [
    [1.224_940_2, -0.224_940_2, 0.0],
    [-0.042_056_95, 1.042_056_9, 0.0],
    [-0.019_637_55, -0.078_636_05, 1.098_273_6],
];

/// SDR white in nits — the BT.2408 reference white the absolute and
/// relative HDR encodings calibrate against: working-space `1.0`
/// presents at 203 nits. `present.wgsl` uses the same constant.
pub const REFERENCE_WHITE_NITS: f64 = 203.0;

/// The headroom a BT.2100 PQ destination carries — the 10 000-nit PQ
/// range at the reference white.
const PQ_HEADROOM: f64 = 10_000.0 / REFERENCE_WHITE_NITS;

/// The headroom a BT.2100 HLG destination carries — the 1000-nit
/// nominal peak at the reference white.
const HLG_HEADROOM: f64 = 1_000.0 / REFERENCE_WHITE_NITS;

/// Presents `image` to an sRGB output at display `headroom`.
///
/// The `present.wgsl` shader-transfer path per pixel under premultiplied
/// output alpha: unpremultiply, the P3 → sRGB matrix, the headroom tone
/// map of [`crate::tone`], the analytic `OKLab` gamut map of
/// [`crate::gamut`], the sRGB transfer, re-premultiply. In-gamut P3
/// colours pass through bit-for-bit; out-of-gamut colours keep their hue
/// and land on the sRGB boundary instead of clipping per-channel (#96).
/// Values above `1.0` compress into `min(headroom, 1)` — an sRGB
/// destination cannot store more — with the highlight shoulder rolling
/// off instead of the old clamp (#97).
///
/// Returns premultiplied *encoded* sRGB in `[0, 1]` with linear alpha —
/// the values an sRGB presentation texture stores.
///
/// `headroom` is the scene's declared presentation headroom; the sRGB
/// surface's representable ceiling caps the tone-map target at `1`.
#[must_use]
pub fn present_srgb(headroom: f64, image: &Image) -> Image {
    let target = headroom.min(1.0);
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| present_srgb_pixel(p, target))
            .collect(),
    }
}

/// Presents `image` to an extended linear Display P3 host texture
/// (`OutputColor::LinearDisplayP3`).
///
/// Unpremultiply, tone-map to the display `headroom` on the maximum
/// channel, re-premultiply — the `f16` host attachment holds the
/// `[0, headroom]` extended range (#97).
#[must_use]
pub fn present_linear_p3(headroom: f64, image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| present_linear_p3_pixel(p, headroom))
            .collect(),
    }
}

/// Presents `image` to an SDR Display P3 destination
/// (`OutputColor::DisplayP3`) at display `headroom`.
///
/// Unpremultiply, tone-map to `min(headroom, 1)`, map to the *P3*
/// gamut boundary — the destination-specific cusp coefficients, not
/// sRGB's (#98) — sRGB transfer (Display P3's transfer is sRGB's),
/// re-premultiply. Returns premultiplied encoded values in `[0, 1]`.
#[must_use]
pub fn present_display_p3(headroom: f64, image: &Image) -> Image {
    let target = headroom.min(1.0);
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| present_display_p3_pixel(p, target))
            .collect(),
    }
}

/// Presents `image` to an extended linear sRGB destination
/// (`OutputColor::ExtendedSrgbLinear`, scRGB).
///
/// Tone-map to the display `headroom`, convert primaries, linear
/// transfer — no `[0, 1]` gamut clip: negative and above-one components
/// carry the wide gamut (#98). Returns premultiplied linear
/// sRGB-primaries values.
#[must_use]
pub fn present_extended_srgb_linear(headroom: f64, image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| {
                let straight = straighten(p);
                let srgb = mat3_mul(
                    &P3_TO_LINEAR_SRGB,
                    crate::tone::tone_map(headroom, straight),
                );
                [srgb[0] * p[3], srgb[1] * p[3], srgb[2] * p[3], p[3]]
            })
            .collect(),
    }
}

/// Presents `image` to an encoded extended sRGB destination
/// (`OutputColor::ExtendedSrgb`): the signed extended transfer after the
/// primary conversion (#98).
#[must_use]
pub fn present_extended_srgb(headroom: f64, image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| {
                let straight = straighten(p);
                let srgb = mat3_mul(
                    &P3_TO_LINEAR_SRGB,
                    crate::tone::tone_map(headroom, straight),
                );
                [
                    srgb_encode_extended(srgb[0]) * p[3],
                    srgb_encode_extended(srgb[1]) * p[3],
                    srgb_encode_extended(srgb[2]) * p[3],
                    p[3],
                ]
            })
            .collect(),
    }
}

/// Presents `image` to an encoded extended Display P3 destination
/// (`OutputColor::ExtendedDisplayP3`): the signed extended transfer on
/// the working primaries — not a raw linear-P3 attachment (#98).
#[must_use]
pub fn present_extended_display_p3(headroom: f64, image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| {
                let straight = straighten(p);
                let p3 = crate::tone::tone_map(headroom, straight);
                [
                    srgb_encode_extended(p3[0]) * p[3],
                    srgb_encode_extended(p3[1]) * p[3],
                    srgb_encode_extended(p3[2]) * p[3],
                    p[3],
                ]
            })
            .collect(),
    }
}

/// Presents `image` to a BT.2100 PQ destination (`OutputColor::Bt2100Pq`).
///
/// Tone-map to the display headroom (capped at the PQ range), convert to
/// BT.2020, scale SDR white to 203 nits of the 10 000-nit signal, ST 2084
/// encode, re-premultiply (#98). Returns premultiplied PQ signal values.
///
/// wgpu supplies no mastering metadata for the pair, so this signal is
/// calibrated by the reference white alone — not an authored HDR10 stream.
#[must_use]
pub fn present_pq(headroom: f64, image: &Image) -> Image {
    let target = headroom.min(PQ_HEADROOM);
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| {
                let straight = straighten(p);
                let nits = mat3_mul(
                    &P3_TO_LINEAR_BT2020,
                    crate::tone::tone_map(target, straight),
                )
                .map(|c| c * (REFERENCE_WHITE_NITS / 10_000.0));
                [
                    pq_encode(nits[0]) * p[3],
                    pq_encode(nits[1]) * p[3],
                    pq_encode(nits[2]) * p[3],
                    p[3],
                ]
            })
            .collect(),
    }
}

/// Presents `image` to a BT.2100 HLG destination (`OutputColor::Bt2100Hlg`).
///
/// Tone-map to the display headroom (capped at the nominal peak), convert
/// to BT.2020 at the 1000-nit peak, the reference OOTF's inverse (system
/// gamma 1.2), then the OETF, re-premultiply (#98).
#[must_use]
pub fn present_hlg(headroom: f64, image: &Image) -> Image {
    let target = headroom.min(HLG_HEADROOM);
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&p| {
                let straight = straighten(p);
                let display = mat3_mul(
                    &P3_TO_LINEAR_BT2020,
                    crate::tone::tone_map(target, straight),
                )
                .map(|c| c * (REFERENCE_WHITE_NITS / 1_000.0));
                let scene = hlg_inverse_ootf(display);
                [
                    hlg_encode_channel(scene[0]) * p[3],
                    hlg_encode_channel(scene[1]) * p[3],
                    hlg_encode_channel(scene[2]) * p[3],
                    p[3],
                ]
            })
            .collect(),
    }
}

/// Lifts one premultiplied encoded-P3 presented pixel back into the
/// working space (`OutputColor::DisplayP3` readbacks).
#[must_use]
pub fn presented_display_p3_to_working([r, g, b, a]: [f64; 4]) -> [f64; 4] {
    [srgb_decode(r), srgb_decode(g), srgb_decode(b), a]
}

/// Lifts one premultiplied extended-sRGB (linear or encoded) presented
/// pixel back into the working space. `encoded` selects the wire format.
#[must_use]
pub fn presented_extended_srgb_to_working(encoded: bool, [r, g, b, a]: [f64; 4]) -> [f64; 4] {
    let decode = if encoded { srgb_decode_extended } else { |c| c };
    let p3 = linear_srgb_to_linear_p3([decode(r), decode(g), decode(b)]);
    [p3[0], p3[1], p3[2], a]
}

/// Lifts one premultiplied encoded extended-P3 presented pixel back into
/// the working space.
#[must_use]
pub fn presented_extended_p3_to_working([r, g, b, a]: [f64; 4]) -> [f64; 4] {
    [
        srgb_decode_extended(r),
        srgb_decode_extended(g),
        srgb_decode_extended(b),
        a,
    ]
}

/// Lifts one premultiplied PQ signal pixel back into the working space:
/// PQ decode to the 10 000-nit range, rescale by the reference white,
/// BT.2020 → P3 primaries.
#[must_use]
pub fn presented_pq_to_working([r, g, b, a]: [f64; 4]) -> [f64; 4] {
    let bt2020 = [
        crate::color::pq_decode(r) * (10_000.0 / REFERENCE_WHITE_NITS),
        crate::color::pq_decode(g) * (10_000.0 / REFERENCE_WHITE_NITS),
        crate::color::pq_decode(b) * (10_000.0 / REFERENCE_WHITE_NITS),
    ];
    let p3 = linear_bt2020_to_linear_p3(bt2020);
    [p3[0], p3[1], p3[2], a]
}

/// Lifts one premultiplied HLG signal pixel back into the working space:
/// inverse OETF, the reference OOTF (system gamma 1.2), rescale by the
/// nominal peak, BT.2020 → P3 primaries.
#[must_use]
pub fn presented_hlg_to_working([r, g, b, a]: [f64; 4]) -> [f64; 4] {
    let scene = [
        crate::color::hlg_decode_channel(r),
        crate::color::hlg_decode_channel(g),
        crate::color::hlg_decode_channel(b),
    ];
    let display = crate::color::hlg_ootf(scene).map(|c| c * (1_000.0 / REFERENCE_WHITE_NITS));
    let p3 = linear_bt2020_to_linear_p3(display);
    [p3[0], p3[1], p3[2], a]
}

/// A premultiplied pixel's straight colour — the shared headroom-tone-map
/// input (#98 destinations all unpremultiply first).
fn straighten([r, g, b, a]: [f64; 4]) -> [f64; 3] {
    if a > 0.0 {
        [r / a, g / a, b / a]
    } else {
        [0.0; 3]
    }
}

/// Linear Display P3 → linear BT.2020 — `inverse(REC2020_TO_XYZ) ·
/// P3_TO_XYZ`, the same product `linear_p3_to_linear_bt2020` computes.
const P3_TO_LINEAR_BT2020: [[f64; 3]; 3] = [
    [
        0.753_833_034_276_766_6,
        0.198_597_369_169_493_2,
        0.047_569_596_501_021_77,
    ],
    [
        0.045_743_849_054_211_916,
        0.941_777_219_393_263_8,
        0.012_478_931_245_100_001,
    ],
    [
        -0.001_210_340_388_370_433_8,
        0.017_601_717_307_645_23,
        0.983_608_623_002_275_9,
    ],
];

/// `present.wgsl`'s Display-P3 SDR path for one premultiplied working
/// pixel under `OutputAlpha::Premultiplied`: tone-map in the working
/// space, map to the P3 gamut boundary, sRGB encode, re-premultiply.
fn present_display_p3_pixel([r, g, b, a]: [f64; 4], target: f64) -> [f64; 4] {
    let straight = straighten([r, g, b, a]);
    let toned = crate::tone::tone_map(target, straight);
    let mapped = crate::gamut::gamut_map_p3_analytic(toned).map(srgb_encode);
    [mapped[0] * a, mapped[1] * a, mapped[2] * a, a]
}

/// Quantizes a presented image to the unorm-8 steps a destination texture
/// stores.
///
/// `round(clamp(c, 0, 1) * 255) / 255` per channel — the value an ideal
/// presenter stores; the store also clamps, which shows for a `>1` alpha
/// or premultiplied channel (e.g. plus-lighter output). Comparing a
/// read-back u8 output against the quantized reference removes the format's
/// quantization floor from the metric, so what remains is the presentation
/// pass's own error.
#[must_use]
pub fn quantize_unorm8(image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() / 255.0))
            .collect(),
    }
}

/// Lifts one premultiplied encoded-sRGB presented pixel back into the
/// working space.
///
/// The sRGB transfer is decoded, then sRGB → P3 primaries applied on the
/// premultiplied channels (a linear matrix commutes with the alpha scale).
/// A presented sRGB output and a presented reference compare in
/// [`crate::metrics`] once both are lifted like this.
#[must_use]
pub fn presented_srgb_to_working([r, g, b, a]: [f64; 4]) -> [f64; 4] {
    let p3 = linear_srgb_to_linear_p3([srgb_decode(r), srgb_decode(g), srgb_decode(b)]);
    [p3[0], p3[1], p3[2], a]
}

/// `present.wgsl`'s sRGB path for one premultiplied working-space pixel
/// under `OutputAlpha::Premultiplied`: straight alpha is recovered for
/// the headroom tone map in the working space (#97 — an sRGB-intensity
/// P3 colour never takes the shoulder), then the matrix into linear sRGB,
/// the gamut map, and the encoded colour is re-premultiplied.
fn present_srgb_pixel([r, g, b, a]: [f64; 4], target: f64) -> [f64; 4] {
    let straight = if a > 0.0 {
        crate::tone::tone_map(target, [r / a, g / a, b / a])
    } else {
        [0.0; 3]
    };
    let srgb = crate::gamut::gamut_map_srgb_analytic(mat3_mul(&P3_TO_LINEAR_SRGB, straight))
        .map(srgb_encode);
    [srgb[0] * a, srgb[1] * a, srgb[2] * a, a]
}

/// `present.wgsl`'s linear P3 path for one premultiplied pixel: the tone
/// map runs on the straight colour, then the result re-premultiplies.
fn present_linear_p3_pixel([r, g, b, a]: [f64; 4], headroom: f64) -> [f64; 4] {
    let straight = if a > 0.0 {
        [r / a, g / a, b / a]
    } else {
        [0.0; 3]
    };
    let p3 = crate::tone::tone_map(headroom, straight);
    [p3[0] * a, p3[1] * a, p3[2] * a, a]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(p: [f64; 4]) -> Image {
        Image::filled(1, 1, p)
    }

    fn px(image: &Image) -> [f64; 4] {
        image.pixels[0]
    }

    fn assert_close(actual: [f64; 4], expected: [f64; 4]) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() < 1e-6, "{a} != {e}");
        }
    }

    #[test]
    fn srgb_mid_gray_in_gamut() {
        // 0.5 in P3 maps to ~0.5 in sRGB; sRGB-encodes to ~0.7354.
        assert_close(
            px(&present_srgb(1.0, &img([0.5, 0.5, 0.5, 1.0]))),
            [0.735_357, 0.735_357, 0.735_357, 1.0],
        );
    }

    #[test]
    fn srgb_p3_red_maps_to_hue_preserved_red() {
        // P3's red primary is outside sRGB: the gamut map keeps its hue
        // and lands on the boundary — encoded sRGB red with a perceptual
        // chroma remainder, not the old per-channel clamp's [1, 0, 0].
        assert_close(
            px(&present_srgb(1.0, &img([1.0, 0.0, 0.0, 1.0]))),
            [1.0, 0.202_234, 0.157_756, 1.0],
        );
    }

    #[test]
    fn srgb_out_of_gamut_maps_to_boundary() {
        // P3 (0, 1, 0.5): out of gamut on r and g; the map keeps the
        // green-cyan hue — the clamp had produced encoded [0, 1, 0.716].
        assert_close(
            px(&present_srgb(1.0, &img([0.0, 1.0, 0.5, 1.0]))),
            [0.000_001, 0.974_930, 0.748_692, 1.0],
        );
    }

    #[test]
    fn srgb_hdr_white_maps_to_sdr_white() {
        // 4× SDR white tone-maps onto the sRGB target's ceiling (h = 1
        // regardless of the display's headroom) — not the old channel
        // clamp, but the same stored bytes here.
        assert_close(
            px(&present_srgb(4.0, &img([4.0, 4.0, 4.0, 1.0]))),
            [1.0, 1.0, 1.0, 1.0],
        );
    }

    #[test]
    fn srgb_in_gamut_colour_encodes() {
        // P3 (0.25, 0.5, 0.75): in-gamut after the matrix, then encoded.
        assert_close(
            px(&present_srgb(1.0, &img([0.25, 0.5, 0.75, 1.0]))),
            [0.477_456, 0.742_240, 0.895_978, 1.0],
        );
    }

    #[test]
    fn srgb_premultiplied_output() {
        // Premultiplied input is unpremultiplied for the conversion and
        // the encoded result is re-premultiplied.
        assert_close(
            px(&present_srgb(1.0, &img([0.1, 0.2, 0.05, 0.5]))),
            [0.215_091, 0.335_728, 0.151_213, 0.5],
        );
        // Transparent pixels present as transparent black.
        assert_close(px(&present_srgb(1.0, &img([0.0, 0.0, 0.0, 0.0]))), [0.0; 4]);
    }

    #[test]
    fn unorm8_quantization_rounds_to_store() {
        // Round-to-nearest on the unorm-8 grid: what the destination
        // format stores.
        let image = img([0.4_f64 / 255.0, 0.6 / 255.0, 0.5, 1.0]);
        let q = px(&quantize_unorm8(&image));
        assert_eq!(q[0].to_bits(), 0.0f64.to_bits());
        assert_eq!(q[1].to_bits(), (1.0_f64 / 255.0).to_bits());
        assert_eq!(q[2].to_bits(), (128.0_f64 / 255.0).to_bits());
        assert_eq!(q[3].to_bits(), 1.0f64.to_bits());
    }

    #[test]
    fn linear_p3_sdr_is_identity() {
        // The SDR range — including out-of-gamut negative channels —
        // passes through bit-for-bit at any headroom.
        let pixel = [0.125, -0.05, 0.25, 0.5];
        assert_eq!(
            px(&present_linear_p3(4.0, &img(pixel))).map(f64::to_bits),
            pixel.map(f64::to_bits)
        );
    }

    #[test]
    fn linear_p3_hdr_rolls_off_to_headroom() {
        // 8× SDR white on a headroom-2 display compresses below the peak:
        // the extended range is tone-mapped, never handed over beyond the
        // headroom and never hard-clipped.
        let [r, g, b, a] = px(&present_linear_p3(2.0, &img([8.0, 8.0, 8.0, 1.0])));
        assert_eq!(g.to_bits(), r.to_bits());
        assert_eq!(b.to_bits(), r.to_bits());
        assert!(r < 2.0 && r > 1.0, "8x at h=2: {r}");
        assert_eq!(a.to_bits(), 1.0f64.to_bits());
        // At headroom 1 the same pixel lands on SDR white.
        assert_eq!(
            px(&present_linear_p3(1.0, &img([8.0, 8.0, 8.0, 1.0]))).map(f64::to_bits),
            [1.0, 1.0, 1.0, 1.0].map(f64::to_bits)
        );
    }

    #[test]
    fn presented_srgb_round_trips_to_working() {
        // An in-gamut colour presented and lifted back lands on the
        // original working-space value (the simplified WGSL constants are
        // not the exact matrices, so the round trip is approximate).
        let pixel = [0.25, 0.5, 0.75, 1.0];
        let presented = px(&present_srgb(1.0, &img(pixel)));
        let lifted = presented_srgb_to_working(presented);
        assert_close(lifted, pixel);
    }
}
