//! Writes the initial scene corpus to `scenes/corpus/`.
//!
//! Run `prepare-fonts` first: it produces the OFL subsets in `scenes/fonts/`
//! that this binary shapes against with parley. Each scene lands in
//! `scenes/corpus/<name>/` as a `scene.json` plus a `resources/` directory of
//! BLAKE3-addressed blobs (fonts, images).

#[path = "generate_corpus/authored.rs"]
mod authored;
#[path = "generate_corpus/text_layout.rs"]
mod text_layout;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use cherenkov_scene::corpus;
use cherenkov_scene::kurbo::{
    Affine, BezPath, Ellipse, Line, Point, Rect, RoundedRect, RoundedRectRadii, Vec2,
};
use cherenkov_scene::{
    BackdropEffectSpec, BackdropFilter, BlendMode, Color, ColorSpace, Draw, Extend, FillRule,
    FilterBlend, Glyph, GlyphRun, GradientStop, ImageColorSpace, ImageEncoding, ImagePaint,
    LayerBuilder, LayerFilter, LinearGradient, Live, Motion, MotionAnimation, NormalizedCoord,
    Paint, Projection, RadialGradient, ResourceHash, Sampling, Scene, SceneBuilder, SceneError,
    Shape, StrokeStyle, SweepGradient,
};
use fontique::FontWeight;
use parley::{
    FontContext, LayoutContext, PositionedLayoutItem, StyleProperty, fontique::Blob,
    style::FontFamily,
};
use read_fonts::types::F2Dot14;
use skrifa::MetadataProvider;

/// `kurbo::Affine::rotate` evaluated with `libm` so the sine/cosine
/// coefficients cannot drift one ulp between host libms.
fn rotate(th: f64) -> Affine {
    let (s, c) = libm::sincos(th);
    Affine::new([c, s, -s, c, 0.0, 0.0])
}

/// `kurbo::Affine::rotate_about` over [`rotate`].
fn rotate_about(th: f64, center: impl Into<Point>) -> Affine {
    let center = center.into().to_vec2();
    Affine::translate(center) * rotate(th) * Affine::translate(-center)
}

/// Padding around text scenes so ascenders/descenders stay inside.
const TEXT_PAD: f32 = 12.0;
/// Maximum line advance for shaped text.
const TEXT_WRAP: f32 = 280.0;

const FILTER_COLOR_MATRIX: [f64; 12] =
    [0.2, 0.7, 0.1, 0.0, 0.6, 0.3, 0.1, 0.05, 0.1, 0.1, 0.8, 0.0];
const FILTER_SEPIA_MATRIX: [f64; 12] = [
    0.393, 0.769, 0.189, 0.0, 0.349, 0.686, 0.168, 0.0, 0.272, 0.534, 0.131, 0.0,
];
const FILTER_CHAIN_SECOND: [f64; 12] = [
    1.2, 0.0, 0.0, -0.1, 0.0, 1.2, 0.0, -0.1, 0.0, 0.0, 1.2, -0.1,
];

const fn srgb(r: f32, g: f32, b: f32) -> Color {
    Color::srgb(r, g, b)
}

const fn srgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::srgb(r, g, b).with_alpha(a)
}

const fn solid(c: Color) -> Paint {
    Paint::Solid(c)
}

/// A colour in the linear Display P3 working space (wide gamut).
const fn p3(r: f32, g: f32, b: f32) -> Color {
    Color::new(ColorSpace::LinearP3, [r, g, b, 1.0])
}

/// An HDR colour in the linear Display P3 working space (channels > 1).
const fn hdr(r: f32, g: f32, b: f32) -> Color {
    Color::new(ColorSpace::LinearP3, [r, g, b, 1.0])
}

#[derive(Clone, Copy)]
struct FilterColorSet {
    gradient_space: ColorSpace,
    gradient_start: Color,
    gradient_end: Color,
    matrix_circle: Color,
    matrix_rounded_rect: Color,
    blur_rect: Color,
    blur_circle: Color,
    blur_star: Color,
    blur_stroke: Color,
    blend_round_rect: Color,
    blend_circle: Color,
    blend_rect: Color,
    nested_blue: Color,
    nested_red: Color,
    blended_descendant: Color,
    nested_blend_outer: Color,
}

const FILTER_COLORS_SRGB: FilterColorSet = FilterColorSet {
    gradient_space: ColorSpace::Srgb,
    gradient_start: srgb(1.0, 0.0, 0.0),
    gradient_end: srgb(0.0, 0.0, 1.0),
    matrix_circle: srgb(0.15, 0.8, 0.28),
    matrix_rounded_rect: srgb(0.18, 0.3, 0.9),
    blur_rect: srgb(0.9, 0.15, 0.12),
    blur_circle: srgb(0.12, 0.35, 0.9),
    blur_star: srgb(0.95, 0.65, 0.08),
    blur_stroke: srgb(0.12, 0.62, 0.24),
    blend_round_rect: srgba(0.92, 0.18, 0.22, 0.72),
    blend_circle: srgba(0.1, 0.75, 0.88, 0.68),
    blend_rect: srgba(0.74, 0.24, 0.82, 0.66),
    nested_blue: srgb(0.12, 0.32, 0.9),
    nested_red: srgb(0.9, 0.24, 0.12),
    blended_descendant: Color::new(ColorSpace::DisplayP3, [0.0, 0.85, 0.3, 1.0]),
    nested_blend_outer: Color::new(ColorSpace::LinearSrgb, [2.0, 0.3, 0.1, 1.0]),
};

const FILTER_COLORS_P3: FilterColorSet = FilterColorSet {
    gradient_space: ColorSpace::LinearP3,
    gradient_start: p3(1.0, 0.0, 0.0),
    gradient_end: p3(0.0, 1.0, 0.0),
    matrix_circle: p3(0.0, 1.0, 0.0),
    matrix_rounded_rect: p3(0.0, 0.0, 1.0),
    blur_rect: p3(1.0, 0.0, 0.0),
    blur_circle: p3(0.0, 1.0, 0.0),
    blur_star: p3(0.0, 0.0, 1.0),
    blur_stroke: p3(0.0, 1.0, 0.0),
    blend_round_rect: p3(1.0, 0.0, 0.0).with_alpha(0.72),
    blend_circle: p3(0.0, 1.0, 0.0).with_alpha(0.68),
    blend_rect: p3(0.0, 0.0, 1.0).with_alpha(0.66),
    nested_blue: p3(0.0, 0.0, 1.0),
    nested_red: p3(1.0, 0.0, 0.0),
    blended_descendant: p3(0.0, 1.0, 0.0),
    nested_blend_outer: p3(1.0, 0.0, 0.0),
};

const FILTER_COLORS_HDR: FilterColorSet = FilterColorSet {
    gradient_space: ColorSpace::LinearP3,
    gradient_start: hdr(7.0, 2.0, 0.2),
    gradient_end: hdr(0.2, 5.0, 7.0),
    matrix_circle: hdr(2.0, 6.0, 1.0),
    matrix_rounded_rect: hdr(2.0, 1.0, 7.0),
    blur_rect: hdr(6.0, 2.0, 1.0),
    blur_circle: hdr(0.5, 3.0, 7.0),
    blur_star: hdr(5.0, 4.0, 0.5),
    blur_stroke: hdr(1.0, 4.0, 2.0),
    blend_round_rect: hdr(7.0, 2.0, 1.0).with_alpha(0.72),
    blend_circle: hdr(0.5, 6.0, 7.0).with_alpha(0.68),
    blend_rect: hdr(5.0, 1.0, 6.0).with_alpha(0.66),
    nested_blue: hdr(1.0, 2.0, 7.0),
    nested_red: hdr(7.0, 2.0, 1.0),
    blended_descendant: hdr(1.0, 6.0, 1.5),
    nested_blend_outer: hdr(7.0, 2.0, 1.0),
};

fn stops2() -> Vec<GradientStop> {
    vec![
        GradientStop {
            offset: 0.0,
            color: srgb(1.0, 0.0, 0.0),
        },
        GradientStop {
            offset: 1.0,
            color: srgb(0.0, 0.0, 1.0),
        },
    ]
}

fn stops8() -> Vec<GradientStop> {
    const RAINBOW: [[f32; 3]; 8] = [
        [1.0, 0.0, 0.0],
        [1.0, 0.5, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.5, 0.0, 1.0],
        [1.0, 0.0, 0.5],
    ];
    RAINBOW
        .iter()
        .zip(0u16..)
        .map(|([r, g, b], i)| GradientStop {
            offset: f32::from(i) / 7.0,
            color: srgb(*r, *g, *b),
        })
        .collect()
}

/// Fonts loaded from `scenes/fonts/`, plus a parley context to shape with.
struct TextContext {
    fcx: FontContext,
    lcx: LayoutContext<[u8; 4]>,
    /// Family name per subset file name.
    families: BTreeMap<&'static str, String>,
    /// Every registered font blob, keyed by its content hash.
    blobs: BTreeMap<ResourceHash, Vec<u8>>,
    /// Content hash per font file name.
    hashes: BTreeMap<&'static str, ResourceHash>,
}

impl TextContext {
    fn new(fonts_dir: &Path) -> Result<Self, SceneError> {
        let mut fcx = FontContext::new();
        let mut families = BTreeMap::new();
        let mut blobs = BTreeMap::new();
        let mut hashes = BTreeMap::new();
        for spec in corpus::FONTS {
            let path = fonts_dir.join(spec.subset_file);
            let bytes = std::fs::read(&path)?;
            let hash = ResourceHash::of(&bytes);
            let registered = fcx
                .collection
                .register_fonts(Blob::new(Arc::new(bytes.clone())), None);
            let (family_id, _) = registered
                .first()
                .unwrap_or_else(|| panic!("font {} registered no family", spec.subset_file));
            let family_id = *family_id;
            let name = fcx
                .collection
                .family_name(family_id)
                .unwrap_or_else(|| panic!("font {} has no family name", spec.subset_file));
            families.insert(spec.subset_file, name.to_string());
            hashes.insert(spec.subset_file, hash);
            blobs.insert(hash, bytes);
        }
        for file in corpus::TEST_FONTS {
            let bytes = std::fs::read(fonts_dir.join(*file))?;
            let hash = ResourceHash::of(&bytes);
            let registered = fcx
                .collection
                .register_fonts(Blob::new(Arc::new(bytes.clone())), None);
            let (family_id, _) = registered
                .first()
                .unwrap_or_else(|| panic!("font {file} registered no family"));
            let name = fcx
                .collection
                .family_name(*family_id)
                .unwrap_or_else(|| panic!("font {file} has no family name"));
            families.insert(*file, name.to_string());
            hashes.insert(*file, hash);
            blobs.insert(hash, bytes);
        }
        for file in corpus::BITMAP_FONTS {
            let path = fonts_dir.join(file);
            let bytes = std::fs::read(&path)?;
            let hash = ResourceHash::of(&bytes);
            let registered = fcx
                .collection
                .register_fonts(Blob::new(Arc::new(bytes.clone())), None);
            let (family_id, _) = registered
                .first()
                .unwrap_or_else(|| panic!("font {file} registered no family"));
            let family_id = *family_id;
            let name = fcx
                .collection
                .family_name(family_id)
                .unwrap_or_else(|| panic!("font {file} has no family name"));
            families.insert(file, name.to_string());
            hashes.insert(file, hash);
            blobs.insert(hash, bytes);
        }
        Ok(Self {
            fcx,
            lcx: LayoutContext::new(),
            families,
            blobs,
            hashes,
        })
    }

    /// Shape `text` in the font loaded from `file` and return scene glyph
    /// runs positioned with `TEXT_PAD` padding.
    fn shape(
        &mut self,
        file: &str,
        text: &str,
        size: f32,
        weight: FontWeight,
        paint: &Paint,
    ) -> Vec<GlyphRun> {
        let family = self.families[file].clone();
        let mut builder = self.lcx.ranged_builder(&mut self.fcx, text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(
            family.as_str(),
        )));
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::FontWeight(weight));
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(TEXT_WRAP));

        let mut runs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else {
                    continue;
                };
                let run = gr.run();
                let font_data = run.font();
                let data: &[u8] = font_data.data.data();
                let font = skrifa::FontRef::new(data).expect("registered font parses");
                let axes: Vec<String> = font.axes().iter().map(|a| a.tag().to_string()).collect();
                let coords = run
                    .normalized_coords()
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| **v != 0)
                    .map(|(i, v)| NormalizedCoord {
                        tag: axes.get(i).cloned().unwrap_or_default(),
                        value: F2Dot14::from_bits(*v).to_f32(),
                    })
                    .collect();
                let glyphs = gr
                    .positioned_glyphs()
                    .map(|g| Glyph {
                        id: g.id,
                        x: g.x + TEXT_PAD,
                        y: g.y + TEXT_PAD,
                        transform: None,
                    })
                    .collect();
                runs.push(GlyphRun {
                    stroke: None,
                    font: ResourceHash::of(data),
                    font_index: font_data.index,
                    size,
                    normalized_coords: coords,
                    glyphs,
                    paint: paint.clone(),
                });
            }
        }
        runs
    }
}

/// A 96×96 clipped destructive-layer scene: gradient backdrop, then a
/// rounded-rect clipped layer blending a circle gradient with `mode`.
fn blend_clip_scene(
    corpus: &mut Corpus,
    name: String,
    mode: BlendMode,
    backdrop: [Color; 2],
    content: [Color; 2],
    interpolation: ColorSpace,
) {
    corpus.scene(name, 96, 96, srgb(0.7, 0.5, 0.2), |l| {
        l.fill(
            Shape::rect(0.0, 0.0, 96.0, 96.0),
            Paint::Linear(LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(96.0, 96.0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: backdrop[0],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: backdrop[1],
                    },
                ],
                extend: Extend::Pad,
                interpolation,
            }),
        );
        l.layer(|a| {
            a.clip(Shape::rounded_rect(16.0, 16.0, 64.0, 64.0, 14.0));
            a.blend(mode);
            a.fill(
                Shape::circle(48.0, 48.0, 24.0),
                Paint::Linear(LinearGradient {
                    start: Point::new(24.0, 24.0),
                    end: Point::new(72.0, 72.0),
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: content[0],
                        },
                        GradientStop {
                            offset: 1.0,
                            color: content[1],
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation,
                }),
            );
        });
    });
}

/// The font blob a [`GlyphRun`] was shaped from.
fn font_blob<'a>(ctx: &'a TextContext, run: &'a GlyphRun) -> &'a Vec<u8> {
    &ctx.blobs[&run.font]
}

/// A `blend-*` scene under a rounded clip: full-canvas gradient backdrop,
/// then a layer clipped to a rounded rect whose gradient content overruns
/// the clip so the anti-aliased clip edge has source under it.
fn blend_mode_clip_scene(
    corpus: &mut Corpus,
    name: String,
    mode: BlendMode,
    backdrop: [Color; 2],
    content: [Color; 2],
    interpolation: ColorSpace,
) {
    corpus.scene(name, 96, 96, srgb(0.7, 0.5, 0.2), |l| {
        l.fill(
            Shape::rect(0.0, 0.0, 96.0, 96.0),
            Paint::Linear(LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(96.0, 96.0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: backdrop[0],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: backdrop[1],
                    },
                ],
                extend: Extend::Pad,
                interpolation,
            }),
        );
        l.layer(|a| {
            a.clip(Shape::rounded_rect(16.0, 16.0, 64.0, 64.0, 14.0));
            a.blend(mode);
            a.fill(
                Shape::circle(48.0, 48.0, 40.0),
                Paint::Linear(LinearGradient {
                    start: Point::new(24.0, 24.0),
                    end: Point::new(72.0, 72.0),
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: content[0],
                        },
                        GradientStop {
                            offset: 1.0,
                            color: content[1],
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation,
                }),
            );
        });
    });
}

fn encode_png_rgba(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(pixels).expect("png data");
    }
    out
}

/// An 8x8 two-colour checker pattern.
fn checker_png() -> Vec<u8> {
    let mut px = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8u8 {
        for x in 0..8u8 {
            if (x + y) % 2 == 0 {
                px.extend_from_slice(&[255, 255, 255, 255]);
            } else {
                px.extend_from_slice(&[0, 32, 200, 255]);
            }
        }
    }
    encode_png_rgba(8, 8, &px)
}

/// A 16x16 gradient with an alpha ramp.
fn gradient_png() -> Vec<u8> {
    let mut px = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u8 {
        for x in 0..16u8 {
            px.extend_from_slice(&[
                x * 16,
                128,
                y * 16,
                u8::try_from(u16::from(x) * 255 / 15).expect("ramp alpha fits u8"),
            ]);
        }
    }
    encode_png_rgba(16, 16, &px)
}

/// A `width`×`height` PNG whose bytes are sRGB-transfer-encoded Display P3 values:
/// a ramp from P3 red through P3 green (both outside the sRGB gamut).
fn p3_png(width: u32, height: u32) -> Vec<u8> {
    let w = u16::try_from(width).expect("canvas width fits u16");
    let h = u16::try_from(height).expect("canvas height fits u16");
    let mut px = Vec::with_capacity(usize::from(w) * usize::from(h) * 4);
    for row in 0..h {
        for col in 0..w {
            let t = f32::from(col) / f32::from((w - 1).max(1));
            let dark = f32::from(row) / f32::from((h - 1).max(1));
            // Linear P3 red→green horizontally, darkened vertically.
            let lin = [1.0 - t, t, dark];
            for c in lin {
                #[expect(
                    clippy::suboptimal_flops,
                    reason = "the mul_add rewrite changes the encoded byte; corpus pixels are pinned"
                )]
                let enc = if c <= 0.003_130_8 {
                    c * 12.92
                } else {
                    1.055 * libm::powf(c, 1.0 / 2.4) - 0.055
                };
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped and rounded to an integral value in 0..=255"
                )]
                let byte = (enc.clamp(0.0, 1.0) * 255.0).round() as u8;
                px.push(byte);
            }
            px.push(255);
        }
    }
    encode_png_rgba(width, height, &px)
}

/// A `w`×`h` `Rgba16F` blob in linear Display P3 (straight alpha) filled by
/// `f(x, y) -> [r, g, b, a]` in `f32` linear light.
fn rgba16f_blob(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(w as usize * h as usize * 8);
    for y in 0..h {
        for x in 0..w {
            for c in f(x, y) {
                blob.extend_from_slice(&half::f16::from_f32(c).to_le_bytes());
            }
        }
    }
    blob
}

fn filter_image_png() -> Vec<u8> {
    let mut px = Vec::with_capacity(128 * 128 * 4);
    for y in 0u8..128 {
        for x in 0u8..128 {
            let blue = u8::try_from((u16::from(x) + u16::from(y)) * 255 / 254)
                .expect("gradient channel is at most 255");
            let color = if (16..40).contains(&x) && (16..40).contains(&y) {
                [240, 36, 48, 255]
            } else if (82..110).contains(&x) && (18..42).contains(&y) {
                [24, 208, 196, 255]
            } else if (18..44).contains(&x) && (82..108).contains(&y) {
                [244, 204, 24, 255]
            } else if (82..110).contains(&x) && (82..108).contains(&y) {
                [144, 48, 224, 255]
            } else {
                [x * 2, y * 2, blue, 255]
            };
            px.extend_from_slice(&color);
        }
    }
    encode_png_rgba(128, 128, &px)
}

fn filter_color_content(l: &mut LayerBuilder<'_>, colors: &FilterColorSet) {
    l.fill(
        Shape::rect(20.0, 20.0, 88.0, 88.0),
        Paint::Linear(LinearGradient {
            start: Point::new(24.0, 28.0),
            end: Point::new(104.0, 100.0),
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: colors.gradient_start,
                },
                GradientStop {
                    offset: 1.0,
                    color: colors.gradient_end,
                },
            ],
            extend: Extend::Pad,
            interpolation: colors.gradient_space,
        }),
    );
    l.fill(Shape::circle(48.0, 64.0, 16.0), solid(colors.matrix_circle));
    l.fill(
        Shape::rounded_rect(68.0, 40.0, 28.0, 48.0, 6.0),
        solid(colors.matrix_rounded_rect),
    );
}

fn filter_blur_content(l: &mut LayerBuilder<'_>, colors: &FilterColorSet) {
    l.fill(Shape::rect(30.0, 30.0, 68.0, 68.0), solid(colors.blur_rect));
    l.fill(Shape::circle(84.0, 48.0, 18.0), solid(colors.blur_circle));
    l.fill(
        Shape::Path {
            path: star_path(68.0, 78.0, 8.0, 18.0),
        },
        solid(colors.blur_star),
    );
    l.stroke(
        Shape::Line(Line::new((30.0, 96.0), (98.0, 96.0))),
        StrokeStyle {
            width: 1.0,
            ..StrokeStyle::default()
        },
        solid(colors.blur_stroke),
    );
}

fn filter_blend_content(l: &mut LayerBuilder<'_>, colors: &FilterColorSet) {
    l.fill(
        Shape::rounded_rect(24.0, 24.0, 80.0, 80.0, 8.0),
        solid(colors.blend_round_rect),
    );
    l.fill(Shape::circle(50.0, 62.0, 20.0), solid(colors.blend_circle));
    l.fill(
        Shape::rect(64.0, 52.0, 32.0, 36.0),
        solid(colors.blend_rect),
    );
}

fn filter_nested_content(l: &mut LayerBuilder<'_>, colors: &FilterColorSet) {
    for y in [
        15.0, 16.0, 31.0, 32.0, 47.0, 48.0, 127.0, 128.0, 191.0, 192.0,
    ] {
        l.fill(Shape::rect(26.0, y, 12.0, 1.0), solid(colors.nested_blue));
    }
    l.fill(
        Shape::rect(26.0, 122.0, 12.0, 12.0),
        solid(colors.nested_red),
    );
}

fn filter_blended_descendant_content(l: &mut LayerBuilder<'_>, colors: &FilterColorSet) {
    l.fill(
        Shape::rect(8.0, 8.0, 48.0, 80.0),
        solid(colors.blended_descendant),
    );
    l.layer(|cutout| {
        cutout.blend(BlendMode::DestOut);
        cutout.fill(
            Shape::rect(20.0, 30.0, 24.0, 36.0),
            solid(srgb(1.0, 1.0, 1.0)),
        );
    });
}

fn filter_isolates_nested_blend_content(l: &mut LayerBuilder<'_>, colors: &FilterColorSet) {
    l.fill(
        Shape::rect(4.0, 4.0, 56.0, 88.0),
        solid(colors.nested_blend_outer),
    );
    l.layer(|inner| {
        inner.fill(
            Shape::rect(12.0, 20.0, 40.0, 56.0),
            solid(colors.nested_blue),
        );
        inner.layer(|cutout| {
            cutout.blend(BlendMode::DestOut);
            cutout.fill(
                Shape::rect(24.0, 36.0, 16.0, 24.0),
                solid(srgb(1.0, 1.0, 1.0)),
            );
        });
    });
}

/// A self-intersecting figure-eight-ish cubic path.
fn self_intersecting_path() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((20.0, 64.0));
    p.curve_to((20.0, 10.0), (108.0, 10.0), (108.0, 64.0));
    p.curve_to((108.0, 118.0), (20.0, 118.0), (20.0, 64.0));
    p.curve_to((20.0, 30.0), (108.0, 30.0), (108.0, 64.0));
    p.curve_to((108.0, 98.0), (20.0, 98.0), (20.0, 64.0));
    p.close_path();
    p
}

/// Two same-direction circles overlapping as one `NonZero` path (their
/// union renders flat), a self-crossing bow-tie quad, and two nested
/// same-direction squares for `EvenOdd` hole-punching.
fn overlap_winding_path() -> (BezPath, BezPath, BezPath) {
    // κ·r for a four-cubic circle approximation.
    const R: f64 = 28.0;
    const KR: f64 = 0.5523 * R;
    let mut circles = BezPath::new();
    for (cx, cy) in [(44.3, 52.6), (72.7, 60.2)] {
        circles.move_to((cx + R, cy));
        circles.curve_to((cx + R, cy - KR), (cx + KR, cy - R), (cx, cy - R));
        circles.curve_to((cx - KR, cy - R), (cx - R, cy - KR), (cx - R, cy));
        circles.curve_to((cx - R, cy + KR), (cx - KR, cy + R), (cx, cy + R));
        circles.curve_to((cx + KR, cy + R), (cx + R, cy + KR), (cx + R, cy));
        circles.close_path();
    }
    let mut bow_tie = BezPath::new();
    bow_tie.move_to((20.0, 90.0));
    bow_tie.line_to((108.0, 120.0));
    bow_tie.line_to((108.0, 90.0));
    bow_tie.line_to((20.0, 120.0));
    bow_tie.close_path();
    let mut squares = BezPath::new();
    for (x0, y0, x1, y1) in [(12.0, 12.0, 52.0, 52.0), (22.5, 22.5, 41.5, 41.5)] {
        squares.move_to((x0, y0));
        squares.line_to((x1, y0));
        squares.line_to((x1, y1));
        squares.line_to((x0, y1));
        squares.close_path();
    }
    (circles, bow_tie, squares)
}

/// A path mixing lines and curves.
fn curved_path() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((16.0, 96.0));
    p.curve_to((16.0, 30.0), (64.0, 10.0), (64.0, 48.0));
    p.quad_to((64.0, 86.0), (96.0, 96.0));
    p.line_to((112.0, 96.0));
    p.line_to((112.0, 48.0));
    p.curve_to((112.0, 20.0), (88.0, 12.0), (64.0, 24.0));
    p.close_path();
    p
}

