//! The exact pre-sweep test of [`resolve_winding`](super::resolve_winding),
//! and the ordered event stream it shares with the sweep.
//!
//! The test proves, without the band sweep, that a path's flattened
//! segments cannot form an overlapping region, so the sweep would return
//! its input unchanged. When it cannot, everything it built is handed to
//! the sweep: the normalized segments, and the y-ordered chain events,
//! which the sweep's segment order and band ordinates are merged from in
//! place of its own sorts.
//!
//! # What the sweep decides
//!
//! The sweep reports overlap in exactly two ways. A pair of segments that
//! is adjacent in a band's left-to-right order, with slopes more than
//! `EPS` apart, has its computed crossing ordinate strictly inside
//! `(ya + EPS, yb - EPS)`, splitting the band; or a band's winding prefix
//! (the running sum of segment directions, left to right) reaches `±2` or
//! takes both signs. Its band order is the order of each segment's
//! `x_at` at the band's midpoint, ties broken by slope.
//!
//! # The event stream
//!
//! The segments are grouped into maximal y-monotone chains in input order:
//! consecutive segments of one direction whose shared vertex is bitwise the
//! same point. A chain's segments are consecutive input indices, and their
//! tops strictly monotone. Each chain contributes a start event (its top
//! ordinate) and an end event (its bottom ordinate), keyed by the
//! ordinate's `total_cmp` order and then the chain id; the two lists are
//! sorted once, on plain integer keys.
//!
//! # The test
//!
//! The chains are swept by their events only — at each ordinate the
//! chains ending there leave the live x order in one compaction, then the
//! chains starting there are inserted at their positions. After the
//! events of each ordinate, every pair of chains that became adjacent is
//! walked over its whole remaining common y-span, and the directions are
//! checked where the order changed. The test returns `true` only when:
//!
//! 1. every coordinate is finite with magnitude at most `2^19`, and every
//!    ordinate is `0` or at least `2^-6` in magnitude;
//! 2. at every vertex ordinate of a walked pair, the right chain's `x`
//!    exceeds the left chain's by at least `GAP_ULPS·ε·m`, `m` the two
//!    segments' coordinate magnitudes; the only exception is a pair of
//!    chains that start (or end) at one shared vertex, whose segments
//!    there diverge with slopes at least `APEX_ULPS·ε·m / EPS` apart;
//! 3. every winding prefix is `0` or one global sign `s ∈ {1, -1}`: the
//!    first live chain runs in direction `s` and adjacent chains run in
//!    opposite directions, which is the same condition, since a prefix of
//!    `±1` steps stays in `{0, s}` exactly when it starts with `s` and
//!    alternates. Only the chains whose neighbours changed at an ordinate
//!    are checked there; every other adjacency was checked when it formed.
//!
//! The order in which the chains of one ordinate are inserted does not
//! matter: each lands after every live chain left of its top vertex and
//! after every chain at that vertex with a smaller or equal slope, so the
//! live order is the same up to chains that coincide in vertex and slope,
//! and an adjacent coinciding pair fails (2) in any order.
//!
//! # Why `true` means the sweep returns its input unchanged
//!
//! By (1), distinct `f32` ordinates are more than `2^-29 > EPS` apart, so
//! the sweep's `EPS` admission and retirement select exactly the segments
//! whose open y-span covers the band, and its bands are the intervals
//! between consecutive endpoint ordinates — chain events and chain-internal
//! vertices alike, which never change the chain order. Two segments live in
//! one band lie in chains that are adjacent there or separated by others;
//! by (2) and linearity, every such pair is `GAP_ULPS·ε·m` apart over the
//! band, except at a shared apex. The rounding of `x_at`, of the slope and
//! of the crossing numerator is a few `ε·m`, so the midpoint keys order the
//! band exactly as the geometry does; at a shared apex both keys are
//! `fma(slope, t, x0)` with one `x0` and `t`, monotone in the slope, with
//! the slope as tie-break, so the order is exact there too. A walked pair
//! keeps its gap over the band, so the true crossing of its lines lies
//! outside the band by at least `gap/|Δslope|`, while the computed
//! ordinate is off by at most `4ε·m/|Δslope| + 3ε·|y|`; with the gap bound
//! (and `|y| ≤ 2^19`) the computed ordinate never enters `(ya, yb)`. A
//! shared apex has its true crossing exactly at the apex ordinate, the
//! band's top (or bottom), and the slope bound keeps the computed one
//! within `EPS` of it, outside `(ya + EPS, yb - EPS)`. No band splits.
//! The sweep's band order equals the chain order, so its winding prefixes
//! are the ones (3) checked, and none reaches `±2` or both signs. The
//! sweep, given the same segments, returns `None`.
//!
//! When any check fails the test answers `false` — "in doubt", not
//! "overlapping" — and the sweep decides. A work budget proportional to
//! the input bounds the walks, the checks and the reordering of the live
//! chains, so the test stays `O(n + k log k)` for `k` chains; exhausting
//! it is a doubt too.
//!
//! # Why the handed-over order is the sweep's own
//!
//! The sweep visits the segments in `y0` order, stable in input order, and
//! bands at the sorted, deduplicated endpoint ordinates. Both follow from
//! the sorted chain events without another sort. A chain's segments enter
//! in `y0` order from its top on, and chains enter in start-event order,
//! which is `(y0, top index)` order because a lower chain id holds only
//! lower indices; merging the entered chains on `(y0 key, index)` yields
//! every segment exactly once in `(y0, index)` order — the stable sort's
//! result. The endpoint ordinates are the segments' tops and the chains'
//! bottoms, since an interior vertex is bitwise the top of the segment
//! below it; merging the ordered tops with the ordered end events in
//! `total_cmp` order and dropping each value equal to the previous one is
//! the sort-and-deduplicate's result. The merge costs `O(n log w)` for at
//! most `w` chains entered at once, against the sorts' `O(n log n)`.

