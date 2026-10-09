//! The YUV decode reference against hand-computed values: chroma at and
//! between its sites, clamping at the frame edges and on odd frame sizes,
//! the primaries conversion and full-range decodes against values derived
//! independently from the published standards, and whole-pipeline values
//! for BT.709 SDR, BT.2020 PQ and BT.2020 HLG checked by encoding the
//! decoded light forward again with the published equations.

use cherenkov_oracle::Image;
use cherenkov_oracle::color::{linear_p3_to_linear_bt2020, linear_p3_to_linear_srgb};
use cherenkov_oracle::yuv::{
    ChromaSiting, Matrix, Nv12, P010, PlaneLayout, Primaries, Range, Transfer, YuvColor, YuvError,
    YuvFrame, decode,
};

/// SMPTE ST 2084's constants as the standard publishes them.
const PQ_M1: f64 = 2610.0 / 16_384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;

/// BT.2100 Table 5's HLG OETF constants: `a`, `b = 1 - 4a` and
/// `c = 0.5 - a ln(4a)`, at the published precision.
const HLG_A: f64 = 0.178_832_77;
const HLG_B: f64 = 0.284_668_92;
const HLG_C: f64 = 0.559_910_73;

/// `Cr` codes of the 3×3 chroma plane behind the 6×6 siting frame, by
/// chroma row then column. Irregular, so no pair of weightings can agree
/// by accident.
const CR: [[u8; 3]; 3] = [[60, 100, 180], [90, 200, 120], [150, 70, 210]];
/// `Cb` codes of the same plane.
const CB: [[u8; 3]; 3] = [[200, 140, 80], [110, 220, 160], [50, 130, 240]];

/// A `width × height` frame of `(Cb, Cr)` chroma `chroma` decoded with
/// `siting`: luma code 0 throughout, full range, BT.709 matrix, linear
/// transfer, Display P3 primaries — so each pixel's red and blue channels
/// are its reconstructed `Cr` and `Cb` through the matrix alone.
fn chroma_frame(width: u32, height: u32, chroma: &[[u8; 2]], siting: ChromaSiting) -> Image {
    let luma = vec![0u8; width as usize * height as usize];
    let frame = YuvFrame::<Nv12> {
        width,
        height,
        luma: &luma,
        chroma,
    };
    let color = YuvColor {
        matrix: Matrix::Bt709,
        range: Range::Full,
        siting,
        primaries: Primaries::DisplayP3,
        transfer: Transfer::Linear,
    };
    decode(&frame, &color).expect("the chroma frame decodes")
}

/// The 6×6 siting frame over the 3×3 [`CB`] and [`CR`] plane.
fn siting_frame(siting: ChromaSiting) -> Image {
    let chroma: Vec<[u8; 2]> = (0..3)
        .flat_map(|row| (0..3).map(move |col| [CB[row][col], CR[row][col]]))
        .collect();
    chroma_frame(6, 6, &chroma, siting)
}

/// The `(Cb, Cr)` codes pixel `(x, y)` was decoded with. With `Y' = 0`
/// and full range, BT.709 gives `B' = 1.8556 · (Cb - 128) / 255` and
/// `R' = 1.5748 · (Cr - 128) / 255`.
fn chroma_codes(image: &Image, x: usize, y: usize) -> [f64; 2] {
    let pixel = image.pixels[y * image.width + x];
    [
        (pixel[2] / 1.8556).mul_add(255.0, 128.0),
        (pixel[0] / 1.5748).mul_add(255.0, 128.0),
    ]
}

fn assert_chroma(image: &Image, x: usize, y: usize, expected: [f64; 2]) {
    let got = chroma_codes(image, x, y);
    assert!(
        (got[0] - expected[0]).abs() < 1e-9 && (got[1] - expected[1]).abs() < 1e-9,
        "pixel ({x}, {y}): (Cb, Cr) {got:?}, expected {expected:?}"
    );
}

#[test]
fn a_luma_sample_on_a_chroma_site_takes_that_sample() {
    // Cosited on both axes, chroma sample (k, j) sits on luma (2k, 2j).
    let image = siting_frame(ChromaSiting::TOP_LEFT);
    assert_chroma(&image, 0, 0, [200.0, 60.0]);
    // Luma (2, 4) is chroma site (1, 2).
    assert_chroma(&image, 2, 4, [130.0, 70.0]);
}

