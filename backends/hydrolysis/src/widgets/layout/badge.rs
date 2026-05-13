use waterui::component::badge::BadgeConfig;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_text::styled::StyledStr;

use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_view_dimensions_with_proposal, measure_view_intrinsic, normalize_view_for_render,
};
use crate::widgets::widget_theme;

fn badge_content_size(
    state: &mut HydroState,
    badge: &Native<BadgeConfig>,
    env: &Environment,
) -> LayoutSize {
    let content = normalize_view_for_render(badge.as_inner().content.build(), env);
    measure_view_intrinsic(&content, state, env)
}

fn badge_large_label(value: i32, env: &Environment) -> StyledStr {
    let theme = widget_theme(env);
    StyledStr::plain(value.to_string())
        .font(theme.badge_label_font())
        .foreground(theme.badge_label_color())
}

impl HydroNativeView for Native<BadgeConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, badge: Self, env: &Environment) {
        let badge = badge.into_inner();
        let content = normalize_view_for_render(badge.content.build(), env);
        ctx.dispatch_in_rect(env, content, ctx.bounds);

        let theme = widget_theme(env);
        let metrics = theme.badge_metrics();
        let value = ctx.renderer_mut().read_signal(&badge.value);
        let content_width = ctx.bounds.width();
        let x0 = if value == 0 {
            ctx.bounds.x0 + content_width * 0.5 + metrics.small_offset_x
        } else {
            ctx.bounds.x0 + content_width * 0.5 + metrics.large_offset_x
        };
        let y0 = if value == 0 {
            ctx.bounds.y0 + metrics.small_offset_y
        } else {
            ctx.bounds.y0 + metrics.large_offset_y
        };

        if value == 0 {
            let rect =
                vello::kurbo::Rect::new(x0, y0, x0 + metrics.small_size, y0 + metrics.small_size);
            let mut draw = ctx.draw_context();
            theme.draw_badge_small(&mut draw, rect);
            return;
        }

        let label = badge_large_label(value, env);
        let text_size = HydrolysisRenderer::measure_text_dimensions(
            ctx.state_mut(),
            label.clone(),
            HorizontalAlignment::Center,
            env,
            None,
            Some(1),
        )
        .size;
        let width = (f64::from(text_size.width) + metrics.large_horizontal_padding * 2.0)
            .max(metrics.large_size);
        let rect = vello::kurbo::Rect::new(x0, y0, x0 + width, y0 + metrics.large_size);
        {
            let mut draw = ctx.draw_context();
            theme.draw_badge_large(&mut draw, rect);
        }

        let text_height = f64::from(text_size.height);
        let text_rect = vello::kurbo::Rect::new(
            rect.x0,
            rect.y0 + (rect.height() - text_height) * 0.5,
            rect.x1,
            rect.y1,
        );
        let text_ctx = RenderContext {
            transform: ctx.transform,
            hit_transform: ctx.hit_transform,
            bounds: text_rect,
        };
        let (state, scene) = ctx.renderer_mut().state_and_scene_mut();
        HydrolysisRenderer::render_styled_text(
            state,
            scene,
            text_ctx,
            label,
            HorizontalAlignment::Center,
            env,
        );
    }

    fn intrinsic(state: &mut HydroState, badge: &Self, env: &Environment) -> LayoutSize {
        badge_content_size(state, badge, env)
    }

    fn dimensions(
        state: &mut HydroState,
        badge: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let content = normalize_view_for_render(badge.as_inner().content.build(), env);
        measure_view_dimensions_with_proposal(&content, proposal, state, env)
    }

    fn accessibility_is_render_driven() -> bool {
        true
    }
}