use super::{EPS, Seg};

/// The gap two live chains must keep, in units of `ε` times the two
/// segments' coordinate magnitudes: far above the few-ulp rounding of the
/// sweep's keys and crossing ordinates, and above its parallel-pair bound
/// (`256·ε·m`).
const GAP_ULPS: f64 = 1024.0;
/// The slope difference a shared apex needs, in units of `ε·m / EPS`: the
/// crossing numerator's rounding (at most `4ε·m`) divided by it stays
/// below `EPS / 4`.
const APEX_ULPS: f64 = 16.0;
/// Coordinates beyond this magnitude are left to the sweep: it keeps the
/// crossing ordinate's relative rounding (`3ε·|y|`) below `EPS / 4`.
const LIMIT: f64 = 524_288.0;
/// The smallest non-zero ordinate magnitude the test accepts. `f32` values
/// at or above it are at least `2^-29` apart, more than `EPS`.
const Y_FLOOR: f64 = 0.015_625;
/// Walk, check and reordering steps the test may spend per segment before
/// it leaves the input to the sweep.
const BUDGET_PER_SEGMENT: usize = 16;

/// The scratch words one chain occupies: top segment, bottom segment,
/// direction, bottom `x` (`f32` bits), cursor, verified right neighbour,
/// position in the live order.
const CHAIN_WORDS: usize = 7;
/// The scratch words per chain of its two events, each `[key, chain]`.
const EVENT_WORDS: usize = 4;
/// The scratch words per chain the test's sweep adds: a live-order slot
/// and three slots for the chains whose neighbours changed at one ordinate
/// (an insertion changes one, a removal two).
const LIVE_WORDS: usize = 4;
/// The scratch words per chain of the merge's entries
/// `[key, segment, chain]`.
const MERGE_WORDS: usize = 3;
/// No chain, or no position: an empty slot.
const NONE: u32 = u32::MAX;

/// The key of `y` in `f32::total_cmp` order.
const fn key(y: f32) -> u32 {
    let bits = y.to_bits();
    if bits >> 31 == 1 {
        !bits
    } else {
        bits | 0x8000_0000
    }
}

/// The `f32` whose [`key`] is `key`.
const fn ordinate(key: u32) -> f32 {
    f32::from_bits(if key >> 31 == 1 {
        key & 0x7FFF_FFFF
    } else {
        !key
    })
}

/// The [`key`] of a normalized ordinate, which holds an `f32` value.
#[expect(
    clippy::cast_possible_truncation,
    reason = "normalized ordinates are f32 input values widened exactly"
)]
const fn key_of(y: f64) -> u32 {
    key(y as f32)
}

/// Segments in input order, normalized like the sweep's, with their
/// chains and sorted chain events: `scratch` holds `k` chain records, then
/// the start and end events, and has room for the merge.
pub(super) struct Prepared {
    pub(super) segs: Vec<Seg>,
    scratch: Vec<u32>,
    chains: usize,
}

