//! The Rust half of the command-buffer contract test: a fixed set of scenes
//! encoded at test time, together covering every opcode. With
//! `HWUI_CONTRACT_DIR` set, each frame's bytes go to `<scene>-<frame>.bin`
//! and the scene's event log to `<scene>.log`; the JVM `ContractTest`
//! decodes those bytes and requires the same log. Nothing encoded is
//! committed.

use std::collections::BTreeSet;
use std::f64::consts::TAU;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use waterui_graphics::draw::color::WorkingColor;
use waterui_graphics::draw::kurbo::{
    Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, Stroke, Vec2,
};
use waterui_graphics::draw::{
    BlendMode, ChangeSet, ColorStop, ContentOp, Curve, Draw, Extend, FontId, Glyph, GlyphRun,
    GlyphStyle, Group, ImageId, ImagePattern, Instant, Interpolation, LayerId, LayerOp,
    LinearGradient, MeshColorInterpolation, MeshGradient, Op as TreeOp, Paint, Picture, Prop,
    RadialGradient, Sampling, ShaderId, ShaderPaint, Shadow, ShapeData, StaticRecorder,
    SweepGradient,
};

use super::buffer::{CommandBuffer, Field};
use super::encoder::{Encoder, HostEntry};
use super::fonts::FontInfo;
use super::lower::{Caches, Lower, Pools, Scratch};
use super::protocol::{MESH_BAND_PATCHES, NONE, Op};
use super::resources::{Kind, SharedRegistry};
use waterui_graphics::draw::TextLayoutId;

use super::text::{
    ShapeRequest, TextAlignment, TextLayoutIds, TextRun, Utf16Index, describe_paragraph,
    describe_run,
};
use super::{HwuiError, HwuiTarget};

/// One encoded scene: its frames' words and its event log.
struct Scene {
    name: &'static str,
    frames: Vec<Vec<u32>>,
    log: Vec<String>,
}

const fn color(r: f32, g: f32, b: f32, a: f32) -> WorkingColor {
    WorkingColor {
        components: [r, g, b, a],
    }
}

const fn prop<T>(target: T) -> Prop<T> {
    Prop {
        target,
        animation: None,
    }
}

fn stops() -> Vec<ColorStop> {
    vec![
        ColorStop {
            offset: 0.0,
            color: color(1.0, 0.0, 0.0, 1.0),
        },
        ColorStop {
            offset: 1.0,
            color: color(0.0, 0.0, 1.0, 0.5),
        },
    ]
}

fn changes(ops: Vec<TreeOp<HwuiTarget>>) -> ChangeSet<HwuiTarget> {
    ChangeSet {
        clear: Some(color(1.0, 1.0, 1.0, 1.0)),
        ops,
        recycled: Vec::new(),
        animating: false,
    }
}

fn layer(op: LayerOp) -> TreeOp<HwuiTarget> {
    TreeOp::Layer(op)
}

/// A path of every segment kind.
fn triangle() -> BezPath {
    let mut triangle = BezPath::new();
    triangle.move_to((0.0, 0.0));
    triangle.line_to((30.0, 0.0));
    triangle.quad_to((30.0, 15.0), (15.0, 30.0));
    triangle.curve_to((10.0, 30.0), (5.0, 20.0), (0.0, 0.0));
    triangle.close_path();
    triangle
}

