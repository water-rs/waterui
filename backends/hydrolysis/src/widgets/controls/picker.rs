#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_picker_intrinsic, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    Toggled as AccessibilityToggled,
};
use nami::Binding;
use std::rc::Rc;
use waterui::ViewExt;
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::id::Id;
use waterui_core::layout::{HorizontalAlignment, Size as LayoutSize};
use waterui_form::picker::PickerItem;
use waterui_form::picker::{PickerConfig, PickerStyle};
use waterui_text::styled::StyledStr;

use crate::widgets::util::widget_theme;
use waterui_backend_core::widget::PickerMetrics;

impl HydroNativeView for Native<PickerConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_picker(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_picker_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let picker = view.as_inner();
            let items = renderer.read_signal(&picker.items);
            assert!(
                !(items.is_empty()),
                "hydrolysis picker requires at least one item"
            );
            match picker.style {
                PickerStyle::Automatic | PickerStyle::Menu => {
                    let selected = renderer.read_signal(&picker.selection);
                    let selected_index = items
                        .iter()
                        .position(|item| item.tag == selected)
                        .unwrap_or_else(|| {
                            panic!("hydrolysis picker selection is not present in picker items")
                        });
                    let mut option_labels = Vec::with_capacity(items.len());
                    let mut max_item_text_height: f64 = 0.0;
                    for item in &items {
                        let label = renderer
                            .read_resolved_text_styled(&item.content, env)
                            .to_plain();
                        let label_size = HydrolysisRenderer::measure_text_intrinsic_size(
                            renderer.state_mut(),
                            StyledStr::plain(label.clone()),
                            env,
                        );
                        max_item_text_height =
                            max_item_text_height.max(f64::from(label_size.height));
                        option_labels.push(label);
                    }
                    let selected_text = option_labels[selected_index].clone();
                    let mut node = AccessibilityNode::new(
                        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ComboBox),
                    );
                    let label = renderer
                        .resolve_accessibility_label(env, Some(selected_text.as_str().to_owned()));
                    if let Some(label) = label {
                        node.set_label(label);
                    }
                    node.set_value(selected_text.as_str().to_owned());
                    node.add_action(AccessibilityAction::Focus);
                    node.add_action(AccessibilityAction::Click);
                    let metrics = widget_theme(env).picker_metrics(PickerStyle::Menu);
                    let row_height = menu_picker_row_height(max_item_text_height, metrics);
                    let popup_rect =
                        menu_picker_popup_rect(ctx.bounds, row_height, items.len(), metrics);
                    for (index, item) in items.iter().enumerate() {
                        let mut option =
                            AccessibilityNode::new(renderer.resolve_accessibility_role(
                                env,
                                AccessibilityNodeRole::ListBoxOption,
                            ));
                        option.set_label(option_labels[index].as_str().to_owned());
                        let is_selected = item.tag == selected;
                        option.set_selected(is_selected);
                        option.set_toggled(AccessibilityToggled::from(is_selected));
                        option.add_action(AccessibilityAction::Focus);
                        option.add_action(AccessibilityAction::Click);
                        let option_bounds = transformed_rect(
                            ctx.hit_transform,
                            menu_picker_option_rect(popup_rect, row_height, index),
                        );
                        if let Some(option_id) = renderer.register_accessibility_child_node(
                            option,
                            option_bounds,
                            env,
                            Some(AccessibilityActionTarget::PickerSelect {
                                selection: picker.selection.clone(),
                                target: item.tag,
                            }),
                        ) {
                            node.push_child(option_id);
                        }
                    }
                    let ids = items.iter().map(|item| item.tag).collect::<Vec<_>>();
                    let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                    let _ = renderer.register_accessibility_node(
                        node,
                        bounds,
                        env,
                        Some(AccessibilityActionTarget::PickerCycle {
                            selection: picker.selection.clone(),
                            ids,
                        }),
                    );
                }
                PickerStyle::Radio | PickerStyle::Segmented => {
                    let mut group = AccessibilityNode::new(
                        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Group),
                    );
                    let group_label = renderer.resolve_accessibility_label(env, None);
                    if let Some(label) = group_label {
                        group.set_label(label);
                    }
                    let group_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                    let metrics = widget_theme(env).picker_metrics(picker.style);
                    let mut row_y = ctx.bounds.y0 + metrics.vertical_inset;
                    let selected = renderer.read_signal(&picker.selection);
                    let segment_width = ctx.bounds.width() / items.len() as f64;
                    for (index, item) in items.iter().enumerate() {
                        let label = renderer
                            .read_resolved_text_styled(&item.content, env)
                            .to_plain()
                            .to_string();
                        let label_size = HydrolysisRenderer::measure_text_intrinsic_size(
                            renderer.state_mut(),
                            StyledStr::plain(label.clone()),
                            env,
                        );
                        let row_rect = if picker.style == PickerStyle::Segmented {
                            let x0 = ctx.bounds.x0 + segment_width * index as f64;
                            vello::kurbo::Rect::new(
                                x0,
                                ctx.bounds.y0,
                                x0 + segment_width,
                                ctx.bounds.y1,
                            )
                        } else {
                            let row_height =
                                f64::from(label_size.height).max(metrics.radio_indicator_size);
                            let rect = vello::kurbo::Rect::new(
                                ctx.bounds.x0,
                                row_y,
                                ctx.bounds.x1,
                                (row_y + row_height).min(ctx.bounds.y1),
                            );
                            row_y = rect.y1 + metrics.radio_row_spacing;
                            rect
                        };
                        if row_rect.height() <= 0.0 {
                            break;
                        }
                        let mut option =
                            AccessibilityNode::new(renderer.resolve_accessibility_role(
                                env,
                                AccessibilityNodeRole::RadioButton,
                            ));
                        option.set_label(label);
                        let is_selected = item.tag == selected;
                        option.set_toggled(AccessibilityToggled::from(is_selected));
                        option.set_selected(is_selected);
                        option.add_action(AccessibilityAction::Focus);
                        option.add_action(AccessibilityAction::Click);
                        let row_bounds = transformed_rect(ctx.hit_transform, row_rect);
                        if let Some(child_id) = renderer.register_accessibility_child_node(
                            option,
                            row_bounds,
                            env,
                            Some(AccessibilityActionTarget::PickerSelect {
                                selection: picker.selection.clone(),
                                target: item.tag,
                            }),
                        ) {
                            group.push_child(child_id);
                        }
                    }
                    let _ = renderer.register_accessibility_node(group, group_bounds, env, None);
                }
                _ => panic!("hydrolysis PickerStyle variant is not implemented"),
            }
        }
    }
}

