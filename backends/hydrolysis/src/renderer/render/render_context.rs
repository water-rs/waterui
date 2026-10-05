use super::{CapturedLayers, HydrolysisRenderer, TailMark};

use crate::renderer::HydroState;
use crate::renderer::frame::LayerTransforms;
use crate::renderer::navigation::{
    NavigationCapturedScene, NavigationTransitionFrame, draw_navigation_transition,
};
use crate::renderer::tree::SafeAreaLayout;
use waterui::navigation::{AnyNavigationTransition, NavigationTransitionDirection};
use waterui_backend_core::widget::NavigationMotion;
use waterui_core::Environment;
use waterui_core::layout::HorizontalAlignment;
use waterui_text::styled::StyledStr;

/// Render context passed to handlers.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    /// The transform placing the widget in the scene.
    pub transform: kurbo::Affine,
    /// The transform used to resolve hit tests inside the widget.
    pub hit_transform: kurbo::Affine,
    /// The widget's bounds in scene coordinates.
    pub bounds: kurbo::Rect,
}

#[derive(Debug, Clone, Copy)]
pub struct HydrolysisWindowOrigin {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrolysisTextContextMenuMode {
    Overlay,
    NativeWindow,
}

pub struct WidgetRenderContext<'a> {
    renderer: &'a mut HydrolysisRenderer,
    pub transform: kurbo::Affine,
    pub hit_transform: kurbo::Affine,
    pub bounds: kurbo::Rect,
    /// The §7.1 context this widget was laid out against — `None` inside a
    /// scroll surface's context-free content. Retained sub-views the handler
    /// flushes derive theirs from it through [`Self::safe_area_for`] and
    /// [`Self::content_area_for`].
    safe_area: Option<SafeAreaLayout>,
}

/// An explicit offer from a native widget-owned content region.
#[allow(clippy::cast_possible_truncation)]
pub fn bounded_proposal(bounds: kurbo::Rect) -> waterui_core::layout::ProposalSize {
    waterui_core::layout::ProposalSize::new(
        Some(bounds.width() as f32),
        Some(bounds.height() as f32),
    )
}

impl RenderContext {
    pub(crate) const fn with_transforms(
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
    /// The context for a child widget at `transform`/`bounds`, composed with this one.
    pub fn child(&self, transform: kurbo::Affine, bounds: kurbo::Rect) -> Self {
        Self {
            transform: self.transform * transform,
            hit_transform: self.hit_transform * transform,
            bounds,
        }
    }
}

impl<'a> WidgetRenderContext<'a> {
    pub(crate) const fn new(
        renderer: &'a mut HydrolysisRenderer,
        ctx: RenderContext,
        safe_area: Option<SafeAreaLayout>,
    ) -> Self {
        Self {
            renderer,
            transform: ctx.transform,
            hit_transform: ctx.hit_transform,
            bounds: ctx.bounds,
            safe_area,
        }
    }

    /// The §7.1 context for a retained sub-view the widget places at `rect`
    /// in its own bounds space — a bar label, a popup's content: the ambient
    /// boundaries, with the sub-view's frame recorded where the flush puts it
    /// — `self.transform` maps the widget's bounds space to window space, so
    /// the frame the context stores is window-space like the boundaries are.
    pub(crate) fn safe_area_for(&self, rect: kurbo::Rect) -> Option<SafeAreaLayout> {
        self.safe_area
            .as_ref()
            .map(|area| area.with_frame(self.transform.transform_rect_bbox(rect)))
    }

    /// The §7.1 context for retained *content* a chrome container places at
    /// `rect` in its bounds space — `NavigationView`/`Tabs` content, a
    /// popup's content: the band the chrome itself consumed is gone from
    /// the boundaries (they reseed from the frame's own edges), so content's
    /// `.ignore_safe_area` still reaches the window edge and its scroll
    /// surfaces still extend and clear focused fields (§7.1).
    pub(crate) fn content_area_for(&self, rect: kurbo::Rect) -> Option<SafeAreaLayout> {
        self.safe_area
            .as_ref()
            .map(|area| area.for_subtree(self.transform.transform_rect_bbox(rect)))
    }

    pub(crate) const fn render_context(&self) -> RenderContext {
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

    pub(crate) const fn renderer_mut(&mut self) -> &mut HydrolysisRenderer {
        self.renderer
    }

    pub(crate) fn draw_context(
        &mut self,
        body: impl FnOnce(&mut waterui_graphics::draw::Recorder),
    ) {
        let ctx = self.render_context();
        self.renderer.draw_context(ctx, body);
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

    /// Presents captured layers at this context's transform.
    pub(crate) fn present_layers(&mut self, layers: &CapturedLayers) {
        self.renderer.present_layers(layers, self.transform);
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
            renderer: self.renderer,
            transforms: LayerTransforms {
                paint: self.transform,
                hit: self.hit_transform,
            },
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