/// Every shape, paint and recording construct the lowering table names.
fn rich_picture(font: u32, bitmap: u32, runtime: u32, text: u32) -> Picture {
    let shared = Picture::record(|r| {
        r.fill(
            Rect::new(0.0, 0.0, 4.0, 4.0),
            Paint::Solid(color(0.0, 1.0, 0.0, 1.0)),
        );
    });
    let triangle = triangle();
    Picture::record(|r| {
        r.fill(
            Rect::new(0.0, 0.0, 40.0, 30.0),
            Paint::Solid(color(1.0, 0.0, 0.0, 1.0)),
        );
        r.text(
            TextLayoutId::new(u64::from(text)),
            Affine::translate((2.0, 3.0)) * Affine::scale(1.5),
        );
        r.stroke(
            RoundedRect::new(2.0, 2.0, 38.0, 28.0, 4.0),
            Stroke::new(2.0).with_dashes(1.0, [4.0, 2.0]),
            Paint::Linear(LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(40.0, 0.0),
                stops: stops(),
                extend: Extend::Repeat,
                interpolation: Interpolation::SrgbEncoded,
            }),
        );
        r.shadow(
            RoundedRect::new(4.0, 4.0, 20.0, 20.0, 3.0),
            Shadow {
                sigma: 3.0,
                offset: Vec2::new(0.0, 2.0),
                spread: 1.0,
                color: color(0.0, 0.0, 0.0, 0.25),
            },
        );
        r.fill(Circle::new((20.0, 15.0), 5.0), radial_paint());
        r.fill(Rect::new(0.0, 0.0, 10.0, 10.0), sweep_paint());
        r.fill(
            Rect::new(0.0, 0.0, 8.0, 8.0),
            Paint::Image(ImagePattern {
                image: ImageId::new(u64::from(bitmap)),
                transform: Affine::scale(2.0),
                extend_x: Extend::Repeat,
                extend_y: Extend::Reflect,
                sampling: Sampling::Nearest,
            }),
        );
        r.fill(
            Rect::new(0.0, 0.0, 6.0, 6.0),
            Paint::Shader(ShaderPaint {
                shader: ShaderId::new(u64::from(runtime)),
                uniforms: vec![0.5, 1.0],
            }),
        );
        r.fill(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Paint::Sweep(SweepGradient {
                center: Point::new(5.0, 5.0),
                start_angle: 1.0,
                end_angle: 1.0 - TAU / 3.0,
                stops: stops(),
                extend: Extend::Reflect,
                interpolation: Interpolation::SrgbEncoded,
            }),
        );
        r.fill(Rect::new(0.0, 0.0, 10.0, 10.0), mesh_paint());
        r.stroke(
            Circle::new((5.0, 5.0), 4.0),
            Stroke::new(2.0),
            Paint::Mesh(mesh(1, 1).interpolation(MeshColorInterpolation::Smoothstep)),
        );
        r.glyphs(glyph_run(font), mesh_paint());
        r.glyphs(glyph_run(font), Paint::Solid(color(0.0, 0.0, 0.0, 1.0)));
        r.glyphs(varied_run(font), Paint::Solid(color(0.0, 0.0, 0.0, 1.0)));
        r.image(
            ImageId::new(u64::from(bitmap)),
            Rect::new(0.0, 0.0, 16.0, 16.0),
            Sampling::Linear,
        );
        r.stroke(
            Line::new((0.0, 0.0), (10.0, 10.0)),
            Stroke::new(1.0),
            Paint::Solid(color(0.0, 0.0, 0.0, 1.0)),
        );
        record_structure(r, &shared, triangle);
    })
}

/// Clips, a transform, a group and a shared picture.
fn record_structure(r: &mut StaticRecorder, shared: &Picture, triangle: BezPath) {
    r.clip(Rect::new(0.0, 0.0, 20.0, 20.0), |r| {
        r.fill(
            Rect::new(0.0, 0.0, 30.0, 30.0),
            Paint::Solid(color(0.5, 0.5, 0.5, 1.0)),
        );
    });
    r.clip(triangle, |r| {
        r.fill(
            Rect::new(0.0, 0.0, 30.0, 30.0),
            Paint::Solid(color(0.2, 0.4, 0.6, 1.0)),
        );
    });
    r.transform(Affine::skew(0.25, 0.0), |r| {
        r.fill(
            Rect::new(0.0, 0.0, 5.0, 5.0),
            Paint::Solid(color(0.1, 0.1, 0.1, 1.0)),
        );
    });
    r.group(Group::new().opacity(0.5), |r| {
        r.fill(
            Rect::new(0.0, 0.0, 5.0, 5.0),
            Paint::Solid(color(1.0, 0.0, 1.0, 1.0)),
        );
        r.fill(
            Rect::new(2.0, 2.0, 7.0, 7.0),
            Paint::Solid(color(0.0, 1.0, 1.0, 1.0)),
        );
    });
    r.picture(shared, Affine::translate((5.0, 5.0)));
}