pub(crate) fn render_picker(
    ctx: &mut WidgetRenderContext<'_>,
    picker: Native<PickerConfig>,
    env: &Environment,
) {
    let renderer = ctx.renderer_mut();
    let picker = picker.into_inner();
    let items = renderer.read_signal(&picker.items);
    assert!(
        !(items.is_empty()),
        "hydrolysis picker requires at least one item"
    );
    match picker.style {
        PickerStyle::Automatic | PickerStyle::Menu => {
            render_menu_picker(ctx, picker.selection, items, env);
        }
        PickerStyle::Radio => {
            render_radio_picker(ctx, picker.selection, items, env);
        }
        PickerStyle::Segmented => {
            render_segmented_picker(ctx, picker.selection, items, env);
        }
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
    }
}

pub(crate) fn menu_picker_row_height(max_item_text_height: f64, metrics: PickerMetrics) -> f64 {
    metrics
        .popup_row_height
        .max(max_item_text_height + metrics.vertical_inset * 2.0)
}

pub(crate) fn menu_picker_popup_rect(
    field_bounds: vello::kurbo::Rect,
    row_height: f64,
    item_count: usize,
    metrics: PickerMetrics,
) -> vello::kurbo::Rect {
    let y0 = field_bounds.y1 + metrics.popup_top_spacing;
    let y1 = y0 + row_height * item_count as f64;
    vello::kurbo::Rect::new(field_bounds.x0, y0, field_bounds.x1, y1)
}

pub(crate) fn menu_picker_option_rect(
    popup_rect: vello::kurbo::Rect,
    row_height: f64,
    index: usize,
) -> vello::kurbo::Rect {
    let y0 = popup_rect.y0 + row_height * index as f64;
    vello::kurbo::Rect::new(popup_rect.x0, y0, popup_rect.x1, y0 + row_height)
}

