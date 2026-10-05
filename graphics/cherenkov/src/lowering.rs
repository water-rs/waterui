//! Shared command-span bookkeeping for retained backend lowering.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::ops::Range;

use kurbo::Affine;

use crate::{BlendMode, Command, Dirty, DisplayList, FillRule, Group, ShapeData};

pub mod projective;
pub mod shadow;

/// `shape`'s outline flattened to `tolerance` where it is curved, with its
/// fill rule; `None` for a shape without area (a line).
#[must_use]
pub fn shape_outline(shape: &ShapeData, tolerance: f64) -> Option<(kurbo::BezPath, FillRule)> {
    use kurbo::Shape as _;
    match shape {
        ShapeData::Rect(r) => Some((r.to_path(tolerance), FillRule::NonZero)),
        ShapeData::RoundedRect(r) => Some((r.to_path(tolerance), FillRule::NonZero)),
        ShapeData::Continuous(c) => Some((c.to_path(tolerance), FillRule::NonZero)),
        ShapeData::Circle(c) => Some((c.to_path(tolerance), FillRule::NonZero)),
        ShapeData::Ellipse(e) => Some((e.to_path(tolerance), FillRule::NonZero)),
        ShapeData::Line(_) => None,
        ShapeData::Path { elements, rule } => {
            Some((kurbo::BezPath::from_vec(elements.to_vec()), *rule))
        }
    }
}

/// A backend operation whose scope indices can be relocated during a patch.
pub trait Operation {
    /// Matching close index for a scope opener.
    fn end_mut(&mut self) -> Option<&mut u32>;
    /// Whether replacing this operation preserves the command/pass structure.
    fn same_structure(&self, other: &Self) -> bool;
}

/// The backend-specific part of content lowering. Layer state is deliberately
/// absent: it is applied to the retained operations at composition time.
pub trait Compiler {
    /// Retained operation representation.
    type Op: Operation;
    /// Backend lowering error.
    type Error;
    /// Lower one drawing command (scope and picture commands are walked here).
    ///
    /// # Errors
    /// Returns a backend error for unsupported or invalid content.
    fn draw(
        &mut self,
        command: &Command,
        ambient: Affine,
        ops: &mut Vec<Self::Op>,
    ) -> Result<(), Self::Error>;
    /// Lower a command with its index in the root source list, when it belongs
    /// to that list. Nested pictures and expanded glyphs have no root index.
    ///
    /// # Errors
    /// Propagates the backend compiler's error.
    fn draw_at(
        &mut self,
        command: &Command,
        ambient: Affine,
        ops: &mut Vec<Self::Op>,
        _source: Option<usize>,
    ) -> Result<(), Self::Error> {
        self.draw(command, ambient, ops)
    }

    /// Open a clip scope; its matching end is filled by the walker.
    ///
    /// # Errors
    /// Returns a backend error for unsupported or invalid content.
    fn clip(&mut self, shape: &ShapeData, ambient: Affine) -> Result<Self::Op, Self::Error>;
    /// Open a group, or omit isolation when the group is a pass-through.
    /// `isolate` forces isolation for a group that would otherwise compose
    /// in place: a blended descendant group must composite against this
    /// group's own raster, not the parent's.
    ///
    /// # Errors
    /// Returns a backend error for unsupported or invalid content.
    fn group(&mut self, group: &Group, isolate: bool) -> Result<Option<Self::Op>, Self::Error>;
    /// Close a retained scope.
    fn end(&mut self) -> Self::Op;
}

#[derive(Clone, Debug)]
struct Span {
    ops: Range<u32>,
    ambient: Affine,
}

/// Retained backend operations and the source commands that produced them.
#[derive(Debug)]
pub struct Lowered<T> {
    /// Draws and paired scopes in painter order.
    pub ops: Vec<T>,
    spans: Vec<Span>,
}

impl<T: Operation> Lowered<T> {
    /// Resolve all commands once, recording their operation spans.
    ///
    /// # Errors
    /// Propagates the backend compiler's error.
    pub fn full<C: Compiler<Op = T>>(
        list: &DisplayList,
        compiler: &mut C,
    ) -> Result<Self, C::Error> {
        let mut lowered = Self {
            ops: Vec::with_capacity(list.len()),
            spans: Vec::with_capacity(list.len()),
        };
        lowered.refill(list, compiler)?;
        Ok(lowered)
    }

    fn refill<C: Compiler<Op = T>>(
        &mut self,
        list: &DisplayList,
        compiler: &mut C,
    ) -> Result<(), C::Error> {
        self.ops.clear();
        self.spans.clear();
        walk(
            list,
            0..list.len(),
            Affine::IDENTITY,
            compiler,
            &mut self.ops,
            &mut self.spans,
            true,
        )
    }

    /// Patch dirty command spans in place. `None` means the operation count or
    /// pass structure changed and this layer was rebuilt. Otherwise the returned
    /// operation ranges identify precisely which device realizations expired.
    ///
    /// # Errors
    /// Propagates the backend compiler's error.
    ///
    /// # Panics
    /// When dirty ranges refer to another list or the lowered op count exceeds u32.
    pub fn patch<C: Compiler<Op = T>>(
        &mut self,
        list: &DisplayList,
        dirty: &Dirty,
        compiler: &mut C,
    ) -> Result<Option<Vec<Range<usize>>>, C::Error> {
        let mut patches = Vec::new();
        for range in dirty.ranges() {
            let range = range.start as usize..range.end as usize;
            let mut ops = Vec::new();
            let mut spans = Vec::with_capacity(range.len());
            walk(
                list,
                range.clone(),
                self.spans[range.start].ambient,
                compiler,
                &mut ops,
                &mut spans,
                true,
            )?;
            let lo = self.spans[range.start].ops.start as usize;
            let hi = self.spans[range.end - 1].ops.end as usize;
            let old_spans = &self.spans[range.clone()];
            if hi - lo != ops.len()
                || old_spans
                    .iter()
                    .zip(&spans)
                    .any(|(a, b)| a.ops.len() != b.ops.len())
                || self.ops[lo..hi]
                    .iter()
                    .zip(&ops)
                    .any(|(a, b)| !a.same_structure(b))
            {
                *self = Self::full(list, compiler)?;
                return Ok(None);
            }
            let offset = u32::try_from(lo).expect("op index fits u32");
            for op in &mut ops {
                if let Some(end) = op.end_mut() {
                    *end += offset;
                }
            }
            for (dst, src) in self.ops[lo..hi].iter_mut().zip(ops) {
                *dst = src;
            }
            for (dst, mut src) in self.spans[range].iter_mut().zip(spans) {
                src.ops = src.ops.start + offset..src.ops.end + offset;
                *dst = src;
            }
            patches.push(lo..hi);
        }
        Ok(Some(patches))
    }
}

impl<T> Lowered<T> {
    /// Heap bytes of the operation and span buffers, including each
    /// operation's own allocations as reported by `nested`.
    fn heap_bytes(&self, mut nested: impl FnMut(&T) -> u64) -> u64 {
        let mut bytes = (self.ops.capacity() * size_of::<T>()
            + self.spans.capacity() * size_of::<Span>()) as u64;
        for op in &self.ops {
            bytes += nested(op);
        }
        bytes
    }
}

/// Append a nested picture or an expanded colour glyph. Its operations belong
/// to the enclosing source command's span.
///
/// # Errors
/// Propagates the backend compiler's error.
pub fn append<C: Compiler>(
    list: &DisplayList,
    ambient: Affine,
    compiler: &mut C,
    ops: &mut Vec<C::Op>,
) -> Result<(), C::Error> {
    let mut spans = Vec::with_capacity(list.len());
    walk(
        list,
        0..list.len(),
        ambient,
        compiler,
        ops,
        &mut spans,
        false,
    )
}