fn radial_paint() -> Paint {
    Paint::Radial(RadialGradient {
        start_center: Point::new(20.0, 15.0),
        start_radius: 0.0,
        end_center: Point::new(22.0, 15.0),
        end_radius: 5.0,
        stops: stops(),
        extend: Extend::Reflect,
        interpolation: Interpolation::SrgbEncoded,
    })
}

fn sweep_paint() -> Paint {
    Paint::Sweep(SweepGradient {
        center: Point::new(5.0, 5.0),
        start_angle: 0.0,
        end_angle: TAU,
        stops: stops(),
        extend: Extend::Pad,
        interpolation: Interpolation::SrgbEncoded,
    })
}

fn mesh_paint() -> Paint {
    Paint::Mesh(mesh(1, 1))
}

/// A `columns × rows` mesh over a square of side 10 per patch, its
/// corners cycling through four colours.
fn mesh(columns: u32, rows: u32) -> MeshGradient {
    let palette = [
        color(1.0, 0.0, 0.0, 1.0),
        color(0.0, 1.0, 0.0, 1.0),
        color(0.0, 0.0, 1.0, 0.5),
        color(1.0, 1.0, 0.0, 1.0),
    ];
    let (mut points, mut colors) = (Vec::new(), Vec::new());
    for row in 0..=rows {
        for column in 0..=columns {
            points.push(Point::new(f64::from(column) * 10.0, f64::from(row) * 10.0));
            colors.push(palette[((row + column) % 4) as usize]);
        }
    }
    MeshGradient::new(columns, rows, points, colors)
}

/// A dashed stroke of a variation instance of [`glyph_run`]'s font: a
/// derived font, released with its base.
fn varied_run(font: u32) -> GlyphRun {
    GlyphRun {
        coords: Arc::from([8192]),
        style: GlyphStyle::Stroke(Stroke::new(0.5).with_dashes(0.5, [2.0, 1.0, 0.5])),
        ..glyph_run(font)
    }
}

fn glyph_run(font: u32) -> GlyphRun {
    GlyphRun {
        font: FontId::new(u64::from(font)),
        size: 14.0,
        coords: Arc::from([]),
        glyphs: Arc::from([
            Glyph {
                id: 3,
                x: 1.0,
                y: 12.0,
                transform: None,
            },
            Glyph {
                id: 9,
                x: 8.5,
                y: 12.0,
                transform: None,
            },
        ]),
        style: GlyphStyle::Fill,
    }
}

fn plain_picture() -> Picture {
    Picture::record(|r| {
        r.fill(
            Rect::new(0.0, 0.0, 12.0, 12.0),
            Paint::Solid(color(0.0, 0.5, 0.0, 1.0)),
        );
    })
}

