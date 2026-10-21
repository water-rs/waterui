//! Retained fill and stroke geometry in a draw's
//! translation-normalized [`Frame`], and the per-frame table that carries it
//! from a realization about to be dropped to the lowering that replaces it.
//!
//! Flattening and the disjointness proof depend on the shape, stroke and
//! linear transform in its normalized frame. Translations reuse these raw
//! tiled edges; coverage resolves overlapping windings in each raster band.
//!
//! A static scene records its content again every frame, which drops every
//! realization. Before that happens, the retained edges of the realization
//! are offered here under their key, and the lowering that follows takes
//! them for a draw whose key matches bit for bit instead of flattening,
//! testing and quantizing again.
//!
//! Every cacheable draw is offered, disjoint or not: retained geometry is
//! quantized at build time, so a hit also saves the tile build, and a
//! lookup was already paid whenever the table held any entry.
//!
//! The table lives for one frame: it is allocated at the frame's first
//! cacheable offer and [`GeometryCache::finish_frame`] drops it after the
//! frame's lowering, so it holds no memory between frames and its bound is
//! the draws replaced in one frame. Under critical memory pressure
//! [`GeometryCache::clear`] drops it at once.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrayvec::ArrayVec;
use cherenkov::kurbo::{self, Affine, BezPath, PathEl, Point, Vec2};
use cherenkov::{FillRule, ShapeData};
use rustc_hash::{FxHashMap, FxHasher};

use super::coverage::Overlap;
use super::raster::Edge;
use super::tiles::{EdgeScratch, EdgeTiles, Fixed};

/// One offered draw: its key's span in the frame's word arena, its path
/// elements when it is one, its retained edges and the disjointness proof. About 48 bytes, so an insert or a
/// rehash move is a constant-size copy that never calls memcpy.
struct Entry {
    /// `(start, len)` of the key's words in `GeometryCache::words`.
    key: (u32, u32),
    /// The shape's path elements; a path compares by element, so its
    /// contents stay out of the arena.
    path: Option<Arc<[PathEl]>>,
    edges: EdgeTiles,
    rule: FillRule,
    overlap: Overlap,
}

/// Retained draw geometry offered for the current frame, by content hash. A
/// lookup compares the whole key; a different key with the same hash
/// replaces the entry. The compact key words live in `words`, an arena
/// that lives exactly as long as the table.
#[derive(Default)]
pub struct GeometryCache {
    entries: FxHashMap<u64, Entry>,
    /// The frame's key-word arena; every entry's `key` spans into it.
    words: Vec<u64>,
}

/// The offers of one content about to be dropped. The table and the key-word
/// arena are reserved for its `n` ops at its first cacheable offer, so a
/// content that offers nothing (glyph runs, images, uncacheable shapes)
/// allocates nothing.
pub struct Offers<'a> {
    cache: &'a mut GeometryCache,
    /// Ops still to reserve for; zero once reserved.
    reserve: usize,
}

/// Geometry produced for a draw: its edges in the draw's [`Frame`], the rule
/// they fill under, and the disjointness proof.
pub struct Resolved {
    pub edges: EdgeTiles,
    pub rule: FillRule,
    pub overlap: Overlap,
}

impl GeometryCache {
    /// The edges of `shape` under `transform` in its [`Frame`] — its outline
    /// stroked by `stroke` when set. An offered entry with the same key is
    /// reused; otherwise `flatten` produces the flattened frame edges and
    /// their rule, disjointness is tested and the retained form is built
    /// in `scratch`. `None` means the shape has no area.
    pub fn resolve(
        &self,
        shape: &ShapeData,
        stroke: Option<&kurbo::Stroke>,
        transform: Affine,
        scratch: &mut EdgeScratch,
        flatten: impl FnOnce() -> Option<(Vec<Edge>, FillRule)>,
    ) -> Option<Resolved> {
        if !self.entries.is_empty()
            && let Some(key) = Key::new(shape, stroke, transform)
        {
            let hash = key.digest();
            if let Some(entry) = self.entries.get(&hash)
                && entry.matches(&key, &self.words)
            {
                return Some(Resolved {
                    edges: entry.edges.clone(),
                    rule: entry.rule,
                    overlap: entry.overlap,
                });
            }
        }
        let (edges, rule) = flatten()?;
        Some(Resolved {
            overlap: Overlap::of(&edges),
            edges: EdgeTiles::new(scratch, edges.iter().copied()),
            rule,
        })
    }

