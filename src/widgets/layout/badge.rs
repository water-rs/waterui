use nami::{Computed, Signal};
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
    WidgetRenderContext, measure_transient_view_intrinsic, measure_view_dimensions_with_proposal,
    normalize_view_for_render,
};
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
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        self.content.ensure_built(renderer, env);
    }
}

fn badge_content_size(
    state: &mut HydroState,
    badge: &Native<BadgeConfig>,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let content = normalize_view_for_render(badge.as_inner().content.build(), env);
    measure_transient_view_intrinsic(&content, state, env, theme)
}

fn badge_large_label(value: i32, theme: &Rc<dyn crate::engine::WidgetTheme>) -> StyledStr {
    StyledStr::plain(value.to_string())
        .font(theme.badge_label_font())
        .foreground(theme.badge_label_color())
}

impl HydroNativeView for Native<BadgeConfig> {
    fn intrinsic(
        state: &mut HydroState,
        badge: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        badge_content_size(state, badge, env, theme)
    }

    fn dimensions(
        state: &mut HydroState,
        badge: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let content = normalize_view_for_render(badge.as_inner().content.build(), env);
        measure_view_dimensions_with_proposal(&content, proposal, state, env, theme)
    }
}

/// Measures a retained badge leaf from its [`BadgeRenderState`]: the badge sizes
/// itself to its wrapped content, mirroring the dispatch path's `dimensions`.
pub(crate) fn measure_badge_node(
    state: &BadgeRenderState,
    _proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    ViewDimensions::new(state.content.measure_built(hydro, env, theme))
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
        state.content.flush_in_rect(
            ctx.renderer_mut(),
            render_ctx,
            env,
            ProposalSize::UNSPECIFIED,
            bounds,
        );
    }

    let theme = ctx.theme();
    let metrics = theme.badge_metrics();
    let value = {
        let signal = state.borrow().value.clone();
        ctx.renderer_mut().read_signal(&signal)
    };

    // Measure before placing: the badge's leading edge sits `offset_x` inside
    // the content's trailing edge — mirrored to the leading edge in RTL — and
    // its bottom edge overlaps the top edge by `offset_y`, matching
    // `BadgedBox` in Compose.
    let large = (value != 0).then(|| {
        let label = badge_large_label(value, &theme);
        let text_size = HydrolysisRenderer::measure_text_dimensions(
            ctx.state_mut(),
            label.clone(),
            HorizontalAlignment::Center,
            env,
            None,
            Some(1),
        )
        .size;
        (label, text_size)
    });

    let (badge_width, badge_height, offset_x, offset_y) = match &large {
        None => (
            metrics.small_size,
            metrics.small_size,
            metrics.small_offset_x,
            metrics.small_offset_y,
        ),
        Some((_, text_size)) => (
            (f64::from(text_size.width) + metrics.large_horizontal_padding * 2.0)
                .max(metrics.large_size),
            metrics.large_size,
            metrics.large_offset_x,
            metrics.large_offset_y,
        ),
    };

    let x0 = if waterui_core::layout::layout_direction(env)
        .snapshot()
        .is_right_to_left()
    {
        ctx.bounds.x0 + offset_x - badge_width
    } else {
        ctx.bounds.x1 - offset_x
    };
    let y0 = ctx.bounds.y0 + offset_y - badge_height;
    let rect = kurbo::Rect::new(x0, y0, x0 + badge_width, y0 + badge_height);

    let Some((label, text_size)) = large else {
        let mut draw = ctx.draw_context();
        theme.draw_badge_small(&mut draw, rect);
        return;
    };
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
    let text_rect = kurbo::Rect::new(
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

/// Emits a retained badge's accessibility nodes for the semantic walk: the
/// wrapped content's subtree (unsuppressed in the rendered path), then the
/// count indicator's `Label` node — a vector-drawn badge value is otherwise
/// invisible to assistive technology.
#[cfg(feature = "accessibility")]
pub(crate) fn emit_badge_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<BadgeRenderState>>,
    env: &Environment,
) {
    let mut state = state.borrow_mut();
    state.content.emit_accessibility(renderer, env);
    let value = {
        let signal = state.value.clone();
        renderer.read_signal(&signal)
    };
    if value != 0 {
        let mut node = AccessibilityNode::new(AccessibilityNodeRole::Label);
        node.set_label(value.to_string());
        let _ = renderer.register_accessibility_node_semantic(node, env, None);
    }
}
