//! Text-layout scenes (#26): parley layouts the Cherenkov adapters record
//! through the engine's `draw_text`.
//!
//! Each scene's text layer carries the parley input ([`TextSource`]) and,
//! as its items, the reference lowering written here: one glyph run per
//! parley glyph run, and one rectangle per underline or strikethrough
//! stretch, merged along a line while brush and geometry continue. A
//! synthetic bold is the run filled and then stroked with a mitred outline,
//! inside a plain group unless its brush is an opaque colour. Each
//! line draws its underlines, then its glyphs, then its strikethroughs.
//! The oracle renders the items, so it checks the engine's own lowering.

use cherenkov_scene::kurbo::{Affine, Point, Rect};
use cherenkov_scene::{
    BlendMode, BlendSpace, Color, ColorSpace, Draw, Glyph, GlyphRun, Group, GroupItem,
    LayerBuilder, LinearGradient, NormalizedCoord, Paint, SceneError, Shape, StrokeStyle,
    TextDecoration, TextSource, TextSpan,
};
use parley::PositionedLayoutItem;
use read_fonts::types::F2Dot14;
use skrifa::MetadataProvider;

use crate::{Corpus, TextContext};

/// A decoration stretch along a line, in parley's f32 layout units.
struct Stretch {
    start: f32,
    end: f32,
    top: f32,
    size: f32,
    brush: usize,
}

/// `paint`'s index in `paints`, appending it when new: equal paints share
/// one brush, so decorations merge exactly when their paints are equal.
fn intern(paints: &mut Vec<Paint>, paint: &Paint) -> usize {
    paints.iter().position(|p| p == paint).unwrap_or_else(|| {
        paints.push(paint.clone());
        paints.len() - 1
    })
}

/// Extends `stretches` with a run's decoration from `start` to `end`.
#[expect(
    clippy::float_cmp,
    reason = "parley accumulates run offsets exactly, so contiguity is exact"
)]
fn extend(stretches: &mut Vec<Stretch>, start: f32, end: f32, top: f32, size: f32, brush: usize) {
    if let Some(last) = stretches.last_mut()
        && last.end == start
        && last.top == top
        && last.size == size
        && last.brush == brush
    {
        last.end = end;
        return;
    }
    stretches.push(Stretch {
        start,
        end,
        top,
        size,
        brush,
    });
}

/// A stretch as the rectangle fill it draws, moved to `origin`.
fn stretch_fill(stretch: &Stretch, origin: Point, paints: &[Paint]) -> Draw {
    let rect = Rect::new(
        f64::from(stretch.start),
        f64::from(stretch.top),
        f64::from(stretch.end),
        f64::from(stretch.top) + f64::from(stretch.size),
    ) + origin.to_vec2();
    Draw::Fill {
        shape: Shape::Rect(rect),
        rule: cherenkov_scene::FillRule::NonZero,
        paint: paints[stretch.brush].clone(),
    }
}

/// The synthetic bold's stroke width at `size` pixels per em: 1/24 of the
/// em at 9 px and below, 1/32 at 36 px and above, linear in between.
fn embolden_width(size: f32) -> f64 {
    let size = f64::from(size);
    let t = ((size - 9.0) / 27.0).clamp(0.0, 1.0);
    size * t.mul_add(1.0 / 32.0 - 1.0 / 24.0, 1.0 / 24.0)
}

