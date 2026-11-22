//! Backdrop effect shaders on the CPU backend: a shader that restates a
//! built-in effect renders what the built-in renders, under every member
//! shape the composite supports.

#![cfg(all(feature = "shaders", not(target_arch = "wasm32")))]

use cherenkov::kurbo::{Affine, Point, Rect, RoundedRect};
use cherenkov::{
    BackdropEffect, BackdropShaderSource, BackdropSpec, BackdropUnion, CaptureLevels, CaptureScale,
    Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor,
};
use cherenkov_cpu::{Raster, RasterConfig};
use cherenkov_oracle::F32Image;
use cherenkov_oracle::metrics::assert_images_match;

const SIZE: (u32, u32) = (96, 80);

fn engine() -> Engine<Raster> {
    Engine::<Raster>::new(RasterConfig::default()).expect("engine")
}

/// A member's effect: a built-in one, or a shader with its uniforms.
#[derive(Clone)]
enum Effect {
    BuiltIn(BackdropEffect),
    Shader(&'static str, f32, Vec<f32>),
}

/// Renders a striped, wide-gamut backdrop under two rounded members of
/// one group captured per `spec`, each sampling with `effect`; the second
/// member is rotated so its field and local space are not axis-aligned.
#[expect(clippy::cast_precision_loss, reason = "stripe indices are small")]
fn render(engine: &Engine<Raster>, spec: BackdropSpec, effect: &Effect) -> F32Image {
    let surface = engine
        .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(spec);
    // The handle keeps the shader registered through the render.
    let shader = match effect {
        Effect::BuiltIn(_) => None,
        Effect::Shader(source, reach, _) => Some(
            engine
                .backdrop_shader(BackdropShaderSource::wgsl(*source).reach(*reach))
                .expect("the shader compiles"),
        ),
    };
    let sample = |_: &cherenkov::Surface<Raster>| match (effect, &shader) {
        (Effect::Shader(_, _, uniforms), Some(shader)) => {
            group.sample_with(shader.effect(uniforms.clone()))
        }
        (Effect::BuiltIn(effect), _) => group.sample_with(effect.clone()),
        (Effect::Shader(..), None) => unreachable!("a shader effect registered its shader"),
    };
    let (a, b) = (surface.layer(), surface.layer());
    surface.update(|tx| {
        tx[surface.root()]
            .push(&a)
            .push(&b)
            .content(surface.record(|r| {
                for i in 0..12 {
                    let x = f64::from(i) * 8.0;
                    r.fill(
                        Rect::new(x, 0.0, x + 8.0, 80.0),
                        WorkingColor::new([
                            0.08f32.mul_add(i as f32, 0.1),
                            if i % 2 == 0 { 1.4 } else { 0.2 },
                            0.07f32.mul_add(-(i as f32), 1.0),
                            1.0,
                        ]),
                    );
                }
                r.fill(
                    Rect::new(0.0, 36.0, 96.0, 44.0),
                    WorkingColor::new([0.0, 0.0, 0.0, 1.0]),
                );
            }));
        tx[&a]
            .clip(RoundedRect::new(10.0, 8.0, 50.0, 40.0, 10.0))
            .backdrop(sample(&surface));
        tx[&b]
            .transform(Affine::rotate_about(0.3, Point::new(66.0, 52.0)))
            .clip(RoundedRect::new(46.0, 36.0, 86.0, 68.0, 9.0))
            .backdrop(sample(&surface));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    F32Image {
        width: SIZE.0,
        height: SIZE.1,
        pixels: readback.pixels,
    }
}

fn assert_shader_matches(
    spec: BackdropSpec,
    built_in: BackdropEffect,
    shader: &Effect,
    what: &str,
) {
    let engine = engine();
    let expected = render(&engine, spec, &Effect::BuiltIn(built_in));
    let actual = render(&engine, spec, shader);
    let dir = std::env::temp_dir();
    expected
        .write_png(&dir.join(format!("cherenkov-cpu-backdrop-shader-{what}-builtin.png")))
        .expect("png");
    actual
        .write_png(&dir.join(format!("cherenkov-cpu-backdrop-shader-{what}.png")))
        .expect("png");
    assert_images_match(&expected, &actual);
}

const REFRACTION: &str = "
fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
    let t = clamp(1.0 + px.sdf / params[0].x, 0.0, 1.0);
    return backdrop_sample(px.p - px.normal * params[0].y * t * t);
}";

const RIM: &str = "
fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
    let t = clamp(1.0 + px.sdf / params[0].x, 0.0, 1.0);
    let c = backdrop_sample(px.p);
    return vec4<f32>(c.rgb + params[1].rgb * params[1].a * params[0].y * t * t, c.a);
}";

const LEVEL: &str = "
fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
    let t = clamp(1.0 + px.sdf / params[0].x, 0.0, 1.0);
    return backdrop_sample_level(px.p, params[0].z + (params[0].y - params[0].z) * t);
}";

