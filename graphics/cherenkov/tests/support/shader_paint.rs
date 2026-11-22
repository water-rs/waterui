//! Shader paints inherit geometry, clipping, paint transforms and engine
//! frame timing — the contract every backend that draws them keeps.

use cherenkov::Instant;
use cherenkov::kurbo::{Affine, Circle, Rect};
use cherenkov::{__engine_fn as split_fn, __engine_wait as wait};
use cherenkov::{
    Backend, Draw, Engine, FrameTime, Next, Offscreen, OffscreenFormat, Paint, ResourceError,
    ShaderPaint, ShaderPaintCapability, ShaderSource,
};
use cherenkov_oracle::image::F32Image;
use cherenkov_oracle::metrics::{assert_close, assert_images_match};
use std::time::Duration;

/// `params[0].rgb + (time, uv) * params[1].xyz`, `params[0].a` alpha.
const PAINT: &str = include_str!("shaders/paint.wgsl");
/// The producer colour helpers on either half of the texture.
const SRGB: &str = include_str!("shaders/srgb.wgsl");
/// Red is `uv.x`.
const UV_X: &str = "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv.x, 0.0, 0.0, 1.0); }";
/// Opaque white.
const WHITE: &str = "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }";

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// A readback as an image for the visual-equivalence comparison.
fn image(readback: cherenkov::Readback) -> F32Image {
    F32Image {
        width: readback.width,
        height: readback.height,
        pixels: readback.pixels,
    }
}

split_fn! {
pub fn rejects_invalid_source<B: Backend + ShaderPaintCapability>(config: B::Config) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    assert!(matches!(
        engine.shader(ShaderSource::wgsl("invalid shader")),
        Err(ResourceError::Shader(_))
    ));
    Ok(())
}
}

split_fn! {
pub fn coverage_and_presentation_time<B: Backend + ShaderPaintCapability>(
    config: B::Config,
) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(PAINT).animated())?;
    let surface = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16), || {}))?;
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer]
            .clip(Rect::new(0.0, 0.0, 8.0, 16.0))
            .content(surface.record(|r| {
                r.fill(
                    Circle::new((8.0, 8.0), 6.0),
                    ShaderPaint {
                        shader: shader.id(),
                        uniforms: vec![0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
                    },
                );
            }));
    });
    let start = Instant::now();
    assert!(matches!(
        wait!(engine.render(FrameTime::unpresented_at(start)))?,
        Next::At { .. }
    ));
    wait!(engine.render(FrameTime::unpresented_at(start + Duration::from_millis(500))))?;
    let pixels = wait!(surface.readback())?.pixels;
    assert!(
        (pixels[8 * 16 + 5][0] - 0.5).abs() < 0.001,
        "time advances without rerecording: {:?}",
        pixels[8 * 16 + 5]
    );
    assert!(
        pixels[2 * 16 + 2][3].abs() < 0.001,
        "shape excludes bounding-box corner"
    );
    assert!(
        pixels[8 * 16 + 10][3].abs() < 0.001,
        "clip applies to shader paint"
    );
    drop(layer);
    assert_eq!(
        wait!(engine.render(FrameTime::unpresented_at(start + Duration::from_secs(1))))?,
        Next::Idle
    );
    Ok(())
}
}

split_fn! {
pub fn producer_color_helpers<B: Backend + ShaderPaintCapability>(config: B::Config) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(SRGB))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 4), OffscreenFormat::LinearF16), || {}))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 8.0, 4.0),
                ShaderPaint {
                    shader: shader.id(),
                    uniforms: vec![],
                },
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let expected = cherenkov::WorkingColor::from(cherenkov::Color::<cherenkov::Srgb>::new([
        1.0, 0.5, 0.25, 0.5,
    ]));
    let pixels = wait!(surface.readback())?.pixels;
    for index in [9, 14] {
        let [r, g, b, a] = expected.components;
        for (actual, expected) in pixels[index].into_iter().zip([r * a, g * a, b * a, a]) {
            assert!(
                (actual - expected).abs() < 0.001,
                "color conversion: {actual} != {expected}"
            );
        }
    }
    Ok(())
}
}

