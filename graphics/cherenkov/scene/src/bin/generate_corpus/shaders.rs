//! User-shader scenes: shader paints on fills and backdrop shaders on
//! members, each in sRGB-range, wide-gamut and above-SDR colour.

use cherenkov_scene::kurbo::{Ellipse, RoundedRect};
use cherenkov_scene::{
    BackdropEffectSpec, BackdropFilter, BackdropGroup, Color, LayerBuilder, Paint, ShaderPaint,
    Shape,
};

use super::{Corpus, hdr, p3, rotate_about, solid, srgb};

/// The corpus shader paint's source.
const PAINT: &str = include_str!("shaders/paint.wgsl");
/// The corpus backdrop shader's source.
const BACKDROP: &str = include_str!("shaders/backdrop.wgsl");

/// The paint's three corner colours, as premultiplied working-space
/// `vec4`s, and its ring: inner and outer radius in `uv`, alpha outside.
struct PaintColors {
    corners: [[f64; 4]; 3],
    ring: [f64; 4],
}

const PAINT_SRGB: PaintColors = PaintColors {
    corners: [
        [0.80, 0.18, 0.08, 1.0],
        [0.08, 0.25, 0.85, 1.0],
        [0.10, 0.70, 0.25, 1.0],
    ],
    ring: [0.30, 0.45, 0.35, 0.0],
};

/// Corners outside the sRGB gamut: the P3 primaries' linear values.
const PAINT_P3: PaintColors = PaintColors {
    corners: [
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
    ],
    ring: [0.30, 0.45, 0.35, 0.0],
};

/// Corners above SDR white.
const PAINT_HDR: PaintColors = PaintColors {
    corners: [
        [2.4, 0.6, 0.2, 1.0],
        [0.3, 0.9, 3.0, 1.0],
        [1.2, 2.0, 0.4, 1.0],
    ],
    ring: [0.30, 0.45, 0.35, 0.0],
};

fn shader_paint(colors: &PaintColors) -> Paint {
    Paint::Shader(ShaderPaint {
        source: PAINT.to_owned(),
        params: colors
            .corners
            .iter()
            .chain([&colors.ring])
            .flatten()
            .copied()
            .collect(),
    })
}

/// The backdrop shader's tint, rim colour and gain; `stripes` and `discs`
/// are the backdrop it lenses.
struct BackdropColors {
    tint: [f64; 3],
    rim: [f64; 4],
    stripes: [Color; 2],
    discs: [Color; 3],
}

const BACKDROP_SRGB: BackdropColors = BackdropColors {
    tint: [0.85, 0.95, 1.0],
    rim: [1.0, 0.9, 0.7, 0.8],
    stripes: [srgb(0.05, 0.30, 0.95), srgb(0.95, 0.25, 0.05)],
    discs: [
        srgb(0.95, 0.75, 0.05),
        srgb(0.10, 0.85, 0.40),
        srgb(0.70, 0.10, 0.90),
    ],
};

/// P3-only stripes and discs under a rim that lands above SDR white.
const BACKDROP_P3_HDR: BackdropColors = BackdropColors {
    tint: [0.85, 0.95, 1.0],
    rim: [1.0, 0.85, 0.6, 3.0],
    stripes: [p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 0.0)],
    discs: [hdr(1.8, 1.4, 0.1), p3(0.0, 1.0, 0.0), hdr(1.2, 0.2, 2.2)],
};

/// The lens: depth and strength in pixels, then the pyramid level at the
/// edge.
const LENS: [f64; 3] = [14.0, 6.0, 2.0];

/// A member clipped to `clip` under `transform` with the corpus backdrop
/// shader; `top` is the clip's top edge in the member's own space.
fn shader_member(
    l: &mut LayerBuilder,
    colors: &BackdropColors,
    clip: Shape,
    top: f64,
    transform: cherenkov_scene::kurbo::Affine,
) {
    l.layer(|m| {
        m.transform(transform);
        m.clip(clip);
        m.backdrop(1);
        m.backdrop_effect(BackdropEffectSpec::Shader {
            source: BACKDROP.to_owned(),
            params: [
                [colors.tint[0], colors.tint[1], colors.tint[2], 0.0],
                [LENS[0], LENS[1], LENS[2], 0.0],
                colors.rim,
                [0.0, top, 0.0, 0.0],
            ]
            .into_iter()
            .flatten()
            .collect(),
            reach: LENS[1],
        });
    });
}

pub fn generate(corpus: &mut Corpus, clear: Color) {
    for (suffix, colors) in [("", &PAINT_SRGB), ("-p3", &PAINT_P3), ("-hdr", &PAINT_HDR)] {
        corpus.scene(format!("shader-paint{suffix}"), 256, 256, clear, |l| {
            l.fill(
                Shape::RoundedRect(RoundedRect::new(24.0, 20.0, 232.0, 124.0, 18.0)),
                shader_paint(colors),
            );
            // Under a rotation the paint spans the shape's user-space
            // bounds, not its device box.
            l.layer(|m| {
                m.transform(rotate_about(0.35, (128.0, 188.0)));
                m.fill(
                    Shape::Ellipse(Ellipse::new((128.0, 188.0), (84.0, 52.0), 0.0)),
                    shader_paint(colors),
                );
            });
        });
    }

    for (suffix, colors) in [("", &BACKDROP_SRGB), ("-p3-hdr", &BACKDROP_P3_HDR)] {
        corpus.scene_setup(format!("backdrop-shader{suffix}"), 256, 256, clear, |b| {
            b.backdrop_group(BackdropGroup::new(
                1,
                vec![BackdropFilter::GaussianBlur { sigma: 2.0 }],
                1.0,
                3,
            ));
            let l = &mut b.root();
            for i in 0..8 {
                let x0 = 32.0 * f64::from(i);
                l.fill(Shape::rect(x0, 0.0, 16.0, 256.0), solid(colors.stripes[0]));
                l.fill(
                    Shape::rect(x0 + 16.0, 0.0, 16.0, 256.0),
                    solid(colors.stripes[1]),
                );
            }
            l.fill(Shape::circle(64.0, 200.0, 36.0), solid(colors.discs[0]));
            l.fill(Shape::circle(192.0, 56.0, 28.0), solid(colors.discs[1]));
            l.fill(Shape::circle(224.0, 224.0, 24.0), solid(colors.discs[2]));
            shader_member(
                l,
                colors,
                Shape::RoundedRect(RoundedRect::new(24.0, 24.0, 136.0, 112.0, 20.0)),
                24.0,
                cherenkov_scene::kurbo::Affine::IDENTITY,
            );
            // A rotated member: `local` is its own space, not the device.
            shader_member(
                l,
                colors,
                Shape::RoundedRect(RoundedRect::new(120.0, 140.0, 232.0, 220.0, 16.0)),
                140.0,
                rotate_about(-0.3, (176.0, 180.0)),
            );
        });
    }
}
