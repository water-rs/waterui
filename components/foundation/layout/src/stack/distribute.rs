//! Main-axis space distribution shared by the stacks.
//!
//! The maths is identical for `HStack` and `VStack` once the axis is projected to
//! a scalar, so it lives here rather than twice. Everything works on plain extents:
//! the caller extracts widths or heights, calls in, and writes the answer back.
//!
//! Two rules come from `SwiftUI` and are the reason this is not just a division:
//!
//! - **A child never shrinks below the minimum it reports.** That minimum is
//!   measured (propose `0` on the axis), never assumed — the previous fixed floor
//!   squeezed a large control to an unusable size and let a small label claim more
//!   than it could use.
//! - **Layout priority orders who negotiates first.** Bands receive their
//!   offers from the highest priority down while every unprocessed child
//!   retains its measured minimum reservation; inside a band the least
//!   flexible child negotiates first, with original logical order breaking
//!   ties.

use smallvec::SmallVec;

use super::{Axis, stack_stretch_axis};
use crate::{
    HorizontalAlignment, Point, ProposalSize, Rect, Size, SubView, SubviewPlacement,
    VerticalAlignment, ViewDimensions,
};

/// Cached measurement for a child during layout
pub(super) struct ChildMeasurement {
    pub(super) dimensions: ViewDimensions,
    /// The proposal that produced `dimensions`; it is the proposal the child
    /// is recursively placed with.
    pub(super) proposal: ProposalSize,
    main_stretch: bool,
    cross_stretch: bool,
}

impl ChildMeasurement {
    pub(super) const fn size(&self) -> Size {
        self.dimensions.size
    }

    pub(super) const fn stretches_main_axis(&self) -> bool {
        self.main_stretch
    }

    pub(super) const fn stretches_cross_axis(&self) -> bool {
        self.cross_stretch
    }

    pub(super) fn vertical_guide(&self, alignment: VerticalAlignment) -> f32 {
        self.dimensions.vertical(alignment)
    }

    pub(super) fn horizontal_guide(&self, alignment: HorizontalAlignment) -> f32 {
        self.dimensions.horizontal(alignment)
    }
}

const fn main_extent(axis: Axis, size: Size) -> f32 {
    if axis.is_horizontal() {
        size.width
    } else {
        size.height
    }
}

const fn main_proposal(axis: Axis, proposal: ProposalSize) -> Option<f32> {
    if axis.is_horizontal() {
        proposal.width
    } else {
        proposal.height
    }
}

const fn with_main(axis: Axis, proposal: ProposalSize, main: Option<f32>) -> ProposalSize {
    if axis.is_horizontal() {
        proposal.with_width(main)
    } else {
        proposal.with_height(main)
    }
}

/// Allocates cross-axis fill without shrinking a finite measured minimum.
pub(super) const fn place_cross_extent(measured: f32, available: f32, stretch: bool) -> f32 {
    if measured.is_infinite() {
        available
    } else if stretch {
        measured.max(available)
    } else {
        measured
    }
}

/// Where a container anchors the envelope its children form on an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineAnchor {
    /// The envelope sits at the leading edge (`Top`, `Leading`).
    Leading,
    /// The envelope is centred in the container's extent.
    Center,
    /// The envelope sits at the trailing edge (`Bottom`, `Trailing`).
    Trailing,
    /// A custom guide: the envelope sits at the leading edge.
    Custom,
}

impl LineAnchor {
    pub fn vertical(alignment: VerticalAlignment) -> Self {
        if alignment == VerticalAlignment::Top {
            Self::Leading
        } else if alignment == VerticalAlignment::Center {
            Self::Center
        } else if alignment == VerticalAlignment::Bottom {
            Self::Trailing
        } else {
            Self::Custom
        }
    }

    pub fn horizontal(alignment: HorizontalAlignment) -> Self {
        if alignment == HorizontalAlignment::Leading {
            Self::Leading
        } else if alignment == HorizontalAlignment::Center {
            Self::Center
        } else if alignment == HorizontalAlignment::Trailing {
            Self::Trailing
        } else {
            Self::Custom
        }
    }
}

/// The envelope a set of children forms on one axis when their guides are
/// lined up: `above` is how far the furthest guide sits from the leading edge
/// of the envelope, `below` how far the furthest trailing edge sits past the
/// line, so the envelope's extent is `above + below`. Guides are not clamped:
/// an explicit guide outside its child moves the envelope, as `SwiftUI`'s
/// does. Children with an infinite extent on the axis take no part (the fill
/// pass hands them the container's extent).
pub fn cross_envelope(children: impl IntoIterator<Item = (f32, f32)>) -> (f32, f32) {
    let mut above = f32::NEG_INFINITY;
    let mut below = f32::NEG_INFINITY;
    for (extent, guide) in children {
        if !extent.is_finite() {
            continue;
        }
        above = above.max(guide);
        below = below.max(extent - guide);
    }
    if above.is_finite() {
        (above, below)
    } else {
        (0.0, 0.0)
    }
}

