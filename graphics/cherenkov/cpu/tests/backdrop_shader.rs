//! Backdrop effect shaders on the CPU backend: a shader that restates a
//! built-in effect renders what the built-in renders, under every member
//! shape the composite supports.

#![cfg(all(feature = "shaders", not(target_arch = "wasm32")))]

use cherenkov::kurbo::{Affine, Point, Rect, RoundedRect};
use cherenkov::{
    BackdropEffect, BackdropOuter, BackdropShaderSource, BackdropSpec, BackdropUnion,
    CaptureLevels, CaptureScale, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Readback,
    WorkingColor,
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

#[test]
fn backdrop_sibling_reads_adjacent_emitting_member_sdf_normal_and_emission() {
    let engine = engine();
    let shader = engine
        .backdrop_shader(BackdropShaderSource::wgsl(
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                let n = backdrop_sibling_count();
                let self_idx = backdrop_sibling_index();
                let oob = backdrop_sibling(n, px.p);
                if n != 2u || oob.sdf != 0.0 || oob.normal.x != 0.0 || oob.normal.y != 0.0
                    || any(oob.emission != vec4<f32>(0.0)) {
                    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
                }
                for (var i = 0u; i < n; i = i + 1u) {
                    if i == self_idx {
                        continue;
                    }
                    let sib = backdrop_sibling(i, px.p);
                    if params[0].x > 0.5 {
                        return vec4<f32>(sib.emission.rgb, max(sib.emission.a, 1.0));
                    }
                    if abs(sib.emission.a - select(1.0, 0.0, self_idx == 1u)) > 0.001 {
                        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
                    }
                    return vec4<f32>(
                        sib.sdf / 20.0,
                        sib.normal.x * 0.5 + 0.5,
                        sib.normal.y * 0.5 + 0.5,
                        1.0,
                    );
                }
                return vec4<f32>(1.0, 0.0, 0.0, 1.0);
            }",
        ))
        .expect("shader");
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        BackdropSpec::FULL.union(BackdropUnion::new(20.0).expect("union")),
    );
    let a = surface.layer();
    let b = surface.layer();
    let emit = [0.25, 0.5, 0.75, 1.0];
    let set_mode = |mode: f32| {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 48.0, 32.0),
                    WorkingColor::new([0.0, 0.0, 0.0, 0.0]),
                );
            }));
            tx[surface.root()].push(&a);
            tx[&a]
                .clip(Rect::new(4.0, 8.0, 20.0, 24.0))
                .backdrop(group.sample_with(shader.effect(vec![mode])));
            tx[surface.root()].push(&b);
            tx[&b]
                .clip(Rect::new(26.0, 8.0, 42.0, 24.0))
                .backdrop(group.sample_with(shader.effect(vec![mode])).emission(emit));
        });
        engine.render(FrameTime::now()).expect("render");
        surface.readback().expect("readback")
    };
    let sdf_readback = set_mode(0.0);
    let px = |rb: &Readback, x: usize, y: usize| rb.pixels[y * rb.width as usize + x];
    let pa = px(&sdf_readback, 16, 16);
    assert!(pa[0].mul_add(20.0, -9.5).abs() <= 0.01);
    assert!((pa[1].mul_add(2.0, -1.0) + 1.0).abs() <= 0.01);
    assert!(pa[2].mul_add(2.0, -1.0).abs() <= 0.01);
    let pb = px(&sdf_readback, 34, 16);
    assert!(pb[0].mul_add(20.0, -14.5).abs() <= 0.01);
    assert!((pb[1].mul_add(2.0, -1.0) - 1.0).abs() <= 0.01);
    assert!(pb[2].mul_add(2.0, -1.0).abs() <= 0.01);

    let emit_readback = set_mode(1.0);
    for (c, (actual, expected)) in px(&emit_readback, 16, 16)[..3]
        .iter()
        .zip(&emit[..3])
        .enumerate()
    {
        assert!(
            (actual - expected).abs() <= 0.002,
            "channel {c}: {actual} vs {expected}"
        );
    }
    for (c, actual) in px(&emit_readback, 34, 16)[..3].iter().enumerate() {
        assert!(actual.abs() <= 0.002, "channel {c}: {actual}");
    }
}

#[test]
fn backdrop_sibling_non_union_and_non_emitting_union_return_zero() {
    let engine = engine();
    let non_union_shader = engine
        .backdrop_shader(BackdropShaderSource::wgsl(
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                let s = backdrop_sibling(0u, px.p);
                if backdrop_sibling_count() == 0u && backdrop_sibling_index() == 0u
                    && s.sdf == 0.0 && all(s.normal == vec2<f32>(0.0))
                    && all(s.emission == vec4<f32>(0.0)) {
                    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
                }
                return vec4<f32>(1.0, 0.0, 0.0, 1.0);
            }",
        ))
        .expect("shader");
    let union_shader = engine
        .backdrop_shader(BackdropShaderSource::wgsl(
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                if backdrop_sibling_count() != 2u || backdrop_sibling_index() != 0u {
                    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
                }
                let sib = backdrop_sibling(1u, px.p);
                if sib.sdf > 0.0 && all(sib.emission == vec4<f32>(0.0)) {
                    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
                }
                return vec4<f32>(1.0, 0.0, 0.0, 1.0);
            }",
        ))
        .expect("shader");
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let solo_group = surface.backdrop_group_unfiltered(BackdropSpec::FULL);
    let union_group = surface.backdrop_group_unfiltered(
        BackdropSpec::FULL.union(BackdropUnion::new(20.0).expect("union")),
    );
    let solo = surface.layer();
    let u0 = surface.layer();
    let u1 = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 32.0),
                WorkingColor::new([0.0, 0.0, 0.0, 0.0]),
            );
        }));
        tx[surface.root()].push(&solo);
        tx[&solo].clip(Rect::new(4.0, 4.0, 16.0, 16.0)).backdrop(
            solo_group
                .sample_with(non_union_shader.effect(Vec::new()))
                .outer(BackdropOuter::new(2.0).expect("outer")),
        );
        tx[surface.root()].push(&u0);
        tx[&u0]
            .clip(Rect::new(4.0, 20.0, 20.0, 30.0))
            .backdrop(union_group.sample_with(union_shader.effect(Vec::new())));
        tx[surface.root()].push(&u1);
        tx[&u1]
            .clip(Rect::new(26.0, 20.0, 42.0, 30.0))
            .backdrop(union_group.sample());
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    let px = |x: usize, y: usize| readback.pixels[y * readback.width as usize + x];
    assert!((px(10, 10)[1] - 1.0).abs() <= 0.002 && px(10, 10)[0].abs() <= 0.002);
    assert!((px(12, 25)[1] - 1.0).abs() <= 0.002 && px(12, 25)[0].abs() <= 0.002);
}
