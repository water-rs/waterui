use nami::{Computed, Signal};
use std::cell::RefCell;
use std::rc::Rc;
use waterui::component::badge::BadgeConfig;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_text::styled::StyledStr;

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
pub struct BadgeRenderState<E: crate::text::TextEngine> {
    value: Computed<i32>,
    content: RetainedSubview<E>,
}

impl<E: crate::text::TextEngine> BadgeRenderState<E> {
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
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
    ) {
        self.content.ensure_built(renderer, env);
    }

    /// Consume the content sub-view's layout-dirty marks: its measured size
    /// feeds the badge's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.content.take_layout_dirty_if_built()
    }
}

fn badge_content_size<E: crate::text::TextEngine>(
    state: &mut HydroState<E>,
    badge: &Native<BadgeConfig>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> LayoutSize {
    let content = normalize_view_for_render(badge.as_inner().content.build(), env);
    measure_transient_view_intrinsic(&content, state, env, theme)
}

fn badge_large_label(value: i32, theme: &Rc<dyn crate::theme::WidgetTheme>) -> StyledStr {
    StyledStr::plain(value.to_string())
        .font(theme.badge_label_font())
        .foreground(theme.badge_label_color())
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<BadgeConfig> {
    fn intrinsic(
        state: &mut HydroState<E>,
        badge: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        badge_content_size(state, badge, env, theme)
    }

    fn dimensions(
        state: &mut HydroState<E>,
        badge: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let content = normalize_view_for_render(badge.as_inner().content.build(), env);
        measure_view_dimensions_with_proposal(&content, proposal, state, env, theme)
    }
}

/// Measures a retained badge leaf from its [`BadgeRenderState`]: the badge sizes
/// itself to its wrapped content, mirroring the dispatch path's `dimensions`.
pub fn measure_badge_node<E: crate::text::TextEngine>(
    state: &BadgeRenderState<E>,
    _proposal: ProposalSize,
    hydro: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    ViewDimensions::new(state.content.measure_built(hydro, env, theme))
}

/// Renders a retained badge leaf every flush: flushes the content sub-view (whose
/// own dispatch drives accessibility — badge a11y is render-driven) then draws the
/// indicator overlay from the live `value` signal.
pub fn render_badge_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<BadgeRenderState<E>>>,
    env: &Environment,
) {
    render_badge_parts(ctx, state, env);
}

pub fn render_badge_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<BadgeRenderState<E>>>,
    env: &Environment,
) {
    let bounds = ctx.bounds;
    {
        let render_ctx = ctx.render_context();
        let mut state = state.borrow_mut();
        let content_area = ctx.safe_area_for(bounds);
        state.content.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            ProposalSize::UNSPECIFIED,
            bounds,
            content_area,
        );
    }

    let value = {
        let signal = state.borrow().value.clone();
        ctx.renderer_mut().read_signal(&signal)
    };
    draw_badge_indicator(ctx, bounds, value, env);
}

/// Draws the badge indicator for `value` anchored on `anchor`: a count of zero
/// is the theme's small dot, anything else the large count pill. Shared by the
/// badge widget and by tab items, which anchor it on their icon.
pub fn draw_badge_indicator<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    anchor: kurbo::Rect,
    value: i32,
    env: &Environment,
) {
    let theme = ctx.theme();
    let metrics = theme.badge_metrics();

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
            metrics
                .large_horizontal_padding
                .mul_add(2.0, f64::from(text_size.width))
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
        anchor.x0 + offset_x - badge_width
    } else {
        anchor.x1 - offset_x
    };
    let y0 = anchor.y0 + offset_y - badge_height;
    let rect = kurbo::Rect::new(x0, y0, x0 + badge_width, y0 + badge_height);

    let Some((label, text_size)) = large else {
        ctx.draw_context(|draw| {
            theme.draw_badge_small(&mut *draw, rect);
        });
        return;
    };
    {
        ctx.draw_context(|draw| {
            theme.draw_badge_large(&mut *draw, rect);
        });
    }

    // The count indicator is vector-drawn, so it must emit its own semantic
    // node — otherwise the badge value is invisible to assistive technology.
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(AccessibilityNodeRole::Label);
        node.set_label(value.to_string());
        let node_bounds = rect;
        let _ = ctx
            .renderer_mut()
            .register_accessibility_node(node, node_bounds, env, None);
    }

    let text_height = f64::from(text_size.height);
    let text_rect = kurbo::Rect::new(
        rect.x0,
        (rect.height() - text_height).mul_add(0.5, rect.y0),
        rect.x1,
        rect.y1,
    );
    let text_ctx = RenderContext {
        local: ctx.local,
        bounds: text_rect,
    };
    ctx.renderer_mut()
        .render_styled_text(text_ctx, label, HorizontalAlignment::Center, env);
}

/// Emits a retained badge's accessibility nodes for the semantic walk: the
/// wrapped content's subtree (unsuppressed in the rendered path), then the
/// count indicator's `Label` node — a vector-drawn badge value is otherwise
/// invisible to assistive technology.
#[cfg(feature = "accessibility")]
pub fn emit_badge_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<BadgeRenderState<E>>>,
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
