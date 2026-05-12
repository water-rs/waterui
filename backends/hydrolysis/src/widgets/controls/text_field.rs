use crate::engine::{Brush, DrawContext};
use crate::platform::TextInputPurpose;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, TEXT_SELECTION_FILL_COLOR,
    TextInputModel, TextInputTargetRegistration, WidgetRenderContext, clamp_to_char_boundary,
    measure_secure_field_intrinsic, measure_text_field_intrinsic, measure_view_intrinsic,
    normalize_view_for_render, transformed_rect,
};
use crate::time::Instant;
use core::num::NonZeroUsize;
use waterui::cursor::CursorStyle;
use waterui_controls::text_field::ResolvedTextFieldConfig;
use waterui_core::layout::{HorizontalAlignment, Size as LayoutSize};
use waterui_core::{AnyView, Environment, Native, Str};
use waterui_form::secure::SecureFieldConfig;
use waterui_text::styled::StyledStr;

#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::{
    collapsed_accessibility_text_selection, register_accessibility_text_run_node,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};

use crate::widgets::util::widget_theme;

impl HydroNativeView for Native<ResolvedTextFieldConfig> {
    fn accessibility_is_render_driven() -> bool {
        true
    }

    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_text_field(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_text_field_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let text_field = view.as_inner();
            let line_limit = text_field.line_limit.map(NonZeroUsize::get);
            let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
                env,
                if line_limit == Some(1) {
                    AccessibilityNodeRole::TextInput
                } else {
                    AccessibilityNodeRole::MultilineTextInput
                },
            ));
            let prompt_signal = text_field.prompt.content.clone();
            let prompt = renderer.read_signal(&prompt_signal).to_plain().to_string();
            let default_label = renderer
                .accessibility_label_from_label(&text_field.label, env)
                .or_else(|| (!prompt.is_empty()).then_some(prompt.clone()));
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            if !prompt.is_empty() {
                node.set_placeholder(prompt);
            }
            let value = renderer
                .read_signal(&text_field.value)
                .to_plain()
                .to_string();
            if !value.is_empty() {
                node.set_value(value.clone());
            }
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            if let Some(text_run_node_id) =
                register_accessibility_text_run_node(renderer, &value, bounds, env)
            {
                node.set_children(vec![text_run_node_id]);
                node.set_text_selection(collapsed_accessibility_text_selection(
                    text_run_node_id,
                    value.chars().count(),
                ));
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::TextField {
                    value: text_field.value.clone(),
                    line_limit,
                }),
            );
        }
    }
}

impl HydroNativeView for Native<SecureFieldConfig> {
    fn accessibility_is_render_driven() -> bool {
        true
    }

    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_secure_field(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_secure_field_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let secure_field = view.as_inner();
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::PasswordInput),
            );
            let default_label = renderer.accessibility_label_from_label(&secure_field.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let secure_len = renderer
                .read_signal(&secure_field.value)
                .expose()
                .chars()
                .count();
            let masked_value = "*".repeat(secure_len);
            node.set_value(masked_value.clone());
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            if let Some(text_run_node_id) =
                register_accessibility_text_run_node(renderer, &masked_value, bounds, env)
            {
                node.set_children(vec![text_run_node_id]);
                node.set_text_selection(collapsed_accessibility_text_selection(
                    text_run_node_id,
                    masked_value.chars().count(),
                ));
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::SecureField {
                    value: secure_field.value.clone(),
                }),
            );
        }
    }
}

