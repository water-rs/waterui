//! Gradient interval searches across long lists and repeated offsets.

use cherenkov_scene::kurbo::Point;
use cherenkov_scene::{Color, ColorSpace, Extend, GradientStop, LinearGradient, Paint, Shape};

use crate::Corpus;

fn palette(space: ColorSpace, scale: f32) -> [Color; 4] {
    [
        [1.0, 0.0, 0.25, 1.0],
        [0.0, 1.0, 0.5, 0.5],
        [0.0, 0.25, 1.0, 0.75],
        [1.0, 0.5, 0.0, 1.0],
    ]
    .map(|[r, g, b, a]| Color::new(space, [r * scale, g * scale, b * scale, a]))
}

fn stops(offsets: impl IntoIterator<Item = f32>, colors: [Color; 4]) -> Vec<GradientStop> {
    offsets
        .into_iter()
        .enumerate()
        .map(|(i, offset)| GradientStop {
            offset,
            color: colors[i % colors.len()],
        })
        .collect()
}

pub fn add_corpus(corpus: &mut Corpus) {
    for (suffix, space, scale) in [
        ("srgb", ColorSpace::Srgb, 1.0),
        ("p3", ColorSpace::LinearP3, 1.0),
        ("hdr", ColorSpace::LinearP3, 4.0),
    ] {
        let colors = palette(space, scale);
        for (ename, extend) in [
            ("pad", Extend::Pad),
            ("repeat", Extend::Repeat),
            ("reflect", Extend::Reflect),
            ("none", Extend::None),
        ] {
            corpus.scene(
                format!("grad-search-hard-{suffix}-{ename}"),
                128,
                80,
                Color::srgb(0.95, 0.95, 0.95),
                |layer| {
                    for (row, count) in (0u16..).zip([8u16, 9, 17, 65]) {
                        // Duplicate offsets span [1/4, 3/4]. For odd counts,
                        // boundaries coincide with pixel centres, exercising
                        // equality at hard stops as well as both outer ranges.
                        let offsets = (0..count)
                            .map(|i| 0.25 + 0.5 * f32::from(i / 2) / f32::from((count - 1) / 2));
                        layer.fill(
                            Shape::rect(8.0, 16.0f64.mul_add(f64::from(row), 8.0), 112.0, 16.0),
                            Paint::Linear(LinearGradient {
                                start: Point::new(32.5, 0.0),
                                end: Point::new(96.5, 0.0),
                                stops: stops(offsets, colors),
                                extend,
                                interpolation: space,
                            }),
                        );
                    }
                },
            );
        }
    }
}

pub fn add_perf(perf: &mut Corpus) {
    for count in [2u16, 4, 8, 9, 16, 32, 64, 128] {
        perf.scene(
            format!("gradient-stops-{count}"),
            1280,
            720,
            Color::srgb(0.95, 0.95, 0.95),
            |layer| {
                let offsets = (0..count).map(|i| f32::from(i) / f32::from(count - 1));
                layer.fill(
                    Shape::rect(0.0, 0.0, 1280.0, 720.0),
                    Paint::Linear(LinearGradient {
                        start: Point::new(0.0, 0.0),
                        end: Point::new(1280.0, 0.0),
                        stops: stops(offsets, palette(ColorSpace::LinearP3, 1.0)),
                        extend: Extend::Pad,
                        interpolation: ColorSpace::LinearP3,
                    }),
                );
            },
        );
    }
}