#[test]
fn a_luma_sample_between_sites_takes_the_bilinear_weights() {
    // Cosited: luma 3 is halfway between chroma 1 (at 2) and 2 (at 4);
    // luma row 2 is on chroma row 1. Cb (220 + 160) / 2, Cr (200 + 120) / 2.
    let top_left = siting_frame(ChromaSiting::TOP_LEFT);
    assert_chroma(&top_left, 3, 2, [190.0, 160.0]);

    // MPEG-2: luma column 1 halfway between chroma columns 0 and 1; luma
    // row 1 at chroma coordinate 0.25, weights 3/4 on row 0, 1/4 on row 1.
    // Cr 3/4 · (60 + 100) / 2 + 1/4 · (90 + 200) / 2 = 60 + 36.25.
    // Cb 3/4 · (200 + 140) / 2 + 1/4 · (110 + 220) / 2 = 127.5 + 41.25.
    let left = siting_frame(ChromaSiting::LEFT);
    assert_chroma(&left, 1, 1, [168.75, 96.25]);

    // Centred: luma column 2 at chroma coordinate 0.75 (1/4 on column 0,
    // 3/4 on column 1); luma row 3 at 1.25 (3/4 on row 1, 1/4 on row 2).
    // Cr 3/4 · (1/4 · 90 + 3/4 · 200) + 1/4 · (1/4 · 150 + 3/4 · 70)
    //    = 3/4 · 172.5 + 1/4 · 90 = 151.875.
    // Cb 3/4 · (1/4 · 110 + 3/4 · 220) + 1/4 · (1/4 · 50 + 3/4 · 130)
    //    = 3/4 · 192.5 + 1/4 · 110 = 171.875.
    let centered = siting_frame(ChromaSiting::CENTERED);
    assert_chroma(&centered, 2, 3, [171.875, 151.875]);
}

#[test]
fn edges_clamp_to_the_outermost_chroma_sample() {
    let centered = siting_frame(ChromaSiting::CENTERED);
    // Luma 0 lies at chroma coordinate -0.25, before chroma 0: both taps
    // read sample 0, on both axes.
    assert_chroma(&centered, 0, 0, [200.0, 60.0]);
    // Luma 5 lies at 2.25, past the last chroma sample (2).
    assert_chroma(&centered, 5, 5, [240.0, 210.0]);
    // Clamped on x, interpolated on y: row 3 at 1.25.
    // Cr 3/4 · 90 + 1/4 · 150, Cb 3/4 · 110 + 1/4 · 50.
    assert_chroma(&centered, 0, 3, [95.0, 105.0]);

    // Cosited: luma 5 lies at 2.5, between chroma 2 and a sample beyond
    // the plane, which reads chroma 2.
    let top_left = siting_frame(ChromaSiting::TOP_LEFT);
    assert_chroma(&top_left, 5, 0, [80.0, 180.0]);
}

