use super::{CapturedLayers, HydrolysisRenderer, TailMark};

use crate::renderer::HydroState;
use crate::renderer::SafeAreaLayout;
use crate::renderer::frame::LayerTransforms;
use crate::renderer::grow_rect;
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

    /// The widget's window-space frame of a `rect` it places in its own
    /// bounds space: the widget's layout-recorded frame shifted by the
    /// rect's offset inside this context's bounds — the f64 counterpart of
    /// `SafeAreaLayout::child`. `SafeAreaLayout::frame` is the layout fact
    /// recorded at layout, so touches and extensions derive from the
    /// laid-out position "before scroll offsets and visual transforms"
    /// (§7.1): `transform` is never consulted — it is in device pixels and
    /// carries visual transforms that must not move a hosted frame.
    fn hosted_frame(&self, area: &SafeAreaLayout, rect: kurbo::Rect) -> kurbo::Rect {
        area.hosted_frame(self.bounds, rect)
    }

    /// The §7.1 context for a retained sub-view the widget places at `rect`
    /// in its own bounds space — a bar label, a badge's content: the
    /// ambient boundaries, with the sub-view's frame recorded at its
    /// window-space position.
    pub(crate) fn safe_area_for(&self, rect: kurbo::Rect) -> Option<SafeAreaLayout> {
        self.safe_area
            .as_ref()
            .map(|area| area.with_frame(self.hosted_frame(area, rect)))
    }

    /// The §7.1 context for retained *content* a chrome container places at
    /// `rect` in its bounds space — stack pages, split panes, `NavigationView`
    /// content and `Tabs` content: the hosted content inherits the widget's
    /// boundaries and released regions; only the edges whose frame edge still
    /// touches a boundary stay reachable, so a scroll surface inside still
    /// extends and `.ignore_safe_area` inside still releases on the edges the
    /// chrome left on the boundary (§7.1).
    pub(crate) fn content_area_for(&self, rect: kurbo::Rect) -> Option<SafeAreaLayout> {
        self.safe_area
            .as_ref()
            .map(|area| area.hosted(self.hosted_frame(area, rect)))
    }

    /// `self.bounds` grown by the touched-edge offsets of the area its
    /// content paints — the reach a transition clip and a stack backdrop
    /// fill must both cover so an extended bar surface is never clipped and
    /// never lands on the window background. `self.bounds` itself stays
    /// the layout reference.
    pub(crate) fn chrome_paint_bounds(&self) -> kurbo::Rect {
        grow_rect(
            self.bounds,
            self.content_area_for(self.bounds)
                .map(|area| area.touched_edge_offsets())
                .unwrap_or_default(),
        )
    }

    /// §7.1's chrome split over every `(edge, extent)` in `bars` for
    /// this widget's [`bounds`](Self::bounds), derived in one call: each
    /// bar's [`ChromeBar`] — band, context, surface and per-rect area
    /// bound together — plus the one content rect `bounds` leaves once
    /// every band is carved and the one composed content context — see
    /// [`SafeAreaLayout::chrome_splits`]. A widget with no safe-area
    /// context keeps `bounds`' own edges, the placement every
    /// context-free chrome container gets: the bars come back
    /// [`ChromeBar::contextless`] and `content_area` stays `None`.
    pub(crate) fn chrome_splits<const N: usize>(
        &self,
        bars: [(crate::renderer::Edge, f64); N],
    ) -> ChromeGroup<N> {
        self.safe_area.as_ref().map_or_else(
            || {
                let bars = bars.map(|(edge, extent)| {
                    crate::renderer::ChromeBar::contextless(self.bounds, edge, extent)
                });
                ChromeGroup {
                    content: bars
                        .iter()
                        .fold(self.bounds, |content, bar| content.intersect(bar.rest)),
                    bars,
                    content_area: None,
                }
            },
            |area| {
                let splits = area.chrome_splits(self.bounds, bars);
                ChromeGroup {
                    bars: splits.bars,
                    content: splits.content,
                    content_area: Some(splits.content_area),
                }
            },
        )
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
        // The transition's page clips and the stack's backdrop fill cover
        // the same reach — `chrome_paint_bounds` — so an extended bar
        // surface is never clipped mid-animation. `bounds` stays the
        // scale-centre reference.
        let paint_bounds = self.chrome_paint_bounds();
        draw_navigation_transition(NavigationTransitionFrame {
            renderer: self.renderer,
            transforms: LayerTransforms {
                paint: self.transform,
                hit: self.hit_transform,
            },
            bounds: self.bounds,
            paint_bounds,
            style,
            motion,
            direction,
            progress,
            from_scene,
            to_scene,
        });
    }
}

/// The pieces of [`WidgetRenderContext::chrome_splits`]: each bar's
/// [`ChromeBar`] — its band and its context bound together, so a band is
/// never paired with another bar's context — plus the one content rect
/// and composed content context; a context-free widget's bars come back
/// [`ChromeBar::contextless`] and `content_area` `None`.
pub struct ChromeGroup<const N: usize> {
    /// Each bar's share of the split, in the order the edges were given.
    pub bars: [crate::renderer::ChromeBar; N],
    /// The remainder of `bounds` outside every band — always inside
    /// `bounds`, clear of both regions.
    pub content: kurbo::Rect,
    /// The hosted content's context: `hosted` plus every bar's `Docked`
    /// boundary — nothing inside touches, releases or extends through an
    /// edge a bar sits on, and a nested bar whose frame lands on a dock
    /// edge stacks on the outer bar.
    pub content_area: Option<SafeAreaLayout>,
}
