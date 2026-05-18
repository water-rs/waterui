#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_label_intrinsic, measure_view_intrinsic, popup_menu_nodes, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use waterui::ViewExt as _;
use waterui_controls::button::{ButtonConfig, ButtonStyle};
use waterui_controls::label::Label;
use waterui_controls::menu::ResolvedMenu;
use waterui_core::layout::Point as LayoutPoint;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::color::Color;

use crate::widgets::util::{inset_rect, local_interaction_state, widget_theme};

impl HydroNativeView for Native<ButtonConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_button(ctx, view, env);
    }

    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
    ) -> LayoutSize {
        measure_button_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let button = view.as_inner();
            let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
                env,
                match button.style {
                    ButtonStyle::Link => AccessibilityNodeRole::Link,
                    _ => AccessibilityNodeRole::Button,
                },
            ));
            let default_label = renderer.accessibility_label_from_label(&button.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
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

impl HydroNativeView for Native<ResolvedMenu> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_menu(ctx, view, env);
    }

    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
    ) -> LayoutSize {
        measure_menu_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let menu = view.as_inner();
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
            );
            let default_label = Some(
                renderer
                    .read_signal(&menu.accessibility_label)
                    .to_plain()
                    .to_string(),
            );
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
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

pub(crate) fn render_button(
    ctx: &mut WidgetRenderContext<'_>,
    button: Native<ButtonConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let button = button.into_inner();
    let style = button.style;
    let bounds = ctx.bounds;
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
    {
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        theme.draw_button_chrome(&mut draw, bounds, style);
        theme.draw_button_state_layer(&mut draw, bounds, style, interaction);
    }

    let metrics = theme.button_metrics(style);
    let label_bounds = inset_rect(bounds, metrics.padding_x, metrics.padding_y);
    let label = styled_button_label(theme, style, button.label);
    let label_view = button_label_view(theme.button_label_color(style), AnyView::new(label));
    if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 {
        ctx.dispatch_in_rect_without_accessibility(env, label_view, label_bounds);
    } else {
        let render_ctx = ctx.render_context();
        let renderer = ctx.renderer_mut();
        HydrolysisRenderer::dispatch_any_without_accessibility(
            renderer, render_ctx, env, label_view,
        );
    }

    let mut action = button.action;
    let action_env = env.clone();
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |_renderer, _point, _env| {
            action(&action_env);
            true
        },
    );
}

pub(crate) fn render_menu(
    ctx: &mut WidgetRenderContext<'_>,
    menu: Native<ResolvedMenu>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let menu = menu.into_inner();
    let ResolvedMenu {
        label,
        items,
        accessibility_label: _,
    } = menu;
    let style = ButtonStyle::Borderless;
    let bounds = ctx.bounds;
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (interaction, press_slot) = ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
    {
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        theme.draw_button_chrome(&mut draw, bounds, style);
        theme.draw_button_state_layer(&mut draw, bounds, style, interaction);
    }

    let metrics = theme.button_metrics(style);
    let label_bounds = inset_rect(bounds, metrics.padding_x, metrics.padding_y);
    let label = button_label_view(theme.button_label_color(style), label);
    if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 {
        ctx.dispatch_in_rect_without_accessibility(env, label, label_bounds);
    } else {
        let render_ctx = ctx.render_context();
        let renderer = ctx.renderer_mut();
        HydrolysisRenderer::dispatch_any_without_accessibility(renderer, render_ctx, env, label);
    }

    let anchor = LayoutPoint::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |renderer, _point, env| {
            renderer.show_popup_menu_nodes(popup_menu_nodes(&items.get()), anchor, env)
        },
    );
}

pub(crate) fn measure_button_intrinsic(
    button: &ButtonConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(button.style);
    let label = styled_button_label(theme, button.style, button.label.clone());
    let label_size = measure_label_intrinsic(&label, state, env);
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    LayoutSize::new(
        content_width.max(metrics.min_width) as f32,
        content_height.max(metrics.min_height) as f32,
    )
}

pub(crate) fn measure_menu_intrinsic(
    menu: &ResolvedMenu,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(ButtonStyle::Borderless);
    let label_size = measure_view_intrinsic(&menu.label, state, env);
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    LayoutSize::new(
        content_width.max(metrics.min_width) as f32,
        content_height.max(metrics.min_height) as f32,
    )
}

fn button_label_view(color: Option<Color>, label: AnyView) -> AnyView {
    match color {
        Some(color) => AnyView::new(label.foreground(color)),
        None => label,
    }
}

fn styled_button_label(
    theme: &dyn waterui_backend_core::widget::WidgetTheme,
    style: ButtonStyle,
    label: Label,
) -> Label {
    if let Some(font) = theme.button_label_font(style) {
        label.font(font)
    } else {
        label
    }
}
