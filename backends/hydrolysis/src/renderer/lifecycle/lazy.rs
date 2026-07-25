use super::*;
use waterui_layout::stack::{HStackLayout, VStackLayout};

#[derive(Default)]
pub(crate) struct LazyState {
    pub(crate) lazy_viewport_stack: Vec<vello::kurbo::Rect>,
}

impl LazyState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.lazy_viewport_stack.clear();
    }
}

#[derive(Debug, Default)]
pub(crate) struct LazyTableSlot {
    pub(crate) column_widths: Vec<f64>,
    pub(crate) max_rows: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VisibleIndexWindow {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) leading_offset: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VisibleColumnWindow {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) leading_offset: f64,
}

#[derive(Debug, Clone)]
pub(crate) enum LazyStackAxisConfig {
    Vertical {
        spacing: nami::Computed<f32>,
        alignment: HorizontalAlignment,
    },
    Horizontal {
        spacing: nami::Computed<f32>,
        alignment: VerticalAlignment,
    },
}

impl LazyTableSlot {
    pub(crate) fn prepare_columns(
        &mut self,
        len: usize,
        metrics: waterui_backend_core::widget::TableMetrics,
    ) {
        self.column_widths.resize(len, metrics.min_column_width);
    }
}

/// Resolves a [`LazyContainer`]'s layout to the viewport-virtualized stack axis,
/// or `None` for any other layout. A `Some` container is rendered by the lazy
/// scroll-virtualization path (recycles rows by visible window); a `None`
/// container (e.g. `AbsoluteLayout`/`ZStackLayout` overlay) is rendered by the
/// retained per-id collection path, which keeps one cached subtree per item id
/// and reconciles membership changes incrementally.
pub(crate) fn lazy_stack_axis_config(layout: &dyn Layout) -> Option<LazyStackAxisConfig> {
    let layout_any = layout as &dyn core::any::Any;
    if let Some(vstack) = layout_any.downcast_ref::<VStackLayout>() {
        return Some(LazyStackAxisConfig::Vertical {
            spacing: vstack.spacing.clone(),
            alignment: vstack.alignment,
        });
    }
    if let Some(hstack) = layout_any.downcast_ref::<HStackLayout>() {
        return Some(LazyStackAxisConfig::Horizontal {
            spacing: hstack.spacing.clone(),
            alignment: hstack.alignment,
        });
    }
    None
}

/// Places one lazy-stack item: given its measured `size`, `stretch_axis`, the
/// container `bounds`, and the running main-axis `cursor`, returns the item's rect.
/// Shared by the immediate-mode `render_lazy_container` handler and the retained
/// render tree's `LazyStackNode` so the cross-axis sizing/alignment rules live in
/// exactly one place.
pub(crate) fn place_lazy_stack_item(
    axis_config: &LazyStackAxisConfig,
    stretch_axis: StretchAxis,
    size: waterui_core::layout::Size,
    bounds: vello::kurbo::Rect,
    cursor: f64,
) -> vello::kurbo::Rect {
    match axis_config {
        LazyStackAxisConfig::Vertical { alignment, .. } => {
            assert!(
                !(matches!(
                    stretch_axis,
                    StretchAxis::Vertical | StretchAxis::Both | StretchAxis::MainAxis
                )),
                "hydrolysis LazyContainer VStackLayout does not support children stretching on main axis"
            );
            let child_width = if matches!(
                stretch_axis,
                StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::CrossAxis
            ) || size.width.is_infinite()
            {
                bounds.width()
            } else {
                f64::from(size.width).min(bounds.width())
            };
            let child_height = f64::from(size.height);
            let x = if *alignment == HorizontalAlignment::Leading {
                bounds.x0
            } else if *alignment == HorizontalAlignment::Trailing {
                bounds.x1 - child_width
            } else {
                bounds.x0 + (bounds.width() - child_width) / 2.0
            };
            vello::kurbo::Rect::new(x, cursor, x + child_width, cursor + child_height)
        }
        LazyStackAxisConfig::Horizontal { alignment, .. } => {
            assert!(
                !(matches!(
                    stretch_axis,
                    StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::MainAxis
                )),
                "hydrolysis LazyContainer HStackLayout does not support children stretching on main axis"
            );
            let child_width = f64::from(size.width);
            let child_height = if matches!(
                stretch_axis,
                StretchAxis::Vertical | StretchAxis::Both | StretchAxis::CrossAxis
            ) || size.height.is_infinite()
            {
                bounds.height()
            } else {
                f64::from(size.height).min(bounds.height())
            };
            let y = if *alignment == VerticalAlignment::Top {
                bounds.y0
            } else if *alignment == VerticalAlignment::Bottom {
                bounds.y1 - child_height
            } else {
                bounds.y0 + (bounds.height() - child_height) / 2.0
            };
            vello::kurbo::Rect::new(cursor, y, cursor + child_width, y + child_height)
        }
    }
}

