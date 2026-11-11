//! The oracle's user-shader and described-filter paths against the built-in
//! paints, effects and filters they restate.

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
