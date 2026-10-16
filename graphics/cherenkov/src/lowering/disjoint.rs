//! The exact pre-sweep test of [`resolve_winding`](super::resolve_winding):
//! it proves, without the band sweep, that a path's flattened segments
//! cannot form an overlapping region, so the sweep would return its input
//! unchanged.
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
//! # The test
//!
//! The segments are grouped into maximal connected y-monotone chains in
//! input order and swept by chain events only — a chain's top inserts it,
//! its bottom removes it — keeping the live chains in x order. Every pair
//! of chains that becomes adjacent is walked over its whole remaining
//! common y-span, and after every event ordinate the winding prefix of
//! the chain order is checked. The test returns `true` only when:
//!
//! 1. every coordinate is finite with magnitude at most `2^19`, and every
//!    ordinate is `0` or at least `2^-6` in magnitude;
//! 2. at every vertex ordinate of a walked pair, the right chain's `x`
//!    exceeds the left chain's by at least `GAP_ULPS·ε·m`, `m` the two
//!    segments' coordinate magnitudes; the only exception is a pair of
//!    chains that start (or end) at one shared vertex, whose segments
//!    there diverge with slopes at least `APEX_ULPS·ε·m / EPS` apart;
//! 3. every winding prefix is `0` or one global sign `s ∈ {1, -1}`.
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
//! the input bounds the walks, so the test stays `O(n + k log k)` for `k`
//! chains; exhausting it is a doubt too.

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
/// Walk steps the test may spend per segment before it leaves the input
/// to the sweep.
const BUDGET_PER_SEGMENT: usize = 16;

/// The scratch words one chain occupies: top segment, bottom segment,
/// direction, bottom `x` (`f32` bits), cursor, verified right neighbour.
const CHAIN_WORDS: usize = 6;
/// No chain: an empty verified-neighbour slot.
const NONE: u32 = u32::MAX;

/// Segments in input order, normalized like the sweep's, and a scratch
/// buffer of at least `2 * segs.len()` words the sweep reuses for its
/// ordinates.
pub(super) struct Prepared {
    pub(super) segs: Vec<Seg>,
    pub(super) scratch: Vec<u32>,
}

/// Normalizes `segments` exactly as the sweep does (non-finite and
/// horizontal segments dropped, each oriented top to bottom) and reports
/// whether they provably form no overlapping region. The prepared buffers
/// go to the sweep when they do not.
#[expect(
    clippy::float_cmp,
    reason = "connectivity and the horizontal test compare exact f32 input values"
)]
pub(super) fn prepare<S: Copy + Into<(f32, f32, f32, f32)>>(segments: &[S]) -> (bool, Prepared) {
    // Pass 1: count the sweep's segments and the chains, and check the
    // coordinate bounds, without allocating.
    let mut count = 0usize;
    let mut chains = 0usize;
    let mut applicable = true;
    let mut last: Option<(f32, f32, bool)> = None;
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
        if last != Some((x0, y0, down)) {
            chains += 1;
        }
        last = Some((x1, y1, down));
        count += 1;
    }
    let mut segs = Vec::with_capacity(count);
    // Chain records, start events, end events and the live order.
    let words = chains * (CHAIN_WORDS + 3);
    let mut scratch = Vec::with_capacity(words.max(2 * count));
    let mut last: Option<(f32, f32, bool)> = None;
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
        if applicable {
            if last == Some((x0, y0, down)) {
                // Extend the open chain: its bottom moves down (a downward
                // chain) or its top moves up (an upward one), and the
                // cursor with it, since walks start from the top.
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
                scratch.extend_from_slice(&[
                    index,
                    index,
                    step.cast_unsigned(),
                    bottom_x.to_bits(),
                    index,
                    NONE,
                ]);
            }
        }
        last = Some((x1, y1, down));
        let (x0, y0, x1, y1) = (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
        segs.push(if down {
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
        });
    }
    let disjoint = applicable && Sweep::new(&segs, &mut scratch, chains).run();
    (disjoint, Prepared { segs, scratch })
}

/// The chain-level sweep over the prepared segments. Chain records live in
/// `scratch[..k * CHAIN_WORDS]`, followed by the start events, the end
/// events and the live order.
struct Sweep<'a> {
    segs: &'a [Seg],
    chains: &'a mut [u32],
    starts: &'a mut [u32],
    ends: &'a mut [u32],
    live: &'a mut [u32],
    len: usize,
    budget: usize,
}