#[test]
fn an_odd_frame_reads_its_rounded_up_chroma_plane() {
    // A 3×3 frame has a ⌈3/2⌉ × ⌈3/2⌉ = 2×2 chroma plane. By chroma row
    // then column: Cb [[200, 60], [90, 230]], Cr [[40, 170], [220, 130]].
    let chroma: [[u8; 2]; 4] = [[200, 40], [60, 170], [90, 220], [230, 130]];
    let centered = chroma_frame(3, 3, &chroma, ChromaSiting::CENTERED);
    // Centred, luma 0 lies at chroma -0.25 (clamped to sample 0), luma 1
    // at 0.25 (3/4 on sample 0, 1/4 on 1) and the last luma sample, 2, at
    // 0.75 (1/4 on 0, 3/4 on 1): on an odd axis the last luma sample
    // stays inside the chroma plane.
    assert_chroma(&centered, 0, 0, [200.0, 40.0]);
    // Cb 3/4 · (3/4 · 200 + 1/4 · 60) + 1/4 · (3/4 · 90 + 1/4 · 230)
    //    = 3/4 · 165 + 1/4 · 125 = 155;
    // Cr 3/4 · (3/4 · 40 + 1/4 · 170) + 1/4 · (3/4 · 220 + 1/4 · 130)
    //    = 3/4 · 72.5 + 1/4 · 197.5 = 103.75.
    assert_chroma(&centered, 1, 1, [155.0, 103.75]);
    // Cb 1/4 · (1/4 · 200 + 3/4 · 60) + 3/4 · (1/4 · 90 + 3/4 · 230)
    //    = 1/4 · 95 + 3/4 · 195 = 170;
    // Cr 1/4 · (1/4 · 40 + 3/4 · 170) + 3/4 · (1/4 · 220 + 3/4 · 130)
    //    = 1/4 · 137.5 + 3/4 · 152.5 = 148.75.
    assert_chroma(&centered, 2, 2, [170.0, 148.75]);
    // Clamped on x, interpolated on y: Cb 1/4 · 200 + 3/4 · 90,
    // Cr 1/4 · 40 + 3/4 · 220.
    assert_chroma(&centered, 0, 2, [117.5, 175.0]);

    // Cosited, the last luma sample of an odd axis is on the last chroma
    // site: luma (2, 2) is chroma (1, 1).
    let top_left = chroma_frame(3, 3, &chroma, ChromaSiting::TOP_LEFT);
    assert_chroma(&top_left, 2, 2, [230.0, 130.0]);
}

/// A 2×2 frame with one chroma sample, so siting cannot matter.
const fn uniform<'a, L: PlaneLayout>(
    luma: &'a [L::Word; 4],
    chroma: &'a [[L::Word; 2]; 1],
) -> YuvFrame<'a, L> {
    YuvFrame {
        width: 2,
        height: 2,
        luma,
        chroma,
    }
}

/// Every pixel of `image` equals pixel 0.
fn the_pixel(image: &Image) -> [f64; 3] {
    let first = image.pixels[0];
    assert!(
        image
            .pixels
            .iter()
            .all(|pixel| pixel.map(f64::to_bits) == first.map(f64::to_bits)),
        "{image:?}"
    );
    assert!((first[3] - 1.0).abs() < f64::EPSILON, "opaque");
    [first[0], first[1], first[2]]
}

fn assert_near(got: [f64; 3], expected: [f64; 3], tolerance: f64) {
    assert!(
        got.iter()
            .zip(expected)
            .all(|(value, want)| (value - want).abs() < tolerance),
        "got {got:?}, expected {expected:?}"
    );
}

// The expected values of the next two tests are computed outside the
// oracle, in exact rational arithmetic (the sRGB power alone in `f64`),
// from the published definitions:
//
// - H.273 full range at bit depth N: `Y' = D / (2^N - 1)`,
//   `C = (D - 2^(N-1)) / (2^N - 1)`;
// - the `Y'CbCr` matrices inverted, `R' = Y' + 2(1 - Kr) Cr`,
//   `B' = Y' + 2(1 - Kb) Cb`, `G' = (Y' - Kr R' - Kb B') / (1 - Kr - Kb)`,
//   with BT.601 `Kr, Kb = 0.299, 0.114`, BT.709 `0.2126, 0.0722` and
//   BT.2020 `0.2627, 0.0593`;
// - RGB to XYZ from the xy chromaticities and the D65 white
//   `(0.3127, 0.3290)`: BT.709 R (0.64, 0.33) G (0.30, 0.60) B (0.15, 0.06),
//   BT.2020 R (0.708, 0.292) G (0.170, 0.797) B (0.131, 0.046), Display P3
//   (SMPTE EG 432-1) R (0.680, 0.320) G (0.265, 0.690) B (0.150, 0.060);
// - the IEC 61966-2-1 sRGB EOTF, `V / 12.92` up to `V = 0.04045` and
//   `((V + 0.055) / 1.055)^2.4` above.
//
// The derivation, in Python with `from fractions import Fraction as F`:
//
//     def rgb_to_xyz(primaries, white):
//         # Columns (x/y, 1, (1-x-y)/y) per primary, scaled so RGB (1, 1, 1)
//         # lands on the white's XYZ.
//         p = [[x / y, F(1), (1 - x - y) / y] for x, y in primaries]
//         p = [[p[j][i] for j in range(3)] for i in range(3)]
//         wx, wy = white
//         s = mat_vec(mat_inv(p), [wx / wy, F(1), (1 - wx - wy) / wy])
//         return [[p[i][j] * s[j] for j in range(3)] for i in range(3)]
//
//     to_p3 = mat_mul(mat_inv(rgb_to_xyz(P3, D65)), rgb_to_xyz(BT2020, D65))
//     expected = mat_vec(to_p3, ycc_to_rgb(kr, kb, y, cb, cr))
//
// with `mat_inv`, `mat_mul` and `mat_vec` the textbook rational 3×3
// operations. That BT.2020 to Display P3 matrix is
//
//     [ 1.343578252584332,    -0.2821796705261357,  -0.06139858205819628 ]
//     [-0.06529745278911953,   1.0757879158485746,  -0.010490463059454957]
//     [ 0.0028217872617009514, -0.01959849452449406,  1.0167767072627931 ]
//
// and the BT.709 one
//
//     [0.8224619687143623,   0.17753803128563775, 0.0               ]
//     [0.03319419885096162,  0.9668058011490384,  0.0               ]
//     [0.017082630721120033, 0.07239744066396347, 0.9105199286149165]
//
// Both the rational results and the reference's `f64` evaluation are
// within a few ulps of each other on values of order one, so `1e-12` is
// a loose bound that still rejects any error in a published constant.

