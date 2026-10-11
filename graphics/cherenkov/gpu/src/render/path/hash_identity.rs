//! Frozen pre-batching hashing, so cache identity changes fail independently of
//! the optimized implementation. Includes every shape, stroke field and path verb.

use super::*;

mod reference {
    use super::*;
    fn hash_shape_fields(hasher: &mut DefaultHasher, shape: &ShapeData) {
        fn v(hasher: &mut DefaultHasher, x: f64) {
            x.to_bits().hash(hasher);
        }
        fn rect(hasher: &mut DefaultHasher, r: &Rect) {
            v(hasher, r.x0);
            v(hasher, r.y0);
            v(hasher, r.x1);
            v(hasher, r.y1);
        }
        match shape {
            ShapeData::Rect(r) => {
                0u8.hash(hasher);
                rect(hasher, r);
            }
            ShapeData::RoundedRect(rr) => {
                1u8.hash(hasher);
                rect(hasher, &rr.rect());
                let radii = rr.radii();
                v(hasher, radii.top_left);
                v(hasher, radii.top_right);
                v(hasher, radii.bottom_right);
                v(hasher, radii.bottom_left);
            }
            ShapeData::Continuous(c) => {
                2u8.hash(hasher);
                rect(hasher, &c.rect);
                let radii = c.radii;
                v(hasher, radii.top_left);
                v(hasher, radii.top_right);
                v(hasher, radii.bottom_right);
                v(hasher, radii.bottom_left);
                v(hasher, c.smoothing);
            }
            ShapeData::Circle(c) => {
                3u8.hash(hasher);
                v(hasher, c.center.x);
                v(hasher, c.center.y);
                v(hasher, c.radius);
            }
            ShapeData::Ellipse(e) => {
                4u8.hash(hasher);
                let center = e.center();
                let radii = e.radii();
                v(hasher, center.x);
                v(hasher, center.y);
                v(hasher, radii.x);
                v(hasher, radii.y);
                v(hasher, e.rotation());
            }
            ShapeData::Line(l) => {
                5u8.hash(hasher);
                v(hasher, l.p0.x);
                v(hasher, l.p0.y);
                v(hasher, l.p1.x);
                v(hasher, l.p1.y);
            }
            ShapeData::Path { elements, rule } => {
                6u8.hash(hasher);
                (*rule as u8).hash(hasher);
                hash_elements_into(hasher, elements);
            }
        }
    }

    /// A stable hash of a stroked draw: the outline's source shape plus every
    /// stroke parameter and the local flatten tolerance, tagged so it never
    /// collides with a fill of the same geometry.
    pub(super) fn hash_stroke(shape: &ShapeData, stroke: &kurbo::Stroke, tolerance: f64) -> u64 {
        let mut hasher = DefaultHasher::new();
        2u64.hash(&mut hasher);
        hash_shape_fields(&mut hasher, shape);
        stroke.width.to_bits().hash(&mut hasher);
        (stroke.join as u8).hash(&mut hasher);
        stroke.miter_limit.to_bits().hash(&mut hasher);
        (stroke.start_cap as u8).hash(&mut hasher);
        (stroke.end_cap as u8).hash(&mut hasher);
        (stroke.dash_pattern.len() as u64).hash(&mut hasher);
        for v in &stroke.dash_pattern {
            v.to_bits().hash(&mut hasher);
        }
        stroke.dash_offset.to_bits().hash(&mut hasher);
        tolerance.to_bits().hash(&mut hasher);
        hasher.finish()
    }

    /// A stable hash of a local path's element list, tagged by draw mode so a
    /// fill, an even-odd fill and a stroke of the same outline never collide.
    pub(super) fn hash_elements(elements: &[PathEl], tag: u64) -> u64 {
        let mut hasher = DefaultHasher::new();
        tag.hash(&mut hasher);
        hash_elements_into(&mut hasher, elements);
        hasher.finish()
    }

    fn hash_elements_into(hasher: &mut DefaultHasher, elements: &[PathEl]) {
        fn point(hasher: &mut DefaultHasher, disc: u8, p: Point) {
            disc.hash(hasher);
            p.x.to_bits().hash(hasher);
            p.y.to_bits().hash(hasher);
        }
        for el in elements {
            match el {
                PathEl::MoveTo(p) => point(hasher, 0, *p),
                PathEl::LineTo(p) => point(hasher, 1, *p),
                PathEl::QuadTo(c, p) => {
                    point(hasher, 2, *c);
                    point(hasher, 2, *p);
                }
                PathEl::CurveTo(c0, c1, p) => {
                    point(hasher, 3, *c0);
                    point(hasher, 3, *c1);
                    point(hasher, 3, *p);
                }
                PathEl::ClosePath => 4u8.hash(hasher),
            }
        }
    }

