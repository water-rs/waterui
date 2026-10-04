//! Deterministic checks for the reference rasterizer and metrics.

use cherenkov_oracle::color::{linear_srgb_to_linear_p3, to_working};
use cherenkov_oracle::{Renderer, metrics};
use cherenkov_scene::{Color, Extend, GradientStop, LinearGradient, Paint, Scene, Shape};
use kurbo::Rect;

const W: u32 = 16;
const H: u32 = 16;

fn render(scene: &Scene, dir: &std::path::Path) -> cherenkov_oracle::F32Image {
    Renderer::new(W as usize, H as usize)
        .render(scene, dir)
        .expect("oracle render")
}

fn tmp() -> std::path::PathBuf {
    std::env::temp_dir().join("cherenkov-oracle-tests")
}

#[test]
fn self_comparison_is_perfect() {
    let mut b = Scene::builder(W, H);
    b.root().fill(
        Shape::Rect(Rect::new(2.0, 2.0, 14.0, 14.0)),
        Paint::Solid(Color::srgb(0.5, 0.25, 1.0)),
    );
    let scene = b.build();
    let img = render(&scene, &tmp());
    let (m, _heat) = metrics::compare(&img, &img);
    assert_eq!(m.flip_mean, 0.0);
    assert_eq!(m.flip_max, 0.0);
    assert_eq!(m.max_local_error, 0.0);
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "comparing f32 channels against f64 constants"
)]
#[test]
fn solid_rect_fills_pixel_centres() {
    let mut b = Scene::builder(W, H);
    b.root().fill(
        Shape::Rect(Rect::new(4.0, 4.0, 12.0, 12.0)),
        Paint::Solid(Color::srgb(1.0, 0.0, 0.0)),
    );
    let scene = b.build();
    let img = render(&scene, &tmp());
    // Pixel (8,8): fully covered by opaque red.
    let [r, g, bl, a] = img.pixels[8 * W as usize + 8];
    assert_eq!(a, 1.0);
    let want = to_working(&Color::srgb(1.0, 0.0, 0.0));
    assert!((r - want[0] as f32).abs() < 1e-5);
    assert!((g - want[1] as f32).abs() < 1e-5);
    assert!((bl - want[2] as f32).abs() < 1e-5);
    // Pixel (0,0): default clear colour (opaque black), untouched.
    assert_eq!(img.pixels[0], [0.0, 0.0, 0.0, 1.0]);
}

/// Clipping is geometric — the exact area of shape∩clip per pixel — not a
/// product of independently computed coverages.
///
/// A clip `x < 9.75` over a shape `x ∈ [9.25, 16)` yields exact coverage
/// 0.5 in pixel column 9; multiplying the independent coverages
/// (0.75 × 0.75) would give 0.5625. This test locks in the geometric rule.
#[test]
fn clip_is_geometric_intersection() {
    let mut b = Scene::builder(W, H).clear(Color::new(
        cherenkov_scene::ColorSpace::Srgb,
        [0.0, 0.0, 0.0, 0.0],
    ));
    {
        let mut root = b.root();
        root.clip(Shape::Rect(Rect::new(0.0, 0.0, 9.75, 16.0)));
        root.fill(
            Shape::Rect(Rect::new(9.25, 0.0, 16.0, 16.0)),
            Paint::Solid(Color::srgb(1.0, 1.0, 1.0)),
        );
    }
    let scene = b.build();
    let img = render(&scene, &tmp());
    let alpha = img.pixels[9][3];
    assert!(
        (alpha - 0.5).abs() < 1e-5,
        "geometric clip coverage: got {alpha}, want 0.5"
    );
    // The intersection is the sliver [9.25, 9.75): no pixel is fully
    // covered. Column 8 is inside the clip but outside the shape; column 10
    // is inside the shape but outside the clip.
    assert_eq!(img.pixels[8][3], 0.0);
    assert_eq!(img.pixels[10][3], 0.0);
}

