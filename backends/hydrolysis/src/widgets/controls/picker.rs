#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, PickerMenuEntry, PickerMenuRequest,
    RenderContext, RetainedSubview, WidgetRenderContext, measure_picker_intrinsic,
    measure_picker_intrinsic_with_label_size, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    Toggled as AccessibilityToggled,
};
use nami::{Binding, Signal};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui_backend_core::widget::RadioIndicatorState;
use waterui_controls::label::Label;
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::id::Id;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_form::picker::PickerItem;
use waterui_form::picker::{PickerConfig, PickerStyle};
use waterui_text::styled::StyledStr;

use crate::renderer::local_interaction_state;
use crate::widgets::util::label_beside_control_bounds;
#[cfg(feature = "accessibility")]
use crate::widgets::util::widget_disabled;
use waterui_backend_core::widget::PickerMetrics;

impl HydroNativeView for Native<PickerConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_picker_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// State shared by the retained picker node and its popup callbacks: the
/// cloneable [`PickerConfig`] drives the field chrome + accessibility, and its
/// label is held as a [`RetainedSubview`] built once and re-flushed each frame
/// so reactive label content stays live.
pub struct PickerRenderState {
    pub(crate) config: PickerConfig,
    label_view: RetainedSubview,
    menu_open: Rc<Cell<bool>>,
}

impl PickerRenderState {
    pub(crate) fn from_config(config: PickerConfig) -> Self {
        Self {
            label_view: RetainedSubview::new(menu_picker_label_view(&config.label)),
            config,
            menu_open: Rc::new(Cell::new(false)),
        }
    }

    /// Eagerly build the label sub-view (the measure path has only
    /// `&mut HydroState`, no renderer, so it must be built before then).
    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        self.label_view.ensure_built(renderer, env);
    }
}

/// The label view a menu picker draws inside its field: the picker's own label
/// in the platform's field-label chrome — the `Caption` font slot and the
/// `MutedForeground` colour token the theme provides for chrome labels, never a
/// named size or colour. A hidden label keeps its zero-size body: it draws
/// nothing and takes no space. A custom-content label owns its own styling, so
/// only the colour token is applied to it.
pub fn menu_picker_label_view(label: &Label) -> AnyView {
    let styled = if label.has_custom_content() {
        label.clone()
    } else {
        label.clone().font(waterui_text::font::Caption)
    };
    AnyView::new(styled.muted())
}

