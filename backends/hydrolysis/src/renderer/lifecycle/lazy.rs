use super::*;
use std::collections::{BTreeMap, BTreeSet};
use waterui_layout::stack::{HStackLayout, VStackLayout};

#[derive(Default)]
pub(crate) struct LazyState {
    pub(crate) lazy_stack_controller: LazyStackController,
    pub(crate) lazy_list_controller: LazyListController,
    pub(crate) lazy_table_controller: LazyTableController,
    pub(crate) lazy_viewport_stack: Vec<vello::kurbo::Rect>,
    pub(crate) pending_scroll_handles: Vec<ScrollHandle>,
}

impl LazyState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.lazy_stack_controller.begin_rebuild_frame();
        self.lazy_list_controller.begin_rebuild_frame();
        self.lazy_table_controller.begin_rebuild_frame();
        self.lazy_viewport_stack.clear();
        self.pending_scroll_handles.clear();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.lazy_stack_controller.finish_rebuild_frame();
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
pub(crate) struct LazyStackController {
    pub(crate) slots: BTreeMap<LazyStackSlotKey, LazyStackSlot>,
    pub(crate) active_keys: BTreeSet<LazyStackSlotKey>,
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
pub(crate) struct LazyStackSlot {
    pub(crate) item_extents: Vec<Option<f64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LazyStackSlotKey {
    pub(crate) depth: usize,
    pub(crate) transform: [u64; 6],
    pub(crate) bounds: [u64; 4],
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum LazyStackAxisConfig {
    Vertical {
        spacing: f64,
        alignment: HorizontalAlignment,
    },
    Horizontal {
        spacing: f64,
        alignment: VerticalAlignment,
    },
}

impl LazyStackController {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.active_keys.clear();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.slots.retain(|key, _| self.active_keys.contains(key));
    }

    pub(crate) fn bind(&mut self, key: LazyStackSlotKey) -> &mut LazyStackSlot {
        self.active_keys.insert(key);
        self.slots.entry(key).or_default()
    }

    pub(crate) fn slot(&self, key: LazyStackSlotKey) -> &LazyStackSlot {
        self.slots
            .get(&key)
            .expect("lazy stack slot must be bound before read")
    }

    pub(crate) fn slot_mut(&mut self, key: LazyStackSlotKey) -> &mut LazyStackSlot {
        self.slots
            .get_mut(&key)
            .expect("lazy stack slot must be bound before mutation")
    }
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

impl LazyStackSlot {
    pub(crate) fn prepare_len(&mut self, len: usize) {
        self.item_extents.resize(len, None);
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

pub(crate) fn lazy_stack_axis_config(layout: &dyn Layout) -> LazyStackAxisConfig {
    let layout_any = layout as &dyn core::any::Any;
    if let Some(vstack) = layout_any.downcast_ref::<VStackLayout>() {
        return LazyStackAxisConfig::Vertical {
            spacing: f64::from(vstack.spacing),
            alignment: vstack.alignment,
        };
    }
    if let Some(hstack) = layout_any.downcast_ref::<HStackLayout>() {
        return LazyStackAxisConfig::Horizontal {
            spacing: f64::from(hstack.spacing),
            alignment: hstack.alignment,
        };
    }
    panic!("hydrolysis LazyContainer supports VStackLayout and HStackLayout only");
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

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: LazyStackSlotKey = LazyStackSlotKey {
        depth: 1,
        transform: [1, 0, 0, 1, 0, 0],
        bounds: [0, 0, 100, 100],
    };
    const KEY_B: LazyStackSlotKey = LazyStackSlotKey {
        depth: 1,
        transform: [1, 0, 0, 1, 0, 200],
        bounds: [0, 0, 100, 100],
    };

    #[test]
    fn lazy_stack_slots_are_bound_by_key_not_frame_order() {
        let mut controller = LazyStackController::default();
        controller.bind(KEY_A).prepare_len(1);
        controller.slot_mut(KEY_A).item_extents[0] = Some(24.0);
        controller.bind(KEY_B).prepare_len(1);
        controller.slot_mut(KEY_B).item_extents[0] = Some(48.0);
        controller.finish_rebuild_frame();

        controller.begin_rebuild_frame();
        controller.bind(KEY_B).prepare_len(1);

        assert_eq!(controller.slot(KEY_B).item_extents[0], Some(48.0));
        controller.finish_rebuild_frame();
        assert!(!controller.slots.contains_key(&KEY_A));
    }
}
