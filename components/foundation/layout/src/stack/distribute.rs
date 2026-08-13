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

/// A child's room for manoeuvre on the main axis.
#[derive(Clone, Copy, Debug)]
pub struct Extent {
    /// What the child reports when the axis is unconstrained.
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
    let total: f32 = resolved.iter().sum();
    if !exceeds(total, available, resolved.len()) {
        return resolved;
    }

    // Lowest priority gives way first, so walk the bands upward and stop as soon
    // as the deficit is covered.
    let mut priorities: Vec<i32> = extents.iter().map(|extent| extent.priority).collect();
    priorities.sort_unstable();
    priorities.dedup();

    let mut deficit = total - available;
    for priority in priorities {
        if deficit <= 0.0 {
            break;
        }
        let band: Vec<usize> = extents
            .iter()
            .enumerate()
            .filter(|(_, extent)| extent.priority == priority)
            .map(|(index, _)| index)
            .collect();

        let band_ideal: f32 = band.iter().map(|&index| extents[index].ideal).sum();
        let band_floor: f32 = band.iter().map(|&index| extents[index].min).sum();
        let absorbed = deficit.min(band_ideal - band_floor).max(0.0);
        if absorbed <= 0.0 {
            continue;
        }

        let band_extents: Vec<Extent> = band.iter().map(|&index| extents[index]).collect();
        for (slot, width) in band
            .iter()
            .zip(water_fill(&band_extents, band_ideal - absorbed))
        {
            resolved[*slot] = width;
        }
        deficit -= absorbed;
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
fn water_fill(extents: &[Extent], target: f32) -> Vec<f32> {
    let at_cap = |cap: f32| -> f32 {
        extents
            .iter()
            .map(|extent| extent.ideal.min(cap).max(extent.min))
            .sum()
    };

    let mut low = extents
        .iter()
        .map(|extent| extent.min)
        .fold(0.0_f32, f32::min);
    let mut high = extents
        .iter()
        .map(|extent| extent.ideal)
        .fold(0.0_f32, f32::max);

    // The cap is monotone in the resulting total, and the extents are piecewise
    // linear in it, so bisection converges on the exact level. Forty steps takes
    // an f32 interval below its own precision.
    for _ in 0..40 {
        let mid = f32::midpoint(low, high);
        if at_cap(mid) > target {
            high = mid;
        } else {
            low = mid;
        }
    }

    extents
        .iter()
        .map(|extent| extent.ideal.min(low).max(extent.min))
        .collect()
}

/// Whether `total` exceeds `available` by more than the rounding error of
/// measuring and then re-adding the same row.
///
/// Callers check this before probing children for their minimums: that probe is
/// only worth its cost when something actually has to give.
pub fn exceeds(total: f32, available: f32, terms: usize) -> bool {
    let magnitude = total.abs().max(available.abs()).max(1.0);
    total - available > f32::EPSILON * magnitude * usize_to_f32(terms.max(1))
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