/// A member clipped by an ellipse never writes outside the clip: the
/// clip's flattened boundary must be a sealed contour even when the curve
/// approximation's last point lands a rounding ulp off its start (kurbo's
/// `Arc`-based ellipse path ends open, without `ClosePath`).
///
/// Regression for a 24×16 px leak: the unsealed tip let the geometric
/// clip drop the fill's inside piece, and the open contour read as
/// covered to the left of the clip.
#[test]
fn ellipse_clip_writes_nothing_outside_the_tip() {
    const W: usize = 144;
    let mut b = Scene::builder(144, 144).clear(Color::new(
        cherenkov_scene::ColorSpace::Srgb,
        [0.0, 0.0, 0.0, 0.0],
    ));
    {
        let mut root = b.root();
        root.layer(|m| {
            m.clip(Shape::Ellipse(kurbo::Ellipse::new(
                (72.0, 72.0),
                (48.0, 28.0),
                0.0,
            )));
            m.fill(
                Shape::Rect(Rect::new(26.0, 46.0, 144.0, 144.0)),
                Paint::Solid(Color::srgb(1.0, 1.0, 1.0)),
            );
        });
    }
    let scene = b.build();
    let img = Renderer::new(W, W)
        .render(&scene, &tmp())
        .expect("oracle render");
    // The tip is at x = 24: every pixel left of it is outside the clip.
    for y in 64..80usize {
        for x in 0..24usize {
            assert_eq!(
                img.pixels[y * W + x][3],
                0.0,
                "pixel ({x},{y}) written outside the ellipse clip"
            );
        }
    }
    // The interior is painted.
    assert!(img.pixels[72 * W + 72][3] > 0.99);
}

/// Shading rule: gradients are evaluated at the pixel centre and multiplied
/// by exact coverage.
#[expect(
    clippy::cast_possible_truncation,
    reason = "comparing f32 channels against f64 constants"
)]
#[test]
fn linear_gradient_at_pixel_centre() {
    let mut b = Scene::builder(W, H);
    b.root().fill(
        Shape::Rect(Rect::new(0.0, 0.0, 16.0, 16.0)),
        Paint::Linear(LinearGradient {
            start: kurbo::Point::new(0.0, 8.0),
            end: kurbo::Point::new(16.0, 8.0),
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: Color::new(
                        cherenkov_scene::ColorSpace::LinearSrgb,
                        [0.0, 0.0, 0.0, 1.0],
                    ),
                },
                GradientStop {
                    offset: 1.0,
                    color: Color::new(
                        cherenkov_scene::ColorSpace::LinearSrgb,
                        [1.0, 1.0, 1.0, 1.0],
                    ),
                },
            ],
            extend: Extend::Pad,
            interpolation: cherenkov_scene::ColorSpace::LinearSrgb,
        }),
    );
    let scene = b.build();
    let img = render(&scene, &tmp());
    // Pixel 0: centre x=0.5 → fraction 0.5/16 of linear white ≈ 0.03125.
    let r0 = img.pixels[0][0];
    let p3 = linear_srgb_to_linear_p3([0.03125, 0.03125, 0.03125]);
    assert!(
        (r0 - p3[0] as f32).abs() < 1e-3,
        "gradient at pixel centre: got {r0}, want ≈{}",
        p3[0]
    );
    // Symmetry: first and last pixels are not equal — centre eval, not edge.
    let r15 = img.pixels[15][0];
    assert!(r15 > r0);
}