fn refraction() -> (BackdropEffect, Effect) {
    (
        cherenkov::Refraction {
            depth: 9.0,
            strength: 6.0,
        }
        .into(),
        Effect::Shader(REFRACTION, 6.0, vec![9.0, 6.0, 0.0, 0.0]),
    )
}

#[test]
fn a_refraction_shader_matches_the_built_in_refraction() {
    let (built_in, shader) = refraction();
    assert_shader_matches(BackdropSpec::FULL, built_in, &shader, "refraction");
}

#[test]
fn a_refraction_shader_matches_on_a_reduced_capture() {
    let (built_in, shader) = refraction();
    let spec = BackdropSpec::new(CaptureScale::new(0.5).unwrap(), CaptureLevels::ONE);
    assert_shader_matches(spec, built_in, &shader, "refraction-reduced");
}

#[test]
fn a_refraction_shader_matches_under_a_union() {
    let (built_in, shader) = refraction();
    let spec = BackdropSpec::FULL.union(BackdropUnion::new(12.0).unwrap());
    assert_shader_matches(spec, built_in, &shader, "refraction-union");
}

#[test]
fn a_rim_shader_matches_the_built_in_rim() {
    let color = [1.0, 0.8, 0.3, 0.9];
    let built_in = cherenkov::Rim {
        width: 7.0,
        color,
        gain: 2.5,
    }
    .into();
    let mut uniforms = vec![7.0, 2.5, 0.0, 0.0];
    uniforms.extend(color);
    assert_shader_matches(
        BackdropSpec::FULL,
        built_in,
        &Effect::Shader(RIM, 0.0, uniforms),
        "rim",
    );
}

#[test]
fn a_level_shader_matches_the_built_in_level_ramp() {
    let spec = BackdropSpec::new(
        CaptureScale::new(0.5).unwrap(),
        CaptureLevels::new(4).unwrap(),
    );
    let built_in = cherenkov::LevelRamp::new(10.0, 3.0, 0.5).unwrap().into();
    assert_shader_matches(
        spec,
        built_in,
        &Effect::Shader(LEVEL, 0.0, vec![10.0, 3.0, 0.5, 0.0]),
        "level",
    );
}

/// `fwidth` sees quad neighbours: a shader that draws its own
/// half-pixel edge ramp from `px.sdf / fwidth(px.sdf)` covers the
/// interior fully, the exterior not at all, and ramps across the edge.
#[test]
fn derivatives_see_quad_neighbours() {
    let engine = engine();
    let image = render(
        &engine,
        BackdropSpec::FULL,
        &Effect::Shader(
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                let w = fwidth(px.sdf);
                return vec4<f32>(w, clamp(0.5 - px.sdf / w, 0.0, 1.0), 0.0, 1.0);
            }",
            0.0,
            vec![],
        ),
    );
    let at = |x: usize, y: usize| image.pixels[y * SIZE.0 as usize + x];
    // The axis-aligned member's distance grows one per pixel: fwidth is 1.
    let inside = at(30, 24);
    assert!((inside[0] - 1.0).abs() < 1e-4, "fwidth {inside:?}");
    assert!((inside[1] - 1.0).abs() < 1e-4, "interior {inside:?}");
}

#[test]
fn an_invalid_shader_is_rejected_at_registration() {
    let engine = engine();
    let error = engine
        .backdrop_shader(BackdropShaderSource::wgsl(
            "fn backdrop_effect(px: BackdropPixel {",
        ))
        .expect_err("invalid WGSL");
    assert!(
        matches!(error, cherenkov::ResourceError::Shader(_)),
        "{error}"
    );
    let error = engine
        .backdrop_shader(BackdropShaderSource::wgsl(
            "fn backdrop_effect(px: BackdropPixel) -> vec4<f32> { return vec4<f32>(0.0); }",
        ))
        .expect_err("wrong signature");
    assert!(
        matches!(error, cherenkov::ResourceError::Shader(_)),
        "{error}"
    );
}