#[test]
fn primaries_map_into_display_p3_through_the_published_chromaticities() {
    // BT.2020 red as nearly as 8-bit full range carries it: Y 67, Cb 92,
    // Cr 255 (`round(255 Kr)`, `128 + round(-255 Kr / 1.8814)`, Cr's
    // ceiling). Y' = 67/255, Cb = -36/255, Cr = 127/255, so R'G'B' =
    // (0.9971537254901961, 0.0014198645381456418, -0.002864313725490196)
    // — out of P3's gamut, so P3 green and blue go negative.
    let bt2020_red = uniform::<Nv12>(&[67; 4], &[[92, 255]]);
    let bt2020 = YuvColor {
        matrix: Matrix::Bt2020,
        range: Range::Full,
        siting: ChromaSiting::TOP_LEFT,
        primaries: Primaries::Bt2020,
        transfer: Transfer::Linear,
    };
    assert_near(
        the_pixel(&decode(&bt2020_red, &bt2020).expect("decodes")),
        [
            1.339_529_267_945_823_5,
            -0.063_554_077_224_083_8,
            -0.000_126_439_005_202_033_63,
        ],
        1e-12,
    );

    // BT.709 R'G'B' (0.2, 0.8, 0.3) quantized to full range: Y 162,
    // Cb 82, Cr 57, which is R'G'B' = (0.19682039215686276,
    // 0.7994264311093565, 0.300558431372549).
    let bt709_green = uniform::<Nv12>(&[162; 4], &[[82, 57]]);
    let bt709 = YuvColor {
        matrix: Matrix::Bt709,
        primaries: Primaries::Bt709,
        ..bt2020
    };
    assert_near(
        the_pixel(&decode(&bt709_green, &bt709).expect("decodes")),
        [
            0.303_805_881_953_324_8,
            0.779_423_406_423_577_1,
            0.334_903_079_166_99,
        ],
        1e-12,
    );
}

#[test]
fn full_range_p010_bt601_srgb_matches_the_published_equations() {
    // R'G'B' (0.85, 0.40, 0.20) through BT.601 into 10-bit full range:
    // Y 524, Cb 332, Cr 759 (`round(1023 Y')`, `512 + round(1023 C)`).
    // Back: Y' = 524/1023, Cb = -180/1023, Cr = 247/1023, so R'G'B' =
    // (0.8507272727272728, 0.40034493531234755, 0.20043010752688173),
    // each on the sRGB curve's power segment. Display P3 primaries leave
    // the sRGB-decoded light as it is.
    let frame = uniform::<P010>(&[524 << 6; 4], &[[332 << 6, 759 << 6]]);
    let color = YuvColor {
        matrix: Matrix::Bt601,
        range: Range::Full,
        siting: ChromaSiting::CENTERED,
        primaries: Primaries::DisplayP3,
        transfer: Transfer::Srgb,
    };
    assert_near(
        the_pixel(&decode(&frame, &color).expect("decodes")),
        [
            0.693_406_590_755_594_7,
            0.133_110_195_661_414_4,
            0.033_238_935_271_149_975,
        ],
        1e-12,
    );
}