/// A parley glyph run as a filled scene run moved to `origin`, its brush
/// resolved through `paints`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "glyph positions are f32 by contract; the sum rounds once"
)]
fn scene_run(
    glyph_run: &parley::GlyphRun<'_, usize>,
    resources: &cherenkov_scene::FontResources,
    ctx: &TextContext,
    origin: Point,
    paints: &[Paint],
) -> GlyphRun {
    let run = glyph_run.run();
    let transform = run
        .synthesis()
        .skew()
        .map(|degrees| Affine::skew(-libm::tan(f64::from(degrees).to_radians()), 0.0));
    let font = resources
        .resource(run.font())
        .expect("every run's font comes from the scene's stack");
    let axes: Vec<String> = skrifa::FontRef::new(&ctx.blobs[&font])
        .expect("registered font parses")
        .axes()
        .iter()
        .map(|axis| axis.tag().to_string())
        .collect();
    GlyphRun {
        font,
        font_index: run.font().index,
        size: run.font_size(),
        normalized_coords: run
            .normalized_coords()
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(i, v)| NormalizedCoord {
                tag: axes[i].clone(),
                value: F2Dot14::from_bits(*v).to_f32(),
            })
            .collect(),
        glyphs: glyph_run
            .positioned_glyphs()
            .map(|g| Glyph {
                id: g.id,
                x: (f64::from(g.x) + origin.x) as f32,
                y: (f64::from(g.y) + origin.y) as f32,
                transform,
            })
            .collect(),
        stroke: None,
        paint: paints[glyph_run.style().brush].clone(),
    }
}

/// Draws a filled run, or a synthetic bold's fill and stroke: directly
/// under an opaque colour, in a plain group under any other brush.
fn draw_run(l: &mut LayerBuilder, fill: GlyphRun, embolden: bool) {
    if !embolden {
        l.glyphs(fill);
        return;
    }
    let stroke = GlyphRun {
        stroke: Some(StrokeStyle {
            width: embolden_width(fill.size),
            ..StrokeStyle::default()
        }),
        ..fill.clone()
    };
    if matches!(fill.paint, Paint::Solid(color) if color.components[3] >= 1.0) {
        l.glyphs(fill).glyphs(stroke);
    } else {
        l.group(Group {
            items: vec![
                GroupItem::Draw(Draw::Glyphs(fill)),
                GroupItem::Draw(Draw::Glyphs(stroke)),
            ],
            opacity: 1.0,
            blend: BlendMode::Normal,
            blend_space: BlendSpace::Linear,
        });
    }
}

/// Builds `l` as a text layer for `source`: the source plus its reference
/// lowering.
fn text_layer(l: &mut LayerBuilder, ctx: &TextContext, source: TextSource) {
    let mut paints = Vec::new();
    let shaped = source
        .shape(
            |hash| ctx.blobs.get(hash).map(Vec::as_slice),
            |paint| Ok::<_, SceneError>(intern(&mut paints, paint)),
        )
        .expect("the corpus fonts shape every text-layout scene");
    let origin = source.origin;
    for line in shaped.layout.lines() {
        let mut underlines = Vec::new();
        let mut strikethroughs = Vec::new();
        let mut runs = Vec::new();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            runs.push((
                scene_run(&glyph_run, &shaped.resources, ctx, origin, &paints),
                run.synthesis().embolden(),
            ));
            let style = glyph_run.style();
            let metrics = run.metrics();
            let start = glyph_run.offset();
            let end = start + glyph_run.advance();
            if let Some(underline) = &style.underline {
                extend(
                    &mut underlines,
                    start,
                    end,
                    glyph_run.baseline() - underline.offset.unwrap_or(metrics.underline_offset),
                    underline.size.unwrap_or(metrics.underline_size),
                    underline.brush,
                );
            }
            if let Some(strikethrough) = &style.strikethrough {
                extend(
                    &mut strikethroughs,
                    start,
                    end,
                    glyph_run.baseline()
                        - strikethrough.offset.unwrap_or(metrics.strikethrough_offset),
                    strikethrough.size.unwrap_or(metrics.strikethrough_size),
                    strikethrough.brush,
                );
            }
        }
        for stretch in &underlines {
            l.push(stretch_fill(stretch, origin, &paints));
        }
        for (fill, embolden) in runs {
            draw_run(l, fill, embolden);
        }
        for stretch in &strikethroughs {
            l.push(stretch_fill(stretch, origin, &paints));
        }
    }
    l.text(source);
}

/// The byte range of the first `needle` in `text`.
fn range(text: &str, needle: &str) -> [usize; 2] {
    let start = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is in {text:?}"));
    [start, start + needle.len()]
}

/// The colours of one text-layout variant.
struct Inks {
    text: Color,
    accent: Color,
    warm: Color,
    cool: Color,
    line: Color,
    /// A translucent brush.
    glass: Color,
}

