use super::HydrolysisRenderer;
use crate::engine::vello_backend::VelloDrawContext;
use crate::renderer::HydroState;
use crate::renderer::navigation::{
    NavigationTransitionDirection, NavigationTransitionFrame, draw_navigation_transition,
};
use waterui::navigation::NavigationTransition;
use waterui_core::layout::HorizontalAlignment;
use waterui_core::{AnyView, Environment};
use waterui_text::styled::StyledStr;

/// Render context passed to handlers.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    pub transform: vello::kurbo::Affine,
    pub hit_transform: vello::kurbo::Affine,
    pub bounds: vello::kurbo::Rect,
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
    pub transform: vello::kurbo::Affine,
    pub hit_transform: vello::kurbo::Affine,
    pub bounds: vello::kurbo::Rect,
}

impl RenderContext {
    pub(crate) fn with_transforms(
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) -> Self {
        Self {
            transform,
            hit_transform,
            bounds,
        }
    }

    #[must_use]
    pub fn child(&self, transform: vello::kurbo::Affine, bounds: vello::kurbo::Rect) -> Self {
        Self {
            transform: self.transform * transform,
            hit_transform: self.hit_transform * transform,
            bounds,
        }
    }

    #[must_use]
    pub(crate) fn with_identity_transforms(&self, bounds: vello::kurbo::Rect) -> Self {
        Self {
            transform: vello::kurbo::Affine::IDENTITY,
            hit_transform: vello::kurbo::Affine::IDENTITY,
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

    pub(crate) fn child(
        &self,
        transform: vello::kurbo::Affine,
        bounds: vello::kurbo::Rect,
    ) -> RenderContext {
        self.render_context().child(transform, bounds)
    }

    pub(crate) fn with_identity_transforms(&self, bounds: vello::kurbo::Rect) -> RenderContext {
        self.render_context().with_identity_transforms(bounds)
    }

    pub(crate) fn renderer_mut(&mut self) -> &mut HydrolysisRenderer {
        self.renderer
    }

    pub(crate) fn draw_context(&mut self) -> VelloDrawContext<'_> {
        self.renderer.draw_context(self.render_context())
    }

    pub(crate) fn state_mut(&mut self) -> &mut HydroState {
        &mut self.renderer.state
    }

    pub(crate) fn push_layer_rect(&mut self, alpha: f32, clip: vello::kurbo::Rect) {
        self.renderer.push_layer_rect(alpha, self.transform, clip);
    }

    pub(crate) fn pop_layer(&mut self) {
        self.renderer.pop_layer();
    }

    pub(crate) fn render_styled_text(
        &mut self,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        bounds: vello::kurbo::Rect,
    ) {
        self.render_styled_text_limited(styled, alignment, env, bounds, None);
    }

    pub(crate) fn render_styled_text_limited(
        &mut self,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        bounds: vello::kurbo::Rect,
        max_lines: Option<usize>,
    ) {
        let child_ctx = self.child(
            vello::kurbo::Affine::translate((bounds.x0, bounds.y0)),
            vello::kurbo::Rect::new(0.0, 0.0, bounds.width(), bounds.height()),
        );
        let renderer = self.renderer_mut();
        let (state, scene) = renderer.state_and_scene_mut();
        HydrolysisRenderer::render_styled_text_limited(
            state, scene, child_ctx, styled, alignment, env, max_lines,
        );
    }

    pub(crate) fn append_scene(&mut self, scene: &vello::Scene) {
        self.renderer
            .scene_mut()
            .append(scene, Some(self.transform));
    }

    pub(crate) fn draw_navigation_transition(
        &mut self,
        style: NavigationTransition,
        direction: NavigationTransitionDirection,
        progress: f64,
        pushpop_parallax_factor: f64,
        from_scene: &vello::Scene,
        to_scene: &vello::Scene,
    ) {
        draw_navigation_transition(NavigationTransitionFrame {
            scene: self.renderer.scene_mut(),
            transform: self.transform,
            bounds: self.bounds,
            style,
            direction,
            progress,
            pushpop_parallax_factor,
            from_scene,
            to_scene,
        });
    }

    pub(crate) fn dispatch_in_rect(
        &mut self,
        env: &Environment,
        view: AnyView,
        rect: vello::kurbo::Rect,
    ) {
        HydrolysisRenderer::dispatch_in_rect(self.renderer, self.render_context(), env, view, rect);
    }

    pub(crate) fn dispatch_in_rect_without_accessibility(
        &mut self,
        env: &Environment,
        view: AnyView,
        rect: vello::kurbo::Rect,
    ) {
        HydrolysisRenderer::dispatch_in_rect_without_accessibility(
            self.renderer,
            self.render_context(),
            env,
            view,
            rect,
        );
    }
}
