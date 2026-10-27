// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use waterui_layout::stack::LazyStackAxis;

#[derive(Default)]
pub struct LazyState {
    pub lazy_viewport_stack: Vec<LazyViewport>,
}

/// Viewport geometry with the coordinate transform of its scroll content.
#[derive(Clone, Copy)]
pub struct LazyViewport {
    pub(crate) bounds: kurbo::Rect,
    pub(crate) transform: kurbo::Affine,
}

impl LazyState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.lazy_viewport_stack.clear();
    }
}

#[derive(Debug, Default)]
pub struct LazyTableSlot {
    pub(crate) column_widths: Vec<f64>,
    pub max_rows: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct VisibleIndexWindow {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub leading_offset: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct VisibleColumnWindow {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) leading_offset: f64,
}

#[derive(Debug, Clone)]
pub enum LazyStackAxisConfig {
    Vertical {
        spacing: nami::Computed<f32>,
        alignment: HorizontalAlignment,
        direction: nami::Computed<waterui_core::layout::LayoutDirection>,
    },
    Horizontal {
        spacing: nami::Computed<f32>,
        alignment: VerticalAlignment,
        direction: nami::Computed<waterui_core::layout::LayoutDirection>,
    },
}

impl LazyStackAxisConfig {
    pub(crate) const fn direction(&self) -> &nami::Computed<waterui_core::layout::LayoutDirection> {
        match self {
            Self::Vertical { direction, .. } | Self::Horizontal { direction, .. } => direction,
        }
    }
}

impl LazyTableSlot {
    pub(crate) fn prepare_columns(&mut self, len: usize, metrics: crate::theme::TableMetrics) {
        self.column_widths.resize(len, metrics.min_column_width);
    }
}

/// Resolves a [`LazyContainer`]'s layout to the viewport-virtualized stack axis,
/// or `None` for any other layout. A `Some` container is rendered by the lazy
/// scroll-virtualization path (recycles rows by visible window); a `None`
/// container (e.g. `AbsoluteLayout`/`ZStackLayout` overlay) is rendered by the
/// retained per-id collection path, which keeps one cached subtree per item id
/// and reconciles membership changes incrementally.
pub fn lazy_stack_axis_config(
    layout: &dyn Layout,
    direction: nami::Computed<waterui_core::layout::LayoutDirection>,
) -> Option<LazyStackAxisConfig> {
    waterui_layout::stack::lazy_stack_axis(layout).map(|axis| match axis {
        LazyStackAxis::Vertical { spacing, alignment } => LazyStackAxisConfig::Vertical {
            spacing,
            alignment,
            direction,
        },
        LazyStackAxis::Horizontal { spacing, alignment } => LazyStackAxisConfig::Horizontal {
            spacing,
            alignment,
            direction,
        },
    })
}

/// A lazy-stack item's cross extent and whether it stretches the cross axis,
/// following the per-child protocol of the layout specification (§4.1, §5): a
/// cross-axis stretcher takes `max(measured, bounds)`, an unbounded
/// measurement is a stretcher that takes the bounds, and any other item keeps
/// its own measured answer. Nothing is clamped: an item wider than the bounds
/// overflows them, and an overflowing stretcher is pinned to the bounds' min
/// edge rather than alignment-placed.
const fn lazy_cross_extent(measured: f64, flagged: bool, bounds: f64) -> (f64, bool) {
    if measured.is_infinite() {
        (bounds, true)
    } else if flagged {
        (measured.max(bounds), true)
    } else {
        (measured, false)
    }
}

/// Places one lazy-stack item: given its measured `size`, `stretch_axis`, the
/// container `bounds`, and the running main-axis `cursor`, returns the item's rect.
/// `LazyStackNode` in `renderer/tree/collection.rs` is the caller, so the
/// cross-axis sizing/alignment rules live in exactly one place.
pub fn place_lazy_stack_item(
    axis_config: &LazyStackAxisConfig,
    stretch_axis: StretchAxis,
    size: waterui_core::layout::Size,
    bounds: kurbo::Rect,
    cursor: f64,
) -> kurbo::Rect {
    match axis_config {
        LazyStackAxisConfig::Vertical { alignment, .. } => {
            let (child_width, stretches) = lazy_cross_extent(
                f64::from(size.width),
                matches!(
                    stretch_axis,
                    StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::CrossAxis
                ),
                bounds.width(),
            );
            let child_height = f64::from(size.height);
            let logical_x = if stretches || *alignment == HorizontalAlignment::Leading {
                bounds.x0
            } else if *alignment == HorizontalAlignment::Trailing {
                bounds.x1 - child_width
            } else {
                bounds.x0 + (bounds.width() - child_width) / 2.0
            };
            let x = if axis_config.direction().snapshot().is_right_to_left() {
                bounds.x0 + bounds.x1 - logical_x - child_width
            } else {
                logical_x
            };
            kurbo::Rect::new(x, cursor, x + child_width, cursor + child_height)
        }
        LazyStackAxisConfig::Horizontal { alignment, .. } => {
            let child_width = f64::from(size.width);
            let (child_height, stretches) = lazy_cross_extent(
                f64::from(size.height),
                matches!(
                    stretch_axis,
                    StretchAxis::Vertical | StretchAxis::Both | StretchAxis::CrossAxis
                ),
                bounds.height(),
            );
            let y = if stretches || *alignment == VerticalAlignment::Top {
                bounds.y0
            } else if *alignment == VerticalAlignment::Bottom {
                bounds.y1 - child_height
            } else {
                bounds.y0 + (bounds.height() - child_height) / 2.0
            };
            let x = if axis_config.direction().snapshot().is_right_to_left() {
                bounds.x0 + bounds.x1 - cursor - child_width
            } else {
                cursor
            };
            kurbo::Rect::new(x, y, x + child_width, y + child_height)
        }
    }
}

/// Main-axis extent index for a virtualized collection.
///
/// Unknown items use one stable estimate. Measured items update a Fenwick tree,
/// so deep viewport resolution and programmatic item jumps never materialize or
/// linearly scan preceding rows.
#[derive(Debug, Default)]
pub struct VirtualExtentIndex {
    measured: Vec<Option<f64>>,
    fenwick: Vec<f64>,
    estimate: f64,
    spacing: f64,
}

impl VirtualExtentIndex {
    pub(crate) fn reset(&mut self, count: usize, estimate: f64, spacing: f64) {
        assert!(
            estimate.is_finite() && estimate > 0.0,
            "virtualized item estimate must be finite and positive"
        );
        assert!(
            spacing.is_finite() && spacing >= 0.0,
            "virtualized item spacing must be finite and non-negative"
        );
        self.measured.clear();
        self.measured.resize(count, None);
        self.fenwick.clear();
        self.fenwick.resize(count + 1, 0.0);
        self.estimate = estimate;
        self.spacing = spacing;
        let estimated_stride = estimate + spacing;
        for (index, slot) in self.fenwick.iter_mut().enumerate().take(count + 1).skip(1) {
            *slot = estimated_stride * crate::num_cast::usize_as_f64(index.isolate_lowest_one());
        }
    }