#[test]
fn metrics_detect_error() {
    let mut b = Scene::builder(W, H);
    b.root().fill(
        Shape::Rect(Rect::new(0.0, 0.0, 16.0, 16.0)),
        Paint::Solid(Color::srgb(0.0, 0.0, 0.0)),
    );
    let scene = b.build();
    let mut img = render(&scene, &tmp());
    let mut perturbed = img.clone();
    // Flip pixel (8,8) to opaque white — a large local error.
    perturbed.pixels[8 * W as usize + 8] = [1.0, 1.0, 1.0, 1.0];
    let (m, heat) = metrics::compare(&img, &perturbed);
    assert!(m.flip_mean > 0.0);
    assert!(m.max_local_error > 0.0);
    assert_eq!(heat.len(), (W * H * 3) as usize);
    img.pixels[8 * W as usize + 8] = [1.0, 1.0, 1.0, 1.0];
    let (m2, _) = metrics::compare(&img, &perturbed);
    assert_eq!(m2.flip_mean, 0.0);
}

/// The metric pipeline runs in Display P3 primaries: two linear-P3 colours
/// that both land on negative sRGB red (clamped to the same sRGB colour by
/// an sRGB-primary pipeline) must still measure as different.
#[test]
fn p3_colours_outside_srgb_are_distinguished() {
    let mk = |r: f32| cherenkov_oracle::F32Image {
        width: W,
        height: H,
        pixels: vec![[r, 1.0, 0.0, 1.0]; (W * H) as usize],
    };
    let reference = mk(0.0);
    let test = mk(0.12);
    let (m, _) = metrics::compare(&reference, &test);
    assert!(m.flip_mean > 0.0, "P3-only difference measured zero");
    assert!(m.flip_max > 0.0, "P3-only difference measured zero");
    assert!(!m.hdr);
}

fn encode_png(
    color: png::ColorType,
    depth: png::BitDepth,
    w: u32,
    h: u32,
    pixels: &[u8],
    palette: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        if let Some(p) = palette {
            enc.set_palette(p);
        }
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(pixels).expect("png data");
    }
    out
}