#[test]
fn bt709_video_round_trips_through_the_published_equations() {
    // NV12, codes Y 126, Cb 100, Cr 170: Y' = 110/219, Cb = -0.125,
    // Cr = 0.1875, which is R'G'B' ≈ (0.7976, 0.4379, 0.2703).
    let frame = uniform::<Nv12>(&[126; 4], &[[100, 170]]);
    let color = YuvColor {
        matrix: Matrix::Bt709,
        range: Range::Video,
        siting: ChromaSiting::LEFT,
        primaries: Primaries::Bt709,
        transfer: Transfer::Bt709,
    };
    let p3 = the_pixel(&decode(&frame, &color).expect("decodes"));
    // Back to BT.709 primaries, then the BT.1886 EOTF inverted (Lw = 1,
    // Lb = 0: V = L^(1/2.4)).
    let [red, green, blue] = linear_p3_to_linear_srgb(p3).map(|light| light.powf(1.0 / 2.4));
    // BT.709-6 items 3.2 and 3.3: E'Y = 0.2126 E'R + 0.7152 E'G +
    // 0.0722 E'B, E'CB = (E'B - E'Y) / 1.8556, E'CR = (E'R - E'Y) /
    // 1.5748; item 4.6 quantization at 8 bits: 219 E'Y + 16,
    // 224 E'C + 128.
    let luma = 0.0722f64.mul_add(blue, 0.2126f64.mul_add(red, 0.7152 * green));
    let codes = [
        219.0f64.mul_add(luma, 16.0),
        224.0f64.mul_add((blue - luma) / 1.8556, 128.0),
        224.0f64.mul_add((red - luma) / 1.5748, 128.0),
    ];
    assert_near(codes, [126.0, 100.0, 170.0], 1e-9);
}

/// SMPTE ST 2084 inverse EOTF: absolute luminance as a fraction of
/// 10 000 nits to the signal.
fn pq_inverse_eotf(fraction: f64) -> f64 {
    let power = fraction.powf(PQ_M1);
    (PQ_C2.mul_add(power, PQ_C1) / PQ_C3.mul_add(power, 1.0)).powf(PQ_M2)
}

/// BT.2020-2 Tables 4 and 5: `E'Y = 0.2627 E'R + 0.6780 E'G +
/// 0.0593 E'B`, `E'CB = (E'B - E'Y) / 1.8814`, `E'CR = (E'R - E'Y) /
/// 1.4746`; 10-bit quantization `876 E'Y + 64`, `896 E'C + 512`.
fn bt2020_video_codes([red, green, blue]: [f64; 3]) -> [f64; 3] {
    let luma = 0.0593f64.mul_add(blue, 0.2627f64.mul_add(red, 0.6780 * green));
    [
        876.0f64.mul_add(luma, 64.0),
        896.0f64.mul_add((blue - luma) / 1.8814, 512.0),
        896.0f64.mul_add((red - luma) / 1.4746, 512.0),
    ]
}

#[test]
fn bt2020_pq_round_trips_through_the_published_equations() {
    // P010, codes Y 600, Cb 448, Cr 576: Y' = 536/876, Cb = -1/14,
    // Cr = 1/14, which is R'G'B' ≈ (0.7172, 0.5828, 0.4775) — about
    // 728, 207 and 73 nits, 3.58, 1.02 and 0.36 of 203-nit white.
    let frame = uniform::<P010>(&[600 << 6; 4], &[[448 << 6, 576 << 6]]);
    let color = YuvColor {
        matrix: Matrix::Bt2020,
        range: Range::Video,
        siting: ChromaSiting::LEFT,
        primaries: Primaries::Bt2020,
        transfer: Transfer::Pq {
            reference_white: 203.0,
        },
    };
    let p3 = the_pixel(&decode(&frame, &color).expect("decodes"));
    let signal =
        linear_p3_to_linear_bt2020(p3).map(|light| pq_inverse_eotf(light * 203.0 / 10_000.0));
    assert_near(bt2020_video_codes(signal), [600.0, 448.0, 576.0], 1e-9);
}

