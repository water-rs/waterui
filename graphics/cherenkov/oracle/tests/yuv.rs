//! The YUV decode reference against hand-computed values: chroma at and
//! between its sites, clamping at the frame edges, and whole-pipeline
//! values for BT.709 SDR, BT.2020 PQ and BT.2020 HLG checked by encoding
//! the decoded light forward again with the published equations.

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

/// The 6×6 siting frame decoded with `siting`: luma code 0 throughout,
/// full range, BT.709 matrix, linear transfer, Display P3 primaries — so
/// each pixel's red and blue channels are its reconstructed `Cr` and `Cb`
/// through the matrix alone.
fn siting_frame(siting: ChromaSiting) -> Image {
    let luma = [0u8; 36];
    let chroma: Vec<[u8; 2]> = (0..3)
        .flat_map(|row| (0..3).map(move |col| [CB[row][col], CR[row][col]]))
        .collect();
    let frame = YuvFrame::<Nv12> {
        width: 6,
        height: 6,
        luma: &luma,
        chroma: &chroma,
    };
    let color = YuvColor {
        matrix: Matrix::Bt709,
        range: Range::Full,
        siting,
        primaries: Primaries::DisplayP3,
        transfer: Transfer::Linear,
    };
    decode(&frame, &color).expect("the siting frame decodes")
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

fn assert_codes(got: [f64; 3], expected: [f64; 3], tolerance: f64) {
    assert!(
        got.iter()
            .zip(expected)
            .all(|(code, want)| (code - want).abs() < tolerance),
        "codes {got:?}, expected {expected:?}"
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
    assert_codes(codes, [126.0, 100.0, 170.0], 1e-9);
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
    assert_codes(bt2020_video_codes(signal), [600.0, 448.0, 576.0], 1e-9);
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
    // The reference takes Ys from the BT.2020 primaries' own luminance
    // row, which matches Table 5's four-digit coefficients to about 1e-7:
    // a few 1e-5 of a code here.
    assert_codes(bt2020_video_codes(signal), [500.0, 448.0, 576.0], 1e-3);
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