    // exact comparison intended — matches() validates an unmodified estimate against itself
    #[expect(
        clippy::float_cmp,
        reason = "compare the unmodified estimate field exactly; approximate equality would accept drifted caches"
    )]
    pub(crate) fn matches(&self, count: usize, estimate: f64, spacing: f64) -> bool {
        self.measured.len() == count && self.estimate == estimate && self.spacing == spacing
    }

    pub(crate) fn set_measured(&mut self, index: usize, extent: f64) {
        assert!(
            extent.is_finite() && extent >= 0.0,
            "virtualized item extent must be finite and non-negative"
        );
        let previous = self.measured[index].unwrap_or(self.estimate);
        self.measured[index] = Some(extent);
        let delta = extent - previous;
        let mut tree_index = index + 1;
        while tree_index < self.fenwick.len() {
            self.fenwick[tree_index] += delta;
            tree_index += tree_index.isolate_lowest_one();
        }
    }

    #[must_use]
    pub(crate) fn measured(&self, index: usize) -> Option<f64> {
        self.measured[index]
    }

    #[must_use]
    pub(crate) fn offset_of(&self, index: usize) -> f64 {
        assert!(
            index <= self.measured.len(),
            "virtualized item index {index} exceeds collection length {}",
            self.measured.len()
        );
        self.prefix_sum(index)
    }

    #[must_use]
    pub(crate) fn total_extent(&self) -> f64 {
        let count = self.measured.len();
        if count == 0 {
            0.0
        } else {
            self.prefix_sum(count) - self.spacing
        }
    }

    #[must_use]
    pub(crate) fn visible_window(&self, start_offset: f64, end_offset: f64) -> VisibleIndexWindow {
        let count = self.measured.len();
        if count == 0 {
            return VisibleIndexWindow {
                start: 0,
                end: 0,
                leading_offset: 0.0,
            };
        }
        let total = self.total_extent();
        let clamped_start = start_offset.clamp(0.0, total);
        let clamped_end = end_offset.max(clamped_start).clamp(0.0, total);
        let start = self.partition_prefix(clamped_start, true).min(count);
        let end = if clamped_end <= 0.0 || start == count {
            start
        } else {
            self.partition_prefix(clamped_end, false)
                .saturating_add(1)
                .min(count)
                .max(start)
        };
        VisibleIndexWindow {
            start,
            end,
            leading_offset: self.prefix_sum(start),
        }
    }

    fn prefix_sum(&self, count: usize) -> f64 {
        let mut index = count;
        let mut sum = 0.0;
        while index > 0 {
            sum += self.fenwick[index];
            index &= index - 1;
        }
        sum
    }

    /// Returns the largest prefix length whose sum is `< threshold`, or `<=`
    /// it when `inclusive` is true.
    fn partition_prefix(&self, threshold: f64, inclusive: bool) -> usize {
        let mut index = 0usize;
        let mut sum = 0.0;
        let mut bit = self.measured.len().next_power_of_two();
        while bit != 0 {
            let next = index + bit;
            if next < self.fenwick.len() {
                let candidate = sum + self.fenwick[next];
                let accepted = if inclusive {
                    candidate <= threshold
                } else {
                    candidate < threshold
                };
                if accepted {
                    index = next;
                    sum = candidate;
                }
            }
            bit >>= 1;
        }
        index
    }
}