fn walk<C: Compiler>(
    list: &DisplayList,
    range: Range<usize>,
    ambient: Affine,
    compiler: &mut C,
    ops: &mut Vec<C::Op>,
    spans: &mut Vec<Span>,
    root: bool,
) -> Result<(), C::Error> {
    let mut i = range.start;
    while i < range.end {
        let start = u32::try_from(ops.len()).expect("op index fits u32");
        let scope = match &list.commands()[i] {
            Command::BeginTransform { transform, end } => {
                Some((*end as usize, ambient * *transform, None))
            }
            Command::BeginClip { shape, end } => {
                Some((*end as usize, ambient, Some(compiler.clip(shape, ambient)?)))
            }
            Command::BeginGroup { group, end } => {
                // A pass-through group still isolates when a descendant
                // group blends: without it the descendant's composite would
                // land on the parent's raster, not the group's own.
                let isolate = group.blend == BlendMode::Normal
                    && group.opacity >= 1.0
                    && group.filter.is_none()
                    && crate::display_list::blends_within(list, i + 1..*end as usize);
                Some((*end as usize, ambient, compiler.group(group, isolate)?))
            }
            Command::Picture { picture, transform } => {
                append(picture.display_list(), ambient * *transform, compiler, ops)?;
                None
            }
            Command::End => unreachable!("validated scopes consume their end"),
            command => {
                compiler.draw_at(command, ambient, ops, root.then_some(i))?;
                None
            }
        };
        if let Some((end, inner, opener)) = scope {
            let emitted = opener.is_some();
            ops.extend(opener);
            spans.push(Span {
                ops: start..u32::try_from(ops.len()).expect("op index fits u32"),
                ambient,
            });
            walk(list, i + 1..end, inner, compiler, ops, spans, root)?;
            let close = u32::try_from(ops.len()).expect("op index fits u32");
            if emitted {
                *ops[start as usize]
                    .end_mut()
                    .expect("scope opener has an end") = close;
                ops.push(compiler.end());
            }
            spans.push(Span {
                ops: close..u32::try_from(ops.len()).expect("op index fits u32"),
                ambient: inner,
            });
            i = end + 1;
        } else {
            spans.push(Span {
                ops: start..u32::try_from(ops.len()).expect("op index fits u32"),
                ambient,
            });
            i += 1;
        }
    }
    Ok(())
}

/// A retained device realization. Invalidation keeps the storage available for
/// the next patch, while a structural rebuild drops the command layout.
#[derive(Debug)]
pub struct Realization<T> {
    /// Device data and its placement key.
    pub data: Option<T>,
    /// Whether the source operations still match this realization.
    pub valid: bool,
}

impl<T> Default for Realization<T> {
    fn default() -> Self {
        Self {
            data: None,
            valid: false,
        }
    }
}

/// One layer's display list, accumulated dirty commands, and retained output.
#[derive(Debug)]
pub struct Content<O, E> {
    live: bool,
    rebuild: bool,
    list: crate::Picture,
    dirty: Dirty,
    lowered: Option<Lowered<O>>,
    emissions: Vec<Realization<E>>,
}

impl<O: Operation, E> Content<O, E> {
    /// Start an unprepared layer.
    #[must_use]
    pub fn new(list: crate::Picture) -> Self {
        Self {
            live: true,
            rebuild: false,
            list,
            dirty: Dirty::default(),
            lowered: None,
            emissions: Vec::new(),
        }
    }

    /// Replace all source commands while keeping the lowering buffers available.
    pub fn replace(&mut self, list: crate::Picture) -> crate::Picture {
        self.live = true;
        let previous = std::mem::replace(&mut self.list, list);
        self.dirty = Dirty::default();
        self.emissions.clear();
        if let Some(lowered) = &mut self.lowered {
            lowered.ops.clear();
            lowered.spans.clear();
        }
        self.rebuild = true;
        previous
    }

    /// Extract the retained source picture.
    #[must_use]
    pub fn into_picture(self) -> crate::Picture {
        self.list
    }

    /// Retain immutable picture content, which cannot accept slot updates.
    #[must_use]
    pub fn picture(picture: crate::Picture) -> Self {
        Self {
            live: false,
            ..Self::new(picture)
        }
    }

    /// Discard compiled resource references after a resource is removed.
    pub fn invalidate(&mut self) {
        self.lowered = None;
        self.emissions.clear();
    }

    /// Whether the current source commands, slot updates applied and nested
    /// pictures included, sample `resource`.
    #[must_use]
    pub fn references(&self, resource: crate::ResourceId) -> bool {
        self.list.display_list().references(resource)
    }

    /// Current prepared operations and the source they index. Dirty or
    /// unprepared content has no current operations. Backends can inspect
    /// stable content without compiling it a second time.
    #[must_use]
    pub fn current(&self) -> Option<(&[O], &DisplayList)> {
        if self.rebuild || !self.dirty.is_empty() {
            return None;
        }
        self.lowered
            .as_ref()
            .map(|lowered| (lowered.ops.as_slice(), self.list.display_list()))
    }

    /// Discard compiled resource references after image `id`'s pixels were
    /// replaced behind the same id. Lowering resolves an image's dimensions,
    /// and on some backends its storage, into the retained operations, so
    /// content that samples `id` is lowered again. Content with pending slot
    /// updates is discarded too: its operations were compiled from operands
    /// the updates have since replaced, and those may still reference `id`.
    /// Returns whether anything was discarded.
    pub fn invalidate_image(&mut self, id: crate::ImageId) -> bool {
        let stale = self.lowered.is_some()
            && (!self.dirty.is_empty() || self.references(crate::ResourceId::Image(id)));
        if stale {
            self.invalidate();
        }
        stale
    }

    /// Accumulate every commit arriving before the next render.
    ///
    /// # Panics
    /// When slot updates target immutable picture content.
    pub fn update(&mut self, updates: Vec<crate::SlotUpdate>) {
        assert!(self.live, "slot update targets immutable picture content");
        self.dirty.union(self.list.apply(updates));
    }

    /// Resolve dirty source commands; return the number lowered.
    ///
    /// # Errors
    /// Propagates the backend compiler's error.
    ///
    /// # Panics
    /// When the command count exceeds u32.
    pub fn prepare<C: Compiler<Op = O>>(&mut self, compiler: &mut C) -> Result<u32, C::Error> {
        let count;
        if self.rebuild
            && let Some(lowered) = &mut self.lowered
        {
            lowered.refill(self.list.display_list(), compiler)?;
            self.emissions
                .resize_with(lowered.ops.len(), Realization::default);
            self.dirty = Dirty::default();
            self.rebuild = false;
            return Ok(
                u32::try_from(self.list.display_list().len()).expect("command count fits u32")
            );
        }
        if let Some(lowered) = &mut self.lowered {
            if self.dirty.is_empty() {
                return Ok(0);
            }
            if let Some(patches) = lowered.patch(self.list.display_list(), &self.dirty, compiler)? {
                for range in patches {
                    for emission in &mut self.emissions[range] {
                        emission.valid = false;
                    }
                }
                count = self.dirty.ranges().iter().map(|r| r.end - r.start).sum();
            } else {
                self.emissions.clear();
                self.emissions
                    .resize_with(lowered.ops.len(), Realization::default);
                count =
                    u32::try_from(self.list.display_list().len()).expect("command count fits u32");
            }
        } else {
            let lowered = Lowered::full(self.list.display_list(), compiler)?;
            self.emissions
                .resize_with(lowered.ops.len(), Realization::default);
            self.lowered = Some(lowered);
            count = u32::try_from(self.list.display_list().len()).expect("command count fits u32");
        }
        self.dirty = Dirty::default();
        self.rebuild = false;
        Ok(count)
    }

    /// The prepared ops and their independently mutable device realizations.
    ///
    /// # Panics
    /// When composition runs before preparation.
    pub fn prepared(&mut self) -> (&[O], &mut [Realization<E>]) {
        let (ops, emissions, _) = self.prepared_source();
        (ops, emissions)
    }

    /// Prepared operations, their device output, and the source they may index.
    ///
    /// # Panics
    /// When composition runs before preparation.
    pub fn prepared_source(&mut self) -> (&[O], &mut [Realization<E>], &DisplayList) {
        (
            &self
                .lowered
                .as_ref()
                .expect("content prepared before composition")
                .ops,
            &mut self.emissions,
            self.list.display_list(),
        )
    }

    /// Retained device realizations, including invalid entries awaiting replacement.
    /// Backends use this read-only view for residency accounting without preparing content.
    #[must_use]
    pub fn realizations(&self) -> &[Realization<E>] {
        &self.emissions
    }

    /// Release device coverage without re-lowering content on the next frame.
    pub fn trim(&mut self) {
        self.emissions
            .iter_mut()
            .for_each(|entry| *entry = Realization::default());
    }

