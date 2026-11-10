//! Rendered coverage for `waterui::graphics::canvas`: every paint, stroke
//! style and compositing scope through the offscreen Hydrolysis renderer,
//! and a live operand animated by pumped frames. The images are written for
//! review by eye.

use std::path::PathBuf;
use std::time::Duration;

use hydrolysis_m3::Material3;
use waterui::SignalExt as _;
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::graphics::canvas::{
    BlendMode, Canvas, Cap, Circle, ConicGradient, Cx, Ellipse, ImagePattern, ImageSource, Join,
    Layer, LinearGradient, MeshGradient, Path, PixelBuffer, PixelFormat, RadialGradient,
    RoundedRect, Sampling, Shadow, StrokeStyle,
};
use waterui::graphics::color::WorkingColor;
use waterui::graphics::gradient::{AnimatedMeshGradient, AnimatedMeshGradientConfig};
use waterui::reactive::Signal as _;
use waterui::reactive::binding;
use waterui_core::layout::{Affine2, Point, Rect, Size};

use crate::{Role, ui};

fn output(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join("waterui_canvas");
    std::fs::create_dir_all(&directory).expect("the canvas gallery directory is writable");
    directory.join(name)
}

const fn rgb(r: f32, g: f32, b: f32) -> WorkingColor {
    WorkingColor::new([r, g, b, 1.0])
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(width, height))
}

/// An 8x8 checkerboard of opaque red and white texels.
fn checker() -> ImageSource {
    let mut pixels = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8 {
        for x in 0..8 {
            let red = (x + y) % 2 == 0;
            pixels.extend_from_slice(if red {
                &[220, 30, 30, 255]
            } else {
                &[255, 255, 255, 255]
            });
        }
    }
    ImageSource::new(PixelBuffer::new(8, 8, PixelFormat::Rgba8, pixels))
        .expect("a valid checkerboard")
}

/// Row one: linear, focal radial, conic and mesh gradients.
fn gradients(cx: &mut Cx) {
    cx.fill(
        RoundedRect::new(rect(10.0, 10.0, 80.0, 60.0), 10.0),
        LinearGradient::new(Point::new(10.0, 10.0), Point::new(90.0, 10.0))
            .stop(0.0, rgb(1.0, 0.2, 0.2))
            .stop(1.0, rgb(0.2, 0.3, 1.0)),
    );
    cx.fill(
        Circle::new(Point::new(140.0, 40.0), 30.0),
        RadialGradient::two_point(Point::new(130.0, 30.0), 0.0, Point::new(140.0, 40.0), 30.0)
            .stop(0.0, rgb(1.0, 1.0, 1.0))
            .stop(1.0, rgb(0.1, 0.6, 0.2)),
    );
    cx.fill(
        Circle::new(Point::new(220.0, 40.0), 30.0),
        ConicGradient::new(Point::new(220.0, 40.0), 0.0, core::f32::consts::TAU)
            .stop(0.0, rgb(1.0, 0.0, 0.0))
            .stop(0.5, rgb(0.0, 1.0, 0.0))
            .stop(1.0, rgb(1.0, 0.0, 0.0)),
    );
    cx.fill(
        rect(270.0, 10.0, 80.0, 60.0),
        MeshGradient::new(
            1,
            1,
            vec![
                Point::new(270.0, 10.0),
                Point::new(350.0, 10.0),
                Point::new(270.0, 70.0),
                Point::new(350.0, 70.0),
            ],
            vec![
                rgb(1.0, 0.0, 0.0),
                rgb(0.0, 1.0, 0.0),
                rgb(0.0, 0.0, 1.0),
                rgb(1.0, 1.0, 0.0),
            ],
        ),
    );
}

/// Row three: an isolated translucent layer over two overlapping rects, a
/// multiply layer over two overlapping discs, and a scaled image pattern.
fn layers(cx: &mut Cx, image: &ImageSource) {
    cx.layer(Layer::opacity(0.5), |cx| {
        cx.fill(rect(10.0, 180.0, 60.0, 50.0), rgb(1.0, 1.0, 1.0));
        cx.fill(rect(40.0, 195.0, 60.0, 35.0), rgb(1.0, 1.0, 1.0));
    });
    cx.fill(
        Circle::new(Point::new(160.0, 205.0), 25.0),
        rgb(1.0, 0.9, 0.2),
    );
    cx.layer(Layer::opacity(1.0).blend(BlendMode::Multiply), |cx| {
        cx.fill(
            Circle::new(Point::new(185.0, 205.0), 25.0),
            rgb(0.2, 0.6, 1.0),
        );
    });
    cx.fill(
        rect(240.0, 180.0, 110.0, 50.0),
        ImagePattern::new(image.clone())
            .transform(Affine2::scale(3.0, 3.0))
            .sampling(Sampling::Nearest),
    );
}