/// A styled paragraph: brushes (solid and gradient), underlines with the
/// font's metrics and with explicit brush and thickness, strikethroughs,
/// a synthetic oblique, and wrapped lines.
fn styled(ctx: &TextContext, inks: &Inks) -> TextSource {
    let text = "The quick brown fox jumps over the lazy dog. \
                Pack my box with five dozen liquor jugs! 0123456789?";
    let gradient = Paint::Linear(LinearGradient {
        start: Point::new(120.0, 0.0),
        end: Point::new(260.0, 0.0),
        stops: vec![
            cherenkov_scene::GradientStop {
                offset: 0.0,
                color: inks.warm,
            },
            cherenkov_scene::GradientStop {
                offset: 1.0,
                color: inks.cool,
            },
        ],
        extend: cherenkov_scene::Extend::Pad,
        interpolation: ColorSpace::LinearP3,
    });
    let span = |needle: &str| TextSpan {
        range: range(text, needle),
        paint: None,
        italic: false,
        bold: false,
        underline: None,
        strikethrough: None,
    };
    TextSource {
        text: text.to_owned(),
        fonts: vec![ctx.hashes["NotoSans.ttf"]],
        size: 22.0,
        max_advance: Some(320.0),
        origin: Point::new(16.0, 10.0),
        paint: Paint::Solid(inks.text),
        spans: vec![
            TextSpan {
                paint: Some(Paint::Solid(inks.accent)),
                underline: Some(TextDecoration::default()),
                ..span("quick brown")
            },
            TextSpan {
                paint: Some(gradient),
                underline: Some(TextDecoration {
                    paint: Some(Paint::Solid(inks.line)),
                    offset: None,
                    size: Some(2.5),
                }),
                ..span("fox jumps")
            },
            TextSpan {
                strikethrough: Some(TextDecoration::default()),
                ..span("over the lazy")
            },
            TextSpan {
                italic: true,
                underline: Some(TextDecoration::default()),
                ..span("dog.")
            },
            TextSpan {
                strikethrough: Some(TextDecoration {
                    paint: Some(Paint::Solid(inks.line)),
                    offset: Some(9.0),
                    size: Some(3.0),
                }),
                ..span("Pack my box")
            },
            TextSpan {
                paint: Some(Paint::Solid(inks.accent)),
                italic: true,
                ..span("liquor jugs!")
            },
            TextSpan {
                underline: Some(TextDecoration::default()),
                ..span("0123456789")
            },
        ],
    }
}

/// Bidirectional text over a font stack: Latin, Arabic and Hebrew runs in
/// visual order, one underline across all of them.
fn scripts(ctx: &TextContext, inks: &Inks) -> TextSource {
    let text = "Hello مرحبا بالعالم and שלום עולם!";
    TextSource {
        text: text.to_owned(),
        fonts: vec![
            ctx.hashes["NotoSans.ttf"],
            ctx.hashes["NotoSansArabic.ttf"],
            ctx.hashes["NotoSansHebrew.ttf"],
        ],
        size: 20.0,
        max_advance: None,
        origin: Point::new(12.0, 12.0),
        paint: Paint::Solid(inks.text),
        spans: vec![
            TextSpan {
                range: [0, text.len()],
                paint: None,
                italic: false,
                bold: false,
                underline: Some(TextDecoration::default()),
                strikethrough: None,
            },
            TextSpan {
                range: range(text, "بالعالم"),
                paint: Some(Paint::Solid(inks.accent)),
                italic: false,
                bold: false,
                underline: None,
                strikethrough: Some(TextDecoration {
                    paint: Some(Paint::Solid(inks.line)),
                    offset: None,
                    size: None,
                }),
            },
        ],
    }
}

