#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, PickerMenuEntry, PickerMenuRequest,
    RenderContext, WidgetRenderContext, measure_picker_intrinsic, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    Toggled as AccessibilityToggled,
};
use nami::{Binding, Signal};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use waterui_backend_core::widget::RadioIndicatorState;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::id::Id;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_form::picker::PickerItem;
use waterui_form::picker::{PickerConfig, PickerStyle};
use waterui_text::styled::StyledStr;

#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
use crate::renderer::local_interaction_state;
use crate::widgets::util::{widget_disabled, widget_theme};
use waterui_backend_core::widget::PickerMetrics;

impl HydroNativeView for Native<PickerConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_picker_intrinsic(view.as_inner(), state, env)
    }
}

/// State shared by the retained picker node and its popup callbacks.
pub(crate) struct PickerRenderState {
    pub(crate) config: PickerConfig,
    menu_open: Rc<Cell<bool>>,
}

impl PickerRenderState {
    pub(crate) fn new(config: PickerConfig) -> Self {
        Self {
            config,
            menu_open: Rc::new(Cell::new(false)),
        }
    }
}

/// Emits a picker's accessibility tree from its config. Shared by the dispatch
/// path ([`Native<PickerConfig>::accessibility`]) and the retained `Widget`-node
/// path so both produce the same a11y tree.
pub(crate) fn picker_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    picker: &PickerConfig,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let disabled = renderer.read_signal(&widget_disabled(env));
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
                    max_item_text_height = max_item_text_height.max(f64::from(label_size.height));
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
                if disabled {
                    node.set_disabled();
                } else {
                    node.add_action(AccessibilityAction::Click);
                }
                let metrics = widget_theme(env).picker_metrics(PickerStyle::Menu);
                let row_height = menu_picker_row_height(max_item_text_height, metrics);
                let popup_rect =
                    menu_picker_popup_rect(ctx.bounds, row_height, items.len(), metrics);
                for (index, item) in items.iter().enumerate() {
                    let mut option = AccessibilityNode::new(
                        renderer
                            .resolve_accessibility_role(env, AccessibilityNodeRole::ListBoxOption),
                    );
                    option.set_label(option_labels[index].as_str().to_owned());
                    let is_selected = item.tag == selected;
                    option.set_selected(is_selected);
                    option.set_toggled(AccessibilityToggled::from(is_selected));
                    option.add_action(AccessibilityAction::Focus);
                    if disabled {
                        option.set_disabled();
                    } else {
                        option.add_action(AccessibilityAction::Click);
                    }
                    let option_bounds = transformed_rect(
                        ctx.hit_transform,
                        menu_picker_option_rect(popup_rect, row_height, index),
                    );
                    if let Some(option_id) = renderer.register_accessibility_child_node_with_key(
                        i64::from(i32::from(item.tag)),
                        option,
                        option_bounds,
                        env,
                        (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
                            selection: picker.selection.clone(),
                            target: item.tag,
                        }),
                    ) {
                        node.push_child(option_id);
                    }
                }
                let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                let _ = renderer.register_accessibility_node(
                    node,
                    bounds,
                    env,
                    (!disabled).then(|| AccessibilityActionTarget::PointerPrimaryClick {
                        point: accessibility_activation_point(bounds),
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
                    let mut option = AccessibilityNode::new(
                        renderer
                            .resolve_accessibility_role(env, AccessibilityNodeRole::RadioButton),
                    );
                    option.set_label(label);
                    let is_selected = item.tag == selected;
                    option.set_toggled(AccessibilityToggled::from(is_selected));
                    option.set_selected(is_selected);
                    option.add_action(AccessibilityAction::Focus);
                    if disabled {
                        option.set_disabled();
                    } else {
                        option.add_action(AccessibilityAction::Click);
                    }
                    let row_bounds = transformed_rect(ctx.hit_transform, row_rect);
                    if let Some(child_id) = renderer.register_accessibility_child_node_with_key(
                        i64::from(i32::from(item.tag)),
                        option,
                        row_bounds,
                        env,
                        (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
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
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, picker, env);
    }
}

/// Measures a retained picker leaf from its config (mirrors
/// [`measure_picker_intrinsic`]).
pub(crate) fn measure_picker_node(
    state: &PickerRenderState,
    _proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(measure_picker_intrinsic(&state.config, hydro, env))
}

/// Renders a retained picker leaf every flush: emits a11y (unless hidden) then the
/// style-specific chrome + options, reading the items/selection signals each frame.
pub(crate) fn render_picker_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<PickerRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        picker_accessibility(ctx.renderer_mut(), render_ctx, &state.borrow().config, env);
    }
    render_picker_parts(ctx, state, env);
}

pub(crate) fn render_picker_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<PickerRenderState>>,
    env: &Environment,
) {
    // The items/selection signals are read through `read_signal` so a membership or
    // selection change schedules a frame and this persistent node re-renders.
    let (items_signal, selection, style) = {
        let picker = state.borrow();
        (
            picker.config.items.clone(),
            picker.config.selection.clone(),
            picker.config.style,
        )
    };
    let items = ctx.renderer_mut().read_signal(&items_signal);
    assert!(
        !(items.is_empty()),
        "hydrolysis picker requires at least one item"
    );
    match style {
        PickerStyle::Automatic | PickerStyle::Menu => {
            render_menu_picker(ctx, state, selection, items, env);
        }
        PickerStyle::Radio => {
            render_radio_picker(ctx, state, selection, items, env);
        }
        PickerStyle::Segmented => {
            render_segmented_picker(ctx, state, selection, items, env);
        }
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
    }
}