#[test]
fn canvas_gallery_renders_every_paint_and_scope() {
    let image = checker();
    let mut app = ui()
        .viewport(360, 240)
        .theme(Material3::defaults())
        .mount_offscreen(move || {
            let image = image.clone();
            Canvas::new(move |cx| {
                cx.fill(cx.size().map(Rect::from_size), rgb(0.08, 0.08, 0.1));
                gradients(cx);
                // Row two: a dashed, round-capped open path with an arc; a
                // thick bevel-joined triangle; a shadowed ellipse.
                cx.stroke(
                    Path::builder()
                        .move_to(Point::new(15.0, 150.0))
                        .line_to(Point::new(55.0, 100.0))
                        .arc(
                            Point::new(70.0, 130.0),
                            (20.0, 20.0),
                            -core::f32::consts::FRAC_PI_2,
                            core::f32::consts::PI,
                        )
                        .build(),
                    StrokeStyle::new(5.0).cap(Cap::Round).dash([12.0, 8.0], 0.0),
                    rgb(1.0, 0.85, 0.2),
                );
                cx.stroke(
                    Path::builder()
                        .move_to(Point::new(110.0, 150.0))
                        .line_to(Point::new(140.0, 95.0))
                        .line_to(Point::new(170.0, 150.0))
                        .close()
                        .build(),
                    StrokeStyle::new(10.0).join(Join::Bevel),
                    rgb(0.3, 0.8, 1.0),
                );
                cx.shadow(
                    Ellipse::new(rect(195.0, 100.0, 70.0, 45.0)),
                    Shadow::new(6.0, WorkingColor::new([0.0, 0.0, 0.0, 0.9])).offset(4.0, 6.0),
                );
                cx.fill(
                    Ellipse::new(rect(195.0, 100.0, 70.0, 45.0)),
                    rgb(0.9, 0.9, 0.9),
                );
                // An image clipped to a circle, under a transform.
                cx.transform(Affine2::translate(310.0, 125.0), |cx| {
                    cx.clip(Circle::new(Point::zero(), 28.0), |cx| {
                        cx.image(
                            image.clone(),
                            rect(-28.0, -28.0, 56.0, 56.0),
                            Sampling::Nearest,
                        );
                    });
                });
                layers(cx, &image);
            })
            .a11y_role(waterui::accessibility::AccessibilityRole::Image)
            .a11y_label("Canvas gallery")
        });
    app.query()
        .role(Role::IMAGE)
        .label("Canvas gallery")
        .assert_exists();
    app.snapshot()
        .save_png(output("gallery.png"))
        .expect("the gallery snapshot is writable");
}

#[test]
fn canvas_animates_a_live_operand_and_records_inserted_items() {
    #[derive(Clone, Debug, PartialEq)]
    struct Bar {
        id: u32,
        height: f32,
    }
    let x = binding(20.0_f32);
    let bars = binding(vec![
        Bar {
            id: 1,
            height: 40.0,
        },
        Bar {
            id: 2,
            height: 80.0,
        },
    ]);
    let mut app = ui()
        .viewport(240, 160)
        .theme(Material3::defaults())
        .mount_offscreen({
            let (x, bars) = (x.clone(), bars.clone());
            move || {
                let (x, bars) = (x.clone(), bars.clone());
                Canvas::new(move |cx| {
                    cx.fill(cx.size().map(Rect::from_size), rgb(0.1, 0.1, 0.12));
                    // Every change of `x` animates the circle over 400 ms.
                    cx.fill(
                        x.with(Animation::linear(Duration::from_millis(400)))
                            .map(|x| Circle::new(Point::new(x, 30.0), 14.0)),
                        rgb(1.0, 0.4, 0.2),
                    );
                    cx.each(
                        bars,
                        |bar: &Bar| bar.id,
                        |cx, bar| {
                            cx.fill(
                                bar.map(|bar| {
                                    #[expect(
                                        clippy::cast_precision_loss,
                                        reason = "a few test bars"
                                    )]
                                    let left = bar.id as f32 * 40.0;
                                    rect(left, 150.0 - bar.height, 30.0, bar.height)
                                }),
                                rgb(0.3, 0.8, 0.5),
                            );
                        },
                    );
                })
            }
        });
    let _ = app.snapshot();

    // A quarter of the way: the circle near x = 70.
    x.set(220.0);
    app.pump_for(Duration::from_millis(100));
    app.snapshot()
        .save_png(output("animated_alone_mid.png"))
        .expect("the mid-flight snapshot is writable");
    // A structural change mid-flight — one bar inserted, one resized —
    // records only the collection's regions: half-way, the circle is near
    // x = 120, still moving, not at its end.
    bars.set(vec![
        Bar {
            id: 1,
            height: 40.0,
        },
        Bar {
            id: 2,
            height: 120.0,
        },
        Bar {
            id: 3,
            height: 60.0,
        },
    ]);
    app.pump_for(Duration::from_millis(100));
    app.snapshot()
        .save_png(output("animated_mid.png"))
        .expect("the mid-flight snapshot is writable");
    app.pump_for(Duration::from_millis(300));
    app.snapshot()
        .save_png(output("animated_end.png"))
        .expect("the settled snapshot is writable");
}

