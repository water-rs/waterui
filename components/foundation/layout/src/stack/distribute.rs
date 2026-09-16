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
//! - **Layout priority orders who gives way.** Space is taken from the
//!   lowest-priority children first, and a band only starts compressing once every
//!   band below it has been squeezed to its minimums.

use alloc::vec::Vec;

use super::{Axis, stack_stretch_axis};
use crate::{HorizontalAlignment, ProposalSize, Size, SubView, VerticalAlignment, ViewDimensions};

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

pub(super) fn stack_spacing(spacing: f32, count: usize) -> f32 {
    spacing * usize_to_f32(count.saturating_sub(1))
}

/// Negotiates the same child measurements for sizing and placement.
/// Unspecified main axes retain intrinsic sizes. Finite offers distribute
/// through one priority/minimum pool, including children that stretch.
/// Changed allocations are measured again so wrapped content and guides
/// correspond to the exact proposal retained for recursive placement.
pub(super) fn measure_stack(
    axis: Axis,
    proposal: ProposalSize,
    spacing: f32,
    children: &[&dyn SubView],
) -> Vec<ChildMeasurement> {
    let main = main_proposal(axis, proposal);
    let probe = if main == Some(0.0) || main == Some(f32::INFINITY) {
        proposal
    } else {
        with_main(axis, proposal, None)
    };
    let mut measured: Vec<ChildMeasurement> = children
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
    let ideal_total: f64 = measured
        .iter()
        .map(|m| f64::from(main_extent(axis, m.size())))
        .sum();
    if !measured.iter().any(ChildMeasurement::stretches_main_axis)
        && !exceeds(ideal_total, available, children.len())
    {
        return measured;
    }
    let extents: Vec<Extent> = children
        .iter()
        .zip(&measured)
        .map(|(child, measurement)| {
            let minimum = main_extent(
                axis,
                child.measure(with_main(axis, proposal, Some(0.0))).size,
            );
            Extent {
                ideal: if measurement.stretches_main_axis() {
                    available.max(minimum)
                } else {
                    main_extent(axis, measurement.size())
                },
                min: minimum,
                priority: child.priority(),
            }
        })
        .collect();
    for ((child, measurement), allocated) in children
        .iter()
        .zip(&mut measured)
        .zip(compress_to_fit(&extents, available))
    {
        if !measurement.stretches_main_axis() && allocated >= main_extent(axis, measurement.size())
        {
            continue;
        }
        measurement.proposal = with_main(axis, proposal, Some(allocated));
        measurement.dimensions = child.measure(measurement.proposal);
        if measurement.stretches_main_axis() {
            if axis.is_horizontal() {
                measurement.dimensions.size.width = measurement.size().width.max(allocated);
            } else {
                measurement.dimensions.size.height = measurement.size().height.max(allocated);
            }
        }
    }
    measured
}

/// A child's room for manoeuvre on the main axis.
#[derive(Clone, Copy, Debug)]
pub struct Extent {
    /// The target extent before distributing a finite main-axis offer.
    pub ideal: f32,
    /// What the child reports when the axis is proposed `0` — its hard floor.
    pub min: f32,
    /// Higher wins space; ties share it.
    pub priority: i32,
}

/// Clamps `extents` into `available`, taking space from the lowest priorities
/// first and never pushing a child below its own minimum.
///
/// Returns the extent each child should occupy. When even every minimum together
/// exceeds `available` the result overflows rather than collapsing children to
/// nothing: an unreadable row is not a better answer than a clipped one.
pub fn compress_to_fit(extents: &[Extent], available: f32) -> Vec<f32> {
    let mut resolved: Vec<f32> = extents.iter().map(|extent| extent.ideal).collect();
    let total: f64 = resolved.iter().copied().map(f64::from).sum();
    if !exceeds(total, available, resolved.len()) {
        return resolved;
    }

    // Allocate higher priorities first, reserving every lower band's measured minimum.
    let mut priorities: Vec<i32> = extents.iter().map(|extent| extent.priority).collect();
    priorities.sort_unstable_by(|left, right| right.cmp(left));
    priorities.dedup();

    let mut allocated = 0.0;
    for priority in priorities {
        let band: Vec<usize> = extents
            .iter()
            .enumerate()
            .filter(|(_, extent)| extent.priority == priority)
            .map(|(index, _)| index)
            .collect();
        let reserved: f64 = extents
            .iter()
            .filter(|extent| extent.priority < priority)
            .map(|extent| f64::from(extent.min))
            .sum();
        let band_extents: Vec<Extent> = band.iter().map(|&index| extents[index]).collect();
        let budget = f64::from(available) - allocated - reserved;
        for (&index, extent) in band.iter().zip(water_fill(&band_extents, budget)) {
            resolved[index] = extent;
            allocated += f64::from(extent);
        }
    }

    resolved
}