pub fn resolve_visible_column_window(
    widths: &[f64],
    start_offset: f64,
    end_offset: f64,
) -> VisibleColumnWindow {
    let count = widths.len();
    if count == 0 {
        return VisibleColumnWindow {
            start: 0,
            end: 0,
            leading_offset: 0.0,
        };
    }
    let clamped_start = start_offset.max(0.0);
    let clamped_end = end_offset.max(clamped_start);
    let mut index = 0usize;
    let mut offset = 0.0;
    while index < count {
        let width = widths[index];
        if offset + width > clamped_start {
            break;
        }
        offset += width;
        index += 1;
    }
    let start = index.min(count);
    let leading_offset = offset;
    while index < count && offset < clamped_end {
        offset += widths[index];
        index += 1;
    }
    VisibleColumnWindow {
        start,
        end: index.min(count),
        leading_offset,
    }
}

pub fn resolve_table_visible_rows(
    offset_y: f64,
    viewport_height: f64,
    max_rows: usize,
    metrics: crate::theme::TableMetrics,
) -> VisibleIndexWindow {
    let data_start = (offset_y - metrics.header_height).max(0.0);
    let data_end = (offset_y + viewport_height - metrics.header_height).max(0.0);
    let start =
        (crate::num_cast::f64_as_usize((data_start / metrics.row_height).floor())).min(max_rows);
    let end = (crate::num_cast::f64_as_usize((data_end / metrics.row_height).ceil())).min(max_rows);
    VisibleIndexWindow {
        start,
        end: end.max(start),
        leading_offset: crate::num_cast::usize_as_f64(start) * metrics.row_height,
    }
}

pub fn table_metrics_from_slot(
    slot: &LazyTableSlot,
    metrics: crate::theme::TableMetrics,
) -> MeasuredTableMetrics {
    MeasuredTableMetrics {
        column_widths: slot.column_widths.clone(),
        table_width: slot.column_widths.iter().sum(),
        table_height: metrics.row_height.mul_add(
            crate::num_cast::usize_as_f64(slot.max_rows),
            metrics.header_height,
        ),
    }
}