#[test]
fn canvas_time_advances_with_pumped_frames() {
    let seen = std::rc::Rc::new(std::cell::Cell::new(Duration::ZERO));
    let mut app = ui()
        .viewport(120, 40)
        .theme(Material3::defaults())
        .mount_offscreen({
            let seen = std::rc::Rc::clone(&seen);
            move || {
                let seen = std::rc::Rc::clone(&seen);
                Canvas::new(move |cx| {
                    let time = cx.time();
                    cx.keep(time.watch(move |change| seen.set(*change.value())));
                    cx.fill(
                        time.map(|t| {
                            Circle::new(
                                Point::new(t.as_secs_f32().min(1.0).mul_add(100.0, 10.0), 20.0),
                                8.0,
                            )
                        }),
                        rgb(0.3, 0.8, 1.0),
                    );
                })
            }
        });
    let _ = app.snapshot();
    let start = seen.get();
    app.pump_for(Duration::from_millis(480));
    let advanced = seen
        .get()
        .checked_sub(start)
        .expect("the canvas clock never runs backwards");
    // The clock is the pumped frame clock, not the wall clock: within a
    // frame of the pumped duration however fast the pumps ran.
    assert!(
        advanced >= Duration::from_millis(464) && advanced <= Duration::from_millis(496),
        "time advanced {advanced:?} over 480 ms of pumped frames"
    );
    app.snapshot()
        .save_png(output("time_480ms.png"))
        .expect("the clock snapshot is writable");
}

#[test]
fn canvas_scope_records_its_region_again() {
    let formula = binding(String::from("ab"));
    let mut app = ui()
        .viewport(200, 60)
        .theme(Material3::defaults())
        .mount_offscreen({
            let formula = formula.clone();
            move || {
                let formula = formula.clone();
                Canvas::new(move |cx| {
                    cx.fill(cx.size().map(Rect::from_size), rgb(0.1, 0.1, 0.12));
                    // One box per character: the shape, not an operand,
                    // follows the source.
                    cx.scope(formula, |cx, source: &String| {
                        for (index, _) in source.chars().enumerate() {
                            #[expect(clippy::cast_precision_loss, reason = "a few test boxes")]
                            let left = (index as f32).mul_add(30.0, 10.0);
                            cx.fill(rect(left, 15.0, 24.0, 30.0), rgb(0.9, 0.7, 0.2));
                        }
                    });
                    cx.fill(rect(170.0, 15.0, 20.0, 30.0), rgb(0.3, 0.5, 1.0));
                })
            }
        });
    app.snapshot()
        .save_png(output("scope_before.png"))
        .expect("the scope snapshot is writable");
    formula.set(String::from("abcde"));
    app.pump_for(Duration::from_millis(16));
    app.snapshot()
        .save_png(output("scope_after.png"))
        .expect("the scope snapshot is writable");
}

#[test]
fn animated_mesh_gradient_flows_with_the_canvas_clock() {
    let mut app = ui()
        .viewport(360, 240)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            AnimatedMeshGradient::new(AnimatedMeshGradientConfig::aqua_bloom().speed(1.0))
                .a11y_role(waterui::accessibility::AccessibilityRole::Image)
                .a11y_label("Animated mesh")
        });
    app.query()
        .role(Role::IMAGE)
        .label("Animated mesh")
        .assert_exists();
    let _ = app.snapshot();
    app.pump_for(Duration::from_millis(500));
    app.snapshot()
        .save_png(output("animated_mesh_early.png"))
        .expect("the early snapshot is writable");
    app.pump_for(Duration::from_secs(20));
    app.snapshot()
        .save_png(output("animated_mesh_late.png"))
        .expect("the late snapshot is writable");
}
