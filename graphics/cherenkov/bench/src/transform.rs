//! Scene transforms applied to the parsed [`Scene`] before it reaches
//! any engine, so every adapter sees byte-identical content.
//!
//! [`repeated`] grows a scene's load by an integer factor `k`: the root
//! layer's draw list is repeated `k` times, each copy offset by a fixed
//! per-copy translation that wraps inside the canvas, so element count and
//! overdraw grow linearly without enlarging the surface.
//!
//! [`at_native`] retargets a scene to a device's surface size: the canvas
//! becomes the native size and the scene tree is drawn under the uniform
//! scale `s = native_width / scene_width`, like a device pixel ratio, so
//! the pixel load matches the device.

use cherenkov_scene::{Draw, GlyphRun, Item, Layer, Live, MeshGradient, Paint, Scene, Shape};
use kurbo::{Affine, Line, Vec2};

/// The per-copy step of [`repeated`], as a fraction of the canvas. The
/// golden-ratio conjugate spreads successive copies' offsets before they
/// wrap, so a copy overlaps its predecessors instead of re-covering them.
const COPY_STEP: f64 = 0.618_033_988_749_894_9;

/// Copy `i`'s translation: a fixed step per copy, wrapped into the
/// `width`×`height` canvas. Copy 0 sits at the origin — untranslated.
fn copy_offset(i: u32, width: u32, height: u32) -> Vec2 {
    let w = f64::from(width).max(1.0);
    let h = f64::from(height).max(1.0);
    Vec2::new(
        (f64::from(i) * (w * COPY_STEP)) % w,
        (f64::from(i) * (h * COPY_STEP)) % h,
    )
}

/// The scene with its root draw list repeated `k` times.
///
/// Copy `i` of each root item is translated by `copy_offset(i)` — a
/// child layer is shifted through its `transform`, a draw through its
/// geometry — so every copy stays inside the canvas and overdraw grows
/// with `k`. `live` entries are replicated per copy with their frames
/// translated the same way. `k` of 0 or 1 returns the scene unchanged.
/// Features are recomputed; a pure translation never adds one.
#[must_use]
pub fn repeated(scene: &Scene, k: u32) -> Scene {
    if k <= 1 {
        return scene.clone();
    }
    let mut out = scene.clone();
    let base = std::mem::take(&mut out.root.items);
    let base_live = std::mem::take(&mut out.root.live);
    let n = base.len();
    out.root
        .items
        .reserve_exact(n.saturating_mul(k as usize - 1));
    for i in 0..k {
        let d = copy_offset(i, scene.width, scene.height);
        out.root
            .items
            .extend(base.iter().map(|item| translated_item(item, d)));
        out.root.live.extend(base_live.iter().map(|live| {
            Live {
                item: live.item + i as usize * n,
                frames: live
                    .frames
                    .iter()
                    .map(|draw| translate_draw(draw, d))
                    .collect(),
            }
        }));
    }
    out.compute_features();
    out
}

/// One item shifted by `d`: draws translate their geometry, layers carry
/// the offset in their `transform` (clip, scroll, motion and `live` are
/// in the layer's own space and ride along unchanged).
fn translated_item(item: &Item, d: Vec2) -> Item {
    match item {
        Item::Draw(draw) => Item::Draw(translate_draw(draw, d)),
        Item::Layer(layer) => {
            let mut layer = layer.clone();
            layer.transform = Affine::translate(d) * layer.transform;
            Item::Layer(layer)
        }
        Item::Group(group) => Item::Group(translated_group(group, d)),
    }
}

/// A group shifted by `d`: member draws translate; nested groups recurse.
fn translated_group(group: &cherenkov_scene::Group, d: Vec2) -> cherenkov_scene::Group {
    let mut group = group.clone();
    group.items = group
        .items
        .into_iter()
        .map(|item| match item {
            cherenkov_scene::GroupItem::Draw(draw) => {
                cherenkov_scene::GroupItem::Draw(translate_draw(&draw, d))
            }
            cherenkov_scene::GroupItem::Group(g) => {
                cherenkov_scene::GroupItem::Group(translated_group(&g, d))
            }
        })
        .collect();
    group
}