/// Normalizes `segments` exactly as the sweep does (non-finite and
/// horizontal segments dropped, each oriented top to bottom), builds and
/// orders their chain events, and reports whether they provably form no
/// overlapping region. The prepared buffers go to the sweep when they do
/// not.
#[expect(
    clippy::float_cmp,
    reason = "the horizontal test compares exact f32 input values"
)]
pub(super) fn prepare<S: Copy + Into<(f32, f32, f32, f32)>>(segments: &[S]) -> (bool, Prepared) {
    // Pass 1: count the sweep's segments and the chains, and check the
    // coordinate bounds, without allocating.
    let mut count = 0usize;
    let mut chains = 0usize;
    let mut applicable = true;
    let mut last: Option<(u32, u32, bool)> = None;
    for &s in segments {
        let (x0, y0, x1, y1) = s.into();
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            applicable = false;
            last = None;
            continue;
        }
        if y0 == y1 {
            last = None;
            continue;
        }
        applicable &= [x0, y0, x1, y1]
            .iter()
            .all(|v| f64::from(*v).abs() <= LIMIT)
            && [y0, y1]
                .iter()
                .all(|y| *y == 0.0 || f64::from(*y).abs() >= Y_FLOOR);
        let down = y0 < y1;
        if last != Some((x0.to_bits(), y0.to_bits(), down)) {
            chains += 1;
        }
        last = Some((x1.to_bits(), y1.to_bits(), down));
        count += 1;
    }
    let mut segs = Vec::with_capacity(count);
    // Room for the records and events, then the larger of the test's live
    // order (`LIVE_WORDS` per chain) and the merge (`MERGE_WORDS` per chain
    // and one word per segment), which also holds the ordinates.
    let mut scratch =
        Vec::with_capacity(chains * (CHAIN_WORDS + EVENT_WORDS + MERGE_WORDS) + count);
    let mut last: Option<(u32, u32, bool)> = None;
    for &s in segments {
        let (x0, y0, x1, y1) = s.into();
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            last = None;
            continue;
        }
        if y0 == y1 {
            last = None;
            continue;
        }
        let down = y0 < y1;
        let index = u32::try_from(segs.len()).expect("segment count fits u32");
        if last == Some((x0.to_bits(), y0.to_bits(), down)) {
            // Extend the open chain: its bottom moves down (a downward
            // chain) or its top moves up (an upward one), and the cursor
            // with it, since walks start from the top.
            let open = scratch.len() - CHAIN_WORDS;
            if down {
                scratch[open + 1] = index;
                scratch[open + 3] = x1.to_bits();
            } else {
                scratch[open] = index;
                scratch[open + 4] = index;
            }
        } else {
            let (bottom_x, step) = if down { (x1, 1) } else { (x0, -1i32) };
            // Word by word: a slice copy here is a memory-primitive call
            // per chain.
            for word in [
                index,
                index,
                step.cast_unsigned(),
                bottom_x.to_bits(),
                index,
                NONE,
                NONE,
            ] {
                scratch.push(word);
            }
        }
        last = Some((x1.to_bits(), y1.to_bits(), down));
        segs.push(normalized(x0, y0, x1, y1));
    }
    order_events(&segs, &mut scratch, chains);
    let disjoint = applicable && {
        // The live order and changed slots are written before they are
        // read; extending by the chain ids sizes them without a fill.
        let slots = u32::try_from(LIVE_WORDS * chains).expect("slot count fits u32");
        scratch.extend(0..slots);
        let disjoint = Sweep::new(&segs, &mut scratch, chains).run();
        scratch.truncate(chains * (CHAIN_WORDS + EVENT_WORDS));
        disjoint
    };
    (
        disjoint,
        Prepared {
            segs,
            scratch,
            chains,
        },
    )
}

