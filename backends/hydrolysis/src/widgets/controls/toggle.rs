#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, RenderContext, WidgetRenderContext, measure_label_intrinsic,
    transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    Toggled as AccessibilityToggled,
};
use waterui_controls::toggle::{ToggleConfig, ToggleStyle};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{AnyView, Environment, Native};

use crate::renderer::local_interaction_state;
use crate::widgets::util::widget_theme;

impl HydroNativeView for Native<ToggleConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_toggle(ctx, view, env);
    }

    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
    ) -> LayoutSize {
        measure_toggle_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut crate::renderer::HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let toggle = view.as_inner();
            let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
                env,
                match toggle.style {
                    ToggleStyle::Automatic | ToggleStyle::Switch => AccessibilityNodeRole::Switch,
                    ToggleStyle::Checkbox => AccessibilityNodeRole::CheckBox,
                    _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
                },
            ));
            let default_label = renderer.accessibility_label_from_label(&toggle.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let checked = renderer.read_signal(&toggle.toggle);
            node.set_toggled(AccessibilityToggled::from(checked));
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::Toggle {
                    binding: toggle.toggle.clone(),
                }),
            );
        }
    }
}

pub(crate) fn render_toggle(
    ctx: &mut WidgetRenderContext<'_>,
    toggle: Native<ToggleConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let toggle = toggle.into_inner();
    let style = toggle.style;
    let metrics = theme.toggle_metrics(style);
    let label_size = measure_label_intrinsic(&toggle.label, ctx.state_mut(), env);
    let (control_bounds, label_bounds) =
        toggle_control_and_label_bounds(ctx.bounds, style, metrics, label_size);
    if label_bounds.width() > 0.0 {
        ctx.dispatch_in_rect_without_accessibility(env, AnyView::new(toggle.label), label_bounds);
    }

    let thumb_progress = ctx
        .renderer_mut()
        .resolve_toggle_progress(&toggle.toggle, theme.toggle_value_animation());
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
    let interaction = local_interaction_state(interaction, ctx.hit_transform);
    let mut draw = ctx.draw_context();
    match style {
        ToggleStyle::Automatic | ToggleStyle::Switch => {
            theme.draw_toggle_switch(&mut draw, control_bounds, thumb_progress, interaction);
            theme.draw_toggle_switch_state_layer(
                &mut draw,
                control_bounds,
                thumb_progress,
                interaction,
            );
        }
        ToggleStyle::Checkbox => {
            theme.draw_toggle_checkbox(&mut draw, control_bounds, thumb_progress);
            theme.draw_toggle_checkbox_state_layer(
                &mut draw,
                control_bounds,
                thumb_progress,
                interaction,
            );
        }
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }

    let toggle_binding = toggle.toggle;
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |_renderer, _point, _env| {
            let next = !toggle_binding.get();
            toggle_binding.set(next);
            true
        },
    );
}

pub(crate) fn measure_toggle_intrinsic(
    toggle: &ToggleConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.toggle_metrics(toggle.style);
    let label_size = measure_label_intrinsic(&toggle.label, state, env);
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + metrics.label_spacing + metrics.width
    } else {
        metrics.width
    };
    let height = f64::from(label_size.height).max(metrics.height);
    LayoutSize::new(width as f32, height as f32)
}

fn toggle_control_and_label_bounds(
    bounds: vello::kurbo::Rect,
    style: ToggleStyle,
    metrics: waterui_backend_core::widget::ToggleMetrics,
    label_size: LayoutSize,
) -> (vello::kurbo::Rect, vello::kurbo::Rect) {
    let control_y0 = bounds.y0 + ((bounds.height() - metrics.height) / 2.0).max(0.0);
    let control_y1 = control_y0 + metrics.height;
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    match style {
        ToggleStyle::Checkbox => {
            let control_x0 = bounds.x0;
            let control_x1 = control_x0 + metrics.width;
            let label_x0 = if has_label {
                (control_x1 + metrics.label_spacing).min(bounds.x1)
            } else {
                control_x1
            };
            let max_label_width = (bounds.x1 - label_x0).max(0.0);
            let label_width = f64::from(label_size.width).min(max_label_width);
            let label_height = f64::from(label_size.height).min(bounds.height());
            let label_y0 = bounds.y0 + (bounds.height() - label_height) * 0.5;
            (
                vello::kurbo::Rect::new(control_x0, control_y0, control_x1, control_y1),
                vello::kurbo::Rect::new(
                    label_x0,
                    label_y0,
                    label_x0 + label_width,
                    label_y0 + label_height,
                ),
            )
        }
        ToggleStyle::Automatic | ToggleStyle::Switch => {
            let control_x0 = (bounds.x1 - metrics.width).max(bounds.x0);
            let label_x1 = if has_label {
                (control_x0 - metrics.label_spacing).max(bounds.x0)
            } else {
                bounds.x0
            };
            (
                vello::kurbo::Rect::new(
                    control_x0,
                    control_y0,
                    control_x0 + metrics.width,
                    control_y1,
                ),
                vello::kurbo::Rect::new(bounds.x0, bounds.y0, label_x1, bounds.y1),
            )
        }
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }
}

#[cfg(test)]
mod tests {
    use super::toggle_control_and_label_bounds;
    use vello::kurbo::Rect;
    use waterui_backend_core::widget::ToggleMetrics;
    use waterui_controls::toggle::ToggleStyle;
    use waterui_core::layout::Size;

    #[test]
    fn checkbox_layout_places_control_before_label() {
        let metrics = ToggleMetrics::new(18.0, 18.0, 8.0);
        let (control, label) = toggle_control_and_label_bounds(
            Rect::new(16.0, 20.0, 320.0, 60.0),
            ToggleStyle::Checkbox,
            metrics,
            Size::new(64.0, 16.0),
        );

        assert_eq!(control, Rect::new(16.0, 31.0, 34.0, 49.0));
        assert_eq!(label, Rect::new(42.0, 32.0, 106.0, 48.0));
    }

    #[test]
    fn switch_layout_keeps_control_trailing() {
        let metrics = ToggleMetrics::new(52.0, 32.0, 8.0);
        let (control, label) = toggle_control_and_label_bounds(
            Rect::new(16.0, 20.0, 320.0, 60.0),
            ToggleStyle::Switch,
            metrics,
            Size::new(64.0, 16.0),
        );

        assert_eq!(control, Rect::new(268.0, 24.0, 320.0, 56.0));
        assert_eq!(label, Rect::new(16.0, 20.0, 260.0, 60.0));
    }
}
