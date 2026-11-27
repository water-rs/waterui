//! Hydrolysis realization of [`Avatar`](waterui_controls::avatar::Avatar):
//! clipped backdrop, optional derived monogram or custom fallback view, optional
//! image overlay, optional surrounding ring, and a single `Role::Image`
//! accessibility leaf.

use core::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use cherenkov_record::Paint;
use kurbo::{Rect, Stroke};
use nami::{Computed, SignalExt as _};
#[cfg(waterui_feature = "image")]
use waterui_backend_core::media::Photo;
use waterui_controls::avatar::{AvatarConfig, AvatarRing};
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
#[cfg(waterui_feature = "image")]
use waterui_layout::ContentMode;
use waterui_shape::{Circle, ClipShape, PathCommand, ShapeKind};
use waterui_text::styled::StyledStr;

use crate::num_cast::f64_as_f32;
use crate::renderer::mount::{ScopeDelta, ScopeKey};
use crate::renderer::{
    EngineAnimated, HeldChild, HydroState, RenderContext, RetainedSubview, SemanticCore,
    WidgetRenderContext, bounded_proposal, resolve_scope_clip_parts, shape_data_for_parts,
};
use crate::text::TextEngine;
use crate::theme::{AvatarLook, WidgetTheme};

/// Retained render state for an [`Avatar`](waterui_controls::avatar::Avatar).
pub struct AvatarRenderState<E: TextEngine> {
    config: AvatarConfig<HeldChild<E>>,
    shape_kind: ShapeKind,
    shape_commands: Vec<PathCommand>,
    fallback_view: Option<RetainedSubview<E>>,
    #[cfg(waterui_feature = "image")]
    picture_view: Option<RetainedSubview<E>>,
}

impl<E: TextEngine> AvatarRenderState<E> {
    pub(crate) fn from_config(mut config: AvatarConfig<HeldChild<E>>) -> Self {
        let default_shape = ClipShape::new(Circle);
        let shape = config.shape().unwrap_or(&default_shape);
        let shape_kind = shape.kind();
        let shape_commands = shape.commands().to_vec();
        let fallback_view = config.take_fallback().map(RetainedSubview::held);
        #[cfg(waterui_feature = "image")]
        let picture_view = config.image().map(|source| {
            RetainedSubview::new(
                Photo::new(source.clone())
                    .resizable()
                    .content_mode(ContentMode::Fill),
            )
        });
        Self {
            config,
            shape_kind,
            shape_commands,
            fallback_view,
            #[cfg(waterui_feature = "image")]
            picture_view,
        }
    }

    pub(crate) fn prebuild(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        if let Some(fallback) = &mut self.fallback_view {
            fallback.ensure_built(renderer, env);
        }
        #[cfg(waterui_feature = "image")]
        if let Some(picture) = &mut self.picture_view {
            picture.ensure_built(renderer, env);
        }
    }

    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        let fallback_dirty = self
            .fallback_view
            .as_mut()
            .is_some_and(RetainedSubview::take_layout_dirty_if_built);
        #[cfg(waterui_feature = "image")]
        let picture_dirty = self
            .picture_view
            .as_mut()
            .is_some_and(RetainedSubview::take_layout_dirty_if_built);
        #[cfg(not(waterui_feature = "image"))]
        let picture_dirty = false;
        fallback_dirty || picture_dirty
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "WidgetBehavior::contributed_label expects Option<String>"
    )]
    pub(crate) fn contributed_label(
        &self,
        renderer: &mut SemanticCore<E>,
        _env: &Environment,
    ) -> Option<String> {
        Some(renderer.read_signal(self.config.spoken()).to_string())
    }
}

fn avatar_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    ctx: Option<RenderContext>,
    config: &AvatarConfig<HeldChild<E>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::Image,
        ));
        let default_label = Some(renderer.read_signal(config.spoken()).to_string());
        if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
            node.set_label(label);
        }
        let _ = renderer.register_accessibility_leaf(ctx, node, env, None);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, config, env);
    }
}

/// Emits accessibility nodes for an [`Avatar`](waterui_controls::avatar::Avatar)
/// during semantic-only tree walks.
#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
pub fn emit_avatar_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    state: &Rc<RefCell<AvatarRenderState<E>>>,
    env: &Environment,
) {
    if renderer.is_accessibility_hidden(env) {
        return;
    }
    avatar_accessibility(renderer, None, &state.borrow().config, env);
}

/// Measures an [`Avatar`](waterui_controls::avatar::Avatar).
pub fn measure_avatar_node<E: TextEngine>(
    render_state: &AvatarRenderState<E>,
    _proposal: ProposalSize,
    _state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    let look = theme.avatar(env);
    let side = render_state
        .config
        .side()
        .unwrap_or_else(|| look.metrics().default_side())
        .max(0.0);
    ViewDimensions::new(LayoutSize::new(side, side))
}

fn centered_child_rect(inner_rect: Rect, child_size: LayoutSize) -> Rect {
    let child_w = f64::from(child_size.width).min(inner_rect.width()).max(0.0);
    let child_h = f64::from(child_size.height)
        .min(inner_rect.height())
        .max(0.0);
    let origin_x = (inner_rect.width() - child_w).mul_add(0.5, inner_rect.x0);
    let origin_y = (inner_rect.height() - child_h).mul_add(0.5, inner_rect.y0);
    Rect::new(origin_x, origin_y, origin_x + child_w, origin_y + child_h)
}

