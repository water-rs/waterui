#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use vello::kurbo::{Rect, RoundedRectRadii};
use waterui_backend_core::widget::{Brush, DrawContext as _};
use waterui_core::layout::{HorizontalAlignment, Size as LayoutSize};
use waterui_core::{AnyView, Environment, Native};
use waterui_form::picker::color::ColorPickerConfig;
use waterui_graphics::color::Color;
use waterui_text::styled::StyledStr;

use crate::renderer::local_interaction_state;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_label_intrinsic, resolved_color_to_peniko, transformed_rect,
};
use crate::widgets::util::{inset_rect, widget_theme};

const COLOR_SWATCH_SIZE: f64 = 32.0;
const COLOR_SWATCH_RADIUS: f64 = 8.0;
const COLOR_PICKER_MIN_WIDTH: f32 = 160.0;

impl HydroNativeView for Native<ColorPickerConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_color_picker(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_color_picker_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let color_picker = view.as_inner();
            let label = color_picker
                .label
                .semantic_text()
                .resolve(env)
                .content
                .get()
                .to_plain()
                .to_string();
            let value = format!("{:?}", renderer.read_signal(&color_picker.value));
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
            );
            node.set_label(label);
            node.set_value(value);
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let activation_point = accessibility_activation_point(bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::PointerPrimaryClick {
                    point: activation_point,
                }),
            );
        }
    }
}

fn measure_color_picker_intrinsic(
    color_picker: &ColorPickerConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let input_metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(&color_picker.label, state, env);
    let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    LayoutSize::new(
        COLOR_PICKER_MIN_WIDTH,
        (label_height + input_metrics.min_height) as f32,
    )
}

fn render_color_picker(
    ctx: &mut WidgetRenderContext<'_>,
    color_picker: Native<ColorPickerConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let input_metrics = theme.input_field_metrics();
    let color_picker = color_picker.into_inner();
    let label_size = measure_label_intrinsic(&color_picker.label, ctx.state_mut(), env);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = if has_label {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_bounds = Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        ctx.dispatch_in_rect_without_accessibility(
            env,
            AnyView::new(color_picker.label),
            label_bounds,
        );
    }

    let field_bounds = Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0 + label_height,
        ctx.bounds.x1,
        ctx.bounds.y1,
    );
    if field_bounds.width() <= 0.0 || field_bounds.height() <= 0.0 {
        return;
    }

    let hit_bounds = transformed_rect(ctx.hit_transform, field_bounds);
    let (interaction, press_slot, handles) =
        ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
    {
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        // The field chrome samples interaction state (focus/hover tint).
        handles.mark_chrome_state_dependent();
        theme.draw_input_field(&mut draw, field_bounds, interaction);
    }
    let render_ctx = ctx.render_context();
    ctx.renderer_mut().capture_state_layers(
        render_ctx,
        &handles,
        field_bounds,
        false,
        &|draw, state| {
            theme.draw_input_field_state_layer(draw, field_bounds, state);
        },
    );

    let content_bounds = inset_rect(
        field_bounds,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
    );
    let swatch_size = COLOR_SWATCH_SIZE.min(content_bounds.height()).max(0.0);
    let swatch_rect = Rect::new(
        content_bounds.x0,
        content_bounds.y0 + (content_bounds.height() - swatch_size) / 2.0,
        content_bounds.x0 + swatch_size,
        content_bounds.y0 + (content_bounds.height() + swatch_size) / 2.0,
    );
    let color = ctx.renderer_mut().read_signal(&color_picker.value);
    let swatch_color = resolved_color_to_peniko(color.resolve(env).get());
    {
        let mut draw = ctx.draw_context();
        draw.fill_rounded_rect(
            swatch_rect,
            RoundedRectRadii::from_single_radius(COLOR_SWATCH_RADIUS),
            &Brush::from(swatch_color),
        );
        draw.stroke_rounded_rect(
            swatch_rect,
            RoundedRectRadii::from_single_radius(COLOR_SWATCH_RADIUS),
            &Brush::from(resolved_color_to_peniko(
                Color::srgb(0, 0, 0).with_opacity(0.16).resolve(env).get(),
            )),
            1.0,
        );
    }

    let text_bounds = Rect::new(
        swatch_rect.x1 + input_metrics.horizontal_inset,
        content_bounds.y0,
        content_bounds.x1,
        content_bounds.y1,
    );
    if text_bounds.width() > 0.0 {
        let suffix = match (color_picker.support_alpha, color_picker.support_hdr) {
            (true, true) => "Alpha, HDR",
            (true, false) => "Alpha",
            (false, true) => "HDR",
            (false, false) => "Color",
        };
        ctx.render_styled_text(
            StyledStr::plain(suffix),
            HorizontalAlignment::Leading,
            env,
            text_bounds,
        );
    }

    let value = color_picker.value.clone();
    let origin = waterui_core::layout::Point::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
    let support_alpha = color_picker.support_alpha;
    let support_hdr = color_picker.support_hdr;
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |renderer, _point, env| {
            renderer.show_color_picker(value.clone(), support_alpha, support_hdr, origin, env)
        },
    );
}