/// The segment `(x0, y0)`–`(x1, y1)`, not horizontal, oriented top to
/// bottom in `f64`, as the sweep keeps it.
fn normalized(x0: f32, y0: f32, x1: f32, y1: f32) -> Seg {
    let (x0, y0, x1, y1) = (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
    if y0 < y1 {
        Seg {
            x0,
            y0,
            y1,
            slope: (x1 - x0) / (y1 - y0),
            dir: 1.0,
        }
    } else {
        Seg {
            x0: x1,
            y0: y1,
            y1: y0,
            slope: (x0 - x1) / (y0 - y1),
            dir: -1.0,
        }
    }
}

/// Appends each chain's start and end event, `[key, chain]`, after the
/// chain records, and sorts the starts and the ends: by ordinate in
/// `total_cmp` order, then by chain id.
fn order_events(segs: &[Seg], scratch: &mut Vec<u32>, chains: usize) {
    let ids = 0..u32::try_from(chains).expect("chain count fits u32");
    for chain in ids.clone() {
        let top = scratch[chain as usize * CHAIN_WORDS] as usize;
        scratch.push(key_of(segs[top].y0));
        scratch.push(chain);
    }
    for chain in ids {
        let bottom = scratch[chain as usize * CHAIN_WORDS + 1] as usize;
        scratch.push(key_of(segs[bottom].y1));
        scratch.push(chain);
    }
    let events = &mut scratch[chains * CHAIN_WORDS..];
    let (starts, ends) = events.split_at_mut(2 * chains);
    starts.as_chunks_mut::<2>().0.sort_unstable();
    ends.as_chunks_mut::<2>().0.sort_unstable();
}

impl Prepared {
    /// The sweep's input: the segments in `(y0, input index)` order, and
    /// the scratch with the sorted, deduplicated endpoint ordinates (`f32`
    /// bits) from the returned offset on — merged from the chain events,
    /// as the module documentation argues.
    pub(super) fn into_ordered(self) -> (Vec<Seg>, Vec<u32>, usize) {
        let Self {
            mut segs,
            mut scratch,
            chains,
        } = self;
        let events_end = chains * (CHAIN_WORDS + EVENT_WORDS);
        let slots =
            u32::try_from(MERGE_WORDS * chains + segs.len()).expect("slot count fits u32");
        scratch.extend(0..slots);
        let (records, rest) = scratch.split_at_mut(chains * CHAIN_WORDS);
        let (events, rest) = rest.split_at_mut(EVENT_WORDS * chains);
        let (merge, order) = rest.split_at_mut(MERGE_WORDS * chains);
        merge_chains(
            records,
            events[..2 * chains].as_chunks::<2>().0,
            &segs,
            merge.as_chunks_mut::<MERGE_WORDS>().0,
            order,
        );
        permute(&mut segs, order);
        scratch.truncate(events_end);
        // The ordinates: segment tops (now in order) merged with chain
        // bottoms (the sorted end events), equal values dropped.
        let ends = chains * CHAIN_WORDS + 2 * chains;
        let (mut top, mut end) = (0, 0);
        let mut previous: Option<f32> = None;
        loop {
            let next_top = segs.get(top).map(|s| key_of(s.y0));
            let next_end = (end < chains).then(|| scratch[ends + 2 * end]);
            let key = match (next_top, next_end) {
                (Some(t), Some(e)) if t <= e => {
                    top += 1;
                    t
                }
                (_, Some(e)) => {
                    end += 1;
                    e
                }
                (Some(t), None) => {
                    top += 1;
                    t
                }
                (None, None) => break,
            };
            let y = ordinate(key);
            if previous != Some(y) {
                scratch.push(y.to_bits());
                previous = Some(y);
            }
        }
        (segs, scratch, events_end)
    }
}

/// Merges the chains' segments into `(y0 key, index)` order, writing each
/// segment's input index to `order`. Chains enter in start-event order;
/// `heap` is a binary min-heap of the entered chains' next segments.
fn merge_chains(
    records: &[u32],
    starts: &[[u32; 2]],
    segs: &[Seg],
    heap: &mut [[u32; MERGE_WORDS]],
    order: &mut [u32],
) {
    let record = |chain: u32| &records[chain as usize * CHAIN_WORDS..][..CHAIN_WORDS];
    // The segment below `segment` in `chain`, if any.
    let below = |chain: u32, segment: u32| {
        let r = record(chain);
        (segment != r[1]).then(|| segment.wrapping_add_signed(r[2].cast_signed()))
    };
    let mut len = 0usize;
    let mut next_start = 0usize;
    for slot in order.iter_mut() {
        let start = starts.get(next_start).map(|&[key, chain]| [key, record(chain)[0], chain]);
        let entered = match start {
            Some(entry) if len == 0 || entry < heap[0] => {
                next_start += 1;
                Some(entry)
            }
            _ => None,
        };
        let [_, segment, chain] = entered.unwrap_or(heap[0]);
        *slot = segment;
        let next = below(chain, segment).map(|s| [key_of(segs[s as usize].y0), s, chain]);
        match (entered, next) {
            (Some(_), Some(entry)) => {
                heap[len] = entry;
                len += 1;
                sift_up(&mut heap[..len]);
            }
            (Some(_), None) => {}
            (None, Some(entry)) => {
                heap[0] = entry;
                sift_down(&mut heap[..len]);
            }
            (None, None) => {
                len -= 1;
                heap[0] = heap[len];
                sift_down(&mut heap[..len]);
            }
        }
    }
}

/// Restores the min-heap order after its last entry was added.
fn sift_up(heap: &mut [[u32; MERGE_WORDS]]) {
    let mut at = heap.len() - 1;
    while at > 0 {
        let parent = (at - 1) / 2;
        if heap[at] >= heap[parent] {
            break;
        }
        heap.swap(at, parent);
        at = parent;
    }
}

/// Restores the min-heap order after its first entry changed.
fn sift_down(heap: &mut [[u32; MERGE_WORDS]]) {
    let mut at = 0;
    loop {
        let left = 2 * at + 1;
        if left >= heap.len() {
            break;
        }
        let child = if left + 1 < heap.len() && heap[left + 1] < heap[left] {
            left + 1
        } else {
            left
        };
        if heap[child] >= heap[at] {
            break;
        }
        heap.swap(at, child);
        at = child;
    }
}

/// Moves `segs[order[r]]` to position `r` for every `r`, in place, by
/// following each cycle of the permutation; `order` is consumed.
fn permute(segs: &mut [Seg], order: &mut [u32]) {
    for start in 0..order.len() {
        let mut at = start;
        let held = segs[start];
        loop {
            let from = order[at] as usize;
            order[at] = u32::try_from(at).expect("segment count fits u32");
            if from == start {
                segs[at] = held;
                break;
            }
            segs[at] = segs[from];
            at = from;
        }
    }
}

/// The chain-level sweep over the prepared segments. Chain records live in
/// `scratch[..k * CHAIN_WORDS]`, followed by the start events, the end
/// events, the live order and the chains whose neighbours changed at the
/// current ordinate.
struct Sweep<'a> {
    segs: &'a [Seg],
    chains: &'a mut [u32],
    starts: &'a [[u32; 2]],
    ends: &'a [[u32; 2]],
    live: &'a mut [u32],
    changed: &'a mut [u32],
    len: usize,
    changes: usize,
    sign: i32,
    budget: usize,
}