/// A tree of three layers — one scrolled, one skewed — over two frames: the
/// first creates and records everything, the second removes a layer,
/// replaces content and releases every registered resource kind.
fn tree_scene() -> Result<Scene, HwuiError> {
    let mut encoder = Encoder::new(34)?;
    let mut registry = encoder.registry().lock();
    let font = registry.acquire(Kind::Font)?;
    registry.set_font_info(
        font,
        FontInfo::synthetic(
            [-0.25, -0.3, 1.2, 1.1],
            &[(u32::from_be_bytes(*b"wght"), 100.0, 400.0, 900.0)],
        ),
    );
    let bitmap = registry.acquire(Kind::Bitmap)?;
    let runtime = registry.acquire(Kind::RuntimeShader)?;
    let effect = registry.acquire(Kind::Effect)?;
    drop(registry);
    let text_layout = encoder.text_layouts().acquire()?;
    encoder
        .text_layouts()
        .register(text_layout, Rect::new(0.0, 0.0, 20.0, 10.0));
    let root = encoder.tree().root();
    let (card, scroller, skewed) = (LayerId::new(101), LayerId::new(102), LayerId::new(103));
    let mut frames = Vec::new();

    encoder.commit(changes(vec![
        layer(LayerOp::Create(card)),
        layer(LayerOp::Push {
            parent: root,
            child: card,
        }),
        layer(LayerOp::Content(
            card,
            Some(ContentOp::Replace(rich_picture(
                font,
                bitmap,
                runtime,
                text_layout,
            ))),
        )),
        layer(LayerOp::Translation(card, prop(Vec2::new(10.5, 20.0)))),
        layer(LayerOp::Opacity(card, prop(0.5))),
        layer(LayerOp::Clip(
            card,
            Some(ShapeData::RoundedRect(RoundedRect::new(
                0.0, 0.0, 40.0, 30.0, 6.0,
            ))),
        )),
        layer(LayerOp::Blend(card, BlendMode::Multiply)),
        layer(LayerOp::Create(scroller)),
        layer(LayerOp::Push {
            parent: root,
            child: scroller,
        }),
        layer(LayerOp::Content(
            scroller,
            Some(ContentOp::Replace(plain_picture())),
        )),
        layer(LayerOp::ScrollOffset(scroller, prop(Vec2::new(0.0, 30.0)))),
        layer(LayerOp::Rotation(scroller, prop(0.3))),
        layer(LayerOp::Create(skewed)),
        layer(LayerOp::Push {
            parent: scroller,
            child: skewed,
        }),
        layer(LayerOp::Content(
            skewed,
            Some(ContentOp::Replace(plain_picture())),
        )),
        layer(LayerOp::Skew(skewed, prop(Vec2::new(0.3, 0.0)))),
    ]))?;
    encoder.set_host_order(&[HostEntry::Layer(root), HostEntry::PlatformView(7)]);
    frames.push(encoder.frame(Instant::now(), 1.0)?.words.to_vec());

    encoder.commit(changes(vec![
        layer(LayerOp::Detach {
            parent: scroller,
            child: skewed,
        }),
        layer(LayerOp::Remove(skewed)),
        layer(LayerOp::Content(
            card,
            Some(ContentOp::Replace(plain_picture())),
        )),
    ]))?;
    let mut registry = encoder.registry().lock();
    registry.release(Kind::Font, font);
    registry.release(Kind::Bitmap, bitmap);
    registry.release(Kind::RuntimeShader, runtime);
    registry.release(Kind::Effect, effect);
    drop(registry);
    encoder.text_layouts().release(text_layout);
    encoder.set_host_order(&[HostEntry::Layer(root)]);
    frames.push(encoder.frame(Instant::now(), 1.0)?.words.to_vec());

    Ok(Scene {
        name: "tree",
        frames,
        log: encoder.take_log(),
    })
}

/// The op no tree scene reaches before #1750: a node effect.
fn direct_scene() -> Result<Scene, HwuiError> {
    let mut buffer = CommandBuffer::new();
    let mut pools = Pools::new();
    let mut caches = Caches::default();
    let registry = SharedRegistry::new();
    let mut scratch = Scratch::default();
    let text_layouts = TextLayoutIds::new();
    buffer.begin_frame();
    let mut lower = Lower {
        buffer: &mut buffer,
        pools: &mut pools,
        caches: &mut caches,
        registry: &registry,
        text_layouts: &text_layouts,
        scratch: &mut scratch,
        api_level: 34,
        layer: 0,
    };
    let node = lower.create_node()?;
    lower.buffer.op(
        Op::SetEffect,
        &[Field::U("node", node), Field::U("effect", NONE)],
    )?;
    lower.open_at(node, [0, 0], [16, 16])?;
    lower.close()?;
    let words = buffer.finish()?.to_vec();
    Ok(Scene {
        name: "direct",
        frames: vec![words],
        log: buffer.take_log(),
    })
}