/// Emits a picker's accessibility tree from its retained state. The rendered
/// `Widget`-node path passes its [`RenderContext`] and theme (option rows get
/// bounds); the semantic emission walk passes `None` for both — every action
/// target is semantic (selection bindings, direct menu activation), so no
/// geometry or style is needed.
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn picker_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::engine::WidgetTheme>>,
    state: &Rc<RefCell<PickerRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let owner = state;
        let state = state.borrow();
        let picker = &state.config;
        let menu_open = Rc::clone(&state.menu_open);
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
                    crate::renderer::SemanticCore::resolve_accessibility_role(
                        env,
                        AccessibilityNodeRole::ComboBox,
                    ),
                );
                let default_label = crate::renderer::SemanticCore::accessibility_label_from_label(
                    &picker.label,
                    env,
                );
                let label = renderer.resolve_accessibility_label(env, default_label);
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
                // Option row bounds exist only in the rendered runtime, where
                // the popup rect is computable from the trigger's bounds and
                // the theme's picker metrics.
                let option_geometry = ctx.as_ref().zip(theme).map(|(ctx, theme)| {
                    let metrics = theme.picker_metrics(PickerStyle::Menu);
                    let row_height = menu_picker_row_height(max_item_text_height, metrics);
                    let popup_rect =
                        menu_picker_popup_rect(ctx.bounds, row_height, items.len(), metrics);
                    (ctx, row_height, popup_rect)
                });
                for (index, item) in items.iter().enumerate() {
                    let mut option = AccessibilityNode::new(
                        crate::renderer::SemanticCore::resolve_accessibility_role(
                            env,
                            AccessibilityNodeRole::ListBoxOption,
                        ),
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
                    let option_target =
                        (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
                            selection: picker.selection.clone(),
                            target: item.tag,
                        });
                    let option_id = match option_geometry {
                        Some((ctx, row_height, popup_rect)) => {
                            let option_bounds = transformed_rect(
                                ctx.hit_transform,
                                menu_picker_option_rect(popup_rect, row_height, index),
                            );
                            renderer.register_accessibility_child_node_with_key(
                                i64::from(i32::from(item.tag)),
                                option,
                                option_bounds,
                                env,
                                option_target,
                            )
                        }
                        None => renderer.register_accessibility_child_node_with_key_semantic(
                            i64::from(i32::from(item.tag)),
                            option,
                            env,
                            option_target,
                        ),
                    };
                    if let Some(option_id) = option_id {
                        node.push_child(option_id);
                    }
                }
                // Direct activation: `Click` does exactly what the field's
                // pointer target does — toggle the popup. The rendered runtime
                // shows the real window under the trigger; the semantic runtime
                // has no window manager, so it records the open state and
                // active menu group — placement is presentation detail only.
                let action_target = (!disabled).then(|| {
                    let menu_entries: Vec<PickerMenuEntry> = items
                        .iter()
                        .zip(option_labels.iter())
                        .map(|(item, label)| PickerMenuEntry {
                            label: label.to_string(),
                            tag: item.tag,
                        })
                        .collect();
                    let selection = picker.selection.clone();
                    let open = Rc::clone(&menu_open);
                    let request = ctx.as_ref().zip(theme).map(|(ctx, theme)| {
                        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                        let metrics = theme.picker_metrics(PickerStyle::Menu);
                        (
                            waterui_core::layout::Point::new(
                                crate::num_cast::f64_as_f32(bounds.x0),
                                crate::num_cast::f64_as_f32(bounds.y1),
                            ),
                            bounds.width(),
                            menu_picker_row_height(max_item_text_height, metrics),
                            metrics,
                        )
                    });
                    // The popup opens in the picker's environment layered over
                    // the dispatch's (water-rs/hydrolysis#140).
                    let picker_env = env.clone();
                    AccessibilityActionTarget::Activate {
                        action: Rc::new(RefCell::new(
                            move |renderer: &mut crate::renderer::SemanticCore,
                                  env: &Environment| {
                                // `Click` toggles the popup — opening or
                                // dismissing is a handled activation either
                                // way, and so is a menu with nothing to show.
                                if open.get() {
                                    renderer.dismiss_active_popup_menu();
                                } else {
                                    let env = picker_env.layered_on(env);
                                    match request {
                                        Some((origin, width, row_height, metrics)) => {
                                            renderer.show_picker_menu(
                                                PickerMenuRequest {
                                                    entries: menu_entries.clone(),
                                                    selection: selection.clone(),
                                                    open: Rc::clone(&open),
                                                    origin,
                                                    width,
                                                    row_height,
                                                    selected,
                                                },
                                                metrics,
                                                &env,
                                            );
                                        }
                                        None => {
                                            renderer.activate_picker_menu(
                                                menu_entries.clone(),
                                                selection.clone(),
                                                &open,
                                                &env,
                                            );
                                        }
                                    }
                                }
                                true
                            },
                        )),
                    }
                });
                let trigger_id = match ctx {
                    Some(ctx) => {
                        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                        renderer.register_accessibility_node(node, bounds, env, action_target)
                    }
                    None => renderer.register_accessibility_node_semantic(node, env, action_target),
                };
                if let Some(trigger_id) = trigger_id {
                    renderer.register_accessibility_focus_link(
                        &crate::renderer::InteractionKey::for_rc(owner, 0),
                        trigger_id,
                    );
                }
            }
            PickerStyle::Radio | PickerStyle::Segmented => {
                let mut group = AccessibilityNode::new(
                    crate::renderer::SemanticCore::resolve_accessibility_role(
                        env,
                        AccessibilityNodeRole::Group,
                    ),
                );
                let default_label = crate::renderer::SemanticCore::accessibility_label_from_label(
                    &picker.label,
                    env,
                );
                let group_label = renderer.resolve_accessibility_label(env, default_label);
                if let Some(label) = group_label {
                    group.set_label(label);
                }
                let geometry = ctx
                    .as_ref()
                    .zip(theme)
                    .map(|(ctx, theme)| (ctx, theme.picker_metrics(picker.style)));
                // The group heading offsets the option rows exactly as the
                // render path lays them out, so emitted row bounds overlay the
                // drawn rows — the same presence rule: measured height > 0.
                let group_label_height = theme.map_or(0.0, |theme| {
                    f64::from(
                        state
                            .label_view
                            .measure_built(renderer.state_mut(), env, theme)
                            .height,
                    )
                });
                let selected = renderer.read_signal(&picker.selection);
                let mut row_y = geometry.map(|(ctx, metrics)| {
                    ctx.bounds.y0
                        + metrics.vertical_inset
                        + if group_label_height > 0.0 {
                            group_label_height + metrics.label_spacing
                        } else {
                            0.0
                        }
                });
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
                    let row_rect = geometry.map(|(ctx, metrics)| {
                        if picker.style == PickerStyle::Segmented {
                            let segment_width =
                                ctx.bounds.width() / crate::num_cast::usize_as_f64(items.len());
                            let x0 = segment_width
                                .mul_add(crate::num_cast::usize_as_f64(index), ctx.bounds.x0);
                            let top = ctx.bounds.y0
                                + if group_label_height > 0.0 {
                                    group_label_height + metrics.label_spacing
                                } else {
                                    0.0
                                };
                            kurbo::Rect::new(x0, top, x0 + segment_width, ctx.bounds.y1)
                        } else {
                            let y = row_y.unwrap_or(ctx.bounds.y0);
                            let row_height =
                                f64::from(label_size.height).max(metrics.radio_indicator_size);
                            let rect = kurbo::Rect::new(
                                ctx.bounds.x0,
                                y,
                                ctx.bounds.x1,
                                (y + row_height).min(ctx.bounds.y1),
                            );
                            row_y = Some(rect.y1 + metrics.radio_row_spacing);
                            rect
                        }
                    });
                    if let Some(rect) = row_rect
                        && rect.height() <= 0.0
                    {
                        break;
                    }
                    let mut option = AccessibilityNode::new(
                        crate::renderer::SemanticCore::resolve_accessibility_role(
                            env,
                            AccessibilityNodeRole::RadioButton,
                        ),
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
                    let option_target =
                        (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
                            selection: picker.selection.clone(),
                            target: item.tag,
                        });
                    let child_id = match row_rect {
                        Some(row_rect) => {
                            let ctx = ctx.expect("rendered picker emits option bounds");
                            let row_bounds = transformed_rect(ctx.hit_transform, row_rect);
                            renderer.register_accessibility_child_node_with_key(
                                i64::from(i32::from(item.tag)),
                                option,
                                row_bounds,
                                env,
                                option_target,
                            )
                        }
                        None => renderer.register_accessibility_child_node_with_key_semantic(
                            i64::from(i32::from(item.tag)),
                            option,
                            env,
                            option_target,
                        ),
                    };
                    if let Some(child_id) = child_id {
                        group.push_child(child_id);
                        let discriminator =
                            crate::num_cast::i32_as_u32(i32::from(item.tag)) as usize;
                        renderer.register_accessibility_focus_link(
                            &crate::renderer::InteractionKey::for_rc(owner, discriminator),
                            child_id,
                        );
                    }
                }
                match ctx {
                    Some(ctx) => {
                        let group_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                        let _ =
                            renderer.register_accessibility_node(group, group_bounds, env, None);
                    }
                    None => {
                        let _ = renderer.register_accessibility_node_semantic(group, env, None);
                    }
                }
            }
            _ => panic!("hydrolysis PickerStyle variant is not implemented"),
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, theme, state, env);
    }
}

