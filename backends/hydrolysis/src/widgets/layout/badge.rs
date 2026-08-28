use nami::Computed;
use std::cell::RefCell;
use std::rc::Rc;
use waterui::component::badge::BadgeConfig;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_text::styled::StyledStr;

#[cfg(feature = "accessibility")]
use crate::renderer::transformed_rect;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, measure_view_dimensions_with_proposal, measure_view_intrinsic,
    normalize_view_for_render,
};
use crate::widgets::widget_theme;
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};

/// The retained render state of a badge. The wrapped `content` is a move-only
/// `AnyView` (built once from the config's `AnyViewBuilder`), so the persistent
/// `Widget` node holds it as a [`RetainedSubview`] built once and re-flushed each
/// frame; the `value` is read through `read_signal` so a change schedules a frame
/// and the indicator re-renders.
pub(crate) struct BadgeRenderState {
    value: Computed<i32>,
    content: RetainedSubview,
}

impl BadgeRenderState {
    pub(crate) fn from_config(config: BadgeConfig) -> Self {
        let BadgeConfig { value, content, .. } = config;
        Self {
            value,
            content: RetainedSubview::new(content.build()),
        }
    }

    /// Eagerly build the content sub-view (the measure path has only `&mut
    /// HydroState`, no renderer, so it must be built before then).
    pub(crate) fn prebuild_content(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) {
        self.content.ensure_built(renderer, env);
    }
}

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
}

/// Measures a retained badge leaf from its [`BadgeRenderState`]: the badge sizes
/// itself to its wrapped content, mirroring the dispatch path's `dimensions`.
pub(crate) fn measure_badge_node(
    state: &BadgeRenderState,
    _proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(state.content.measure_built(hydro, env))
}

/// Renders a retained badge leaf every flush: flushes the content sub-view (whose
/// own dispatch drives accessibility — badge a11y is render-driven) then draws the
/// indicator overlay from the live `value` signal.
pub(crate) fn render_badge_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<BadgeRenderState>>,
    env: &Environment,
) {
    render_badge_parts(ctx, state, env);
}

pub(crate) fn render_badge_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<BadgeRenderState>>,
    env: &Environment,
) {
    let bounds = ctx.bounds;
    {
        let render_ctx = ctx.render_context();
        let mut state = state.borrow_mut();
        state
            .content
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
    }

    let theme = widget_theme(env);
    let metrics = theme.badge_metrics();
    let value = {
        let signal = state.borrow().value.clone();
        ctx.renderer_mut().read_signal(&signal)
    };
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

    // The count indicator is vector-drawn, so it must emit its own semantic
    // node — otherwise the badge value is invisible to assistive technology.
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(AccessibilityNodeRole::Label);
        node.set_label(value.to_string());
        let node_bounds = transformed_rect(ctx.hit_transform, rect);
        let _ = ctx
            .renderer_mut()
            .register_accessibility_node(node, node_bounds, env, None);
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
    let (hydro, scene) = ctx.renderer_mut().state_and_scene_mut();
    HydrolysisRenderer::render_styled_text(
        hydro,
        scene,
        text_ctx,
        label,
        HorizontalAlignment::Center,
        env,
    );
}