pub(crate) fn render_text_field(
    ctx: &mut WidgetRenderContext<'_>,
    text_field: Native<ResolvedTextFieldConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let input_metrics = theme.input_field_metrics();
    let mut text_field = text_field.into_inner();
    #[cfg(feature = "accessibility")]
    let default_accessibility_label = ctx
        .renderer_mut()
        .accessibility_label_from_label(&text_field.label, env);
    let label_view = normalize_view_for_render(AnyView::new(text_field.label), env);
    let line_limit = text_field.line_limit.map(NonZeroUsize::get);
    #[cfg(feature = "accessibility")]
    {
        let prompt = ctx
            .renderer_mut()
            .read_signal(&text_field.prompt.content)
            .to_plain()
            .to_string();
        let value = ctx
            .renderer_mut()
            .read_signal(&text_field.value)
            .to_plain()
            .to_string();
        let default_label =
            default_accessibility_label.or_else(|| (!prompt.is_empty()).then_some(prompt.clone()));
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let mut node = AccessibilityNode::new(ctx.renderer_mut().resolve_accessibility_role(
            env,
            if line_limit == Some(1) {
                AccessibilityNodeRole::TextInput
            } else {
                AccessibilityNodeRole::MultilineTextInput
            },
        ));
        let label = ctx
            .renderer_mut()
            .resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if !prompt.is_empty() {
            node.set_placeholder(prompt);
        }
        if !value.is_empty() {
            node.set_value(value);
        }
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        node.add_action(AccessibilityAction::SetValue);
        if let Some(node_id) = ctx.renderer_mut().register_accessibility_node(
            node,
            bounds,
            env,
            Some(AccessibilityActionTarget::TextField {
                value: text_field.value.clone(),
                line_limit,
            }),
        ) {
            ctx.renderer_mut()
                .push_pending_text_input_accessibility_node(node_id);
        }
    }
    let label_size = measure_view_intrinsic(&label_view, ctx.state_mut(), env);
    let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        ctx.dispatch_in_rect_without_accessibility(env, label_view, label_rect);
    }

    let field_rect = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0 + label_height,
        ctx.bounds.x1,
        ctx.bounds.y1,
    );
    {
        let mut draw = ctx.draw_context();
        theme.draw_input_field(&mut draw, field_rect);
    }

    let prompt_signal = text_field.prompt.content.clone();
    let selection_slot = ctx.renderer_mut().bind_text_selection_slot();
    let value_binding = text_field.value;
    let input_model = TextInputModel::TextField {
        value: value_binding.clone(),
        line_limit,
        selection_menu: text_field.selection_menu,
    };
    let (text_input_index, prompt, value, preedit, caret_opacity, is_focused, selection_visible) = {
        let text_input_index = ctx.renderer_mut().next_text_input_index();
        let is_focused = ctx.renderer_mut().is_text_input_focused(text_input_index);
        let selection_visible = is_focused
            || ctx.renderer_mut().active_text_context_menu_target() == Some(text_input_index);
        let preedit = if is_focused {
            ctx.renderer_mut().current_ime_preedit().unwrap_or_default()
        } else {
            Str::new()
        };
        let caret_opacity = if is_focused {
            ctx.renderer_mut().text_caret_opacity(Instant::now())
        } else {
            0.0
        };
        (
            text_input_index,
            ctx.renderer_mut().read_signal(&prompt_signal).to_plain(),
            ctx.renderer_mut().read_signal(&value_binding).to_plain(),
            preedit,
            caret_opacity,
            is_focused,
            selection_visible,
        )
    };
    let _ = text_input_index;
    let committed_with_preedit = value.clone() + preedit.as_str();
    let use_placeholder = committed_with_preedit.is_empty();
    let display = if use_placeholder {
        prompt
    } else {
        committed_with_preedit.clone()
    };
    let display_styled = if use_placeholder {
        StyledStr::plain(display).foreground(theme.input_placeholder_color())
    } else {
        StyledStr::plain(display)
    };
    let text_bounds = crate::widgets::util::inset_rect(
        field_rect,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
    );
    let committed_layout = HydrolysisRenderer::build_text_layout(
        ctx.state_mut(),
        StyledStr::plain(value.clone()),
        HorizontalAlignment::Leading,
        env,
        Some(text_bounds.width() as f32),
    );
    let selection = {
        let mut slot = selection_slot.borrow_mut();
        if !slot.initialized {
            slot.anchor = value.len();
            slot.focus = value.len();
            slot.initialized = true;
        }
        slot.anchor = clamp_to_char_boundary(value.as_str(), slot.anchor);
        slot.focus = clamp_to_char_boundary(value.as_str(), slot.focus);
        let anchor_layout = input_model.layout_index_from_plain_index(slot.anchor);
        let focus_layout = input_model.layout_index_from_plain_index(slot.focus);
        let anchor_affinity = if anchor_layout >= value.len() {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let focus_affinity = if focus_layout >= value.len() {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let selection = parley::Selection::new(
            parley::Cursor::from_byte_index(&committed_layout, anchor_layout, anchor_affinity),
            parley::Cursor::from_byte_index(&committed_layout, focus_layout, focus_affinity),
        )
        .refresh(&committed_layout);
        slot.anchor = input_model.plain_index_from_layout_index(selection.anchor().index());
        slot.focus = input_model.plain_index_from_layout_index(selection.focus().index());
        selection
    };
    ctx.push_layer_rect(1.0, text_bounds);
    if selection_visible && !selection.is_collapsed() {
        let selection_brush = Brush::from(vello::peniko::Color::new(TEXT_SELECTION_FILL_COLOR));
        let mut draw = ctx.draw_context();
        for (rect, _) in selection.geometry(&committed_layout) {
            let highlight = vello::kurbo::Rect::new(
                text_bounds.x0 + rect.x0,
                text_bounds.y0 + rect.y0,
                text_bounds.x0 + rect.x1,
                text_bounds.y0 + rect.y1,
            );
            draw.fill_rect(highlight, &selection_brush);
        }
    }
    ctx.render_styled_text_limited(
        display_styled,
        HorizontalAlignment::Leading,
        env,
        text_bounds,
        line_limit,
    );
    ctx.pop_layer();
    let cursor_area = {
        let rect = selection.focus().geometry(&committed_layout, 1.0);
        let x0 = text_bounds.x0 + rect.x0;
        let y0 = text_bounds.y0 + rect.y0;
        let x1 = text_bounds.x0 + rect.x1.max(rect.x0 + 1.0);
        let y1 = text_bounds.y0 + rect.y1.max(rect.y0 + 1.0);
        vello::kurbo::Rect::new(x0, y0, x1, y1)
    };
    if is_focused && selection.is_collapsed() && caret_opacity > 0.0 {
        let mut draw = ctx.draw_context();
        draw.fill_rect(
            cursor_area,
            &Brush::from(vello::peniko::Color::new([0.12, 0.14, 0.18, caret_opacity])),
        );
    }

    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_cursor_target(
        transformed_rect(hit_transform, field_rect),
        CursorStyle::IBeam,
    );
    tracing::trace!(
        target: "waterui::hydrolysis::hit_region",
        component = "text_field",
        layout_bounds = ?ctx.bounds,
        field_bounds = ?transformed_rect(ctx.hit_transform, field_rect),
        cursor_area = ?transformed_rect(ctx.hit_transform, cursor_area),
        "register text field input region"
    );
    ctx.renderer_mut()
        .register_text_input_target(TextInputTargetRegistration {
            bounds: transformed_rect(hit_transform, field_rect),
            cursor_area: transformed_rect(hit_transform, cursor_area),
            text_bounds: transformed_rect(hit_transform, text_bounds),
            layout: committed_layout,
            purpose: TextInputPurpose::Normal,
            model: input_model,
            selection: selection_slot,
        });
}

pub(crate) fn render_secure_field(
    ctx: &mut WidgetRenderContext<'_>,
    secure_field: Native<SecureFieldConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let input_metrics = theme.input_field_metrics();
    let mut secure_field = secure_field.into_inner();
    #[cfg(feature = "accessibility")]
    let default_accessibility_label = ctx
        .renderer_mut()
        .accessibility_label_from_label(&secure_field.label, env);
    let label_view = normalize_view_for_render(AnyView::new(secure_field.label), env);
    #[cfg(feature = "accessibility")]
    {
        let secure_len = ctx
            .renderer_mut()
            .read_signal(&secure_field.value)
            .expose()
            .chars()
            .count();
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let mut node = AccessibilityNode::new(
            ctx.renderer_mut()
                .resolve_accessibility_role(env, AccessibilityNodeRole::PasswordInput),
        );
        let label = ctx
            .renderer_mut()
            .resolve_accessibility_label(env, default_accessibility_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        node.set_value("*".repeat(secure_len));
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        node.add_action(AccessibilityAction::SetValue);
        if let Some(node_id) = ctx.renderer_mut().register_accessibility_node(
            node,
            bounds,
            env,
            Some(AccessibilityActionTarget::SecureField {
                value: secure_field.value.clone(),
            }),
        ) {
            ctx.renderer_mut()
                .push_pending_text_input_accessibility_node(node_id);
        }
    }
    let label_size = measure_view_intrinsic(&label_view, ctx.state_mut(), env);
    let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        ctx.dispatch_in_rect_without_accessibility(env, label_view, label_rect);
    }

    let field_rect = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0 + label_height,
        ctx.bounds.x1,
        ctx.bounds.y1,
    );
    {
        let mut draw = ctx.draw_context();
        theme.draw_input_field(&mut draw, field_rect);
    }

    let selection_slot = ctx.renderer_mut().bind_text_selection_slot();
    let value_binding = secure_field.value;
    let input_model = TextInputModel::SecureField {
        value: value_binding.clone(),
    };
    let (text_input_index, masked, caret_opacity, is_focused, selection_visible, plain_value) = {
        let text_input_index = ctx.renderer_mut().next_text_input_index();
        let is_focused = ctx.renderer_mut().is_text_input_focused(text_input_index);
        let selection_visible = is_focused
            || ctx.renderer_mut().active_text_context_menu_target() == Some(text_input_index);
        let plain_value = ctx
            .renderer_mut()
            .read_signal(&value_binding)
            .expose()
            .to_owned();
        let preedit_count = if is_focused {
            ctx.renderer_mut()
                .current_ime_preedit()
                .as_ref()
                .map_or(0, |value| value.chars().count())
        } else {
            0
        };
        let count = plain_value.chars().count() + preedit_count;
        let caret_opacity = if is_focused {
            ctx.renderer_mut().text_caret_opacity(Instant::now())
        } else {
            0.0
        };
        (
            text_input_index,
            "*".repeat(count),
            caret_opacity,
            is_focused,
            selection_visible,
            plain_value,
        )
    };
    let _ = text_input_index;
    let text_bounds = crate::widgets::util::inset_rect(
        field_rect,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
    );
    let masked_display = StyledStr::plain(masked.clone());
    let committed_layout = HydrolysisRenderer::build_text_layout(
        ctx.state_mut(),
        StyledStr::plain(masked),
        HorizontalAlignment::Leading,
        env,
        Some(text_bounds.width() as f32),
    );
    let selection = {
        let mut slot = selection_slot.borrow_mut();
        if !slot.initialized {
            slot.anchor = plain_value.len();
            slot.focus = plain_value.len();
            slot.initialized = true;
        }
        slot.anchor = clamp_to_char_boundary(plain_value.as_str(), slot.anchor);
        slot.focus = clamp_to_char_boundary(plain_value.as_str(), slot.focus);
        let text_len = plain_value.chars().count();
        let anchor_layout = input_model.layout_index_from_plain_index(slot.anchor);
        let focus_layout = input_model.layout_index_from_plain_index(slot.focus);
        let anchor_affinity = if anchor_layout >= text_len {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let focus_affinity = if focus_layout >= text_len {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let selection = parley::Selection::new(
            parley::Cursor::from_byte_index(&committed_layout, anchor_layout, anchor_affinity),
            parley::Cursor::from_byte_index(&committed_layout, focus_layout, focus_affinity),
        )
        .refresh(&committed_layout);
        slot.anchor = input_model.plain_index_from_layout_index(selection.anchor().index());
        slot.focus = input_model.plain_index_from_layout_index(selection.focus().index());
        selection
    };
    ctx.push_layer_rect(1.0, text_bounds);
    if selection_visible && !selection.is_collapsed() {
        let selection_brush = Brush::from(vello::peniko::Color::new(TEXT_SELECTION_FILL_COLOR));
        let mut draw = ctx.draw_context();
        for (rect, _) in selection.geometry(&committed_layout) {
            let highlight = vello::kurbo::Rect::new(
                text_bounds.x0 + rect.x0,
                text_bounds.y0 + rect.y0,
                text_bounds.x0 + rect.x1,
                text_bounds.y0 + rect.y1,
            );
            draw.fill_rect(highlight, &selection_brush);
        }
    }
    ctx.render_styled_text_limited(
        masked_display,
        HorizontalAlignment::Leading,
        env,
        text_bounds,
        Some(1),
    );
    ctx.pop_layer();
    let cursor_area = {
        let rect = selection.focus().geometry(&committed_layout, 1.0);
        let x0 = text_bounds.x0 + rect.x0;
        let y0 = text_bounds.y0 + rect.y0;
        let x1 = text_bounds.x0 + rect.x1.max(rect.x0 + 1.0);
        let y1 = text_bounds.y0 + rect.y1.max(rect.y0 + 1.0);
        vello::kurbo::Rect::new(x0, y0, x1, y1)
    };
    if is_focused && selection.is_collapsed() && caret_opacity > 0.0 {
        let mut draw = ctx.draw_context();
        draw.fill_rect(
            cursor_area,
            &Brush::from(vello::peniko::Color::new([0.12, 0.14, 0.18, caret_opacity])),
        );
    }

    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_cursor_target(
        transformed_rect(hit_transform, field_rect),
        CursorStyle::IBeam,
    );
    tracing::trace!(
        target: "waterui::hydrolysis::hit_region",
        component = "secure_field",
        layout_bounds = ?ctx.bounds,
        field_bounds = ?transformed_rect(ctx.hit_transform, field_rect),
        cursor_area = ?transformed_rect(ctx.hit_transform, cursor_area),
        "register secure field input region"
    );
    ctx.renderer_mut()
        .register_text_input_target(TextInputTargetRegistration {
            bounds: transformed_rect(hit_transform, field_rect),
            cursor_area: transformed_rect(hit_transform, cursor_area),
            text_bounds: transformed_rect(hit_transform, text_bounds),
            layout: committed_layout,
            purpose: TextInputPurpose::Password,
            model: input_model,
            selection: selection_slot,
        });
}