fn write(dir: &Path, scene: &Scene) {
    for (index, words) in scene.frames.iter().enumerate() {
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        std::fs::write(dir.join(format!("{}-{index}.bin", scene.name)), bytes)
            .expect("the contract directory is writable");
    }
    let mut log = scene.log.join("\n");
    log.push('\n');
    std::fs::write(dir.join(format!("{}.log", scene.name)), log)
        .expect("the contract directory is writable");
}

#[test]
fn the_contract_scenes_cover_every_opcode() {
    let scenes = [
        tree_scene().expect("the tree scene encodes"),
        direct_scene().expect("the direct scene encodes"),
    ];
    let logged: BTreeSet<&str> = scenes
        .iter()
        .flat_map(|scene| &scene.log)
        .map(|line| line.split(' ').next().unwrap_or_default())
        .collect();
    let missing: Vec<&str> = Op::ALL
        .iter()
        .map(|op| op.name())
        .filter(|name| !logged.contains(name))
        .collect();
    assert!(missing.is_empty(), "no contract scene encodes {missing:?}");
    for scene in &scenes {
        let frames = scene
            .log
            .iter()
            .filter(|line| line.starts_with("Frame "))
            .count();
        assert_eq!(frames, scene.frames.len(), "scene {}", scene.name);
    }
    if let Ok(dir) = std::env::var("HWUI_CONTRACT_DIR") {
        let dir = Path::new(&dir);
        std::fs::create_dir_all(dir).expect("the contract directory can be created");
        for scene in &scenes {
            write(dir, scene);
        }
    }
}

/// Style runs over a text with surrogate pairs and a shared family list: the
/// JVM half reads the packed request back and must print what
/// [`describe_paragraph`] and [`describe_run`] print from the request.
#[test]
fn text_runs_decode_to_the_runs_the_engine_packed() {
    let text = "Wave \u{1f30a} \u{e9}";
    let plain = TextRun {
        range: 0..5,
        family: Some("Inter, 'Noto Sans CJK SC', sans-serif".to_owned()),
        size: 14.0,
        weight: 400,
        italic: false,
        underline: false,
        strikethrough: false,
        foreground: None,
        background: None,
        line_height: None,
        letter_spacing: 0.0,
    };
    let runs = [
        plain.clone(),
        TextRun {
            range: 5..9,
            family: None,
            size: 20.0,
            weight: 700,
            italic: true,
            underline: true,
            foreground: Some(super::color::pack(0.0, 0.5, 1.0, 1.0)),
            background: Some(super::color::pack(1.0, 1.0, 0.0, 0.5)),
            line_height: Some(24.0),
            letter_spacing: 2.0,
            ..plain.clone()
        },
        TextRun {
            range: 9..12,
            strikethrough: true,
            ..plain
        },
    ];
    let index = Utf16Index::new(text).expect("the text indexes");
    let request = ShapeRequest {
        locale: "ja-JP",
        max_width: Some(200.0),
        max_lines: Some(2),
        ellipsis: true,
        alignment: TextAlignment::End,
        right_to_left: true,
        strict_families: true,
        ..ShapeRequest::new(text, &runs)
    };
    let packed = request.pack(&index).expect("the runs pack");
    assert_eq!(packed.families.len(), 1);
    let expected: String = std::iter::once(describe_paragraph(&request) + "\n")
        .chain(runs.iter().map(|run| describe_run(run, &index) + "\n"))
        .collect();
    assert!(
        expected.contains("run 0..5 family=Inter|Noto Sans CJK SC|sans-serif"),
        "{expected}"
    );
    if let Ok(dir) = std::env::var("HWUI_CONTRACT_DIR") {
        let dir = Path::new(&dir);
        std::fs::create_dir_all(dir).expect("the contract directory can be created");
        let spans: Vec<String> = packed.spans.iter().map(ToString::to_string).collect();
        let wire = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            packed.locale,
            packed.max_width,
            packed.max_lines,
            packed.paragraph,
            packed.families.len(),
            packed.families.join("\n"),
            packed.span_count,
            spans.join(" ")
        );
        std::fs::write(dir.join("text-runs.wire"), wire)
            .expect("the contract directory is writable");
        std::fs::write(dir.join("text-runs.expected"), expected)
            .expect("the contract directory is writable");
    }
}