fn render_avatar_content<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<AvatarRenderState<E>>>,
    env: &Environment,
    look: &AvatarLook,
    inner_rect: Rect,
) {
    let (monogram, shape_kind, shape_commands, has_custom_fallback) = {
        let state_ref = state.borrow();
        (
            state_ref.config.monogram().cloned(),
            state_ref.shape_kind,
            state_ref.shape_commands.clone(),
            state_ref.fallback_view.is_some(),
        )
    };
    let inner_side = f64_as_f32(inner_rect.width().min(inner_rect.height())).max(0.0);
    let (_clip_path, scope_clip) =
        resolve_scope_clip_parts(shape_kind, &shape_commands, inner_rect);
    let bg_shape = Computed::constant(shape_data_for_parts(
        shape_kind,
        &shape_commands,
        inner_rect,
    ));
    let bg_signal = look.background_color().resolve(env);
    let bg_paint = Computed::new(EngineAnimated(bg_signal.map(Paint::Solid)));
    let scope_key = ScopeKey {
        role: "avatar-clip",
        item: 0,
    };
    let local = ctx.local;
    let child_ctx = ctx.render_context();
    let inner_proposal = bounded_proposal(inner_rect);

    {
        let renderer = ctx.renderer_mut();
        renderer.open_scope(scope_key, 1.0, local, &scope_clip, ScopeDelta::RECORD_SPACE);
        renderer
            .scene_mut()
            .live_fill_paint(local, bg_paint, bg_shape);
    }

    if has_custom_fallback {
        ctx.renderer_mut()
            .with_suppressed_accessibility(|renderer| {
                let mut state_ref = state.borrow_mut();
                if let Some(fallback_view) = state_ref.fallback_view.as_mut() {
                    let child_size = fallback_view.measure(renderer, env, inner_proposal);
                    let child_rect = centered_child_rect(inner_rect, child_size);
                    fallback_view.place(renderer, child_ctx, env, inner_proposal, child_rect, None);
                }
            });
    } else if let Some(monogram_signal) = &monogram {
        let text = ctx.renderer_mut().read_signal(monogram_signal);
        if !text.is_empty() {
            let font = look
                .monogram_font()
                .clone()
                .font_size(inner_side * look.metrics().initials_scale());
            let styled = StyledStr::plain(text.to_string())
                .font(font)
                .foreground(look.monogram_color().clone());
            ctx.render_styled_text_single_line_scaled(styled, env, inner_rect, None, None);
        }
    }

    #[cfg(waterui_feature = "image")]
    {
        let has_picture = state.borrow().picture_view.is_some();
        if has_picture {
            ctx.renderer_mut()
                .with_suppressed_accessibility(|renderer| {
                    let mut state_ref = state.borrow_mut();
                    if let Some(picture_view) = state_ref.picture_view.as_mut() {
                        picture_view.place(
                            renderer,
                            child_ctx,
                            env,
                            inner_proposal,
                            inner_rect,
                            None,
                        );
                    }
                });
        }
    }

    ctx.renderer_mut().close_scope();
}

fn render_avatar_ring<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    env: &Environment,
    bounds: Rect,
    outer_side: f32,
    shape_kind: ShapeKind,
    shape_commands: &[PathCommand],
    ring: &AvatarRing,
) {
    let ring_width = ring.width().max(0.0);
    if ring_width <= 0.0 {
        return;
    }
    let clamped_width = ring_width.min(outer_side);
    let half = f64::from(clamped_width) * 0.5;
    let stroke_rect = Rect::new(
        bounds.x0 + half,
        bounds.y0 + half,
        (bounds.x1 - half).max(bounds.x0 + half),
        (bounds.y1 - half).max(bounds.y0 + half),
    );
    let ring_shape = Computed::constant(shape_data_for_parts(
        shape_kind,
        shape_commands,
        stroke_rect,
    ));
    let color_signal = ring.color().resolve(env);
    let ring_paint = Computed::new(EngineAnimated(color_signal.map(Paint::Solid)));
    let local = ctx.local;
    ctx.renderer_mut().scene_mut().live_stroke_paint(
        Computed::constant(Stroke::new(f64::from(clamped_width))),
        local,
        ring_paint,
        ring_shape,
    );
}

/// Renders a retained [`Avatar`](waterui_controls::avatar::Avatar) leaf.
pub fn render_avatar_node<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<AvatarRenderState<E>>>,
    env: &Environment,
) {
    let hidden = ctx.renderer_mut().is_accessibility_hidden(env);
    if !hidden {
        let render_ctx = ctx.render_context();
        let state_ref = state.borrow();
        avatar_accessibility(ctx.renderer_mut(), Some(render_ctx), &state_ref.config, env);
    }

    let bounds = ctx.bounds;
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }

    let look = ctx.theme().avatar(env);
    let (shape_kind, shape_commands, ring) = {
        let state_ref = state.borrow();
        (
            state_ref.shape_kind,
            state_ref.shape_commands.clone(),
            state_ref.config.ring().cloned(),
        )
    };

    let outer_side = f64_as_f32(bounds.width().min(bounds.height())).max(0.0);
    let ring_width = ring.as_ref().map_or(0.0, |ring| ring.width().max(0.0));
    let inset = f64::from(ring_width);
    let inner_rect = Rect::new(
        bounds.x0 + inset,
        bounds.y0 + inset,
        (bounds.x1 - inset).max(bounds.x0 + inset),
        (bounds.y1 - inset).max(bounds.y0 + inset),
    );

    if inner_rect.width() > 0.0 && inner_rect.height() > 0.0 {
        render_avatar_content(ctx, state, env, &look, inner_rect);
    }

    if let Some(ring) = ring.as_ref() {
        render_avatar_ring(
            ctx,
            env,
            bounds,
            outer_side,
            shape_kind,
            &shape_commands,
            ring,
        );
    }
}
