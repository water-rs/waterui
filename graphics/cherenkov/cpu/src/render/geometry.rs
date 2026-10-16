//! Winding-resolved fill and stroke geometry, carried from a realization
//! that is about to be dropped or rebuilt to the lowering that replaces it.
//!
//! A path whose flattened segments may overlap has to be swept
//! ([`cherenkov::lowering::resolve_winding`]) before its signed area is
//! accumulated, and the sweep is the dominant cost of lowering such a path.
//! Its result depends only on the flattened content, which is a pure
//! function of the shape, the stroke style and the device transform. A
//! static scene records its content again every frame, which drops every
//! realization; a placement whose clip changes rebuilds one. Before either
//! happens, the swept edges of the realization are offered here under that
//! key, and the lowering that follows takes them for a draw whose key
//! matches bit for bit instead of sweeping again.
//!
//! Only swept geometry is offered: input the pre-sweep test proves
//! disjoint costs about as much to flatten and test again as to look up.
//!
//! The table lives for one frame: [`GeometryCache::finish_frame`] drops it
//! after the frame's lowering, so it holds no memory between frames and its
//! bound is the swept draws replaced or rebuilt in one frame. Under
//! critical memory pressure [`GeometryCache::clear`] drops it at once.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrayvec::ArrayVec;
use cherenkov::kurbo::{self, Affine, PathEl, Point};
use cherenkov::lowering::Winding;
use cherenkov::{FillRule, ShapeData};
use rustc_hash::{FxHashMap, FxHasher};

use super::raster::Edge;

/// One offered draw: its key and its swept edges.
struct Entry {
    shape: ShapeData,
    stroke: Option<kurbo::Stroke>,
    transform: [u64; 6],
    edges: Arc<[Edge]>,
    rule: FillRule,
}

/// Swept draw geometry offered for the current frame, by content hash. A
/// lookup compares the whole key; a different key with the same hash
/// replaces the entry.
#[derive(Default)]
pub struct GeometryCache {
    entries: FxHashMap<u64, Entry>,
}

/// Geometry produced for a draw: its device edges, the rule they fill under,
/// and whether the winding sweep produced or confirmed them.
pub struct Resolved {
    pub edges: Arc<[Edge]>,
    pub rule: FillRule,
    pub swept: bool,
}

impl GeometryCache {
    /// The device edges of `shape` under `transform` — its outline stroked
    /// by `stroke` when set. An offered entry with the same key is reused;
    /// otherwise `flatten` produces the flattened edges and their rule, and
    /// the winding is resolved. `None` means the shape has no area.
    pub fn resolve(
        &self,
        shape: &ShapeData,
        stroke: Option<&kurbo::Stroke>,
        transform: Affine,
        flatten: impl FnOnce() -> Option<(Vec<Edge>, FillRule)>,
    ) -> Option<Resolved> {
        if !self.entries.is_empty()
            && let Some(key) = Key::new(shape, stroke, transform)
        {
            let hash = key.digest();
            if let Some(entry) = self.entries.get(&hash)
                && entry.matches(&key)
            {
                return Some(Resolved {
                    edges: entry.edges.clone(),
                    rule: entry.rule,
                    swept: true,
                });
            }
        }
        let (edges, rule) = flatten()?;
        Some(match cherenkov::lowering::resolve_winding(&edges, rule) {
            Winding::Disjoint => Resolved {
                edges: Arc::from(edges),
                rule,
                swept: false,
            },
            Winding::Unchanged => Resolved {
                edges: Arc::from(edges),
                rule,
                swept: true,
            },
            Winding::Resolved(resolved) => Resolved {
                edges: resolved
                    .into_iter()
                    .map(|(x0, y0, x1, y1)| Edge { x0, y0, x1, y1 })
                    .collect(),
                rule: FillRule::NonZero,
                swept: true,
            },
        })
    }

    /// Offers the swept edges of a draw of `shape` under `transform`
    /// (stroked by `stroke` when set) whose realization is being dropped or
    /// rebuilt, for the rest of the frame.
    pub fn offer(
        &mut self,
        shape: &ShapeData,
        stroke: Option<&kurbo::Stroke>,
        transform: Affine,
        edges: &Arc<[Edge]>,
        rule: FillRule,
    ) {
        if let Some(key) = Key::new(shape, stroke, transform) {
            self.entries.insert(
                key.digest(),
                Entry {
                    shape: shape.clone(),
                    stroke: stroke.cloned(),
                    transform: key.transform,
                    edges: edges.clone(),
                    rule,
                },
            );
        }
    }

    /// Drops the frame's table: geometry not taken by now is not reused.
    pub fn finish_frame(&mut self) {
        self.entries = FxHashMap::default();
    }

    /// Drops every entry and the table itself.
    pub fn clear(&mut self) {
        self.entries = FxHashMap::default();
    }

    /// Heap bytes the table holds, and any edges, path elements or dash
    /// pattern only an entry still references. Zero between frames.
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

/// A draw's geometry key, borrowed. Ellipses have none: kurbo keeps their
/// affine private, so they cannot be compared bit for bit.
struct Key<'a> {
    shape: &'a ShapeData,
    stroke: Option<&'a kurbo::Stroke>,
    transform: [u64; 6],
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
            transform: transform.as_coeffs().map(f64::to_bits),
        })
    }

    fn digest(&self) -> u64 {
        key_hash(self.shape, self.stroke, &self.transform)
    }
}

impl Entry {
    fn matches(&self, key: &Key<'_>) -> bool {
        self.transform == key.transform
            && match (&self.stroke, key.stroke) {
                (None, None) => true,
                (Some(a), Some(b)) => stroke_words(a).eq(stroke_words(b)),
                _ => false,
            }
            && shape_eq(&self.shape, key.shape)
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
