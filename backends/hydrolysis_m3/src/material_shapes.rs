//! The rounded polygons Material's loading indicator morphs through.
//!
//! Compose builds these with `androidx.graphics.shapes`, whose polygons carry
//! per-corner rounding and whose morph matches cubics between two shapes. This
//! is a narrower construction serving the one component that needs it: build
//! each outline once with rounded corners, then describe it as a radius per
//! angle.
//!
//! That polar description is what makes the morph well defined. Every shape
//! here is star-convex about its centre — a ray from the centre crosses the
//! outline exactly once — so a shape *is* a radius function, two shapes
//! interpolate by interpolating those radii, and the result stays rotationally
//! aligned instead of drifting the way arc-length matching does.
//!
//! This exists for Material's loading indicator, which is not built yet: the
//! indicator needs a per-frame clock, and in this codebase only backend widgets
//! have one — a composed view cannot tick itself, so the indicator has to
//! become a semantic component the renderer drives. The geometry is not what
//! stands in the way.
//!
//! Three of its seven silhouettes (`Cookie9Sided`, `Sunny`, `Oval`) are built
//! from parameters Compose documents, and [`RoundedPolygon`] makes them
//! exactly. The other four come from control points fed through
//! `MaterialShapes.kt`'s `customPolygon`, whose repetition and mirroring
//! [`RoundedPolygon::repeated`] reproduces.

use vello::kurbo::{BezPath, Point, Vec2};

/// How many angles each shape is sampled at. The outlines are smooth and the
/// indicator is drawn small, so this is well past the point of visible facets.
pub const SAMPLE_COUNT: usize = 180;

/// A vertex and the radius its corner is rounded by, in the shape's own units
/// where the outer radius is 1.
#[derive(Debug, Clone, Copy)]
pub struct RoundedVertex {
    /// Position relative to the shape's centre.
    pub offset: Vec2,
    /// Corner rounding radius; zero leaves the corner sharp.
    pub rounding: f64,
}

impl RoundedVertex {
    /// A vertex at `x`, `y` rounded by `rounding`.
    #[must_use]
    pub const fn new(x: f64, y: f64, rounding: f64) -> Self {
        Self {
            offset: Vec2::new(x, y),
            rounding,
        }
    }
}

/// A closed polygon whose corners are rounded.
#[derive(Debug, Clone)]
pub struct RoundedPolygon {
    vertices: Vec<RoundedVertex>,
}