split_fn! {
pub fn live_operands_keep_other_uses<B: Backend + ShaderPaintCapability>(
    config: B::Config,
) -> Outcome {
    use nami::SignalExt as _;
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(
        "@fragment fn main() -> @location(0) vec4<f32> { return params[0]; }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((16, 8), OffscreenFormat::LinearF16), || {}))?;
    let value = nami::binding([1.0_f32, 0.0, 0.0, 1.0]);
    let id = shader.id();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|recorder| {
            recorder.fill(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                value.map(move |color: [f32; 4]| ShaderPaint {
                    shader: id,
                    uniforms: color.to_vec(),
                }),
            );
            recorder.fill(
                Rect::new(8.0, 0.0, 16.0, 8.0),
                ShaderPaint {
                    shader: id,
                    uniforms: vec![0.0, 0.0, 1.0, 1.0],
                },
            );
        }));
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    value.set([0.0, 1.0, 0.0, 1.0]);
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(engine.stats().commands_lowered, 1);
    let pixels = wait!(surface.readback())?.pixels;
    assert!((pixels[4 * 16 + 4][1] - 1.0).abs() < 0.001);
    assert!((pixels[4 * 16 + 12][2] - 1.0).abs() < 0.001);
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(engine.stats().commands_lowered, 0);
    Ok(())
}
}

split_fn! {
pub fn clipped_paths_keep_full_geometry<B: Backend + ShaderPaintCapability>(
    config: B::Config,
) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(UV_X))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {}))?;
    let mut path = cherenkov::kurbo::BezPath::new();
    path.move_to((-8.0, 0.0));
    path.line_to((8.0, 0.0));
    path.line_to((8.0, 8.0));
    path.line_to((-8.0, 8.0));
    path.close_path();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|recorder| {
            recorder.fill(
                path,
                ShaderPaint {
                    shader: shader.id(),
                    uniforms: vec![],
                },
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let pixel = wait!(surface.readback())?.pixels[4 * 8 + 4];
    assert!(
        (pixel[0] - 12.5 / 16.0).abs() < 0.003,
        "full-path coordinate: {pixel:?}"
    );
    Ok(())
}
}

split_fn! {
pub fn removed_shader_is_an_error<B: Backend + ShaderPaintCapability>(
    config: B::Config,
) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(WHITE))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {}))?;
    let id = shader.id();
    drop(shader);
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                ShaderPaint {
                    shader: id,
                    uniforms: vec![],
                },
            );
        }));
    });
    assert!(
        matches!(wait!(engine.render(FrameTime::now())), Err(cherenkov::RenderError::Render(message)) if message.contains("unregistered shader"))
    );
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                cherenkov::WorkingColor::WHITE,
            );
        }));
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    Ok(())
}
}

split_fn! {
pub fn strokes_and_degenerate_geometry_keep_coverage<B: Backend + ShaderPaintCapability>(
    config: B::Config,
) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(WHITE))?;
    let surface = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16), || {}))?;
    for geometry in 0..3 {
        let mut outputs = Vec::new();
        for paint in [
            Paint::from(cherenkov::WorkingColor::WHITE),
            Paint::from(ShaderPaint {
                shader: shader.id(),
                uniforms: vec![],
            }),
        ] {
            surface.update(|tx| {
                tx[surface.root()].content(surface.record(|r| match geometry {
                    0 => r.stroke(
                        cherenkov::kurbo::Line::new((2.0, 8.0), (14.0, 8.0)),
                        cherenkov::Stroke::new(2.0),
                        paint,
                    ),
                    1 => r.fill(Rect::new(8.0, 2.0, 8.0, 14.0), paint),
                    _ => r.fill(cherenkov::kurbo::BezPath::new(), paint),
                }));
            });
            wait!(engine
                .render(FrameTime::now()))
                .map_err(|error| format!("geometry {geometry}: {error}"))?;
            outputs.push(image(wait!(surface.readback())?));
        }
        assert_images_match(&outputs[0], &outputs[1]);
    }
    Ok(())
}
}