#[cfg(test)]
mod tests {
    use nami::Computed;
    use waterui_core::layout::{
        HorizontalAlignment, LayoutDirection, Size, StretchAxis, VerticalAlignment,
    };

    use super::{LazyStackAxisConfig, VirtualExtentIndex, place_lazy_stack_item};

    #[test]
    fn deep_window_resolves_without_measuring_preceding_items() {
        let mut index = VirtualExtentIndex::default();
        index.reset(100_000, 48.0, 4.0);

        let window = index.visible_window(90_000.0 * 52.0, 90_010.0 * 52.0);

        assert_eq!(window.start, 90_000);
        assert_eq!(window.end, 90_010);
        approx::assert_relative_eq!(window.leading_offset, 90_000.0 * 52.0);
        assert!(index.measured(89_999).is_none());
    }

    #[test]
    fn measured_extents_update_offsets_and_visible_window() {
        let mut index = VirtualExtentIndex::default();
        index.reset(4, 10.0, 2.0);
        index.set_measured(0, 20.0);
        index.set_measured(2, 5.0);

        approx::assert_relative_eq!(index.total_extent(), 51.0);
        approx::assert_relative_eq!(index.offset_of(1), 22.0);
        approx::assert_relative_eq!(index.offset_of(3), 41.0);
        let window = index.visible_window(22.0, 41.0);
        assert_eq!(window.start, 1);
        assert_eq!(window.end, 3);
        approx::assert_relative_eq!(window.leading_offset, 22.0);
    }

    #[test]
    fn empty_index_has_an_empty_window() {
        let mut index = VirtualExtentIndex::default();
        index.reset(0, 10.0, 0.0);

        let window = index.visible_window(500.0, 600.0);

        assert_eq!(window.start, 0);
        assert_eq!(window.end, 0);
        approx::assert_relative_eq!(index.total_extent(), 0.0);
    }

    #[test]
    fn rtl_horizontal_stack_places_first_item_at_the_right_edge() {
        let axis = LazyStackAxisConfig::Horizontal {
            spacing: Computed::constant(0.0),
            alignment: VerticalAlignment::Top,
            direction: Computed::constant(LayoutDirection::RightToLeft),
        };

        let rect = place_lazy_stack_item(
            &axis,
            StretchAxis::None,
            Size::new(20.0, 10.0),
            kurbo::Rect::new(0.0, 0.0, 100.0, 100.0),
            0.0,
        );

        assert_eq!(rect, kurbo::Rect::new(80.0, 0.0, 100.0, 10.0));
    }

    #[test]
    fn vertical_stack_keeps_main_axis_stretch_item_at_ideal_extent() {
        let axis = LazyStackAxisConfig::Vertical {
            spacing: Computed::constant(0.0),
            alignment: HorizontalAlignment::Leading,
            direction: Computed::constant(LayoutDirection::LeftToRight),
        };

        let rect = place_lazy_stack_item(
            &axis,
            StretchAxis::MainAxis,
            Size::new(20.0, 30.0),
            kurbo::Rect::new(0.0, 0.0, 100.0, 100.0),
            8.0,
        );

        assert_eq!(rect, kurbo::Rect::new(0.0, 8.0, 20.0, 38.0));
    }

    #[test]
    fn horizontal_stack_keeps_main_axis_stretch_item_at_ideal_extent() {
        let axis = LazyStackAxisConfig::Horizontal {
            spacing: Computed::constant(0.0),
            alignment: VerticalAlignment::Top,
            direction: Computed::constant(LayoutDirection::LeftToRight),
        };

        let rect = place_lazy_stack_item(
            &axis,
            StretchAxis::MainAxis,
            Size::new(30.0, 20.0),
            kurbo::Rect::new(0.0, 0.0, 100.0, 100.0),
            8.0,
        );

        assert_eq!(rect, kurbo::Rect::new(8.0, 0.0, 38.0, 20.0));
    }