pub(crate) fn render_menu_picker(
    ctx: &mut WidgetRenderContext<'_>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let selected = ctx.renderer_mut().read_signal(&selection);
    let menu_open = ctx.renderer_mut().bind_picker_menu_state();
    let is_menu_open = menu_open.get();
    let selected_index = items
        .iter()
        .position(|item| item.tag == selected)
        .unwrap_or_else(|| panic!("hydrolysis picker selection is not present in picker items"));
    let mut option_texts = Vec::with_capacity(items.len());
    let mut max_item_text_height: f64 = 0.0;
    for item in &items {
        let styled = ctx
            .renderer_mut()
            .read_resolved_text_styled(&item.content, env);
        let plain = styled.to_plain();
        let size = HydrolysisRenderer::measure_text_intrinsic_size(
            ctx.state_mut(),
            StyledStr::plain(plain.clone()),
            env,
        );
        max_item_text_height = max_item_text_height.max(f64::from(size.height));
        option_texts.push(plain);
    }
    let selected_text = option_texts[selected_index].clone();

    {
        let bounds = ctx.bounds;
        let hit_bounds = transformed_rect(ctx.hit_transform, bounds);
        let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
        {
            let mut draw = ctx.draw_context();
            theme.draw_input_field(&mut draw, bounds, interaction);
            theme.draw_picker_state_layer(&mut draw, bounds, interaction);
            theme.draw_picker_indicator(&mut draw, bounds);
        }
        let field_open_state = Rc::clone(&menu_open);
        ctx.renderer_mut().register_interactive_pointer_target(
            hit_bounds,
            press_slot,
            move |_renderer, _point, _env| {
                field_open_state.set(!field_open_state.get());
                true
            },
        );
    }

    let text_bounds = crate::widgets::util::inset_rect(
        ctx.bounds,
        metrics.horizontal_inset,
        metrics.vertical_inset,
    );
    let text_bounds = vello::kurbo::Rect::new(
        text_bounds.x0,
        text_bounds.y0,
        (text_bounds.x1 - metrics.indicator_space).max(text_bounds.x0),
        text_bounds.y1,
    );
    ctx.render_styled_text(
        StyledStr::plain(selected_text),
        HorizontalAlignment::Leading,
        env,
        text_bounds,
    );

    if !is_menu_open {
        return;
    }

    let row_height = menu_picker_row_height(max_item_text_height, metrics);
    let popup_rect = menu_picker_popup_rect(ctx.bounds, row_height, items.len(), metrics);
    {
        let mut draw = ctx.draw_context();
        theme.draw_picker_popup(&mut draw, popup_rect);
    }

    for (index, item) in items.into_iter().enumerate() {
        let row_rect = menu_picker_option_rect(popup_rect, row_height, index);
        let hit_rect = transformed_rect(ctx.hit_transform, row_rect);
        let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_rect, env);
        {
            let mut draw = ctx.draw_context();
            theme.draw_picker_popup_row_background(&mut draw, row_rect, item.tag == selected);
            theme.draw_picker_popup_row_state_layer(
                &mut draw,
                row_rect,
                item.tag == selected,
                interaction,
            );
        }
        if index + 1 < option_texts.len() {
            let separator = vello::kurbo::Rect::new(
                row_rect.x0 + 6.0,
                row_rect.y1 - 1.0,
                row_rect.x1 - 6.0,
                row_rect.y1,
            );
            let mut draw = ctx.draw_context();
            theme.draw_picker_separator(&mut draw, separator);
        }
        let row_text_rect = crate::widgets::util::inset_rect(
            row_rect,
            metrics.horizontal_inset,
            metrics.vertical_inset,
        );
        ctx.render_styled_text(
            StyledStr::plain(option_texts[index].clone()),
            HorizontalAlignment::Leading,
            env,
            row_text_rect,
        );

        let open_state = Rc::clone(&menu_open);
        let selection = selection.clone();
        let tag = item.tag;
        ctx.renderer_mut().register_interactive_pointer_target(
            hit_rect,
            press_slot,
            move |_renderer, _point, _env| {
                if selection.get() != tag {
                    selection.set(tag);
                }
                open_state.set(false);
                true
            },
        );
    }
}