    /// Starts offering the draws of a content of `n` ops.
    pub const fn offers(&mut self, n: usize) -> Offers<'_> {
        Offers {
            cache: self,
            reserve: n,
        }
    }

    /// Drops the frame's table: geometry not taken by now is not reused.
    pub fn finish_frame(&mut self) {
        self.entries = FxHashMap::default();
        self.words = Vec::new();
    }

    /// Drops every entry and the table itself.
    pub fn clear(&mut self) {
        self.entries = FxHashMap::default();
        self.words = Vec::new();
    }

    /// Heap bytes the table holds, and any edges or path elements only an
    /// entry still references. Zero between frames.
    pub fn bytes(&self) -> u64 {
        let table = self.entries.capacity() * (size_of::<(u64, Entry)>() + size_of::<u8>());
        let mut bytes = (table + self.words.capacity() * size_of::<u64>()) as u64;
        for entry in self.entries.values() {
            if entry.edges.strong_count() == 1 {
                bytes += entry.edges.heap_bytes();
            }
            if let Some(elements) = &entry.path
                && Arc::strong_count(elements) == 1
            {
                bytes += (elements.len() * size_of::<PathEl>()) as u64;
            }
        }
        bytes
    }
}

impl Offers<'_> {
    /// Offers the frame edges of a draw of `shape` under `transform`
    /// (stroked by `stroke` when set) whose realization is being dropped,
    /// for the rest of the frame.
    pub fn offer(
        &mut self,
        shape: &ShapeData,
        stroke: Option<&kurbo::Stroke>,
        transform: Affine,
        edges: &EdgeTiles,
        rule: FillRule,
        overlap: Overlap,
    ) {
        let Some(key) = Key::new(shape, stroke, transform) else {
            return;
        };
        if self.reserve != 0 {
            let n = std::mem::take(&mut self.reserve);
            self.cache.entries.reserve(n);
            self.cache.words.reserve(n.saturating_mul(16));
        }
        let start = u32::try_from(self.cache.words.len()).expect("key words fit u32");
        self.cache.words.extend(key.words());
        let len = u32::try_from(self.cache.words.len()).expect("key words fit u32") - start;
        let path = match shape {
            ShapeData::Path { elements, .. } => Some(elements.clone()),
            _ => None,
        };
        self.cache.entries.insert(
            key.digest(),
            Entry {
                key: (start, len),
                path,
                edges: edges.clone(),
                rule,
                overlap,
            },
        );
    }
}

/// A draw's translation-normalized frame. Geometry and its disjointness
/// proof depend only on shape, stroke and the transform's linear part.
/// Device placement adds the fixed offset without changing retained edges.
///
/// The shape is first moved so that the corner of its box — its bounds or a
/// path's control box, grown by the stroke's reach — sits at the origin, the space strokes are
/// outlined in. The linear part then maps it, and an integral shift puts
/// the mapped box at `1` or more on both axes: like geometry on a surface,
/// every frame coordinate is positive and away from zero, where the
/// pre-sweep disjointness proof requires ordinates of at least `2^-6`.
pub struct Frame {
    corner: Vec2,
    map: Affine,
    /// The device position of the frame's origin.
    pub offset: Fixed,
}

impl Frame {
    /// The frame of `shape`, stroked by `stroke` when set, drawn under
    /// `transform`.
    pub fn new(shape: &ShapeData, stroke: Option<&kurbo::Stroke>, transform: Affine) -> Self {
        let reach = stroke.map_or(0.0, stroke_reach);
        let bounds = control_bounds(shape).inflate(reach, reach);
        let corner = bounds.origin().to_vec2();
        let [a, b, c, d, _, _] = transform.as_coeffs();
        let linear = Affine::new([a, b, c, d, 0.0, 0.0]);
        let size = bounds.size();
        let low = [
            (size.width, 0.0),
            (0.0, size.height),
            (size.width, size.height),
        ]
        .into_iter()
        .map(|p| linear * Point::from(p))
        .fold(Point::ORIGIN, |m, p| Point::new(m.x.min(p.x), m.y.min(p.y)));
        let shift = Vec2::new(1.0 - low.x.floor(), 1.0 - low.y.floor());
        let at = transform * corner.to_point() - shift;
        Self {
            corner,
            map: Affine::translate(shift) * linear,
            offset: Fixed::new([at.x, at.y]),
        }
    }

