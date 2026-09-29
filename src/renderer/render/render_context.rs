use super::{HydrolysisRenderer, Recording, SceneDrawContext, TailMark};

use crate::renderer::HydroState;
use crate::renderer::frame::LayerTransforms;
use crate::renderer::navigation::{
    NavigationCapturedScene, NavigationTransitionFrame, draw_navigation_transition,
};
use waterui::navigation::{AnyNavigationTransition, NavigationTransitionDirection};
use waterui_backend_core::widget::NavigationMotion;
use waterui_core::Environment;
use waterui_core::layout::HorizontalAlignment;
use waterui_text::styled::StyledStr;

/// Render context passed to handlers.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    pub transform: kurbo::Affine,
    pub hit_transform: kurbo::Affine,
    pub bounds: kurbo::Rect,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HydrolysisWindowOrigin {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HydrolysisTextContextMenuMode {
    Overlay,
    NativeWindow,
}

pub(crate) struct WidgetRenderContext<'a> {
    renderer: &'a mut HydrolysisRenderer,
    pub transform: kurbo::Affine,
    pub hit_transform: kurbo::Affine,
    pub bounds: kurbo::Rect,
}

/// An explicit offer from a native widget-owned content region.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn bounded_proposal(bounds: kurbo::Rect) -> waterui_core::layout::ProposalSize {
    waterui_core::layout::ProposalSize::new(
        Some(bounds.width() as f32),
        Some(bounds.height() as f32),
    )
}

impl RenderContext {
    pub(crate) fn with_transforms(
        bounds: kurbo::Rect,
        transform: kurbo::Affine,
        hit_transform: kurbo::Affine,
    ) -> Self {
        Self {
            transform,
            hit_transform,
            bounds,
        }
    }

    #[must_use]
    pub fn child(&self, transform: kurbo::Affine, bounds: kurbo::Rect) -> Self {
        Self {
            transform: self.transform * transform,
            hit_transform: self.hit_transform * transform,
            bounds,
        }
    }

    #[must_use]
    pub(crate) fn with_identity_transforms(&self, bounds: kurbo::Rect) -> Self {
        Self {
            transform: kurbo::Affine::IDENTITY,
            hit_transform: kurbo::Affine::IDENTITY,
            bounds,
        }
    }
}

impl<'a> WidgetRenderContext<'a> {
    pub(crate) fn new(renderer: &'a mut HydrolysisRenderer, ctx: RenderContext) -> Self {
        Self {
            renderer,
            transform: ctx.transform,
            hit_transform: ctx.hit_transform,
            bounds: ctx.bounds,
        }
    }

    pub(crate) fn render_context(&self) -> RenderContext {
        RenderContext::with_transforms(self.bounds, self.transform, self.hit_transform)
    }

    /// The renderer-owned widget theme, cloned out as an `Rc` so callers can
    /// hold it without borrowing the context across a `&mut` renderer call.
    pub(crate) fn theme(&self) -> std::rc::Rc<dyn crate::engine::WidgetTheme> {
        self.renderer.theme()
    }

    pub(crate) fn child(&self, transform: kurbo::Affine, bounds: kurbo::Rect) -> RenderContext {
        self.render_context().child(transform, bounds)
    }

    pub(crate) fn renderer_mut(&mut self) -> &mut HydrolysisRenderer {
        self.renderer
    }

    pub(crate) fn draw_context(&mut self) -> SceneDrawContext<'_> {
        self.renderer.draw_context(self.render_context())
    }

    pub(crate) fn state_mut(&mut self) -> &mut HydroState {
        &mut self.renderer.state
    }

    pub(crate) fn push_layer_rect(&mut self, alpha: f32, clip: kurbo::Rect) {
        self.renderer.push_layer_rect(
            alpha,
            LayerTransforms {
                paint: self.transform,
                hit: self.hit_transform,
            },
            clip,
        );
    }

    pub(crate) fn pop_layer(&mut self) {
        self.renderer.pop_layer();
    }

    /// The [`HydrolysisRenderer::with_clip_rect_scope`] pairing through this
    /// context's transforms.
    pub(crate) fn with_clip_rect_scope(
        &mut self,
        alpha: f32,
        clip: kurbo::Rect,
        f: impl FnOnce(&mut Self),
    ) {
        self.push_layer_rect(alpha, clip);
        f(self);
        self.pop_layer();
    }

    /// [`Self::with_clip_rect_scope`] when the scope only exists conditionally
    /// (a disabled-control alpha group, a viewport clip that only out-scrolls
    /// need): pairing stays lexical either way.
    pub(crate) fn with_clip_rect_scope_if(
        &mut self,
        enabled: bool,
        alpha: f32,
        clip: kurbo::Rect,
        f: impl FnOnce(&mut Self),
    ) {
        if enabled {
            self.push_layer_rect(alpha, clip);
        }
        f(self);
        if enabled {
            self.pop_layer();
        }
    }

    pub(crate) fn render_styled_text(
        &mut self,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        bounds: kurbo::Rect,
    ) {
        self.render_styled_text_limited(styled, alignment, env, bounds, None);
    }

    pub(crate) fn render_styled_text_limited(
        &mut self,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        bounds: kurbo::Rect,
        max_lines: Option<usize>,
    ) {
        let child_ctx = self.child(
            kurbo::Affine::translate((bounds.x0, bounds.y0)),
            kurbo::Rect::new(0.0, 0.0, bounds.width(), bounds.height()),
        );
        let renderer = self.renderer_mut();
        let (state, scene) = renderer.state_and_scene_mut();
        HydrolysisRenderer::render_styled_text_limited(
            state,
            scene,
            child_ctx,
            styled,
            alignment,
            env,
            max_lines.map_or(TailMark::None, TailMark::Clip),
        );
    }

    pub(crate) fn render_styled_text_single_line_centered(
        &mut self,
        styled: StyledStr,
        env: &Environment,
        bounds: kurbo::Rect,
    ) {
        let child_ctx = self.child(
            kurbo::Affine::translate((bounds.x0, bounds.y0)),
            kurbo::Rect::new(0.0, 0.0, bounds.width(), bounds.height()),
        );
        let renderer = self.renderer_mut();
        let (state, scene) = renderer.state_and_scene_mut();
        HydrolysisRenderer::render_styled_text_single_line_centered(
            state, scene, child_ctx, styled, env,
        );
    }

    pub(crate) fn append_scene(&mut self, scene: &Recording) {
        self.renderer.scene_mut().append(scene, self.transform);
    }

    pub(crate) fn draw_navigation_transition(
        &mut self,
        style: AnyNavigationTransition,
        motion: NavigationMotion,
        direction: NavigationTransitionDirection,
        progress: f64,
        from_scene: &NavigationCapturedScene,
        to_scene: &NavigationCapturedScene,
    ) {
        draw_navigation_transition(NavigationTransitionFrame {
            scene: self.renderer.scene_mut(),
            transform: self.transform,
            bounds: self.bounds,
            style,
            motion,
            direction,
            progress,
            from_scene,
            to_scene,
        });
    }
}
