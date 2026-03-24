use crate::renderer::lazy::{
    resolve_table_visible_rows, resolve_visible_column_window, table_metrics_from_slot,
};
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext, TABLE_HEADER_HEIGHT,
    measure_table_metrics,
    refresh_table_slot_baseline, table_data_cell_rect, table_header_cell_rect, transformed_rect,
    update_table_slot_visible_cell_widths,
};
#[cfg(feature = "accessibility")]
use accesskit::{Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole};
use nami::Signal;
use waterui::component::table::{TableColumn, TableConfig};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::views::Views;
use waterui_core::{AnyView, Environment, Native};
use waterui_layout::scroll::Axis as ScrollAxis;

use super::{draw_scroll_indicators, inset_rect, widget_theme};

impl HydroNativeView for Native<TableConfig> {
    fn render(
        ctx: &mut WidgetRenderContext<'_>,
        view: Self,
        env: &Environment
    ) {
        render_table(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let columns = view.as_inner().columns.get();
        if columns.is_empty() {
            return LayoutSize::zero();
        }
        let metrics = measure_table_metrics(&columns, state, env);
        LayoutSize::new(metrics.table_width as f32, metrics.table_height as f32)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment
    ) {
        let columns = renderer.read_signal(&view.as_inner().columns);
        if columns.is_empty() {
            return;
        }
        let slot_index = renderer.lazy.lazy_table_controller.bind();
        {
            let (slot, state) = renderer.table_slot_and_state_mut(slot_index);
            refresh_table_slot_baseline(&columns, slot, state, env);
        }
        let viewport = ctx.bounds;
        let table_metrics = {
            let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
            table_metrics_from_slot(slot)
        };
        let handle = renderer.bind_scroll_handle(
            ScrollAxis::All,
            viewport.width(),
            viewport.height(),
            table_metrics.table_width.max(viewport.width()),
            table_metrics.table_height.max(viewport.height()),
        );
        #[cfg(feature = "accessibility")]
        {
            let scroll_metrics = handle.metrics();
            let row_window = {
                let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
                resolve_table_visible_rows(scroll_metrics.offset_y, viewport.height(), slot.max_rows)
            };
            let mut column_window = {
                let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
                resolve_visible_column_window(
                    &slot.column_widths,
                    scroll_metrics.offset_x,
                    scroll_metrics.offset_x + viewport.width(),
                )
            };
            {
                {
                    let (slot, state) = renderer.table_slot_and_state_mut(slot_index);
                    update_table_slot_visible_cell_widths(
                        &columns,
                        slot,
                        row_window,
                        column_window,
                        state,
                        env,
                    );
                }
                let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
                column_window = resolve_visible_column_window(
                    &slot.column_widths,
                    scroll_metrics.offset_x,
                    scroll_metrics.offset_x + viewport.width(),
                );
            }
            let mut table_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Table),
            );
            let table_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = table_label {
                table_node.set_label(label);
            }
            table_node.set_scroll_x(scroll_metrics.offset_x);
            table_node.set_scroll_x_min(0.0);
            table_node.set_scroll_x_max(scroll_metrics.max_x);
            table_node.set_scroll_y(scroll_metrics.offset_y);
            table_node.set_scroll_y_min(0.0);
            table_node.set_scroll_y_max(scroll_metrics.max_y);
            table_node.add_action(AccessibilityAction::ScrollLeft);
            table_node.add_action(AccessibilityAction::ScrollRight);
            table_node.add_action(AccessibilityAction::ScrollUp);
            table_node.add_action(AccessibilityAction::ScrollDown);

            let origin_x = viewport.x0 - scroll_metrics.offset_x;
            let origin_y = viewport.y0 - scroll_metrics.offset_y;
            let mut x_offset = column_window.leading_offset;
            for column_index in column_window.start..column_window.end {
                let column = &columns[column_index];
                let width =
                    renderer.lazy.lazy_table_controller.slots[slot_index].column_widths[column_index];
                let header_cell = table_header_cell_rect(origin_x, origin_y, x_offset, width);
                let header_view = AnyView::new(column.label());
                let mut header_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ColumnHeader),
                );
                let default_label = renderer.accessibility_label_from_view(&header_view, env);
                let label = renderer.resolve_accessibility_label(env, default_label);
                if let Some(label) = label {
                    header_node.set_label(label);
                }
                header_node.add_action(AccessibilityAction::Focus);
                if let Some(header_node_id) = renderer.register_accessibility_child_node(
                    header_node,
                    transformed_rect(ctx.hit_transform, header_cell),
                    env,
                    None,
                ) {
                    table_node.push_child(header_node_id);
                }
                let rows = column.rows();
                for row_index in row_window.start..row_window.end {
                    let cell_rect =
                        table_data_cell_rect(origin_x, origin_y, x_offset, width, row_index);
                    if let Some(cell) = rows.get_view(row_index) {
                        let cell_view = AnyView::new(cell);
                        let mut cell_node = AccessibilityNode::new(
                            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Cell),
                        );
                        let default_label = renderer.accessibility_label_from_view(&cell_view, env);
                        let label = renderer.resolve_accessibility_label(env, default_label);
                        if let Some(label) = label {
                            cell_node.set_label(label);
                        }
                        cell_node.add_action(AccessibilityAction::Focus);
                        if let Some(cell_node_id) = renderer.register_accessibility_child_node(
                            cell_node,
                            transformed_rect(ctx.hit_transform, cell_rect),
                            env,
                            None,
                        ) {
                            table_node.push_child(cell_node_id);
                        }
                    }
                }
                x_offset += width;
            }

            let _ = renderer.register_accessibility_node(
                table_node,
                transformed_rect(ctx.hit_transform, viewport),
                env,
                Some(AccessibilityActionTarget::Scroll {
                    handle: handle.clone(),
                    axis: ScrollAxis::All,
                }),
            );
        }
    }
}