    #[test]
    fn cross_placement_follows_direction_alignment_and_stretch() {
        use StretchAxis::{CrossAxis, Horizontal, None as NoStretch};
        const INF: f32 = f32::INFINITY;
        let (leading, center, trailing) = (
            HorizontalAlignment::Leading,
            HorizontalAlignment::Center,
            HorizontalAlignment::Trailing,
        );
        let ltr = LayoutDirection::LeftToRight;
        let rtl = LayoutDirection::RightToLeft;
        // Vertical stack in 100 x 100 with the cursor at 8:
        // (direction, alignment, stretch, measured width, expected x0..x1).
        let vertical = [
            // Narrow non-stretchers follow alignment, mirrored in RTL.
            (ltr, leading, NoStretch, 20.0, (0.0, 20.0)),
            (ltr, center, NoStretch, 20.0, (40.0, 60.0)),
            (ltr, trailing, NoStretch, 20.0, (80.0, 100.0)),
            (rtl, leading, NoStretch, 20.0, (80.0, 100.0)),
            (rtl, center, NoStretch, 20.0, (40.0, 60.0)),
            (rtl, trailing, NoStretch, 20.0, (0.0, 20.0)),
            // Overflowing non-stretchers are unclamped and alignment-placed.
            (ltr, leading, NoStretch, 300.0, (0.0, 300.0)),
            (ltr, center, NoStretch, 300.0, (-100.0, 200.0)),
            (ltr, trailing, NoStretch, 300.0, (-200.0, 100.0)),
            (rtl, leading, NoStretch, 300.0, (-200.0, 100.0)),
            // Narrow stretchers fill the bounds whatever the alignment.
            (ltr, trailing, Horizontal, 20.0, (0.0, 100.0)),
            (rtl, trailing, CrossAxis, 20.0, (0.0, 100.0)),
            // Overflowing stretchers keep their extent, pinned to the min
            // edge for every alignment; RTL mirrors that pin.
            (ltr, leading, Horizontal, 300.0, (0.0, 300.0)),
            (ltr, center, CrossAxis, 300.0, (0.0, 300.0)),
            (ltr, trailing, StretchAxis::Both, 300.0, (0.0, 300.0)),
            (rtl, trailing, Horizontal, 300.0, (-200.0, 100.0)),
            // An unbounded measurement is a stretcher taking the bounds.
            (ltr, trailing, NoStretch, INF, (0.0, 100.0)),
        ];
        for (direction, alignment, stretch, width, (x0, x1)) in vertical {
            let axis = LazyStackAxisConfig::Vertical {
                spacing: Computed::constant(0.0),
                alignment,
                direction: Computed::constant(direction),
            };
            let rect = place_lazy_stack_item(
                &axis,
                stretch,
                Size::new(width, 10.0),
                kurbo::Rect::new(0.0, 0.0, 100.0, 100.0),
                8.0,
            );
            assert_eq!(
                rect,
                kurbo::Rect::new(x0, 8.0, x1, 18.0),
                "vertical {direction:?} {alignment:?} {stretch:?} width={width}"
            );
        }

        let (top, bottom) = (VerticalAlignment::Top, VerticalAlignment::Bottom);
        // Horizontal stack in 100 x 100 with the cursor at 8 (LTR):
        // (alignment, stretch, measured height, expected y0..y1).
        let horizontal = [
            (top, NoStretch, 20.0, (0.0, 20.0)),
            (VerticalAlignment::Center, NoStretch, 20.0, (40.0, 60.0)),
            (bottom, NoStretch, 20.0, (80.0, 100.0)),
            (bottom, NoStretch, 300.0, (-200.0, 100.0)),
            (bottom, StretchAxis::Vertical, 20.0, (0.0, 100.0)),
            (bottom, StretchAxis::Vertical, 300.0, (0.0, 300.0)),
            (bottom, NoStretch, INF, (0.0, 100.0)),
        ];
        for (alignment, stretch, height, (y0, y1)) in horizontal {
            let axis = LazyStackAxisConfig::Horizontal {
                spacing: Computed::constant(0.0),
                alignment,
                direction: Computed::constant(ltr),
            };
            let rect = place_lazy_stack_item(
                &axis,
                stretch,
                Size::new(10.0, height),
                kurbo::Rect::new(0.0, 0.0, 100.0, 100.0),
                8.0,
            );
            assert_eq!(
                rect,
                kurbo::Rect::new(8.0, y0, 18.0, y1),
                "horizontal {alignment:?} {stretch:?} height={height}"
            );
        }
    }
}