#[test]
fn a_text_layout_is_released_once_no_content_draws_it() {
    let mut encoder = Encoder::new(34).expect("API 34 is supported");
    let layout = encoder.text_layouts().acquire().expect("an id is free");
    encoder
        .text_layouts()
        .register(layout, Rect::new(0.0, 0.0, 20.0, 10.0));
    let root = encoder.tree().root();
    let label = LayerId::new(201);
    let text = Picture::record(|r| r.text(TextLayoutId::new(u64::from(layout)), Affine::IDENTITY));
    encoder
        .commit(changes(vec![
            layer(LayerOp::Create(label)),
            layer(LayerOp::Push {
                parent: root,
                child: label,
            }),
            layer(LayerOp::Content(label, Some(ContentOp::Replace(text)))),
        ]))
        .expect("the label commits");
    encoder.text_layouts().release(layout);
    encoder
        .frame(Instant::now(), 1.0)
        .expect("the frame drawing the layout encodes");
    encoder
        .commit(changes(vec![layer(LayerOp::Content(
            label,
            Some(ContentOp::Replace(plain_picture())),
        ))]))
        .expect("the replacement commits");
    encoder
        .frame(Instant::now(), 1.0)
        .expect("the frame after it encodes");
    let log = encoder.take_log();
    let at = |prefix: &str| -> Vec<usize> {
        log.iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with(prefix))
            .map(|(index, _)| index)
            .collect()
    };
    let (frames, texts, releases) = (at("Frame "), at("Text "), at("ReleaseTextLayout "));
    assert_eq!(frames.len(), 2, "{log:#?}");
    assert_eq!(texts.len(), 1, "{log:#?}");
    assert!(texts[0] < frames[1], "{log:#?}");
    assert_eq!(releases.len(), 1, "{log:#?}");
    assert!(releases[0] > frames[1], "{log:#?}");
}

/// The event log of one layer whose content is `picture`.
fn single_layer_log(picture: Picture) -> Result<Vec<String>, HwuiError> {
    let mut encoder = Encoder::new(34)?;
    let root = encoder.tree().root();
    let layer_id = LayerId::new(201);
    encoder.commit(changes(vec![
        layer(LayerOp::Create(layer_id)),
        layer(LayerOp::Push {
            parent: root,
            child: layer_id,
        }),
        layer(LayerOp::Content(
            layer_id,
            Some(ContentOp::Replace(picture)),
        )),
    ]))?;
    encoder.frame(Instant::now(), 1.0)?;
    Ok(encoder.take_log())
}

/// The `patches` operand of every `Mesh` op in `log`.
fn mesh_bands(log: &[String]) -> Vec<u32> {
    log.iter()
        .filter_map(|line| line.strip_prefix("Mesh "))
        .map(|line| {
            let patches = line
                .split(' ')
                .find_map(|field| field.strip_prefix("patches="))
                .expect("a mesh op carries its patch count");
            patches.parse().expect("the patch count is a number")
        })
        .collect()
}

#[test]
fn a_mesh_of_65536_vertices_encodes_in_bands_of_whole_records() {
    let fill = |mesh: MeshGradient| {
        Picture::record(|r| r.fill(Rect::new(0.0, 0.0, 2560.0, 2560.0), Paint::Mesh(mesh)))
    };
    let band = MESH_BAND_PATCHES;
    let whole = single_layer_log(fill(mesh(band, 1))).expect("a full band encodes");
    assert_eq!(mesh_bands(&whole), [band]);
    let split = single_layer_log(fill(mesh(band + 1, 1))).expect("two bands encode");
    assert_eq!(mesh_bands(&split), [band, 1]);
    // A 256 × 256 vertex grid: 65536 vertices, 255 × 255 patches.
    let largest = single_layer_log(fill(mesh(255, 255))).expect("the largest grid encodes");
    let bands = mesh_bands(&largest);
    assert_eq!(bands.iter().sum::<u32>(), 255 * 255);
    assert!(
        bands[..bands.len() - 1]
            .iter()
            .all(|&patches| patches == band)
    );
}