#[test]
fn bt2020_hlg_round_trips_through_the_published_equations() {
    // P010, codes Y 500, Cb 448, Cr 576: R'G'B' ≈ (0.6030, 0.4687,
    // 0.3633), one channel on each segment of the OETF. A 2000-nit peak
    // makes the system gamma 1.2 + 0.42 · log10(2).
    let frame = uniform::<P010>(&[500 << 6; 4], &[[448 << 6, 576 << 6]]);
    let (white, peak) = (203.0, 2000.0);
    let color = YuvColor {
        matrix: Matrix::Bt2020,
        range: Range::Video,
        siting: ChromaSiting::LEFT,
        primaries: Primaries::Bt2020,
        transfer: Transfer::Hlg {
            reference_white: white,
            peak,
        },
    };
    let p3 = the_pixel(&decode(&frame, &color).expect("decodes"));
    // Display light in nits, then BT.2100 Table 5's OOTF inverted:
    // Fd = Lw · Ys^(γ - 1) · E, so Yd = Lw · Ys^γ.
    let display = linear_p3_to_linear_bt2020(p3).map(|light| light * white);
    let gamma = 0.42f64.mul_add((peak / 1000.0).log10(), 1.2);
    let display_luminance = 0.0593f64.mul_add(
        display[2],
        0.2627f64.mul_add(display[0], 0.6780 * display[1]),
    );
    let scene_luminance = (display_luminance / peak).powf(1.0 / gamma);
    let scene = display.map(|nits| nits / (peak * scene_luminance.powf(gamma - 1.0)));
    // The HLG OETF.
    let signal = scene.map(|light| {
        if light <= 1.0 / 12.0 {
            (3.0 * light).sqrt()
        } else {
            HLG_A.mul_add(12.0f64.mul_add(light, -HLG_B).ln(), HLG_C)
        }
    });
    // Both directions weigh luminance with Table 5's 0.2627, 0.6780,
    // 0.0593, so the round trip is exact up to rounding: some forty f64
    // operations on codes below 1024, each off by at most half an ulp
    // (~1e-13 of a code), leave the result within ~1e-11 of the input
    // codes. 1e-9 keeps that margin while rejecting a coefficient error at
    // the 1e-7 level, which moves these codes by about 2e-5.
    assert_near(bt2020_video_codes(signal), [500.0, 448.0, 576.0], 1e-9);
}

#[test]
fn invalid_frames_are_errors() {
    let video_pq = YuvColor {
        matrix: Matrix::Bt2020,
        range: Range::Video,
        siting: ChromaSiting::LEFT,
        primaries: Primaries::Bt2020,
        transfer: Transfer::Pq {
            reference_white: 203.0,
        },
    };
    let padded = uniform::<P010>(&[(600 << 6) | 1; 4], &[[512 << 6, 512 << 6]]);
    assert_eq!(
        decode(&padded, &video_pq).unwrap_err(),
        YuvError::Padding {
            word: (600 << 6) | 1
        }
    );

    let no_white = YuvColor {
        transfer: Transfer::Pq {
            reference_white: 0.0,
        },
        ..video_pq
    };
    let frame = uniform::<P010>(&[600 << 6; 4], &[[512 << 6, 512 << 6]]);
    assert!(matches!(
        decode(&frame, &no_white).unwrap_err(),
        YuvError::Level { .. }
    ));

    // A 3×3 frame needs ⌈3/2⌉² = 4 chroma pairs.
    let short = YuvFrame::<P010> {
        width: 3,
        height: 3,
        luma: &[64 << 6; 9],
        chroma: &[[512 << 6; 2]; 3],
    };
    assert_eq!(
        decode(&short, &video_pq).unwrap_err(),
        YuvError::PlaneSize {
            plane: "chroma",
            expected: 4,
            actual: 3
        }
    );

    let empty = YuvFrame::<P010> {
        width: 0,
        height: 2,
        luma: &[],
        chroma: &[],
    };
    assert_eq!(decode(&empty, &video_pq).unwrap_err(), YuvError::Empty);
}