    /// Heap bytes retained by this content: the source list, the lowered
    /// operations and the device realizations. `command`, `op` and
    /// `emission` report each element's own heap allocations.
    ///
    /// The source list is an `Arc`: contents shared with another layer's
    /// picture are counted at each retain.
    pub fn heap_bytes(
        &self,
        mut command: impl FnMut(&Command) -> u64,
        mut op: impl FnMut(&O) -> u64,
        mut emission: impl FnMut(&E) -> u64,
    ) -> u64 {
        let mut bytes = self.list.display_list().heap_bytes(&mut command)
            + (self.emissions.capacity() * size_of::<Realization<E>>()) as u64;
        if let Some(lowered) = &self.lowered {
            bytes += lowered.heap_bytes(&mut op);
        }
        for entry in &self.emissions {
            if let Some(data) = &entry.data {
                bytes += emission(data);
            }
        }
        bytes
    }
}

/// Comparison epsilon for sweep positions and boundary times.
const EPS: f64 = 1e-9;

/// One non-horizontal segment normalized to run top-to-bottom in `f64`.
struct Seg {
    /// X at the top endpoint.
    x0: f64,
    /// Top y.
    y0: f64,
    /// Bottom y.
    y1: f64,
    /// dx/dy.
    slope: f64,
    /// Signed winding deposit: +1 downward, -1 upward.
    dir: f64,
}

impl Seg {
    fn x_at(&self, y: f64) -> f64 {
        self.slope.mul_add(y - self.y0, self.x0)
    }
}

/// Crossing ordinate of the pair `(p, q)` evaluated in operand order.
/// The expression is not symmetric in its operands, so an event is
/// always evaluated in the pair's current `active` order — the order
/// the old per-band midpoint sort would produce, which is the operand
/// order its crossing scan used.
fn xing_y(p: &Seg, q: &Seg) -> f64 {
    // p.x0 + p.slope*(y-p.y0) == q.x0 + q.slope*(y-q.y0)
    q.slope.mul_add(q.y0, p.slope.mul_add(-p.y0, p.x0) - q.x0) / (q.slope - p.slope)
}

/// Bound on the distance between a pair event's stored ordinate and the
/// true crossing, given `m_glob` — the maximum intermediate magnitude
/// over all segments. It grows with `|appr|` more slowly than `appr`
/// itself, so a heap head beyond a target plus this bound ends the
/// scan: nothing after it can reach the target either.
fn xing_err(appr: f64, m_glob: f64) -> f64 {
    (m_glob + appr.abs()) * (64.0 * f64::EPSILON)
}

/// Crossing ordinate of the pair adjacent in `active` order `(p, q)`,
/// pushed as a pending event. Below a pair's crossing the smaller-slope
/// segment sits left, so a pair stored `(p, q)` with `sp.slope < sq.slope`
/// is in post-cross orientation and also gets a `posts` entry: a later
/// band whose midpoint drops below the crossing may have to revert it.
/// Events carry a `det` flag: only a pair whose slopes differ by more
/// than `EPS` can produce a band split (the old scan skipped the rest),
/// but every non-parallel pair still changes order when its crossing
/// passes, so near-parallel pairs get order events without the flag.
/// Exactly parallel pairs never cross: their per-band order comes from
/// the midpoint keys alone — rounding makes coincident pairs flip
/// arbitrarily — and they go on `eqs` to be re-checked each band.
#[expect(
    clippy::cast_possible_truncation,
    reason = "segment counts stay under u32"
)]
fn push_xing(
    xings: &mut BinaryHeap<Reverse<(Split, u32, u32, u8)>>,
    posts: &mut BinaryHeap<(Split, u32, u32)>,
    eqs: &mut Vec<(u32, u32)>,
    segs: &[Seg],
    p: usize,
    q: usize,
    m_glob: f64,
) {
    let (sp, sq) = (&segs[p], &segs[q]);
    let ds = sp.slope - sq.slope;
    if ds == 0.0 {
        // Parallel segments keep a constant x offset, so only a pair
        // whose offset is within rounding distance of zero can ever
        // flip its midpoint-key order — those go on `eqs` to be
        // re-checked each band; anything clearly apart keeps its
        // order permanently and needs no event.
        let gap = sp.slope.mul_add(sq.y0 - sp.y0, sp.x0 - sq.x0);
        let bound = (m_glob + gap.abs()) * (256.0 * f64::EPSILON);
        if gap.abs() <= bound {
            eqs.push((p as u32, q as u32));
        }
        return;
    }
    let appr = xing_y(sp, sq);
    if !appr.is_finite() {
        return;
    }
    let det = u8::from(ds.abs() > EPS);
    xings.push(Reverse((Split(appr), p as u32, q as u32, det)));
    if sp.slope < sq.slope {
        // Popped once the band's midpoint reaches below the ordinate:
        // the pair could need reverting to pre-cross order. The stored
        // threshold is `appr` plus the ordinate's error bound — after
        // the first check in order, the entry is re-keyed to that `ym`
        // so only a still-lower band re-examines it.
        let err = if det != 0 {
            xing_err(appr, m_glob)
        } else {
            (m_glob + appr.abs()) * 1e-6
        };
        posts.push((Split(appr + err), p as u32, q as u32));
    }
}

/// Whether the pair — adjacent left-to-right as `(l, r)` — sorts
/// `(r, l)` at `ym`: the exact comparison the old midpoint sort
/// applied.
#[expect(
    clippy::float_cmp,
    reason = "exact key equality mirrors the sort's total_cmp tie-break"
)]
fn post_cross_at(l: &Seg, r: &Seg, ym: f64) -> bool {
    let (xl, xr) = (l.x_at(ym), r.x_at(ym));
    xr.total_cmp(&xl) == Ordering::Less
        || (xr == xl && r.slope.total_cmp(&l.slope) == Ordering::Less)
}

/// Target capacity of an `Active` chunk: insertions and removals
/// shift at most this many elements instead of the whole list.
const CHUNK: usize = 64;

/// Inside/outside state of a winding prefix under the fill rule.
fn inside_at(w: f64, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => w != 0.0,
        FillRule::EvenOdd => w.rem_euclid(2.0) > 0.5,
    }
}

/// Chunked order-statistics list of the sweep's live segments.
/// Keeping the list as fixed-capacity chunks bounds each admission
/// and retirement to an in-chunk shift, and each chunk caches its
/// winding contribution — direction sum plus the local prefix range —
/// so the emit pass can evaluate the winding level at a rank, or
/// enumerate the ranks where the prefix sits at a boundary level,
/// without rescanning the list.
struct Active {
    /// Chunks in list order. A chunk's `id` is its slot in
    /// `order_pos`; splitting inserts a chunk without renumbering the
    /// ids `chunk_of` records.
    chunks: Vec<Chunk>,
    /// Segment → chunk id while admitted, `usize::MAX` otherwise.
    chunk_of: Vec<usize>,
    /// Chunk id → position in `chunks`.
    order_pos: Vec<u32>,
    /// Next chunk id to hand out.
    next_id: u32,
    /// Fenwick tree over the chunks' (element count, direction sum)
    /// pairs — `ft[0]` unused; `ft[k]` covers the `lowbit(k)` chunks
    /// ending at `k`. Point updates are O(log n) so rank queries stay
    /// cheap through insert/remove without a per-edit rebuild.
    ft: Vec<(u32, f64)>,
    len: usize,
    /// Empty tombstone chunks kept in place — removing one would
    /// renumber `order_pos` for everything after it. Compacted once
    /// they dominate.
    empty: usize,
}

#[derive(Default)]
struct Chunk {
    /// Stable identifier into `order_pos`.
    id: u32,
    els: Vec<u32>,
    /// Sum of the elements' winding directions.
    ds: f64,
    /// Min and max of the local winding prefix — the running sum of
    /// directions after each element, starting from zero.
    mn: f64,
    mx: f64,
}

impl Active {
    fn new(cap: usize) -> Self {
        Self {
            chunks: Vec::new(),
            chunk_of: vec![usize::MAX; cap],
            order_pos: Vec::new(),
            next_id: 0,
            ft: vec![(0, 0.0)],
            len: 0,
            empty: 0,
        }
    }

    /// Recompute a chunk's direction sum and local prefix range.
    fn recalc(&mut self, segs: &[Seg], c: usize) {
        let ch = &mut self.chunks[c];
        ch.ds = 0.0;
        ch.mn = 0.0;
        ch.mx = 0.0;
        for &e in &ch.els {
            ch.ds += segs[e as usize].dir;
            ch.mn = ch.mn.min(ch.ds);
            ch.mx = ch.mx.max(ch.ds);
        }
    }