/// Where the container's alignment line sits inside `extent`, the cross
/// extent it was placed in: the envelope is anchored by the alignment and the
/// line is `above` past the envelope's leading edge. With default guides this
/// is `0`, `extent / 2` and `extent` for the three edge alignments, and the
/// envelope's own line for a custom guide. A child is placed at
/// `line - guide`.
pub fn container_line(extent: f32, anchor: LineAnchor, above: f32, below: f32) -> f32 {
    let slack = extent - (above + below);
    let offset = match anchor {
        LineAnchor::Leading | LineAnchor::Custom => 0.0,
        LineAnchor::Center => slack * 0.5,
        LineAnchor::Trailing => slack,
    };
    offset + above
}

/// The stack's members: the children that take a slot.
///
/// A child that renders nothing ([`SubView::is_empty`] — `WaterUI`'s empty
/// view, possibly under layout-transparent wrappers) is not a stack member:
/// it takes no slot and no spacing. Membership is therefore the child list
/// minus the empties, in order, and `Layout::place` still answers one
/// placement per child — a non-member's placement is a zero-size frame at
/// the cursor it would have occupied, since nothing is drawn there anyway.
pub(super) fn stack_members<'v>(children: &[&'v dyn SubView]) -> SmallVec<[&'v dyn SubView; 4]> {
    children
        .iter()
        .copied()
        .filter(|child| !child.is_empty())
        .collect()
}

/// The placement a non-member child gets: nothing drawn, so the frame only
/// needs to be valid — a zero-size rect at `origin` under the container's
/// negotiated `proposal`.
pub(super) const fn empty_placement(origin: Point, proposal: ProposalSize) -> SubviewPlacement {
    SubviewPlacement::new(Rect::new(origin, Size::zero()), proposal)
}

pub(super) fn stack_spacing(spacing: f32, count: usize) -> f32 {
    spacing * usize_to_f32(count.saturating_sub(1))
}

/// Negotiates the same child measurements for sizing and placement.
/// Unspecified main axes retain intrinsic sizes. A finite main extent is
/// negotiated sequentially: priority bands from highest to lowest, the least
/// flexible child in a band first, and each child's reported extent — its
/// answer, or `max(answer, offer)` for a main-axis stretcher — deducted
/// before the next child is offered (layout-spec §4.2). The proposal each
/// child was measured with is retained for recursive placement.
pub(super) fn measure_stack(
    axis: Axis,
    proposal: ProposalSize,
    spacing: f32,
    children: &[&dyn SubView],
) -> SmallVec<[ChildMeasurement; 4]> {
    let main = main_proposal(axis, proposal);
    let probe = if main == Some(0.0) || main == Some(f32::INFINITY) {
        proposal
    } else {
        with_main(axis, proposal, None)
    };
    let mut measured: SmallVec<[ChildMeasurement; 4]> = children
        .iter()
        .map(|child| {
            let physical = stack_stretch_axis(axis, &[child.stretch_axis()]);
            let horizontal = physical.stretches_horizontal();
            let vertical = physical.stretches_vertical();
            ChildMeasurement {
                dimensions: child.measure(probe),
                proposal: probe,
                main_stretch: if axis.is_horizontal() {
                    horizontal
                } else {
                    vertical
                },
                cross_stretch: if axis.is_horizontal() {
                    vertical
                } else {
                    horizontal
                },
            }
        })
        .collect();
    let Some(main) = main.filter(|value| value.is_finite() && *value != 0.0) else {
        return measured;
    };
    let available = (main - stack_spacing(spacing, children.len())).max(0.0);
    let slots: SmallVec<[Slot; 4]> = children
        .iter()
        .zip(&measured)
        .enumerate()
        .map(|(index, (child, measurement))| {
            let minimum = main_extent(
                axis,
                child.measure(with_main(axis, proposal, Some(0.0))).size,
            );
            let maximum = if measurement.stretches_main_axis() {
                f32::INFINITY
            } else {
                main_extent(
                    axis,
                    child
                        .measure(with_main(axis, proposal, Some(f32::INFINITY)))
                        .size,
                )
            };
            Slot {
                index,
                minimum,
                target: if measurement.stretches_main_axis() {
                    available.max(minimum)
                } else {
                    maximum.min(available).max(minimum)
                },
                flexibility: maximum - minimum,
                priority: child.priority(),
            }
        })
        .collect();
    negotiate(axis, proposal, available, children, &slots, &mut measured);
    measured
}

/// The inputs retained per child while a finite main extent is negotiated.
struct Slot {
    /// The child's slot in logical member order — the tiebreak inside a band.
    index: usize,
    /// The child's measured minimum: its answer to main proposal `0`.
    minimum: f32,
    /// The extent the child claims: `max(minimum, available)` for a main-axis
    /// stretcher, otherwise its maximum-probe answer clamped into
    /// `[minimum, available]`.
    target: f32,
    /// Maximum-probe answer minus minimum; a stretcher's unbounded maximum
    /// negotiates it last inside its band.
    flexibility: f32,
    /// The child's band; higher bands negotiate first.
    priority: i32,
}