impl RoundedPolygon {
    /// A star with `points` spikes, alternating between the outer radius and
    /// `inner_radius`, every corner rounded by `rounding`.
    ///
    /// This is `RoundedPolygon.star(numVerticesPerRadius, innerRadius,
    /// rounding)`.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "vertex and sample counts are small and exact in f64"
    )]
    pub fn star(points: usize, inner_radius: f64, rounding: f64) -> Self {
        let step = core::f64::consts::PI / points as f64;
        let vertices = (0..points * 2)
            .map(|index| {
                let angle = index as f64 * step;
                let radius = if index % 2 == 0 { 1.0 } else { inner_radius };
                RoundedVertex::new(angle.cos() * radius, angle.sin() * radius, rounding)
            })
            .collect();
        Self { vertices }
    }

    /// A circle, as a polygon fine enough that its corners need no rounding.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "vertex and sample counts are small and exact in f64"
    )]
    pub fn circle() -> Self {
        const SEGMENTS: usize = 64;
        let vertices = (0..SEGMENTS)
            .map(|index| {
                let angle = index as f64 / SEGMENTS as f64 * core::f64::consts::TAU;
                RoundedVertex::new(angle.cos(), angle.sin(), 0.0)
            })
            .collect();
        Self { vertices }
    }

    /// Repeats `base` around the centre, optionally mirroring alternate
    /// sections so each pair is symmetric.
    ///
    /// This is `MaterialShapes.kt`'s `doRepeat`. Without mirroring the base
    /// points are simply rotated `reps` times.
    ///
    /// Mirroring works in polar terms and keeps each point's distance from the
    /// centre, moving only its angle. There are twice as many sections; an even
    /// one lays the base angles down turned by its own share, and an odd one
    /// walks them back reflected about the first point's angle. That reflection
    /// maps the first point onto the one the previous section started from, so
    /// the odd section drops it — a repeated vertex has no edges to round
    /// between and would notch the outline.
    #[allow(
        clippy::cast_precision_loss,
        reason = "repetition counts are small and exact in f64"
    )]
    #[must_use]
    pub fn repeated(base: &[RoundedVertex], reps: usize, mirroring: bool) -> Self {
        if !mirroring {
            let sector = core::f64::consts::TAU / reps as f64;
            let vertices = (0..reps)
                .flat_map(|rep| {
                    let rotation = sector * rep as f64;
                    base.iter().map(move |vertex| rotate(*vertex, rotation))
                })
                .collect();
            return Self { vertices };
        }

        let angles: Vec<f64> = base
            .iter()
            .map(|vertex| vertex.offset.y.atan2(vertex.offset.x))
            .collect();
        let distances: Vec<f64> = base.iter().map(|vertex| vertex.offset.hypot()).collect();
        let sections = reps * 2;
        let sector = core::f64::consts::TAU / sections as f64;
        let mut vertices = Vec::with_capacity(base.len() * sections);
        for section in 0..sections {
            let forwards = section % 2 == 0;
            for index in 0..base.len() {
                let point = if forwards {
                    index
                } else {
                    base.len() - 1 - index
                };
                if point == 0 && !forwards {
                    continue;
                }
                let within = if forwards {
                    angles[point]
                } else {
                    2.0f64.mul_add(angles[0], sector - angles[point])
                };
                let angle = sector.mul_add(section as f64, within);
                vertices.push(RoundedVertex {
                    offset: Vec2::new(angle.cos(), angle.sin()) * distances[point],
                    rounding: base[point].rounding,
                });
            }
        }
        Self { vertices }
    }

    /// Scales this polygon's vertical extent, as Compose's `Oval` does before
    /// rotating.
    #[must_use]
    pub fn scaled_y(mut self, factor: f64) -> Self {
        for vertex in &mut self.vertices {
            vertex.offset.y *= factor;
        }
        self
    }

    /// Rotates this polygon by `radians`.
    #[must_use]
    pub fn rotated(mut self, radians: f64) -> Self {
        for vertex in &mut self.vertices {
            *vertex = rotate(*vertex, radians);
        }
        self
    }

    /// The outline as a closed path with every corner rounded.
    #[must_use]
    pub fn outline(&self) -> BezPath {
        let count = self.vertices.len();
        let mut path = BezPath::new();
        let mut started = false;
        for index in 0..count {
            let previous = self.vertices[(index + count - 1) % count].offset;
            let current = self.vertices[index];
            let next = self.vertices[(index + 1) % count].offset;
            let (start, end, centre) = round_corner(previous, current, next);
            if started {
                path.line_to(start);
            } else {
                path.move_to(start);
                started = true;
            }
            if let Some(centre) = centre {
                append_arc(&mut path, centre, start, end);
            }
        }
        path.close_path();
        path
    }

    /// The distance from the centre to the outline at each of
    /// [`SAMPLE_COUNT`] evenly spaced angles.
    ///
    /// Two shapes sampled this way share an index space, which is what lets
    /// [`morph`] interpolate them.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "sample counts are small and exact in f64"
    )]
    pub fn radii(&self) -> Vec<f64> {
        let outline = flatten(&self.outline());
        (0..SAMPLE_COUNT)
            .map(|index| {
                let angle = index as f64 / SAMPLE_COUNT as f64 * core::f64::consts::TAU;
                radius_at(&outline, angle)
            })
            .collect()
    }
}

