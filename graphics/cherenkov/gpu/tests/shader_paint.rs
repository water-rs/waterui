//! Shader paints inherit geometry, clipping and engine frame timing.

use cherenkov::Instant;
use cherenkov::kurbo::{Circle, Rect};
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Draw, Engine, FrameTime, Next, Offscreen, OffscreenFormat, ResourceError, ShaderPaint,
    ShaderSource,
};
use cherenkov_gpu::{Gpu, GpuConfig};
use std::time::Duration;

split_test! {
fn shader_registration_rejects_invalid_source() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    assert!(matches!(
        engine.shader(ShaderSource::wgsl("invalid shader")),
        Err(ResourceError::Shader(_))
    ));
    Ok(())
}
}

split_test! {
fn shader_paint_uses_shape_coverage_and_presentation_time() -> Result<(), Box<dyn std::error::Error>>
{
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader =
        engine.shader(ShaderSource::wgsl(include_str!("shaders/paint.wgsl")).animated())?;
    let surface = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16)))?;
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
        wait!(engine.render(FrameTime::at(start)))?,
        Next::At { .. }
    ));
    wait!(engine.render(FrameTime::at(start + Duration::from_millis(500))))?;
    let pixels = wait!(surface.readback())?.pixels;
    assert!(
        (pixels[8 * 16 + 5][0] - 0.5).abs() < 0.001,
        "time advances without rerecording"
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
        wait!(engine.render(FrameTime::at(start + Duration::from_secs(1))))?,
        Next::Idle
    );
    Ok(())
}
}

split_test! {
fn producer_color_helpers_match_working_space_and_alpha() -> Result<(), Box<dyn std::error::Error>>
{
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.shader(ShaderSource::wgsl(include_str!("shaders/srgb.wgsl")))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 4), OffscreenFormat::LinearF16)))?;
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

split_test! {
fn live_shader_operands_keep_other_cached_texture_uses() -> Result<(), Box<dyn std::error::Error>> {
    use nami::SignalExt as _;
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.shader(ShaderSource::wgsl(
        "@fragment fn main() -> @location(0) vec4<f32> { return params[0]; }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((16, 8), OffscreenFormat::LinearF16)))?;
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

split_test! {
fn clipped_paths_keep_full_geometry_shader_coordinates() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.shader(ShaderSource::wgsl("@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv.x, 0.0, 0.0, 1.0); }"))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
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

split_test! {
fn removed_shader_reports_an_error_instead_of_panicking() -> Result<(), Box<dyn std::error::Error>>
{
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.shader(ShaderSource::wgsl(
        "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
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

split_test! {
fn shader_strokes_and_degenerate_geometry_keep_ordinary_coverage()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.shader(ShaderSource::wgsl(
        "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16)))?;
    for geometry in 0..3 {
        let mut outputs = Vec::new();
        for paint in [
            cherenkov::Paint::from(cherenkov::WorkingColor::WHITE),
            cherenkov::Paint::from(ShaderPaint {
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
            outputs.push(
                wait!(surface
                    .readback())?
                    .pixels
                    .into_iter()
                    .map(|pixel| pixel.map(f32::to_bits))
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(outputs[0], outputs[1], "geometry {geometry}");
    }
    Ok(())
}
}

split_test! {
fn shader_stroke_coordinates_include_the_outline() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.shader(ShaderSource::wgsl("@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv.x, 0.0, 0.0, 1.0); }"))?;
    let surface = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16)))?;
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