pub(crate) fn menu_picker_row_height(max_item_text_height: f64, metrics: PickerMetrics) -> f64 {
    metrics
        .popup_row_height
        .max(max_item_text_height + metrics.vertical_inset * 2.0)
}

#[cfg(feature = "accessibility")]
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

#[cfg(feature = "accessibility")]
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
    owner: &Rc<RefCell<PickerRenderState>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(owner, 0);
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let selected = ctx.renderer_mut().read_signal(&selection);
    // Register this node-owned open handle so an outside click can dismiss it; the
    // registry is Rc-pruned, so re-registering the same handle each frame is a no-op.
    let menu_open = Rc::clone(&owner.borrow().menu_open);
    ctx.renderer_mut().register_picker_menu(&menu_open);
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
    let row_height = menu_picker_row_height(max_item_text_height, metrics);
    let entries = items
        .iter()
        .zip(option_texts.iter())
        .map(|(item, label)| PickerMenuEntry {
            label: label.to_string(),
            tag: item.tag,
        })
        .collect::<Vec<_>>();

    {
        let bounds = ctx.bounds;
        let hit_bounds = transformed_rect(ctx.hit_transform, bounds);
        let (interaction, press_slot, _) =
            ctx.renderer_mut()
                .bind_interaction_target(interaction_key, hit_bounds, env);
        {
            let interaction = local_interaction_state(interaction, ctx.hit_transform);
            let mut draw = ctx.draw_context();
            theme.draw_input_field(&mut draw, bounds, interaction);
            theme.draw_picker_indicator(&mut draw, bounds);
            theme.draw_picker_state_layer(&mut draw, bounds, interaction);
        }
        let field_open_state = Rc::clone(&menu_open);
        let picker_selection = selection.clone();
        let menu_entries = entries.clone();
        let menu_origin =
            waterui_core::layout::Point::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
        let menu_width = hit_bounds.width();
        ctx.renderer_mut().register_interactive_pointer_target(
            hit_bounds,
            press_slot,
            move |renderer, _point, env| {
                if field_open_state.get() {
                    renderer.dismiss_active_popup_menu();
                    false
                } else {
                    renderer.show_picker_menu(
                        PickerMenuRequest {
                            entries: menu_entries.clone(),
                            selection: picker_selection.clone(),
                            open: Rc::clone(&field_open_state),
                            origin: menu_origin,
                            width: menu_width,
                            row_height,
                            selected,
                        },
                        env,
                    )
                }
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
}

pub(crate) fn render_radio_picker(
    ctx: &mut WidgetRenderContext<'_>,
    owner: &Rc<RefCell<PickerRenderState>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Radio);
    let radio_motion = theme.radio_selection_motion();
    let selection_identity = selection.identity();
    let selected = ctx.renderer_mut().read_signal(&selection);
    let bounds = ctx.bounds;
    let mut row_y = bounds.y0 + metrics.vertical_inset;
    for (row_index, item) in items.into_iter().enumerate() {
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
        let radio_indicator_state = if let Some(identity) = selection_identity {
            ctx.renderer_mut().sample_radio_indicator_state(
                AnimationKey::radio_indicator_with_discriminator(identity, row_index),
                is_selected,
                &radio_motion,
            )
        } else {
            let selected_progress = if is_selected { 1.0 } else { 0.0 };
            RadioIndicatorState {
                selected: is_selected,
                outer_selected_progress: selected_progress,
                inner_scale: 1.0,
                inner_opacity: selected_progress,
            }
        };
        let hit_rect = transformed_rect(ctx.hit_transform, row_rect);
        let discriminator = i32::from(item.tag) as u32 as usize;
        let interaction_key = crate::renderer::InteractionKey::for_rc(owner, discriminator);
        let (interaction, press_slot, _) =
            ctx.renderer_mut()
                .bind_interaction_target(interaction_key, hit_rect, env);
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        {
            let mut draw = ctx.draw_context();
            theme.draw_radio_indicator(
                &mut draw,
                indicator_center,
                indicator_radius,
                radio_indicator_state,
            );
            theme.draw_radio_state_layer(
                &mut draw,
                indicator_center,
                indicator_radius,
                is_selected,
                interaction,
            );
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
    owner: &Rc<RefCell<PickerRenderState>>,
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
        let discriminator = i32::from(item.tag) as u32 as usize;
        let interaction_key = crate::renderer::InteractionKey::for_rc(owner, discriminator);
        let (interaction, press_slot, _) =
            ctx.renderer_mut()
                .bind_interaction_target(interaction_key, hit_rect, env);
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
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
        // Render the segment label directly as styled text (no dispatch), mirroring
        // the radio/menu picker styles. The item's resolved `StyledStr` carries its
        // own per-chunk styling; the selected/unselected foreground override is
        // applied to all chunks so the live selection signal drives the color each
        // frame without rebuilding a view.
        let label = ctx
            .renderer_mut()
            .read_resolved_text_styled(&item.content, env);
        let label_size =
            HydrolysisRenderer::measure_text_intrinsic_size(ctx.state_mut(), label.clone(), env);
        let label_rect = segmented_label_rect(segment_rect, label_size, metrics);
        let styled = match theme.segmented_picker_label_color(is_selected) {
            Some(color) => label.foreground(color),
            None => label,
        };
        ctx.render_styled_text(styled, HorizontalAlignment::Leading, env, label_rect);

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
use crate::animation::AnimationKey;