/// A draw command shifted by `d`, paint included so a copy looks like a
/// copy rather than re-sampling anchored paint.
fn translate_draw(draw: &Draw, d: Vec2) -> Draw {
    match draw {
        Draw::Fill { shape, rule, paint } => Draw::Fill {
            shape: translate_shape(shape, d),
            rule: *rule,
            paint: translate_paint(paint, d),
        },
        Draw::Stroke {
            shape,
            stroke,
            paint,
        } => Draw::Stroke {
            shape: translate_shape(shape, d),
            stroke: stroke.clone(),
            paint: translate_paint(paint, d),
        },
        // `offset` is the drop direction relative to the shape — it does
        // not translate with it.
        Draw::Shadow {
            shape,
            blur_sigma,
            offset,
            color,
        } => Draw::Shadow {
            shape: translate_shape(shape, d),
            blur_sigma: *blur_sigma,
            offset: *offset,
            color: *color,
        },
        Draw::Glyphs(run) => Draw::Glyphs(translate_run(run, d)),
        Draw::Image {
            image,
            encoding,
            dst,
            sampling,
        } => Draw::Image {
            image: *image,
            encoding: *encoding,
            dst: translate_rect(*dst, d),
            sampling: *sampling,
        },
    }
}

/// A shape shifted by `d`, preserving its variant (a translated rect is
/// still a rect — cheaper lowering than a generic path).
fn translate_shape(shape: &Shape, d: Vec2) -> Shape {
    match shape {
        Shape::Rect(r) => Shape::Rect(translate_rect(*r, d)),
        Shape::RoundedRect(r) => {
            let rect = translate_rect(r.rect(), d);
            Shape::RoundedRect(kurbo::RoundedRect::new(
                rect.x0,
                rect.y0,
                rect.x1,
                rect.y1,
                r.radii(),
            ))
        }
        Shape::Continuous(c) => Shape::Continuous(cherenkov_scene::ContinuousRect::new(
            translate_rect(c.rect, d),
            c.corner_radius,
            c.smoothing,
        )),
        Shape::Circle(c) => Shape::Circle(kurbo::Circle::new(c.center + d, c.radius)),
        Shape::Ellipse(e) => Shape::Ellipse(kurbo::Ellipse::new(e.center() + d, e.radii(), 0.0)),
        Shape::Line(l) => Shape::Line(Line::new(l.p0 + d, l.p1 + d)),
        Shape::Path { path } => Shape::Path {
            path: Affine::translate(d) * path.clone(),
        },
    }
}

fn translate_rect(r: kurbo::Rect, d: Vec2) -> kurbo::Rect {
    kurbo::Rect::new(r.x0 + d.x, r.y0 + d.y, r.x1 + d.x, r.y1 + d.y)
}

/// A paint whose anchored coordinates shifted by `d`, so gradients,
/// meshes and image patterns travel with the shape they fill.
fn translate_paint(paint: &Paint, d: Vec2) -> Paint {
    match paint {
        Paint::Transformed { paint, transform } => Paint::Transformed {
            paint: Box::new(translate_paint(paint, d)),
            transform: Affine::translate(d) * *transform,
        },
        Paint::Linear(g) => Paint::Linear(cherenkov_scene::LinearGradient {
            start: g.start + d,
            end: g.end + d,
            stops: g.stops.clone(),
            extend: g.extend,
            interpolation: g.interpolation,
        }),
        Paint::Radial(g) => Paint::Radial(cherenkov_scene::RadialGradient {
            center0: g.center0 + d,
            r0: g.r0,
            center1: g.center1 + d,
            r1: g.r1,
            stops: g.stops.clone(),
            extend: g.extend,
            interpolation: g.interpolation,
        }),
        Paint::Sweep(g) => Paint::Sweep(cherenkov_scene::SweepGradient {
            center: g.center + d,
            start_angle: g.start_angle,
            end_angle: g.end_angle,
            stops: g.stops.clone(),
            extend: g.extend,
            interpolation: g.interpolation,
        }),
        Paint::Mesh(m) => Paint::Mesh(
            MeshGradient::new(
                m.columns(),
                m.rows(),
                m.points().iter().map(|p| *p + d).collect(),
                m.colors().to_vec(),
            )
            .interpolation(m.interpolation_mode()),
        ),
        Paint::Image(ip) => Paint::Image(cherenkov_scene::ImagePaint {
            image: ip.image,
            encoding: ip.encoding,
            transform: Affine::translate(d) * ip.transform,
            extend_x: ip.extend_x,
            extend_y: ip.extend_y,
            sampling: ip.sampling,
        }),
        Paint::Solid(_) => paint.clone(),
    }
}