impl<'a> Sweep<'a> {
    const fn new(segs: &'a [Seg], scratch: &'a mut [u32], chains: usize) -> Self {
        let (chain_words, rest) = scratch.split_at_mut(chains * CHAIN_WORDS);
        let (events, rest) = rest.split_at_mut(EVENT_WORDS * chains);
        let (starts, ends) = events.split_at(2 * chains);
        let (live, changed) = rest.split_at_mut(chains);
        Self {
            segs,
            chains: chain_words,
            starts: starts.as_chunks::<2>().0,
            ends: ends.as_chunks::<2>().0,
            live,
            changed,
            len: 0,
            changes: 0,
            sign: 0,
            budget: segs.len() * BUDGET_PER_SEGMENT + 1024,
        }
    }

    const fn word(&self, chain: u32, field: usize) -> u32 {
        self.chains[chain as usize * CHAIN_WORDS + field]
    }

    const fn set(&mut self, chain: u32, field: usize, value: u32) {
        self.chains[chain as usize * CHAIN_WORDS + field] = value;
    }

    const fn top(&self, chain: u32) -> &Seg {
        &self.segs[self.word(chain, 0) as usize]
    }

    const fn bottom(&self, chain: u32) -> &Seg {
        &self.segs[self.word(chain, 1) as usize]
    }

    const fn step(&self, chain: u32) -> i32 {
        self.word(chain, 2).cast_signed()
    }

    fn bottom_x(&self, chain: u32) -> f64 {
        f64::from(f32::from_bits(self.word(chain, 3)))
    }

    /// The segment below `segment` in its chain.
    const fn below(&self, chain: u32, segment: u32) -> u32 {
        segment.wrapping_add_signed(self.step(chain))
    }

    /// The exact `x` of the chain's segment `segment` at `y`, which lies
    /// within the segment's span: vertices are read, interiors evaluated
    /// as the sweep's `x_at`.
    #[expect(clippy::float_cmp, reason = "vertices are exact input values")]
    fn x(&self, chain: u32, segment: u32, y: f64) -> f64 {
        let seg = &self.segs[segment as usize];
        if y == seg.y0 {
            seg.x0
        } else if y == seg.y1 {
            if segment == self.word(chain, 1) {
                self.bottom_x(chain)
            } else {
                self.segs[self.below(chain, segment) as usize].x0
            }
        } else {
            seg.x_at(y)
        }
    }