/// The seven shapes Material's loading indicator cycles through, in Compose's
/// order, each sampled into the shared index space [`morph`] needs.
///
/// `Cookie9Sided`, `Sunny` and `Oval` come from parameters Compose documents.
/// The rest are its published control points fed through [`RoundedPolygon::repeated`].
///
/// Each shape is normalized to reach the same distance at its widest, as
/// `MaterialShapes` does with `RoundedPolygon.normalized()`. Their control
/// points are authored at whatever scale suited each shape, so without this the
/// sequence would swell and shrink as it morphed from one to the next.
#[must_use]
pub fn material_shape_sequence() -> Vec<Vec<f64>> {
    material_shape_polygons()
        .iter()
        .map(RoundedPolygon::radii)
        .map(|radii| normalized(&radii))
        .collect()
}

/// The seven polygons themselves, before they are sampled into radii.
fn material_shape_polygons() -> Vec<RoundedPolygon> {
    // Compose authors these around a centre of (0.5, 0.5); they are stated here
    // relative to the centre instead, so a shape is its own radius function.
    let soft_burst = RoundedPolygon::repeated(
        &[
            RoundedVertex::new(-0.307, -0.223, 0.053),
            RoundedVertex::new(-0.324, -0.445, 0.053),
        ],
        10,
        false,
    );
    let cookie_9_sided = RoundedPolygon::star(9, 0.8, 0.5).rotated(-core::f64::consts::FRAC_PI_2);
    let pentagon = RoundedPolygon::repeated(
        &[
            RoundedVertex::new(0.0, -0.509, 0.172),
            RoundedVertex::new(0.530, -0.135, 0.164),
            RoundedVertex::new(0.328, 0.470, 0.169),
        ],
        1,
        true,
    );
    let pill = RoundedPolygon::repeated(
        &[
            RoundedVertex::new(0.461, -0.461, 0.426),
            RoundedVertex::new(0.501, -0.072, 0.0),
            RoundedVertex::new(0.500, 0.109, 1.0),
        ],
        2,
        true,
    );
    let sunny = RoundedPolygon::star(8, 0.8, 0.15);
    let cookie_4_sided = RoundedPolygon::repeated(
        &[
            RoundedVertex::new(0.737, 0.736, 0.258),
            RoundedVertex::new(0.0, 0.418, 0.233),
        ],
        4,
        false,
    );
    let oval = RoundedPolygon::circle()
        .scaled_y(0.64)
        .rotated(-core::f64::consts::FRAC_PI_4);

    vec![
        soft_burst,
        cookie_9_sided,
        pentagon,
        pill,
        sunny,
        cookie_4_sided,
        oval,
    ]
}

/// Scales a shape's radii so its widest reach is exactly one.
fn normalized(radii: &[f64]) -> Vec<f64> {
    let widest = radii.iter().copied().fold(f64::MIN, f64::max);
    assert!(
        widest > 0.0,
        "a shape with no extent cannot be normalized: it collapsed to a point"
    );
    radii.iter().map(|radius| radius / widest).collect()
}

/// Interpolates between two shapes' radii.
#[must_use]
pub fn morph(from: &[f64], to: &[f64], progress: f64) -> Vec<f64> {
    from.iter()
        .zip(to)
        .map(|(from, to)| (to - from).mul_add(progress, *from))
        .collect()
}