/// Synthetic bold over a font with one regular face and no weight axis,
/// beside its regular weight: an opaque brush (drawn directly), a
/// translucent and a gradient brush (each isolated), and a bold oblique
/// with an underline.
fn bold(ctx: &TextContext, inks: &Inks) -> TextSource {
    let text = "Regular and bold ink, bold glass, bold gradient and bold oblique.";
    let span = |needle: &str| TextSpan {
        range: range(text, needle),
        paint: None,
        italic: false,
        bold: true,
        underline: None,
        strikethrough: None,
    };
    let gradient = Paint::Linear(LinearGradient {
        start: Point::new(0.0, 0.0),
        end: Point::new(180.0, 0.0),
        stops: vec![
            cherenkov_scene::GradientStop {
                offset: 0.0,
                color: inks.warm,
            },
            cherenkov_scene::GradientStop {
                offset: 1.0,
                color: inks.cool,
            },
        ],
        extend: cherenkov_scene::Extend::Pad,
        interpolation: ColorSpace::LinearP3,
    });
    TextSource {
        text: text.to_owned(),
        fonts: vec![ctx.hashes["CherenkovStaticSans.ttf"]],
        size: 24.0,
        max_advance: Some(330.0),
        origin: Point::new(14.0, 10.0),
        paint: Paint::Solid(inks.text),
        spans: vec![
            span("bold ink"),
            TextSpan {
                paint: Some(Paint::Solid(inks.glass)),
                ..span("bold glass")
            },
            TextSpan {
                paint: Some(gradient),
                ..span("bold gradient")
            },
            TextSpan {
                paint: Some(Paint::Solid(inks.accent)),
                italic: true,
                underline: Some(TextDecoration::default()),
                ..span("bold oblique")
            },
        ],
    }
}

/// Adds the text-layout scenes: each in sRGB, P3-only and HDR inks.
pub fn scenes(corpus: &mut Corpus, ctx: &TextContext) {
    let variants = [
        (
            "",
            Inks {
                text: Color::srgb(0.08, 0.09, 0.12),
                accent: Color::srgb(0.1, 0.35, 0.85),
                warm: Color::srgb(0.9, 0.25, 0.1),
                cool: Color::srgb(0.2, 0.6, 0.3),
                line: Color::srgb(0.85, 0.1, 0.35),
                glass: Color::new(ColorSpace::Srgb, [0.1, 0.35, 0.85, 0.45]),
            },
        ),
        (
            "-p3",
            Inks {
                text: Color::new(ColorSpace::LinearP3, [0.0, 0.02, 0.05, 1.0]),
                accent: Color::new(ColorSpace::LinearP3, [0.0, 0.3, 1.0, 1.0]),
                warm: Color::new(ColorSpace::LinearP3, [1.0, 0.0, 0.0, 1.0]),
                cool: Color::new(ColorSpace::LinearP3, [0.0, 1.0, 0.0, 1.0]),
                line: Color::new(ColorSpace::LinearP3, [1.0, 0.0, 0.6, 1.0]),
                glass: Color::new(ColorSpace::LinearP3, [0.0, 0.3, 1.0, 0.45]),
            },
        ),
        (
            "-hdr",
            Inks {
                text: Color::new(ColorSpace::LinearP3, [0.0, 0.02, 0.05, 1.0]),
                accent: Color::new(ColorSpace::LinearP3, [0.2, 1.5, 6.0, 1.0]),
                warm: Color::new(ColorSpace::LinearP3, [8.0, 1.0, 0.2, 1.0]),
                cool: Color::new(ColorSpace::LinearP3, [0.3, 4.0, 0.5, 1.0]),
                line: Color::new(ColorSpace::LinearP3, [3.0, 0.2, 2.0, 1.0]),
                glass: Color::new(ColorSpace::LinearP3, [0.2, 1.5, 6.0, 0.45]),
            },
        ),
    ];
    let white = Color::srgb(1.0, 1.0, 1.0);
    for (suffix, inks) in &variants {
        for (name, source, height) in [
            ("text-layout-styled", styled(ctx, inks), 200),
            ("text-layout-scripts", scripts(ctx, inks), 56),
            ("text-layout-bold", bold(ctx, inks), 120),
        ] {
            let blobs = source
                .fonts
                .iter()
                .map(|hash| ctx.blobs[hash].clone())
                .collect();
            corpus.scene_with_blobs(
                format!("{name}{suffix}"),
                360,
                height,
                white,
                |l| {
                    l.layer(|t| text_layer(t, ctx, source));
                },
                blobs,
            );
        }
    }
}