/// Two concentric stars for even-odd vs non-zero comparison.
fn star_path(cx: f64, cy: f64, r0: f64, r1: f64) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..10 {
        let angle = f64::from(i) * std::f64::consts::TAU / 10.0 - std::f64::consts::FRAC_PI_2;
        let r = if i % 2 == 0 { r1 } else { r0 };
        let pt = (
            r.mul_add(libm::cos(angle), cx),
            r.mul_add(libm::sin(angle), cy),
        );
        if i == 0 {
            p.move_to(pt);
        } else {
            p.line_to(pt);
        }
    }
    p.close_path();
    p
}

/// Clone `run` with all glyph positions shifted by `(dx, dy)`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "glyph offsets stay inside the f32 position range"
)]
fn offset_run(run: &GlyphRun, dx: f64, dy: f64) -> GlyphRun {
    let mut r = run.clone();
    for g in &mut r.glyphs {
        g.x += dx as f32;
        g.y += dy as f32;
    }
    r
}

/// The distinct font blobs a set of glyph runs needs, in hash order.
fn font_blobs(ctx: &TextContext, runs: &[&[GlyphRun]]) -> Vec<Vec<u8>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for group in runs {
        for r in *group {
            if seen.insert(r.font) {
                out.push(font_blob(ctx, r).clone());
            }
        }
    }
    out
}

/// A `Rect` -> [`Shape`] constructor in the stroke-join table.
type ShapeBuild = fn(Rect) -> Shape;

/// xorshift64* — deterministic pseudo-random content without a `rand` dep.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform sample in `[0, 1)`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "only the top 53 bits are sampled"
    )]
    fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform sample in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }
}

/// A small right-pointing chevron centred at `(x, y)`.
fn chevron(x: f64, y: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x - 8.0, y - 12.0));
    p.line_to((x + 8.0, y));
    p.line_to((x - 8.0, y + 12.0));
    p.line_to((x - 4.0, y));
    p.close_path();
    p
}

/// One silhouette-shadow corpus scene: `shape` under `transform` with a
/// shadow, then the same shape filled on a light clear.
fn silhouette_scene(
    corpus: &mut Corpus,
    name: &str,
    shape: Shape,
    transform: Affine,
    color: Color,
) {
    corpus.scene(name, 128, 128, srgb(0.95, 0.95, 0.95), |l| {
        l.transform(transform);
        l.shadow(shape.clone(), 3.0, [7.0, 8.0], color);
        l.fill(shape, solid(srgb(1.0, 0.65, 0.15)));
    });
}

/// The star silhouette the shadow scenes share.
fn shadow_star() -> Shape {
    let mut path = star_path(64.0, 64.0, 24.0, 56.0);
    path.extend(star_path(64.0, 64.0, 10.0, 24.0));
    Shape::Path { path }
}

/// The ellipse silhouette the shadow scenes share.
fn shadow_ellipse() -> Shape {
    Shape::Ellipse(Ellipse::new((48.0, 48.0), (40.0, 24.0), 0.0))
}

/// The uneven-corner silhouette the shadow scenes share.
const fn shadow_uneven() -> Shape {
    Shape::Continuous(cherenkov_scene::ContinuousRect::new(
        Rect::new(16.0, 16.0, 80.0, 80.0),
        28.0,
        1.0,
    ))
}

/// Write `corpus` into `dir`, one `<name>/` per entry.
fn write_corpus(out: &Path, corpus: &Corpus) -> Result<(), SceneError> {
    std::fs::create_dir_all(out)?;
    for entry in &corpus.entries {
        let dir = out.join(&entry.name);
        match &entry.body {
            EntryBody::Scene(scene) => scene.save(&dir)?,
            EntryBody::Json(text) => {
                std::fs::create_dir_all(&dir)?;
                std::fs::write(dir.join("scene.json"), text)?;
            }
        }
        for blob in &entry.blobs {
            Scene::store_resource(&dir, blob)?;
        }
    }
    Ok(())
}

/// One emitted scene plus the blobs its `resources/` needs.
struct Entry {
    name: String,
    body: EntryBody,
    blobs: Vec<Vec<u8>>,
}

/// How an entry's `scene.json` is produced: the typed builder, or text
/// laid out byte-for-byte the way the hand-committed scenes were written
/// (see `authored`).
enum EntryBody {
    Scene(Box<Scene>),
    Json(String),
}

struct Corpus {
    entries: Vec<Entry>,
}

impl Corpus {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Build a `w`x`h` scene via `f` and queue it for output.
    fn scene(
        &mut self,
        name: impl Into<String>,
        w: u32,
        h: u32,
        clear: Color,
        f: impl FnOnce(&mut LayerBuilder),
    ) {
        self.scene_with_blobs(name, w, h, clear, f, Vec::new());
    }

    /// Like [`Corpus::scene`] but `f` gets the whole builder — for scenes
    /// that declare scene-level state such as backdrop groups.
    fn scene_setup(
        &mut self,
        name: impl Into<String>,
        w: u32,
        h: u32,
        clear: Color,
        f: impl FnOnce(&mut SceneBuilder),
    ) {
        let mut builder = Scene::builder(w, h).clear(clear);
        f(&mut builder);
        let scene = builder.build();
        self.entries.push(Entry {
            name: name.into(),
            body: EntryBody::Scene(Box::new(scene)),
            blobs: Vec::new(),
        });
    }

    /// Queue a scene whose `scene.json` is already rendered text (the
    /// hand-authored scenes in `authored`).
    fn push_json(&mut self, name: &str, text: String, blobs: Vec<Vec<u8>>) {
        self.entries.push(Entry {
            name: name.to_string(),
            body: EntryBody::Json(text),
            blobs,
        });
    }

    fn scene_with_blobs(
        &mut self,
        name: impl Into<String>,
        w: u32,
        h: u32,
        clear: Color,
        f: impl FnOnce(&mut LayerBuilder),
        blobs: Vec<Vec<u8>>,
    ) {
        self.scene_from(name, Scene::builder(w, h).clear(clear), f, blobs);
    }

    /// `w`x`h` scene via `f` that asks `render --present` to tone-map to
    /// the display `headroom` (`Scene::present_headroom`, #97).
    fn scene_headroom(
        &mut self,
        name: impl Into<String>,
        w: u32,
        h: u32,
        clear: Color,
        headroom: f64,
        f: impl FnOnce(&mut LayerBuilder),
    ) {
        let builder = Scene::builder(w, h).clear(clear).present_headroom(headroom);
        self.scene_from(name, builder, f, Vec::new());
    }

    /// Build a scene from a configured `builder` whose root layer is
    /// drawn by `f`, and queue it with its image `blobs`.
    fn scene_from(
        &mut self,
        name: impl Into<String>,
        mut builder: SceneBuilder,
        f: impl FnOnce(&mut LayerBuilder),
        blobs: Vec<Vec<u8>>,
    ) {
        {
            let mut root = builder.root();
            f(&mut root);
        }
        let scene = builder.build();
        self.entries.push(Entry {
            name: name.into(),
            body: EntryBody::Scene(Box::new(scene)),
            blobs,
        });
    }
}

/// The colours of one projective card variant (SDR, P3-only or HDR).
struct CardColors {
    face: Color,
    ink: Color,
    accent: Color,
    line: Color,
}

/// Text on a projective card: Latin, CJK and Arabic lines, shaped once.
struct CardText {
    runs: Vec<GlyphRun>,
    blobs: Vec<Vec<u8>>,
}

impl CardText {
    fn new(ctx: &mut TextContext, ink: &Paint) -> Self {
        let lines = [
            ("NotoSans.ttf", corpus::LATIN, 13.0, 0.0),
            ("NotoSansSC.ttf", corpus::CJK, 16.0, 36.0),
            ("NotoSansArabic.ttf", corpus::ARABIC, 16.0, 60.0),
        ];
        let mut runs = Vec::new();
        for (file, text, size, dy) in lines {
            for run in ctx.shape(file, text, size, FontWeight::NORMAL, ink) {
                runs.push(offset_run(&run, 0.0, dy));
            }
        }
        let blobs = font_blobs(ctx, &[&runs]);
        Self { runs, blobs }
    }
}

/// A 256×128 local card at the origin, clipped to a rounded rect: a face,
/// text in three scripts, one-pixel strokes, a rounded badge and an image.
fn projective_card(l: &mut LayerBuilder, text: &CardText, image: ResourceHash, c: &CardColors) {
    l.clip(Shape::rounded_rect(0.0, 0.0, 256.0, 128.0, 14.0));
    l.fill(Shape::rect(0.0, 0.0, 256.0, 128.0), solid(c.face));
    for run in &text.runs {
        l.glyphs(offset_run(run, 4.0, 2.0));
    }
    for i in 0..6 {
        let y = 0.5_f64.mul_add(1.0, f64::from(i).mul_add(5.0, 96.0));
        l.stroke(
            Shape::Line(Line::new((12.0, y), (150.0, y))),
            StrokeStyle {
                width: 1.0,
                ..StrokeStyle::default()
            },
            solid(c.line),
        );
    }
    l.fill(
        Shape::rounded_rect(168.0, 88.0, 72.0, 28.0, 10.0),
        solid(c.accent),
    );
    l.image(
        image,
        Rect::new(196.0, 14.0, 244.0, 62.0),
        Sampling::Bilinear,
    );
}

/// A perspective card placed with its local origin at `(x, y)` and tilted
/// about its centre.
fn card_projection(tilt: Vec2, distance: f64) -> Projection {
    Projection {
        matrix: Projection::perspective(distance),
        tilt,
        pivot: Vec2::new(128.0, 64.0),
        ..Projection::default()
    }
}

/// A `size`×`size` one-texel checkerboard of `a` and `b`.
fn fine_checker_png(size: u8, a: [u8; 4], b: [u8; 4]) -> Vec<u8> {
    let mut px = Vec::with_capacity(usize::from(size) * usize::from(size) * 4);
    for y in 0..size {
        for x in 0..size {
            px.extend_from_slice(if (x + y) % 2 == 0 { &a } else { &b });
        }
    }
    encode_png_rgba(u32::from(size), u32::from(size), &px)
}

/// The two-stop linear gradient from `a` at `start` to `b` at `end`.
fn linear2(start: Point, end: Point, a: Color, b: Color) -> Paint {
    Paint::Linear(LinearGradient {
        start,
        end,
        stops: vec![
            GradientStop {
                offset: 0.0,
                color: a,
            },
            GradientStop {
                offset: 1.0,
                color: b,
            },
        ],
        extend: Extend::Pad,
        interpolation: ColorSpace::LinearP3,
    })
}

/// Projective layers (#84): cards rotated in depth with text, thin paths,
/// rounded geometry and an image; flips; the horizon; nesting; blends;
/// backdrops; and minification.
#[expect(
    clippy::too_many_lines,
    reason = "one scene family per block, in the corpus's usual style"
)]
fn projective_scenes(corpus: &mut Corpus, ctx: &mut TextContext) {
    use std::f64::consts::{PI, TAU};
    let sdr = CardColors {
        face: srgb(0.97, 0.96, 0.92),
        ink: srgb(0.1, 0.1, 0.14),
        accent: srgb(0.85, 0.3, 0.2),
        line: srgb(0.2, 0.35, 0.8),
    };
    let wide = CardColors {
        face: p3(0.95, 0.97, 0.9),
        ink: p3(0.05, 0.3, 0.05),
        accent: p3(0.0, 0.85, 0.2),
        line: p3(0.9, 0.0, 0.3),
    };
    let bright = CardColors {
        face: srgb(0.9, 0.9, 0.95),
        ink: srgb(0.05, 0.05, 0.1),
        accent: hdr(4.0, 1.2, 0.3),
        line: hdr(0.5, 1.5, 6.0),
    };
    let checker = checker_png();
    let backdrop = srgb(0.86, 0.88, 0.92);
    let texts = [
        CardText::new(ctx, &solid(sdr.ink)),
        CardText::new(ctx, &solid(wide.ink)),
        CardText::new(ctx, &solid(bright.ink)),
    ];
    let with_image = |text: &CardText| {
        let mut blobs = text.blobs.clone();
        blobs.push(checker.clone());
        blobs
    };

    // Tilt about the vertical axis through the card centre, perspective
    // at 480 units.
    for degrees in [0_u32, 30, 60, 80, 89] {
        let tilt = Vec2::new(0.0, f64::from(degrees).to_radians());
        corpus.scene_with_blobs(
            format!("projective-card-{degrees:02}"),
            320,
            192,
            backdrop,
            |l| {
                l.layer(|card| {
                    card.transform(Affine::translate((32.0, 32.0)));
                    card.projection(card_projection(tilt, 480.0));
                    projective_card(card, &texts[0], ResourceHash::of(&checker), &sdr);
                });
            },
            with_image(&texts[0]),
        );
    }
    for (suffix, colors, text) in [("p3", &wide, &texts[1]), ("hdr", &bright, &texts[2])] {
        corpus.scene_with_blobs(
            format!("projective-card-60-{suffix}"),
            320,
            192,
            backdrop,
            |l| {
                l.layer(|card| {
                    card.transform(Affine::translate((32.0, 32.0)));
                    card.projection(card_projection(Vec2::new(0.35, 60_f64.to_radians()), 480.0));
                    projective_card(card, text, ResourceHash::of(&checker), colors);
                });
            },
            with_image(text),
        );
    }

    // A full turn about a non-central pivot on a spring, pushed back in
    // depth: the settled frame shows the front face again.
    corpus.scene_with_blobs(
        "projective-flip",
        320,
        192,
        backdrop,
        |l| {
            l.layer(|card| {
                card.transform(Affine::translate((32.0, 32.0)));
                card.projection(Projection {
                    matrix: Projection::perspective(420.0),
                    tilt: Vec2::new(0.0, TAU),
                    depth: -40.0,
                    pivot: Vec2::new(64.0, 64.0),
                });
                card.motion(Motion::Tilt {
                    from: Vec2::ZERO,
                    animation: MotionAnimation::Spring {
                        response: 0.4,
                        damping: 0.8,
                    },
                });
                projective_card(card, &texts[0], ResourceHash::of(&checker), &sdr);
            });
        },
        with_image(&texts[0]),
    );
    // A curve-timed tilt that crosses density buckets, settling showing the
    // back face (turned half way, both sides are drawn).
    corpus.scene_with_blobs(
        "projective-flip-back",
        320,
        192,
        backdrop,
        |l| {
            l.layer(|card| {
                card.transform(Affine::translate((32.0, 32.0)));
                card.projection(Projection {
                    matrix: Projection::perspective(300.0),
                    tilt: Vec2::new(0.2, PI - 0.3),
                    depth: 30.0,
                    pivot: Vec2::new(128.0, 64.0),
                });
                card.motion(Motion::Tilt {
                    from: Vec2::new(0.0, 0.0),
                    animation: MotionAnimation::Curve {
                        duration_ms: 250,
                        x1: 0.25,
                        y1: 0.1,
                        x2: 0.25,
                        y2: 1.0,
                    },
                });
                projective_card(card, &texts[0], ResourceHash::of(&checker), &sdr);
            });
        },
        with_image(&texts[0]),
    );

    // The horizon: a tall plane leaning back under a close camera. Its far
    // part recedes toward the horizon line and its near part crosses
    // W = 0 (`y > 328`) and passes behind the viewer, where it contributes
    // nothing. The rescaled variant multiplies the matrix by a positive
    // factor and must render identically.
    let floor = |l: &mut LayerBuilder| {
        let extent = Rect::new(0.0, -600.0, 320.0, 520.0);
        l.clip(Shape::Rect(extent));
        l.fill(
            Shape::Rect(extent),
            linear2(
                Point::new(0.0, -600.0),
                Point::new(0.0, 520.0),
                srgb(0.2, 0.3, 0.7),
                srgb(0.95, 0.8, 0.3),
            ),
        );
        for i in 0..11 {
            let x = f64::from(i) * 32.0;
            l.stroke(
                Shape::Line(Line::new((x, -600.0), (x, 520.0))),
                StrokeStyle {
                    width: 2.0,
                    ..StrokeStyle::default()
                },
                solid(srgb(0.1, 0.1, 0.1)),
            );
        }
        for i in 0..35 {
            let y = f64::from(i).mul_add(32.0, -600.0);
            l.fill(
                Shape::rect(0.0, y, 320.0, 3.0),
                solid(srgb(0.95, 0.95, 0.95)),
            );
        }
    };
    for (name, scale) in [
        ("projective-horizon", 1.0),
        ("projective-horizon-rescaled", 4.0),
    ] {
        corpus.scene(name, 320, 192, backdrop, |l| {
            l.layer(|plane| {
                plane.transform(Affine::translate((0.0, -8.0)));
                plane.projection(Projection {
                    matrix: Projection::perspective(160.0).map(|row| row.map(|v| v * scale)),
                    tilt: Vec2::new(1.25, 0.0),
                    pivot: Vec2::new(160.0, 160.0),
                    ..Projection::default()
                });
                floor(plane);
            });
        });
    }
    // Behind the viewer: every visible W is negative, nothing is drawn.
    corpus.scene("projective-horizon-behind", 128, 96, backdrop, |l| {
        l.layer(|plane| {
            plane.projection(Projection {
                matrix: Projection::perspective(100.0),
                depth: 150.0,
                ..Projection::default()
            });
            floor(plane);
        });
    });
    // An edge exactly on W = 0: W = 1 − (y − 32)/128 vanishes at y = 160,
    // the clip's bottom edge.
    corpus.scene("projective-horizon-edge", 256, 256, backdrop, |l| {
        l.layer(|plane| {
            let mut matrix = Projection::identity();
            matrix[3][1] = -1.0 / 128.0;
            plane.projection(Projection {
                matrix,
                pivot: Vec2::new(128.0, 32.0),
                ..Projection::default()
            });
            plane.clip(Shape::rect(96.0, 32.0, 64.0, 128.0));
            plane.fill(
                Shape::rect(96.0, 32.0, 64.0, 128.0),
                linear2(
                    Point::new(0.0, 32.0),
                    Point::new(0.0, 160.0),
                    srgb(0.9, 0.2, 0.2),
                    srgb(0.2, 0.2, 0.9),
                ),
            );
            for i in 0..8 {
                let y = f64::from(i).mul_add(16.0, 34.0);
                plane.fill(Shape::rect(96.0, y, 64.0, 4.0), solid(srgb(1.0, 1.0, 1.0)));
            }
        });
    });

    // Nesting flattens at each projective layer: the child's tilt is baked
    // into the parent's local image, its opacity composites there, and the
    // parent's blur runs over the flattened child before the parent tilts.
    corpus.scene_with_blobs(
        "projective-nested",
        320,
        224,
        backdrop,
        |l| {
            l.layer(|parent| {
                parent.transform(Affine::translate((32.0, 24.0)));
                parent.projection(Projection {
                    matrix: Projection::perspective(500.0),
                    tilt: Vec2::new(0.5, 0.0),
                    pivot: Vec2::new(128.0, 88.0),
                    ..Projection::default()
                });
                parent.clip(Shape::rounded_rect(0.0, 0.0, 256.0, 176.0, 12.0));
                parent.fill(
                    Shape::rect(0.0, 0.0, 256.0, 176.0),
                    solid(srgb(0.3, 0.32, 0.4)),
                );
                parent.layer(|child| {
                    child.transform(Affine::translate((24.0, 40.0)) * Affine::scale(0.75));
                    child.opacity(0.75);
                    child.projection(card_projection(Vec2::new(0.0, 0.9), 360.0));
                    projective_card(child, &texts[0], ResourceHash::of(&checker), &sdr);
                });
            });
        },
        with_image(&texts[0]),
    );
    corpus.scene_with_blobs(
        "projective-nested-filter",
        320,
        224,
        backdrop,
        |l| {
            l.layer(|parent| {
                parent.transform(Affine::translate((32.0, 24.0)));
                parent.filter(LayerFilter::GaussianBlur { sigma: 1.5 });
                parent.projection(Projection {
                    matrix: Projection::perspective(500.0),
                    tilt: Vec2::new(-0.4, 0.3),
                    pivot: Vec2::new(128.0, 88.0),
                    ..Projection::default()
                });
                parent.clip(Shape::rect(0.0, 0.0, 256.0, 176.0));
                parent.fill(
                    Shape::rect(0.0, 0.0, 256.0, 176.0),
                    solid(srgb(0.25, 0.5, 0.45)),
                );
                parent.layer(|child| {
                    child.transform(Affine::translate((24.0, 40.0)) * Affine::scale(0.75));
                    child.projection(card_projection(Vec2::new(0.6, 0.0), 300.0));
                    projective_card(child, &texts[0], ResourceHash::of(&checker), &sdr);
                });
            });
        },
        with_image(&texts[0]),
    );

    // Blends: translucent overlapping children flatten first, then the
    // layer multiplies onto the gradient behind it; a destructive `src`
    // blend clears its whole projected clip, including where its content
    // is transparent.
    let blend_ground = |l: &mut LayerBuilder, a: Color, b: Color| {
        l.fill(
            Shape::rect(0.0, 0.0, 256.0, 192.0),
            linear2(Point::new(0.0, 0.0), Point::new(256.0, 192.0), a, b),
        );
    };
    for (name, mode, colors) in [
        (
            "projective-blend-multiply",
            BlendMode::Multiply,
            [srgba(0.9, 0.2, 0.2, 0.6), srgba(0.2, 0.3, 0.9, 0.6)],
        ),
        (
            "projective-blend-multiply-p3",
            BlendMode::Multiply,
            [p3(0.0, 0.9, 0.1), p3(0.95, 0.0, 0.5)],
        ),
        (
            "projective-blend-src",
            BlendMode::Src,
            [srgba(0.9, 0.6, 0.1, 0.8), srgba(0.1, 0.6, 0.9, 0.5)],
        ),
    ] {
        corpus.scene(name, 256, 192, backdrop, |l| {
            blend_ground(l, srgb(0.95, 0.9, 0.3), srgb(0.3, 0.8, 0.9));
            l.layer(|layer| {
                layer.transform(Affine::translate((48.0, 32.0)));
                layer.blend(mode);
                layer.projection(Projection {
                    matrix: Projection::perspective(300.0),
                    tilt: Vec2::new(0.45, -0.6),
                    pivot: Vec2::new(80.0, 64.0),
                    ..Projection::default()
                });
                layer.clip(Shape::rounded_rect(0.0, 0.0, 160.0, 128.0, 16.0));
                layer.layer(|a| {
                    a.fill(Shape::circle(60.0, 60.0, 44.0), solid(colors[0]));
                });
                layer.layer(|b| {
                    b.fill(
                        Shape::rounded_rect(70.0, 40.0, 70.0, 60.0, 8.0),
                        solid(colors[1]),
                    );
                });
            });
        });
    }

    // Backdrops: a group captured inside a projected card samples the
    // card's local image; a group outside samples the surface, the
    // projected card included.
    corpus.scene_setup("projective-backdrop-inside", 256, 192, backdrop, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 3.0 }]);
        let l = &mut b.root();
        l.layer(|card| {
            card.transform(Affine::translate((24.0, 24.0)));
            card.projection(Projection {
                matrix: Projection::perspective(360.0),
                tilt: Vec2::new(0.0, 0.7),
                pivot: Vec2::new(104.0, 72.0),
                ..Projection::default()
            });
            card.clip(Shape::rect(0.0, 0.0, 208.0, 144.0));
            backdrop_background(card);
            card.layer(|m| {
                m.clip(Shape::rounded_rect(32.0, 32.0, 144.0, 80.0, 16.0));
                m.backdrop(1);
                m.fill(
                    Shape::rect(32.0, 32.0, 144.0, 80.0),
                    solid(srgba(1.0, 1.0, 1.0, 0.3)),
                );
            });
        });
    });
    corpus.scene_setup("projective-backdrop-outside-hdr", 256, 192, backdrop, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 4.0 }]);
        let l = &mut b.root();
        l.layer(|card| {
            card.transform(Affine::translate((24.0, 24.0)));
            card.projection(Projection {
                matrix: Projection::perspective(360.0),
                tilt: Vec2::new(0.6, 0.0),
                pivot: Vec2::new(104.0, 72.0),
                ..Projection::default()
            });
            card.clip(Shape::rect(0.0, 0.0, 208.0, 144.0));
            stripes(
                card,
                hdr(3.0, 0.8, 0.2),
                p3(0.0, 0.7, 0.9),
                [hdr(0.4, 2.5, 0.6), p3(0.9, 0.0, 0.4), srgb(0.9, 0.9, 0.2)],
            );
        });
        l.layer(|m| {
            m.clip(Shape::rounded_rect(64.0, 96.0, 128.0, 72.0, 20.0));
            m.backdrop(1);
            m.fill(
                Shape::rect(64.0, 96.0, 128.0, 72.0),
                solid(srgba(1.0, 1.0, 1.0, 0.2)),
            );
        });
    });

    // Minification at high anisotropy: a one-texel checkerboard, one-pixel
    // strokes and small text on a plane seen at a grazing angle.
    let fine = fine_checker_png(64, [250, 250, 250, 255], [20, 20, 30, 255]);
    let small = ctx.shape(
        "NotoSans.ttf",
        corpus::LATIN,
        9.0,
        FontWeight::NORMAL,
        &solid(sdr.ink),
    );
    let mut blobs = font_blobs(ctx, &[&small]);
    blobs.push(fine.clone());
    corpus.scene_with_blobs(
        "projective-minification",
        256,
        192,
        backdrop,
        |l| {
            l.layer(|plane| {
                plane.transform(Affine::translate((0.0, 40.0)));
                plane.projection(Projection {
                    matrix: Projection::perspective(260.0),
                    tilt: Vec2::new(1.35, 0.25),
                    pivot: Vec2::new(128.0, 64.0),
                    ..Projection::default()
                });
                plane.clip(Shape::rect(0.0, 0.0, 256.0, 128.0));
                plane.fill(
                    Shape::rect(0.0, 0.0, 256.0, 128.0),
                    solid(srgb(1.0, 1.0, 1.0)),
                );
                plane.image(
                    ResourceHash::of(&fine),
                    Rect::new(0.0, 0.0, 64.0, 64.0),
                    Sampling::Nearest,
                );
                for i in 0..24 {
                    let x = f64::from(i).mul_add(8.0, 68.5);
                    plane.stroke(
                        Shape::Line(Line::new((x, 0.0), (x, 128.0))),
                        StrokeStyle {
                            width: 1.0,
                            ..StrokeStyle::default()
                        },
                        solid(srgb(0.1, 0.2, 0.6)),
                    );
                }
                for run in &small {
                    plane.glyphs(offset_run(run, 0.0, 70.0));
                }
            });
        },
        blobs,
    );
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(%e, "corpus generation failed");
            ExitCode::FAILURE
        }
    }
}