    /// Moves the chain's cursor to the segment covering `y` from below,
    /// or its bottom segment at its bottom, and returns it.
    fn seek(&mut self, chain: u32, y: f64) -> u32 {
        let mut segment = self.word(chain, 4);
        while segment != self.word(chain, 1) && self.segs[segment as usize].y1 <= y {
            segment = self.below(chain, segment);
        }
        self.set(chain, 4, segment);
        segment
    }

    /// Runs the events ordinate by ordinate: ends, then starts, then the
    /// checks. `0` and `-0` are one ordinate, as in the sweep's `f64`
    /// comparisons; their keys are adjacent, so they group.
    #[expect(clippy::float_cmp, reason = "events group at exactly equal ordinates")]
    fn run(mut self) -> bool {
        let (starts, ends) = (self.starts, self.ends);
        let (mut next_start, mut next_end) = (0, 0);
        while next_end < ends.len() {
            let y_end = self.bottom(ends[next_end][1]).y1;
            let y = match starts.get(next_start) {
                Some(&[_, chain]) => self.top(chain).y0.min(y_end),
                None => y_end,
            };
            let first = next_end;
            while next_end < ends.len() && self.bottom(ends[next_end][1]).y1 == y {
                next_end += 1;
            }
            if !self.remove(&ends[first..next_end]) {
                return false;
            }
            while next_start < starts.len() && self.top(starts[next_start][1]).y0 == y {
                if !self.insert(starts[next_start][1], y) {
                    return false;
                }
                next_start += 1;
            }
            if !self.settle(y) {
                return false;
            }
        }
        true
    }

    /// Charges `count` steps to the budget; `false` when it runs out.
    const fn spend(&mut self, count: usize) -> bool {
        if self.budget < count {
            return false;
        }
        self.budget -= count;
        true
    }

    /// Places `chain` at live position `at` and records it there.
    fn place(&mut self, at: usize, chain: u32) {
        self.live[at] = chain;
        self.set(chain, 6, u32::try_from(at).expect("live position fits u32"));
    }

    /// Records that `chain`'s neighbours changed at the current ordinate.
    const fn touch(&mut self, chain: u32) {
        self.changed[self.changes] = chain;
        self.changes += 1;
    }

