use std::rc::Rc;

#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    local_state_child_env, materialize_list_item, materialize_list_row, measure_list_intrinsic,
    measure_list_item_row_height, measure_view_intrinsic, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use waterui::component::list::{ListConfig, Move};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::views::Views;
use waterui_core::{Environment, Native};
use waterui_layout::scroll::Axis as ScrollAxis;

use crate::renderer::lazy::{resolve_visible_index_window, sum_cached_or_estimated};
use crate::widgets::{draw_scroll_indicators, widget_theme};

impl HydroNativeView for Native<ListConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_list(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_list_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        let list = view.as_inner();
        let row_count_signal = list.contents.len();
        let row_count = renderer.read_signal(&row_count_signal);
        let slot_index = {
            let index = renderer.lazy.lazy_list_controller.bind();
            renderer.lazy.lazy_list_controller.slots[index].prepare_len(row_count);
            index
        };
        let viewport = ctx.bounds;
        let list_metrics = crate::widgets::widget_theme(env).list_metrics();
        let content_height = sum_cached_or_estimated(
            &renderer.lazy.lazy_list_controller.slots[slot_index].row_extents,
            list_metrics.one_line_row_height,
        )
        .max(viewport.height());
        let handle = renderer.bind_scroll_handle(
            ScrollAxis::Vertical,
            viewport.width(),
            viewport.height(),
            viewport.width(),
            content_height,
        );
        #[cfg(feature = "accessibility")]
        {
            let metrics = handle.metrics();
            let window = resolve_visible_index_window(
                row_count,
                metrics.offset_y,
                metrics.offset_y + viewport.height(),
                |index| {
                    let cached_extent =
                        renderer.lazy.lazy_list_controller.slots[slot_index].row_extents[index];
                    if let Some(extent) = cached_extent {
                        return extent;
                    }
                    let (_item, extent) =
                        materialize_list_row(&list.contents, index, renderer.state_mut(), env);
                    renderer.lazy.lazy_list_controller.slots[slot_index].row_extents[index] =
                        Some(extent);
                    extent
                },
            );
            let mut list_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::List),
            );
            let list_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = list_label {
                list_node.set_label(label);
            }
            list_node.set_scroll_y(metrics.offset_y);
            list_node.set_scroll_y_min(0.0);
            list_node.set_scroll_y_max(metrics.max_y);
            list_node.add_action(AccessibilityAction::ScrollUp);
            list_node.add_action(AccessibilityAction::ScrollDown);
            let mut y = viewport.y0 - metrics.offset_y + window.leading_offset;
            for index in window.start..window.end {
                let row_env = local_state_child_env(env, index);
                let item = materialize_list_item(&list.contents, index, &row_env);
                let row_height = {
                    let cached_extent =
                        renderer.lazy.lazy_list_controller.slots[slot_index].row_extents[index];
                    if let Some(extent) = cached_extent {
                        extent
                    } else {
                        let extent =
                            measure_list_item_row_height(&item, renderer.state_mut(), &row_env);
                        renderer.lazy.lazy_list_controller.slots[slot_index].row_extents[index] =
                            Some(extent);
                        extent
                    }
                };
                let row_rect = vello::kurbo::Rect::new(viewport.x0, y, viewport.x1, y + row_height);
                y += row_height;
                if row_rect.y1 <= viewport.y0 || row_rect.y0 >= viewport.y1 {
                    continue;
                }
                let mut row_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ListItem),
                );
                let default_label = renderer.accessibility_label_from_view(&item.content, &row_env);
                let label = renderer.resolve_accessibility_label(&row_env, default_label);
                if let Some(label) = label {
                    row_node.set_label(label);
                }
                row_node.add_action(AccessibilityAction::Focus);
                if let Some(row_node_id) = renderer.register_accessibility_child_node(
                    row_node,
                    transformed_rect(ctx.hit_transform, row_rect),
                    &row_env,
                    None,
                ) {
                    list_node.push_child(row_node_id);
                }
            }
            let _ = renderer.register_accessibility_node(
                list_node,
                transformed_rect(ctx.hit_transform, viewport),
                env,
                Some(AccessibilityActionTarget::Scroll {
                    handle: handle.clone(),
                    axis: ScrollAxis::Vertical,
                }),
            );
        }
    }
}