/// Lowers a common cap until the band fits `target`, so the widest children give
/// up the most and equally-wide children shrink by the same amount — the property
/// that keeps a row of equal columns (a calendar week, say) at a uniform pitch
/// instead of crushing whichever happens to come first.
///
/// Each child is clamped into its own `[min, ideal]`, so a child already at its
/// floor stops contributing and the rest absorb the remainder.
fn water_fill(extents: &[Extent], target: f64) -> Vec<f32> {
    use num_traits::ToPrimitive;

    let mut total: f64 = extents.iter().map(|extent| f64::from(extent.min)).sum();
    if target <= total {
        return extents.iter().map(|extent| extent.min).collect();
    }
    let mut breakpoints: Vec<(f64, f64)> = extents
        .iter()
        .flat_map(|extent| {
            [
                (f64::from(extent.min), 1.0),
                (f64::from(extent.ideal), -1.0),
            ]
        })
        .collect();
    breakpoints.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));
    let mut cap = 0.0;
    let mut active = 0.0;
    for (breakpoint, change) in breakpoints {
        let next_total = total + active * (breakpoint - cap);
        if active > 0.0 && target <= next_total {
            let cap = (cap + (target - total) / active)
                .to_f32()
                .expect("water-filling cap must be representable as f32");
            return extents
                .iter()
                .map(|extent| extent.ideal.min(cap).max(extent.min))
                .collect();
        }
        total = next_total;
        cap = breakpoint;
        active += change;
    }
    extents.iter().map(|extent| extent.ideal).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn extent(ideal: f32, min: f32) -> Extent {
        Extent {
            ideal,
            min,
            priority: 0,
        }
    }

    #[test]
    fn insufficient_space_preserves_exact_minima() {
        let extents = [
            extent(120.0, 120.0),
            extent(164.0, 72.0),
            extent(120.0, 120.0),
        ];
        assert_eq!(compress_to_fit(&extents, 164.0), vec![120.0, 72.0, 120.0]);
    }

    #[test]
    fn one_flexible_child_receives_the_exact_remainder() {
        let extents = [extent(50.0, 50.0), extent(280.0, 0.0), extent(80.0, 80.0)];
        assert_eq!(compress_to_fit(&extents, 280.0), vec![50.0, 150.0, 80.0]);
    }

    #[test]
    fn large_ideals_do_not_erase_a_small_budget() {
        let extents = [extent(1.0e30, 0.0), extent(1.0e30, 0.0)];
        assert_eq!(compress_to_fit(&extents, 2.0), vec![1.0, 1.0]);
    }

    #[test]
    fn everything_fits_is_a_no_op() {
        let extents = vec![extent(30.0, 10.0), extent(40.0, 10.0)];
        assert_eq!(compress_to_fit(&extents, 100.0), vec![30.0, 40.0]);
    }

    #[test]
    fn equal_children_shrink_equally() {
        // Seven equal columns in a bound 56pt short: every column must lose the
        // same 8pt rather than the leading ones absorbing it all.
        let extents: Vec<Extent> = (0..7).map(|_| extent(40.0, 0.0)).collect();
        let resolved = compress_to_fit(&extents, 224.0);
        for width in resolved {
            assert!((width - 32.0).abs() < 0.01, "expected 32pt, got {width}");
        }
    }

    #[test]
    fn the_widest_child_absorbs_the_deficit_alone() {
        // A 50pt label beside a 200pt text in 140pt: the text gives way and the
        // label keeps its intrinsic width.
        let extents = vec![extent(50.0, 0.0), extent(200.0, 0.0)];
        let resolved = compress_to_fit(&extents, 140.0);
        assert!((resolved[0] - 50.0).abs() < 0.01, "got {}", resolved[0]);
        assert!((resolved[1] - 90.0).abs() < 0.01, "got {}", resolved[1]);
    }

    #[test]
    fn a_child_never_shrinks_below_its_reported_minimum() {
        // The second child cannot go below 80, so the first absorbs everything
        // it can and the row overflows by the remainder.
        let extents = vec![extent(100.0, 20.0), extent(100.0, 80.0)];
        let resolved = compress_to_fit(&extents, 120.0);
        assert!(resolved[1] >= 80.0 - 0.01, "got {}", resolved[1]);
        assert!(resolved[0] >= 20.0 - 0.01, "got {}", resolved[0]);
    }

    #[test]
    fn lower_priority_gives_way_first() {
        let extents = vec![
            Extent {
                ideal: 100.0,
                min: 0.0,
                priority: 1,
            },
            Extent {
                ideal: 100.0,
                min: 0.0,
                priority: 0,
            },
        ];
        let resolved = compress_to_fit(&extents, 150.0);
        assert!(
            (resolved[0] - 100.0).abs() < 0.01,
            "the prioritized child keeps its ideal, got {}",
            resolved[0]
        );
        assert!(
            (resolved[1] - 50.0).abs() < 0.01,
            "the lower-priority child gives up the whole deficit, got {}",
            resolved[1]
        );
    }
}
