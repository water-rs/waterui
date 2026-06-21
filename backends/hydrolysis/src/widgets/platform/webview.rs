use std::cell::RefCell;
use std::rc::Rc;

use nami::SignalExt;
use waterui::ViewExt as _;
use waterui::accessibility::{AccessibilityLabel, AccessibilityRole};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment};
use waterui_graphics::Gradient;
use waterui_graphics::color::Srgb;
use waterui_layout::stack::{vstack, zstack};
use waterui_text::Text;
use waterui_webview::{WebView, WebViewEvent};

use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RetainedSubview, WidgetRenderContext,
    measure_view_dimensions_with_proposal, measure_view_intrinsic, normalize_view_for_render,
};

const WEBVIEW_SURFACE_HEIGHT: f32 = 184.0;

fn webview_surface_label(env: &Environment) -> String {
    env.get::<AccessibilityLabel>()
        .map(|label| label.as_str().as_str().to_owned())
        .unwrap_or_else(|| String::from("WebView"))
}

fn webview_status_text(event: WebViewEvent) -> String {
    match event {
        WebViewEvent::None => String::from("Status: Idle"),
        WebViewEvent::WillNavigate { url } => format!("Status: Navigating to {url}"),
        WebViewEvent::Loading { progress } => {
            format!("Status: Loading {:.0}%", f64::from(progress) * 100.0)
        }
        WebViewEvent::Loaded => String::from("Status: Loaded"),
        WebViewEvent::Redirect { from, to } => format!("Status: Redirect {from} -> {to}"),
        WebViewEvent::Error(error) => format!("Status: Error {error}"),
        WebViewEvent::StateChanged { .. } => unreachable!(
            "WebView::event() should filter internal StateChanged updates before Hydrolysis render"
        ),
    }
}

fn webview_content(view: &WebView, surface_label: &str, env: &Environment) -> AnyView {
    let status = Text::display(view.event().map(webview_status_text));
    let navigation = Text::display(
        view.can_go_back()
            .zip(&view.can_go_forward())
            .map(|(back, forward)| format!("Navigation: back={back} forward={forward}")),
    );

    normalize_view_for_render(
        AnyView::new(vstack((zstack((
            Gradient::linear(
                vec![
                    (0.0, Srgb::new(0.12, 0.14, 0.20).resolve()),
                    (1.0, Srgb::new(0.22, 0.29, 0.40).resolve()),
                ],
                [0.0, 0.0],
                [1.0, 1.0],
            )
            .height(WEBVIEW_SURFACE_HEIGHT)
            .a11y_role(AccessibilityRole::Group)
            .a11y_label(surface_label.to_owned()),
            vstack((Text::new("Hydrolysis WebView"), status, navigation))
                .spacing(8.0)
                .padding_with(16.0),
        )),))),
        env,
    )
}

/// Environment with the webview's own accessibility label/role removed, so the
/// inner content's render-driven a11y emits the surface label rather than inheriting
/// the outer wrapper's. The webview a11y is render-driven, so the inner content owns it.
fn webview_render_env(env: &Environment) -> Environment {
    let mut render_env = env.clone();
    render_env.remove::<AccessibilityLabel>();
    render_env.remove::<AccessibilityRole>();
    render_env
}

/// The retained render state of a webview: the composed content (a `vstack` of a
/// gradient surface plus reactive status/navigation `Text`s built from the view's
/// `event`/`can_go_back`/`can_go_forward` signals) is a move-only `AnyView`, so the
/// persistent `Widget` node holds it as a [`RetainedSubview`] built once and
/// re-flushed each frame. The status/navigation reactivity is carried by the inner
/// `Text` nodes, which become live `Dynamic`/`Text` nodes inside the sub-view — a
/// navigation or load event updates without rebuilding the node.
pub(crate) struct WebViewRenderState {
    /// The scoped environment the content was built under (webview a11y label/role
    /// removed), reused at flush so the inner render-driven a11y matches the build.
    render_env: Environment,
    content: RetainedSubview,
}

impl WebViewRenderState {
    pub(crate) fn from_view(view: &WebView, env: &Environment) -> Self {
        let render_env = webview_render_env(env);
        let surface_label = webview_surface_label(env);
        let content = webview_content(view, &surface_label, &render_env);
        Self {
            render_env,
            content: RetainedSubview::new(content),
        }
    }

    /// Eagerly build the content sub-view (the measure path has only `&mut
    /// HydroState`, no renderer, so it must be built before then).
    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        let _ = env;
        let render_env = self.render_env.clone();
        self.content.ensure_built(renderer, &render_env);
    }
}

impl HydroNativeView for WebView {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let render_env = webview_render_env(env);
        let surface_label = webview_surface_label(env);
        measure_view_intrinsic(
            &webview_content(view, &surface_label, &render_env),
            state,
            &render_env,
        )
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let render_env = webview_render_env(env);
        let surface_label = webview_surface_label(env);
        measure_view_dimensions_with_proposal(
            &webview_content(view, &surface_label, &render_env),
            proposal,
            state,
            &render_env,
        )
    }
}

/// Measures a retained webview leaf from its [`WebViewRenderState`]: the webview
/// sizes itself to its composed content, mirroring the dispatch path's `dimensions`.
pub(crate) fn measure_webview_node(
    state: &WebViewRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    let render_env = state.render_env.clone();
    ViewDimensions::new(
        state
            .content
            .measure_built_with_proposal(hydro, &render_env, proposal),
    )
}

/// Renders a retained webview leaf every flush: flushes the composed content
/// sub-view at the node's bounds. The content's own dispatch drives accessibility
/// (webview a11y is render-driven) and its inner reactive `Text` nodes stay live.
pub(crate) fn render_webview_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<WebViewRenderState>>,
    env: &Environment,
) {
    render_webview_parts(ctx, state, env);
}

pub(crate) fn render_webview_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<WebViewRenderState>>,
    _env: &Environment,
) {
    let bounds = ctx.bounds;
    let render_ctx = ctx.render_context();
    let mut state = state.borrow_mut();
    let render_env = state.render_env.clone();
    state
        .content
        .flush_in_rect(ctx.renderer_mut(), render_ctx, &render_env, bounds);
}
