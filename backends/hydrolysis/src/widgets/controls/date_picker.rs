#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_date_picker_intrinsic, measure_label_intrinsic, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use waterui_core::layout::{HorizontalAlignment, Size as LayoutSize};
use waterui_core::{AnyView, Environment, Native};
use waterui_form::picker::PickerStyle;
use waterui_form::picker::date::DatePickerConfig;
use waterui_text::styled::StyledStr;

use crate::widgets::util::{inset_rect, widget_theme};

const DATE_PICKER_LABEL_SPACING: f64 = 8.0;

impl HydroNativeView for Native<DatePickerConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_date_picker(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_date_picker_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let date_picker = view.as_inner();
            let value = date_picker.ty.format_value(
                renderer
                    .read_signal(&date_picker.value)
                    .clamp(*date_picker.range.start(), *date_picker.range.end()),
            );
            let default_label = Some(value.clone());
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ComboBox),
            );
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            node.set_value(value);
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::DatePicker {
                    value: date_picker.value.clone(),
                    range: date_picker.range.clone(),
                    ty: date_picker.ty,
                }),
            );
        }
    }
}

pub(crate) fn render_date_picker(
    ctx: &mut WidgetRenderContext<'_>,
    date_picker: Native<DatePickerConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let date_picker = date_picker.into_inner();
    let label_size = measure_label_intrinsic(&date_picker.label, ctx.state_mut(), env);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_width = if has_label {
        f64::from(label_size.width)
    } else {
        0.0
    };
    let spacing = if has_label {
        DATE_PICKER_LABEL_SPACING
    } else {
        0.0
    };
    let field_x0 = (ctx.bounds.x0 + label_width + spacing).min(ctx.bounds.x1);
    let label_bounds = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0,
        (field_x0 - spacing).max(ctx.bounds.x0),
        ctx.bounds.y1,
    );
    if has_label && label_bounds.width() > 0.0 {
        ctx.dispatch_in_rect_without_accessibility(env, AnyView::new(date_picker.label), label_bounds);
    }

    let field_bounds =
        vello::kurbo::Rect::new(field_x0, ctx.bounds.y0, ctx.bounds.x1, ctx.bounds.y1);
    if field_bounds.width() <= 0.0 || field_bounds.height() <= 0.0 {
        return;
    }

    let value = date_picker.ty.format_value(
        ctx.renderer_mut()
            .read_signal(&date_picker.value)
            .clamp(*date_picker.range.start(), *date_picker.range.end()),
    );

    {
        let mut draw = ctx.draw_context();
        theme.draw_input_field(&mut draw, field_bounds);
        theme.draw_picker_indicator(&mut draw, field_bounds);
    }

    let text_bounds = inset_rect(
        field_bounds,
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
        StyledStr::plain(value),
        HorizontalAlignment::Leading,
        env,
        text_bounds,
    );

    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_pointer_target(
        transformed_rect(hit_transform, field_bounds),
        move |_renderer, _point, _env| true,
    );
}