    /// Fenwick point update: add `(dc, dd)` to chunk `pos`.
    fn fadd(&mut self, pos: usize, dc: i32, dd: f64) {
        let mut k = pos + 1;
        while k < self.ft.len() {
            self.ft[k].0 = self.ft[k].0.wrapping_add_signed(dc);
            self.ft[k].1 += dd;
            k += k & k.wrapping_neg();
        }
    }

    /// Fenwick prefix over chunks `0..pos` — element count and
    /// direction sum of the chunks before `pos`.
    fn fsum(&self, pos: usize) -> (u32, f64) {
        let mut k = pos;
        let (mut c, mut d) = (0u32, 0.0);
        while k > 0 {
            c += self.ft[k].0;
            d += self.ft[k].1;
            k &= k - 1;
        }
        (c, d)
    }

    /// Number of chunks whose combined length is `<= k` — the index
    /// of the chunk holding rank `k`.
    fn flower(&self, k: u32) -> usize {
        let mut pos = 0usize;
        let mut k = k;
        let mut bit = self.ft.len().next_power_of_two() >> 1;
        while bit > 0 {
            let next = pos + bit;
            if next < self.ft.len() && self.ft[next].0 <= k {
                k -= self.ft[next].0;
                pos = next;
            }
            bit >>= 1;
        }
        pos
    }

    /// Rebuild the Fenwick tree after the `chunks` vec's shape
    /// changed (chunk insert, removal or compaction).
    fn rebuild_ft(&mut self) {
        let n = self.chunks.len();
        self.ft.clear();
        self.ft.resize(n + 1, (0, 0.0));
        for i in 1..=n {
            let ch = &self.chunks[i - 1];
            self.ft[i].0 += u32::try_from(ch.els.len()).expect("a chunk holds at most 2*CHUNK");
            self.ft[i].1 += ch.ds;
            let j = i + (i & i.wrapping_neg());
            if j <= n {
                let v = self.ft[i];
                self.ft[j].0 += v.0;
                self.ft[j].1 += v.1;
            }
        }
    }

    /// Position of the chunk holding rank `k`, and `k`'s offset in it.
    fn locate(&self, k: usize) -> (usize, usize) {
        let c = self.flower(u32::try_from(k).expect("rank fits u32"));
        let base = self.fsum(c).0;
        (c, k - base as usize)
    }

    /// Rank of a live segment: chunk base plus its offset inside.
    fn rank(&self, e: u32) -> usize {
        let c = self.order_pos[self.chunk_of[e as usize]] as usize;
        self.fsum(c).0 as usize
            + self.chunks[c]
                .els
                .iter()
                .position(|&x| x == e)
                .expect("live segment sits in its chunk")
    }

    /// Segment at a rank.
    fn at(&self, r: usize) -> u32 {
        let (c, o) = self.locate(r);
        self.chunks[c].els[o]
    }