/// A glyph run shifted by `d`: glyph positions are pen-relative, so
/// shifting every glyph origin translates the whole run.
#[expect(
    clippy::cast_possible_truncation,
    reason = "glyph positions are f32 at the format boundary"
)]
fn translate_run(run: &GlyphRun, d: Vec2) -> GlyphRun {
    GlyphRun {
        font: run.font,
        font_index: run.font_index,
        size: run.size,
        normalized_coords: run.normalized_coords.clone(),
        glyphs: run
            .glyphs
            .iter()
            .map(|g| cherenkov_scene::Glyph {
                x: g.x + d.x as f32,
                y: g.y + d.y as f32,
                ..*g
            })
            .collect(),
        stroke: run.stroke.clone(),
        paint: translate_paint(&run.paint, d),
    }
}

/// The scene retargeted to a native `width`×`height` surface.
///
/// The whole layer tree — the original root included — is nested under a
/// new root whose transform is the uniform scale `s = width /
/// scene.width`, so transforms, clips, scroll offsets, motion and `live`
/// all scale exactly as a device pixel ratio would scale them. Returns
/// the retargeted scene and `s`. The canvas clamps the overflow the
/// aspect mismatch produces; that is what "pixel load matches the
/// device" means.
#[must_use]
pub fn at_native(scene: &Scene, width: u32, height: u32) -> (Scene, f64) {
    let scale = f64::from(width) / f64::from(scene.width);
    let mut out = scene.clone();
    let root = std::mem::take(&mut out.root);
    out.width = width;
    out.height = height;
    out.root = Layer {
        transform: Affine::scale(scale),
        items: vec![Item::Layer(root)],
        ..Layer::default()
    };
    (out, scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov_scene::{Color, ColorSpace, FillRule, Glyph, ResourceHash, StrokeStyle};

    fn draw_count(layer: &Layer) -> usize {
        layer
            .items
            .iter()
            .map(|item| match item {
                Item::Draw(_) => 1,
                Item::Layer(l) => draw_count(l),
                Item::Group(g) => group_count(g),
            })
            .sum()
    }

    fn group_count(group: &cherenkov_scene::Group) -> usize {
        group
            .items
            .iter()
            .map(|item| match item {
                cherenkov_scene::GroupItem::Draw(_) => 1,
                cherenkov_scene::GroupItem::Group(g) => group_count(g),
            })
            .sum()
    }

    fn fill(x: f64) -> Item {
        Item::Draw(Draw::Fill {
            shape: Shape::Rect(kurbo::Rect::new(x, 10.0, x + 20.0, 30.0)),
            rule: FillRule::NonZero,
            paint: Paint::Solid(Color {
                space: ColorSpace::Srgb,
                components: [1.0, 0.0, 0.0, 1.0],
            }),
        })
    }

    fn scene_with(items: Vec<Item>) -> Scene {
        let mut scene = Scene::new(
            1000,
            800,
            Color {
                space: ColorSpace::Srgb,
                components: [0.0, 0.0, 0.0, 1.0],
            },
        );
        scene.root.items = items;
        scene.compute_features();
        scene
    }

    #[test]
    fn k1_is_the_identity() {
        let scene = scene_with(vec![
            fill(5.0),
            Item::Layer(Layer {
                transform: Affine::translate(Vec2::new(3.0, 4.0)),
                items: vec![fill(0.0), fill(50.0)],
                ..Layer::default()
            }),
        ]);
        assert_eq!(repeated(&scene, 0), scene);
        assert_eq!(repeated(&scene, 1), scene);
    }

    #[test]
    fn k3_triples_the_element_count() {
        let mut child = Layer {
            transform: Affine::translate(Vec2::new(3.0, 4.0)),
            ..Layer::default()
        };
        child.items = vec![fill(0.0), fill(50.0)];
        let scene = scene_with(vec![fill(5.0), Item::Layer(child)]);
        let out = repeated(&scene, 3);
        assert_eq!(draw_count(&scene.root) * 3, draw_count(&out.root));
        assert_eq!(out.root.items.len(), 3 * scene.root.items.len());
    }

    #[test]
    fn copies_wrap_inside_the_canvas() {
        let scene = scene_with(vec![fill(0.0)]);
        let out = repeated(&scene, 4);
        let w = f64::from(scene.width);
        for i in 0..4usize {
            let Item::Draw(Draw::Fill {
                shape: Shape::Rect(r),
                ..
            }) = &out.root.items[i]
            else {
                panic!("expected a rect fill");
            };
            let d = copy_offset(u32::try_from(i).expect("i < 4"), scene.width, scene.height);
            assert!(d.x >= 0.0 && d.x < w);
            assert!((r.x0 - d.x).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn live_entries_follow_their_copies() {
        let mut scene = scene_with(vec![fill(0.0), fill(40.0)]);
        let frames = vec![match &scene.root.items[1] {
            Item::Draw(d) => d.clone(),
            Item::Layer(_) | Item::Group(_) => unreachable!(),
        }];
        scene.root.live.push(Live { item: 1, frames });
        let out = repeated(&scene, 3);
        assert_eq!(out.root.live.len(), 3);
        assert_eq!(
            out.root.live.iter().map(|l| l.item).collect::<Vec<_>>(),
            vec![1, 3, 5]
        );
        let Draw::Fill {
            shape: Shape::Rect(r),
            ..
        } = &out.root.live[1].frames[0]
        else {
            panic!("expected a rect fill");
        };
        let d = copy_offset(1, scene.width, scene.height);
        assert!((r.x0 - (40.0 + d.x)).abs() < f64::EPSILON);
    }

    #[test]
    fn glyph_runs_translate() {
        let run = GlyphRun {
            font: ResourceHash::of(b"f"),
            font_index: 0,
            size: 12.0,
            normalized_coords: Vec::new(),
            glyphs: vec![Glyph {
                id: 1,
                x: 10.0,
                y: 20.0,
                transform: None,
            }],
            stroke: None,
            paint: Paint::Solid(Color {
                space: ColorSpace::Srgb,
                components: [0.0, 0.0, 0.0, 1.0],
            }),
        };
        let moved = translate_run(&run, Vec2::new(7.0, 9.0));
        assert_eq!((moved.glyphs[0].x, moved.glyphs[0].y), (17.0, 29.0));
    }

    #[test]
    fn native_scales_under_a_wrapper() {
        let mut scene = scene_with(vec![fill(0.0)]);
        scene.root.transform = Affine::translate(Vec2::new(2.0, 3.0));
        scene.root.live.push(Live {
            item: 0,
            frames: vec![],
        });
        let (out, s) = at_native(&scene, 2752, 2064);
        assert_eq!((out.width, out.height), (2752, 2064));
        assert!((s - 2.752).abs() < f64::EPSILON);
        // The scene's own root — transform, items and live — sits one
        // level down under the uniform scale.
        assert_eq!(out.root.transform, Affine::scale(2.752));
        let [Item::Layer(inner)] = out.root.items.as_slice() else {
            panic!("expected one wrapper child");
        };
        assert_eq!(inner.transform, Affine::translate(Vec2::new(2.0, 3.0)));
        assert_eq!(inner.live.len(), 1);
        assert_eq!(draw_count(inner), 1);
    }

    #[test]
    fn stroke_styles_and_line_shapes_translate() {
        let draw = Draw::Stroke {
            shape: Shape::Line(Line::new(
                kurbo::Point::new(0.0, 0.0),
                kurbo::Point::new(5.0, 5.0),
            )),
            stroke: StrokeStyle::default(),
            paint: Paint::Solid(Color {
                space: ColorSpace::Srgb,
                components: [0.0, 0.0, 0.0, 1.0],
            }),
        };
        let Draw::Stroke {
            shape: Shape::Line(l),
            ..
        } = translate_draw(&draw, Vec2::new(2.0, 3.0))
        else {
            panic!("expected a stroked line");
        };
        assert_eq!(
            (l.p0, l.p1),
            (kurbo::Point::new(2.0, 3.0), kurbo::Point::new(7.0, 8.0))
        );
    }
}