/// Main-axis extent index for a virtualized collection.
///
/// Unknown items use one stable estimate. Measured items update a Fenwick tree,
/// so deep viewport resolution and programmatic item jumps never materialize or
/// linearly scan preceding rows.
#[derive(Debug, Default)]
pub(crate) struct VirtualExtentIndex {
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
        for index in 1..=count {
            self.fenwick[index] = estimated_stride * (index & index.wrapping_neg()) as f64;
        }
    }

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
            tree_index += tree_index & tree_index.wrapping_neg();
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

pub(crate) fn resolve_visible_column_window(
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

pub(crate) fn resolve_table_visible_rows(
    offset_y: f64,
    viewport_height: f64,
    max_rows: usize,
    metrics: waterui_backend_core::widget::TableMetrics,
) -> VisibleIndexWindow {
    let data_start = (offset_y - metrics.header_height).max(0.0);
    let data_end = (offset_y + viewport_height - metrics.header_height).max(0.0);
    let start = ((data_start / metrics.row_height).floor() as usize).min(max_rows);
    let end = ((data_end / metrics.row_height).ceil() as usize).min(max_rows);
    VisibleIndexWindow {
        start,
        end: end.max(start),
        leading_offset: start as f64 * metrics.row_height,
    }
}

pub(crate) fn table_metrics_from_slot(
    slot: &LazyTableSlot,
    metrics: waterui_backend_core::widget::TableMetrics,
) -> MeasuredTableMetrics {
    MeasuredTableMetrics {
        column_widths: slot.column_widths.clone(),
        table_width: slot.column_widths.iter().sum(),
        table_height: metrics.header_height + metrics.row_height * slot.max_rows as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::VirtualExtentIndex;

    #[test]
    fn deep_window_resolves_without_measuring_preceding_items() {
        let mut index = VirtualExtentIndex::default();
        index.reset(100_000, 48.0, 4.0);

        let window = index.visible_window(90_000.0 * 52.0, 90_010.0 * 52.0);

        assert_eq!(window.start, 90_000);
        assert_eq!(window.end, 90_010);
        assert_eq!(window.leading_offset, 90_000.0 * 52.0);
        assert!(index.measured(89_999).is_none());
    }

    #[test]
    fn measured_extents_update_offsets_and_visible_window() {
        let mut index = VirtualExtentIndex::default();
        index.reset(4, 10.0, 2.0);
        index.set_measured(0, 20.0);
        index.set_measured(2, 5.0);

        assert_eq!(index.total_extent(), 51.0);
        assert_eq!(index.offset_of(1), 22.0);
        assert_eq!(index.offset_of(3), 41.0);
        let window = index.visible_window(22.0, 41.0);
        assert_eq!(window.start, 1);
        assert_eq!(window.end, 3);
        assert_eq!(window.leading_offset, 22.0);
    }

    #[test]
    fn empty_index_has_an_empty_window() {
        let mut index = VirtualExtentIndex::default();
        index.reset(0, 10.0, 0.0);

        let window = index.visible_window(500.0, 600.0);

        assert_eq!(window.start, 0);
        assert_eq!(window.end, 0);
        assert_eq!(index.total_extent(), 0.0);
    }
}