    /// Allocate a chunk id.
    const fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// First rank whose midpoint key `(x_at(ym), slope)` is not below
    /// `(xi, si)` — the position the old midpoint sort produced.
    #[expect(
        clippy::float_cmp,
        reason = "exact key equality mirrors the sort's tie-break"
    )]
    fn slot(&self, segs: &[Seg], ym: f64, xi: f64, si: f64) -> usize {
        let (mut lo, mut hi) = (0usize, self.len);
        while lo < hi {
            let mid = lo.midpoint(hi);
            let j = self.at(mid) as usize;
            let xj = segs[j].x_at(ym);
            if xj < xi || (xj == xi && segs[j].slope <= si) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Insert a segment at a rank, splitting an overfull chunk.
    fn insert(&mut self, segs: &[Seg], at: usize, e: u32) {
        if self.chunks.is_empty() {
            let id = self.new_id();
            self.order_pos.push(0);
            self.chunks.push(Chunk {
                id,
                ..Chunk::default()
            });
            self.rebuild_ft();
        }
        let (c, o) = if at >= self.len {
            (
                self.chunks.len() - 1,
                self.chunks.last().map_or(0, |ch| ch.els.len()),
            )
        } else {
            self.locate(at)
        };
        if self.chunks[c].els.is_empty() && self.empty > 0 {
            self.empty -= 1;
        }
        self.chunks[c].els.insert(o, e);
        self.len += 1;
        self.chunk_of[e as usize] = self.chunks[c].id as usize;
        self.fadd(c, 1, segs[e as usize].dir);
        if self.chunks[c].els.len() > 2 * CHUNK {
            let tail: Vec<u32> = self.chunks[c].els.split_off(CHUNK);
            let id = self.new_id();
            self.chunks.insert(
                c + 1,
                Chunk {
                    id,
                    els: tail,
                    ..Chunk::default()
                },
            );
            if self.order_pos.len() <= id as usize {
                self.order_pos.resize(id as usize + 1, 0);
            }
            for p in c + 1..self.chunks.len() {
                self.order_pos[self.chunks[p].id as usize] =
                    u32::try_from(p).expect("chunk index fits u32");
            }
            for &x in &self.chunks[c + 1].els {
                self.chunk_of[x as usize] = id as usize;
            }
            self.recalc(segs, c + 1);
            self.recalc(segs, c);
            self.rebuild_ft();
        }
        self.recalc(segs, c);
    }

    /// Remove a live segment; tombstones the chunk if it empties.
    fn remove(&mut self, segs: &[Seg], e: u32) {
        let c = self.order_pos[self.chunk_of[e as usize]] as usize;
        let o = self.chunks[c]
            .els
            .iter()
            .position(|&x| x == e)
            .expect("live segment sits in its chunk");
        self.chunks[c].els.remove(o);
        self.len -= 1;
        self.chunk_of[e as usize] = usize::MAX;
        self.fadd(c, -1, -segs[e as usize].dir);
        if self.chunks[c].els.is_empty() {
            self.empty += 1;
            if self.empty * 4 > self.chunks.len() && self.chunks.len() > 8 {
                self.compact(segs);
                return;
            }
        }
        self.recalc(segs, c);
    }

    /// Rebuild the list from the live elements, dropping tombstones.
    fn compact(&mut self, segs: &[Seg]) {
        let mut els: Vec<u32> = Vec::with_capacity(self.len);
        for ch in &self.chunks {
            els.extend_from_slice(&ch.els);
        }
        self.chunks.clear();
        self.order_pos.clear();
        self.next_id = 0;
        for (i, part) in els.chunks(CHUNK).enumerate() {
            let id = self.new_id();
            for &x in part {
                self.chunk_of[x as usize] = id as usize;
            }
            self.order_pos
                .push(u32::try_from(i).expect("chunk index fits u32"));
            self.chunks.push(Chunk {
                id,
                els: part.to_vec(),
                ..Chunk::default()
            });
            let last = self.chunks.len() - 1;
            self.recalc(segs, last);
        }
        self.empty = 0;
        self.rebuild_ft();
    }

    /// Swap the adjacent pair at ranks `r`, `r + 1` in place.
    fn swap_adj(&mut self, segs: &[Seg], r: usize) {
        let (c0, o0) = self.locate(r);
        let (c1, o1) = self.locate(r + 1);
        if c0 == c1 {
            self.chunks[c0].els.swap(o0, o1);
        } else {
            let x = self.chunks[c0].els[o0];
            let y = self.chunks[c1].els[o1];
            self.chunks[c0].els[o0] = y;
            self.chunks[c1].els[o1] = x;
            self.chunk_of[x as usize] = self.chunks[c1].id as usize;
            self.chunk_of[y as usize] = self.chunks[c0].id as usize;
            let d = segs[y as usize].dir - segs[x as usize].dir;
            self.fadd(c0, 0, d);
            self.fadd(c1, 0, -d);
            self.recalc(segs, c1);
        }
        self.recalc(segs, c0);
    }

    /// Global min and max winding prefix over all positions.
    fn w_minmax(&self) -> (f64, f64) {
        let mut wmin = 0.0f64;
        let mut wmax = 0.0f64;
        let mut base = 0.0;
        for ch in &self.chunks {
            if !ch.els.is_empty() {
                wmin = wmin.min(base + ch.mn.min(0.0));
                wmax = wmax.max(base + ch.mx.max(0.0));
            }
            base += ch.ds;
        }
        (wmin, wmax)
    }
}

/// Fix `active`'s inversions against the order at `ym`, which is the
/// order the old per-band sort produced. `xings` carries an event for
/// every pair adjacent in list order, keyed by the crossing ordinate
/// evaluated in that order; popping while an entry could still reach
/// `ym` covers every pair that must invert here. A live `(l, r)` pair
/// inverted at `ym` swaps in place; the swap leaves the pair ordered
/// `(r, l)` with the same crossing still ahead — its own re-queue plus
/// a `posts` entry let a later band with a lower midpoint revert it,
/// and the new outer neighbours get their own events. Entries that are
/// no longer adjacent are stale and dropped. `posts` is a max-heap of
/// swapped pairs: a band whose `ym` sits below a swapped pair's
/// crossing needs the pair reverted to pre-cross order, so entries
/// above `ym` are popped and un-swapped when their ordering at `ym`
/// says so.
#[expect(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    reason = "the sweep state is one borrow; segment counts stay under u32; eqs grows by push"
)]
fn drain_xings(
    xings: &mut BinaryHeap<Reverse<(Split, u32, u32, u8)>>,
    posts: &mut BinaryHeap<(Split, u32, u32)>,
    eqs: &mut Vec<(u32, u32)>,
    segs: &[Seg],
    active: &mut Active,
    ya: f64,
    ym: f64,
    m_glob: f64,
    swapped: &mut Vec<u32>,
) {
    // Alternate the passes until none moves anything: a forward swap
    // can join a pair that must revert, and a revert can join a pair
    // that must swap — the cascades settle the list into the exact
    // order the old sort produced at `ym`. Each swap reports its
    let mut check_eqs = true;
    loop {
        let mut moved = false;
        let mut requeue: Vec<Reverse<(Split, u32, u32, u8)>> = Vec::new();
        while let Some(&Reverse((Split(appr), l, r, det))) = xings.peek() {
            // For split-detectable pairs the stored ordinate sits within
            // `xing_err` of the true crossing; near-parallel pairs get a
            // wider bound, since the error scales with 1/|slope diff|.
            let err = if det != 0 {
                xing_err(appr, m_glob)
            } else {
                (m_glob + appr.abs()) * 1e-6
            };
            if appr - err > ym {
                break;
            }
            xings.pop();
            let (l, r) = (l as usize, r as usize);
            if active.chunk_of[l] == usize::MAX || active.chunk_of[r] == usize::MAX {
                continue;
            }
            let (pl, pr) = (active.rank(l as u32), active.rank(r as u32));
            if pl + 1 != pr {
                continue;
            }
            if post_cross_at(&segs[l], &segs[r], ym) {
                moved = true;
                active.swap_adj(segs, pl);
                swapped.push(l as u32);
                swapped.push(r as u32);
                if pl > 0 {
                    push_xing(
                        xings,
                        posts,
                        eqs,
                        segs,
                        active.at(pl - 1) as usize,
                        r,
                        m_glob,
                    );
                }
                let back = xing_y(&segs[r], &segs[l]);
                xings.push(Reverse((Split(back), r as u32, l as u32, det)));
                posts.push((Split(ym), r as u32, l as u32));
                if pr + 1 < active.len {
                    push_xing(
                        xings,
                        posts,
                        eqs,
                        segs,
                        l,
                        active.at(pr + 1) as usize,
                        m_glob,
                    );
                }
            } else if segs[l].slope > segs[r].slope || appr > ya {
                // A pre-cross pair is still waiting on its crossing, and
                // a crossing at or above `ya` can still split this band
                // — both stay queued. A post-cross pair already ordered
                // with its crossing behind the band is done: `posts`
                // covers any later dip below the crossing.
                requeue.push(Reverse((Split(appr), l as u32, r as u32, det)));
            }
        }
        for e in requeue {
            xings.push(e);
        }
        // Bands can revisit a lower `ym` after a split, reverting pairs
        // swapped or joined into post-cross order at a higher one: pop
        // every such pair whose crossing is below this `ym` and
        // un-swap the ones still out of order.
        // Post-cross entries carry the ordinate below which their pair
        // needs re-checking — a fresh pair's crossing plus its error
        // bound, or once verified the `ym` it checked out at — so only
        // a band dipping below that pops them. A live pair still out
        // of order un-swaps; a correct one re-keys to this `ym`.
        let mut repost: Vec<(Split, u32, u32)> = Vec::new();
        while let Some(&(Split(t), b, a)) = posts.peek() {
            if t <= ym {
                break;
            }
            posts.pop();
            let (b, a) = (b as usize, a as usize);
            if active.chunk_of[b] == usize::MAX || active.chunk_of[a] == usize::MAX {
                continue;
            }
            let (pb, pa) = (active.rank(b as u32), active.rank(a as u32));
            if pb + 1 != pa {
                continue;
            }
            if post_cross_at(&segs[b], &segs[a], ym) {
                moved = true;
                active.swap_adj(segs, pb);
                swapped.push(b as u32);
                swapped.push(a as u32);
                if pb > 0 {
                    push_xing(
                        xings,
                        posts,
                        eqs,
                        segs,
                        active.at(pb - 1) as usize,
                        a,
                        m_glob,
                    );
                }
                push_xing(xings, posts, eqs, segs, a, b, m_glob);
                if pa + 1 < active.len {
                    push_xing(
                        xings,
                        posts,
                        eqs,
                        segs,
                        b,
                        active.at(pa + 1) as usize,
                        m_glob,
                    );
                }
            } else {
                repost.push((Split(ym), b as u32, a as u32));
            }
        }
        for e in repost {
            posts.push(e);
        }
        // Exactly-parallel pairs have no crossing to key an event on,
        // yet the old sort re-evaluated their midpoint keys every band
        // — coincident pairs flip on rounding noise — so each still-
        // adjacent one is re-checked against the order at `ym` here.
        // One pass per band suffices — a pair's ordering depends only
        // on its own two keys — so later iterations rescan only after
        // the pass itself swapped something.
        if check_eqs {
            check_eqs = false;
            let mut i = 0;
            while i < eqs.len() {
                let (e0, e1) = (eqs[i].0 as usize, eqs[i].1 as usize);
                let (p0, p1) = (active.chunk_of[e0], active.chunk_of[e1]);
                if p0 == usize::MAX || p1 == usize::MAX {
                    eqs.swap_remove(i);
                    continue;
                }
                let (p0, p1) = (active.rank(e0 as u32), active.rank(e1 as u32));
                let (l, r, pl, pr) = if p0 + 1 == p1 {
                    (e0, e1, p0, p1)
                } else if p1 + 1 == p0 {
                    (e1, e0, p1, p0)
                } else {
                    eqs.swap_remove(i);
                    continue;
                };
                if post_cross_at(&segs[l], &segs[r], ym) {
                    moved = true;
                    check_eqs = true;
                    active.swap_adj(segs, pl);
                    swapped.push(l as u32);
                    swapped.push(r as u32);
                    eqs[i] = (r as u32, l as u32);
                    if pl > 0 {
                        push_xing(
                            xings,
                            posts,
                            eqs,
                            segs,
                            active.at(pl - 1) as usize,
                            r,
                            m_glob,
                        );
                    }
                    if pr + 1 < active.len {
                        push_xing(
                            xings,
                            posts,
                            eqs,
                            segs,
                            l,
                            active.at(pr + 1) as usize,
                            m_glob,
                        );
                    }
                }
                i += 1;
            }
        }
        if !moved {
            break;
        }
    }
}

/// A band split point pending consumption, ordered by `f64::total_cmp`.
/// Only finite values are ever pushed: a crossing is inserted strictly
/// inside its band.
#[derive(Clone, Copy, Debug)]
struct Split(f64);