    /// A path draw's cache key and placement.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Placement {
        /// Cache key: content hash + matrix + quantized subpixel + surface.
        pub key: u64,
        /// `key` plus the integer translation: coverage clipped by the
        /// surface is only valid at this offset.
        pub key_exact: u64,
        /// The transform to rasterize under: the true 2x2 and the translation
        /// snapped to the 1/4 px grid.
        pub raster: Affine,
        /// The integer translation the cache's stored rects are relative to.
        pub offset: Vec2,
    }

    /// Builds the [`Placement`] for a draw under `transform` on a
    /// `surface`-pixel target. The key holds the 2x2, the translation's
    /// fractional part and the surface size, so identical geometry at
    /// different integer translations replays the same emission.
    #[expect(clippy::cast_possible_truncation)]
    #[expect(
        clippy::many_single_char_names,
        reason = "a..f are the conventional affine coefficient names"
    )]
    pub(super) fn placement(
        content_hash: u64,
        transform: Affine,
        surface: (u32, u32),
    ) -> Placement {
        let [a, b, c, d, e, f] = transform.as_coeffs();
        let ix = e.floor();
        let iy = f.floor();
        let qx = e - ix;
        let qy = f - iy;
        let mut hasher = DefaultHasher::new();
        content_hash.hash(&mut hasher);
        for v in [a, b, c, d, qx, qy] {
            (v as f32).to_bits().hash(&mut hasher);
        }
        surface.hash(&mut hasher);
        let key = hasher.finish();
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (ix as i64).hash(&mut hasher);
        (iy as i64).hash(&mut hasher);
        Placement {
            key,
            key_exact: hasher.finish(),
            raster: Affine::new([a, b, c, d, ix + qx, iy + qy]),
            offset: Vec2::new(ix, iy),
        }
    }
}

#[test]
fn batched_hashes_preserve_every_stroke_field_and_path_verb() {
    use cherenkov::kurbo::{Cap, Circle, Ellipse, Join, Line, RoundedRect, Stroke};
    let mut seed = 0x938c_147b_a6e2_105fu64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        f64::from_bits(seed)
    };
    let mut elements = vec![PathEl::MoveTo(Point::new(-0.0, f64::NAN))];
    for n in 0..96usize {
        let a = Point::new(next(), next());
        let b = Point::new(next(), next());
        let c = Point::new(next(), next());
        elements.push(match n % 5 {
            0 => PathEl::MoveTo(a),
            1 => PathEl::LineTo(a),
            2 => PathEl::QuadTo(a, b),
            3 => PathEl::CurveTo(a, b, c),
            _ => PathEl::ClosePath,
        });
        for tag in [0, 1, 2, u64::MAX] {
            assert_eq!(
                hash_elements(&elements, tag),
                reference::hash_elements(&elements, tag)
            );
        }
        let rect = Rect::new(-0.0, -3.5, 30.0, 120.25);
        let shapes = [
            ShapeData::Rect(rect),
            ShapeData::RoundedRect(RoundedRect::from_rect(rect, (1.0, 2.0, 3.0, 4.0))),
            ShapeData::Continuous(cherenkov::ContinuousRect::new(rect, 8.0).with_smoothing(next())),
            ShapeData::Circle(Circle::new(a, next())),
            ShapeData::Ellipse(Ellipse::new(a, (next(), next()), next())),
            ShapeData::Line(Line::new(a, b)),
            ShapeData::Path {
                elements: elements.clone().into(),
                rule: FillRule::NonZero,
            },
            ShapeData::Path {
                elements: elements.clone().into(),
                rule: FillRule::EvenOdd,
            },
        ];
        for shape in &shapes {
            let mut stroke = Stroke::new(next());
            stroke.join = [Join::Miter, Join::Round, Join::Bevel][n % 3];
            stroke.start_cap = [Cap::Butt, Cap::Round, Cap::Square][n % 3];
            stroke.end_cap = [Cap::Butt, Cap::Round, Cap::Square][(n / 3) % 3];
            stroke.miter_limit = next();
            stroke.dash_offset = next();
            stroke.dash_pattern.extend((0..n % 19).map(|_| next()));
            for tolerance in [0.0, -0.0, FLATTEN, next(), f64::INFINITY, f64::NAN] {
                assert_eq!(
                    hash_stroke(shape, &stroke, tolerance),
                    reference::hash_stroke(shape, &stroke, tolerance)
                );
            }
        }
        let transform = Affine::new([next(), next(), next(), next(), next(), next()]);
        let surface = (u32::try_from(n).unwrap(), u32::MAX);
        let content = next().to_bits();
        let actual = placement(content, transform, surface);
        let expected = reference::placement(content, transform, surface);
        assert_eq!(
            (actual.key, actual.key_exact()),
            (expected.key, expected.key_exact)
        );
        assert_eq!(
            actual.raster.as_coeffs().map(f64::to_bits),
            expected.raster.as_coeffs().map(f64::to_bits)
        );
        assert_eq!(
            [actual.offset.x.to_bits(), actual.offset.y.to_bits()],
            [expected.offset.x.to_bits(), expected.offset.y.to_bits()]
        );
    }
}