/// Runs the finite main-axis negotiation (layout-spec §4.2).
///
/// When every target fits, each child is offered its target; when the minima
/// alone overflow `available`, each child is offered its minimum and the row
/// overflows rather than collapsing children. Otherwise children negotiate in
/// order — priority bands high to low, flexibility low to high inside a band,
/// logical member order on ties — and each offer reserves the minima of every
/// unprocessed lower-priority child and of the not-yet-offered peers in its
/// band:
///
/// `offer = max(minimum, min(target, remaining_band_budget / k, remaining_band_budget - peer_minima))`
///
/// A child's reported extent is deducted before the next offer, so extent a
/// compressed child declines remains available to the children still
/// negotiating. A processed child is never re-offered; an answer larger than
/// its offer overflows rather than being clipped.
fn negotiate(
    axis: Axis,
    proposal: ProposalSize,
    available: f32,
    children: &[&dyn SubView],
    slots: &[Slot],
    measured: &mut [ChildMeasurement],
) {
    use num_traits::ToPrimitive;
    let count = slots.len();
    let total_targets: f64 = slots.iter().map(|slot| f64::from(slot.target)).sum();
    if !exceeds(total_targets, available, count) {
        for slot in slots {
            select(
                axis,
                proposal,
                children[slot.index],
                &mut measured[slot.index],
                slot.target,
            );
        }
        return;
    }
    let total_minima: f64 = slots.iter().map(|slot| f64::from(slot.minimum)).sum();
    if exceeds(total_minima, available, count) {
        for slot in slots {
            select(
                axis,
                proposal,
                children[slot.index],
                &mut measured[slot.index],
                slot.minimum,
            );
        }
        return;
    }
    // One sort: bands high to low, flexibility low to high inside a band;
    // `sort_by` is stable, so equal flexibility keeps logical member order.
    let mut order: SmallVec<[usize; 4]> = (0..count).collect();
    order.sort_by(|&a, &b| {
        slots[b]
            .priority
            .cmp(&slots[a].priority)
            .then_with(|| slots[a].flexibility.total_cmp(&slots[b].flexibility))
    });
    // Suffix minimum sums over the sorted order: the bands below the one
    // ending at `band_end` reserve `lower_minima[band_end]`.
    let mut lower_minima = SmallVec::<[f64; 4]>::from_elem(0.0, count + 1);
    for position in (0..count).rev() {
        lower_minima[position] =
            lower_minima[position + 1] + f64::from(slots[order[position]].minimum);
    }
    let mut reported = 0.0_f64;
    let mut cursor = 0;
    while cursor < count {
        let priority = slots[order[cursor]].priority;
        let mut band_end = cursor + 1;
        while band_end < count && slots[order[band_end]].priority == priority {
            band_end += 1;
        }
        let reserved_below = lower_minima[band_end];
        let mut band_minima: f64 = (cursor..band_end)
            .map(|position| f64::from(slots[order[position]].minimum))
            .sum();
        for position in cursor..band_end {
            let slot = &slots[order[position]];
            let unprocessed = usize_to_f64(band_end - position);
            let budget = f64::from(available) - reported - reserved_below;
            let offer = f64::from(slot.target)
                .min(budget / unprocessed)
                .min(budget - (band_minima - f64::from(slot.minimum)))
                .max(f64::from(slot.minimum))
                .to_f32()
                .expect("a negotiated offer is f32-representable");
            reported += f64::from(select(
                axis,
                proposal,
                children[slot.index],
                &mut measured[slot.index],
                offer,
            ));
            band_minima -= f64::from(slot.minimum);
        }
        cursor = band_end;
    }
}

/// Measures `child` at `offer` on the main axis, retains the offer as its
/// selected proposal, and returns the reported main extent: a main-axis
/// stretcher reports `max(answer, offer)`.
fn select(
    axis: Axis,
    proposal: ProposalSize,
    child: &dyn SubView,
    measurement: &mut ChildMeasurement,
    offer: f32,
) -> f32 {
    measurement.proposal = with_main(axis, proposal, Some(offer));
    measurement.dimensions = child.measure(measurement.proposal);
    if measurement.stretches_main_axis() {
        if axis.is_horizontal() {
            measurement.dimensions.size.width = measurement.size().width.max(offer);
        } else {
            measurement.dimensions.size.height = measurement.size().height.max(offer);
        }
    }
    main_extent(axis, measurement.size())
}

/// Whether `total` exceeds `available` by more than the rounding error of
/// measuring and then re-adding the same row.
///
/// Callers check this before probing children for their minimums: that probe is
/// only worth its cost when something actually has to give.
pub fn exceeds(total: f64, available: f32, terms: usize) -> bool {
    let available = f64::from(available);
    let magnitude = total.abs().max(available.abs()).max(1.0);
    total > available
        && (total.is_infinite()
            || total - available
                > f64::from(f32::EPSILON) * magnitude * f64::from(usize_to_f32(terms.max(1))))
}

fn usize_to_f32(value: usize) -> f32 {
    use num_traits::ToPrimitive;
    value
        .to_f32()
        .expect("child count must be representable as f32")
}

fn usize_to_f64(value: usize) -> f64 {
    use num_traits::ToPrimitive;
    value
        .to_f64()
        .expect("child count must be representable as f64")
}