impl<'a> Sweep<'a> {
    fn new(segs: &'a [Seg], scratch: &'a mut Vec<u32>, chains: usize) -> Self {
        let records = chains * CHAIN_WORDS;
        scratch.resize(chains * (CHAIN_WORDS + 3), 0);
        let (chain_words, rest) = scratch.split_at_mut(records);
        let (starts, rest) = rest.split_at_mut(chains);
        let (ends, live) = rest.split_at_mut(chains);
        for (i, (start, end)) in starts.iter_mut().zip(ends.iter_mut()).enumerate() {
            let id = u32::try_from(i).expect("chain count fits u32");
            *start = id;
            *end = id;
        }
        Self {
            segs,
            chains: chain_words,
            starts,
            ends,
            live,
            len: 0,
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

    #[expect(clippy::float_cmp, reason = "events group at exactly equal ordinates")]
    fn run(mut self) -> bool {
        let segs = self.segs;
        self.starts.sort_unstable_by(|&a, &b| {
            let (sa, sb) = (
                &segs[self.chains[a as usize * CHAIN_WORDS] as usize],
                &segs[self.chains[b as usize * CHAIN_WORDS] as usize],
            );
            sa.y0
                .total_cmp(&sb.y0)
                .then(sa.x0.total_cmp(&sb.x0))
                .then(sa.slope.total_cmp(&sb.slope))
        });
        self.ends.sort_unstable_by(|&a, &b| {
            segs[self.chains[a as usize * CHAIN_WORDS + 1] as usize]
                .y1
                .total_cmp(&segs[self.chains[b as usize * CHAIN_WORDS + 1] as usize].y1)
        });
        let (mut next_start, mut next_end) = (0, 0);
        let mut sign = 0i32;
        while next_end < self.ends.len() {
            let y_end = self.bottom(self.ends[next_end]).y1;
            let y = match self.starts.get(next_start) {
                Some(&s) => self.top(s).y0.min(y_end),
                None => y_end,
            };
            while next_end < self.ends.len() && self.bottom(self.ends[next_end]).y1 == y {
                if !self.remove(self.ends[next_end], y) {
                    return false;
                }
                next_end += 1;
            }
            while next_start < self.starts.len() && self.top(self.starts[next_start]).y0 == y {
                self.insert(self.starts[next_start], y);
                next_start += 1;
            }
            if !self.settle(y, &mut sign) {
                return false;
            }
        }
        true
    }

    /// Inserts `chain`, which starts at `y`, at its position in the live
    /// order: after every chain left of its top vertex, and after a chain
    /// starting at the same vertex with a smaller or equal slope.
    fn insert(&mut self, chain: u32, y: f64) {
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
        self.live.copy_within(lo..self.len, lo + 1);
        self.live[lo] = chain;
        self.len += 1;
    }

    /// Removes `chain`, which ends at `y`. Chains are strictly ordered
    /// between vertices, so it sits among the chains whose `x` at `y` equals
    /// its bottom vertex — the partners of a shared bottom apex.
    #[expect(clippy::float_cmp, reason = "a shared vertex ties exactly")]
    fn remove(&mut self, chain: u32, y: f64) -> bool {
        let x = self.bottom_x(chain);
        let (mut lo, mut hi) = (0, self.len);
        while lo < hi {
            let mid = lo.midpoint(hi);
            let other = self.live[mid];
            let segment = self.seek(other, y);
            if self.x(other, segment, y) < x {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        while lo < self.len && self.live[lo] != chain {
            let other = self.live[lo];
            let segment = self.seek(other, y);
            if self.x(other, segment, y) != x {
                return false;
            }
            lo += 1;
        }
        if lo == self.len {
            return false;
        }
        self.live.copy_within(lo + 1..self.len, lo);
        self.len -= 1;
        true
    }

    /// After every event at `y`: checks each winding prefix of the live
    /// order and walks each adjacent pair not yet walked.
    fn settle(&mut self, y: f64, sign: &mut i32) -> bool {
        let mut winding = 0;
        for i in 0..self.len {
            if self.budget == 0 {
                return false;
            }
            self.budget -= 1;
            let chain = self.live[i];
            winding += self.step(chain);
            if winding != 0 {
                if *sign == 0 {
                    *sign = winding;
                }
                if winding != *sign {
                    return false;
                }
            }
            if i + 1 < self.len {
                let right = self.live[i + 1];
                if self.word(chain, 5) != right {
                    if !self.walk(chain, right, y) {
                        return false;
                    }
                    self.set(chain, 5, right);
                }
            }
        }
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
