//! The oracle's user-shader and described-filter paths against the built-in
//! paints, effects and filters they restate.

use cherenkov_oracle::shader::{BackdropModule, BackdropPixel, BackdropReads, params};
use cherenkov_oracle::{F32Image, Renderer, metrics};
use cherenkov_scene::{
    BackdropEffectSpec, BackdropFilter, BackdropGroup, Color, ColorSpace, Extend, GradientStop,
    LayerBuilder, LayerFilter, LinearGradient, Paint, Scene, SceneBuilder, ShaderPaint, Shape,
};
use kurbo::{Affine, Point, Rect, RoundedRect};

const SIZE: u32 = 64;

fn render(scene: &Scene) -> F32Image {
    Renderer::new(SIZE as usize, SIZE as usize)
        .render(
            scene,
            &std::env::temp_dir().join("cherenkov-oracle-shader-tests"),
        )
        .expect("oracle render")
}

fn scene(build: impl FnOnce(&mut SceneBuilder)) -> Scene {
    let mut b = Scene::builder(SIZE, SIZE).clear(Color::srgb(0.95, 0.95, 0.95));
    build(&mut b);
    b.build()
}

const fn linear_p3(r: f32, g: f32, b: f32) -> Color {
    Color::new(ColorSpace::LinearP3, [r, g, b, 1.0])
}

/// A rounded rect and, under a rotation, a second one, both filled with
/// `paint`'s function of their own bounds.
fn two_fills(l: &mut LayerBuilder<'_>, paint: impl Fn(Rect) -> Paint) {
    let first = Rect::new(6.0, 4.0, 58.0, 28.0);
    l.fill(
        Shape::RoundedRect(RoundedRect::from_rect(first, 6.0)),
        paint(first),
    );
    let second = Rect::new(12.0, 34.0, 52.0, 60.0);
    l.layer(|m| {
        m.transform(Affine::rotate_about(0.4, second.center()));
        m.fill(
            Shape::RoundedRect(RoundedRect::from_rect(second, 6.0)),
            paint(second),
        );
    });
}

/// A shader paint spans its fill's user-space bounds: one that ramps across
/// `uv.x` is the linear gradient across the bounds, rotated with the shape.
#[test]
fn a_shader_paint_spans_its_fill_bounds() {
    let (from, to) = (linear_p3(1.4, 0.1, 0.0), linear_p3(0.0, 0.3, 1.0));
    let shader = scene(|b| {
        two_fills(&mut b.root(), |_| {
            Paint::Shader(ShaderPaint {
                source: "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> \
                         { return mix(params[0], params[1], uv.x); }"
                    .to_owned(),
                params: vec![1.4, 0.1, 0.0, 1.0, 0.0, 0.3, 1.0, 1.0],
            })
        });
    });
    let gradient = scene(|b| {
        two_fills(&mut b.root(), |bounds| {
            Paint::Linear(LinearGradient {
                start: Point::new(bounds.x0, bounds.y0),
                end: Point::new(bounds.x1, bounds.y0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: from,
                    },
                    GradientStop {
                        offset: 1.0,
                        color: to,
                    },
                ],
                extend: Extend::Pad,
                interpolation: ColorSpace::LinearP3,
            })
        });
    });
    metrics::assert_images_match(&render(&shader), &render(&gradient));
}

/// Striped backdrop under one rounded member sampling group 1 with
/// `effect`.
fn refraction_scene(effect: BackdropEffectSpec) -> Scene {
    scene(|b| {
        b.backdrop_group(BackdropGroup::new(
            1,
            vec![BackdropFilter::GaussianBlur { sigma: 1.5 }],
            1.0,
            1,
        ));
        let l = &mut b.root();
        for i in 0..8 {
            let x0 = 8.0 * f64::from(i);
            l.fill(
                Shape::rect(x0, 0.0, 4.0, 64.0),
                Paint::Solid(linear_p3(0.0, 0.2, 1.0)),
            );
            l.fill(
                Shape::rect(x0 + 4.0, 0.0, 4.0, 64.0),
                Paint::Solid(linear_p3(1.2, 0.1, 0.0)),
            );
        }
        l.layer(|m| {
            m.clip(Shape::RoundedRect(RoundedRect::new(
                10.0, 12.0, 54.0, 50.0, 10.0,
            )));
            m.backdrop(1);
            m.backdrop_effect(effect);
        });
    })
}

/// A backdrop shader restating the built-in refraction through the
/// contract's `BackdropPixel` and `backdrop_sample` renders it.
#[test]
fn a_backdrop_shader_restating_refraction_matches_it() {
    let shader = refraction_scene(BackdropEffectSpec::Shader {
        source: "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                     let t = clamp(1.0 + px.sdf / params[0].x, 0.0, 1.0);
                     return backdrop_sample(px.p - px.normal * (params[0].y * t * t));
                 }"
        .to_owned(),
        params: vec![9.0, 5.0],
        reach: 5.0,
    });
    let built_in = refraction_scene(BackdropEffectSpec::Refraction {
        depth: 9.0,
        strength: 5.0,
    });
    metrics::assert_images_match(&render(&shader), &render(&built_in));
}

