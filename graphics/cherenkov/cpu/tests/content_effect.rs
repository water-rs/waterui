//! Content effect shaders on the CPU backend.

#![cfg(all(feature = "shaders", not(target_arch = "wasm32")))]

use cherenkov::kurbo::{BezPath, Rect, RoundedRect};
use cherenkov::{
    ContentShaderSource, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, RecordingScale,
    RenderError, ResourceError, WorkingColor,
};
use cherenkov_cpu::{Raster, RasterConfig};
use nami::{SignalExt, binding};

const SIZE: (u32, u32) = (96, 80);

fn engine() -> Engine<Raster> {
    Engine::<Raster>::new(RasterConfig::default()).expect("engine")
}

fn pixel(readback: &cherenkov::Readback, x: usize, y: usize) -> [f32; 4] {
    readback.pixels[y * readback.width as usize + x]
}

fn assert_pixel(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() <= tolerance,
            "pixel {actual:?}, expected {expected:?}"
        );
    }
}

#[test]
fn invalid_content_shader_is_rejected_at_registration() {
    let engine = engine();

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
}

#[test]
fn unclipped_or_path_clipped_content_effect_is_unsupported() {
    let engine = engine();
    let shader = engine
        .content_shader(ContentShaderSource::wgsl(
            "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return content_sample(px.p);
            }",
        ))
        .expect("shader");

    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content_effect(shader.effect(vec![]));
    });
    let err = engine.render(FrameTime::now()).expect_err("unclipped");
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
    let err = engine.render(FrameTime::now()).expect_err("path clip");
    assert!(
        matches!(err, RenderError::Unsupported("content-effect-sdf-path")),
        "{err}"
    );
}

#[test]
fn content_effect_displaces_isolated_content_without_refracting_backdrop() {
    let engine = engine();
    let shader = engine
        .content_shader(
            ContentShaderSource::wgsl(
                "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                    return content_sample(px.p - vec2<f32>(params[0].x, 0.0));
                }",
            )
            .reach(12.0),
        )
        .expect("shader");

    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");

    assert_pixel(pixel(&readback, 28, 32), [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 14, 32), [0.0, 0.2, 0.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 44, 32), [0.0, 0.8, 0.0, 1.0], 1e-3);
}

#[test]
fn content_effect_receives_sdf_normal_scale_and_live_uniforms() {
    let engine = engine();
    let shader = engine
        .content_shader(
            ContentShaderSource::wgsl(
                "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                    let c = content_sample(px.p);
                    let rim = clamp(1.0 + px.sdf / 6.0, 0.0, 1.0);
                    let gain = params[0].x * px.scale;
                    return vec4<f32>(c.rgb + vec3<f32>(0.0, 0.0, gain * rim) * c.a, c.a);
                }",
            )
            .reach(4.0),
        )
        .expect("shader");

    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render 1");
    let rb1 = surface.readback().expect("readback 1");
    let edge1 = pixel(&rb1, 10, 32);
    assert!(edge1[2] > 1.0, "expected blue rim lift, got {edge1:?}");
    assert_pixel(pixel(&rb1, 32, 32), [0.5, 0.0, 0.0, 1.0], 1e-3);

    gain.set(0.0);
    engine.render(FrameTime::now()).expect("render 2");
    let rb2 = surface.readback().expect("readback 2");
    assert_pixel(pixel(&rb2, 10, 32), [0.5, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn content_effect_vertical_displacement_crosses_cpu_band_seams_cleanly() {
    let engine = engine();
    let shader = engine
        .content_shader(
            ContentShaderSource::wgsl(
                "fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                    return content_sample(px.p - vec2<f32>(0.0, params[0].x));
                }",
            )
            .reach(12.0),
        )
        .expect("shader");

    let surface = engine
        .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()]
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, f64::from(SIZE.0), f64::from(SIZE.1)),
                    WorkingColor::new([0.05, 0.08, 0.12, 1.0]),
                );
            }))
            .push(&layer);
        // Horizontal stripe in band 0 (y = 8..16) shifted downward by +10 px into
        // band 1 (y = 18..26), crossing the 16-row band seam cleanly via the apron.
        tx[&layer]
            .clip(RoundedRect::new(8.0, 4.0, 88.0, 76.0, 8.0))
            .content_effect(shader.effect(vec![10.0]))
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(16.0, 8.0, 80.0, 16.0),
                    WorkingColor::new([0.9, 0.3, 0.1, 1.0]),
                );
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let rb = surface.readback().expect("readback");
    // Shifted stripe lands in band 1 at y = 22.
    assert_pixel(pixel(&rb, 48, 22), [0.9, 0.3, 0.1, 1.0], 1e-3);
    // Original stripe position at y = 12 samples y = 2 (above clip y = 4 -> transparent),
    // revealing the background.
    assert_pixel(pixel(&rb, 48, 12), [0.05, 0.08, 0.12, 1.0], 1e-3);
}