/// Every PNG colour type decodes through the shared oracle/bench decoder.
#[test]
fn decode_png_all_colour_types() {
    use cherenkov_oracle::image::decode_png_rgba8;
    let rgba = |px: &[u8]| px.to_vec();

    // Grey 8-bit.
    let png = encode_png(
        png::ColorType::Grayscale,
        png::BitDepth::Eight,
        2,
        1,
        &[0, 255],
        None,
    );
    let (w, h, px) = decode_png_rgba8(&png).expect("grey8");
    assert_eq!((w, h), (2, 1));
    assert_eq!(px, rgba(&[0, 0, 0, 255, 255, 255, 255, 255]));

    // Grey 16-bit (big-endian, stripped to 8-bit).
    let png = encode_png(
        png::ColorType::Grayscale,
        png::BitDepth::Sixteen,
        1,
        1,
        &[0x80, 0x00],
        None,
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("grey16");
    assert_eq!(px, rgba(&[128, 128, 128, 255]));

    // Grey + alpha 8-bit.
    let png = encode_png(
        png::ColorType::GrayscaleAlpha,
        png::BitDepth::Eight,
        1,
        1,
        &[128, 64],
        None,
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("greyalpha8");
    assert_eq!(px, rgba(&[128, 128, 128, 64]));

    // Grey + alpha 16-bit.
    let png = encode_png(
        png::ColorType::GrayscaleAlpha,
        png::BitDepth::Sixteen,
        1,
        1,
        &[0x80, 0x00, 0x40, 0x00],
        None,
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("greyalpha16");
    assert_eq!(px, rgba(&[128, 128, 128, 64]));

    // RGB 8-bit.
    let png = encode_png(
        png::ColorType::Rgb,
        png::BitDepth::Eight,
        1,
        1,
        &[10, 20, 30],
        None,
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("rgb8");
    assert_eq!(px, rgba(&[10, 20, 30, 255]));

    // 16-bit RGB (big-endian, stripped to 8-bit).
    let png = encode_png(
        png::ColorType::Rgb,
        png::BitDepth::Sixteen,
        1,
        1,
        &[0xff, 0xff, 0, 0, 0x80, 0x00],
        None,
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("rgb16");
    assert_eq!(px, rgba(&[255, 0, 128, 255]));

    // RGBA 8-bit.
    let png = encode_png(
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        1,
        1,
        &[10, 20, 30, 40],
        None,
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("rgba8");
    assert_eq!(px, rgba(&[10, 20, 30, 40]));

    // RGBA 16-bit (big-endian, stripped to 8-bit).
    let png = encode_png(
        png::ColorType::Rgba,
        png::BitDepth::Sixteen,
        1,
        1,
        &[0xff, 0xff, 0, 0, 0x80, 0x00, 0x40, 0x00],
        None,
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("rgba16");
    assert_eq!(px, rgba(&[255, 0, 128, 64]));

    // Indexed palette.
    let png = encode_png(
        png::ColorType::Indexed,
        png::BitDepth::Eight,
        2,
        1,
        &[0, 1],
        Some(vec![255, 0, 0, 0, 255, 0]),
    );
    let (_, _, px) = decode_png_rgba8(&png).expect("indexed");
    assert_eq!(px, rgba(&[255, 0, 0, 255, 0, 255, 0, 255]));
}

/// Resource images are sRGB-encoded on disk but stored premultiplied in
/// linear Display P3: an opaque sRGB (1,0,0) texel must come out as linear
/// P3 `(≈0.917, ≈0.200, ≈0.139)`, not the unconverted sRGB `(1,0,0)`.
#[test]
fn resource_image_converts_srgb_to_linear_p3() {
    let dir = tmp().join("res-p3");
    let png = encode_png(
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        1,
        1,
        &[255, 0, 0, 255],
        None,
    );
    let hash = Scene::store_resource(&dir, &png).expect("store");
    let mut res = cherenkov_oracle::Resources::new(dir);
    let img = res
        .image(hash, cherenkov_scene::ImageEncoding::default())
        .expect("decode");
    let want = linear_srgb_to_linear_p3([1.0, 0.0, 0.0]);
    let px = img.pixels[0];
    assert!(
        (px[0] - want[0]).abs() < 2e-3
            && (px[1] - want[1]).abs() < 2e-3
            && (px[2] - want[2]).abs() < 2e-3
            && (px[3] - 1.0).abs() < 2e-3,
        "sRGB→P3 premultiplied: got {px:?}, want ≈{want:?}"
    );
    // The sRGB→P3 primaries differ materially (sRGB red ≈ (0.822,
    // 0.033, 0.017) in P3 coordinates); this fails if the conversion
    // is skipped.
    assert!(want[0] < 0.90 && want[1] < 0.10);
}

/// A PNG tagged `DisplayP3` skips the primaries matrix: encoded red
/// (255,0,0) decodes to linear-P3 exactly (1,0,0), not to sRGB red's
/// P3 coordinates.
#[test]
fn resource_image_display_p3_skips_primaries_conversion() {
    use cherenkov_scene::{ImageColorSpace, ImageEncoding};
    let dir = tmp().join("res-display-p3");
    let png = encode_png(
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        1,
        1,
        &[255, 0, 0, 255],
        None,
    );
    let hash = Scene::store_resource(&dir, &png).expect("store");
    let mut res = cherenkov_oracle::Resources::new(dir);
    let img = res
        .image(
            hash,
            ImageEncoding::Png {
                color_space: ImageColorSpace::DisplayP3,
            },
        )
        .expect("decode");
    let px = img.pixels[0];
    assert!(
        (px[0] - 1.0).abs() < 2e-3 && px[1].abs() < 2e-3 && px[2].abs() < 2e-3,
        "DisplayP3 red must decode to P3 (1,0,0); got {px:?}"
    );
}

/// A `Rgba16F` blob in `LinearP3` is raw half floats straight to the
/// working space — a 16.0 channel survives unclamped.
#[test]
fn resource_image_rgba16f_linear_p3_is_unclamped() {
    use cherenkov_scene::{ImageColorSpace, ImageEncoding};
    let dir = tmp().join("res-f16");
    let mut blob = Vec::new();
    for c in [16.0f32, 0.0, 0.0, 1.0] {
        blob.extend_from_slice(&half::f16::from_f32(c).to_le_bytes());
    }
    let hash = Scene::store_resource(&dir, &blob).expect("store");
    let mut res = cherenkov_oracle::Resources::new(dir);
    let img = res
        .image(
            hash,
            ImageEncoding::Rgba16F {
                width: 1,
                height: 1,
                color_space: ImageColorSpace::LinearP3,
            },
        )
        .expect("decode");
    let px = img.pixels[0];
    assert!(
        (px[0] - 16.0).abs() < 1e-6 && (px[3] - 1.0).abs() < 1e-6,
        "HDR texel must survive: got {px:?}"
    );

    // A truncated blob is an error, not a silent partial image.
    let dir2 = tmp().join("res-f16-short");
    let hash2 = Scene::store_resource(&dir2, &blob[..4]).expect("store");
    let mut res2 = cherenkov_oracle::Resources::new(dir2);
    assert!(
        res2.image(
            hash2,
            ImageEncoding::Rgba16F {
                width: 1,
                height: 1,
                color_space: ImageColorSpace::LinearP3,
            },
        )
        .is_err()
    );
}

/// Exact covered area under winding: two overlapping rectangles
/// `A = [0,0.6]×[0,1]` and `B = [0.24,0.84]×[0,1]` inside one pixel.
///
/// Same orientation — non-zero covers the union `0.84`; even-odd covers
/// the symmetric difference `0.48`. Opposite orientation — non-zero
/// covers only the singly-wound regions, also `0.48`. A signed-area fold
/// answers `1.0`/`0.2`/`0` for these cases instead.
#[test]
fn coverage_overlapping_windings_exact() {
    use cherenkov_oracle::coverage::Coverage;
    use cherenkov_scene::FillRule;
    let rect = |x0: f64, x1: f64, rev: bool| {
        if rev {
            vec![(x0, 0.0), (x0, 1.0), (x1, 1.0), (x1, 0.0)]
        } else {
            vec![(x0, 0.0), (x1, 0.0), (x1, 1.0), (x0, 1.0)]
        }
    };
    let cases = [
        (false, FillRule::NonZero, 0.84),
        (false, FillRule::EvenOdd, 0.48),
        (true, FillRule::NonZero, 0.48),
        (true, FillRule::EvenOdd, 0.48),
    ];
    for (rev, rule, want) in cases {
        let mut c = Coverage::new(1, 1);
        c.add_polyline(&rect(0.0, 0.6, false), true);
        c.add_polyline(&rect(0.24, 0.84, rev), true);
        let area = c.finish(rule)[0];
        assert!(
            (area - want).abs() < 1e-9,
            "rev={rev} {rule:?}: got {area}, want {want}"
        );
    }
}

/// A figure-8 self-intersection: an hourglass bow-tie made of two
/// triangles sharing only the crossing vertex `(0.5, 0.5)` — `0.25` of
/// the pixel each, `0.5` total. The lobes wind oppositely, so a naive
/// signed fold gives `0`; under both fill rules each lobe counts as
/// covered and the exact area is `0.5`.
#[test]
fn coverage_figure8_exact() {
    use cherenkov_oracle::coverage::Coverage;
    use cherenkov_scene::FillRule;
    let mut c = Coverage::new(1, 1);
    // Bow-tie: top triangle and bottom triangle, opposite windings.
    c.add_polyline(&[(0.0, 0.0), (1.0, 0.0), (0.5, 0.5)], true);
    c.add_polyline(&[(0.5, 0.5), (0.0, 1.0), (1.0, 1.0)], true);
    let non_zero = c.finish(FillRule::NonZero)[0];
    let even_odd = c.finish(FillRule::EvenOdd)[0];
    assert!(
        (non_zero - 0.5).abs() < 1e-9,
        "figure-8 non-zero: got {non_zero}, want 0.5"
    );
    assert!(
        (even_odd - 0.5).abs() < 1e-9,
        "figure-8 even-odd: got {even_odd}, want 0.5"
    );
}

/// An hourglass bow-tie `(0,0)→(1,1)→(0,1)→(1,0)` whose two diagonals
/// cross *inside* the pixel — the crossing at `(0.5, 0.5)` is interior
/// to both edges, not a shared vertex. The covered region is two
/// triangles of area `0.25` each; strips must be cut at the crossing
/// y or the midline sampling sees a kinked boundary.
#[test]
fn coverage_bowtie_interior_crossing() {
    use cherenkov_oracle::coverage::Coverage;
    use cherenkov_scene::FillRule;
    let mut c = Coverage::new(1, 1);
    c.add_polyline(&[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0)], true);
    let non_zero = c.finish(FillRule::NonZero)[0];
    let even_odd = c.finish(FillRule::EvenOdd)[0];
    assert!(
        (non_zero - 0.5).abs() < 1e-9,
        "bowtie non-zero: got {non_zero}, want 0.5"
    );
    assert!(
        (even_odd - 0.5).abs() < 1e-9,
        "bowtie even-odd: got {even_odd}, want 0.5"
    );
}

/// Two overlapping squares whose edges cross at interior points of a
/// single pixel: `A = [0.1, 0.9]²` axis-aligned, `B` the same centre
/// rotated 45° with circumradius `0.45` (vertices `(0.5,0.05)`,
/// `(0.95,0.5)`, `(0.5,0.95)`, `(0.05,0.5)`). Each of `B`'s edges
/// crosses two of `A`'s edges inside the pixel.
///
/// `|A| = 0.64`, `|B| = 2·0.45² = 0.405`; `B` pokes out of `A` in four
/// tips (e.g. `(0.5,0.05)-(0.45,0.1)-(0.55,0.1)`, area `0.0025` each),
/// so `|A∩B| = 0.395`. Non-zero covers the union `0.65`; even-odd
/// covers the symmetric difference `0.65 − 0.395 = 0.255`.
#[test]
fn coverage_rotated_squares_exact() {
    use cherenkov_oracle::coverage::Coverage;
    use cherenkov_scene::FillRule;
    let square_a = vec![(0.1, 0.1), (0.9, 0.1), (0.9, 0.9), (0.1, 0.9)];
    let square_b = vec![(0.5, 0.05), (0.95, 0.5), (0.5, 0.95), (0.05, 0.5)];
    for (rule, want) in [(FillRule::NonZero, 0.65), (FillRule::EvenOdd, 0.255)] {
        let mut c = Coverage::new(1, 1);
        c.add_polyline(&square_a, true);
        c.add_polyline(&square_b, true);
        let area = c.finish(rule)[0];
        assert!(
            (area - want).abs() < 1e-9,
            "{rule:?}: got {area}, want {want}"
        );
    }
}

/// Edge-clamped bilinear sampling: a sample coordinate inside the border
/// texel's outer half (`u ∈ [0, 0.5)`) collapses onto the edge texel
/// exactly — it must not blend with the *interior* neighbour under a
/// flipped weight.
#[test]
fn bilinear_clamps_at_edges() {
    use cherenkov_oracle::paint::sample_image;
    use cherenkov_scene::Sampling;
    let img = cherenkov_oracle::Image {
        width: 2,
        height: 2,
        pixels: vec![
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ],
    };
    // Left edge: u in [0, 0.5) must sample column 0 only (v=0.5 sits on
    // row 0's centre, so the sample is texel (0,0) exactly).
    for u in [-0.25, 0.0, 0.2, 0.49] {
        let px = sample_image(&img, u, 0.5, Sampling::Bilinear);
        assert_eq!(
            px,
            [1.0, 0.0, 0.0, 1.0],
            "left edge u={u}: got {px:?}, want pure edge texel"
        );
    }
    // Top edge: v in [0, 0.5) must sample row 0 only.
    for v in [-0.25, 0.0, 0.49] {
        let px = sample_image(&img, 1.5, v, Sampling::Bilinear);
        assert_eq!(
            px,
            [0.0, 1.0, 0.0, 1.0],
            "top edge v={v}: got {px:?}, want pure edge texel"
        );
    }
    // Interior point stays a true blend.
    let px = sample_image(&img, 1.0, 1.0, Sampling::Bilinear);
    let want = [0.5, 0.5, 0.5, 1.0];
    for i in 0..4 {
        assert!(
            (px[i] - want[i]).abs() < 1e-12,
            "centre sample: got {px:?}, want {want:?}"
        );
    }
}

/// Device-space flattening: a circle drawn under a 4× scale must keep
/// `1e-4` px tolerance *after* the transform. The covered area of an
/// `r = 30` circle scaled to `r = 120` is `π·120²`; a user-space-tolerance
/// flattening would under-cover by ~0.5%.
#[test]
fn circle_area_exact_under_scaling() {
    const S: u32 = 96;
    let mut b = Scene::builder(S, S).clear(Color::new(
        cherenkov_scene::ColorSpace::Srgb,
        [0.0, 0.0, 0.0, 0.0],
    ));
    b.root().transform(kurbo::Affine::scale(4.0));
    b.root().fill(
        Shape::Circle(kurbo::Circle::new((12.0, 12.0), 6.0)),
        Paint::Solid(Color::srgb(1.0, 1.0, 1.0)),
    );
    let scene = b.build();
    let img = Renderer::new(S as usize, S as usize)
        .render(&scene, &tmp())
        .expect("render");
    let area: f64 = img.pixels.iter().map(|p| f64::from(p[3])).sum();
    let want = std::f64::consts::PI * 24.0 * 24.0;
    assert!(
        (area - want).abs() / want < 1e-3,
        "scaled circle area: got {area}, want ≈{want}"
    );
}

/// Shadow Gaussian tails: a step-edge blurred with `σ = 4` must track the
/// analytic `½·erfc(d/(σ√2))` out past `3σ` — a kernel truncated at `3σ`
/// reports `0` at `3.5σ`, where the true value is `≈2.7e-4`.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel indices are far below 2^53"
)]
#[test]
fn gaussian_tail_matches_erf() {
    use cherenkov_oracle::shadow::gaussian_blur;
    let sigma = 4.0;
    let n = 96;
    let mut f = vec![0.0_f64; n];
    for v in f.iter_mut().skip(48) {
        *v = 1.0;
    }
    let g = gaussian_blur(&f, n, 1, sigma);
    let inv = 1.0 / (sigma * std::f64::consts::SQRT_2);
    for x in [16usize, 34, 44, 48] {
        let want = 0.5 * libm::erfc((48.0 - (x as f64 + 0.5)) * inv);
        assert!(
            (g[x] - want).abs() < 1e-4,
            "tail x={x}: got {}, want ≈{want}",
            g[x]
        );
    }
    // The point ~3.5σ from the step specifically must be non-zero.
    assert!(g[34] > 1e-5);
}

/// HDR-FLIP runs a fixed exposure ladder — 0 EV (SDR white = 1.0) to
/// +4 EV (16× white) in five steps — recorded on every metrics JSON so
/// reports are self-describing and comparable across scenes.
#[test]
fn hdr_flip_fixed_exposures_recorded() {
    let mut hdr_img = cherenkov_oracle::F32Image {
        width: 2,
        height: 2,
        pixels: vec![[2.0, 0.5, 0.1, 1.0]; 4],
    };
    let (m, _) = metrics::compare(&hdr_img, &hdr_img.clone());
    assert!(m.hdr);
    assert_eq!(m.hdr_flip_exposure_ev, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    // An SDR comparison records the same ladder for consistency.
    hdr_img.pixels = vec![[0.5, 0.5, 0.5, 1.0]; 4];
    let (m2, _) = metrics::compare(&hdr_img, &hdr_img.clone());
    assert!(!m2.hdr);
    assert_eq!(m2.hdr_flip_exposure_ev, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
}

/// `max_local_error` is `max |box3x3(ref) − box3x3(test)|`, not
/// `max box3x3(|ref − test|)`: with `+δ` at `(2,2)` and `−δ` at `(3,2)`,
/// windows containing both cancel (`(δ−δ)/9 = 0`), and the max is `δ/9`,
/// not `2δ/9`.
#[test]
fn max_local_error_filters_then_diffs() {
    let mk = |v: f32| cherenkov_oracle::F32Image {
        width: 7,
        height: 7,
        pixels: vec![[v, v, v, v]; 49],
    };
    let mut r = mk(0.0);
    r.pixels[3 * 7 + 3] = [0.9, 0.9, 0.9, 0.9];
    r.pixels[4 * 7 + 3] = [-0.9, -0.9, -0.9, -0.9];
    let mle = metrics::max_local_error(&r, &mk(0.0));
    assert!(
        (mle - 0.1).abs() < 1e-6,
        "signed-difference cancellation: got {mle}, want 0.1 (box-of-abs gives 0.2)"
    );
}

/// A destructive layer operator is bounded by the effective clip: inside
/// `rect(4,4,8,8)` the clear erases the opaque backdrop; outside it the
/// backdrop is untouched. Unclipped, the same clear covers the whole parent.
#[expect(
    clippy::float_cmp,
    reason = "cleared and opaque-white pixels are exact"
)]
#[test]
fn destructive_layer_blend_is_bounded_by_the_clip() {
    use cherenkov_scene::BlendMode;
    let mut b = Scene::builder(W, H).clear(Color::srgb(1.0, 1.0, 1.0));
    b.root().layer(|a| {
        a.clip(Shape::Rect(Rect::new(4.0, 4.0, 8.0, 8.0)));
        a.blend(BlendMode::Clear);
    });
    let scene = b.build();
    let img = render(&scene, &tmp());
    // (6,6) is inside the clip: cleared to transparent.
    assert_eq!(img.pixels[6 * W as usize + 6], [0.0; 4]);
    // (0,0) is outside: the opaque white backdrop survives.
    assert_eq!(img.pixels[0], [1.0, 1.0, 1.0, 1.0]);
    // (12,12) is outside the clip too.
    assert_eq!(img.pixels[12 * W as usize + 12], [1.0, 1.0, 1.0, 1.0]);

    let mut b = Scene::builder(W, H).clear(Color::srgb(1.0, 1.0, 1.0));
    b.root().layer(|a| {
        a.blend(BlendMode::Clear);
    });
    let scene = b.build();
    let img = render(&scene, &tmp());
    assert!(
        img.pixels.iter().all(|p| *p == [0.0; 4]),
        "unclipped clear covers the whole parent"
    );
}

#[test]
fn refraction_on_a_path_clip_is_the_named_error() {
    use cherenkov_scene::{BackdropEffectSpec, BackdropFilter};
    let mut b = Scene::builder(W, H);
    b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 2.0 }]);
    b.root().layer(|m| {
        let mut path = kurbo::BezPath::new();
        path.move_to((2.0, 2.0));
        path.line_to((14.0, 2.0));
        path.line_to((14.0, 14.0));
        path.close_path();
        m.clip(Shape::Path { path });
        m.backdrop(1);
        m.backdrop_effect(BackdropEffectSpec::Refraction {
            depth: 4.0,
            strength: 2.0,
        });
    });
    let scene = b.build();
    let result = Renderer::new(W as usize, H as usize).render(&scene, &tmp());
    match result {
        Err(e) => assert!(
            e.to_string().contains("backdrop-effect-sdf-path"),
            "unexpected error {e}"
        ),
        Ok(_) => panic!("path clip with an SDF effect must fail"),
    }
}