/// Measures a retained picker leaf from its [`PickerRenderState`], reading the
/// label size from its already-built [`RetainedSubview`] so layout and the
/// in-field label render agree (mirrors [`measure_picker_intrinsic`]).
pub fn measure_picker_node(
    state: &PickerRenderState,
    _proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let label_size = state.label_view.measure_built(hydro, env, theme);
    ViewDimensions::new(measure_picker_intrinsic_with_label_size(
        &state.config,
        label_size,
        hydro,
        env,
        theme,
    ))
}

/// Renders a retained picker leaf every flush: emits a11y (unless hidden) then the
/// style-specific chrome + options, reading the items/selection signals each frame.
pub fn render_picker_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<PickerRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let theme = ctx.theme();
        let render_ctx = ctx.render_context();
        picker_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            state,
            env,
        );
    }
    render_picker_parts(ctx, state, env);
}

pub fn render_picker_parts(
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

pub const fn menu_picker_row_height(max_item_text_height: f64, metrics: PickerMetrics) -> f64 {
    metrics
        .popup_row_height
        .max(metrics.vertical_inset.mul_add(2.0, max_item_text_height))
}

#[cfg(feature = "accessibility")]
pub fn menu_picker_popup_rect(
    field_bounds: kurbo::Rect,
    row_height: f64,
    item_count: usize,
    metrics: PickerMetrics,
) -> kurbo::Rect {
    let y0 = field_bounds.y1 + metrics.popup_top_spacing;
    let y1 = row_height.mul_add(crate::num_cast::usize_as_f64(item_count), y0);
    kurbo::Rect::new(field_bounds.x0, y0, field_bounds.x1, y1)
}

#[cfg(feature = "accessibility")]
pub fn menu_picker_option_rect(
    popup_rect: kurbo::Rect,
    row_height: f64,
    index: usize,
) -> kurbo::Rect {
    let y0 = row_height.mul_add(crate::num_cast::usize_as_f64(index), popup_rect.y0);
    kurbo::Rect::new(popup_rect.x0, y0, popup_rect.x1, y0 + row_height)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
pub fn render_menu_picker(
    ctx: &mut WidgetRenderContext<'_>,
    owner: &Rc<RefCell<PickerRenderState>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(owner, 0);
    let theme = ctx.theme();
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
            ctx.draw_context(|draw| {
                theme.draw_input_field(&mut *draw, bounds, interaction);
                theme.draw_picker_indicator(&mut *draw, bounds);
                theme.draw_picker_state_layer(&mut *draw, bounds, interaction);
            });
        }
        let field_open_state = Rc::clone(&menu_open);
        let picker_selection = selection;
        let menu_entries = entries;
        let menu_origin = waterui_core::layout::Point::new(
            crate::num_cast::f64_as_f32(hit_bounds.x0),
            crate::num_cast::f64_as_f32(hit_bounds.y1),
        );
        let menu_width = hit_bounds.width();
        // The popup opens in the picker's environment layered over the
        // dispatch's (water-rs/hydrolysis#140).
        let picker_env = env.clone();
        ctx.renderer_mut().register_interactive_pointer_target(
            hit_bounds,
            press_slot,
            move |renderer, _point, env| {
                if field_open_state.get() {
                    renderer.dismiss_active_popup_menu();
                    false
                } else {
                    let env = picker_env.layered_on(env);
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
                        metrics,
                        &env,
                    )
                }
            },
        );
    }

    let label_size = owner
        .borrow_mut()
        .label_view
        .measure_intrinsic(ctx.renderer_mut(), env);
    // One rule decides the label's presence in both measure and render: it is
    // drawn iff its measured height is nonzero.
    let (label_bounds, text_bounds) =
        menu_picker_content_rects(ctx.bounds, metrics, f64::from(label_size.height));
    if let Some(label_bounds) = label_bounds {
        flush_picker_label(ctx, owner, env, label_bounds);
    }
    ctx.render_styled_text(
        StyledStr::plain(selected_text),
        HorizontalAlignment::Leading,
        env,
        text_bounds,
    );
}