impl PartialEq for Split {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Split {}

impl PartialOrd for Split {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Split {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Winding resolution before signed-area accumulation.
///
/// The area accumulator reads each pixel's net winding: overlapping
/// same-sign regions clamp (`0.6 + 0.6 -> 1.0`, losing the union's true
/// 0.84) and opposite-sign regions cancel — both wrong under `NonZero`,
/// and the raw winding can exceed any rule's range on self-overlapping
/// outlines (kurbo stroke output overlaps at joins and caps).
/// `resolve_winding` sweeps the flattened device-space segments left to
/// right inside crossing-free horizontal bands and emits each band's
/// inside/outside boundary edges, so every covered region carries
/// winding `1` and the accumulator becomes exact under `NonZero`.
///
/// `None` means the input had no overlap: the caller keeps the original
/// segments and rule untouched, leaving every non-overlapping scene
/// bit-identical.
#[expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    reason = "one sweep per design; emitted coordinates fit f32; key equality mirrors the sort; the winding prefix is a sum of unit directions, so its boundary test is an exact comparison"
)]
pub fn resolve_winding(
    segments: &[(f32, f32, f32, f32)],
    rule: FillRule,
) -> Option<Vec<(f32, f32, f32, f32)>> {
    let mut segs: Vec<Seg> = Vec::with_capacity(segments.len());
    for &(x0, y0, x1, y1) in segments {
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            continue;
        }
        let (x0, y0, x1, y1) = (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
        #[expect(clippy::float_cmp, reason = "horizontal edges carry no area")]
        if y0 == y1 {
            continue;
        }
        if y0 < y1 {
            segs.push(Seg {
                x0,
                y0,
                y1,
                slope: (x1 - x0) / (y1 - y0),
                dir: 1.0,
            });
        } else {
            segs.push(Seg {
                x0: x1,
                y0: y1,
                y1: y0,
                slope: (x0 - x1) / (y0 - y1),
                dir: -1.0,
            });
        }
    }
    if segs.is_empty() {
        return None;
    }
    let mut ys: Vec<f64> = Vec::with_capacity(2 * segs.len());
    ys.extend(segs.iter().flat_map(|s| [s.y0, s.y1]));
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    segs.sort_by(|a, b| a.y0.total_cmp(&b.y0));

    let mut out: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(segs.len());
    let mut overlap = false;
    // `next` admits segments as the sweep reaches their top. `active`
    // holds the live segments in left-to-right order just below the
    // current band's top — the same order the old per-band midpoint sort
    // produced, maintained incrementally instead of re-sorted every
    // band: a new segment is inserted at its position at the band top,
    // retirement removes in place, and order changes happen only at the
    // crossings the sweep detects (an adjacent pair swaps). `pos` maps a
    // segment to its slot in `active` (`usize::MAX` once retired).
    // `retire` pops segments ending at or above the band top and
    // `xings` carries the crossing ordinate of every adjacent pair,
    // computed once when the pair forms — the earliest in-band crossing
    // is then the heap head rather than a full rescan, and pairs whose
    // crossing reaches the band top swap back into order.
    let mut next = 0usize;
    let mut active = Active::new(segs.len());
    let mut retire = BinaryHeap::<Reverse<(Split, u32)>>::new();
    let mut xings = BinaryHeap::<Reverse<(Split, u32, u32, u8)>>::new();
    // Swapped pairs awaiting possible reversion when a later band's
    // midpoint sits below their crossing; max-heap keyed by the
    // ordinate in swapped order.
    let mut posts = BinaryHeap::<(Split, u32, u32)>::new();
    // Adjacent pairs with exactly equal slopes: no crossing exists to
    // key an event on, but the old sort still ordered them by their
    // midpoint keys each band, so they are re-checked per band.
    let mut eqs: Vec<(u32, u32)> = Vec::new();
    // Per-segment open boundary run: (index into `out`, orientation)
    // while `alive` marks it live. A segment that is a boundary again
    // in the very next band with the same orientation extends its
    // emitted edge instead of starting a new piece; a run that stops
    // firing closes itself. The merged edge is exactly the union of
    // the per-band pieces of the same line.
    let mut runs: Vec<Option<(usize, bool)>> = vec![None; segs.len()];
    // The transitions detected at the previous emitting band, kept
    // for the cross-segment collinear merge: (rank at that band,
    // segment, index into `out`, orientation). New births insert by
    // their rank — the ordering the old `open` push order produced —
    // and deaths remove their entry; a split `continue` leaves the
    // list untouched since no band was emitted between. The matched
    // endpoint and slope are recomputed from the segment at match
    // time: the previous band's bottom is this band's `ya`.
    // Open transitions of the previous and this emit — (seg, piece,
    // orient, x at the band's bottom, slope). The last two fields are
    // what the continuation match tests; caching them keeps each
    // candidate check to a tuple load and two compares instead of two
    // `segs` accesses and an `x_at` call.
    let mut prev_open: Vec<(u32, usize, bool, f64, f64)> = Vec::new();
    let mut open: Vec<(u32, usize, bool, f64, f64)> = Vec::new();
    // `prev_open` sorted by endpoint abscissa: the continuation match
    // window-searches it instead of scanning the whole list — the
    // winning entry is still the first in emit order among the
    // verified candidates (its stored index is the emit-order rank).
    let mut pix: Vec<(f64, f64, bool, usize, u32)> = Vec::new();
    // Whether each segment's transition is live: a live transition
    // fires at every band until an event flips it, so `alive`
    // standing means the run continued through the last band.
    let mut alive: Vec<bool> = vec![false; segs.len()];
    // Segments swapped during `drain_xings`, transitions dying this
    // band — (rank, segment).
    let mut swapped: Vec<u32> = Vec::new();
    let mut dead: Vec<(u32, u32)> = Vec::new();
    // Band bottoms by band index, for the deferred piece-end writes,
    // and the last emitting band's bottom.
    let mut yb_hist: Vec<f64> = Vec::new();
    let mut last_yb = 0.0;
    // Band tops and bottoms: `cursor` walks the sorted endpoint list while
    // `pending` holds split points found inside bands. Every split a band
    // detects is strictly inside it, hence smaller than every boundary
    // still to come, so the next band bottom is always the smaller of the
    // two heads — the same sequence `ys.insert(band + 1, yc)` produced,
    // without an O(n) shift and re-run per insertion (issue #210). A band
    // bottom is consumed only once the band emits (or is skipped empty):
    // a split re-runs the band with the new, smaller boundary and the
    // deferred bottom becomes a later band's bottom, exactly as the
    // insertion left it. `band` counts processed bands for the `runs`
    // continuation check below; a band rerun after a split does not count.
    let mut pending = BinaryHeap::<Reverse<Split>>::new();
    let mut cursor = 1usize;
    let mut ya = ys[0];
    // A single operand-order bound for every pair event: the stored
    // heap ordinate sits within `xing_err` of the scan-order value for
    // any pair, so heads with ordinates beyond a target plus this bound
    // can never win and the pops below stop there.
    let m_glob = segs
        .iter()
        .fold(0.0f64, |m, s| m.max((s.slope * s.y0).abs()).max(s.x0.abs()));
    loop {
        let (yb, from_pending) = match pending.peek() {
            Some(&Reverse(Split(yc))) if ys.get(cursor).is_none_or(|&base| yc < base) => (yc, true),
            _ => match ys.get(cursor) {
                Some(&base) => (base, false),
                None => break,
            },
        };
        let ym = ya.midpoint(yb);
        // Bring the carried-over list into the order the old sort
        // produced at this band's midpoint first, so the binary-search
        // admission below inserts on a sorted list.
        drain_xings(
            &mut xings,
            &mut posts,
            &mut eqs,
            &segs,
            &mut active,
            ya,
            ym,
            m_glob,
            &mut swapped,
        );
        swapped.clear();
        while next < segs.len() && segs[next].y0 <= ya + EPS {
            let i = next;
            next += 1;
            retire.push(Reverse((Split(segs[i].y1), i as u32)));
            // Insert at the segment's position at the band's midpoint:
            // the position the old sort produced. A tie on x goes by
            // slope, and equal keys land after existing and
            // already-admitted members, matching the stable sort.
            let (xi, si) = (segs[i].x_at(ym), segs[i].slope);
            let at = active.slot(&segs, ym, xi, si);
            active.insert(&segs, at, i as u32);
            if at > 0 {
                push_xing(
                    &mut xings,
                    &mut posts,
                    &mut eqs,
                    &segs,
                    active.at(at - 1) as usize,
                    i,
                    m_glob,
                );
            }
            if at + 1 < active.len {
                push_xing(
                    &mut xings,
                    &mut posts,
                    &mut eqs,
                    &segs,
                    i,
                    active.at(at + 1) as usize,
                    m_glob,
                );
            }
        }
        while let Some(&Reverse((Split(y1), i))) = retire.peek() {
            if y1 > ya + EPS {
                break;
            }
            retire.pop();
            let i = i as usize;
            let at = active.rank(i as u32);
            // The retiring edge's own transition dies with it; its
            // last firing was the previous band.
            if alive[i] {
                alive[i] = false;
                dead.push((at as u32, i as u32));
            }
            active.remove(&segs, i as u32);
            if at > 0 && at < active.len {
                push_xing(
                    &mut xings,
                    &mut posts,
                    &mut eqs,
                    &segs,
                    active.at(at - 1) as usize,
                    active.at(at) as usize,
                    m_glob,
                );
            }
        }
        // Transitions that died on retirement close their pieces in
        // the rank order they held while firing — each write is the
        // piece's endpoint at the previous band's bottom.
        dead.sort_by_key(|&(r, _)| r);
        for &(_, s) in &dead {
            let i = s as usize;
            if let Some((edge, orient)) = runs[i].take() {
                let y = yb_hist.last().copied().unwrap_or(last_yb);
                let x = segs[i].x_at(y);
                let piece = &mut out[edge];
                if orient {
                    piece.2 = x;
                    piece.3 = y;
                } else {
                    piece.0 = x;
                    piece.1 = y;
                }
            }
        }
        dead.clear();
        if active.len == 0 {
            prev_open.clear();
            pix.clear();
            yb_hist.push(yb);
            if from_pending {
                pending.pop();
            } else {
                cursor += 1;
            }
            ya = yb;
            continue;
        }
        // After the first drain the carried list is in `ym` order, and
        // admissions and retirements keep it there — inserts land at
        // their key position and removals join neighbours that were
        // already ordered — so no second drain is needed.
        // Split at the smallest crossing strictly inside the band.
        // `active` is now exactly the order the old midpoint sort
        // produced, so the adjacent-pair set and each crossing's
        // operand order match the old scan — the heap head is the
        // minimum without re-scoring.
        let mut split = None;
        let mut keep: Vec<Reverse<(Split, u32, u32, u8)>> = Vec::new();
        while let Some(&Reverse((Split(appr), l, r, det))) = xings.peek() {
            if appr >= yb - EPS {
                break;
            }
            xings.pop();
            let (l, r) = (l as usize, r as usize);
            if active.chunk_of[l] == usize::MAX
                || active.chunk_of[r] == usize::MAX
                || active.rank(l as u32) + 1 != active.rank(r as u32)
            {
                continue;
            }
            keep.push(Reverse((Split(appr), l as u32, r as u32, det)));
            if det != 0 && appr > ya + EPS {
                split = Some(appr);
                break;
            }
        }
        for e in keep {
            xings.push(e);
        }
        if let Some(yc) = split {
            pending.push(Reverse(Split(yc)));
            overlap = true;
            continue;
        }
        // Consume the boundary the band ends at: it stays in `pending`
        // while the band is split so the sweep revisits it in order.
        if from_pending {
            pending.pop();
        } else {
            cursor += 1;
        }
        // Emit pass: every live element is evaluated once, in rank
        // order — the same order the old `active` vec walked it in —
        // carrying the winding prefix `w`. A live transition fires at
        // every band, so a segment's piece endpoint is written once:
        // at the band after its last firing — the value the old
        // per-band writes converged to.
        let pend = yb_hist.last().copied().unwrap_or(last_yb);
        let mut w = 0.0;
        for ch in &active.chunks {
            for &e in &ch.els {
                let i = e as usize;
                let seg = &segs[i];
                let dir = seg.dir;
                let (l, now) = match rule {
                    FillRule::NonZero => {
                        // w is a sum of unit directions, so the
                        // boundary test is a direct comparison.
                        if dir > 0.0 {
                            (w != 0.0, w != -1.0)
                        } else {
                            (w != 0.0, w != 1.0)
                        }
                    }
                    FillRule::EvenOdd => (inside_at(w, rule), inside_at(w + dir, rule)),
                };
                w += dir;
                if l == now {
                    // No transition here any more: if the run was
                    // live, it died — its last firing was the
                    // previous band.
                    if alive[i] {
                        alive[i] = false;
                        dead.push((0, e));
                    }
                    continue;
                }
                let (xa, xb) = (seg.x_at(ya), seg.x_at(yb));
                if alive[i]
                    && let Some((edge, orient)) = runs[i]
                {
                    if orient == now {
                        // Same run keeps firing — its piece was
                        // already emitted; nothing changes.
                        open.push((e, edge, orient, xb, seg.slope));
                        continue;
                    }
                    // The orientation flipped: the old run's last
                    // firing was the previous band — close its piece.
                    let x = seg.x_at(pend);
                    let piece = &mut out[edge];
                    if orient {
                        piece.2 = x;
                        piece.3 = pend;
                    } else {
                        piece.0 = x;
                        piece.1 = pend;
                    }
                }
                // A new or re-born transition: continue an open
                // run from the previous band that ends where this
                // piece starts and shares its slope — coincident
                // collinear segments form one boundary line, so
                // the extension is exact; at a kink the slope
                // differs and a new edge starts. `pix` is `prev_open`
                // sorted by endpoint abscissa, so the match window
                // is a binary-search range instead of a full scan;
                // among the candidates the lowest stored index is
                // the first entry in emit order — the same pick the
                // linear scan made.
                let tol = EPS * (1.0 + xa.abs());
                let (xlo, xhi) = (xa - tol, xa + tol);
                let lo = pix.partition_point(|&(xe, _, _, _, _)| xe < xlo);
                let hi = pix.partition_point(|&(xe, _, _, _, _)| xe <= xhi);
                let mut best: Option<(u32, usize)> = None;
                for &(_, sl, orient, edge, oidx) in &pix[lo..hi] {
                    if orient == now && (sl - seg.slope).abs() <= EPS {
                        match best {
                            Some((b, _)) if b <= oidx => {}
                            _ => best = Some((oidx, edge)),
                        }
                    }
                }
                let edge = best.map_or_else(
                    || {
                        if now {
                            out.push((xa, ya, xb, yb));
                        } else {
                            out.push((xb, yb, xa, ya));
                        }
                        out.len() - 1
                    },
                    |(_, edge)| edge,
                );
                runs[i] = Some((edge, now));
                alive[i] = true;
                open.push((e, edge, now, xb, seg.slope));
            }
        }
        std::mem::swap(&mut open, &mut prev_open);
        open.clear();
        pix.clear();
        pix.extend(
            prev_open
                .iter()
                .enumerate()
                .map(|(idx, &(_, edge, orient, xe, sl))| (xe, sl, orient, edge, idx as u32)),
        );
        pix.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Dying transitions close their pieces in rank order — the
        // chunk walk already yielded them that way.
        for &(_, s) in &dead {
            let i = s as usize;
            if let Some((edge, orient)) = runs[i].take() {
                let x = segs[i].x_at(pend);
                let piece = &mut out[edge];
                if orient {
                    piece.2 = x;
                    piece.3 = pend;
                } else {
                    piece.0 = x;
                    piece.1 = pend;
                }
            }
        }
        dead.clear();
        // A winding magnitude above one, or both signs in one band,
        // means regions overlap — only then is rewriting needed.
        let (wmin, wmax) = active.w_minmax();
        if wmax >= 2.0 || wmin <= -2.0 || (wmin < 0.0 && wmax > 0.0) {
            overlap = true;
        }
        last_yb = yb;
        yb_hist.push(yb);
        ya = yb;
    }
    // Transitions still live finalize at the last emitting band's
    // bottom — the old walk's last write — in rank order so the
    // last-emitted transition wins shared pieces.
    for r in 0..active.len {
        let i = active.at(r) as usize;
        if alive[i]
            && let Some((edge, orient)) = runs[i]
        {
            let x = segs[i].x_at(last_yb);
            let piece = &mut out[edge];
            if orient {
                piece.2 = x;
                piece.3 = last_yb;
            } else {
                piece.0 = x;
                piece.1 = last_yb;
            }
        }
    }
    if !overlap {
        return None;
    }
    Some(
        out.iter()
            .map(|&(x0, y0, x1, y1)| (x0 as f32, y0 as f32, x1 as f32, y1 as f32))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed area of the region whose boundary is `edges`: on the
    /// winding-0/1 emitted set, Σ x̄·(y1−y0) is the shoelace vertical
    /// contribution and horizontal connectors carry no area.
    fn area(edges: &[(f32, f32, f32, f32)]) -> f64 {
        edges
            .iter()
            .map(|&(x0, y0, x1, y1)| f64::midpoint(x0.into(), x1.into()) * f64::from(y1 - y0))
            .sum()
    }

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32, f32, f32)> {
        vec![
            (x0, y0, x1, y0),
            (x1, y0, x1, y1),
            (x1, y1, x0, y1),
            (x0, y1, x0, y0),
        ]
    }

    #[test]
    fn a_single_square_needs_no_resolution() {
        let segs = square(0.0, 0.0, 9.0, 9.0);
        assert!(resolve_winding(&segs, FillRule::NonZero).is_none());
        assert!(resolve_winding(&segs, FillRule::EvenOdd).is_none());
    }

    #[test]
    fn overlapping_squares_emit_their_union_boundary() {
        // [0,3]² and [1.5,4.5]², same winding: union 9 + 9 − 1.5² = 15.75.
        let mut segs = square(0.0, 0.0, 3.0, 3.0);
        segs.extend(square(1.5, 1.5, 4.5, 4.5));
        let resolved = resolve_winding(&segs, FillRule::NonZero).expect("overlap present");
        // One merged edge per boundary line: x=0, x=3, x=1.5, x=4.5.
        assert_eq!(resolved.len(), 4, "merged edges: {resolved:?}");
        assert!(
            (area(&resolved).abs() - 15.75).abs() < 1e-3,
            "union area {}",
            area(&resolved)
        );
        // The overlap's interior boundary edges cancel: between x=1.5
        // and x=3 the two squares' shared edges are inside the union and
        // must not appear.
        assert!(
            resolved
                .iter()
                .all(|e| e.0 >= 0.0 && e.1 >= 0.0 && e.2 >= 0.0 && e.3 >= 0.0),
            "edges outside input bounds: {resolved:?}"
        );
    }

    #[test]
    fn stacked_rectangles_emit_one_edge_per_side() {
        // 40 rectangles [0,10]×[0.5i, 0.5i+5], all same winding: many
        // bands and heavy overlap, but the union is one rectangle
        // [0,10]×[0,24.5] — two merged boundary edges, area 245.
        let mut segs = Vec::new();
        for i in 0..40u8 {
            let y = 0.5 * f32::from(i);
            segs.extend(square(0.0, y, 10.0, y + 5.0));
        }
        let resolved = resolve_winding(&segs, FillRule::NonZero).expect("overlap present");
        assert_eq!(resolved.len(), 2, "merged edges: {resolved:?}");
        assert!(
            (area(&resolved).abs() - 245.0).abs() < 1e-3,
            "union area {}",
            area(&resolved)
        );
    }

    #[test]
    fn a_bow_tie_keeps_both_lobes() {
        // Self-crossing quad: mixed-sign windings cancel in the raw
        // accumulator; resolved coverage is the two-triangle union,
        // area 8.0.
        let segs = vec![
            (0.0, 0.0, 4.0, 4.0),
            (4.0, 4.0, 4.0, 0.0),
            (4.0, 0.0, 0.0, 4.0),
            (0.0, 4.0, 0.0, 0.0),
        ];
        let resolved = resolve_winding(&segs, FillRule::NonZero).expect("mixed signs overlap");
        assert!(
            (area(&resolved).abs() - 8.0).abs() < 1e-3,
            "lobes area {}",
            area(&resolved).abs()
        );
    }

    #[test]
    fn even_odd_nested_squares_open_the_hole() {
        let mut segs = square(0.0, 0.0, 4.0, 4.0);
        segs.extend(square(1.0, 1.0, 3.0, 3.0));
        let resolved = resolve_winding(&segs, FillRule::EvenOdd).expect("winding reaches 2");
        // Outer minus inner area: the hole's boundary edges run
        // reversed (the inner square's winding is cancelled).
        assert!(
            (area(&resolved).abs() - 12.0).abs() < 1e-3,
            "ring area {}",
            area(&resolved).abs()
        );
        assert!(
            resolved
                .iter()
                .any(|&(x0, y0, x1, y1)| { x0 >= 1.0 && x1 >= 1.0 && y1 < y0 && (y0 - y1) > 1.0 }),
            "no upward inner edge emitted for the hole: {resolved:?}"
        );
    }

    /// Whether the open interiors of two segments intersect.
    fn edges_cross(e: (f32, f32, f32, f32), f: (f32, f32, f32, f32)) -> bool {
        let cross = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
            (q.1 - p.1).mul_add(-(r.0 - p.0), (q.0 - p.0) * (r.1 - p.1))
        };
        let (e0, e1, f0, f1) = (
            (f64::from(e.0), f64::from(e.1)),
            (f64::from(e.2), f64::from(e.3)),
            (f64::from(f.0), f64::from(f.1)),
            (f64::from(f.2), f64::from(f.3)),
        );
        let (d1, d2, d3, d4) = (
            cross(f0, f1, e0),
            cross(f0, f1, e1),
            cross(e0, e1, f0),
            cross(e0, e1, f1),
        );
        ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) && d1 != 0.0 && d2 != 0.0
    }

    #[test]
    fn a_later_crossing_still_splits_the_band() {
        // The first inverted pair (0,0)-(4,4) vs (1e-10,0)-(-4,4) crosses
        // at y≈5e-11 — on the band's top boundary, a tie, no valid split.
        // A scan that stops at that pair would walk (5,0)-(4,4) and
        // (6,0)-(3,4) while they cross inside at y=2.
        let segs = vec![
            (0.0f32, 0.0, 4.0, 4.0),
            (1e-10, 0.0, -4.0, 4.0),
            (5.0, 0.0, 4.0, 4.0),
            (6.0, 0.0, 3.0, 4.0),
        ];
        let resolved = resolve_winding(&segs, FillRule::NonZero).expect("crossing present");
        // The crossing splits the band, so no two emitted edges cross
        // strictly inside (the pre-fix walk emitted crossing pieces).
        for (i, first) in resolved.iter().enumerate() {
            for second in &resolved[i + 1..] {
                assert!(
                    !edges_cross(*first, *second),
                    "emitted edges cross: {first:?} x {second:?}"
                );
            }
        }
    }

    #[test]
    fn a_nonadjacent_crossing_still_splits_the_band() {
        // Real geometry from a stroked glyph: at the band top the sweep
        // order is A(+1) B(−1) C(−1) with B just left of C, so the only
        // *adjacent* inversion is B×C — which crossed exactly at the
        // band's top boundary and yields no valid split. C also crosses
        // A inside the band at y≈42.72, non-adjacently; an adjacent-only
        // scan misses it and emits edges that cross inside the band.
        // Verbatim segments from a stroked glyph (the +1 edge, the two
        // diagonals, and the long contour edge sharing its top vertex).
        let segs = vec![
            (110.472_26f32, 43.915_257, 108.866_936, 27.915_59),
            (110.856_94, 27.715_923, 112.462_265, 43.715_59),
            (113.483_76, 27.452_364, 115.089_07, 43.452_03),
            (113.099_07, 43.651_7, 111.493_744, 27.652_03),
            (111.367_424, 42.820_42, 113.994_24, 42.55686),
            (113.994_24, 42.55686, 113.099_07, 43.651_7),
        ];
        let resolved = resolve_winding(&segs, FillRule::NonZero).expect("crossing present");
        assert!(
            resolved
                .iter()
                .any(|e| (e.1 - 42.7206).abs() < 1e-3 || (e.3 - 42.7206).abs() < 1e-3),
            "no edge boundary at the y≈42.72 crossing: {resolved:?}"
        );
    }
}

#[cfg(test)]
mod content_tests {
    use super::{Content, Operation};
    use crate::Draw;

    struct TestOp;

    impl Operation for TestOp {
        fn end_mut(&mut self) -> Option<&mut u32> {
            None
        }

        fn same_structure(&self, _other: &Self) -> bool {
            true
        }
    }

    #[test]
    fn replacement_returns_the_previous_picture() {
        let previous = crate::Picture::from_list(crate::DisplayList::default());
        let replacement = crate::Picture::record(|c| {
            c.fill(
                crate::kurbo::Rect::new(0., 0., 1., 1.),
                crate::WorkingColor::WHITE,
            );
        });
        let mut content = Content::<TestOp, ()>::new(previous.clone());

        assert_eq!(content.replace(replacement.clone()), previous);
        assert_eq!(content.into_picture(), replacement);
    }
}