#[test]
fn a_folded_mesh_patch_is_refused_naming_it() {
    let mut points = vec![
        Point::new(0.0, 0.0),
        Point::new(10.0, 0.0),
        Point::new(0.0, 10.0),
        Point::new(10.0, 10.0),
    ];
    points.swap(2, 3);
    let colors = vec![color(1.0, 0.0, 0.0, 1.0); 4];
    let picture = Picture::record(|r| {
        r.fill(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Paint::Mesh(MeshGradient::new(1, 1, points, colors)),
        );
    });
    let error = single_layer_log(picture).expect_err("a bow-tie patch has no bilinear inverse");
    assert!(
        error.to_string().contains("row 0, column 0 folds"),
        "{error}"
    );
}

#[test]
fn a_mesh_painted_stroke_masks_the_mesh_into_its_coverage() {
    let picture = Picture::record(|r| {
        r.stroke(
            Circle::new((5.0, 5.0), 4.0),
            Stroke::new(2.0),
            Paint::Mesh(mesh(1, 1).interpolation(MeshColorInterpolation::Smoothstep)),
        );
    });
    let log = single_layer_log(picture).expect("a mesh stroke encodes");
    let position = |prefix: &str| {
        log.iter()
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("no {prefix} in {log:#?}"))
    };
    // The mesh node composites SRC_IN onto the stroke's coverage.
    assert!(
        log.iter()
            .any(|line| line.starts_with("SetComposite") && line.contains(" blend=5 ")),
        "{log:#?}"
    );
    assert!(
        position("Mesh interpolation=1 ") < position("Stroke "),
        "{log:#?}"
    );
}

#[test]
fn a_tilt_animation_updates_node_camera_properties_without_re_recording() {
    let mut encoder = Encoder::new(34).expect("API 34 is supported");
    let root = encoder.tree().root();
    let card = LayerId::new(201);
    let perspective =
        waterui_graphics::draw::Projective::perspective(800.0).expect("the distance is valid");
    encoder
        .commit(changes(vec![
            layer(LayerOp::Create(card)),
            layer(LayerOp::Push {
                parent: root,
                child: card,
            }),
            layer(LayerOp::Pivot(card, prop(Vec2::new(40.0, 20.0)))),
            layer(LayerOp::Projection(card, perspective)),
            layer(LayerOp::Tilt(card, prop(Vec2::new(0.3, 0.0)))),
            layer(LayerOp::Content(
                card,
                Some(ContentOp::Replace(plain_picture())),
            )),
        ]))
        .expect("the card commits");
    encoder
        .frame(Instant::now(), 1.0)
        .expect("the first frame encodes");
    let first = encoder.take_log();
    let tilted = |log: &[String]| {
        log.iter()
            .filter(|line| line.starts_with("SetTransform "))
            .any(|line| !line.contains("rotation_x=0 ") || !line.contains("rotation_y=0 "))
    };
    assert!(tilted(&first), "{first:#?}");
    let projective_concat = |line: &&String| {
        line.starts_with("Concat ") && !line.ends_with(",00000000,00000000,3f800000]")
    };
    assert!(
        !first.iter().any(|line| projective_concat(&line)),
        "{first:#?}"
    );

    encoder
        .commit(changes(vec![layer(LayerOp::Tilt(
            card,
            prop(Vec2::new(0.5, -0.2)),
        ))]))
        .expect("the new tilt commits");
    encoder
        .frame(Instant::now(), 1.0)
        .expect("the second frame encodes");
    let second = encoder.take_log();
    assert!(tilted(&second), "{second:#?}");
    assert!(
        !second.iter().any(|line| line.starts_with("Record ")),
        "{second:#?}"
    );
}

