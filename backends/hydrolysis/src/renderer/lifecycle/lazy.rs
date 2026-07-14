use super::*;
use waterui_layout::stack::{HStackLayout, VStackLayout};

#[derive(Default)]
pub(crate) struct LazyState {
    pub(crate) lazy_list_controller: LazyListController,
    pub(crate) lazy_table_controller: LazyTableController,
    pub(crate) lazy_viewport_stack: Vec<vello::kurbo::Rect>,
    pub(crate) pending_scroll_handles: Vec<ScrollHandle>,
}

impl LazyState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.lazy_list_controller.begin_rebuild_frame();
        self.lazy_table_controller.begin_rebuild_frame();
        self.lazy_viewport_stack.clear();
        self.pending_scroll_handles.clear();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.lazy_list_controller.finish_rebuild_frame();
        self.lazy_table_controller.finish_rebuild_frame();
    }

    pub(crate) fn push_pending_scroll_handle(&mut self, handle: ScrollHandle) {
        self.pending_scroll_handles.push(handle);
    }

    pub(crate) fn take_pending_scroll_handle(&mut self, caller: &'static str) -> ScrollHandle {
        self.pending_scroll_handles
            .pop()
            .unwrap_or_else(|| panic!("hydrolysis {caller} requires prebound scroll handle"))
    }
}

#[derive(Debug, Default)]
pub(crate) struct LazyListController {
    pub(crate) slots: Vec<LazyListSlot>,
    pub(crate) cursor: usize,
}

#[derive(Debug, Default)]
pub(crate) struct LazyTableController {
    pub(crate) slots: Vec<LazyTableSlot>,
    pub(crate) cursor: usize,
}

#[derive(Debug, Default)]
pub(crate) struct LazyListSlot {
    pub(crate) row_extents: Vec<Option<f64>>,
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

impl LazyListController {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    pub(crate) fn bind(&mut self) -> usize {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("lazy list controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(LazyListSlot::default());
        }
        index
    }
}

impl LazyTableController {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    pub(crate) fn bind(&mut self) -> usize {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("lazy table controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(LazyTableSlot::default());
        }
        index
    }
}

impl LazyListSlot {
    pub(crate) fn prepare_len(&mut self, len: usize) {
        self.row_extents.resize(len, None);
    }
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

pub(crate) fn sum_cached_or_estimated(extents: &[Option<f64>], estimate: f64) -> f64 {
    extents
        .iter()
        .map(|extent| extent.unwrap_or(estimate))
        .sum::<f64>()
}

pub(crate) fn resolve_visible_index_window(
    count: usize,
    start_offset: f64,
    end_offset: f64,
    mut extent_at: impl FnMut(usize) -> f64,
) -> VisibleIndexWindow {
    if count == 0 {
        return VisibleIndexWindow {
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
        let extent = extent_at(index);
        if offset + extent > clamped_start {
            break;
        }
        offset += extent;
        index += 1;
    }
    let start = index.min(count);
    let leading_offset = offset;
    while index < count && offset < clamped_end {
        offset += extent_at(index);
        index += 1;
    }
    VisibleIndexWindow {
        start,
        end: index.min(count),
        leading_offset,
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
