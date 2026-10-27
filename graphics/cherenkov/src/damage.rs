//! Device-space damage regions shared by the GPU and CPU backends.
//!
//! A [`Region`] accumulates the integer device-pixel rectangles of one
//! render target that have to be repainted. It is bookkeeping only — it
//! records coverage; deciding what produces damage, intersecting it with
//! a scissor, or handing it to a compositor is the integration work of
//! the engine and the backends that own a `Region`.
//!
//! Every mutating operation keeps three invariants:
//!
//! - each [`Rect`] is nonempty (`x0 < x1`, `y0 < y1`) and lies fully
//!   inside the target extent;
//! - the rectangles are pairwise disjoint — edge-adjacent coverage is
//!   fine, overlap never is;
//! - coverage equal to the whole extent normalizes to one rectangle
//!   spanning it, keeping [`Region::is_full`] a constant-time check.
//!
//! The rectangles live in a plain [`Vec`]: no tile grid, no count cap, no
//! per-pixel bitmap. Insertion scans the recorded rectangles and
//! subtracts each from the incoming coverage; a coalescing pass then
//! joins only exactly adjacent equal-span pairs, so clean holes are never
//! filled in.

use core::mem::size_of;

use crate::config::Bytes;

/// A nonempty axis-aligned rectangle in integer device pixels.
///
/// The rectangle covers the half-open ranges `x0..x1` by `y0..y1`. There
/// is no public constructor — a [`Region`] produces every `Rect`, clipped
/// to its extent, so a `Rect` a caller reads is always nonempty and
/// inside its region's target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Rect {
    /// The left edge, inclusive.
    #[must_use]
    pub const fn x0(&self) -> u32 {
        self.x0
    }

    /// The top edge, inclusive.
    #[must_use]
    pub const fn y0(&self) -> u32 {
        self.y0
    }

    /// The right edge, exclusive.
    #[must_use]
    pub const fn x1(&self) -> u32 {
        self.x1
    }

    /// The bottom edge, exclusive.
    #[must_use]
    pub const fn y1(&self) -> u32 {
        self.y1
    }

    /// The covered width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.x1 - self.x0
    }

    /// The covered height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.y1 - self.y0
    }

    /// `(x, y, width, height)` — the rectangle in the shape
    /// [`Region::add`] consumes.
    #[must_use]
    pub const fn xywh(&self) -> (u32, u32, u32, u32) {
        (self.x0, self.y0, self.width(), self.height())
    }

    /// The covered pixel count, widened: a full `u32` extent is wider
    /// than `u32`.
    fn area(self) -> u64 {
        u64::from(self.width()) * u64::from(self.height())
    }

    /// Whether `other` lies entirely inside `self`.
    const fn contains(self, other: Self) -> bool {
        self.x0 <= other.x0 && other.x1 <= self.x1 && self.y0 <= other.y0 && other.y1 <= self.y1
    }

    /// Whether the two rectangles cover at least one common pixel —
    /// edge-adjacent rectangles do not overlap.
    const fn overlaps(self, other: Self) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    /// Splits `fragment` into the pieces `self` does not cover and pushes
    /// them onto `out`: up to four rectangles. `self` must overlap
    /// `fragment` — the caller checks [`overlaps`](Self::overlaps) first.
    /// The pieces are the full-height left and right strips plus the top
    /// and bottom strips of the remaining columns, each pushed only when
    /// nonempty.
    fn subtract_from(self, fragment: Self, out: &mut Vec<Self>) {
        if fragment.x0 < self.x0 {
            out.push(Self {
                x1: self.x0,
                ..fragment
            });
        }
        if self.x1 < fragment.x1 {
            out.push(Self {
                x0: self.x1,
                ..fragment
            });
        }
        let (x0, x1) = (fragment.x0.max(self.x0), fragment.x1.min(self.x1));
        if fragment.y0 < self.y0 {
            out.push(Self {
                x0,
                x1,
                y0: fragment.y0,
                y1: self.y0,
            });
        }
        if self.y1 < fragment.y1 {
            out.push(Self {
                x0,
                x1,
                y0: self.y1,
                y1: fragment.y1,
            });
        }
    }
}