    /// `path`, in shape space, moved so the box's corner sits at the
    /// origin: the space strokes are outlined in.
    pub fn anchored(&self, mut path: BezPath) -> BezPath {
        path.apply_affine(Affine::translate(-self.corner));
        path
    }

    /// An anchored `path` mapped into the frame.
    pub fn map(&self, path: BezPath) -> BezPath {
        self.map * path
    }
}

/// A box containing `shape`: a path's control box, which holds its curves
/// without solving for their extrema, or the shape's own bounds.
fn control_bounds(shape: &ShapeData) -> kurbo::Rect {
    let ShapeData::Path { elements, .. } = shape else {
        return shape.bounds();
    };
    let mut points = elements.iter().flat_map(|el| element_parts(el).1);
    points.next().map_or(kurbo::Rect::ZERO, |first| {
        points.fold(kurbo::Rect::from_points(first, first), |r, p| r.union_pt(p))
    })
}

/// How far `stroke`'s outline reaches from its centreline: half the width,
/// times the miter limit for miter joins and `√2` for square caps.
fn stroke_reach(stroke: &kurbo::Stroke) -> f64 {
    let mut factor: f64 = 1.0;
    if stroke.join == kurbo::Join::Miter {
        factor = factor.max(stroke.miter_limit);
    }
    if [stroke.start_cap, stroke.end_cap].contains(&kurbo::Cap::Square) {
        factor = factor.max(std::f64::consts::SQRT_2);
    }
    stroke.width * 0.5 * factor
}

/// Whether `a` and `b` have the same linear part, bit for bit: the frame
/// geometry of a draw under one is the frame geometry under the other.
pub fn same_linear(a: Affine, b: Affine) -> bool {
    linear_bits(a) == linear_bits(b)
}

/// The linear part of `transform` as exact words.
fn linear_bits(transform: Affine) -> [u64; 4] {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    [a, b, c, d].map(f64::to_bits)
}

/// A draw's geometry key, borrowed: the frame geometry does not depend on
/// the transform's translation. Ellipses have none: kurbo keeps their
/// affine private, so they cannot be compared bit for bit.
struct Key<'a> {
    shape: &'a ShapeData,
    stroke: Option<&'a kurbo::Stroke>,
    linear: [u64; 4],
}

impl<'a> Key<'a> {
    fn new(
        shape: &'a ShapeData,
        stroke: Option<&'a kurbo::Stroke>,
        transform: Affine,
    ) -> Option<Self> {
        (!matches!(shape, ShapeData::Ellipse(_))).then(|| Self {
            shape,
            stroke,
            linear: linear_bits(transform),
        })
    }

    fn digest(&self) -> u64 {
        key_hash(self.shape, self.stroke, &self.linear)
    }

    /// The key as a word stream: the four linear words, the stroke tag
    /// and its words, then the shape's tag and rule or its scalar fields.
    /// A path's elements are not words: they compare through
    /// `Entry::path`.
    fn words(&self) -> impl Iterator<Item = u64> + '_ {
        self.linear
            .into_iter()
            .chain([u64::from(self.stroke.is_some())])
            .chain(self.stroke.into_iter().flat_map(stroke_words))
            .chain(shape_words(self.shape))
    }
}

impl Entry {
    /// Whether the entry was offered under `key`: the key's words compare
    /// one at a time — never a slice compare — and a path's elements
    /// compare by `Arc` identity or element-wise.
    fn matches(&self, key: &Key<'_>, words: &[u64]) -> bool {
        let (start, len) = (self.key.0 as usize, self.key.1 as usize);
        words[start..start + len].iter().copied().eq(key.words())
            && match (&self.path, key.shape) {
                (Some(a), ShapeData::Path { elements, .. }) => {
                    Arc::ptr_eq(a, elements) || elements_eq(a, elements)
                }
                (Some(_), _) | (None, ShapeData::Path { .. }) => false,
                (None, _) => true,
            }
    }
}

fn key_hash(shape: &ShapeData, stroke: Option<&kurbo::Stroke>, linear: &[u64; 4]) -> u64 {
    let mut hasher = FxHasher::default();
    linear.hash(&mut hasher);
    match stroke {
        Some(stroke) => {
            1u8.hash(&mut hasher);
            for word in stroke_words(stroke) {
                word.hash(&mut hasher);
            }
        }
        None => 0u8.hash(&mut hasher),
    }
    shape_hash(shape, &mut hasher);
    hasher.finish()
}