pub(crate) fn render_list(
    ctx: &mut WidgetRenderContext<'_>,
    list: Native<ListConfig>,
    env: &Environment,
) {
    let list = list.into_inner();
    let editing = ctx.renderer_mut().read_signal(&list.editing);
    let row_count_signal = list.contents.len();
    let row_count = ctx.renderer_mut().read_signal(&row_count_signal);
    let contents = list.contents.clone();
    let slot_index = {
        let renderer = ctx.renderer_mut();
        let index = renderer.lazy.lazy_list_controller.bind();
        renderer.lazy.lazy_list_controller.slots[index].prepare_len(row_count);
        index
    };

    let viewport = ctx.bounds;
    let list_metrics = widget_theme(env).list_metrics();
    let handle = ctx.renderer_mut().take_pending_scroll_handle("render_list");
    let metrics = handle.metrics();
    let needs_viewport_clip = metrics.max_y > 0.0;
    if needs_viewport_clip {
        ctx.push_layer_rect(1.0, viewport);
    }

    let window = resolve_visible_index_window(
        row_count,
        metrics.offset_y,
        metrics.offset_y + viewport.height(),
        |index| {
            let cached_extent = {
                ctx.renderer_mut().lazy.lazy_list_controller.slots[slot_index].row_extents[index]
            };
            if let Some(extent) = cached_extent {
                return extent;
            }
            let (_item, extent) = materialize_list_row(&contents, index, ctx.state_mut(), env);
            ctx.renderer_mut().lazy.lazy_list_controller.slots[slot_index].row_extents[index] =
                Some(extent);
            extent
        },
    );
    let delete_action = list.on_delete.map(Rc::new);
    let move_action = list.on_move.map(Rc::new);
    let total_rows = row_count;
    let mut y = viewport.y0 - metrics.offset_y + window.leading_offset;
    for index in window.start..window.end {
        let row_env = local_state_child_env(env, index);
        let item = materialize_list_item(&contents, index, &row_env);
        let row_height = {
            let cached_extent = {
                ctx.renderer_mut().lazy.lazy_list_controller.slots[slot_index].row_extents[index]
            };
            if let Some(extent) = cached_extent {
                extent
            } else {
                let extent = measure_list_item_row_height(&item, ctx.state_mut(), &row_env);
                ctx.renderer_mut().lazy.lazy_list_controller.slots[slot_index].row_extents[index] =
                    Some(extent);
                extent
            }
        };
        let row_rect = vello::kurbo::Rect::new(viewport.x0, y, viewport.x1, y + row_height);
        y += row_height;
        if row_rect.y1 <= viewport.y0 || row_rect.y0 >= viewport.y1 {
            continue;
        }
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_list_row_background(&mut draw, row_rect, index % 2 == 1);
        }

        let deletable = ctx.renderer_mut().read_signal(&item.deletable);
        let content_size = measure_view_intrinsic(&item.content, ctx.state_mut(), &row_env);
        let stretches_horizontally = matches!(
            crate::renderer::effective_stretch_axis(&item.content),
            waterui_core::layout::StretchAxis::Horizontal
                | waterui_core::layout::StretchAxis::Both
                | waterui_core::layout::StretchAxis::MainAxis
        );
        let mut content_rect =
            list_content_rect(row_rect, list_metrics, content_size, stretches_horizontally);
        let mut trailing_x = row_rect.x1 - 8.0;

        if let (true, Some(move_action)) = (editing, move_action.as_ref()) {
            let control_width = list_metrics.move_control_width;
            let vertical_inset = list_metrics.trailing_control_vertical_inset;
            let control_height = (row_height - vertical_inset * 2.0).max(vertical_inset * 2.0);
            let control_rect = vello::kurbo::Rect::new(
                trailing_x - control_width,
                row_rect.y0 + vertical_inset,
                trailing_x,
                row_rect.y0 + vertical_inset + control_height,
            );
            trailing_x -= control_width + list_metrics.trailing_control_spacing;
            let up_rect = vello::kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0,
                control_rect.x1,
                control_rect.y0 + control_rect.height() / 2.0,
            );
            let down_rect = vello::kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0 + control_rect.height() / 2.0,
                control_rect.x1,
                control_rect.y1,
            );
            let up_interaction = (index > 0).then(|| {
                let hit_bounds = transformed_rect(ctx.hit_transform, up_rect);
                let (_, slot, handles) = ctx
                    .renderer_mut()
                    .bind_interaction_target(hit_bounds, &row_env);
                (hit_bounds, handles, slot)
            });
            let down_interaction = (index + 1 < total_rows).then(|| {
                let hit_bounds = transformed_rect(ctx.hit_transform, down_rect);
                let (_, slot, handles) = ctx
                    .renderer_mut()
                    .bind_interaction_target(hit_bounds, &row_env);
                (hit_bounds, handles, slot)
            });
            {
                let theme = widget_theme(env);
                let mut draw = ctx.draw_context();
                theme.draw_list_move_control(&mut draw, control_rect);
            }
            let theme = widget_theme(env);
            let render_ctx = ctx.render_context();
            if let Some((_, handles, _)) = &up_interaction {
                ctx.renderer_mut().capture_state_layers(
                    render_ctx,
                    handles,
                    up_rect,
                    false,
                    &|draw, state| {
                        theme.draw_list_move_control_state_layer(draw, up_rect, state);
                    },
                );
            }
            if let Some((_, handles, _)) = &down_interaction {
                ctx.renderer_mut().capture_state_layers(
                    render_ctx,
                    handles,
                    down_rect,
                    false,
                    &|draw, state| {
                        theme.draw_list_move_control_state_layer(draw, down_rect, state);
                    },
                );
            }

            if let Some((hit_bounds, _, press_slot)) = up_interaction {
                let action = Rc::clone(move_action);
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, env| {
                        (action.as_ref())(env, Move::new(index, index - 1));
                        true
                    },
                );
            }
            if let Some((hit_bounds, _, press_slot)) = down_interaction {
                let action = Rc::clone(move_action);
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, env| {
                        (action.as_ref())(env, Move::new(index, index + 1));
                        true
                    },
                );
            }
        }

        if let (true, true, Some(delete_action)) = (editing, deletable, delete_action.as_ref()) {
            let delete_rect = vello::kurbo::Rect::new(
                trailing_x - list_metrics.delete_control_width,
                row_rect.y0 + list_metrics.trailing_control_vertical_inset,
                trailing_x,
                row_rect.y1 - list_metrics.trailing_control_vertical_inset,
            );
            trailing_x = delete_rect.x0 - list_metrics.trailing_control_spacing;
            let delete_hit_bounds = transformed_rect(ctx.hit_transform, delete_rect);
            let (_, delete_press_slot, delete_handles) = ctx
                .renderer_mut()
                .bind_interaction_target(delete_hit_bounds, &row_env);
            {
                let theme = widget_theme(env);
                let mut draw = ctx.draw_context();
                theme.draw_list_delete_control(&mut draw, delete_rect);
            }
            let theme = widget_theme(env);
            let render_ctx = ctx.render_context();
            ctx.renderer_mut().capture_state_layers(
                render_ctx,
                &delete_handles,
                delete_rect,
                false,
                &|draw, state| {
                    theme.draw_list_delete_control_state_layer(draw, delete_rect, state);
                },
            );
            let action = Rc::clone(delete_action);
            ctx.renderer_mut().register_interactive_pointer_target(
                delete_hit_bounds,
                delete_press_slot,
                move |_renderer, _point, env| {
                    (action.as_ref())(env, index);
                    true
                },
            );
        }

        content_rect.x1 = content_rect.x1.min(trailing_x);
        if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
            ctx.dispatch_in_rect_without_accessibility(&row_env, item.content, content_rect);
        }

        {
            let separator = vello::kurbo::Rect::new(
                row_rect.x0 + list_metrics.divider_leading_inset,
                row_rect.y1 - 1.0,
                row_rect.x1 - list_metrics.divider_trailing_inset,
                row_rect.y1,
            );
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_list_separator(&mut draw, separator);
        }
    }

    if needs_viewport_clip {
        ctx.pop_layer();
    }

    let handle_for_input = handle.clone();
    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_scroll_target(
        transformed_rect(hit_transform, viewport),
        move |dx, dy, is_line_delta| handle_for_input.apply_scroll_delta(dx, dy, is_line_delta),
    );
    draw_scroll_indicators(ctx, env, viewport, metrics, ScrollAxis::Vertical);
}

fn list_content_rect(
    row_rect: vello::kurbo::Rect,
    metrics: waterui_backend_core::widget::ListMetrics,
    content_size: waterui_core::layout::Size,
    stretches_horizontally: bool,
) -> vello::kurbo::Rect {
    let x0 = row_rect.x0 + metrics.horizontal_inset;
    let max_x1 = row_rect.x1 - metrics.horizontal_inset;
    // List rows are leading-aligned: non-stretching content is placed at its
    // intrinsic width from the leading inset (Material list item layout),
    // while stretching content fills the available row width.
    let x1 = if stretches_horizontally {
        max_x1
    } else {
        (x0 + f64::from(content_size.width)).min(max_x1)
    };
    let available_height = (row_rect.height() - metrics.vertical_inset * 2.0).max(0.0);
    let height = f64::from(content_size.height).min(available_height);
    let y0 = row_rect.y0 + (row_rect.height() - height) * 0.5;
    vello::kurbo::Rect::new(x0, y0, x1, y0 + height)
}