/// The damaged part of one fixed-size render target.
///
/// Construct with [`new`](Self::new) for the target's extent in device
/// pixels, [`add`](Self::add) each damaged rectangle, and read the
/// disjoint coverage back through [`rectangles`](Self::rectangles). The
/// extent is fixed for the region's life: a target that resizes replaces
/// its `Region`.
#[derive(Clone, Debug)]
pub struct Region {
    /// The target's `(width, height)` in device pixels.
    extent: (u32, u32),
    /// The recorded coverage: nonempty pairwise-disjoint rectangles
    /// inside `extent`, one full-extent rectangle when coverage is
    /// complete.
    rects: Vec<Rect>,
    /// Fragments of the coverage [`add`](Self::add) is inserting. Kept
    /// allocated across calls and [`clear`](Self::clear) so steady-state
    /// insertion does not allocate.
    scratch: Vec<Rect>,
}

impl Region {
    /// An empty damage region for a target of `extent.0` × `extent.1`
    /// device pixels. A zero-size extent keeps the region permanently
    /// empty: every [`add`](Self::add) clips away to nothing.
    #[must_use]
    pub const fn new(extent: (u32, u32)) -> Self {
        Self {
            extent,
            rects: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// The target extent in device pixels: `(width, height)`.
    #[must_use]
    pub const fn extent(&self) -> (u32, u32) {
        self.extent
    }

    /// Whether no pixel is damaged.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// Whether every pixel of the extent is damaged.
    ///
    /// Coverage equal to the extent is normalized to a single rectangle
    /// by [`add`](Self::add) and [`mark_full`](Self::mark_full), so this
    /// is a constant-time check. A zero-area extent can never be full —
    /// `mark_full` on it leaves the region empty.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.rects == [self.full()]
    }

    /// The damaged rectangles: nonempty, pairwise disjoint, inside the
    /// extent, in no specified order.
    #[must_use]
    pub const fn rectangles(&self) -> &[Rect] {
        self.rects.as_slice()
    }

    /// The exact number of damaged pixels.
    #[must_use]
    pub fn pixel_area(&self) -> u64 {
        self.rects.iter().map(|r| r.area()).sum()
    }

    /// The heap capacity the region holds, in bytes: the rectangle list
    /// plus the insertion scratch. Both survive [`clear`](Self::clear),
    /// so a region reporting nonzero bytes is not necessarily damaged.
    /// Reported for [`Engine::memory`](crate::Engine::memory) accounting.
    #[must_use]
    pub const fn heap_bytes(&self) -> Bytes {
        let rects = self.rects.capacity() + self.scratch.capacity();
        Bytes(rects as u64 * size_of::<Rect>() as u64)
    }

    /// Records the device-space rectangle `x, y, width, height` as
    /// damaged.
    ///
    /// The rectangle is clipped to the extent — `x + width` and
    /// `y + height` are computed in `u64`, so overflowing coordinates
    /// clip instead of wrapping — and a clipped-empty or zero-size
    /// rectangle adds nothing. Coverage already inside one recorded
    /// rectangle costs one scan of [`rectangles`](Self::rectangles) and
    /// no allocation.
    pub fn add(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let Some(added) = self.clip(x, y, width, height) else {
            return;
        };
        if self.rects.iter().any(|r| r.contains(added)) {
            return;
        }
        self.scratch.push(added);
        // Subtract every recorded rectangle from the pending fragments;
        // what survives is the coverage not already damaged.
        for &existing in &self.rects {
            let scratch = &mut self.scratch;
            let mut i = 0;
            while i < scratch.len() {
                let fragment = scratch[i];
                if existing.overlaps(fragment) {
                    scratch.swap_remove(i);
                    existing.subtract_from(fragment, scratch);
                } else {
                    i += 1;
                }
            }
            if scratch.is_empty() {
                return;
            }
        }
        self.rects.append(&mut self.scratch);
        if self.pixel_area() == self.extent_area() {
            self.rects.clear();
            self.rects.push(self.full());
        } else {
            self.coalesce();
        }
    }

    /// Marks every pixel of the extent damaged. A zero-area extent stays
    /// empty: no nonempty rectangle fits it.
    pub fn mark_full(&mut self) {
        self.rects.clear();
        if self.extent.0 != 0 && self.extent.1 != 0 {
            self.rects.push(self.full());
        }
    }

    /// Drops all recorded damage. Keeps the rectangle and scratch
    /// capacity for the next frame's insertions — see
    /// [`heap_bytes`](Self::heap_bytes).
    pub fn clear(&mut self) {
        self.rects.clear();
        self.scratch.clear();
    }

    /// The rectangle covering the whole extent — empty when the extent
    /// has zero area, and never recorded in that case.
    const fn full(&self) -> Rect {
        Rect {
            x0: 0,
            y0: 0,
            x1: self.extent.0,
            y1: self.extent.1,
        }
    }

    /// The extent's pixel count, widened.
    fn extent_area(&self) -> u64 {
        u64::from(self.extent.0) * u64::from(self.extent.1)
    }

    /// The `x, y, width, height` rectangle clipped to the extent, or
    /// `None` when nothing of it lands inside — a zero `width` or
    /// `height` included. The arithmetic runs in `u64` so `x + width` and
    /// `y + height` saturate the clip rather than wrap `u32`.
    fn clip(&self, x: u32, y: u32, width: u32, height: u32) -> Option<Rect> {
        let (ew, eh) = (u64::from(self.extent.0), u64::from(self.extent.1));
        let (x0, y0) = (u64::from(x).min(ew), u64::from(y).min(eh));
        let x1 = (u64::from(x) + u64::from(width)).min(ew);
        let y1 = (u64::from(y) + u64::from(height)).min(eh);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        Some(Rect {
            x0: u32::try_from(x0).expect("clipped to the u32 extent"),
            y0: u32::try_from(y0).expect("clipped to the u32 extent"),
            x1: u32::try_from(x1).expect("clipped to the u32 extent"),
            y1: u32::try_from(y1).expect("clipped to the u32 extent"),
        })
    }

    /// Joins exactly adjacent equal-span pairs until none remain: equal
    /// `x` spans touching in `y`, then equal `y` spans touching in `x`,
    /// repeated while merges happen. Each merge unites two recorded
    /// rectangles, so it never adds coverage and never fills a clean
    /// hole.
    fn coalesce(&mut self) {
        loop {
            let before = self.rects.len();
            merge_adjacent(&mut self.rects, true);
            merge_adjacent(&mut self.rects, false);
            if self.rects.len() == before {
                return;
            }
        }
    }
}

/// One coalescing pass over `rects`. `vertical` joins rectangles of equal
/// `x` span whose `y` ranges touch; `!vertical` joins equal `y` spans
/// whose `x` ranges touch.
fn merge_adjacent(rects: &mut Vec<Rect>, vertical: bool) {
    // Sorting groups equal spans into runs ordered along the merge axis.
    // Disjointness keeps each run's intervals strictly increasing, so one
    // scan merges every touching chain.
    if vertical {
        rects.sort_unstable_by_key(|r| (r.x0, r.x1, r.y0));
        rects.dedup_by(|next, kept| {
            if (kept.x0, kept.x1) == (next.x0, next.x1) && kept.y1 == next.y0 {
                kept.y1 = next.y1;
                true
            } else {
                false
            }
        });
    } else {
        rects.sort_unstable_by_key(|r| (r.y0, r.y1, r.x0));
        rects.dedup_by(|next, kept| {
            if (kept.y0, kept.y1) == (next.y0, next.y1) && kept.x1 == next.x0 {
                kept.x1 = next.x1;
                true
            } else {
                false
            }
        });
    }
}