/// The text-page body: 38 lines cycling through the shaped scripts.
fn text_page_body(l: &mut LayerBuilder, shaped: &[Vec<GlyphRun>]) {
    let pitch = 54.0;
    for (i, runs) in (0u8..38).map(|i| (i, &shaped[usize::from(i) % shaped.len()])) {
        // Exact either way: i < 38 and pitch = 54 are small integers.
        let y = f64::from(i).mul_add(pitch, 56.0);
        for run in runs {
            l.glyphs(offset_run(run, 24.0 - f64::from(TEXT_PAD), y));
        }
    }
}

/// The map-like page: ~2,000 stroked and filled paths — short segments,
/// closed polygons and curved outlines distributed over the viewport.
#[expect(
    clippy::suboptimal_flops,
    reason = "the suggested mul_add rewrites alter serialized float bytes; perf scenes are pinned"
)]
fn map_body(l: &mut LayerBuilder, pw: f64, ph: f64) {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let palette = [
        srgb(0.45, 0.60, 0.45),
        srgb(0.55, 0.50, 0.38),
        srgb(0.40, 0.55, 0.65),
        srgb(0.70, 0.55, 0.45),
        srgb(0.50, 0.45, 0.60),
    ];
    let thin = StrokeStyle {
        start_cap: kurbo::Cap::Round,
        end_cap: kurbo::Cap::Round,
        ..StrokeStyle::default()
    };
    for i in 0..2000 {
        let (x, y) = (rng.f64() * pw, rng.f64() * ph);
        let color = palette[rng.below(palette.len())];
        match i % 4 {
            // Polyline segment.
            0 => {
                let (dx, dy) = (rng.f64() * 90.0 - 45.0, rng.f64() * 90.0 - 45.0);
                l.stroke(
                    Shape::Line(Line::new((x, y), (x + dx, y + dy))),
                    StrokeStyle {
                        width: 0.5 + rng.f64() * 4.0,
                        ..thin.clone()
                    },
                    solid(color),
                );
            }
            // Closed polygon (park/parcel fill).
            1 => {
                let radius = 8.0 + rng.f64() * 48.0;
                let sides = 3 + rng.below(5);
                let mut path = BezPath::new();
                for vi in 0..sides {
                    #[expect(clippy::cast_precision_loss, reason = "vertex index is below 8")]
                    let angle = (vi as f64) * std::f64::consts::TAU / (sides as f64);
                    let pt = (x + radius * libm::cos(angle), y + radius * libm::sin(angle));
                    if vi == 0 {
                        path.move_to(pt);
                    } else {
                        path.line_to(pt);
                    }
                }
                path.close_path();
                l.fill(Shape::Path { path }, solid(color));
            }
            // Open polyline of 3-6 points (road/river line).
            2 => {
                let mut p = BezPath::new();
                let (mut cx, mut cy) = (x, y);
                p.move_to((cx, cy));
                for _ in 0..2 + rng.below(4) {
                    cx += rng.f64() * 120.0 - 60.0;
                    cy += rng.f64() * 60.0 - 30.0;
                    p.line_to((cx, cy));
                }
                l.stroke(
                    Shape::Path { path: p },
                    StrokeStyle {
                        width: 1.0 + rng.f64() * 5.0,
                        ..thin.clone()
                    },
                    solid(color),
                );
            }
            // Curved closed path (lake/contour).
            _ => {
                let (rx, ry) = (10.0 + rng.f64() * 60.0, 6.0 + rng.f64() * 40.0);
                l.fill(
                    Shape::Ellipse(Ellipse::new((x, y), (rx, ry), 0.0)),
                    solid(color),
                );
            }
        }
    }
}

/// Issue #210 reproduction: a dense city-map frame at 1600x1200 in the
/// Positron style — a Manhattan street grid of overlapping filled ribbons,
/// one merged fill path of ~235k building-footprint elements, and one
/// stroked path whose outline expands toward ~1M segments. Building lots
/// that rotate to a star footprint self-intersect. The merged single paths
/// are what drove `resolve_winding` quadratic; a tile-of-small-paths layout
/// would not.
///
/// Deterministic: fixed seed, no system input. The lots are generated once
/// into `manhattan_blocks`, so the fill and the stroked outline replay the
/// exact same footprints.
const MAP_SEED: u64 = 0x5EED_1057_A1A7_7A11;

/// One city block of building lots, or a park taking the whole block.
enum MapBlock {
    /// A park: one soft polygon, `pts` around the block centre.
    Park([(f64, f64); 9]),
    /// Packed building lots.
    Lots(Vec<MapLot>),
}

/// One building-lot footprint.
enum MapLot {
    /// A chamfered rectangle: 8 vertices.
    Chamfered([(f64, f64); 8]),
    /// A 10-point star outline, which self-intersects.
    Star { cx: f64, cy: f64, r: f64 },
}

/// The deterministic block layout: ~14 avenues x ~15 streets, each block
/// packed with 12x12 lots on a ~7x5 px cell. A rotating minority of blocks
/// (~2%) is a park; ~10% of lots is a self-intersecting star.
#[expect(
    clippy::suboptimal_flops,
    reason = "the suggested mul_add rewrites alter serialized float bytes; corpus scenes are pinned"
)]
#[expect(
    clippy::cast_precision_loss,
    reason = "loop counters are small integers; the layout is deterministic"
)]
fn manhattan_blocks() -> Vec<MapBlock> {
    let mut rng = Rng(MAP_SEED ^ 0xB11D);
    let mut blocks = Vec::new();
    for bx in 0..14u32 {
        for by in 0..15u32 {
            let x0 = f64::from(bx) * 100.0 + 68.0;
            let y0 = f64::from(by) * 76.0 + 47.0;
            if rng.below(48) == 0 {
                let cxm = x0 + 42.0;
                let cym = y0 + 31.0;
                let mut pts = [(0.0, 0.0); 9];
                for (vi, pt) in pts.iter_mut().enumerate() {
                    let angle = (vi as f64) * std::f64::consts::TAU / 9.0;
                    let r = 26.0 + rng.f64() * 10.0;
                    *pt = (cxm + r * angle.cos() * 1.4, cym + r * angle.sin());
                }
                blocks.push(MapBlock::Park(pts));
                continue;
            }
            let mut lots = Vec::with_capacity(144);
            for cx in 0..12u32 {
                for cy in 0..12u32 {
                    let fx = x0 + f64::from(cx) * 7.0;
                    let fy = y0 + f64::from(cy) * 5.15;
                    let w = 5.0 + rng.f64() * 1.6;
                    let h = 3.6 + rng.f64() * 1.4;
                    let c = 0.8 + rng.f64() * 0.7;
                    lots.push(if rng.below(10) == 0 {
                        MapLot::Star {
                            cx: fx + w * 0.5,
                            cy: fy + h * 0.5,
                            r: w * 0.7,
                        }
                    } else {
                        MapLot::Chamfered([
                            (fx + c, fy),
                            (fx + w - c, fy),
                            (fx + w, fy + c),
                            (fx + w, fy + h - c),
                            (fx + w - c, fy + h),
                            (fx + c, fy + h),
                            (fx, fy + h - c),
                            (fx, fy + c),
                        ])
                    });
                }
            }
            blocks.push(MapBlock::Lots(lots));
        }
    }
    blocks
}

/// Append one lot's subpath (`move_to`, `line_to` x n, `close_path`).
fn lot_subpath(path: &mut BezPath, lot: &MapLot) {
    match *lot {
        MapLot::Chamfered(pts) => {
            path.move_to(pts[0]);
            for pt in &pts[1..] {
                path.line_to(*pt);
            }
        }
        MapLot::Star { cx, cy, r } => {
            for vi in 0..10u32 {
                let rr = if vi % 2 == 0 { r } else { r * 0.4 };
                let angle = f64::from(vi)
                    .mul_add(std::f64::consts::TAU / 10.0, -std::f64::consts::FRAC_PI_2);
                let pt = (rr.mul_add(angle.cos(), cx), rr.mul_add(angle.sin(), cy));
                if vi == 0 {
                    path.move_to(pt);
                } else {
                    path.line_to(pt);
                }
            }
        }
    }
    path.close_path();
}

/// The scene body: overlapping street fills, the merged building fill,
/// then the merged stroke of outlines and road centre-lines.
#[expect(
    clippy::suboptimal_flops,
    reason = "the suggested mul_add rewrites alter serialized float bytes; corpus scenes are pinned"
)]
fn map_manhattan_body(l: &mut LayerBuilder) {
    let land = srgb(0.92, 0.93, 0.89);
    let street = srgb(0.97, 0.97, 0.96);
    let water = srgb(0.78, 0.86, 0.90);
    let building = srgb(0.71, 0.72, 0.70);
    let park = srgb(0.72, 0.80, 0.66);
    let road_paint = srgb(0.55, 0.57, 0.58);

    // Land mass and the East River along the right edge.
    l.fill(Shape::rect(0.0, 0.0, 1600.0, 1200.0), solid(land));
    l.fill(Shape::rect(1500.0, 0.0, 100.0, 1200.0), solid(water));

    // The street grid as overlapping filled ribbons.
    for i in 0..15u32 {
        let x = 60.0 + f64::from(i) * 100.0;
        l.fill(Shape::rect(x - 6.0, 0.0, 12.0, 1200.0), solid(street));
    }
    for i in 0..16u32 {
        let y = 40.0 + f64::from(i) * 76.0;
        l.fill(Shape::rect(0.0, y - 5.0, 1500.0, 10.0), solid(street));
    }

    let blocks = manhattan_blocks();

    // The merged building fill: ~30k lots in one path.
    let mut buildings = BezPath::new();
    for block in &blocks {
        if let MapBlock::Lots(lots) = block {
            for lot in lots {
                lot_subpath(&mut buildings, lot);
            }
        }
    }
    l.fill(Shape::Path { path: buildings }, solid(building));

    // Park blocks paint over the lots they replaced.
    for block in &blocks {
        if let MapBlock::Park(pts) = block {
            let mut p = BezPath::new();
            p.move_to(pts[0]);
            for pt in &pts[1..] {
                p.line_to(*pt);
            }
            p.close_path();
            l.fill(Shape::Path { path: p }, solid(park));
        }
    }

    // The stroked layer: every building outline plus jittered road
    // centre-lines, again as one path.
    let mut stroke = BezPath::new();
    let mut rng = Rng(MAP_SEED ^ 0x80AD);
    for block in &blocks {
        match block {
            MapBlock::Park(pts) => {
                // A drive ring around the park's centre.
                let cxm = pts.iter().map(|p| p.0).sum::<f64>() / 9.0;
                let cym = pts.iter().map(|p| p.1).sum::<f64>() / 9.0;
                for vi in 0..40u32 {
                    let angle = f64::from(vi) * std::f64::consts::TAU / 40.0;
                    let pt = (cxm + 30.0 * angle.cos(), cym + 24.0 * angle.sin());
                    if vi == 0 {
                        stroke.move_to(pt);
                    } else {
                        stroke.line_to(pt);
                    }
                }
                stroke.close_path();
            }
            MapBlock::Lots(lots) => {
                for lot in lots {
                    lot_subpath(&mut stroke, lot);
                }
            }
        }
    }
    for i in 0..15u32 {
        let x = 60.0 + f64::from(i) * 100.0;
        stroke.move_to((x, 0.0));
        for k in 0..600u32 {
            let y = (f64::from(k) + 1.0) * 2.0;
            stroke.line_to((x + rng.f64() * 2.0 - 1.0, y));
        }
    }
    for i in 0..16u32 {
        let y = 40.0 + f64::from(i) * 76.0;
        stroke.move_to((0.0, y));
        for k in 0..750u32 {
            let x = (f64::from(k) + 1.0) * 2.0;
            stroke.line_to((x, y + rng.f64() * 2.0 - 1.0));
        }
    }
    l.stroke(
        Shape::Path { path: stroke },
        StrokeStyle {
            width: 0.8,
            ..StrokeStyle::default()
        },
        solid(road_paint),
    );
}

/// The dense city-map frame of #211: a 1600×1200 Positron-style view —
/// water along the west edge, parkland, a street grid crossed by
/// diagonal avenues, building footprints and street labels — whose live
/// coverage exceeds one atlas page.
///
/// Deterministic: the `Rng` stream is seeded, so the scene is identical
/// on every run. Element and segment counts stay bounded because the
/// oracle is O(segments × pixels); the wide diagonal avenues carry the
/// coverage past the 4096² cap with few segments.
#[expect(
    clippy::suboptimal_flops,
    reason = "the suggested mul_add rewrites alter serialized float bytes; corpus scenes are pinned"
)]
#[expect(clippy::too_many_lines, reason = "the scene is a flat element list")]
fn dense_map_body(l: &mut LayerBuilder, labels: &[Vec<GlyphRun>]) {
    const W: f64 = 1600.0;
    const H: f64 = 1200.0;
    let mut rng = Rng(0x9D15_5EED_5EED_5EED);

    let water = srgb(0.76, 0.86, 0.91);
    let park_fill = srgb(0.76, 0.87, 0.70);
    let facade = srgb(0.83, 0.81, 0.77);
    let facade_line = srgb(0.66, 0.64, 0.60);
    let road = srgb(0.99, 0.99, 0.98);
    let alley = srgb(0.88, 0.88, 0.85);

    let round = StrokeStyle {
        start_cap: kurbo::Cap::Round,
        end_cap: kurbo::Cap::Round,
        ..StrokeStyle::default()
    };

    // Water: the west river, a wavy filled strip.
    let mut river = BezPath::new();
    river.move_to((0.0, 0.0));
    for i in 0..=12u32 {
        let y = f64::from(i) * (H / 12.0);
        river.line_to((160.0 + rng.f64() * 80.0, y));
    }
    river.line_to((0.0, H));
    river.close_path();
    l.fill(Shape::Path { path: river }, solid(water));

    // Parkland: an irregular green polygon mid-north.
    let mut park = BezPath::new();
    let park_corners = [
        (620.0, 70.0),
        (1140.0, 90.0),
        (1160.0, 260.0),
        (1100.0, 430.0),
        (640.0, 410.0),
        (600.0, 240.0),
    ];
    for (i, (px, py)) in park_corners.iter().enumerate() {
        let (jx, jy) = (rng.f64() * 16.0 - 8.0, rng.f64() * 16.0 - 8.0);
        if i == 0 {
            park.move_to((px + jx, py + jy));
        } else {
            park.line_to((px + jx, py + jy));
        }
    }
    park.close_path();
    l.fill(Shape::Path { path: park }, solid(park_fill));

    // Streets: horizontal lines with slight jitter, light carriageways.
    for s in 0..15u32 {
        let y = 130.0 + f64::from(s) * 70.0 + rng.f64() * 10.0;
        let mut street = BezPath::new();
        street.move_to((190.0, y));
        for k in 1..4u32 {
            street.line_to((190.0 + f64::from(k) * 460.0, y + rng.f64() * 12.0 - 6.0));
        }
        l.stroke(
            Shape::Path { path: street },
            StrokeStyle {
                width: 2.5 + rng.f64() * 3.0,
                ..round.clone()
            },
            solid(road),
        );
    }

    // Avenues: verticals, wider than the streets.
    for a in 0..12u32 {
        let x = 230.0 + f64::from(a) * 112.0 + rng.f64() * 12.0;
        let mut ave = BezPath::new();
        ave.move_to((x, 20.0));
        for k in 1..4u32 {
            ave.line_to((x + rng.f64() * 16.0 - 8.0, 20.0 + f64::from(k) * 390.0));
        }
        l.stroke(
            Shape::Path { path: ave },
            StrokeStyle {
                width: 5.0 + rng.f64() * 5.0,
                ..round.clone()
            },
            solid(road),
        );
    }

    // Diagonal avenues, in both directions.
    for d in 0..8u32 {
        let x0 = 200.0 + f64::from(d) * 160.0 + rng.f64() * 30.0;
        let (w_run, rise) = (650.0 + rng.f64() * 350.0, 1050.0 + rng.f64() * 120.0);
        let mut ave = BezPath::new();
        ave.move_to((x0, H));
        ave.line_to((x0 + w_run * 0.55, H - rise * 0.45 + rng.f64() * 40.0 - 20.0));
        ave.line_to((x0 + w_run, H - rise));
        l.stroke(
            Shape::Path { path: ave },
            StrokeStyle {
                width: 8.0 + rng.f64() * 6.0,
                ..round.clone()
            },
            solid(road),
        );
    }
    for d in 0..4u32 {
        let x0 = 260.0 + f64::from(d) * 280.0 + rng.f64() * 40.0;
        let (w_run, rise) = (550.0 + rng.f64() * 300.0, 980.0 + rng.f64() * 140.0);
        let mut ave = BezPath::new();
        ave.move_to((x0 + w_run, H));
        ave.line_to((x0 + w_run * 0.45, H - rise * 0.5 + rng.f64() * 40.0 - 20.0));
        ave.line_to((x0, H - rise));
        l.stroke(
            Shape::Path { path: ave },
            StrokeStyle {
                width: 6.0 + rng.f64() * 5.0,
                ..round.clone()
            },
            solid(road),
        );
    }

    // Boulevards: long wide strokes sweeping across the frame at shallow
    // angles, like the arterials a dense map style layers under the
    // street grid. Their coverage is what pushes the frame past one
    // atlas page: each strip cell spans hundreds of columns.
    for _ in 0..32u32 {
        let x0 = rng.f64() * W * 0.4;
        let y0 = 80.0 + rng.f64() * (H - 160.0);
        let run_x = 1200.0 + rng.f64() * 420.0;
        let rise = run_x * (0.24 + rng.f64() * 0.24) * if rng.below(2) == 0 { 1.0 } else { -1.0 };
        let mut exp = BezPath::new();
        exp.move_to((x0 - rng.f64() * 300.0, y0));
        exp.line_to((x0 + run_x * 0.5, y0 + rise * 0.5 + rng.f64() * 40.0 - 20.0));
        exp.line_to((x0 + run_x, y0 + rise));
        l.stroke(
            Shape::Path { path: exp },
            StrokeStyle {
                width: 14.0 + rng.f64() * 10.0,
                ..round.clone()
            },
            solid(road),
        );
    }

    // Side streets: short connecting strokes inside the grid.
    for _ in 0..120u32 {
        let x = 250.0 + rng.f64() * 1260.0;
        let y = 140.0 + rng.f64() * 1000.0;
        let mut a = BezPath::new();
        a.move_to((x, y));
        a.line_to((x + rng.f64() * 400.0 - 200.0, y + rng.f64() * 200.0 - 100.0));
        l.stroke(
            Shape::Path { path: a },
            StrokeStyle {
                width: 2.5 + rng.f64() * 4.0,
                ..round.clone()
            },
            solid(alley),
        );
    }

    // Buildings: filled polygon footprints with thin stroked outlines,
    // packed into the blocks the streets and avenues leave.
    for s in 0..14u32 {
        let by = 138.0 + f64::from(s) * 72.0;
        for a in 0..10u32 {
            let bx = 238.0 + f64::from(a) * 130.0;
            for _ in 0..=rng.below(2) {
                let bw = 8.0 + rng.f64() * 30.0;
                let bh = 6.0 + rng.f64() * 20.0;
                let bx0 = bx + rng.f64() * 40.0;
                let by0 = by + rng.f64() * 18.0;
                if bx0 + bw > 1520.0 || by0 + bh > 1140.0 {
                    continue;
                }
                let sides = 4 + rng.below(3);
                let mut b = BezPath::new();
                for v in 0..sides {
                    let px =
                        bx0 + if v == 1 || v == 2 { bw } else { 0.0 } + (rng.f64() * 4.0 - 2.0);
                    let py = by0 + if v < 2 { 0.0 } else { bh } + (rng.f64() * 4.0 - 2.0);
                    if v == 0 {
                        b.move_to((px, py));
                    } else {
                        b.line_to((px, py));
                    }
                }
                b.close_path();
                let shape = Shape::Path { path: b };
                l.fill(shape.clone(), solid(facade));
                l.stroke(
                    shape,
                    StrokeStyle {
                        width: 0.8,
                        ..round.clone()
                    },
                    solid(facade_line),
                );
            }
        }
    }

    // Street and place labels, the text a map frame carries.
    for (i, runs) in labels.iter().enumerate() {
        let lx = (f64::from(u32::try_from(i).expect("labels fit u32")) * 137.0) % 1180.0
            + 240.0
            + rng.f64() * 60.0;
        let ly = (f64::from(u32::try_from(i).expect("labels fit u32")) * 211.0) % 960.0
            + 140.0
            + rng.f64() * 40.0;
        for run in runs {
            l.glyphs(offset_run(run, lx, ly));
        }
    }
}

/// Shared background: saturated shapes and a diagonal gradient so the
/// sampled backdrop is visibly different from a flat fill.
fn backdrop_background(l: &mut LayerBuilder) {
    l.fill(
        Shape::rect(0.0, 0.0, 256.0, 256.0),
        Paint::Linear(LinearGradient {
            start: Point::new(0.0, 0.0),
            end: Point::new(256.0, 256.0),
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: srgb(0.15, 0.20, 0.55),
                },
                GradientStop {
                    offset: 1.0,
                    color: srgb(0.85, 0.35, 0.15),
                },
            ],
            extend: Extend::Pad,
            interpolation: ColorSpace::Srgb,
        }),
    );
    l.fill(
        Shape::circle(64.0, 72.0, 52.0),
        solid(srgb(0.85, 0.15, 0.20)),
    );
    l.fill(
        Shape::circle(196.0, 60.0, 40.0),
        solid(srgb(0.10, 0.60, 0.85)),
    );
    l.fill(
        Shape::rect(40.0, 150.0, 176.0, 82.0),
        solid(srgb(0.90, 0.65, 0.10)),
    );
    l.fill(
        Shape::Ellipse(Ellipse::new((160.0, 150.0), (70.0, 46.0), 0.0)),
        solid(srgb(0.40, 0.18, 0.75)),
    );
}

// Refraction backdrop: hard 16 px stripes so the edge displacement is
// obvious, plus three discs for organic geometry.
fn stripes(l: &mut LayerBuilder, a: Color, b: Color, discs: [Color; 3]) {
    for i in 0..8 {
        let x0 = 32.0 * f64::from(i);
        l.fill(Shape::rect(x0, 0.0, x0 + 16.0, 256.0), solid(a));
        l.fill(Shape::rect(x0 + 16.0, 0.0, x0 + 32.0, 256.0), solid(b));
    }
    l.fill(Shape::circle(64.0, 200.0, 36.0), solid(discs[0]));
    l.fill(Shape::circle(192.0, 56.0, 28.0), solid(discs[1]));
    l.fill(Shape::circle(224.0, 224.0, 24.0), solid(discs[2]));
}

// The three members: rounded-rect clips (radius 16) of sizes 48, 96 and
// 160 px, each centred on a stripe boundary so refraction has two
// colours to pull.
fn refraction_member(
    l: &mut LayerBuilder,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    depth: f64,
    strength: f64,
) {
    l.layer(|m| {
        m.clip(Shape::RoundedRect(RoundedRect::new(x0, y0, x1, y1, 16.0)));
        m.backdrop(1);
        m.backdrop_effect(BackdropEffectSpec::Refraction { depth, strength });
        m.fill(
            Shape::rect(x0 + 2.0, y0 + 2.0, x1 - 2.0, y1 - 2.0),
            solid(srgba(1.0, 1.0, 1.0, 0.12)),
        );
    });
}

// A phone-shaped scene: busy backdrop, one blurred group, a top bar
// and a bottom bar far enough apart to take two capture regions.
fn bars_background(l: &mut LayerBuilder, tile: impl Fn(u32) -> Color) {
    l.fill(
        Shape::rect(0.0, 0.0, 1024.0, 2216.0),
        Paint::Linear(LinearGradient {
            start: Point::new(0.0, 0.0),
            end: Point::new(1024.0, 2216.0),
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: srgb(0.08, 0.12, 0.35),
                },
                GradientStop {
                    offset: 1.0,
                    color: srgb(0.60, 0.20, 0.30),
                },
            ],
            extend: Extend::Pad,
            interpolation: ColorSpace::Srgb,
        }),
    );
    for i in 0..40u32 {
        let (tx, ty) = (i % 5, i / 5);
        let x0 = f64::from(tx) * 196.0;
        let y0 = f64::from(ty) * 258.0;
        let (x0, y0) = (x0 + 32.0, y0 + 160.0);
        if i % 3 == 0 {
            l.fill(Shape::circle(x0 + 64.0, y0 + 64.0, 60.0), solid(tile(i)));
        } else {
            l.fill(
                Shape::RoundedRect(RoundedRect::new(x0, y0, x0 + 128.0, y0 + 128.0, 20.0)),
                solid(tile(i)),
            );
        }
    }
}

fn bars(l: &mut LayerBuilder) {
    l.layer(|m| {
        m.clip(Shape::RoundedRect(RoundedRect::new(
            0.0, 16.0, 1024.0, 112.0, 24.0,
        )));
        m.backdrop(1);
    });
    l.layer(|m| {
        m.clip(Shape::RoundedRect(RoundedRect::new(
            0.0, 2088.0, 1024.0, 2216.0, 24.0,
        )));
        m.backdrop(1);
    });
}