/// Builds a closed path through `radii`, scaled by `scale` about `centre`.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "sample counts are small and exact in f64"
)]
pub fn radii_to_path(radii: &[f64], centre: Point, scale: f64) -> BezPath {
    let mut path = BezPath::new();
    for (index, radius) in radii.iter().enumerate() {
        let angle = index as f64 / radii.len() as f64 * core::f64::consts::TAU;
        let point = Point::new(
            radius.mul_add(angle.cos() * scale, centre.x),
            radius.mul_add(angle.sin() * scale, centre.y),
        );
        if index == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close_path();
    path
}

fn rotate(vertex: RoundedVertex, radians: f64) -> RoundedVertex {
    let (sin, cos) = radians.sin_cos();
    RoundedVertex {
        offset: Vec2::new(
            vertex.offset.x.mul_add(cos, -(vertex.offset.y * sin)),
            vertex.offset.x.mul_add(sin, vertex.offset.y * cos),
        ),
        rounding: vertex.rounding,
    }
}

/// Where a rounded corner starts and ends, and the centre of its arc.
///
/// The cut distance is clamped to half of each adjacent edge, so a generous
/// rounding on a short edge eats the edge rather than overshooting past its
/// neighbour and folding the outline inside out.
fn round_corner(
    previous: Vec2,
    vertex: RoundedVertex,
    next: Vec2,
) -> (Point, Point, Option<Point>) {
    let corner = vertex.offset;
    let to_previous = previous - corner;
    let to_next = next - corner;
    let previous_length = to_previous.hypot();
    let next_length = to_next.hypot();
    if vertex.rounding <= 0.0 || previous_length == 0.0 || next_length == 0.0 {
        let point = corner.to_point();
        return (point, point, None);
    }
    let previous_dir = to_previous / previous_length;
    let next_dir = to_next / next_length;
    let cos = previous_dir.dot(next_dir).clamp(-1.0, 1.0);
    let interior = cos.acos();
    let half = interior / 2.0;
    if half <= f64::EPSILON || (core::f64::consts::PI - interior).abs() <= f64::EPSILON {
        let point = corner.to_point();
        return (point, point, None);
    }
    // How far back along each edge the arc begins. Clamping to half an edge
    // keeps a generous rounding from overshooting its neighbour and folding the
    // outline inside out; the arc then has a smaller effective radius, which is
    // recovered from the clamped cut rather than assumed.
    let cut = (vertex.rounding / half.tan())
        .min(previous_length / 2.0)
        .min(next_length / 2.0);
    let effective_radius = cut * half.tan();
    let start = corner + previous_dir * cut;
    let end = corner + next_dir * cut;
    // The centre sits along the bisector at `hypot(cut, radius)`, the distance
    // that leaves the arc tangent to both edges.
    let bisector = (previous_dir + next_dir).normalize();
    let centre = corner + bisector * cut.hypot(effective_radius);
    (start.to_point(), end.to_point(), Some(centre.to_point()))
}

/// How closely an arc is approximated by cubics, in the shape's own units
/// where the outer radius is 1.
const ARC_TOLERANCE: f64 = 0.000_1;

/// Appends the shorter arc from `start` to `end` about `centre`.
fn append_arc(path: &mut BezPath, centre: Point, start: Point, end: Point) {
    let radius = (start - centre).hypot();
    if radius <= f64::EPSILON {
        path.line_to(end);
        return;
    }
    let start_angle = (start - centre).atan2();
    let end_angle = (end - centre).atan2();
    // Wrap the sweep into -PI..=PI so the arc takes the short way round.
    let raw = end_angle - start_angle;
    let sweep = core::f64::consts::TAU.mul_add(-(raw / core::f64::consts::TAU).round(), raw);
    let arc = vello::kurbo::Arc::new(centre, (radius, radius), start_angle, sweep, 0.0);
    arc.to_cubic_beziers(ARC_TOLERANCE, |p1, p2, p3| {
        path.curve_to(p1, p2, p3);
    });
}

/// Flattens a path into the polyline its outline traces.
fn flatten(path: &BezPath) -> Vec<Point> {
    const TOLERANCE: f64 = 0.001;
    let mut points = Vec::new();
    vello::kurbo::flatten(path.iter(), TOLERANCE, |element| match element {
        vello::kurbo::PathEl::MoveTo(point) | vello::kurbo::PathEl::LineTo(point) => {
            points.push(point);
        }
        _ => {}
    });
    points
}

/// Where a ray from the centre at `angle` crosses `outline`.
///
/// The shapes are star-convex, so there is exactly one crossing; a segment that
/// does not straddle the ray simply contributes nothing.
fn radius_at(outline: &[Point], angle: f64) -> f64 {
    let direction = Vec2::new(angle.cos(), angle.sin());
    let normal = Vec2::new(-direction.y, direction.x);
    let mut best = 0.0f64;
    for window in 0..outline.len() {
        let a = outline[window].to_vec2();
        let b = outline[(window + 1) % outline.len()].to_vec2();
        let side_a = a.dot(normal);
        let side_b = b.dot(normal);
        if (side_a > 0.0) == (side_b > 0.0) && side_a != 0.0 {
            continue;
        }
        let span = side_a - side_b;
        let t = if span.abs() <= f64::EPSILON {
            0.0
        } else {
            side_a / span
        };
        let crossing = a + (b - a) * t;
        let distance = crossing.dot(direction);
        if distance > best {
            best = distance;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::{
        RoundedPolygon, RoundedVertex, SAMPLE_COUNT, material_shape_polygons,
        material_shape_sequence, morph,
    };

    /// The angles of a polygon's vertices, in degrees, unwrapped so the walk
    /// reads as one pass around the centre rather than jumping at ±180°.
    fn unwrapped_degrees(polygon: &RoundedPolygon) -> Vec<f64> {
        let mut previous = f64::NEG_INFINITY;
        polygon
            .vertices
            .iter()
            .map(|vertex| {
                let degrees = vertex.offset.y.atan2(vertex.offset.x).to_degrees();
                // Lift each angle above the previous one so the walk reads as
                // one pass rather than wrapping at ±180°.
                let turns = ((previous - degrees) / 360.0).ceil().max(0.0);
                let degrees = 360.0f64.mul_add(turns, degrees);
                previous = degrees;
                degrees
            })
            .collect()
    }

    /// `doRepeat`'s mirrored branch, checked against the angles Compose's own
    /// arithmetic produces for `pentagon()`.
    ///
    /// Its first point sits at -90°, so each mirrored section reflects about
    /// the vertical, so the base angles -90, -14.3 and 55.1 come back as 124.9
    /// and 194.3 — the reflection of the first is dropped because it lands on
    /// that first point again.
    #[test]
    fn a_mirrored_repetition_reflects_about_the_first_points_angle() {
        let pentagon = RoundedPolygon::repeated(
            &[
                RoundedVertex::new(0.0, -0.509, 0.172),
                RoundedVertex::new(0.530, -0.135, 0.164),
                RoundedVertex::new(0.328, 0.470, 0.169),
            ],
            1,
            true,
        );
        let expected = [-90.0, -14.30, 55.10, 124.90, 194.30];
        let actual = unwrapped_degrees(&pentagon);
        assert_eq!(actual.len(), expected.len(), "pentagon has five corners");
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 0.05,
                "vertex angle was {actual}°, expected {expected}°"
            );
        }
    }

    /// Every polygon walks its vertices once around the centre, in order.
    ///
    /// This is what makes a shape a radius function, and it is the property a
    /// misordered repetition breaks: reflecting a two-repetition shape the
    /// wrong way still yields a closed outline of the right vertex count, but
    /// one that doubles back on itself and reads as a bowtie.
    #[test]
    fn every_polygon_walks_its_vertices_once_around_the_centre() {
        for (index, polygon) in material_shape_polygons().iter().enumerate() {
            let degrees = unwrapped_degrees(polygon);
            let sweep = degrees.last().expect("a polygon has vertices") - degrees[0];
            assert!(
                sweep < 360.0,
                "shape {index} doubles back: its vertices sweep {sweep}°"
            );
        }
    }

    /// A circle is the same distance from its centre whichever way you look.
    #[test]
    fn a_circle_samples_to_a_constant_radius() {
        let radii = RoundedPolygon::circle().radii();
        assert_eq!(radii.len(), SAMPLE_COUNT);
        for radius in radii {
            assert!((radius - 1.0).abs() < 0.01, "circle radius was {radius}");
        }
    }

    /// A star alternates between its outer radius and its inner one, and
    /// rounding pulls the extremes in rather than pushing them out.
    #[test]
    fn a_star_samples_between_its_two_radii() {
        let radii = RoundedPolygon::star(8, 0.8, 0.15).radii();
        let max = radii.iter().copied().fold(f64::MIN, f64::max);
        let min = radii.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            max <= 1.0 + 1e-6,
            "rounding must not push past the outer radius: {max}"
        );
        assert!(
            min >= 0.7,
            "rounding must not collapse the inner radius: {min}"
        );
        assert!(max > min, "a star is not a circle");
    }

    /// Scaling one axis makes the shape reach further along it than across it.
    #[test]
    fn scaling_one_axis_shows_up_in_the_samples() {
        let oval = RoundedPolygon::circle().scaled_y(0.64);
        let radii = oval.radii();
        // Index 0 looks along +x, a quarter of the way round looks along +y.
        let across = radii[0];
        let along = radii[SAMPLE_COUNT / 4];
        assert!(
            across > along,
            "the unscaled axis should reach further: {across} vs {along}"
        );
        assert!((along - 0.64).abs() < 0.02, "scaled axis was {along}");
    }

    /// Compose cycles seven shapes, and the morph only works because they all
    /// sample into the same index space.
    #[test]
    fn the_loading_sequence_is_seven_shapes_sharing_one_sample_space() {
        let shapes = material_shape_sequence();
        assert_eq!(shapes.len(), 7);
        for shape in &shapes {
            assert_eq!(shape.len(), SAMPLE_COUNT);
        }
    }

    /// Every shape closes and has real extent: a ray from the centre finds the
    /// outline whichever way it looks, and never past the unit radius. An
    /// outline that failed to close reads as a zero radius at some angle.
    #[test]
    fn every_loading_shape_is_closed_and_bounded() {
        for (index, shape) in material_shape_sequence().iter().enumerate() {
            let max = shape.iter().copied().fold(f64::MIN, f64::max);
            let min = shape.iter().copied().fold(f64::MAX, f64::min);
            assert!(max > 0.3, "shape {index} collapsed: max radius {max}");
            assert!(max <= 1.05, "shape {index} exceeded the unit radius: {max}");
            assert!(
                min > 0.0,
                "shape {index} does not close: zero radius somewhere"
            );
        }
    }

    /// Every shape reaches exactly as far as every other.
    ///
    /// The control points are authored at whatever scale suited each shape —
    /// the pentagon's sit around 0.51 from centre while the stars' reach 1.0 —
    /// so an un-normalized sequence swells and shrinks as it morphs. Nothing
    /// else here would catch that: each shape on its own is closed, bounded and
    /// different from its neighbour, which is exactly what a pulsing sequence
    /// looks like to those tests.
    #[test]
    fn every_loading_shape_reaches_the_same_distance_at_its_widest() {
        for (index, shape) in material_shape_sequence().iter().enumerate() {
            let widest = shape.iter().copied().fold(f64::MIN, f64::max);
            assert!(
                (widest - 1.0).abs() < 1e-9,
                "shape {index} reaches {widest} at its widest, not 1"
            );
        }
    }

    /// Consecutive shapes differ, or the cycle would visibly stall on a beat.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the divisor is SAMPLE_COUNT, which is 180"
    )]
    #[test]
    fn consecutive_loading_shapes_actually_differ() {
        let shapes = material_shape_sequence();
        for index in 0..shapes.len() {
            let current = &shapes[index];
            let next = &shapes[(index + 1) % shapes.len()];
            let difference: f64 = current
                .iter()
                .zip(next)
                .map(|(a, b)| (a - b).abs())
                .sum::<f64>()
                / current.len() as f64;
            assert!(
                difference > 0.01,
                "shapes {index} and {} are nearly identical: {difference}",
                (index + 1) % shapes.len()
            );
        }
    }

    /// A morph starts at one shape, ends at the other, and stays between them.
    #[test]
    fn a_morph_travels_between_its_two_shapes() {
        let from = RoundedPolygon::star(9, 0.8, 0.5).radii();
        let to = RoundedPolygon::circle().radii();

        assert_eq!(morph(&from, &to, 0.0), from);
        assert_eq!(morph(&from, &to, 1.0), to);

        let middle = morph(&from, &to, 0.5);
        for ((from, to), middle) in from.iter().zip(&to).zip(&middle) {
            let (low, high) = if from <= to { (from, to) } else { (to, from) };
            assert!(
                middle >= low && middle <= high,
                "{middle} left the range {low}..{high}"
            );
        }
    }
}