    /// Inserts `chain`, which starts at `y`, at its position in the live
    /// order: after every chain left of its top vertex, and after a chain
    /// starting at the same vertex with a smaller or equal slope.
    fn insert(&mut self, chain: u32, y: f64) -> bool {
        let (x, slope) = (self.top(chain).x0, self.top(chain).slope);
        let (mut lo, mut hi) = (0, self.len);
        while lo < hi {
            let mid = lo.midpoint(hi);
            let other = self.live[mid];
            let segment = self.seek(other, y);
            let xo = self.x(other, segment, y);
            #[expect(clippy::float_cmp, reason = "a shared vertex ties exactly")]
            let before = xo < x || (xo == x && self.segs[segment as usize].slope <= slope);
            if before {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if !self.spend(self.len - lo) {
            return false;
        }
        for at in (lo..self.len).rev() {
            self.place(at + 1, self.live[at]);
        }
        self.place(lo, chain);
        self.len += 1;
        self.touch(chain);
        true
    }

    /// Removes the chains of `ending`, which end at the current ordinate,
    /// in one compaction of the live order; each one's neighbours are
    /// recorded, so the chains that become adjacent are checked.
    fn remove(&mut self, ending: &[[u32; 2]]) -> bool {
        let mut lowest = self.len;
        for &[_, chain] in ending {
            let at = self.word(chain, 6) as usize;
            if at > 0 {
                self.touch(self.live[at - 1]);
            }
            if at + 1 < self.len {
                self.touch(self.live[at + 1]);
            }
            self.set(chain, 6, NONE);
            lowest = lowest.min(at);
        }
        if !self.spend(self.len - lowest) {
            return false;
        }
        let mut kept = lowest;
        for at in lowest..self.len {
            let chain = self.live[at];
            if self.word(chain, 6) != NONE {
                self.place(kept, chain);
                kept += 1;
            }
        }
        self.len = kept;
        true
    }

    /// After every event at `y`: around each still-live chain whose
    /// neighbours changed, checks the directions — the first chain runs in
    /// the global sign, neighbours run opposite — and walks each adjacent
    /// pair not yet walked.
    fn settle(&mut self, y: f64) -> bool {
        for i in 0..self.changes {
            if !self.spend(1) {
                return false;
            }
            let chain = self.changed[i];
            let at = self.word(chain, 6);
            if at == NONE {
                continue;
            }
            let at = at as usize;
            let step = self.step(chain);
            if at == 0 {
                if self.sign == 0 {
                    self.sign = step;
                }
                if step != self.sign {
                    return false;
                }
            } else if !self.verify(self.live[at - 1], chain, y) {
                return false;
            }
            if at + 1 < self.len && !self.verify(chain, self.live[at + 1], y) {
                return false;
            }
        }
        self.changes = 0;
        true
    }

    /// Whether the adjacent pair `left`, `right` runs in opposite
    /// directions and keeps its gap from `y` on; a pair is walked once
    /// for as long as it stays adjacent.
    fn verify(&mut self, left: u32, right: u32, y: f64) -> bool {
        if self.step(left) == self.step(right) {
            return false;
        }
        if self.word(left, 5) == right {
            return true;
        }
        if !self.walk(left, right, y) {
            return false;
        }
        self.set(left, 5, right);
        true
    }

    /// Whether `left` stays at least the gap bound left of `right` from
    /// `y` to the bottom of the shorter of the two, apart from a shared
    /// top or bottom apex.
    #[expect(clippy::float_cmp, reason = "vertices are exact input values")]
    fn walk(&mut self, left: u32, right: u32, y: f64) -> bool {
        let end = self.bottom(left).y1.min(self.bottom(right).y1);
        let (mut a, mut b) = (self.seek(left, y), self.seek(right, y));
        let mut at = y;
        loop {
            if self.budget == 0 {
                return false;
            }
            self.budget -= 1;
            let (sa, sb) = (&self.segs[a as usize], &self.segs[b as usize]);
            let (xa, xb) = (self.x(left, a, at), self.x(right, b, at));
            let m = magnitude(sa) + magnitude(sb);
            if xb - xa < GAP_ULPS * f64::EPSILON * m {
                let spread = APEX_ULPS * f64::EPSILON * m / EPS;
                let apex = xa == xb
                    && if at == y {
                        at == self.top(left).y0
                            && at == self.top(right).y0
                            && sb.slope - sa.slope >= spread
                    } else {
                        at == end
                            && at == self.bottom(left).y1
                            && at == self.bottom(right).y1
                            && sa.slope - sb.slope >= spread
                    };
                if !apex {
                    return false;
                }
            }
            if at == end {
                return true;
            }
            // The next vertex of either chain; at an interior one the
            // chain that has it steps onto its segment below.
            at = sa.y1.min(sb.y1);
            if at != end {
                if self.segs[a as usize].y1 == at {
                    a = self.below(left, a);
                }
                if self.segs[b as usize].y1 == at {
                    b = self.below(right, b);
                }
            }
        }
    }
}

/// A bound on the magnitude of everything the sweep computes from `seg`:
/// both endpoints' coordinates and its slope times its ordinates.
fn magnitude(seg: &Seg) -> f64 {
    let rise = seg.y1 - seg.y0;
    seg.slope.abs().mul_add(seg.y0.abs() + rise, seg.x0.abs()) + seg.y0.abs() + seg.y1.abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(points: &[(f32, f32)]) -> Vec<(f32, f32, f32, f32)> {
        (0..points.len())
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                (a.0, a.1, b.0, b.1)
            })
            .collect()
    }

    fn disjoint(segments: &[(f32, f32, f32, f32)]) -> bool {
        prepare(segments).0
    }

    #[test]
    fn simple_shapes_are_disjoint() {
        let star_free = polygon(&[
            (10.0, 10.0),
            (40.0, 12.0),
            (35.0, 30.0),
            (20.0, 45.0),
            (5.0, 25.0),
        ]);
        assert!(disjoint(&star_free));
        let mut two = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        two.extend(polygon(&[
            (20.0, 5.0),
            (30.0, 5.0),
            (30.0, 15.0),
            (20.0, 15.0),
        ]));
        assert!(disjoint(&two));
    }

    #[test]
    fn overlap_touch_and_sign_mix_are_left_to_the_sweep() {
        let mut overlapping = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        overlapping.extend(polygon(&[
            (5.0, 5.0),
            (15.0, 5.0),
            (15.0, 15.0),
            (5.0, 15.0),
        ]));
        assert!(!disjoint(&overlapping));
        // Sharing an edge: coincident segments of opposite direction.
        let mut touching = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        touching.extend(polygon(&[
            (10.0, 0.0),
            (20.0, 0.0),
            (20.0, 10.0),
            (10.0, 10.0),
        ]));
        assert!(!disjoint(&touching));
        // Side by side with opposite orientation: the sweep sees both
        // signs in one band.
        let mut mixed = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        mixed.extend(polygon(&[
            (20.0, 0.0),
            (20.0, 10.0),
            (30.0, 10.0),
            (30.0, 0.0),
        ]));
        assert!(!disjoint(&mixed));
        let bowtie = polygon(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
        assert!(!disjoint(&bowtie));
    }

    /// Small deterministic generator for the property test.
    struct Lcg(u64);

    impl Lcg {
        #[expect(clippy::cast_precision_loss)]
        fn next(&mut self, scale: f32) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * scale
        }
    }