pub(crate) fn render_table(
    ctx: &mut WidgetRenderContext<'_>,
    table: Native<TableConfig>,
    env: &Environment,
) {
    let table = table.into_inner();
    let columns: Vec<TableColumn> = ctx.renderer_mut().read_signal(&table.columns);
    if columns.is_empty() {
        return;
    }
    let viewport = ctx.bounds;
    let handle = ctx.renderer_mut().take_pending_scroll_handle("render_table");
    let scroll_metrics = handle.metrics();
    let slot_index = ctx.renderer_mut().lazy.lazy_table_controller.bind();
    {
        let (slot, state) = ctx.renderer_mut().table_slot_and_state_mut(slot_index);
        refresh_table_slot_baseline(&columns, slot, state, env);
    }
    let row_window = {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        resolve_table_visible_rows(scroll_metrics.offset_y, viewport.height(), slot.max_rows)
    };
    let mut column_window = {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        resolve_visible_column_window(
            &slot.column_widths,
            scroll_metrics.offset_x,
            scroll_metrics.offset_x + viewport.width(),
        )
    };
    {
        let (slot, state) = ctx.renderer_mut().table_slot_and_state_mut(slot_index);
        update_table_slot_visible_cell_widths(&columns, slot, row_window, column_window, state, env);
    }
    {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        column_window = resolve_visible_column_window(
            &slot.column_widths,
            scroll_metrics.offset_x,
            scroll_metrics.offset_x + viewport.width(),
        );
    }
    let table_metrics = {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        table_metrics_from_slot(slot)
    };

    ctx.push_layer_rect(1.0, viewport);

    let origin_x = viewport.x0 - scroll_metrics.offset_x;
    let origin_y = viewport.y0 - scroll_metrics.offset_y;
    {
        let header_rect = vello::kurbo::Rect::new(
            origin_x,
            origin_y,
            origin_x + table_metrics.table_width,
            origin_y + TABLE_HEADER_HEIGHT,
        );
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_table_header_background(&mut draw, header_rect);
    }

    let mut x_offset = column_window.leading_offset;
    for column_index in column_window.start..column_window.end {
        let column = &columns[column_index];
        let width = table_metrics.column_widths[column_index];
        let header_cell = table_header_cell_rect(origin_x, origin_y, x_offset, width);
        let header_view = AnyView::new(column.label());
        ctx.dispatch_in_rect_without_accessibility(env, header_view, inset_rect(header_cell, 8.0, 6.0));

        let rows = column.rows();
        for row_index in row_window.start..row_window.end {
            let cell_rect = table_data_cell_rect(origin_x, origin_y, x_offset, width, row_index);
            if let Some(cell) = rows.get_view(row_index) {
                let cell_view = AnyView::new(cell);
                ctx.dispatch_in_rect_without_accessibility(
                    env,
                    cell_view,
                    inset_rect(cell_rect, 8.0, 6.0),
                );
            }
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_table_cell_border(&mut draw, cell_rect);
        }

        let separator_from = vello::kurbo::Point::new(origin_x + x_offset + width, origin_y);
        let separator_to = vello::kurbo::Point::new(
            origin_x + x_offset + width,
            origin_y + table_metrics.table_height,
        );
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_table_column_separator(&mut draw, separator_from, separator_to);
        x_offset += width;
    }

    ctx.pop_layer();

    let handle_for_input = handle.clone();
    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_scroll_target(
        transformed_rect(hit_transform, viewport),
        move |dx, dy, is_line_delta| handle_for_input.apply_scroll_delta(dx, dy, is_line_delta),
    );
    draw_scroll_indicators(ctx, env, viewport, scroll_metrics, ScrollAxis::All);
}
