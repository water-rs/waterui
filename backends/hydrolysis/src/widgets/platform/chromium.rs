use std::cell::RefCell;
use std::rc::Rc;

use waterui_chromium::{ChromiumView, PageMode};
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};

use crate::renderer::{HydroNativeView, HydroState, HydrolysisRenderer, WidgetRenderContext};
use crate::widgets::platform::browser_cef::CefSurfaceRenderState;

pub(crate) struct ChromiumRenderState {
    _source: ChromiumView,
    cef: CefSurfaceRenderState,
}

impl ChromiumRenderState {
    pub(crate) fn from_view(view: ChromiumView, env: &Environment) -> Self {
        assert_eq!(
            view.page().mode(),
            PageMode::Visible,
            "headless Chromium pages cannot be rendered as ChromiumView"
        );
        let page = view
            .page()
            .handle()
            .downcast_ref::<waterui_browser_cef::CefPageHandle>()
            .unwrap_or_else(|| {
                panic!("Hydrolysis ChromiumView handle does not use the selected CEF runtime")
            })
            .clone();
        Self {
            _source: view,
            cef: CefSurfaceRenderState::new(page, env),
        }
    }

    pub(crate) fn prebuild(&self, renderer: &mut HydrolysisRenderer) {
        self.cef.prebuild(renderer);
    }
}

impl HydroNativeView for ChromiumView {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let _ = (state, view, env);
        LayoutSize::zero()
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let _ = (state, view, env);
        ViewDimensions::new(LayoutSize::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }
}

pub(crate) fn measure_chromium_node(
    state: &ChromiumRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let _ = (state, hydro, env);
    ViewDimensions::new(LayoutSize::new(
        proposal.width.unwrap_or(0.0),
        proposal.height.unwrap_or(0.0),
    ))
}

pub(crate) fn render_chromium_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ChromiumRenderState>>,
    env: &Environment,
) {
    // CEF rasterizes the page into a texture the host tree cannot see into, so
    // without this node the whole region is missing from the accessibility tree
    // -- the same reason `WebView` publishes one, and the same node.
    super::register_web_surface_accessibility(ctx, env);
    let bounds = ctx.bounds;
    let transform = ctx.render_context().transform;
    let hit_transform = ctx.hit_transform;
    state
        .borrow()
        .cef
        .render(ctx.renderer_mut(), bounds, transform, hit_transform);
}
