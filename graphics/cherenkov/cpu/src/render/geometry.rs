//! Winding-resolved fill and stroke geometry, reused across lowerings.
//!
//! A path whose flattened segments overlap has to be swept
//! ([`cherenkov::lowering::resolve_winding`]) before its signed area is
//! accumulated, and the sweep is the dominant cost of lowering such a path.
//! Its result depends only on the flattened content, which is a pure
//! function of the shape, the stroke style and the device transform. The
//! cache keys on exactly those, compared bit for bit, so content recorded
//! again (or drawn twice) at the same placement reuses the swept edges. The
//! key shares the recording's path allocation rather than holding a second
//! copy of the flattened content.
//!
//! Only swept geometry is kept: input the pre-sweep test proves disjoint
//! costs about as much to flatten and test again as to look up.
//!
//! The cache holds no geometry of its own: an entry lives while a retained
//! realization still draws its edges, and [`GeometryCache::evict`] drops
//! the rest after every frame, so its bound is one entry per live swept
//! draw. Under critical memory pressure [`GeometryCache::clear`] empties it.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrayvec::ArrayVec;
use cherenkov::kurbo::{self, Affine, PathEl, Point};
use cherenkov::lowering::Winding;
use cherenkov::{FillRule, ShapeData};
use rustc_hash::{FxHashMap, FxHasher};

use super::raster::Edge;

/// One cached draw: its key and its resolved edges.
struct Entry {
    shape: ShapeData,
    stroke: Option<kurbo::Stroke>,
    transform: [u64; 6],
    edges: Arc<[Edge]>,
    rule: FillRule,
}

/// Swept draw geometry by content hash. A lookup compares the whole key;
/// a different key with the same hash replaces the entry.
#[derive(Default)]
pub struct GeometryCache {
    entries: FxHashMap<u64, Entry>,
}

impl GeometryCache {
    /// The device edges of `shape` under `transform` — its outline stroked
    /// by `stroke` when set — with the rule they fill under. `flatten`
    /// produces the flattened edges and their rule on a miss; `None` means
    /// the shape has no area. Edges proved disjoint are returned as
    /// flattened and not kept.
    pub fn resolve(
        &mut self,
        shape: &ShapeData,
        stroke: Option<&kurbo::Stroke>,
        transform: Affine,
        flatten: impl FnOnce() -> Option<(Vec<Edge>, FillRule)>,
    ) -> Option<(Arc<[Edge]>, FillRule)> {
        let cacheable = !matches!(shape, ShapeData::Ellipse(_));
        let transform = transform.as_coeffs().map(f64::to_bits);
        let key = cacheable.then(|| key_hash(shape, stroke, &transform));
        if let Some(key) = key
            && let Some(entry) = self.entries.get_mut(&key)
            && entry.matches(shape, stroke, &transform)
        {
            // Hold the newest recording's path, so an older recording's
            // allocation is released with it.
            entry.shape.clone_from(shape);
            return Some((entry.edges.clone(), entry.rule));
        }
        let (edges, rule) = flatten()?;
        let (edges, rule, swept) = match cherenkov::lowering::resolve_winding(&edges, rule) {
            Winding::Disjoint => (Arc::from(edges), rule, false),
            Winding::Unchanged => (Arc::from(edges), rule, true),
            Winding::Resolved(resolved) => (
                resolved
                    .into_iter()
                    .map(|(x0, y0, x1, y1)| Edge { x0, y0, x1, y1 })
                    .collect(),
                FillRule::NonZero,
                true,
            ),
        };
        if swept && let Some(key) = key {
            self.entries.insert(
                key,
                Entry {
                    shape: shape.clone(),
                    stroke: stroke.cloned(),
                    transform,
                    edges: edges.clone(),
                    rule,
                },
            );
        }
        Some((edges, rule))
    }

    /// Drops every entry no retained realization draws any more.
    pub fn evict(&mut self) {
        self.entries
            .retain(|_, entry| Arc::strong_count(&entry.edges) > 1);
    }

    /// Drops every entry and the table itself.
    pub fn clear(&mut self) {
        self.entries = FxHashMap::default();
    }

    /// Heap bytes the cache holds beyond what retained realizations and
    /// recordings already account for: the table, and any edges, path
    /// elements or dash pattern only an entry still references.
    pub fn bytes(&self) -> u64 {
        let table = self.entries.capacity() * (size_of::<(u64, Entry)>() + size_of::<u8>());
        let mut bytes = table as u64;
        for entry in self.entries.values() {
            if Arc::strong_count(&entry.edges) == 1 {
                bytes += (entry.edges.len() * size_of::<Edge>()) as u64;
            }
            if let ShapeData::Path { elements, .. } = &entry.shape
                && Arc::strong_count(elements) == 1
            {
                bytes += (elements.len() * size_of::<PathEl>()) as u64;
            }
            if let Some(stroke) = &entry.stroke
                && stroke.dash_pattern.spilled()
            {
                bytes += (stroke.dash_pattern.capacity() * size_of::<f64>()) as u64;
            }
        }
        bytes
    }
}

impl Entry {
    fn matches(
        &self,
        shape: &ShapeData,
        stroke: Option<&kurbo::Stroke>,
        transform: &[u64; 6],
    ) -> bool {
        self.transform == *transform
            && match (&self.stroke, stroke) {
                (None, None) => true,
                (Some(a), Some(b)) => stroke_words(a).eq(stroke_words(b)),
                _ => false,
            }
            && shape_eq(&self.shape, shape)
    }
}

fn key_hash(shape: &ShapeData, stroke: Option<&kurbo::Stroke>, transform: &[u64; 6]) -> u64 {
    let mut hasher = FxHasher::default();
    transform.hash(&mut hasher);
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

fn shape_eq(a: &ShapeData, b: &ShapeData) -> bool {
    match (a, b) {
        (
            ShapeData::Path {
                elements: ea,
                rule: ra,
            },
            ShapeData::Path {
                elements: eb,
                rule: rb,
            },
        ) => {
            ra == rb
                && (Arc::ptr_eq(ea, eb)
                    || (ea.len() == eb.len()
                        && ea.iter().zip(eb.iter()).all(|(x, y)| {
                            let ((vx, px), (vy, py)) = (element_parts(x), element_parts(y));
                            vx == vy
                                && px.iter().zip(&py).all(|(p, q)| {
                                    p.x.to_bits() == q.x.to_bits() && p.y.to_bits() == q.y.to_bits()
                                })
                        })))
        }
        (ShapeData::Path { .. }, _) | (_, ShapeData::Path { .. }) => false,
        (a, b) => scalar_words(a) == scalar_words(b),
    }
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