#[expect(
    clippy::too_many_lines,
    reason = "a linear sequence of independent scene builders; it reads top to bottom"
)]
#[expect(
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    reason = "the suggested mul_add/hypot rewrites alter serialized float bytes;               corpus and perf scenes are pinned byte-identical"
)]
fn run() -> Result<(), SceneError> {
    let root = corpus::repo_root();
    let out = corpus::corpus_dir(&root);
    let mut ctx = TextContext::new(&corpus::fonts_dir(&root))?;
    let mut corpus = Corpus::new();

    let white = srgb(0.95, 0.95, 0.95);
    let dark = srgb(0.1, 0.1, 0.12);

    // ---- Basic shapes ------------------------------------------------------

    corpus.scene("rect-plain", 96, 96, white, |l| {
        l.fill(
            Shape::rect(8.0, 8.0, 48.0, 48.0),
            solid(srgb(0.8, 0.1, 0.1)),
        );
        l.fill(
            Shape::rect(40.0, 40.0, 48.0, 48.0),
            solid(srgba(0.1, 0.4, 0.9, 0.7)),
        );
    });

    for (name, r) in [
        ("rounded-r00", 0.0),
        ("rounded-r08", 8.0),
        ("rounded-r24", 24.0),
        ("rounded-r32", 32.0),
    ] {
        corpus.scene(name, 96, 96, white, |l| {
            l.fill(
                Shape::rounded_rect(16.0, 16.0, 64.0, 64.0, r),
                solid(srgb(0.2, 0.6, 0.3)),
            );
        });
    }

    corpus.scene("rounded-mixed", 96, 96, white, |l| {
        let rr = RoundedRect::from_rect(
            Rect::new(12.0, 12.0, 84.0, 84.0),
            RoundedRectRadii::new(4.0, 16.0, 28.0, 40.0),
        );
        l.fill(Shape::RoundedRect(rr), solid(srgb(0.5, 0.2, 0.7)));
    });

    for (name, smoothing) in [
        ("continuous-s00", 0.0),
        ("continuous-s50", 0.5),
        ("continuous-s100", 1.0),
    ] {
        corpus.scene(name, 96, 96, white, |l| {
            l.fill(
                Shape::Continuous(cherenkov_scene::ContinuousRect::new(
                    Rect::new(16.0, 16.0, 80.0, 80.0),
                    28.0,
                    smoothing,
                )),
                solid(srgb(0.9, 0.4, 0.1)),
            );
        });
    }

    // Sharp (r = 0) corners on fractional pixel boundaries: the corner's
    // pixel gets the exact intersection area of the two folded
    // half-planes. Four axis-aligned rects put their corners at .25/.5/.75
    // offsets; the last rect is rotated 30° so the same corner has an
    // oblique gradient.
    for (name, c0, c1, c2, c3, c4) in [
        (
            "fill-sharp-subpixel",
            srgb(0.85, 0.25, 0.35),
            srgb(0.2, 0.45, 0.85),
            srgb(0.25, 0.6, 0.35),
            srgb(0.75, 0.45, 0.15),
            srgb(0.5, 0.25, 0.65),
        ),
        (
            "fill-sharp-subpixel-p3",
            p3(1.0, 0.0, 0.6),
            p3(0.0, 0.4, 1.0),
            p3(0.0, 0.9, 0.3),
            p3(1.0, 0.5, 0.0),
            p3(0.6, 0.0, 1.0),
        ),
        (
            "fill-sharp-subpixel-hdr",
            hdr(8.0, 0.0, 4.0),
            hdr(0.0, 8.0, 16.0),
            hdr(0.0, 10.0, 2.0),
            hdr(12.0, 6.0, 0.0),
            hdr(10.0, 0.0, 16.0),
        ),
    ] {
        corpus.scene(name, 128, 128, white, |l| {
            l.fill(Shape::rect(12.25, 12.5, 44.25, 44.5), solid(c0));
            l.fill(Shape::rect(68.75, 16.25, 100.75, 48.25), solid(c1));
            l.fill(Shape::rect(16.5, 76.75, 48.5, 108.75), solid(c2));
            l.fill(Shape::rect(80.25, 68.5, 112.25, 100.5), solid(c3));
            l.layer(|a| {
                a.transform(
                    Affine::translate((64.0, 64.0))
                        * rotate(std::f64::consts::FRAC_PI_6)
                        * Affine::translate((-64.0, -64.0)),
                );
                a.fill(Shape::rect(48.25, 48.5, 80.25, 80.5), solid(c4));
            });
        });
    }

    corpus.scene("circle", 96, 96, white, |l| {
        l.fill(Shape::circle(48.0, 48.0, 36.0), solid(srgb(0.1, 0.3, 0.8)));
        l.fill(
            Shape::circle(72.0, 30.0, 12.0),
            solid(srgba(1.0, 0.6, 0.0, 0.8)),
        );
    });

    corpus.scene("ellipse", 96, 96, white, |l| {
        l.fill(
            Shape::Ellipse(Ellipse::new((48.0, 48.0), (40.0, 24.0), 0.0)),
            solid(srgb(0.2, 0.5, 0.5)),
        );
    });

    corpus.scene("path-curved", 128, 128, white, |l| {
        l.fill(
            Shape::Path {
                path: curved_path(),
            },
            solid(srgb(0.3, 0.2, 0.7)),
        );
    });

    corpus.scene("path-curved-transform", 128, 128, white, |l| {
        l.layer(|a| {
            a.transform(
                Affine::translate((64.3, 63.8))
                    * rotate(0.3)
                    * Affine::scale(0.85)
                    * Affine::translate((-64.0, -64.0)),
            );
            a.fill(
                Shape::Path {
                    path: curved_path(),
                },
                solid(srgb(0.3, 0.2, 0.7)),
            );
        });
    });

    corpus.scene("path-self-intersect", 128, 128, white, |l| {
        l.fill(
            Shape::Path {
                path: self_intersecting_path(),
            },
            solid(srgb(0.8, 0.2, 0.4)),
        );
    });

    // Self-overlapping fills: overlapping windings resolve to the union
    // boundary, a bow-tie keeps both lobes, and even-odd punches a hole.
    for (name, c0, c1, c2) in [
        (
            "path-overlap-winding",
            srgb(0.85, 0.25, 0.35),
            srgb(0.2, 0.45, 0.85),
            srgb(0.25, 0.6, 0.35),
        ),
        (
            "path-overlap-winding-p3",
            p3(1.0, 0.0, 0.6),
            p3(0.0, 0.4, 1.0),
            p3(0.0, 0.9, 0.3),
        ),
        (
            "path-overlap-winding-hdr",
            hdr(8.0, 0.0, 4.0),
            hdr(0.0, 8.0, 16.0),
            hdr(0.0, 10.0, 2.0),
        ),
    ] {
        corpus.scene(name, 128, 128, white, |l| {
            let (circles, bow_tie, squares) = overlap_winding_path();
            l.fill(Shape::Path { path: circles }, solid(c0));
            l.fill(Shape::Path { path: bow_tie }, solid(c1));
            l.fill_rule(Shape::Path { path: squares }, FillRule::EvenOdd, solid(c2));
        });
    }

    for (name, rule) in [
        ("path-nonzero", FillRule::NonZero),
        ("path-evenodd", FillRule::EvenOdd),
    ] {
        corpus.scene(name, 128, 128, white, |l| {
            let mut p = star_path(64.0, 64.0, 24.0, 56.0);
            p.extend(star_path(64.0, 64.0, 10.0, 24.0));
            l.fill_rule(Shape::Path { path: p }, rule, solid(srgb(0.1, 0.5, 0.6)));
        });
    }

    // ---- Strokes -----------------------------------------------------------

    corpus.scene("stroke-widths", 128, 128, white, |l| {
        for (w, i) in [1.0, 3.0, 8.0].iter().zip(0u16..) {
            let y = f64::from(i).mul_add(40.0, 24.0);
            l.stroke(
                Shape::Line(Line::new((12.0, y), (116.0, y))),
                StrokeStyle {
                    width: *w,
                    ..StrokeStyle::default()
                },
                solid(srgb(0.2, 0.2, 0.2)),
            );
        }
    });

    corpus.scene("stroke-joins", 128, 128, white, |l| {
        for (join, i) in [
            cherenkov_scene::kurbo::Join::Miter,
            cherenkov_scene::kurbo::Join::Round,
            cherenkov_scene::kurbo::Join::Bevel,
        ]
        .iter()
        .zip(0u16..)
        {
            let y = f64::from(i).mul_add(40.0, 20.0);
            let mut p = BezPath::new();
            p.move_to((16.0, y + 20.0));
            p.line_to((44.0, y));
            p.line_to((72.0, y + 20.0));
            p.line_to((100.0, y));
            l.stroke(
                Shape::Path { path: p },
                StrokeStyle {
                    width: 6.0,
                    join: *join,
                    ..StrokeStyle::default()
                },
                solid(srgb(0.6, 0.1, 0.1)),
            );
        }
    });

    corpus.scene("stroke-caps", 128, 128, white, |l| {
        for (cap, i) in [
            cherenkov_scene::kurbo::Cap::Butt,
            cherenkov_scene::kurbo::Cap::Round,
            cherenkov_scene::kurbo::Cap::Square,
        ]
        .iter()
        .zip(0u16..)
        {
            let y = f64::from(i).mul_add(36.0, 28.0);
            l.stroke(
                Shape::Line(Line::new((16.0, y), (112.0, y))),
                StrokeStyle {
                    width: 10.0,
                    start_cap: *cap,
                    end_cap: *cap,
                    ..StrokeStyle::default()
                },
                solid(srgb(0.1, 0.3, 0.6)),
            );
        }
    });

    corpus.scene("stroke-dash", 128, 128, white, |l| {
        l.stroke(
            Shape::Path {
                path: curved_path(),
            },
            StrokeStyle {
                width: 3.0,
                dash_pattern: vec![8.0, 4.0, 2.0, 4.0],
                dash_offset: 2.0,
                ..StrokeStyle::default()
            },
            solid(srgb(0.4, 0.1, 0.5)),
        );
    });
    corpus.scene("stroke-continuous-s50", 96, 96, white, |l| {
        l.stroke(
            Shape::Continuous(cherenkov_scene::ContinuousRect::new(
                Rect::new(16.0, 16.0, 80.0, 80.0),
                28.0,
                0.5,
            )),
            StrokeStyle {
                width: 6.0,
                ..StrokeStyle::default()
            },
            solid(srgb(0.2, 0.4, 0.9)),
        );
    });

    corpus.scene("stroke-curve", 128, 128, white, |l| {
        l.stroke(
            Shape::rounded_rect(16.0, 16.0, 96.0, 96.0, 24.0),
            StrokeStyle {
                width: 5.0,
                ..StrokeStyle::default()
            },
            solid(srgb(0.1, 0.5, 0.3)),
        );
    });

    // Every analytic shape family x every join class, each at all three
    // caps, solid and dashed. The dashed row's period (18) divides the box
    // edge (36), so every corner sits centred in a dash.
    let families: &[(&str, ShapeBuild)] = &[
        ("rect", |r| Shape::Rect(r)),
        ("rounded", |r| {
            Shape::RoundedRect(RoundedRect::from_rect(
                r,
                RoundedRectRadii::new(0.0, 2.0, 8.0, 14.0),
            ))
        }),
        ("continuous", |r| {
            Shape::Continuous(cherenkov_scene::ContinuousRect::new(r, 10.0, 0.6))
        }),
        ("continuous-sharp", |r| {
            Shape::Continuous(cherenkov_scene::ContinuousRect::new(r, 0.0, 0.6))
        }),
        ("continuous-s00", |r| {
            Shape::Continuous(cherenkov_scene::ContinuousRect::new(r, 0.0, 0.0))
        }),
        ("ellipse", |r| {
            Shape::Ellipse(Ellipse::new(r.center(), (18.0, 12.0), 0.0))
        }),
    ];
    let joins: &[(&str, kurbo::Join, f64)] = &[
        ("miter-lo", kurbo::Join::Miter, 1.0),
        ("miter-rt2", kurbo::Join::Miter, std::f64::consts::SQRT_2),
        ("miter-hi", kurbo::Join::Miter, 4.0),
        ("round", kurbo::Join::Round, 4.0),
        ("bevel", kurbo::Join::Bevel, 4.0),
    ];
    for &(family, build) in families {
        for &(suffix, join, miter_limit) in joins {
            corpus.scene(
                format!("stroke-join-{family}-{suffix}"),
                156,
                108,
                white,
                |l| {
                    for (cap, col) in [kurbo::Cap::Butt, kurbo::Cap::Round, kurbo::Cap::Square]
                        .iter()
                        .zip(0u16..)
                    {
                        let x0 = 12.0 + 48.0 * f64::from(col);
                        for row in 0..2 {
                            let y0 = 12.0 + 48.0 * f64::from(row);
                            let dashed = row == 1;
                            l.stroke(
                                build(Rect::new(x0, y0, x0 + 36.0, y0 + 36.0)),
                                StrokeStyle {
                                    width: 5.0,
                                    join,
                                    miter_limit,
                                    start_cap: *cap,
                                    end_cap: *cap,
                                    dash_pattern: if dashed { vec![12.0, 6.0] } else { Vec::new() },
                                    dash_offset: if dashed { 6.0 } else { 0.0 },
                                },
                                solid(if dashed {
                                    srgb(0.6, 0.1, 0.1)
                                } else {
                                    srgb(0.1, 0.3, 0.6)
                                }),
                            );
                        }
                    }
                },
            );
        }
    }

    corpus.scene("stroke-caps-mixed", 128, 128, white, |l| {
        for ((start_cap, end_cap), i) in [
            (kurbo::Cap::Butt, kurbo::Cap::Round),
            (kurbo::Cap::Round, kurbo::Cap::Square),
            (kurbo::Cap::Square, kurbo::Cap::Butt),
            (kurbo::Cap::Butt, kurbo::Cap::Square),
        ]
        .iter()
        .zip(0u16..)
        {
            let y = f64::from(i).mul_add(32.0, 24.0);
            let dashed = i == 3;
            l.stroke(
                Shape::Line(Line::new((16.0, y), (112.0, y))),
                StrokeStyle {
                    width: 10.0,
                    start_cap: *start_cap,
                    end_cap: *end_cap,
                    dash_pattern: if dashed { vec![16.0, 8.0] } else { Vec::new() },
                    ..StrokeStyle::default()
                },
                solid(srgb(0.1, 0.3, 0.6)),
            );
        }
    });

    // ---- Gradients ---------------------------------------------------------

    let gradient_rect = Shape::rect(8.0, 8.0, 112.0, 112.0);
    for extends in [Extend::Pad, Extend::Repeat, Extend::Reflect, Extend::None] {
        let ename = match extends {
            Extend::Pad => "pad",
            Extend::Repeat => "repeat",
            Extend::Reflect => "reflect",
            Extend::None => "none",
        };
        for (n, stops) in [(2u8, stops2()), (8, stops8())] {
            corpus.scene(format!("grad-linear-{n}-{ename}"), 128, 128, white, |l| {
                l.fill(
                    gradient_rect.clone(),
                    Paint::Linear(LinearGradient {
                        start: Point::new(32.0, 48.0),
                        end: Point::new(96.0, 80.0),
                        stops: stops.clone(),
                        extend: extends,
                        interpolation: ColorSpace::Srgb,
                    }),
                );
            });
            corpus.scene(format!("grad-radial-{n}-{ename}"), 128, 128, white, |l| {
                l.fill(
                    gradient_rect.clone(),
                    Paint::Radial(RadialGradient {
                        center0: Point::new(64.0, 64.0),
                        r0: 8.0,
                        center1: Point::new(80.0, 72.0),
                        r1: 40.0,
                        stops: stops.clone(),
                        extend: extends,
                        interpolation: ColorSpace::Srgb,
                    }),
                );
            });
            corpus.scene(format!("grad-sweep-{n}-{ename}"), 128, 128, white, |l| {
                l.fill(
                    gradient_rect.clone(),
                    Paint::Sweep(SweepGradient {
                        center: Point::new(64.0, 64.0),
                        start_angle: 0.0,
                        end_angle: 1.6 * std::f64::consts::PI,
                        stops: stops.clone(),
                        extend: extends,
                        interpolation: ColorSpace::Srgb,
                    }),
                );
            });
        }
    }

    // ---- Images ------------------------------------------------------------

    let checker = checker_png();
    let grad_img = gradient_png();

    corpus.scene_with_blobs(
        "img-nearest",
        96,
        96,
        white,
        |l| {
            l.image(
                ResourceHash::of(&checker),
                Rect::new(16.0, 16.0, 80.0, 80.0),
                Sampling::Nearest,
            );
        },
        vec![checker.clone()],
    );

    corpus.scene_with_blobs(
        "img-bilinear",
        96,
        96,
        white,
        |l| {
            l.image(
                ResourceHash::of(&grad_img),
                Rect::new(12.0, 12.0, 84.0, 84.0),
                Sampling::Bilinear,
            );
        },
        vec![grad_img.clone()],
    );

    corpus.scene_with_blobs(
        "imgpattern-repeat",
        128,
        128,
        white,
        |l| {
            l.fill(
                Shape::rounded_rect(8.0, 8.0, 112.0, 112.0, 16.0),
                Paint::Image(ImagePaint {
                    image: ResourceHash::of(&checker),
                    encoding: cherenkov_scene::ImageEncoding::default(),
                    transform: Affine::translate((40.0, 40.0)) * Affine::scale(4.0),
                    extend_x: Extend::Repeat,
                    extend_y: Extend::Repeat,
                    sampling: Sampling::Bilinear,
                }),
            );
        },
        vec![checker.clone()],
    );

    // ---- Clips, opacity, transforms ----------------------------------------

    corpus.scene("clip-nested", 128, 128, white, |l| {
        l.layer(|a| {
            a.clip(Shape::rounded_rect(8.0, 8.0, 112.0, 112.0, 24.0));
            a.fill(
                Shape::rect(0.0, 0.0, 128.0, 128.0),
                solid(srgb(0.9, 0.7, 0.1)),
            );
            a.layer(|b| {
                b.clip(Shape::rounded_rect(28.0, 28.0, 72.0, 72.0, 16.0));
                b.fill(
                    Shape::rect(0.0, 0.0, 128.0, 128.0),
                    solid(srgb(0.1, 0.3, 0.7)),
                );
                b.layer(|c| {
                    c.clip(Shape::circle(64.0, 64.0, 26.0));
                    c.fill(
                        Shape::rect(0.0, 0.0, 128.0, 128.0),
                        solid(srgb(0.9, 0.1, 0.2)),
                    );
                });
            });
        });
    });

    corpus.scene("clip-transform", 128, 128, white, |l| {
        l.layer(|a| {
            a.transform(rotate(0.4) * Affine::translate((-20.0, -10.0)));
            a.clip(Shape::rounded_rect(16.0, 16.0, 96.0, 96.0, 12.0));
            a.fill(
                Shape::rect(0.0, 0.0, 160.0, 160.0),
                solid(srgb(0.2, 0.6, 0.4)),
            );
        });
    });

    // Large path clips: at 320×320 the rasterized mask exceeds the glyph
    // atlas's mask budget and lives on its own texture.

    corpus.scene("clip-path-full-surface", 320, 320, white, |l| {
        // A rounded frame with an inward notch cut from its bottom edge.
        let mut frame = BezPath::new();
        frame.move_to((20.0, 20.0));
        frame.line_to((300.0, 20.0));
        frame.quad_to((312.0, 20.0), (312.0, 32.0));
        frame.line_to((312.0, 288.0));
        frame.quad_to((312.0, 300.0), (300.0, 300.0));
        frame.line_to((220.0, 300.0));
        frame.quad_to((200.0, 264.0), (180.0, 300.0));
        frame.line_to((20.0, 300.0));
        frame.quad_to((8.0, 300.0), (8.0, 288.0));
        frame.line_to((8.0, 32.0));
        frame.quad_to((8.0, 20.0), (20.0, 20.0));
        frame.close_path();
        l.layer(|a| {
            a.clip(Shape::Path { path: frame });
            // Fills overrunning the surface show the clip edge.
            a.fill(
                Shape::rect(-40.0, -40.0, 400.0, 200.0),
                solid(srgb(0.15, 0.45, 0.8)),
            );
            a.fill(
                Shape::rect(-40.0, 120.0, 400.0, 240.0),
                solid(srgb(0.85, 0.35, 0.15)),
            );
        });
    });

    corpus.scene("clip-path-oversized", 320, 320, white, |l| {
        // A star whose vertices lie far outside the surface.
        let mut star = BezPath::new();
        let c = Point::new(160.0, 160.0);
        for i in 0..5 {
            let angle = f64::from(i).mul_add(144.0, -90.0).to_radians();
            let p = c + Vec2::new(libm::cos(angle) * 240.0, libm::sin(angle) * 240.0);
            if i == 0 {
                star.move_to(p);
            } else {
                star.line_to(p);
            }
        }
        star.close_path();
        l.layer(|a| {
            a.clip(Shape::Path { path: star });
            a.fill(
                Shape::rect(-40.0, -40.0, 400.0, 200.0),
                solid(srgb(0.2, 0.7, 0.4)),
            );
            a.fill(
                Shape::rect(-40.0, 120.0, 400.0, 240.0),
                solid(srgb(0.55, 0.2, 0.7)),
            );
        });
    });

    corpus.scene("clip-path-nested", 320, 320, white, |l| {
        // Outer: a blob of arcs covering most of the surface.
        let mut blob = BezPath::new();
        blob.move_to((160.0, 16.0));
        blob.curve_to((300.0, 16.0), (304.0, 140.0), (304.0, 170.0));
        blob.curve_to((304.0, 304.0), (220.0, 304.0), (160.0, 304.0));
        blob.curve_to((40.0, 304.0), (16.0, 220.0), (16.0, 160.0));
        blob.curve_to((16.0, 60.0), (60.0, 16.0), (160.0, 16.0));
        blob.close_path();
        // Inner: a rotated square.
        let mut diamond = BezPath::new();
        diamond.move_to((160.0, 56.0));
        diamond.line_to((264.0, 160.0));
        diamond.line_to((160.0, 264.0));
        diamond.line_to((56.0, 160.0));
        diamond.close_path();
        l.layer(|a| {
            a.clip(Shape::Path { path: blob });
            a.fill(
                Shape::rect(-20.0, -20.0, 360.0, 360.0),
                solid(srgb(0.15, 0.5, 0.75)),
            );
            a.layer(|b| {
                b.clip(Shape::Path { path: diamond });
                b.fill(
                    Shape::rect(-20.0, -20.0, 360.0, 360.0),
                    solid(srgb(0.85, 0.55, 0.15)),
                );
                b.layer(|c| {
                    c.clip(Shape::rect(120.0, 120.0, 80.0, 80.0));
                    c.fill(
                        Shape::rect(-20.0, -20.0, 360.0, 360.0),
                        solid(srgb(0.75, 0.15, 0.35)),
                    );
                });
            });
        });
    });

    corpus.scene("clip-path-transform", 320, 320, white, |l| {
        let mut leaf = BezPath::new();
        leaf.move_to((40.0, 160.0));
        leaf.curve_to((40.0, 40.0), (160.0, 40.0), (160.0, 40.0));
        leaf.curve_to((280.0, 40.0), (280.0, 160.0), (160.0, 280.0));
        leaf.curve_to((100.0, 280.0), (40.0, 220.0), (40.0, 160.0));
        leaf.close_path();
        l.layer(|a| {
            a.transform(
                Affine::translate((160.0, 160.0))
                    * rotate(0.3)
                    * Affine::scale(1.4)
                    * Affine::translate((-160.0, -160.0)),
            );
            a.clip(Shape::Path { path: leaf });
            a.fill(
                Shape::rect(-60.0, -60.0, 440.0, 220.0),
                solid(srgb(0.3, 0.35, 0.85)),
            );
            a.fill(
                Shape::rect(-60.0, 140.0, 440.0, 260.0),
                solid(srgb(0.25, 0.7, 0.55)),
            );
        });
    });

    corpus.scene("layer-isolates-blended-child", 64, 96, white, |l| {
        l.layer(|outer| {
            outer.fill(
                Shape::rect(8.0, 8.0, 48.0, 80.0),
                solid(Color::new(ColorSpace::DisplayP3, [0.0, 0.85, 0.3, 1.0])),
            );
            outer.layer(|cutout| {
                cutout.blend(BlendMode::DestOut);
                cutout.fill(
                    Shape::rect(20.0, 30.0, 24.0, 36.0),
                    solid(srgb(1.0, 1.0, 1.0)),
                );
            });
        });
    });

    corpus.scene("layer-isolates-nested-blend", 64, 96, white, |l| {
        l.layer(|outer| {
            outer.fill(
                Shape::rect(4.0, 4.0, 56.0, 88.0),
                solid(Color::new(ColorSpace::LinearSrgb, [2.0, 0.3, 0.1, 1.0])),
            );
            outer.layer(|inner| {
                inner.fill(
                    Shape::rect(12.0, 20.0, 40.0, 56.0),
                    solid(srgb(0.12, 0.32, 0.9)),
                );
                inner.layer(|cutout| {
                    cutout.blend(BlendMode::DestOut);
                    cutout.fill(
                        Shape::rect(24.0, 36.0, 16.0, 24.0),
                        solid(srgb(1.0, 1.0, 1.0)),
                    );
                });
            });
        });
    });

    corpus.scene("layer-isolates-blended-child-hdr", 64, 96, white, |l| {
        l.layer(|outer| {
            outer.fill(Shape::rect(8.0, 8.0, 48.0, 80.0), solid(hdr(0.5, 6.0, 2.0)));
            outer.layer(|cutout| {
                cutout.blend(BlendMode::DestOut);
                cutout.fill(
                    Shape::rect(20.0, 30.0, 24.0, 36.0),
                    solid(srgb(1.0, 1.0, 1.0)),
                );
            });
        });
    });

    corpus.scene("layer-isolates-nested-blend-p3", 64, 96, white, |l| {
        l.layer(|outer| {
            outer.fill(Shape::rect(4.0, 4.0, 56.0, 88.0), solid(p3(0.0, 1.0, 0.0)));
            outer.layer(|inner| {
                inner.fill(
                    Shape::rect(12.0, 20.0, 40.0, 56.0),
                    solid(p3(0.0, 0.0, 1.0)),
                );
                inner.layer(|cutout| {
                    cutout.blend(BlendMode::DestOut);
                    cutout.fill(
                        Shape::rect(24.0, 36.0, 16.0, 24.0),
                        solid(srgb(1.0, 1.0, 1.0)),
                    );
                });
            });
        });
    });

    corpus.scene("group-opacity", 128, 128, white, |l| {
        l.fill(
            Shape::rect(0.0, 0.0, 128.0, 128.0),
            solid(srgb(0.9, 0.2, 0.2)),
        );
        l.layer(|a| {
            a.opacity(0.5);
            a.fill(Shape::circle(64.0, 64.0, 44.0), solid(srgb(0.1, 0.2, 0.8)));
        });
    });

    corpus.scene("transform-rotate", 128, 128, white, |l| {
        l.layer(|a| {
            a.transform(rotate_about(0.6, Point::new(64.0, 64.0)));
            a.fill(
                Shape::rounded_rect(24.0, 40.0, 80.0, 48.0, 10.0),
                solid(srgb(0.5, 0.2, 0.6)),
            );
            a.layer(|b| {
                b.transform(Affine::translate((8.0, 8.0)) * Affine::scale_non_uniform(1.0, 0.8));
                b.fill(
                    Shape::circle(64.0, 64.0, 20.0),
                    solid(srgba(0.1, 0.6, 0.8, 0.8)),
                );
            });
        });
    });

    // ---- Blend modes --------------------------------------------------------

    let modes = [
        ("multiply", BlendMode::Multiply),
        ("screen", BlendMode::Screen),
        ("overlay", BlendMode::Overlay),
        ("darken", BlendMode::Darken),
        ("lighten", BlendMode::Lighten),
        ("color-dodge", BlendMode::ColorDodge),
        ("color-burn", BlendMode::ColorBurn),
        ("hard-light", BlendMode::HardLight),
        ("soft-light", BlendMode::SoftLight),
        ("difference", BlendMode::Difference),
        ("exclusion", BlendMode::Exclusion),
        ("hue", BlendMode::Hue),
        ("saturation", BlendMode::Saturation),
        ("color", BlendMode::Color),
        ("luminosity", BlendMode::Luminosity),
        ("normal", BlendMode::Normal),
    ];
    for (mname, mode) in modes {
        corpus.scene(format!("blend-{mname}"), 96, 96, srgb(0.7, 0.5, 0.2), |l| {
            l.fill(
                Shape::rect(0.0, 0.0, 96.0, 96.0),
                Paint::Linear(LinearGradient {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(96.0, 96.0),
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: srgb(0.9, 0.5, 0.1),
                        },
                        GradientStop {
                            offset: 1.0,
                            color: srgb(0.1, 0.3, 0.8),
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation: ColorSpace::Srgb,
                }),
            );
            l.layer(|a| {
                a.blend(mode);
                a.fill(
                    Shape::circle(48.0, 48.0, 34.0),
                    Paint::Linear(LinearGradient {
                        start: Point::new(14.0, 14.0),
                        end: Point::new(82.0, 82.0),
                        stops: vec![
                            GradientStop {
                                offset: 0.0,
                                color: srgb(0.2, 0.9, 0.4),
                            },
                            GradientStop {
                                offset: 1.0,
                                color: srgb(0.9, 0.2, 0.6),
                            },
                        ],
                        extend: Extend::Pad,
                        interpolation: ColorSpace::Srgb,
                    }),
                );
            });
        });
    }

    // Clipped destructive layers: the operator is bounded by the rounded
    // clip rect, so the backdrop survives outside it. Three operators and
    // three palettes (sRGB, wide-gamut P3, HDR).
    for (mname, mode) in [
        ("clear", BlendMode::Clear),
        ("src", BlendMode::Src),
        ("dest-in", BlendMode::DestIn),
    ] {
        for (suffix, backdrop, content, interpolation) in [
            (
                "",
                [srgb(0.9, 0.5, 0.1), srgb(0.1, 0.3, 0.8)],
                [srgb(0.2, 0.9, 0.4), srgb(0.9, 0.2, 0.6)],
                ColorSpace::Srgb,
            ),
            (
                "-p3",
                [p3(1.0, 0.0, 0.6), p3(0.0, 1.0, 1.0)],
                [p3(0.0, 1.0, 0.0), p3(1.0, 0.0, 0.0)],
                ColorSpace::LinearP3,
            ),
            (
                "-hdr",
                [hdr(16.0, 2.0, 0.5), hdr(0.0, 8.0, 16.0)],
                [hdr(4.0, 16.0, 1.0), hdr(16.0, 16.0, 16.0)],
                ColorSpace::LinearP3,
            ),
        ] {
            blend_clip_scene(
                &mut corpus,
                format!("blend-{mname}-clip{suffix}"),
                mode,
                backdrop,
                content,
                interpolation,
            );
        }
    }

    // Every non-linear blend mode under a rounded clip the content
    // overruns: coverage at the anti-aliased clip edge must scale the
    // source, not the whole composite.
    for (mname, mode) in modes[..modes.len() - 1].iter().copied() {
        for (suffix, backdrop, content, interpolation) in [
            (
                "",
                [srgb(0.9, 0.5, 0.1), srgb(0.1, 0.3, 0.8)],
                [srgb(0.2, 0.9, 0.4), srgb(0.9, 0.2, 0.6)],
                ColorSpace::Srgb,
            ),
            (
                "-p3",
                [p3(1.0, 0.0, 0.6), p3(0.0, 1.0, 1.0)],
                [p3(0.0, 1.0, 0.0), p3(1.0, 0.0, 0.0)],
                ColorSpace::LinearP3,
            ),
            (
                "-hdr",
                [hdr(16.0, 2.0, 0.5), hdr(0.0, 8.0, 16.0)],
                [hdr(4.0, 16.0, 1.0), hdr(16.0, 16.0, 16.0)],
                ColorSpace::LinearP3,
            ),
        ] {
            blend_mode_clip_scene(
                &mut corpus,
                format!("blend-{mname}-clip{suffix}"),
                mode,
                backdrop,
                content,
                interpolation,
            );
        }
    }

    // ---- Shadows ------------------------------------------------------------

    for sigma in [0.0, 2.0, 5.0, 10.0, 20.0] {
        corpus.scene(format!("shadow-s{sigma:03.0}"), 128, 128, dark, |l| {
            l.shadow(
                Shape::rounded_rect(32.0, 32.0, 64.0, 64.0, 12.0),
                sigma,
                [0.0, 0.0],
                srgba(0.0, 0.0, 0.0, 0.8),
            );
            l.fill(
                Shape::rounded_rect(32.0, 32.0, 64.0, 64.0, 12.0),
                solid(srgb(0.95, 0.9, 0.3)),
            );
        });
    }

    corpus.scene("shadow-offset", 128, 128, white, |l| {
        l.shadow(
            Shape::circle(56.0, 56.0, 30.0),
            4.0,
            [10.0, 14.0],
            srgba(0.0, 0.0, 0.2, 0.7),
        );
        l.fill(Shape::circle(56.0, 56.0, 30.0), solid(srgb(0.2, 0.5, 0.9)));
    });

    // ---- Filters -----------------------------------------------------------

    let filter_color_variants = [
        ("", &FILTER_COLORS_SRGB),
        ("-p3", &FILTER_COLORS_P3),
        ("-hdr", &FILTER_COLORS_HDR),
    ];

    for &(suffix, colors) in &filter_color_variants {
        corpus.scene(
            format!("filter-color-matrix{suffix}"),
            128,
            128,
            white,
            |l| {
                l.layer(|group| {
                    group.filter(LayerFilter::ColorMatrix {
                        matrix: FILTER_COLOR_MATRIX,
                    });
                    filter_color_content(group, colors);
                });
            },
        );
    }

    for &(suffix, colors) in &filter_color_variants {
        corpus.scene(
            format!("filter-color-matrix-chain{suffix}"),
            128,
            128,
            white,
            |l| {
                l.layer(|group| {
                    group.filter(LayerFilter::ColorMatrixChain {
                        first: FILTER_SEPIA_MATRIX,
                        second: FILTER_CHAIN_SECOND,
                    });
                    filter_color_content(group, colors);
                });
            },
        );
    }

    for &(suffix, colors) in &filter_color_variants {
        corpus.scene(
            format!("filter-gaussian-blur{suffix}"),
            128,
            128,
            white,
            |l| {
                l.layer(|group| {
                    group.filter(LayerFilter::GaussianBlur { sigma: 4.0 });
                    filter_blur_content(group, colors);
                });
            },
        );
    }

    for &(suffix, colors) in &filter_color_variants {
        corpus.scene(format!("filter-box-blur{suffix}"), 128, 128, white, |l| {
            l.layer(|group| {
                group.filter(LayerFilter::BoxBlur { radius: 3.0 });
                filter_blur_content(group, colors);
            });
        });
    }

    corpus.scene("filter-gaussian-blur-small-sigma", 128, 128, white, |l| {
        l.layer(|group| {
            group.filter(LayerFilter::GaussianBlur { sigma: 0.6 });
            filter_blur_content(group, &FILTER_COLORS_P3);
        });
    });

    corpus.scene(
        "filter-gaussian-blur-hdr-highlights",
        128,
        128,
        Color::new(ColorSpace::LinearP3, [0.01, 0.012, 0.02, 1.0]),
        |l| {
            l.layer(|group| {
                group.filter(LayerFilter::GaussianBlur { sigma: 12.0 });
                group.fill(
                    Shape::rect(58.0, 58.0, 12.0, 12.0),
                    solid(Color::new(ColorSpace::LinearP3, [6.0, 5.0, 4.0, 1.0])),
                );
            });
        },
    );

    let image = filter_image_png();
    let image_hash = ResourceHash::of(&image);
    for (name, amount, mode) in [
        ("filter-blend-image", 0.8, FilterBlend::Multiply),
        (
            "filter-blend-image-luminosity",
            1.0,
            FilterBlend::Luminosity,
        ),
    ] {
        for &(suffix, colors) in &filter_color_variants {
            corpus.scene_with_blobs(
                format!("{name}{suffix}"),
                128,
                128,
                white,
                |l| {
                    l.layer(|group| {
                        group.opacity(0.82);
                        group.filter(LayerFilter::BlendImage {
                            image: image_hash,
                            amount,
                            mode,
                        });
                        filter_blend_content(group, colors);
                    });
                },
                vec![image.clone()],
            );
        }
    }

    for &(suffix, colors) in &filter_color_variants {
        corpus.scene(format!("filter-nested{suffix}"), 64, 256, white, |l| {
            l.layer(|outer| {
                outer.opacity(0.8);
                outer.filter(LayerFilter::ColorMatrix {
                    matrix: FILTER_COLOR_MATRIX,
                });
                outer.layer(|inner| {
                    inner.filter(LayerFilter::GaussianBlur { sigma: 6.0 });
                    filter_nested_content(inner, colors);
                });
            });
        });
    }

    for &(suffix, colors) in &filter_color_variants {
        corpus.scene(
            format!("filter-blended-descendant{suffix}"),
            64,
            96,
            white,
            |l| {
                l.layer(|outer| {
                    outer.filter(LayerFilter::GaussianBlur { sigma: 3.0 });
                    filter_blended_descendant_content(outer, colors);
                });
            },
        );
    }

    for &(suffix, colors) in &filter_color_variants {
        corpus.scene(
            format!("filter-isolates-nested-blend{suffix}"),
            64,
            96,
            white,
            |l| {
                l.layer(|outer| {
                    outer.filter(LayerFilter::GaussianBlur { sigma: 3.0 });
                    filter_isolates_nested_blend_content(outer, colors);
                });
            },
        );
    }

    // ---- HDR -----------------------------------------------------------------

    corpus.scene(
        "hdr-bright",
        96,
        96,
        Color::new(ColorSpace::LinearSrgb, [0.02, 0.02, 0.02, 1.0]),
        |l| {
            l.fill(
                Shape::rect(8.0, 8.0, 80.0, 80.0),
                Paint::Linear(LinearGradient {
                    start: Point::new(8.0, 48.0),
                    end: Point::new(88.0, 48.0),
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: Color::new(ColorSpace::LinearSrgb, [0.4, 0.2, 0.1, 1.0]),
                        },
                        GradientStop {
                            offset: 0.5,
                            color: Color::new(ColorSpace::LinearSrgb, [2.5, 1.8, 0.4, 1.0]),
                        },
                        GradientStop {
                            offset: 1.0,
                            color: Color::new(ColorSpace::Rec2020, [1.5, 0.2, 1.2, 1.0]),
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation: ColorSpace::LinearSrgb,
                }),
            );
            l.fill(
                Shape::circle(48.0, 48.0, 22.0),
                solid(Color::new(ColorSpace::DisplayP3, [1.0, 0.4, 0.0, 1.0])),
            );
        },
    );

    // ---- Wide gamut (linear P3) and HDR ------------------------------------
    //
    // A `-p3` sibling of each feature family draws the same geometry with
    // colours outside the sRGB gamut; a `-hdr` sibling uses linear-P3
    // colours with channels reaching 16.0.

    corpus.scene("fill-p3", 96, 96, white, |l| {
        l.fill(Shape::rect(8.0, 8.0, 48.0, 48.0), solid(p3(0.0, 1.0, 0.0)));
        l.fill(
            Shape::rect(40.0, 40.0, 48.0, 48.0),
            solid(Color::new(ColorSpace::LinearP3, [1.0, 0.0, 0.6, 0.7])),
        );
    });
    corpus.scene("fill-hdr", 96, 96, white, |l| {
        l.fill(
            Shape::rect(8.0, 8.0, 48.0, 48.0),
            solid(hdr(16.0, 16.0, 16.0)),
        );
        l.fill(
            Shape::rect(40.0, 40.0, 48.0, 48.0),
            solid(Color::new(ColorSpace::LinearP3, [16.0, 2.0, 0.5, 0.7])),
        );
    });

    corpus.scene("stroke-p3", 128, 128, white, |l| {
        for (w, i) in [1.0, 3.0, 8.0].iter().zip(0u16..) {
            let y = f64::from(i).mul_add(40.0, 24.0);
            l.stroke(
                Shape::Line(Line::new((12.0, y), (116.0, y))),
                StrokeStyle {
                    width: *w,
                    ..StrokeStyle::default()
                },
                solid(p3(0.0, 1.0, 1.0)),
            );
        }
    });
    corpus.scene("stroke-hdr", 128, 128, white, |l| {
        for (w, i) in [1.0, 3.0, 8.0].iter().zip(0u16..) {
            let y = f64::from(i).mul_add(40.0, 24.0);
            l.stroke(
                Shape::Line(Line::new((12.0, y), (116.0, y))),
                StrokeStyle {
                    width: *w,
                    ..StrokeStyle::default()
                },
                solid(hdr(16.0, 16.0, 16.0)),
            );
        }
    });

    corpus.scene("stroke-dash-p3", 128, 128, white, |l| {
        l.stroke(
            Shape::Path {
                path: curved_path(),
            },
            StrokeStyle {
                width: 3.0,
                dash_pattern: vec![8.0, 4.0, 2.0, 4.0],
                dash_offset: 2.0,
                ..StrokeStyle::default()
            },
            solid(p3(1.0, 0.0, 0.6)),
        );
    });
    corpus.scene("stroke-dash-hdr", 128, 128, white, |l| {
        l.stroke(
            Shape::Path {
                path: curved_path(),
            },
            StrokeStyle {
                width: 3.0,
                dash_pattern: vec![8.0, 4.0, 2.0, 4.0],
                dash_offset: 2.0,
                ..StrokeStyle::default()
            },
            solid(hdr(0.0, 8.0, 16.0)),
        );
    });

    // Ellipse and Lamé corners with wide-gamut / HDR paint (#160).
    for (shape, name) in [
        (
            Shape::Continuous(cherenkov_scene::ContinuousRect::new(
                Rect::new(16.0, 16.0, 80.0, 80.0),
                28.0,
                0.5,
            )),
            "continuous-s50",
        ),
        (
            Shape::Ellipse(Ellipse::new((48.0, 48.0), (40.0, 24.0), 0.0)),
            "ellipse",
        ),
    ] {
        corpus.scene(format!("{name}-p3"), 96, 96, white, |l| {
            l.fill(shape.clone(), solid(p3(0.0, 1.0, 0.0)));
            l.fill(
                shape.clone(),
                solid(Color::new(ColorSpace::LinearP3, [1.0, 0.0, 0.6, 0.7])),
            );
        });
        corpus.scene(format!("{name}-hdr"), 96, 96, white, |l| {
            l.fill(shape.clone(), solid(hdr(16.0, 16.0, 16.0)));
            l.fill(
                shape.clone(),
                solid(Color::new(ColorSpace::LinearP3, [16.0, 2.0, 0.5, 0.7])),
            );
        });
    }

    // Gradient stops crossing the sRGB boundary (p3) or the [0,1] range
    // (hdr); both interpolate in the working space.
    let stops_p3 = vec![
        GradientStop {
            offset: 0.0,
            color: srgb(0.2, 0.4, 0.9),
        },
        GradientStop {
            offset: 0.35,
            color: p3(0.0, 1.0, 1.0),
        },
        GradientStop {
            offset: 0.7,
            color: p3(0.0, 1.0, 0.0),
        },
        GradientStop {
            offset: 1.0,
            color: p3(1.0, 0.0, 0.6),
        },
    ];
    let stops_hdr = vec![
        GradientStop {
            offset: 0.0,
            color: hdr(0.5, 0.5, 0.5),
        },
        GradientStop {
            offset: 0.35,
            color: hdr(4.0, 16.0, 1.0),
        },
        GradientStop {
            offset: 0.7,
            color: hdr(16.0, 16.0, 16.0),
        },
        GradientStop {
            offset: 1.0,
            color: hdr(16.0, 2.0, 0.5),
        },
    ];
    for (suffix, stops) in [("p3", stops_p3), ("hdr", stops_hdr)] {
        corpus.scene(format!("grad-linear-{suffix}"), 128, 128, white, |l| {
            l.fill(
                gradient_rect.clone(),
                Paint::Linear(LinearGradient {
                    start: Point::new(32.0, 48.0),
                    end: Point::new(96.0, 80.0),
                    stops: stops.clone(),
                    extend: Extend::Pad,
                    interpolation: ColorSpace::LinearP3,
                }),
            );
        });
        corpus.scene(format!("grad-radial-{suffix}"), 128, 128, white, |l| {
            l.fill(
                gradient_rect.clone(),
                Paint::Radial(RadialGradient {
                    center0: Point::new(64.0, 64.0),
                    r0: 8.0,
                    center1: Point::new(80.0, 72.0),
                    r1: 40.0,
                    stops: stops.clone(),
                    extend: Extend::Pad,
                    interpolation: ColorSpace::LinearP3,
                }),
            );
        });
        corpus.scene(format!("grad-sweep-{suffix}"), 128, 128, white, |l| {
            l.fill(
                gradient_rect.clone(),
                Paint::Sweep(SweepGradient {
                    center: Point::new(64.0, 64.0),
                    start_angle: 0.0,
                    end_angle: 1.6 * std::f64::consts::PI,
                    stops: stops.clone(),
                    extend: Extend::Pad,
                    interpolation: ColorSpace::LinearP3,
                }),
            );
        });
    }

    // Images in wide-gamut and half-float encodings. The P3 PNG carries
    // Display-P3-encoded bytes; the f16 blobs are linear P3 straight alpha
    // with a radial ramp to 16.0 and some alpha < 1 texels.
    let p3_img = p3_png(32, 32);
    let f16_img = rgba16f_blob(32, 32, |x, y| {
        let dx = f32::from(u16::try_from(x).expect("blob coords fit u16")) - 15.5;
        let dy = f32::from(u16::try_from(y).expect("blob coords fit u16")) - 15.5;
        let d = (dx * dx + dy * dy).sqrt() / 16.0;
        let v = (1.0 - d.min(1.0)) * 16.0;
        [v, v * 0.5, 16.0 - v, if x < 8 { 0.5 } else { 1.0 }]
    });
    let f16_encoding = ImageEncoding::Rgba16F {
        width: 32,
        height: 32,
        color_space: ImageColorSpace::LinearP3,
    };

    corpus.scene_with_blobs(
        "img-p3",
        96,
        96,
        white,
        |l| {
            l.image_encoded(
                ResourceHash::of(&p3_img),
                ImageEncoding::Png {
                    color_space: ImageColorSpace::DisplayP3,
                },
                Rect::new(12.0, 12.0, 84.0, 84.0),
                Sampling::Bilinear,
            );
        },
        vec![p3_img.clone()],
    );
    corpus.scene_with_blobs(
        "img-f16-hdr",
        96,
        96,
        white,
        |l| {
            l.image_encoded(
                ResourceHash::of(&f16_img),
                f16_encoding,
                Rect::new(12.0, 12.0, 84.0, 84.0),
                Sampling::Bilinear,
            );
        },
        vec![f16_img.clone()],
    );
    corpus.scene_with_blobs(
        "imgpattern-p3",
        128,
        128,
        white,
        |l| {
            l.fill(
                Shape::rounded_rect(8.0, 8.0, 112.0, 112.0, 16.0),
                Paint::Image(ImagePaint {
                    image: ResourceHash::of(&p3_img),
                    encoding: ImageEncoding::Png {
                        color_space: ImageColorSpace::DisplayP3,
                    },
                    transform: Affine::translate((40.0, 40.0)) * Affine::scale(4.0),
                    extend_x: Extend::Repeat,
                    extend_y: Extend::Repeat,
                    sampling: Sampling::Bilinear,
                }),
            );
        },
        vec![p3_img.clone()],
    );
    corpus.scene_with_blobs(
        "imgpattern-f16-hdr",
        128,
        128,
        white,
        |l| {
            l.fill(
                Shape::rounded_rect(8.0, 8.0, 112.0, 112.0, 16.0),
                Paint::Image(ImagePaint {
                    image: ResourceHash::of(&f16_img),
                    encoding: f16_encoding,
                    transform: Affine::translate((40.0, 40.0)) * Affine::scale(4.0),
                    extend_x: Extend::Repeat,
                    extend_y: Extend::Repeat,
                    sampling: Sampling::Bilinear,
                }),
            );
        },
        vec![f16_img.clone()],
    );

    // HDR presentation scenes for the #97 tone map: identical content at
    // declared display headrooms 1, 2 and 4 — an 0..8x SDR-white ramp, nine
    // saturated P3 highlight swatches, and the f16 radial HDR image.
    for (hname, headroom) in [("h1", 1.0_f64), ("h2", 2.0), ("h4", 4.0)] {
        corpus.scene_headroom(
            format!("hdr-gradient-{hname}"),
            128,
            128,
            white,
            headroom,
            |l| {
                for (top, to) in [
                    (8.0, hdr(8.0, 8.0, 8.0)),
                    (48.0, hdr(8.0, 0.0, 0.0)),
                    (88.0, hdr(8.0, 3.0, 0.3)),
                ] {
                    l.fill(
                        Shape::rect(8.0, top, 112.0, 32.0),
                        Paint::Linear(LinearGradient {
                            start: Point::new(8.0, 0.0),
                            end: Point::new(120.0, 0.0),
                            stops: vec![
                                GradientStop {
                                    offset: 0.0,
                                    color: hdr(0.0, 0.0, 0.0),
                                },
                                GradientStop {
                                    offset: 1.0,
                                    color: to,
                                },
                            ],
                            extend: Extend::Pad,
                            interpolation: ColorSpace::LinearP3,
                        }),
                    );
                }
            },
        );
        corpus.scene_headroom(
            format!("hdr-p3-highlights-{hname}"),
            96,
            96,
            white,
            headroom,
            |l| {
                l.fill(
                    Shape::rect(0.0, 0.0, 96.0, 96.0),
                    solid(hdr(0.02, 0.02, 0.04)),
                );
                for (i, swatch) in [
                    hdr(4.0, 0.0, 0.0),
                    hdr(0.0, 4.0, 0.0),
                    hdr(0.0, 0.0, 4.0),
                    hdr(4.0, 4.0, 0.0),
                    hdr(0.0, 4.0, 4.0),
                    hdr(4.0, 0.0, 4.0),
                    hdr(4.0, 2.0, 0.5),
                    hdr(8.0, 4.0, 1.0),
                    hdr(8.0, 8.0, 8.0),
                ]
                .into_iter()
                .enumerate()
                {
                    let x = 4.0 + 32.0 * f64::from(u32::try_from(i % 3).unwrap());
                    let y = 4.0 + 32.0 * f64::from(u32::try_from(i / 3).unwrap());
                    l.fill(Shape::rect(x, y, 28.0, 28.0), solid(swatch));
                }
            },
        );
        corpus.scene_from(
            format!("hdr-image-f16-{hname}"),
            Scene::builder(128, 128)
                .clear(white)
                .present_headroom(headroom),
            |l| {
                l.fill(
                    Shape::rect(8.0, 8.0, 112.0, 112.0),
                    Paint::Image(ImagePaint {
                        image: ResourceHash::of(&f16_img),
                        encoding: f16_encoding,
                        transform: Affine::translate((8.0, 8.0)) * Affine::scale(3.5),
                        extend_x: Extend::Pad,
                        extend_y: Extend::Pad,
                        sampling: Sampling::Bilinear,
                    }),
                );
            },
            vec![f16_img.clone()],
        );
    }

    // ---- #98 presentation corpus ------------------------------------------
    // The content every `render --present` encoding (P3, scRGB, extended
    // sRGB/P3, PQ, HLG) must carry: neutral values 0/0.18/1/2/4/8, P3
    // colours outside sRGB, negative extended components, transparent
    // coloured edges, and a glass highlight above SDR white. The headroom
    // sequence 4→2→1→4 is display state, not scene content — cherenkov's
    // `headroom_updates_present_without_regenerating_content` test covers
    // it. Declared headroom 4 exercises the HDR tone-map branch.
    corpus.scene_headroom("present-neutrals", 196, 44, white, 4.0, |l| {
        for (i, v) in [0.0f32, 0.18, 1.0, 2.0, 4.0, 8.0].into_iter().enumerate() {
            let x = 4.0 + 32.0 * f64::from(u32::try_from(i).unwrap());
            l.fill(Shape::rect(x, 4.0, 28.0, 36.0), solid(hdr(v, v, v)));
        }
    });

    corpus.scene_headroom("present-p3-outside-srgb", 100, 100, white, 4.0, |l| {
        for (i, swatch) in [
            p3(1.0, 0.0, 0.0),
            p3(0.0, 1.0, 0.0),
            p3(0.0, 0.0, 1.0),
            p3(0.0, 1.0, 0.4),
            p3(1.0, 0.0, 0.6),
            p3(1.0, 0.6, 0.0),
            p3(0.0, 0.9, 0.9),
            p3(0.6, 0.0, 1.0),
            p3(0.2, 1.0, 0.2),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 4.0 + 32.0 * f64::from(u32::try_from(i % 3).unwrap());
            let y = 4.0 + 32.0 * f64::from(u32::try_from(i / 3).unwrap());
            l.fill(Shape::rect(x, y, 28.0, 28.0), solid(swatch));
        }
    });

    corpus.scene_headroom("present-extended-negatives", 100, 68, white, 4.0, |l| {
        for (i, swatch) in [
            Color::new(ColorSpace::LinearP3, [-0.3, 0.7, 0.4, 1.0]),
            Color::new(ColorSpace::LinearP3, [0.1, -0.15, 0.3, 1.0]),
            Color::new(ColorSpace::LinearP3, [1.2, -0.05, 0.5, 1.0]),
            Color::new(ColorSpace::LinearP3, [-0.2, -0.2, 1.0, 1.0]),
            Color::new(ColorSpace::LinearP3, [0.5, 0.5, -0.1, 1.0]),
            Color::new(ColorSpace::LinearP3, [-0.05, 2.0, -0.05, 1.0]),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 4.0 + 32.0 * f64::from(u32::try_from(i % 3).unwrap());
            let y = 4.0 + 32.0 * f64::from(u32::try_from(i / 3).unwrap());
            l.fill(Shape::rect(x, y, 28.0, 28.0), solid(swatch));
        }
    });

    // Fractional-alpha wide-gamut fills: the anti-aliased edges are the
    // transparent coloured edges the premultiply/encode order must carry.
    corpus.scene_headroom(
        "present-transparent-edges",
        96,
        96,
        srgb(0.06, 0.06, 0.09),
        4.0,
        |l| {
            l.fill(
                Shape::circle(30.0, 30.0, 22.0),
                solid(Color::new(ColorSpace::LinearP3, [1.0, 0.1, 0.3, 0.5])),
            );
            l.fill(
                Shape::circle(66.0, 30.0, 22.0),
                solid(Color::new(ColorSpace::LinearP3, [0.0, 1.0, 0.5, 0.25])),
            );
            l.fill(
                Shape::rounded_rect(12.0, 56.0, 52.0, 90.0, 8.0),
                solid(Color::new(ColorSpace::LinearP3, [0.2, 0.4, 1.0, 0.75])),
            );
            l.layer(|m| {
                m.transform(rotate_about(0.4, Point::new(76.0, 72.0)));
                m.fill(
                    Shape::rect(60.0, 56.0, 92.0, 88.0),
                    solid(Color::new(ColorSpace::LinearP3, [2.0, 1.4, 0.4, 0.6])),
                );
            });
        },
    );

    // "Glass": a blurred backdrop panel over HDR content whose specular
    // highlight sits above SDR white — the transparency edge case for
    // every encoded output.
    let mut glass = Scene::builder(256, 256).clear(white).present_headroom(4.0);
    glass.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 6.0 }]);
    corpus.scene_from(
        "present-glass-highlights",
        glass,
        |l| {
            l.fill(
                Shape::rect(0.0, 0.0, 256.0, 256.0),
                Paint::Linear(LinearGradient {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(256.0, 256.0),
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: p3(0.05, 0.10, 0.30),
                        },
                        GradientStop {
                            offset: 1.0,
                            color: p3(0.45, 0.12, 0.05),
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation: ColorSpace::LinearP3,
                }),
            );
            l.fill(Shape::circle(96.0, 96.0, 48.0), solid(hdr(4.0, 1.0, 0.5)));
            l.fill(
                Shape::rect(140.0, 120.0, 236.0, 190.0),
                solid(Color::new(ColorSpace::Rec2020, [0.9, 0.15, 0.6, 1.0])),
            );
            l.layer(|m| {
                let clip = Shape::RoundedRect(RoundedRect::new(40.0, 40.0, 216.0, 216.0, 28.0));
                m.clip(clip);
                m.backdrop(1);
                // The specular strip: translucent white at 4x SDR white.
                m.fill(
                    Shape::rect(48.0, 52.0, 208.0, 76.0),
                    solid(Color::new(ColorSpace::LinearP3, [4.0, 4.0, 4.0, 0.5])),
                );
                m.fill(
                    Shape::rect(42.0, 42.0, 172.0, 172.0),
                    solid(srgba(1.0, 1.0, 1.0, 0.1)),
                );
            });
        },
        Vec::new(),
    );

    // Blend modes over P3 and HDR content, same geometry as `blend-*`.
    for mode in BlendMode::ALL {
        let mname = serde_json::to_value(mode)
            .expect("blend mode serializes")
            .as_str()
            .expect("blend mode name is a string")
            .to_owned();
        corpus.scene(
            format!("blend-{mname}-p3"),
            96,
            96,
            srgb(0.7, 0.5, 0.2),
            |l| {
                l.fill(
                    Shape::rect(0.0, 0.0, 96.0, 96.0),
                    Paint::Linear(LinearGradient {
                        start: Point::new(0.0, 0.0),
                        end: Point::new(96.0, 96.0),
                        stops: vec![
                            GradientStop {
                                offset: 0.0,
                                color: p3(1.0, 0.0, 0.6),
                            },
                            GradientStop {
                                offset: 1.0,
                                color: p3(0.0, 1.0, 1.0),
                            },
                        ],
                        extend: Extend::Pad,
                        interpolation: ColorSpace::LinearP3,
                    }),
                );
                l.layer(|a| {
                    a.blend(mode);
                    a.fill(
                        Shape::circle(48.0, 48.0, 34.0),
                        Paint::Linear(LinearGradient {
                            start: Point::new(14.0, 14.0),
                            end: Point::new(82.0, 82.0),
                            stops: vec![
                                GradientStop {
                                    offset: 0.0,
                                    color: p3(0.0, 1.0, 0.0),
                                },
                                GradientStop {
                                    offset: 1.0,
                                    color: p3(1.0, 0.0, 0.0),
                                },
                            ],
                            extend: Extend::Pad,
                            interpolation: ColorSpace::LinearP3,
                        }),
                    );
                });
            },
        );
        corpus.scene(
            format!("blend-{mname}-hdr"),
            96,
            96,
            srgb(0.7, 0.5, 0.2),
            |l| {
                l.fill(
                    Shape::rect(0.0, 0.0, 96.0, 96.0),
                    Paint::Linear(LinearGradient {
                        start: Point::new(0.0, 0.0),
                        end: Point::new(96.0, 96.0),
                        stops: vec![
                            GradientStop {
                                offset: 0.0,
                                color: hdr(16.0, 2.0, 0.5),
                            },
                            GradientStop {
                                offset: 1.0,
                                color: hdr(0.0, 8.0, 16.0),
                            },
                        ],
                        extend: Extend::Pad,
                        interpolation: ColorSpace::LinearP3,
                    }),
                );
                l.layer(|a| {
                    a.blend(mode);
                    a.fill(
                        Shape::circle(48.0, 48.0, 34.0),
                        Paint::Linear(LinearGradient {
                            start: Point::new(14.0, 14.0),
                            end: Point::new(82.0, 82.0),
                            stops: vec![
                                GradientStop {
                                    offset: 0.0,
                                    color: hdr(4.0, 16.0, 1.0),
                                },
                                GradientStop {
                                    offset: 1.0,
                                    color: hdr(16.0, 16.0, 16.0),
                                },
                            ],
                            extend: Extend::Pad,
                            interpolation: ColorSpace::LinearP3,
                        }),
                    );
                });
            },
        );
    }

    corpus.scene("shadow-p3", 128, 128, dark, |l| {
        l.shadow(
            Shape::rounded_rect(32.0, 32.0, 64.0, 64.0, 12.0),
            5.0,
            [0.0, 0.0],
            Color::new(ColorSpace::LinearP3, [0.0, 0.5, 0.0, 0.8]),
        );
        l.fill(
            Shape::rounded_rect(32.0, 32.0, 64.0, 64.0, 12.0),
            solid(p3(1.0, 0.0, 0.0)),
        );
    });
    corpus.scene("shadow-hdr", 128, 128, dark, |l| {
        l.shadow(
            Shape::rounded_rect(32.0, 32.0, 64.0, 64.0, 12.0),
            5.0,
            [0.0, 0.0],
            Color::new(ColorSpace::LinearP3, [8.0, 8.0, 8.0, 0.8]),
        );
        l.fill(
            Shape::rounded_rect(32.0, 32.0, 64.0, 64.0, 12.0),
            solid(hdr(4.0, 16.0, 1.0)),
        );
    });

    // ---- Silhouette shadows --------------------------------------------------
    //
    // Shapes the analytic rounded-box kernel cannot express — paths,
    // ellipses, superellipse rects, and any shape under a non-axis-aligned
    // transform — capture and convolve a silhouette instead. The shared
    // layout: a blue shadow, offset down-right, under an orange fill of the
    // same shape on a light clear.

    let blue = srgba(0.1, 0.25, 0.6, 0.8);
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-ellipse",
        shadow_ellipse(),
        Affine::IDENTITY,
        blue,
    );
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-path",
        shadow_star(),
        Affine::IDENTITY,
        blue,
    );
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-affine-path",
        shadow_star(),
        Affine::new([0.72, 0.22, -0.15, 0.7, 28.0, 4.0]),
        blue,
    );
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-affine-ellipse",
        shadow_ellipse(),
        Affine::new([0.72, 0.22, -0.15, 0.7, 28.0, 4.0]),
        blue,
    );
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-affine-continuous",
        shadow_uneven(),
        Affine::new([0.72, 0.22, -0.15, 0.7, 28.0, 4.0]),
        blue,
    );
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-continuous",
        shadow_uneven(),
        Affine::IDENTITY,
        blue,
    );
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-p3",
        shadow_star(),
        Affine::IDENTITY,
        Color::new(ColorSpace::LinearP3, [1.0, 0.0, 0.3, 0.9]),
    );
    silhouette_scene(
        &mut corpus,
        "shadow-silhouette-hdr",
        shadow_star(),
        Affine::IDENTITY,
        Color::new(ColorSpace::LinearP3, [3.6, 0.0, 0.6, 0.9]),
    );

    // ---- Text ----------------------------------------------------------------

    let text_specs: &[(&str, &str, &str, f32)] = &[
        ("NotoSans.ttf", "text-latin", corpus::LATIN, 30.0),
        ("NotoSansSC.ttf", "text-cjk", corpus::CJK, 30.0),
        ("NotoSansArabic.ttf", "text-arabic", corpus::ARABIC, 34.0),
        ("NotoSansHebrew.ttf", "text-hebrew", corpus::HEBREW, 34.0),
        (
            "NotoSansDevanagari.ttf",
            "text-devanagari",
            corpus::DEVANAGARI,
            34.0,
        ),
        ("NotoSansThai.ttf", "text-thai", corpus::THAI, 30.0),
        ("NotoEmoji.ttf", "text-emoji", corpus::EMOJI, 48.0),
        ("Nabla.ttf", "text-colr", corpus::COLR, 64.0),
    ];
    for (file, name, text, size) in text_specs {
        let runs = ctx.shape(file, text, *size, FontWeight::NORMAL, &solid(dark));
        let blobs: Vec<Vec<u8>> = runs.iter().map(|r| font_blob(&ctx, r).clone()).collect();
        corpus.scene_with_blobs(
            *name,
            320,
            160,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    // Wide-gamut and HDR text: the same Latin sheet and the COLR glyphs
    // drawn with linear-P3 paints.
    for (name, paint) in [
        ("text-p3", solid(p3(0.0, 1.0, 0.0))),
        ("text-hdr", solid(hdr(16.0, 2.0, 0.5))),
    ] {
        let runs = ctx.shape(
            "NotoSans.ttf",
            corpus::LATIN,
            30.0,
            FontWeight::NORMAL,
            &paint,
        );
        let blobs: Vec<Vec<u8>> = runs.iter().map(|r| font_blob(&ctx, r).clone()).collect();
        corpus.scene_with_blobs(
            name,
            320,
            160,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }
    for (name, paint) in [
        ("text-colr-p3", solid(p3(0.0, 1.0, 1.0))),
        ("text-colr-hdr", solid(hdr(4.0, 16.0, 1.0))),
    ] {
        let runs = ctx.shape("Nabla.ttf", corpus::COLR, 64.0, FontWeight::NORMAL, &paint);
        let blobs: Vec<Vec<u8>> = runs.iter().map(|r| font_blob(&ctx, r).clone()).collect();
        corpus.scene_with_blobs(
            name,
            320,
            160,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    // A variable-weight Latin line: the font is a `wdth,wght` variable font, so
    // a bold weight lands as non-zero normalized coords.
    {
        let text = "AaBbGg 0123456789";
        let runs = ctx.shape("NotoSans.ttf", text, 40.0, FontWeight::BOLD, &solid(dark));
        let blobs: Vec<Vec<u8>> = runs.iter().map(|r| font_blob(&ctx, r).clone()).collect();
        corpus.scene_with_blobs(
            "text-weight",
            320,
            128,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    // Parley layouts recorded through the engine's text adapter (#26).
    text_layout::scenes(&mut corpus, &ctx);

    // ---- COLR test-font scenes ----------------------------------------------
    //
    // `CherenkovColrTest.ttf` (in `corpus::TEST_FONTS`) puts predictable
    // COLRv1 paints on PUA codepoints; these scenes exercise transformed
    // brushes, composite modes, clips, foreground brushes and a COLR v0
    // record. The foreground paint is evaluated in font units, like the
    // GPU and oracle backends bake it.
    let colr_font = "CherenkovColrTest.ttf";

    {
        let text: String = (0xe000u32..=0xe008)
            .map(|c| char::from_u32(c).unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let runs = ctx.shape(colr_font, &text, 56.0, FontWeight::NORMAL, &solid(dark));
        let blobs = font_blobs(&ctx, &[&runs]);
        corpus.scene_with_blobs(
            "colr-gradient-transform",
            320,
            200,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    {
        let text: String = (0xe100u32..=0xe11b)
            .map(|c| char::from_u32(c).unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let runs = ctx.shape(colr_font, &text, 30.0, FontWeight::NORMAL, &solid(dark));
        let blobs = font_blobs(&ctx, &[&runs]);
        corpus.scene_with_blobs(
            "colr-composite",
            320,
            150,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    {
        let text = "\u{e200} \u{e201} \u{e400} \u{e500} \u{e600}";
        let runs = ctx.shape(colr_font, text, 44.0, FontWeight::NORMAL, &solid(dark));
        let blobs = font_blobs(&ctx, &[&runs]);
        corpus.scene_with_blobs(
            "colr-clip-nested",
            320,
            64,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    {
        // The run paint is a gradient in font units: a horizontal
        // red→blue ramp across the em.
        let fg = Paint::Linear(LinearGradient {
            start: Point::new(0.0, 450.0),
            end: Point::new(1000.0, 450.0),
            stops: stops2(),
            extend: Extend::Pad,
            interpolation: ColorSpace::Srgb,
        });
        let text = "\u{e300} \u{e301} \u{e302} \u{e303}";
        let runs = ctx.shape(colr_font, text, 52.0, FontWeight::NORMAL, &fg);
        let blobs = font_blobs(&ctx, &[&runs]);
        corpus.scene_with_blobs(
            "colr-foreground-gradient",
            320,
            80,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    {
        // The run paint is the 8x8 checker image at 8 font units per
        // tile: glyph U+E301 carries COLR alpha 0.4 over an image
        // foreground, so it must read as 40% of the image scene.
        let fg = Paint::Image(ImagePaint {
            image: ResourceHash::of(&checker),
            transform: Affine::scale(8.0),
            extend_x: Extend::Repeat,
            extend_y: Extend::Repeat,
            sampling: Sampling::Bilinear,
            encoding: ImageEncoding::default(),
        });
        let text = "\u{e300} \u{e301}";
        let runs = ctx.shape(colr_font, text, 52.0, FontWeight::NORMAL, &fg);
        let mut blobs = font_blobs(&ctx, &[&runs]);
        blobs.push(checker.clone());
        corpus.scene_with_blobs(
            "colr-foreground-image",
            320,
            80,
            white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    {
        // A whole-run affine over the Nabla COLR text.
        let runs = ctx.shape(
            "Nabla.ttf",
            corpus::COLR,
            64.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let blobs = font_blobs(&ctx, &[&runs]);
        corpus.scene_with_blobs(
            "colr-run-transform",
            320,
            160,
            white,
            |l| {
                l.layer(|a| {
                    a.transform(
                        Affine::translate((70.0, 95.0))
                            * rotate(-0.35)
                            * Affine::skew(0.3, 0.0)
                            * Affine::scale_non_uniform(1.2, 0.8),
                    );
                    for run in &runs {
                        a.glyphs(run.clone());
                    }
                });
            },
            blobs,
        );
    }

    // ---- Winding test-font scenes -------------------------------------------
    //
    // `CherenkovWindingTest.ttf` (in `corpus::TEST_FONTS`) puts overlapping
    // contours on PUA codepoints: a composite of two overlapping circles, a
    // self-crossing bowtie and two same-direction nested squares. Each must
    // fill by its union, as paths do since #136.
    {
        let text = "\u{e200}\u{e201}\u{e202}";
        for (name, paint) in [
            ("glyph-overlap-winding", solid(srgb(0.2, 0.45, 0.85))),
            ("glyph-overlap-winding-p3", solid(p3(0.0, 0.4, 1.0))),
            ("glyph-overlap-winding-hdr", solid(hdr(0.0, 8.0, 16.0))),
        ] {
            let runs = ctx.shape(
                "CherenkovWindingTest.ttf",
                text,
                64.0,
                FontWeight::NORMAL,
                &paint,
            );
            let blobs = font_blobs(&ctx, &[&runs]);
            corpus.scene_with_blobs(
                name,
                256,
                96,
                white,
                |l| {
                    for run in &runs {
                        l.glyphs(run.clone());
                    }
                },
                blobs,
            );
        }
    }

    // ---- Bitmap colour fonts -----------------------------------------------

    let bitmap_white = srgb(1.0, 1.0, 1.0);
    for (file, name) in [
        ("NotoColorEmojiSubset.ttf", "text-cbdt"),
        ("CherenkovSbixTest.ttf", "text-sbix"),
    ] {
        let runs = ctx.shape(
            file,
            corpus::BITMAP_EMOJI,
            48.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let blobs = font_blobs(&ctx, &[&runs]);
        corpus.scene_with_blobs(
            name,
            320,
            160,
            bitmap_white,
            |l| {
                for run in &runs {
                    l.glyphs(run.clone());
                }
            },
            blobs,
        );
    }

    for (file, name) in [
        ("NotoColorEmojiSubset.ttf", "text-cbdt-sizes"),
        ("CherenkovSbixTest.ttf", "text-sbix-sizes"),
    ] {
        let specs = [
            (14.0, corpus::BITMAP_EMOJI),
            (24.0, corpus::BITMAP_EMOJI),
            (40.0, corpus::BITMAP_EMOJI),
            (72.0, "\u{2615}\u{26A0}\u{26A1}"),
            (104.0, "\u{1F600}\u{2764}"),
        ];
        let rows: Vec<Vec<GlyphRun>> = specs
            .iter()
            .map(|(size, text)| ctx.shape(file, text, *size, FontWeight::NORMAL, &solid(dark)))
            .collect();
        let blobs = font_blobs(&ctx, &rows.iter().map(Vec::as_slice).collect::<Vec<_>>());
        corpus.scene_with_blobs(
            name,
            480,
            360,
            bitmap_white,
            |l| {
                for (runs, y) in rows.iter().zip([0.0, 24.0, 58.0, 112.0, 204.0]) {
                    l.layer(|row| {
                        row.transform(Affine::translate((0.0, y)));
                        for run in runs {
                            row.glyphs(run.clone());
                        }
                    });
                }
            },
            blobs,
        );
    }

    for (file, name) in [
        ("NotoColorEmojiSubset.ttf", "text-cbdt-transform"),
        ("CherenkovSbixTest.ttf", "text-sbix-transform"),
    ] {
        let first = ctx.shape(
            file,
            corpus::BITMAP_EMOJI,
            40.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let second = ctx.shape(
            file,
            "\u{1F600}\u{2764}",
            72.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let blobs = font_blobs(&ctx, &[&first, &second]);
        corpus.scene_with_blobs(
            name,
            400,
            280,
            bitmap_white,
            |l| {
                l.layer(|rotated| {
                    rotated.transform(
                        Affine::translate((40.0, 30.0))
                            * rotate(15_f64.to_radians())
                            * Affine::scale_non_uniform(1.2, 0.8),
                    );
                    for run in &first {
                        rotated.glyphs(run.clone());
                    }
                });
                l.layer(|scaled| {
                    scaled.transform(Affine::translate((60.0, 200.0)) * Affine::scale(0.5));
                    for run in &second {
                        scaled.glyphs(run.clone());
                    }
                });
            },
            blobs,
        );
    }

    {
        let sbix_transforms = [
            rotate(20_f64.to_radians()),
            Affine::skew(0.35, 0.0),
            Affine::skew(0.0, -0.25),
            Affine::scale_non_uniform(1.4, 0.8),
            rotate((-15_f64).to_radians())
                * Affine::skew(0.2, -0.12)
                * Affine::scale_non_uniform(0.8, 1.25),
        ];
        let cbdt_transforms = [
            rotate((-20_f64).to_radians()),
            Affine::skew(-0.25, 0.15),
            Affine::scale_non_uniform(0.7, 1.3),
            rotate(45_f64.to_radians()),
            rotate(12_f64.to_radians())
                * Affine::skew(0.18, 0.08)
                * Affine::scale_non_uniform(1.15, 0.9),
        ];
        let mut sbix = ctx.shape(
            "CherenkovSbixTest.ttf",
            corpus::BITMAP_EMOJI,
            24.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let mut cbdt = ctx.shape(
            "NotoColorEmojiSubset.ttf",
            corpus::BITMAP_EMOJI,
            40.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let mut index = 0;
        for run in &mut sbix {
            for glyph in &mut run.glyphs {
                #[expect(clippy::cast_precision_loss, reason = "glyph count is small")]
                let x = 70.113 + index as f32 * 69.0;
                glyph.x = x;
                glyph.transform = Some(sbix_transforms[index]);
                index += 1;
            }
        }
        assert_eq!(index, sbix_transforms.len());
        index = 0;
        for run in &mut cbdt {
            for glyph in &mut run.glyphs {
                #[expect(clippy::cast_precision_loss, reason = "glyph count is small")]
                let x = 46.214 + index as f32 * 76.0;
                glyph.x = x;
                glyph.transform = Some(cbdt_transforms[index]);
                index += 1;
            }
        }
        assert_eq!(index, cbdt_transforms.len());
        let blobs = font_blobs(&ctx, &[&sbix, &cbdt]);
        corpus.scene_with_blobs(
            "text-bitmap-glyph-transform",
            440,
            195,
            bitmap_white,
            |l| {
                l.layer(|row| {
                    row.transform(Affine::translate((0.0, 15.439)));
                    for run in &sbix {
                        row.glyphs(run.clone());
                    }
                });
                l.layer(|row| {
                    row.transform(Affine::translate((0.0, 88.392)));
                    for run in &cbdt {
                        row.glyphs(run.clone());
                    }
                });
            },
            blobs,
        );
    }

    for (suffix, paint) in [
        ("", solid(dark)),
        ("-p3", solid(p3(0.0, 1.0, 0.0))),
        ("-hdr", solid(hdr(16.0, 2.0, 0.5))),
    ] {
        let coffee = ctx.shape("NotoSans.ttf", "Coffee ", 32.0, FontWeight::NORMAL, &paint);
        let cbdt = ctx.shape(
            "NotoColorEmojiSubset.ttf",
            corpus::BITMAP_EMOJI,
            32.0,
            FontWeight::NORMAL,
            &paint,
        );
        let warning = ctx.shape("NotoSans.ttf", "Warning ", 32.0, FontWeight::NORMAL, &paint);
        let sbix = ctx.shape(
            "CherenkovSbixTest.ttf",
            corpus::BITMAP_EMOJI,
            32.0,
            FontWeight::NORMAL,
            &paint,
        );
        let overlap = ctx.shape(
            "NotoColorEmojiSubset.ttf",
            corpus::BITMAP_EMOJI,
            40.0,
            FontWeight::NORMAL,
            &paint,
        );
        let latin = ctx.shape(
            "NotoSans.ttf",
            "overlapping text",
            24.0,
            FontWeight::NORMAL,
            &paint,
        );
        let blobs = font_blobs(&ctx, &[&coffee, &cbdt, &warning, &sbix, &overlap, &latin]);
        corpus.scene_with_blobs(
            format!("text-bitmap-mixed{suffix}"),
            480,
            240,
            bitmap_white,
            |l| {
                l.layer(|line| {
                    line.transform(Affine::translate((0.0, 0.0)));
                    for run in &coffee {
                        line.glyphs(run.clone());
                    }
                });
                l.layer(|line| {
                    line.transform(Affine::translate((140.0, 0.0)));
                    for run in &cbdt {
                        line.glyphs(run.clone());
                    }
                });
                l.layer(|line| {
                    line.transform(Affine::translate((0.0, 62.0)));
                    for run in &warning {
                        line.glyphs(run.clone());
                    }
                });
                l.layer(|line| {
                    line.transform(Affine::translate((160.0, 62.0)));
                    for run in &sbix {
                        line.glyphs(run.clone());
                    }
                });
                l.layer(|line| {
                    line.transform(Affine::translate((0.0, 122.0)));
                    for run in &overlap {
                        line.glyphs(run.clone());
                    }
                });
                l.layer(|line| {
                    line.transform(Affine::translate((108.0, 136.0)));
                    for run in &latin {
                        line.glyphs(run.clone());
                    }
                });
            },
            blobs,
        );
    }

    // ---- Motion and scrolling ----------------------------------------------
    //
    // Scenes exercising `Layer::scroll_offset` and `Layer::motion`. The
    // oracle renders the settled state; the cherenkov adapters commit the
    // `from` state then the animation/decay, and `render --readback`
    // comparisons run until `Next::Idle`.

    // A card that springs in from above the viewport (Spring 0.5/1.0).
    corpus.scene("anim-spring-card", 256, 256, srgb(0.94, 0.95, 0.98), |l| {
        l.layer(|card| {
            card.transform(Affine::translate((0.0, 0.0)));
            card.motion(Motion::Transform {
                from: Affine::translate((0.0, -200.0)),
                animation: MotionAnimation::Spring {
                    response: 0.5,
                    damping: 1.0,
                },
            });
            let rect = RoundedRect::from_rect(
                Rect::new(32.0, 96.0, 224.0, 208.0),
                RoundedRectRadii::new(16.0, 16.0, 16.0, 16.0),
            );
            card.shadow(
                Shape::RoundedRect(rect),
                8.0,
                [0.0, 6.0],
                srgba(0.0, 0.0, 0.0, 0.25),
            );
            card.fill(Shape::RoundedRect(rect), solid(white));
            card.fill(
                Shape::RoundedRect(RoundedRect::from_rect(
                    Rect::new(48.0, 120.0, 208.0, 140.0),
                    RoundedRectRadii::new(6.0, 6.0, 6.0, 6.0),
                )),
                solid(srgb(0.35, 0.45, 0.85)),
            );
            card.fill(
                Shape::RoundedRect(RoundedRect::from_rect(
                    Rect::new(48.0, 152.0, 168.0, 164.0),
                    RoundedRectRadii::new(4.0, 4.0, 4.0, 4.0),
                )),
                solid(srgb(0.8, 0.82, 0.88)),
            );
            card.fill(
                Shape::RoundedRect(RoundedRect::from_rect(
                    Rect::new(48.0, 174.0, 190.0, 186.0),
                    RoundedRectRadii::new(4.0, 4.0, 4.0, 4.0),
                )),
                solid(srgb(0.8, 0.82, 0.88)),
            );
        });
    });

    // A panel sliding in on a 400 ms ease-in-out curve.
    corpus.scene("anim-curve-slide", 256, 256, srgb(0.92, 0.94, 0.96), |l| {
        l.layer(|panel| {
            panel.transform(Affine::translate((0.0, 0.0)));
            panel.motion(Motion::Transform {
                from: Affine::translate((-180.0, 0.0)),
                animation: MotionAnimation::Curve {
                    duration_ms: 400,
                    x1: 0.42,
                    y1: 0.0,
                    x2: 0.58,
                    y2: 1.0,
                },
            });
            panel.fill(
                Shape::RoundedRect(RoundedRect::from_rect(
                    Rect::new(24.0, 64.0, 232.0, 200.0),
                    RoundedRectRadii::new(12.0, 12.0, 12.0, 12.0),
                )),
                solid(srgb(0.22, 0.5, 0.6)),
            );
            for i in 0u8..4 {
                let y = 88.0 + f64::from(i) * 28.0;
                panel.fill(
                    Shape::RoundedRect(RoundedRect::from_rect(
                        Rect::new(44.0, y, 44.0 + 150.0 - 22.0 * f64::from(i), y + 12.0),
                        RoundedRectRadii::new(4.0, 4.0, 4.0, 4.0),
                    )),
                    solid(srgba(1.0, 1.0, 1.0, 0.75)),
                );
            }
        });
    });

    // A card body whose fill springs coral → teal (Spring 0.5/1.0): the
    // animated operand is the fill's paint inside the recorded content,
    // not a layer property.
    corpus.scene("anim-paint-spring", 256, 256, srgb(0.94, 0.95, 0.98), |l| {
        l.layer(|card| {
            let rect = RoundedRect::from_rect(
                Rect::new(32.0, 96.0, 224.0, 208.0),
                RoundedRectRadii::new(16.0, 16.0, 16.0, 16.0),
            );
            card.shadow(
                Shape::RoundedRect(rect),
                8.0,
                [0.0, 6.0],
                srgba(0.0, 0.0, 0.0, 0.25),
            );
            let body = card.item_count();
            card.fill(Shape::RoundedRect(rect), solid(srgb(0.35, 0.55, 0.62)));
            card.motion(Motion::Paint {
                item: body,
                from: Box::new(solid(srgb(0.92, 0.35, 0.30))),
                animation: MotionAnimation::Spring {
                    response: 0.5,
                    damping: 1.0,
                },
            });
            card.fill(
                Shape::RoundedRect(RoundedRect::from_rect(
                    Rect::new(48.0, 120.0, 208.0, 140.0),
                    RoundedRectRadii::new(6.0, 6.0, 6.0, 6.0),
                )),
                solid(srgba(1.0, 1.0, 1.0, 0.85)),
            );
            card.fill(
                Shape::RoundedRect(RoundedRect::from_rect(
                    Rect::new(48.0, 152.0, 168.0, 164.0),
                    RoundedRectRadii::new(4.0, 4.0, 4.0, 4.0),
                )),
                solid(srgba(1.0, 1.0, 1.0, 0.55)),
            );
        });
    });

    // A panel gradient whose geometry and stop colours ease on a 400 ms
    // ease-in-out curve: gradient lanes animate inside the recorded
    // content.
    corpus.scene("anim-paint-curve", 256, 256, srgb(0.92, 0.94, 0.96), |l| {
        l.layer(|panel| {
            let shape = Shape::RoundedRect(RoundedRect::from_rect(
                Rect::new(24.0, 64.0, 232.0, 200.0),
                RoundedRectRadii::new(12.0, 12.0, 12.0, 12.0),
            ));
            let item = panel.item_count();
            panel.fill(
                shape,
                Paint::Linear(LinearGradient {
                    start: Point::new(24.0, 64.0),
                    end: Point::new(232.0, 200.0),
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: srgb(0.22, 0.5, 0.6),
                        },
                        GradientStop {
                            offset: 1.0,
                            color: srgb(0.8, 0.86, 0.9),
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation: ColorSpace::Srgb,
                }),
            );
            panel.motion(Motion::Paint {
                item,
                from: Box::new(Paint::Linear(LinearGradient {
                    start: Point::new(24.0, 24.0),
                    end: Point::new(24.0, 200.0),
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: srgb(0.9, 0.4, 0.4),
                        },
                        GradientStop {
                            offset: 1.0,
                            color: srgb(0.55, 0.35, 0.7),
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation: ColorSpace::Srgb,
                })),
                animation: MotionAnimation::Curve {
                    duration_ms: 400,
                    x1: 0.42,
                    y1: 0.0,
                    x2: 0.58,
                    y2: 1.0,
                },
            });
            for i in 0u8..4 {
                let y = 88.0 + f64::from(i) * 28.0;
                panel.fill(
                    Shape::RoundedRect(RoundedRect::from_rect(
                        Rect::new(44.0, y, 44.0 + 150.0 - 22.0 * f64::from(i), y + 12.0),
                        RoundedRectRadii::new(4.0, 4.0, 4.0, 4.0),
                    )),
                    solid(srgba(1.0, 1.0, 1.0, 0.75)),
                );
            }
        });
    });

    // A clipped list scrolled to a static offset: rows 3.. are visible.
    corpus.scene("scroll-static", 256, 192, srgb(0.97, 0.97, 0.98), |l| {
        l.layer(|list| {
            list.clip(Shape::Rect(Rect::new(8.0, 16.0, 248.0, 176.0)));
            list.scroll_offset(Vec2::new(0.0, 96.0));
            for i in 0u8..12 {
                let y = f64::from(i) * 48.0;
                list.fill(
                    Shape::RoundedRect(RoundedRect::from_rect(
                        Rect::new(16.0, y, 240.0, y + 40.0),
                        RoundedRectRadii::new(8.0, 8.0, 8.0, 8.0),
                    )),
                    solid(srgb(
                        0.3 + 0.05 * f32::from(i % 4),
                        0.5,
                        0.85 - 0.04 * f32::from(i % 4),
                    )),
                );
            }
        });
    });

    // A 60-row list in a clip: a fling decaying from below the rest
    // position. from = rest - v/k with v = (0, -1800), k = 4.
    corpus.scene("scroll-decay", 256, 320, srgb(0.97, 0.97, 0.98), |l| {
        l.layer(|list| {
            list.clip(Shape::Rect(Rect::new(8.0, 16.0, 248.0, 304.0)));
            list.scroll_offset(Vec2::new(0.0, 600.0));
            list.motion(Motion::Scroll {
                from: Vec2::new(0.0, 1050.0),
                velocity: Vec2::new(0.0, -1800.0),
                deceleration: 4.0,
                bounds: None,
            });
            for i in 0u8..60 {
                let y = f64::from(i) * 48.0;
                list.fill(
                    Shape::RoundedRect(RoundedRect::from_rect(
                        Rect::new(16.0, y, 240.0, y + 40.0),
                        RoundedRectRadii::new(8.0, 8.0, 8.0, 8.0),
                    )),
                    Paint::Linear(LinearGradient {
                        start: Point::new(16.0, y),
                        end: Point::new(240.0, y),
                        extend: Extend::Pad,
                        interpolation: ColorSpace::Srgb,
                        stops: vec![
                            GradientStop {
                                offset: 0.0,
                                color: srgb(
                                    0.25 + 0.01 * f32::from(i % 20),
                                    0.5,
                                    0.8 - 0.02 * f32::from(i % 10),
                                ),
                            },
                            GradientStop {
                                offset: 1.0,
                                color: srgb(0.6, 0.7, 0.9),
                            },
                        ],
                    }),
                );
            }
        });
    });

    // The same list, flung hard enough to overshoot its bounds and
    // rubber-band back; rest = the bound = the static scroll_offset.
    corpus.scene(
        "scroll-rubber-band",
        256,
        320,
        srgb(0.97, 0.97, 0.98),
        |l| {
            l.layer(|list| {
                list.clip(Shape::Rect(Rect::new(8.0, 16.0, 248.0, 304.0)));
                list.scroll_offset(Vec2::new(0.0, 300.0));
                list.motion(Motion::Scroll {
                    from: Vec2::new(0.0, 100.0),
                    velocity: Vec2::new(0.0, 2000.0),
                    deceleration: 4.0,
                    bounds: Some(Rect::new(0.0, 0.0, 0.0, 300.0)),
                });
                for i in 0u8..60 {
                    let y = f64::from(i) * 48.0;
                    list.fill(
                        Shape::RoundedRect(RoundedRect::from_rect(
                            Rect::new(16.0, y, 240.0, y + 40.0),
                            RoundedRectRadii::new(8.0, 8.0, 8.0, 8.0),
                        )),
                        Paint::Linear(LinearGradient {
                            start: Point::new(16.0, y),
                            end: Point::new(240.0, y),
                            extend: Extend::Pad,
                            interpolation: ColorSpace::Srgb,
                            stops: vec![
                                GradientStop {
                                    offset: 0.0,
                                    color: srgb(0.75, 0.55, 0.85),
                                },
                                GradientStop {
                                    offset: 1.0,
                                    color: srgb(0.5, 0.35 + 0.01 * f32::from(i % 20), 0.75),
                                },
                            ],
                        }),
                    );
                }
            });
        },
    );

    // ---- Performance set ---------------------------------------------------
    //
    // Full-resolution scenes (the Pixel 9 Pro viewport, 1024x2216) modelling
    // real app content — a performance set for `measure`, separate from the
    // per-primitive correctness sweeps above. Deterministic throughout; the
    // only pseudo-randomness is the seeded xorshift in the map scene.

    let (pw, ph) = (1024u32, 2216u32);
    let mut perf = Corpus::new();

    // Scrolling UI list: 30 rows, each a shadowed rounded card with an
    // avatar, two text runs and a chevron icon path.
    {
        let title = ctx.shape(
            "NotoSans.ttf",
            "Message from Ada",
            30.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let sub = ctx.shape(
            "NotoSans.ttf",
            "See you at the renderer sync-up",
            22.0,
            FontWeight::NORMAL,
            &solid(srgb(0.4, 0.42, 0.45)),
        );
        let blobs = font_blobs(&ctx, &[&title, &sub]);
        perf.scene_with_blobs(
            "ui-list",
            pw,
            ph,
            srgb(0.96, 0.96, 0.97),
            |l| {
                let pitch = 72.0;
                for i in 0u8..30 {
                    let y = 24.0 + f64::from(i) * pitch;
                    let card = RoundedRect::from_rect(
                        Rect::new(24.0, y, 1000.0, y + 64.0),
                        RoundedRectRadii::new(14.0, 14.0, 14.0, 14.0),
                    );
                    l.shadow(
                        Shape::RoundedRect(card),
                        4.0,
                        [0.0, 3.0],
                        srgba(0.0, 0.0, 0.0, 0.22),
                    );
                    l.fill(Shape::RoundedRect(card), solid(white));
                    l.fill(
                        Shape::circle(60.0, y + 32.0, 20.0),
                        solid(srgb(0.3 + 0.02 * f32::from(i), 0.4, 0.8)),
                    );
                    for run in &title {
                        l.glyphs(offset_run(run, 96.0, y + 28.0));
                    }
                    for run in &sub {
                        l.glyphs(offset_run(run, 96.0, y + 54.0));
                    }
                    l.fill(
                        Shape::Path {
                            path: chevron(960.0, y + 32.0),
                        },
                        solid(srgb(0.6, 0.6, 0.62)),
                    );
                }
            },
            blobs,
        );
    }

    // Dense multi-script text page: Latin, CJK, Arabic, Devanagari and emoji
    // lines cycling down the full page height.
    {
        let scripts = [
            ("NotoSans.ttf", corpus::LATIN, 34.0_f32),
            ("NotoSansSC.ttf", corpus::CJK, 34.0),
            ("NotoSansArabic.ttf", corpus::ARABIC, 36.0),
            ("NotoSansDevanagari.ttf", corpus::DEVANAGARI, 34.0),
            ("NotoEmoji.ttf", corpus::EMOJI, 34.0),
        ];
        let shaped: Vec<Vec<GlyphRun>> = scripts
            .iter()
            .map(|(file, text, size)| {
                ctx.shape(file, text, *size, FontWeight::NORMAL, &solid(dark))
            })
            .collect();
        let blobs = font_blobs(&ctx, &shaped.iter().map(Vec::as_slice).collect::<Vec<_>>());
        perf.scene_with_blobs(
            "text-page",
            pw,
            ph,
            white,
            |l| {
                text_page_body(l, &shaped);
            },
            blobs,
        );
    }

    // The text page under a smooth pan: a fractional offset per frame for
    // 4 s, then at rest at identity (= the text-page scene itself).
    // Exercises the glyph cache's behaviour when the translation's
    // fraction changes every frame.
    {
        let scripts = [
            ("NotoSans.ttf", corpus::LATIN, 34.0_f32),
            ("NotoSansSC.ttf", corpus::CJK, 34.0),
            ("NotoSansArabic.ttf", corpus::ARABIC, 36.0),
            ("NotoSansDevanagari.ttf", corpus::DEVANAGARI, 34.0),
            ("NotoEmoji.ttf", corpus::EMOJI, 34.0),
        ];
        let shaped: Vec<Vec<GlyphRun>> = scripts
            .iter()
            .map(|(file, text, size)| {
                ctx.shape(file, text, *size, FontWeight::NORMAL, &solid(dark))
            })
            .collect();
        let blobs = font_blobs(&ctx, &shaped.iter().map(Vec::as_slice).collect::<Vec<_>>());
        perf.scene_with_blobs(
            "text-pan",
            pw,
            ph,
            white,
            |l| {
                l.layer(|pan| {
                    pan.transform(Affine::IDENTITY);
                    pan.motion(Motion::Transform {
                        from: Affine::translate((-281.7, -209.3)),
                        animation: MotionAnimation::Curve {
                            duration_ms: 4000,
                            x1: 0.25,
                            y1: 0.25,
                            x2: 0.75,
                            y2: 0.75,
                        },
                    });
                    text_page_body(pan, &shaped);
                });
            },
            blobs,
        );
    }

    // Map-like page: ~2,000 stroked and filled paths — short segments,
    // closed polygons and curved outlines distributed over the viewport.

    {
        perf.scene("map", pw, ph, srgb(0.93, 0.95, 0.90), |l| {
            map_body(l, f64::from(pw), f64::from(ph));
        });
    }

    // The map under a smooth pan: a fractional offset per frame for 4 s,
    // then at rest at identity (= the map scene itself). Exercises the
    // path cache's behaviour when the translation's fraction changes
    // every frame.
    {
        perf.scene("map-pan", pw, ph, srgb(0.93, 0.95, 0.90), |l| {
            l.layer(|pan| {
                pan.transform(Affine::IDENTITY);
                pan.motion(Motion::Transform {
                    from: Affine::translate((-281.7, -209.3)),
                    animation: MotionAnimation::Curve {
                        duration_ms: 4000,
                        x1: 0.25,
                        y1: 0.25,
                        x2: 0.75,
                        y2: 0.75,
                    },
                });
                map_body(pan, f64::from(pw), f64::from(ph));
            });
        });
    }

    // Chart page: a 2,000-point line chart, a bar chart, axes and labels.
    {
        let label = ctx.shape(
            "NotoSans.ttf",
            "Frame time (ms)",
            28.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let tick = ctx.shape(
            "NotoSans.ttf",
            "250",
            24.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let blobs = font_blobs(&ctx, &[&label, &tick]);
        perf.scene_with_blobs(
            "chart",
            pw,
            ph,
            white,
            |l| {
                // Axes + grid.
                let axis = StrokeStyle {
                    width: 2.0,
                    ..StrokeStyle::default()
                };
                l.stroke(
                    Shape::Line(Line::new((80.0, 100.0), (80.0, 1400.0))),
                    axis.clone(),
                    solid(dark),
                );
                l.stroke(
                    Shape::Line(Line::new((80.0, 1400.0), (1000.0, 1400.0))),
                    axis,
                    solid(dark),
                );
                let grid = StrokeStyle {
                    width: 0.5,
                    ..StrokeStyle::default()
                };
                for g in 1u8..7 {
                    let gy = 1400.0 - f64::from(g) * 200.0;
                    l.stroke(
                        Shape::Line(Line::new((80.0, gy), (1000.0, gy))),
                        grid.clone(),
                        solid(srgb(0.85, 0.85, 0.85)),
                    );
                    for run in &tick {
                        l.glyphs(offset_run(run, 28.0, gy + 8.0));
                    }
                }
                // Line chart: 2,000 points of a deterministic waveform.
                let mut line = BezPath::new();
                for i in 0u16..2000 {
                    let x = 80.0 + f64::from(i) * (920.0 / 1999.0);
                    let t = f64::from(i) * 0.01;
                    let y = 750.0
                        - 320.0 * (0.5 + 0.3 * libm::sin(t) + 0.2 * libm::cos(3.1 * t))
                        - 40.0 * libm::sin(t * 17.3);
                    if i == 0 {
                        line.move_to((x, y));
                    } else {
                        line.line_to((x, y));
                    }
                }
                l.stroke(
                    Shape::Path { path: line },
                    StrokeStyle {
                        width: 2.5,
                        ..StrokeStyle::default()
                    },
                    solid(srgb(0.15, 0.45, 0.85)),
                );
                for run in &label {
                    l.glyphs(offset_run(run, 80.0, 60.0));
                }
                // Bar chart: 44 bars.
                let bw = (920.0 - 40.0 * 8.0) / 44.0;
                for i in 0u8..44 {
                    let h = 120.0 + 380.0 * libm::sin(f64::from(i) * 0.37).mul_add(0.5, 0.5);
                    let x = 90.0 + f64::from(i) * (bw + 8.0);
                    l.fill(
                        Shape::Rect(Rect::new(x, 2050.0 - h, x + bw, 2050.0)),
                        solid(srgb(0.85, 0.45, 0.25)),
                    );
                }
            },
            blobs,
        );
    }

    // Effects page: 20 shadowed cards under a group-opacity layer.
    {
        perf.scene("effects", pw, ph, srgb(0.98, 0.97, 0.96), |l| {
            l.layer(|group| {
                group.opacity(0.82);
                for i in 0u8..20 {
                    let (col, row) = (i % 4, i / 4);
                    let (x, y) = (32.0 + f64::from(col) * 250.0, 48.0 + f64::from(row) * 420.0);
                    let card = RoundedRect::from_rect(
                        Rect::new(x, y, x + 226.0, y + 380.0),
                        RoundedRectRadii::new(20.0, 20.0, 20.0, 20.0),
                    );
                    group.shadow(
                        Shape::RoundedRect(card),
                        12.0,
                        [0.0, 10.0],
                        srgba(0.15, 0.1, 0.3, 0.35),
                    );
                    group.fill(
                        Shape::RoundedRect(card),
                        solid(srgb(
                            0.55 + 0.05 * f32::from(col),
                            0.35 + 0.03 * f32::from(row),
                            0.75,
                        )),
                    );
                    group.fill(
                        Shape::RoundedRect(RoundedRect::from_rect(
                            Rect::new(x + 20.0, y + 24.0, x + 206.0, y + 120.0),
                            RoundedRectRadii::new(10.0, 10.0, 10.0, 10.0),
                        )),
                        solid(srgba(1.0, 1.0, 1.0, 0.6)),
                    );
                }
            });
        });
    }

    // A heavy scrolling list: 120 shadowed card rows inside a clipped,
    // scroll-decaying layer — the frame-time scene for `measure` while a
    // scroll is animating.
    {
        let title = ctx.shape(
            "NotoSans.ttf",
            "Inbox message subject",
            28.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        let sub = ctx.shape(
            "NotoSans.ttf",
            "A short preview line of the message body",
            20.0,
            FontWeight::NORMAL,
            &solid(srgb(0.4, 0.42, 0.45)),
        );
        let blobs = font_blobs(&ctx, &[&title, &sub]);
        perf.scene_with_blobs(
            "scroll-list",
            pw,
            ph,
            srgb(0.96, 0.96, 0.97),
            |l| {
                l.layer(|list| {
                    list.clip(Shape::Rect(Rect::new(
                        0.0,
                        0.0,
                        f64::from(pw),
                        f64::from(ph),
                    )));
                    list.scroll_offset(Vec2::new(0.0, 1200.0));
                    list.motion(Motion::Scroll {
                        from: Vec2::new(0.0, 1700.0),
                        velocity: Vec2::new(0.0, -2000.0),
                        deceleration: 4.0,
                        bounds: Some(Rect::new(0.0, 0.0, 0.0, 120.0 * 96.0 - f64::from(ph))),
                    });
                    let pitch = 96.0;
                    for i in 0u16..120 {
                        let y = 24.0 + f64::from(i) * pitch;
                        let card = RoundedRect::from_rect(
                            Rect::new(24.0, y, 1000.0, y + 88.0),
                            RoundedRectRadii::new(14.0, 14.0, 14.0, 14.0),
                        );
                        list.shadow(
                            Shape::RoundedRect(card),
                            4.0,
                            [0.0, 3.0],
                            srgba(0.0, 0.0, 0.0, 0.22),
                        );
                        list.fill(Shape::RoundedRect(card), solid(white));
                        list.fill(
                            Shape::circle(64.0, y + 44.0, 24.0),
                            solid(srgb(0.3 + 0.02 * f32::from(i % 16), 0.4, 0.8)),
                        );
                        for run in &title {
                            list.glyphs(offset_run(run, 112.0, y + 40.0));
                        }
                        for run in &sub {
                            list.glyphs(offset_run(run, 112.0, y + 72.0));
                        }
                    }
                });
            },
            blobs,
        );
    }

    // A text-heavy dashboard whose per-frame values ride engine slot
    // updates: one glyph run (a two-digit counter) and one bar of the
    // chart change every frame; everything else is static.
    {
        let title = ctx.shape(
            "NotoSans.ttf",
            "Dashboard",
            40.0,
            FontWeight::BOLD,
            &solid(dark),
        );
        let body = ctx.shape(
            "NotoSans.ttf",
            "Revenue, signups and latency at a glance for the last quarter.",
            22.0,
            FontWeight::NORMAL,
            &solid(srgb(0.4, 0.42, 0.45)),
        );
        let label = ctx.shape(
            "NotoSans.ttf",
            "Users",
            20.0,
            FontWeight::NORMAL,
            &solid(dark),
        );
        // Same glyph count every frame (two digits) so the run stays a
        // value slot update.
        let digits: Vec<Vec<GlyphRun>> = (0u8..60)
            .map(|n| {
                ctx.shape(
                    "NotoSans.ttf",
                    &format!("{n:02}"),
                    48.0,
                    FontWeight::BOLD,
                    &solid(srgb(0.1, 0.4, 0.9)),
                )
            })
            .collect();
        let blobs = font_blobs(&ctx, &[&title, &body, &label, &digits[0]]);
        perf.scene_with_blobs(
            "live-dashboard",
            pw,
            ph,
            srgb(0.96, 0.96, 0.97),
            |l| {
                for run in &title {
                    l.glyphs(offset_run(run, 40.0, 40.0));
                }
                for (run, i) in body.iter().zip(0u16..) {
                    l.glyphs(offset_run(run, 40.0, 140.0 + 34.0 * f64::from(i)));
                }
                // Card grid: 4 columns x 3 rows of shadowed cards.
                for row in 0u8..3 {
                    for col in 0u8..4 {
                        let x = 40.0 + f64::from(col) * 246.0;
                        let y = 280.0 + f64::from(row) * 220.0;
                        let card = RoundedRect::from_rect(
                            Rect::new(x, y, x + 226.0, y + 200.0),
                            RoundedRectRadii::new(14.0, 14.0, 14.0, 14.0),
                        );
                        l.shadow(
                            Shape::RoundedRect(card),
                            4.0,
                            [0.0, 3.0],
                            srgba(0.0, 0.0, 0.0, 0.22),
                        );
                        l.fill(Shape::RoundedRect(card), solid(white));
                        l.stroke(
                            Shape::RoundedRect(card),
                            StrokeStyle {
                                width: 1.0,
                                ..StrokeStyle::default()
                            },
                            solid(srgba(0.0, 0.0, 0.0, 0.12)),
                        );
                        for run in &label {
                            l.glyphs(offset_run(run, x + 20.0, y + 24.0));
                        }
                    }
                }
                // Bar chart: 24 bars inside a framed plot.
                let chart_top = 1000.0;
                let chart_h = 300.0;
                l.stroke(
                    Shape::Rect(Rect::new(40.0, chart_top, 984.0, chart_top + chart_h)),
                    StrokeStyle {
                        width: 1.0,
                        ..StrokeStyle::default()
                    },
                    solid(srgba(0.0, 0.0, 0.0, 0.25)),
                );
                let bar_paint = solid(srgb(0.2, 0.5, 0.9));
                for i in 0u16..24 {
                    let x = 56.0 + f64::from(i) * 39.0;
                    let h = 40.0 + f64::from((i * 37) % 200);
                    l.fill(
                        Shape::Rect(Rect::new(
                            x,
                            chart_top + chart_h - h,
                            x + 28.0,
                            chart_top + chart_h,
                        )),
                        bar_paint.clone(),
                    );
                }
                // Live counter, in the same content layer: the digits of
                // "active users" ticking 00..59.
                let counter = l.item_count();
                for run in &digits[0] {
                    l.glyphs(offset_run(run, 60.0, 240.0));
                }
                l.live(Live {
                    item: counter,
                    frames: digits
                        .iter()
                        .map(|runs| Draw::Glyphs(offset_run(&runs[0], 60.0, 240.0)))
                        .collect(),
                });
                // Live bar: the last bar pulses every frame.
                let last = l.item_count();
                let bx = 56.0 + 23.0 * 39.0;
                let live_bar = |h: f64| {
                    Shape::Rect(Rect::new(
                        bx + 39.0,
                        chart_top + chart_h - h,
                        bx + 39.0 + 28.0,
                        chart_top + chart_h,
                    ))
                };
                // `frames[0]` equals the base item (the first frame's h).
                l.fill(live_bar(40.0), bar_paint.clone());
                l.live(Live {
                    item: last,
                    frames: (0u8..60)
                        .map(|n| {
                            let h = 40.0 + f64::from(n) * 3.0;
                            Draw::Fill {
                                shape: live_bar(h),
                                rule: FillRule::NonZero,
                                paint: bar_paint.clone(),
                            }
                        })
                        .collect(),
                });
            },
            blobs,
        );
    }

    // ---- Backdrop groups ---------------------------------------------------

    corpus.scene_setup("backdrop-plain", 256, 256, white, |b| {
        b.backdrop_group(1, Vec::new());
        let l = &mut b.root();
        backdrop_background(l);
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(56.0, 56.0, 200.0, 200.0, 20.0));
            m.clip(clip.clone());
            m.backdrop(1);
            m.fill(
                Shape::rect(58.0, 58.0, 140.0, 140.0),
                solid(srgba(1.0, 1.0, 1.0, 0.4)),
            );
            m.stroke(
                clip,
                StrokeStyle {
                    width: 2.0,
                    ..StrokeStyle::default()
                },
                solid(srgb(1.0, 1.0, 1.0)),
            );
        });
    });

    corpus.scene_setup("backdrop-blur", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 6.0 }]);
        let l = &mut b.root();
        backdrop_background(l);
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(24.0, 24.0, 140.0, 92.0, 14.0));
            m.clip(clip);
            m.backdrop(1);
            m.fill(
                Shape::rect(26.0, 26.0, 112.0, 64.0),
                solid(srgba(1.0, 1.0, 1.0, 0.25)),
            );
        });
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(140.0, 176.0, 232.0, 216.0, 20.0));
            m.clip(clip);
            m.backdrop(1);
            m.fill(
                Shape::rect(142.0, 178.0, 88.0, 36.0),
                solid(srgba(1.0, 1.0, 1.0, 0.25)),
            );
        });
    });

    corpus.scene_setup("backdrop-nested", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 8.0 }]);
        b.backdrop_group(2, vec![BackdropFilter::GaussianBlur { sigma: 3.0 }]);
        let l = &mut b.root();
        backdrop_background(l);
        l.layer(|a| {
            let clip = Shape::RoundedRect(RoundedRect::new(24.0, 24.0, 232.0, 232.0, 24.0));
            a.clip(clip);
            a.backdrop(1);
            a.fill(
                Shape::rect(26.0, 26.0, 204.0, 204.0),
                solid(srgba(1.0, 1.0, 1.0, 0.15)),
            );
            // A member of group B inside A's member: B's capture includes
            // A's sample.
            a.layer(|m| {
                let clip = Shape::RoundedRect(RoundedRect::new(44.0, 44.0, 140.0, 140.0, 16.0));
                m.clip(clip);
                m.backdrop(2);
                m.fill(
                    Shape::rect(46.0, 46.0, 92.0, 92.0),
                    solid(srgba(1.0, 1.0, 1.0, 0.3)),
                );
            });
            // A second B member, sibling of the first: it shares B's
            // capture (taken at the first member's paint point).
            a.layer(|m| {
                let clip = Shape::RoundedRect(RoundedRect::new(120.0, 120.0, 212.0, 212.0, 16.0));
                m.clip(clip);
                m.backdrop(2);
                m.fill(
                    Shape::rect(122.0, 122.0, 88.0, 88.0),
                    solid(srgba(0.0, 0.0, 0.0, 0.2)),
                );
            });
        });
    });

    corpus.scene_setup("backdrop-transform", 256, 256, white, |b| {
        // A saturation-boost colour matrix on premultiplied colour
        // (luminance-preserving, s = 1.6).
        let s = 1.6;
        let (l0, l1, l2) = (0.2126, 0.7152, 0.0722);
        b.backdrop_group(
            1,
            vec![BackdropFilter::ColorMatrix {
                matrix: [
                    l0 * (1.0 - s) + s,
                    l1 * (1.0 - s),
                    l2 * (1.0 - s),
                    0.0,
                    l0 * (1.0 - s),
                    l1 * (1.0 - s) + s,
                    l2 * (1.0 - s),
                    0.0,
                    l0 * (1.0 - s),
                    l1 * (1.0 - s),
                    l2 * (1.0 - s) + s,
                    0.0,
                ],
            }],
        );
        b.backdrop_group(2, vec![BackdropFilter::GaussianBlur { sigma: 4.0 }]);
        let l = &mut b.root();
        backdrop_background(l);
        l.layer(|m| {
            m.transform(
                Affine::translate(Vec2::new(128.0, 128.0))
                    * rotate(17.0f64.to_radians())
                    * Affine::scale(1.2),
            );
            m.clip(Shape::rect(-44.0, -44.0, 88.0, 88.0));
            m.backdrop(1);
            m.fill(
                Shape::rect(-42.0, -42.0, 84.0, 84.0),
                solid(srgba(1.0, 1.0, 1.0, 0.3)),
            );
        });
        // A blurred member inside a scrolled parent: the member's clip
        // rides the parent's content transform.
        l.layer(|p| {
            p.scroll_offset(Vec2::new(12.0, 20.0));
            p.layer(|m| {
                let clip = Shape::RoundedRect(RoundedRect::new(40.0, 60.0, 150.0, 120.0, 12.0));
                m.clip(clip);
                m.backdrop(2);
                m.fill(
                    Shape::rect(42.0, 62.0, 106.0, 56.0),
                    solid(srgba(1.0, 1.0, 1.0, 0.25)),
                );
            });
        });
    });

    corpus.scene_setup("backdrop-hdr", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 5.0 }]);
        let l = &mut b.root();
        // A gradient base plus HDR peaks: 16x white in P3, a P3 green
        // outside sRGB, and a Rec. 2020 accent.
        l.fill(
            Shape::rect(0.0, 0.0, 256.0, 256.0),
            Paint::Linear(LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(256.0, 256.0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: Color::new(ColorSpace::DisplayP3, [0.05, 0.10, 0.30, 1.0]),
                    },
                    GradientStop {
                        offset: 1.0,
                        color: Color::new(ColorSpace::DisplayP3, [0.45, 0.12, 0.05, 1.0]),
                    },
                ],
                extend: Extend::Pad,
                interpolation: ColorSpace::LinearP3,
            }),
        );
        l.fill(
            Shape::rect(24.0, 24.0, 104.0, 104.0),
            solid(Color::new(ColorSpace::DisplayP3, [16.0, 16.0, 16.0, 1.0])),
        );
        l.fill(
            Shape::circle(196.0, 76.0, 56.0),
            solid(Color::new(ColorSpace::DisplayP3, [0.0, 1.0, 0.1, 1.0])),
        );
        l.fill(
            Shape::rect(60.0, 160.0, 172.0, 64.0),
            solid(Color::new(ColorSpace::Rec2020, [0.9, 0.15, 0.6, 1.0])),
        );
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(40.0, 40.0, 216.0, 216.0, 28.0));
            m.clip(clip);
            m.backdrop(1);
            m.fill(
                Shape::rect(42.0, 42.0, 172.0, 172.0),
                solid(srgba(1.0, 1.0, 1.0, 0.1)),
            );
        });
    });

    // ---- Per-member backdrop effects --------------------------------------

    corpus.scene_setup("backdrop-refraction", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 6.0 }]);
        let l = &mut b.root();
        stripes(
            l,
            srgb(0.05, 0.30, 0.95),
            srgb(0.95, 0.25, 0.05),
            [
                srgb(0.95, 0.75, 0.05),
                srgb(0.10, 0.85, 0.40),
                srgb(0.70, 0.10, 0.90),
            ],
        );
        refraction_member(l, 40.0, 32.0, 88.0, 80.0, 12.0, 6.0);
        refraction_member(l, 112.0, 96.0, 208.0, 192.0, 12.0, 6.0);
        refraction_member(l, 80.0, 200.0, 240.0, 250.0, 12.0, 6.0);
    });

    // The same backdrop and blur with non-rounded member clips: the
    // ellipse and the continuous rect run the second-order SDFs (#173),
    // not the rounded-rect closed form.
    corpus.scene_setup("backdrop-refraction-shapes", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 6.0 }]);
        let l = &mut b.root();
        stripes(
            l,
            srgb(0.05, 0.30, 0.95),
            srgb(0.95, 0.25, 0.05),
            [
                srgb(0.95, 0.75, 0.05),
                srgb(0.10, 0.85, 0.40),
                srgb(0.70, 0.10, 0.90),
            ],
        );
        l.layer(|m| {
            m.clip(Shape::Ellipse(Ellipse::new(
                (72.0, 72.0),
                (48.0, 28.0),
                0.0,
            )));
            m.backdrop(1);
            m.backdrop_effect(BackdropEffectSpec::Refraction {
                depth: 12.0,
                strength: 6.0,
            });
            m.fill(
                Shape::rect(26.0, 46.0, 118.0, 98.0),
                solid(srgba(1.0, 1.0, 1.0, 0.12)),
            );
        });
        l.layer(|m| {
            m.clip(Shape::Continuous(cherenkov_scene::ContinuousRect::new(
                Rect::new(120.0, 120.0, 240.0, 232.0),
                32.0,
                0.6,
            )));
            m.backdrop(1);
            m.backdrop_effect(BackdropEffectSpec::Refraction {
                depth: 12.0,
                strength: 6.0,
            });
            m.fill(
                Shape::rect(122.0, 122.0, 238.0, 230.0),
                solid(srgba(1.0, 1.0, 1.0, 0.12)),
            );
        });
    });

    corpus.scene_setup("backdrop-refraction-p3", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 6.0 }]);
        let l = &mut b.root();
        stripes(
            l,
            p3(1.0, 0.0, 0.0),
            p3(0.0, 1.0, 0.0),
            [p3(0.0, 0.9, 0.4), p3(0.9, 0.6, 0.0), p3(0.4, 0.0, 1.0)],
        );
        refraction_member(l, 40.0, 32.0, 88.0, 80.0, 12.0, 6.0);
        refraction_member(l, 112.0, 96.0, 208.0, 192.0, 12.0, 6.0);
        refraction_member(l, 80.0, 200.0, 240.0, 250.0, 12.0, 6.0);
    });

    corpus.scene_setup("backdrop-tint", 256, 256, white, |b| {
        b.backdrop_group(1, Vec::new());
        let l = &mut b.root();
        backdrop_background(l);
        // A plain member beside a warm-tinted one; the bias sits in the
        // matrix's fourth column (multiplied by alpha).
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(24.0, 24.0, 140.0, 124.0, 16.0));
            m.clip(clip);
            m.backdrop(1);
        });
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(116.0, 132.0, 232.0, 232.0, 16.0));
            m.clip(clip);
            m.backdrop(1);
            m.backdrop_effect(BackdropEffectSpec::ColorMatrix {
                matrix: [
                    1.1, 0.0, 0.0, 0.05, //
                    0.0, 0.95, 0.0, 0.02, //
                    0.0, 0.0, 0.8, 0.0,
                ],
            });
        });
    });

    corpus.scene_setup("backdrop-tint-p3", 256, 256, white, |b| {
        b.backdrop_group(1, Vec::new());
        let l = &mut b.root();
        l.fill(
            Shape::rect(0.0, 0.0, 128.0, 256.0),
            solid(p3(1.0, 0.0, 0.0)),
        );
        l.fill(
            Shape::rect(128.0, 0.0, 256.0, 256.0),
            solid(p3(0.0, 0.9, 0.4)),
        );
        l.fill(Shape::circle(96.0, 96.0, 56.0), solid(p3(0.9, 0.7, 0.0)));
        l.fill(Shape::circle(190.0, 180.0, 48.0), solid(p3(0.5, 0.0, 1.0)));
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(24.0, 24.0, 140.0, 124.0, 16.0));
            m.clip(clip);
            m.backdrop(1);
        });
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(116.0, 132.0, 232.0, 232.0, 16.0));
            m.clip(clip);
            m.backdrop(1);
            m.backdrop_effect(BackdropEffectSpec::ColorMatrix {
                matrix: [
                    1.3, 0.0, 0.0, 0.05, //
                    0.0, 0.9, 0.0, 0.0, //
                    0.0, 0.0, 0.85, 0.0,
                ],
            });
        });
    });

    corpus.scene_setup("backdrop-rim-hdr", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 8.0 }]);
        let l = &mut b.root();
        // Mid-grey base with a 16x SDR-white disc and an HDR accent so the
        // rim highlight lands on a >1 substrate.
        l.fill(
            Shape::rect(0.0, 0.0, 256.0, 256.0),
            solid(p3(0.45, 0.45, 0.5)),
        );
        l.fill(
            Shape::circle(128.0, 128.0, 72.0),
            solid(hdr(16.0, 16.0, 16.0)),
        );
        l.fill(Shape::circle(64.0, 208.0, 32.0), solid(hdr(0.5, 8.0, 3.0)));
        let rim = BackdropEffectSpec::RimLight {
            width: 10.0,
            color: [1.0, 0.9, 0.7, 1.0],
            gain: 4.0,
        };
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(32.0, 48.0, 224.0, 176.0, 20.0));
            m.clip(clip);
            m.backdrop(1);
            m.backdrop_effect(rim.clone());
        });
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(80.0, 160.0, 240.0, 240.0, 24.0));
            m.clip(clip);
            m.backdrop(1);
            m.backdrop_effect(rim);
        });
    });

    corpus.scene_setup("backdrop-rim-p3", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 8.0 }]);
        let l = &mut b.root();
        // P3-only colours, none of them HDR: the rim highlight must carry
        // wide gamut without exceeding SDR white.
        l.fill(
            Shape::rect(0.0, 0.0, 256.0, 256.0),
            solid(p3(0.0, 0.35, 0.6)),
        );
        l.fill(Shape::circle(128.0, 128.0, 72.0), solid(p3(1.0, 0.0, 0.0)));
        l.fill(Shape::circle(64.0, 208.0, 32.0), solid(p3(0.0, 0.9, 0.4)));
        let rim = BackdropEffectSpec::RimLight {
            width: 10.0,
            color: [0.0, 1.0, 0.0, 1.0],
            gain: 1.0,
        };
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(32.0, 48.0, 224.0, 176.0, 20.0));
            m.clip(clip);
            m.backdrop(1);
            m.backdrop_effect(rim.clone());
        });
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(80.0, 160.0, 240.0, 240.0, 24.0));
            m.clip(clip);
            m.backdrop(1);
            m.backdrop_effect(rim);
        });
    });

    corpus.scene_setup("backdrop-refraction-hdr", 256, 256, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 6.0 }]);
        let l = &mut b.root();
        stripes(
            l,
            hdr(6.0, 0.5, 0.2),
            hdr(0.3, 0.8, 12.0),
            [
                hdr(16.0, 16.0, 16.0),
                hdr(0.2, 5.0, 1.0),
                hdr(3.0, 0.1, 8.0),
            ],
        );
        refraction_member(l, 40.0, 32.0, 88.0, 80.0, 12.0, 6.0);
        refraction_member(l, 112.0, 96.0, 208.0, 192.0, 12.0, 6.0);
        refraction_member(l, 80.0, 200.0, 240.0, 250.0, 12.0, 6.0);
    });

    corpus.scene_setup("backdrop-tint-hdr", 256, 256, white, |b| {
        b.backdrop_group(1, Vec::new());
        let l = &mut b.root();
        // HDR backdrop: the tint matrix scales values already above one.
        l.fill(
            Shape::rect(0.0, 0.0, 128.0, 256.0),
            solid(hdr(4.0, 1.0, 0.5)),
        );
        l.fill(
            Shape::rect(128.0, 0.0, 256.0, 256.0),
            solid(hdr(0.5, 2.0, 8.0)),
        );
        l.fill(
            Shape::circle(96.0, 96.0, 56.0),
            solid(hdr(16.0, 16.0, 16.0)),
        );
        l.fill(Shape::circle(190.0, 180.0, 48.0), solid(hdr(0.2, 6.0, 1.0)));
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(24.0, 24.0, 140.0, 124.0, 16.0));
            m.clip(clip);
            m.backdrop(1);
        });
        l.layer(|m| {
            let clip = Shape::RoundedRect(RoundedRect::new(116.0, 132.0, 232.0, 232.0, 16.0));
            m.clip(clip);
            m.backdrop(1);
            m.backdrop_effect(BackdropEffectSpec::ColorMatrix {
                matrix: [
                    1.1, 0.0, 0.0, 0.05, //
                    0.0, 0.95, 0.0, 0.02, //
                    0.0, 0.0, 0.8, 0.0,
                ],
            });
        });
    });

    // ---- Sparse capture (#117) ----------------------------------------------

    corpus.scene_setup("backdrop-bars", 1024, 2216, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 8.0 }]);
        let l = &mut b.root();
        let palette = [
            srgb(0.90, 0.25, 0.20),
            srgb(0.20, 0.70, 0.35),
            srgb(0.15, 0.40, 0.95),
            srgb(0.95, 0.70, 0.10),
            srgb(0.60, 0.15, 0.80),
        ];
        bars_background(l, |i| palette[(i as usize) % palette.len()]);
        bars(l);
    });

    corpus.scene_setup("backdrop-bars-p3", 1024, 2216, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 8.0 }]);
        let l = &mut b.root();
        let palette = [
            p3(1.0, 0.0, 0.0),
            p3(0.0, 1.0, 0.0),
            p3(0.0, 0.9, 0.4),
            p3(0.9, 0.6, 0.0),
            p3(0.4, 0.0, 1.0),
        ];
        bars_background(l, |i| palette[(i as usize) % palette.len()]);
        bars(l);
    });

    // One group, five members: an overlapping pair, a nearly-touching
    // pair inside the apron (helped by the Refraction member's reach)
    // and one far corner member. The empty space between the pairs
    // costs more than the merge overhead, so the group resolves to
    // three regions: each pair and the corner.
    corpus.scene_setup("backdrop-sparse-mixed", 512, 512, white, |b| {
        b.backdrop_group(1, vec![BackdropFilter::GaussianBlur { sigma: 4.0 }]);
        let l = &mut b.root();
        l.fill(
            Shape::rect(0.0, 0.0, 256.0, 512.0),
            solid(srgb(0.85, 0.30, 0.15)),
        );
        l.fill(
            Shape::rect(256.0, 0.0, 512.0, 512.0),
            solid(srgb(0.10, 0.35, 0.85)),
        );
        l.fill(
            Shape::circle(128.0, 256.0, 96.0),
            solid(srgb(0.20, 0.75, 0.40)),
        );
        l.fill(
            Shape::circle(384.0, 384.0, 80.0),
            solid(srgb(0.90, 0.70, 0.10)),
        );
        let member = |l: &mut LayerBuilder, rect: [f64; 4], effect: Option<BackdropEffectSpec>| {
            l.layer(|m| {
                m.clip(Shape::RoundedRect(RoundedRect::new(
                    rect[0], rect[1], rect[2], rect[3], 12.0,
                )));
                m.backdrop(1);
                if let Some(effect) = effect {
                    m.backdrop_effect(effect);
                }
            });
        };
        // Overlapping pair.
        member(l, [24.0, 24.0, 96.0, 96.0], None);
        member(l, [64.0, 64.0, 136.0, 136.0], None);
        // Nearly touching (10 px gap inside the σ=4 apron), one with a
        // Refraction reach that widens its aproned rect.
        member(l, [280.0, 24.0, 380.0, 124.0], None);
        member(
            l,
            [390.0, 24.0, 490.0, 124.0],
            Some(BackdropEffectSpec::Refraction {
                depth: 10.0,
                strength: 8.0,
            }),
        );
        // Far corner: stays a separate region.
        member(l, [440.0, 428.0, 508.0, 508.0], None);
    });

    // The #211 dense city map: a 1600×1200 frame whose live coverage
    // exceeds one atlas page. Its output is generated — never committed.
    {
        const LABELS: &[&str] = &[
            "1 AV",
            "2 AV",
            "LEXINGTON AV",
            "PARK AV",
            "5 AV",
            "6 AV",
            "7 AV",
            "BROADWAY",
            "W 14 ST",
            "W 23 ST",
            "W 34 ST",
            "W 42 ST",
            "W 57 ST",
            "W 72 ST",
            "HOUSTON ST",
            "CANAL ST",
        ];
        let shaped: Vec<Vec<GlyphRun>> = LABELS
            .iter()
            .map(|t| ctx.shape("NotoSans.ttf", t, 12.0, FontWeight::NORMAL, &solid(dark)))
            .collect();
        let blobs = font_blobs(&ctx, &shaped.iter().map(Vec::as_slice).collect::<Vec<_>>());
        corpus.scene_with_blobs(
            "dense-map",
            1600,
            1200,
            srgb(0.93, 0.94, 0.92),
            |l| dense_map_body(l, &shaped),
            blobs,
        );
    }

    // ---- Scenes committed before the generator covered them ----------

    authored::add(&mut corpus, &mut ctx)?;

    // ---- Projective layers (#84) -------------------------------------------

    projective_scenes(&mut corpus, &mut ctx);

    // ---- Stress scenes -----------------------------------------------------

    // Issue #210: the merged single-path dense map that drove
    // `resolve_winding` quadratic.
    corpus.scene("map-manhattan", 1600, 1200, srgb(0.92, 0.93, 0.89), |l| {
        map_manhattan_body(l);
    });

    // ---- Write out ---------------------------------------------------------

    let perf_out = corpus::perf_dir(&root);
    write_corpus(&out, &corpus)?;
    write_corpus(&perf_out, &perf)?;
    tracing::info!(count = corpus.entries.len(), dir = %out.display(), "corpus written");
    tracing::info!(count = perf.entries.len(), dir = %perf_out.display(), "perf set written");
    Ok(())
}