#[test]
fn a_frame_writes_only_the_layers_an_op_or_a_running_animation_moved() {
    let mut encoder = Encoder::new(34).expect("API 34 is supported");
    let root = encoder.tree().root();
    let moving = LayerId::new(301);
    let still = LayerId::new(302);
    let mut ops = Vec::new();
    for id in [moving, still] {
        ops.push(layer(LayerOp::Create(id)));
        ops.push(layer(LayerOp::Push {
            parent: root,
            child: id,
        }));
        ops.push(layer(LayerOp::Content(
            id,
            Some(ContentOp::Replace(plain_picture())),
        )));
    }
    encoder.commit(changes(ops)).expect("the layers commit");
    let start = Instant::now();
    encoder.frame(start, 1.0).expect("the first frame encodes");
    encoder.take_log();
    let moving_node = encoder.node(moving).expect("the moving layer is mirrored");
    let still_node = encoder.node(still).expect("the still layer is mirrored");
    let names = |log: &[String], node: u32| {
        log.iter()
            .any(|line| line.split(' ').any(|field| field == format!("node={node}")))
    };

    encoder
        .commit(changes(vec![layer(LayerOp::Opacity(
            moving,
            Prop {
                target: 0.0,
                animation: Some(Curve::linear(Duration::from_secs(1)).into()),
            },
        ))]))
        .expect("the fade commits");
    encoder.frame(start, 1.0).expect("the fade starts");
    let mut fading = encoder.take_log();
    encoder
        .frame(start + Duration::from_millis(500), 1.0)
        .expect("the fade steps");
    fading.extend(encoder.take_log());
    assert!(
        fading
            .iter()
            .any(|line| line.starts_with(&format!("SetAlpha node={moving_node} "))),
        "{fading:#?}"
    );
    assert!(!names(&fading, still_node), "{fading:#?}");
    assert!(
        !fading.iter().any(|line| line.starts_with("Record ")),
        "{fading:#?}"
    );

    encoder
        .frame(start + Duration::from_secs(2), 1.0)
        .expect("the fade settles");
    let settled = encoder.take_log();
    assert!(!names(&settled, still_node), "{settled:#?}");
    encoder
        .frame(start + Duration::from_secs(3), 1.0)
        .expect("an idle frame encodes");
    let idle = encoder.take_log();
    assert!(!idle.iter().any(|line| line.contains("node=")), "{idle:#?}");
}

#[test]
fn a_child_blend_change_isolates_its_parent() {
    let mut encoder = Encoder::new(34).expect("API 34 is supported");
    let root = encoder.tree().root();
    let group = LayerId::new(311);
    let child = LayerId::new(312);
    encoder
        .commit(changes(vec![
            layer(LayerOp::Create(group)),
            layer(LayerOp::Push {
                parent: root,
                child: group,
            }),
            layer(LayerOp::Create(child)),
            layer(LayerOp::Push {
                parent: group,
                child,
            }),
            layer(LayerOp::Content(
                child,
                Some(ContentOp::Replace(plain_picture())),
            )),
        ]))
        .expect("the layers commit");
    encoder
        .frame(Instant::now(), 1.0)
        .expect("the first frame encodes");
    encoder.take_log();
    let group_node = encoder.node(group).expect("the group is mirrored");

    // Only the child's outer stamp moves; the group must still isolate.
    encoder
        .commit(changes(vec![layer(LayerOp::Blend(
            child,
            BlendMode::Multiply,
        ))]))
        .expect("the blend commits");
    encoder
        .frame(Instant::now(), 1.0)
        .expect("the blended frame encodes");
    let log = encoder.take_log();
    assert!(
        log.iter().any(|line| {
            line.starts_with(&format!("SetComposite node={group_node} "))
                && line.split(' ').any(|field| field == "layer=1")
        }),
        "{log:#?}"
    );
}