/// The menu picker's in-field content layout: the field label drawn above the
/// selected value, both inset by the metrics' horizontal inset and kept clear
/// of the dropdown indicator, with the theme's label spacing between them.
/// A label that measures empty (a hidden or content-free label) draws nothing
/// and takes no space — the value keeps its full-height inset.
pub fn menu_picker_content_rects(
    bounds: kurbo::Rect,
    metrics: PickerMetrics,
    label_height: f64,
) -> (Option<kurbo::Rect>, kurbo::Rect) {
    let text_x0 = bounds.x0 + metrics.horizontal_inset;
    let text_x1 = (bounds.x1 - metrics.horizontal_inset - metrics.indicator_space).max(text_x0);
    let text_bottom = bounds.y1 - metrics.vertical_inset;
    if label_height > 0.0 {
        let label_rect = kurbo::Rect::new(
            text_x0,
            bounds.y0 + metrics.vertical_inset,
            text_x1,
            bounds.y0 + metrics.vertical_inset + label_height,
        );
        let value_rect = kurbo::Rect::new(
            text_x0,
            label_rect.y1 + metrics.label_spacing,
            text_x1,
            text_bottom,
        );
        (Some(label_rect), value_rect)
    } else {
        (
            None,
            kurbo::Rect::new(
                text_x0,
                bounds.y0 + metrics.vertical_inset,
                text_x1,
                text_bottom,
            ),
        )
    }
}