pub(crate) fn render_radio_picker(
    ctx: &mut WidgetRenderContext<'_>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Radio);
    let selected = ctx.renderer_mut().read_signal(&selection);
    let bounds = ctx.bounds;
    let mut row_y = bounds.y0 + metrics.vertical_inset;
    for item in items {
        let label = ctx
            .renderer_mut()
            .read_resolved_text_styled(&item.content, env);
        let label_size =
            HydrolysisRenderer::measure_text_intrinsic_size(ctx.state_mut(), label.clone(), env);
        let row_height = f64::from(label_size.height).max(metrics.radio_indicator_size);
        let row_rect = vello::kurbo::Rect::new(
            bounds.x0,
            row_y,
            bounds.x1,
            (row_y + row_height).min(bounds.y1),
        );
        if row_rect.height() <= 0.0 {
            break;
        }
        row_y = row_rect.y1 + metrics.radio_row_spacing;

        let indicator_center = vello::kurbo::Point::new(
            row_rect.x0 + metrics.horizontal_inset + metrics.radio_indicator_size / 2.0,
            row_rect.y0 + row_rect.height() / 2.0,
        );
        let indicator_radius = metrics.radio_indicator_size / 2.0;
        let is_selected = item.tag == selected;
        let hit_rect = transformed_rect(ctx.hit_transform, row_rect);
        let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_rect, env);
        {
            let mut draw = ctx.draw_context();
            theme.draw_radio_state_layer(
                &mut draw,
                indicator_center,
                indicator_radius,
                is_selected,
                interaction,
            );
            theme.draw_radio_indicator(&mut draw, indicator_center, indicator_radius, is_selected);
        }

        let label_rect = vello::kurbo::Rect::new(
            indicator_center.x + indicator_radius + metrics.radio_label_spacing,
            row_rect.y0,
            row_rect.x1 - metrics.horizontal_inset,
            row_rect.y1,
        );
        ctx.render_styled_text(label, HorizontalAlignment::Leading, env, label_rect);

        let tag = item.tag;
        ctx.renderer_mut()
            .register_interactive_pointer_target(hit_rect, press_slot, {
                let selection = selection.clone();
                move |_renderer, _point, _env| {
                    if selection.get() == tag {
                        return false;
                    }
                    selection.set(tag);
                    true
                }
            });
    }
}

pub(crate) fn render_segmented_picker(
    ctx: &mut WidgetRenderContext<'_>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Segmented);
    let selected = ctx.renderer_mut().read_signal(&selection);
    let bounds = ctx.bounds;
    let item_count = items.len();
    let segment_width = bounds.width() / item_count as f64;

    for (index, item) in items.into_iter().enumerate() {
        let x0 = bounds.x0 + segment_width * index as f64;
        let segment_rect = vello::kurbo::Rect::new(x0, bounds.y0, x0 + segment_width, bounds.y1);
        let is_selected = item.tag == selected;
        let hit_rect = transformed_rect(ctx.hit_transform, segment_rect);
        let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_rect, env);
        {
            let mut draw = ctx.draw_context();
            theme.draw_segmented_picker_segment(
                &mut draw,
                segment_rect,
                is_selected,
                index == 0,
                index + 1 == item_count,
            );
            theme.draw_segmented_picker_state_layer(
                &mut draw,
                segment_rect,
                is_selected,
                interaction,
            );
        }

        let label = ctx
            .renderer_mut()
            .read_resolved_text_styled(&item.content, env);
        let label_size =
            HydrolysisRenderer::measure_text_intrinsic_size(ctx.state_mut(), label, env);
        let label_rect = segmented_label_rect(segment_rect, label_size, metrics);
        let label_view = match theme.segmented_picker_label_color(is_selected) {
            Some(color) => AnyView::new(item.content.foreground(color)),
            None => AnyView::new(item.content),
        };
        ctx.dispatch_in_rect_without_accessibility(env, label_view, label_rect);

        let tag = item.tag;
        ctx.renderer_mut()
            .register_interactive_pointer_target(hit_rect, press_slot, {
                let selection = selection.clone();
                move |_renderer, _point, _env| {
                    if selection.get() == tag {
                        return false;
                    }
                    selection.set(tag);
                    true
                }
            });
    }

    let mut draw = ctx.draw_context();
    theme.draw_segmented_picker_container(&mut draw, bounds, item_count);
}

fn segmented_label_rect(
    segment_rect: vello::kurbo::Rect,
    label_size: waterui_core::layout::Size,
    metrics: PickerMetrics,
) -> vello::kurbo::Rect {
    let max_width = (segment_rect.width() - metrics.horizontal_inset * 2.0).max(0.0);
    let width = f64::from(label_size.width).min(max_width);
    let height = f64::from(label_size.height).min(segment_rect.height());
    let x0 = segment_rect.x0 + (segment_rect.width() - width) * 0.5;
    let y0 = segment_rect.y0 + (segment_rect.height() - height) * 0.5;
    vello::kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}