split_fn! {
pub fn stroke_coordinates_include_the_outline<B: Backend + ShaderPaintCapability>(
    config: B::Config,
) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(UV_X))?;
    let surface = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16), || {}))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.stroke(
                Rect::new(4.0, 4.0, 12.0, 12.0),
                cherenkov::Stroke::new(4.0),
                ShaderPaint {
                    shader: shader.id(),
                    uniforms: vec![],
                },
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let pixel = wait!(surface.readback())?.pixels[8 * 16 + 3];
    assert!(
        (pixel[0] - 1.5 / 12.0).abs() < 0.002,
        "complete stroke coordinates: {pixel:?}"
    );
    Ok(())
}
}

split_fn! {
pub fn glyph_runs_span_their_ink<B: Backend + ShaderPaintCapability>(config: B::Config) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(UV_X))?;
    let font = engine.font(cherenkov::FontSource::bytes(std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scenes/fonts/NotoSans.ttf"
    ))?))?;
    let surface = wait!(engine.surface(Offscreen::new((96, 48), OffscreenFormat::LinearF32), || {}))?;
    let run = cherenkov::GlyphRun {
        font: font.id(),
        size: 32.0,
        coords: Vec::new().into(),
        glyphs: [8.0_f32, 50.0]
            .map(|x| cherenkov::Glyph {
                id: 36,
                x,
                y: 36.0,
                transform: None,
            })
            .to_vec()
            .into(),
        style: cherenkov::GlyphStyle::Fill,
    };
    let mut renders = Vec::new();
    for paint in [
        Paint::from(cherenkov::WorkingColor::WHITE),
        Paint::from(ShaderPaint {
            shader: shader.id(),
            uniforms: vec![],
        }),
    ] {
        let run = run.clone();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| r.glyphs(run, paint)));
        });
        wait!(engine.render(FrameTime::now()))?;
        renders.push(wait!(surface.readback())?.pixels);
    }
    // The shader leaves the glyph masks' coverage alone.
    let alpha = |pixels: &[[f32; 4]]| pixels.iter().map(|p| [p[3]; 4]).collect::<Vec<_>>();
    let as_image = |pixels| F32Image {
        width: 96,
        height: 48,
        pixels,
    };
    assert_images_match(&as_image(alpha(&renders[0])), &as_image(alpha(&renders[1])));
    // uv.x spans the run's ink: the leftmost inked column reads near 0,
    // the rightmost near 1.
    let inked: Vec<usize> = (0..96)
        .filter(|&x| (0..48).any(|y| renders[0][y * 96 + x][3] > 0.5))
        .collect();
    let red = |x: usize| {
        let (sum, count) = (0..48)
            .map(|y| renders[1][y * 96 + x])
            .filter(|p| p[3] > 0.5)
            .fold((0.0_f32, 0.0_f32), |(sum, count), p| {
                (sum + p[0] / p[3], count + 1.0)
            });
        sum / count
    };
    let (first, last) = (inked[0], inked[inked.len() - 1]);
    assert!(red(first) < 0.08, "leftmost ink reads uv.x {}", red(first));
    assert!(red(last) > 0.92, "rightmost ink reads uv.x {}", red(last));
    Ok(())
}
}

split_fn! {
pub fn paint_transform_moves_sampling_not_geometry<B: Backend + ShaderPaintCapability>(
    config: B::Config,
) -> Outcome {
    let engine = wait!(Engine::<B>::new(config))?;
    let shader = engine.shader(ShaderSource::wgsl(PAINT))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(8.0, 8.0, 24.0, 24.0),
                Paint::from(ShaderPaint {
                    shader: shader.id(),
                    uniforms: vec![0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
                })
                .transformed(Affine::translate((4.0, 0.0))),
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let sample = pixels[18 * 32 + 18];
    assert!(
        (sample[1] - 0.40625).abs() < 0.002,
        "translated shader x: {sample:?}"
    );
    assert!(
        (sample[2] - 0.65625).abs() < 0.002,
        "shader y unchanged: {sample:?}"
    );
    // The paint moved, the coverage did not.
    assert_close(&[pixels[16 * 32 + 27][3]], &[0.0]);
    Ok(())
}
}