/// The radio group's label layout: the heading sits in the top inset band
/// inside the horizontal insets — aligned with the option rows' leading edge —
/// and the rows begin below it with the metrics' label spacing between them.
/// The label is present iff its measured height is nonzero — the same rule the
/// measure path uses. A label measuring zero height (a hidden one) draws
/// nothing and takes no space: the rows begin at `content_y_without_label`.
pub fn radio_group_label_area(
    bounds: kurbo::Rect,
    metrics: PickerMetrics,
    label_height: f64,
    content_y_without_label: f64,
) -> (Option<kurbo::Rect>, f64) {
    if label_height > 0.0 {
        let heading = kurbo::Rect::new(
            bounds.x0 + metrics.horizontal_inset,
            bounds.y0 + metrics.vertical_inset,
            bounds.x1 - metrics.horizontal_inset,
            bounds.y0 + metrics.vertical_inset + label_height,
        );
        (Some(heading), heading.y1 + metrics.label_spacing)
    } else {
        (None, content_y_without_label)
    }
}

/// The segmented picker's label layout: the segment row is flush with the
/// control bounds, so the heading spans the full width at the top edge — edge
/// to edge like the row below it — and the row keeps exactly its unlabelled
/// height in the space below the heading and the metrics' label spacing. Same
/// presence rule: a label measuring zero height (a hidden one) draws nothing
/// and takes no space — the row keeps the full bounds.
pub fn segmented_label_area(
    bounds: kurbo::Rect,
    metrics: PickerMetrics,
    label_height: f64,
) -> (Option<kurbo::Rect>, kurbo::Rect) {
    if label_height > 0.0 {
        let heading = kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + label_height);
        let row = kurbo::Rect::new(
            bounds.x0,
            heading.y1 + metrics.label_spacing,
            bounds.x1,
            bounds.y1,
        );
        (Some(heading), row)
    } else {
        (None, bounds)
    }
}

