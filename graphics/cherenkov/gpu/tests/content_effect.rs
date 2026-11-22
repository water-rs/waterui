//! Content effect shaders on the GPU backend.

use std::error::Error;

use cherenkov::kurbo::{BezPath, Rect, RoundedRect};
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    ContentShaderSource, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, RecordingScale,
    RenderError, ResourceError, WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig};
use nami::{SignalExt, binding};

fn pixel(readback: &cherenkov::Readback, x: usize, y: usize) -> [f32; 4] {
    let p = &readback.pixels[y * readback.width as usize + x];
    [p[0], p[1], p[2], p[3]]
}

fn assert_pixel(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() <= tolerance,
            "pixel {actual:?}, expected {expected:?}"
        );
    }
}

split_test! {
fn invalid_content_shader_is_rejected_at_registration() -> Result<(), Box<dyn Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;

    let syntax = engine
        .content_shader(ContentShaderSource::wgsl(
            "fn content_effect(px: ContentPixel {",
        ))
        .expect_err("invalid WGSL syntax");
    assert!(matches!(syntax, ResourceError::Shader(_)), "{syntax}");

    let wrong_sig = engine
        .content_shader(ContentShaderSource::wgsl(
            "fn content_effect(px: ContentPixel) -> vec4<f32> { return vec4<f32>(0.0); }",
        ))
        .expect_err("wrong signature");
    assert!(matches!(wrong_sig, ResourceError::Shader(_)), "{wrong_sig}");

    let wrong_builtin = engine
        .content_shader(ContentShaderSource::wgsl(
            "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return backdrop_sample(px.p);
            }",
        ))
        .expect_err("backdrop_sample is not declared in content shaders");
    assert!(
        matches!(wrong_builtin, ResourceError::Shader(_)),
        "{wrong_builtin}"
    );
    Ok(())
}
}

split_test! {
fn unclipped_or_path_clipped_content_effect_is_unsupported()
-> Result<(), Box<dyn Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.content_shader(ContentShaderSource::wgsl(
        "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
            return content_sample(px.p);
        }",
    ))?;

    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content_effect(shader.effect(vec![]));
    });
    let err = wait!(engine.render(FrameTime::now())).expect_err("unclipped");
    assert!(
        matches!(err, RenderError::Unsupported("content-effect-sdf-path")),
        "{err}"
    );

    let mut path = BezPath::new();
    path.move_to((4.0, 4.0));
    path.line_to((28.0, 6.0));
    path.line_to((16.0, 28.0));
    path.close_path();
    surface.update(|tx| {
        tx[&layer].clip(path);
    });
    let err = wait!(engine.render(FrameTime::now())).expect_err("path clip");
    assert!(
        matches!(err, RenderError::Unsupported("content-effect-sdf-path")),
        "{err}"
    );
    Ok(())
}
}

split_test! {
fn content_effect_displaces_isolated_content_without_refracting_backdrop()
-> Result<(), Box<dyn Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.content_shader(
        ContentShaderSource::wgsl(
            "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return content_sample(px.p - vec2<f32>(params[0].x, 0.0));
            }",
        )
        .reach(12.0),
    )?;

    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let content = surface.layer();
    surface.update(|tx| {
        tx[surface.root()]
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 32.0, 64.0),
                    WorkingColor::new([0.0, 0.2, 0.0, 1.0]),
                );
                r.fill(
                    Rect::new(32.0, 0.0, 64.0, 64.0),
                    WorkingColor::new([0.0, 0.8, 0.0, 1.0]),
                );
            }))
            .push(&content);
        tx[&content]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .content_effect(shader.effect(vec![10.0]))
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(12.0, 16.0, 24.0, 48.0),
                    WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
                );
            }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;

    // Displaced red bar landed at x = 28.
    assert_pixel(pixel(&readback, 28, 32), [1.0, 0.0, 0.0, 1.0], 2e-3);
    // Out-of-bounds content_sample returns transparent, leaving dark-green backdrop undistorted.
    assert_pixel(pixel(&readback, 14, 32), [0.0, 0.2, 0.0, 1.0], 2e-3);
    // Right half outside displaced bar shows bright-green backdrop undistorted.
    assert_pixel(pixel(&readback, 44, 32), [0.0, 0.8, 0.0, 1.0], 2e-3);
    Ok(())
}
}

split_test! {
fn content_effect_receives_sdf_normal_scale_and_live_uniforms()
-> Result<(), Box<dyn Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.content_shader(
        ContentShaderSource::wgsl(
            "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                let c = content_sample(px.p);
                let rim = clamp(1.0 + px.sdf / 6.0, 0.0, 1.0);
                let gain = params[0].x * px.scale;
                return vec4<f32>(c.rgb + vec3<f32>(0.0, 0.0, gain * rim) * c.a, c.a);
            }",
        )
        .reach(4.0),
    )?;

    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let layer = surface.layer();
    let gain = binding(1.0_f32);
    let effect = gain.map(move |g| {
        shader
            .effect(vec![g])
            .scale(RecordingScale::new(2.0).expect("scale"))
    });
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer]
            .clip(RoundedRect::new(8.0, 8.0, 56.0, 56.0, 10.0))
            .content_effect(effect)
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(8.0, 8.0, 56.0, 56.0),
                    WorkingColor::new([0.5, 0.0, 0.0, 1.0]),
                );
            }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let rb1 = wait!(surface.readback())?;
    let edge1 = pixel(&rb1, 10, 32);
    assert!(edge1[2] > 1.0, "expected blue rim lift, got {edge1:?}");
    assert_pixel(pixel(&rb1, 32, 32), [0.5, 0.0, 0.0, 1.0], 5e-3);

    gain.set(0.0);
    wait!(engine.render(FrameTime::now()))?;
    let rb2 = wait!(surface.readback())?;
    assert_pixel(pixel(&rb2, 10, 32), [0.5, 0.0, 0.0, 1.0], 5e-3);
    Ok(())
}
}
