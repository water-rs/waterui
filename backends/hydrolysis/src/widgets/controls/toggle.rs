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

use crate::widgets::util::widget_theme;

pub(crate) const TOGGLE_LABEL_SPACING: f64 = 8.0;
pub(crate) const CONTROL_SPRING_STIFFNESS: f32 = 300.0;
pub(crate) const CONTROL_SPRING_DAMPING: f32 = 20.0;

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
    let spacing = TOGGLE_LABEL_SPACING;
    let control_x0 = (ctx.bounds.x1 - metrics.width).max(ctx.bounds.x0);
    let control_y0 = ctx.bounds.y0 + ((ctx.bounds.height() - metrics.height) / 2.0).max(0.0);
    let control_bounds = vello::kurbo::Rect::new(
        control_x0,
        control_y0,
        control_x0 + metrics.width,
        control_y0 + metrics.height,
    );
    let label_bounds = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0,
        (control_x0 - spacing).max(ctx.bounds.x0),
        ctx.bounds.y1,
    );
    if label_bounds.width() > 0.0 {
        ctx.dispatch_in_rect_without_accessibility(env, AnyView::new(toggle.label), label_bounds);
    }

    let thumb_progress = ctx.renderer_mut().resolve_toggle_progress(&toggle.toggle);
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
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
        label_width + TOGGLE_LABEL_SPACING + metrics.width
    } else {
        metrics.width
    };
    let height = f64::from(label_size.height).max(metrics.height);
    LayoutSize::new(width as f32, height as f32)
}