    /// The handover's contract: the merged order and ordinates are the
    /// ones the sweep's stable sort and sort-and-deduplicate produce, bit
    /// for bit — on grids full of ties, signed zeros, horizontal and
    /// non-finite segments, and chains broken and joined at every vertex.
    #[test]
    fn the_handed_over_order_is_the_sorted_order() {
        let mut rng = Lcg(0x2811_0002);
        let grid = [-2.0, -1.0, -0.0, 0.0, 1.0, 2.0, 3.5];
        for case in 0..3000 {
            let mut segments = Vec::new();
            let mut at = (0.0f32, 0.0f32);
            for _ in 0..=case % 40 {
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pick = |rng: &mut Lcg| grid[(rng.next(7.0) as usize).min(6)];
                let to = if case % 3 == 0 {
                    (rng.next(100.0), rng.next(100.0))
                } else {
                    (pick(&mut rng), pick(&mut rng))
                };
                // Mostly continue from the last vertex (chains), sometimes jump.
                let from = if rng.next(1.0) < 0.8 {
                    at
                } else {
                    (pick(&mut rng), pick(&mut rng))
                };
                let to = if rng.next(1.0) < 0.02 {
                    (f32::NAN, to.1)
                } else {
                    to
                };
                segments.push((from.0, from.1, to.0, to.1));
                at = to;
            }
            let prepared = prepare(&segments).1;
            let mut sorted: Vec<Seg> = prepared.segs.clone();
            sorted.sort_by(|a, b| a.y0.total_cmp(&b.y0));
            #[expect(clippy::cast_possible_truncation)]
            let mut ys: Vec<f32> = sorted
                .iter()
                .flat_map(|s| [s.y0 as f32, s.y1 as f32])
                .collect();
            ys.sort_by(f32::total_cmp);
            ys.dedup();
            let (merged, scratch, from) = prepared.into_ordered();
            let bits = |s: &Seg| {
                [s.x0, s.y0, s.y1, s.slope, s.dir].map(f64::to_bits)
            };
            assert_eq!(
                merged.iter().map(bits).collect::<Vec<_>>(),
                sorted.iter().map(bits).collect::<Vec<_>>(),
                "case {case}: segment order differs: {segments:?}"
            );
            assert_eq!(
                scratch[from..],
                ys.iter().map(|y| y.to_bits()).collect::<Vec<_>>(),
                "case {case}: ordinates differ: {segments:?}"
            );
        }
    }

    /// The test's contract: whenever it answers `true`, the sweep returns
    /// the input unchanged.
    #[test]
    fn disjoint_inputs_are_never_rewritten_by_the_sweep() {
        let mut rng = Lcg(0x2811);
        let mut accepted = 0;
        for case in 0..4000 {
            let mut segments = Vec::new();
            for _ in 0..=case % 4 {
                let (cx, cy) = (rng.next(200.0), rng.next(200.0));
                let n = 3 + (case % 9);
                let mut points = Vec::new();
                for k in 0..n {
                    #[expect(clippy::cast_precision_loss)]
                    let angle = (k as f32 + rng.next(0.9)) / n as f32 * std::f32::consts::TAU;
                    let radius = 5.0 + rng.next(60.0);
                    points.push((
                        radius.mul_add(angle.cos(), cx),
                        radius.mul_add(angle.sin(), cy),
                    ));
                }
                if case % 2 == 0 {
                    points.reverse();
                }
                segments.extend(polygon(&points));
            }
            if disjoint(&segments) {
                accepted += 1;
                for rule in [crate::FillRule::NonZero, crate::FillRule::EvenOdd] {
                    assert!(
                        super::super::sweep(prepare(&segments).1, rule).is_none(),
                        "case {case}: the test accepted an input the sweep rewrites: {segments:?}"
                    );
                }
            }
        }
        assert!(
            accepted > 1000,
            "the generator exercises acceptance ({accepted})"
        );
    }
}