/// A 3×3 convolution of equal ninths is the radius-1 box blur.
#[test]
fn a_mean_convolution_is_the_radius_one_box_blur() {
    let blurred = |filter: LayerFilter| {
        render(&scene(|b| {
            b.root().layer(|l| {
                l.filter(filter);
                l.fill(
                    Shape::rect(8.0, 8.0, 30.0, 20.0),
                    Paint::Solid(linear_p3(1.5, 0.2, 0.0)),
                );
                l.fill(
                    Shape::circle(40.0, 40.0, 14.0),
                    Paint::Solid(linear_p3(0.0, 1.0, 0.1)),
                );
            });
        }))
    };
    let convolution = blurred(LayerFilter::Convolution3x3 {
        kernel: [1.0 / 9.0; 9],
    });
    let box_blur = blurred(LayerFilter::BoxBlur { radius: 1.0 });
    metrics::assert_close(
        convolution.pixels.as_flattened(),
        box_blur.pixels.as_flattened(),
    );
}

#[test]
fn a_backdrop_shader_reads_union_sibling_geometry() {
    let s = scene(|b| {
        let mut group = BackdropGroup::new(1, vec![], 1.0, 1);
        group.union = Some(20.0);
        b.backdrop_group(group);
        let l = &mut b.root();
        l.layer(|m| {
            m.clip(Shape::Rect(Rect::new(4.0, 8.0, 20.0, 24.0)));
            m.backdrop(1);
            m.backdrop_effect(BackdropEffectSpec::Shader {
                source: "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                    if backdrop_sibling_count() != 2u || backdrop_sibling_index() != 0u {
                        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
                    }
                    let sib = backdrop_sibling(1u, px.p);
                    return vec4<f32>(sib.sdf / 20.0, sib.normal.x * 0.5 + 0.5, sib.normal.y * 0.5 + 0.5, 1.0);
                }"
                .to_owned(),
                params: vec![],
                reach: 0.0,
            });
        });
        l.layer(|m| {
            m.clip(Shape::Rect(Rect::new(26.0, 8.0, 42.0, 24.0)));
            m.backdrop(1);
        });
    });
    let img = render(&s);
    let p = img.pixels[16 * SIZE as usize + 16];
    assert!(
        p[0].mul_add(20.0, -9.5).abs() <= 0.01,
        "sdf {}, expected 9.5",
        p[0] * 20.0
    );
    assert!(
        (p[1].mul_add(2.0, -1.0) + 1.0).abs() <= 0.01,
        "normal.x {}",
        p[1].mul_add(2.0, -1.0)
    );
    assert!(
        p[2].mul_add(2.0, -1.0).abs() <= 0.01,
        "normal.y {}",
        p[2].mul_add(2.0, -1.0)
    );
}

#[test]
fn backdrop_module_binds_sibling_emission() {
    struct MockReads;

    impl BackdropReads for MockReads {
        fn sample(&self, _: [f64; 2]) -> [f64; 4] {
            [0.0; 4]
        }

        fn sample_level(&self, _: [f64; 2], _: f64) -> [f64; 4] {
            [0.0; 4]
        }

        fn field(&self, _: [f64; 2]) -> (f64, [f64; 2], f64) {
            (0.0, [0.0, 0.0], 1.0)
        }

        fn sibling_count(&self) -> u32 {
            2
        }

        fn sibling_index(&self) -> u32 {
            0
        }

        fn sibling(&self, index: u32, _: [f64; 2]) -> (f64, [f64; 2], [f64; 4]) {
            match index {
                1 => (9.5, [-1.0, 0.0], [0.25, 0.5, 0.75, 1.0]),
                _ => (0.0, [0.0, 0.0], [0.0; 4]),
            }
        }
    }

    let module = BackdropModule::new(
        "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
            let n = backdrop_sibling_count();
            let self_idx = backdrop_sibling_index();
            for (var i = 0u; i < n; i = i + 1u) {
                if i == self_idx {
                    continue;
                }
                let sib = backdrop_sibling(i, px.p);
                return vec4<f32>(sib.emission.rgb, sib.sdf / 20.0);
            }
            return vec4<f32>(0.0);
        }",
    )
    .expect("module parses");
    let px = BackdropPixel {
        p: [16.5, 16.5],
        sdf: -3.5,
        normal: [1.0, 0.0],
        own_sdf: -3.5,
        size: [16.0, 16.0],
        scale: 1.0,
        local: [16.5, 16.5],
    };
    let out = module.eval(&MockReads).at(&px, &params(&[]));
    assert!((out[0] - 0.25).abs() < 1e-6);
    assert!((out[1] - 0.5).abs() < 1e-6);
    assert!((out[2] - 0.75).abs() < 1e-6);
    assert!((out[3] - 9.5 / 20.0).abs() < 1e-6);
}
