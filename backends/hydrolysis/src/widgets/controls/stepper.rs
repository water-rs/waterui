use crate::engine::{Brush, DrawContext};
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, RenderContext, WidgetRenderContext, measure_label_intrinsic,
    transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use waterui_controls::stepper::StepperConfig;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{AnyView, Environment, Native};

use crate::widgets::util::widget_theme;

pub(crate) const STEPPER_BUTTON_SPACING: f64 = 4.0;
pub(crate) const STEPPER_LABEL_SPACING: f64 = 8.0;

impl HydroNativeView for Native<StepperConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_stepper(ctx, view, env);
    }

    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
    ) -> LayoutSize {
        measure_stepper_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut crate::renderer::HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let stepper = view.as_inner();
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::SpinButton),
            );
            let default_label = renderer.accessibility_label_from_label(&stepper.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let start = *stepper.range.start();
            let end = *stepper.range.end();
            assert!(
                (start <= end),
                "hydrolysis stepper requires an ordered range"
            );
            let current = stepper.value.get().clamp(start, end);
            node.set_numeric_value(f64::from(current));
            node.set_min_numeric_value(f64::from(start));
            node.set_max_numeric_value(f64::from(end));
            let step = stepper.step.get();
            assert!((step > 0), "hydrolysis stepper requires positive step");
            node.set_numeric_value_step(f64::from(step));
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Increment);
            node.add_action(AccessibilityAction::Decrement);
            node.add_action(AccessibilityAction::SetValue);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::Stepper {
                    value: stepper.value.clone(),
                    step: stepper.step.clone(),
                    range: stepper.range.clone(),
                }),
            );
        }
    }
}

pub(crate) fn render_stepper(
    ctx: &mut WidgetRenderContext<'_>,
    stepper: Native<StepperConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let stepper = stepper.into_inner();
    let theme_metrics = theme.stepper_metrics();
    let button_size = ctx
        .bounds
        .height()
        .clamp(theme_metrics.button_min_size, theme_metrics.button_max_size);
    let spacing = STEPPER_BUTTON_SPACING;
    let controls_width = button_size * 2.0 + spacing;
    let controls_x0 = (ctx.bounds.x1 - controls_width).max(ctx.bounds.x0);

    let label_bounds =
        vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y0, controls_x0, ctx.bounds.y1);
    if label_bounds.width() > 0.0 {
        ctx.dispatch_in_rect_without_accessibility(env, AnyView::new(stepper.label), label_bounds);
    }

    let button_y0 = ctx.bounds.y0 + ((ctx.bounds.height() - button_size) / 2.0).max(0.0);
    let minus_bounds = vello::kurbo::Rect::new(
        controls_x0,
        button_y0,
        controls_x0 + button_size,
        button_y0 + button_size,
    );
    let plus_bounds = vello::kurbo::Rect::new(
        controls_x0 + button_size + spacing,
        button_y0,
        controls_x0 + controls_width,
        button_y0 + button_size,
    );
    let mut draw = ctx.draw_context();
    theme.draw_stepper_button(&mut draw, minus_bounds);
    theme.draw_stepper_button(&mut draw, plus_bounds);

    let line_color = Brush::from(vello::peniko::Color::new([0.2, 0.2, 0.22, 1.0]));
    draw.stroke_line(
        vello::kurbo::Point::new(minus_bounds.x0 + 6.0, minus_bounds.y0 + button_size / 2.0),
        vello::kurbo::Point::new(minus_bounds.x1 - 6.0, minus_bounds.y0 + button_size / 2.0),
        &line_color,
        2.0,
    );
    draw.stroke_line(
        vello::kurbo::Point::new(plus_bounds.x0 + 6.0, plus_bounds.y0 + button_size / 2.0),
        vello::kurbo::Point::new(plus_bounds.x1 - 6.0, plus_bounds.y0 + button_size / 2.0),
        &line_color,
        2.0,
    );
    draw.stroke_line(
        vello::kurbo::Point::new(plus_bounds.x0 + button_size / 2.0, plus_bounds.y0 + 6.0),
        vello::kurbo::Point::new(plus_bounds.x0 + button_size / 2.0, plus_bounds.y1 - 6.0),
        &line_color,
        2.0,
    );

    let range_start = *stepper.range.start();
    let range_end = *stepper.range.end();
    assert!(
        (range_start <= range_end),
        "hydrolysis stepper requires an ordered range"
    );

    let value_binding_minus = stepper.value.clone();
    let value_binding_plus = stepper.value;
    let step_signal_minus = stepper.step.clone();
    let step_signal_plus = stepper.step;

    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_pointer_target(
        transformed_rect(hit_transform, minus_bounds),
        move |_renderer, _point, _env| {
            let step = step_signal_minus.get();
            assert!((step > 0), "hydrolysis stepper requires positive step");
            let current = value_binding_minus.get();
            let next = current.saturating_sub(step).clamp(range_start, range_end);
            value_binding_minus.set(next);
            true
        },
    );
    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_pointer_target(
        transformed_rect(hit_transform, plus_bounds),
        move |_renderer, _point, _env| {
            let step = step_signal_plus.get();
            assert!((step > 0), "hydrolysis stepper requires positive step");
            let current = value_binding_plus.get();
            let next = current.saturating_add(step).clamp(range_start, range_end);
            value_binding_plus.set(next);
            true
        },
    );
}

pub(crate) fn measure_stepper_intrinsic(
    stepper: &StepperConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.stepper_metrics();
    let label_size = measure_label_intrinsic(&stepper.label, state, env);
    let controls_width = metrics.button_intrinsic_size * 2.0 + STEPPER_BUTTON_SPACING;
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + STEPPER_LABEL_SPACING + controls_width
    } else {
        controls_width
    };
    let height = f64::from(label_size.height).max(metrics.button_intrinsic_size);
    LayoutSize::new(width as f32, height as f32)
}