/// Every field of `stroke` as exact words.
fn stroke_words(stroke: &kurbo::Stroke) -> impl Iterator<Item = u64> + '_ {
    [
        stroke.width.to_bits(),
        stroke.miter_limit.to_bits(),
        stroke.dash_offset.to_bits(),
        stroke.join as u64,
        stroke.start_cap as u64,
        stroke.end_cap as u64,
    ]
    .into_iter()
    .chain(stroke.dash_pattern.iter().map(|d| d.to_bits()))
}

/// The scalar fields of a shape without a path, as exact words.
fn scalar_words(shape: &ShapeData) -> ArrayVec<u64, 14> {
    let point = |p: Point| [p.x.to_bits(), p.y.to_bits()];
    let rect = |r: kurbo::Rect| {
        [
            r.x0.to_bits(),
            r.y0.to_bits(),
            r.x1.to_bits(),
            r.y1.to_bits(),
        ]
    };
    let radii = |r: kurbo::RoundedRectRadii| {
        [
            r.top_left.to_bits(),
            r.top_right.to_bits(),
            r.bottom_right.to_bits(),
            r.bottom_left.to_bits(),
        ]
    };
    let mut words = ArrayVec::new();
    match shape {
        ShapeData::Rect(r) => {
            words.push(0);
            words.extend(rect(*r));
        }
        ShapeData::RoundedRect(r) => {
            words.push(1);
            words.extend(rect(r.rect()));
            words.extend(radii(r.radii()));
        }
        ShapeData::Continuous(c) => {
            words.push(2);
            words.extend(rect(c.rect));
            words.extend(radii(c.radii));
            words.push(c.smoothing.to_bits());
        }
        ShapeData::Circle(c) => {
            words.push(3);
            words.extend(point(c.center));
            words.push(c.radius.to_bits());
        }
        ShapeData::Line(l) => {
            words.push(4);
            words.extend(point(l.p0));
            words.extend(point(l.p1));
        }
        ShapeData::Ellipse(_) | ShapeData::Path { .. } => {
            unreachable!("ellipses are not cached and paths compare by element")
        }
    }
    words
}

fn shape_hash(shape: &ShapeData, hasher: &mut FxHasher) {
    match shape {
        ShapeData::Path { elements, rule } => {
            5u64.hash(hasher);
            (*rule as u8).hash(hasher);
            for el in elements.iter() {
                let (verb, points) = element_parts(el);
                verb.hash(hasher);
                for p in points {
                    p.x.to_bits().hash(hasher);
                    p.y.to_bits().hash(hasher);
                }
            }
        }
        shape => scalar_words(shape).hash(hasher),
    }
}

/// The shape's key words: a path's tag and fill rule — its elements
/// compare through the entry's `path` — or `scalar_words` for a shape
/// without a path.
fn shape_words(shape: &ShapeData) -> ArrayVec<u64, 14> {
    let mut words = ArrayVec::new();
    match shape {
        ShapeData::Path { rule, .. } => {
            words.push(5);
            words.push(*rule as u64);
        }
        shape => words = scalar_words(shape),
    }
    words
}

/// Two path element lists, element by element.
fn elements_eq(a: &[PathEl], b: &[PathEl]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| {
            let ((vx, px), (vy, py)) = (element_parts(x), element_parts(y));
            vx == vy
                && px
                    .iter()
                    .zip(&py)
                    .all(|(p, q)| p.x.to_bits() == q.x.to_bits() && p.y.to_bits() == q.y.to_bits())
        })
}

/// A path element's verb and its points.
fn element_parts(el: &PathEl) -> (u8, ArrayVec<Point, 3>) {
    let mut points = ArrayVec::new();
    let verb = match *el {
        PathEl::MoveTo(p) => {
            points.push(p);
            0
        }
        PathEl::LineTo(p) => {
            points.push(p);
            1
        }
        PathEl::QuadTo(c, p) => {
            points.extend([c, p]);
            2
        }
        PathEl::CurveTo(c0, c1, p) => {
            points.extend([c0, c1, p]);
            3
        }
        PathEl::ClosePath => 4,
    };
    (verb, points)
}