/// Flushes the picker's retained label sub-view at `rect` with accessibility
/// suppressed: the label is a retained node re-flushed each frame so reactive
/// content stays live, and its semantics are merged into the picker's own node
/// by `picker_accessibility`, so the sub-view emits no node of its own.
fn flush_picker_label(
    ctx: &mut WidgetRenderContext<'_>,
    owner: &Rc<RefCell<PickerRenderState>>,
    env: &Environment,
    rect: kurbo::Rect,
) {
    let mut state = owner.borrow_mut();
    let render_ctx = ctx.render_context();
    let label_view = &mut state.label_view;
    ctx.renderer_mut()
        .with_suppressed_accessibility(|renderer| {
            label_view.flush_in_rect(renderer, render_ctx, env, ProposalSize::UNSPECIFIED, rect);
        });
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_radio_picker(
    ctx: &mut WidgetRenderContext<'_>,
    owner: &Rc<RefCell<PickerRenderState>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    let metrics = theme.picker_metrics(PickerStyle::Radio);
    let radio_motion = theme.radio_selection_motion();
    let selection_identity = selection.identity();
    let selected = ctx.renderer_mut().read_signal(&selection);
    let bounds = ctx.bounds;
    // The group heading sits in the top inset band; the option rows begin
    // below it with the metrics' label spacing, at the same y the
    // accessibility rows and the measured height agree on.
    let label_size = owner
        .borrow_mut()
        .label_view
        .measure_intrinsic(ctx.renderer_mut(), env);
    let (heading, mut row_y) = radio_group_label_area(
        bounds,
        metrics,
        f64::from(label_size.height),
        bounds.y0 + metrics.vertical_inset,
    );
    if let Some(heading) = heading {
        flush_picker_label(ctx, owner, env, heading);
    }
    for (row_index, item) in items.into_iter().enumerate() {
        let label = ctx
            .renderer_mut()
            .read_resolved_text_styled(&item.content, env);
        let label_size =
            HydrolysisRenderer::measure_text_intrinsic_size(ctx.state_mut(), label.clone(), env);
        let row_height = f64::from(label_size.height).max(metrics.radio_indicator_size);
        let row_rect = kurbo::Rect::new(
            bounds.x0,
            row_y,
            bounds.x1,
            (row_y + row_height).min(bounds.y1),
        );
        if row_rect.height() <= 0.0 {
            break;
        }
        row_y = row_rect.y1 + metrics.radio_row_spacing;

        let indicator_center = kurbo::Point::new(
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
        let discriminator = crate::num_cast::i32_as_u32(i32::from(item.tag)) as usize;
        let interaction_key = crate::renderer::InteractionKey::for_rc(owner, discriminator);
        let (interaction, press_slot, _) =
            ctx.renderer_mut()
                .bind_interaction_target(interaction_key, hit_rect, env);
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        {
            ctx.draw_context(|draw| {
                theme.draw_radio_indicator(
                    &mut *draw,
                    indicator_center,
                    indicator_radius,
                    radio_indicator_state,
                );
                theme.draw_radio_state_layer(
                    &mut *draw,
                    indicator_center,
                    indicator_radius,
                    is_selected,
                    interaction,
                );
            });
        }

        let indicator_rect = kurbo::Rect::new(
            indicator_center.x - indicator_radius,
            indicator_center.y - indicator_radius,
            indicator_center.x + indicator_radius,
            indicator_center.y + indicator_radius,
        );
        let label_rect = label_beside_control_bounds(
            indicator_center.x + indicator_radius + metrics.radio_label_spacing,
            row_rect.x1 - metrics.horizontal_inset,
            row_rect,
            indicator_rect,
            f64::from(label_size.height),
        );
        ctx.render_styled_text(label, HorizontalAlignment::Leading, env, label_rect);

        let tag = item.tag;
        ctx.renderer_mut()
            .register_interactive_pointer_target(hit_rect, press_slot, {
                let selection = selection.clone();
                move |_renderer, _point, _env| {
                    if selection.snapshot() == tag {
                        return false;
                    }
                    selection.set(tag);
                    true
                }
            });
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
pub fn render_segmented_picker(
    ctx: &mut WidgetRenderContext<'_>,
    owner: &Rc<RefCell<PickerRenderState>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    let metrics = theme.picker_metrics(PickerStyle::Segmented);
    let selected = ctx.renderer_mut().read_signal(&selection);
    let bounds = ctx.bounds;
    let item_count = items.len();
    // The group heading spans the full width at the top edge; the segment
    // row keeps its unlabelled height in the space below it, and keeps the
    // full bounds when the label is hidden.
    let label_size = owner
        .borrow_mut()
        .label_view
        .measure_intrinsic(ctx.renderer_mut(), env);
    let (heading, row_bounds) = segmented_label_area(bounds, metrics, f64::from(label_size.height));
    if let Some(heading) = heading {
        flush_picker_label(ctx, owner, env, heading);
    }
    let segment_width = row_bounds.width() / crate::num_cast::usize_as_f64(item_count);

    for (index, item) in items.into_iter().enumerate() {
        let x0 = segment_width.mul_add(crate::num_cast::usize_as_f64(index), row_bounds.x0);
        let segment_rect = kurbo::Rect::new(x0, row_bounds.y0, x0 + segment_width, row_bounds.y1);
        let is_selected = item.tag == selected;
        let hit_rect = transformed_rect(ctx.hit_transform, segment_rect);
        let discriminator = crate::num_cast::i32_as_u32(i32::from(item.tag)) as usize;
        let interaction_key = crate::renderer::InteractionKey::for_rc(owner, discriminator);
        let (interaction, press_slot, _) =
            ctx.renderer_mut()
                .bind_interaction_target(interaction_key, hit_rect, env);
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        {
            ctx.draw_context(|draw| {
                theme.draw_segmented_picker_segment(
                    &mut *draw,
                    segment_rect,
                    is_selected,
                    index == 0,
                    index + 1 == item_count,
                );
                theme.draw_segmented_picker_state_layer(
                    &mut *draw,
                    segment_rect,
                    is_selected,
                    index == 0,
                    index + 1 == item_count,
                    interaction,
                );
            });
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
                    if selection.snapshot() == tag {
                        return false;
                    }
                    selection.set(tag);
                    true
                }
            });
    }

    ctx.draw_context(|draw| {
        theme.draw_segmented_picker_container(&mut *draw, row_bounds, item_count);
    });
}

fn segmented_label_rect(
    segment_rect: kurbo::Rect,
    label_size: waterui_core::layout::Size,
    metrics: PickerMetrics,
) -> kurbo::Rect {
    let max_width = metrics
        .horizontal_inset
        .mul_add(-2.0, segment_rect.width())
        .max(0.0);
    let width = f64::from(label_size.width).min(max_width);
    let height = f64::from(label_size.height).min(segment_rect.height());
    let x0 = (segment_rect.width() - width).mul_add(0.5, segment_rect.x0);
    let y0 = (segment_rect.height() - height).mul_add(0.5, segment_rect.y0);
    kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}
use crate::animation::AnimationKey;

/// Emits a retained picker's accessibility tree for the semantic walk — the
/// same nodes `picker_accessibility` registers, with no bounds.
#[cfg(feature = "accessibility")]
pub fn emit_picker_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<PickerRenderState>>,
    env: &Environment,
) {
    picker_accessibility(renderer, None, None, state, env);
}

#[cfg(test)]
mod tests {
    use super::{menu_picker_content_rects, radio_group_label_area, segmented_label_area};
    use kurbo::Rect;
    use waterui_backend_core::widget::PickerMetrics;

    fn metrics() -> PickerMetrics {
        PickerMetrics {
            min_width: 280.0,
            min_height: 56.0,
            horizontal_inset: 16.0,
            vertical_inset: 8.0,
            label_spacing: 8.0,
            indicator_space: 18.0,
            radio_indicator_size: 20.0,
            radio_label_spacing: 8.0,
            radio_row_spacing: 8.0,
            popup_top_spacing: 4.0,
            popup_row_height: 48.0,
            popup_corner_radius: 4.0,
            segment_min_width: 58.0,
        }
    }

    /// The M3 exposed-dropdown label sits inside the field, directly above the
    /// selected value with the metrics' label spacing between them.
    #[test]
    fn labelled_content_places_label_above_value_inside_the_field() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 56.0);
        let (label, value) = menu_picker_content_rects(bounds, metrics(), 12.0);
        let label = label.expect("a drawn label must be placed");

        assert_eq!(label, Rect::new(16.0, 8.0, 286.0, 20.0));
        assert_eq!(value, Rect::new(16.0, 28.0, 286.0, 48.0));
        assert!(bounds.contains_rect(label) && bounds.contains_rect(value));
        assert_eq!(value.y0, label.y1 + metrics().label_spacing);
    }

    /// A hidden label measures empty: it draws nothing, takes no space, and the
    /// value keeps the field's full inset height.
    #[test]
    fn hidden_label_leaves_the_value_alone_in_the_field() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 56.0);
        let (label, value) = menu_picker_content_rects(bounds, metrics(), 0.0);

        assert!(label.is_none());
        assert_eq!(value, Rect::new(16.0, 8.0, 286.0, 48.0));
    }

    /// The radio group's heading sits in the top inset band inside the
    /// horizontal insets, and the first option row begins below it with the
    /// metrics' label spacing.
    #[test]
    fn radio_label_heading_sits_above_the_rows_with_label_spacing() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 96.0);
        let default_row_y = bounds.y0 + metrics().vertical_inset;
        let (heading, row_y) = radio_group_label_area(bounds, metrics(), 12.0, default_row_y);
        let heading = heading.expect("a drawn group label must be placed");

        assert_eq!(heading, Rect::new(16.0, 8.0, 304.0, 20.0));
        assert_eq!(row_y, heading.y1 + metrics().label_spacing);
        assert!(bounds.contains_rect(heading));
    }

    /// The segmented group's heading spans the full control width at the top
    /// edge — edge to edge like the row it heads — and the segment row fills
    /// the space below it down to the bottom edge.
    #[test]
    fn segmented_label_heading_spans_edge_to_edge_above_the_row() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 64.0);
        let (heading, row) = segmented_label_area(bounds, metrics(), 12.0);
        let heading = heading.expect("a drawn group label must be placed");

        assert_eq!(heading, Rect::new(0.0, 0.0, 320.0, 12.0));
        assert_eq!(row, Rect::new(0.0, 20.0, 320.0, 64.0));
        assert_eq!(row.y0, heading.y1 + metrics().label_spacing);
    }

    /// A hidden group label draws nothing and takes no space: no heading rect
    /// and the content keeps its unlabelled span for either group style.
    #[test]
    fn hidden_group_label_leaves_the_content_start_untouched() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 96.0);

        assert_eq!(
            radio_group_label_area(bounds, metrics(), 0.0, 14.0),
            (None, 14.0)
        );
        assert_eq!(segmented_label_area(bounds, metrics(), 0.0), (None, bounds));
    }
}
