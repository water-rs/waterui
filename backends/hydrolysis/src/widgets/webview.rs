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
    HydroNativeView, HydroState, WidgetRenderContext, measure_view_dimensions_with_proposal,
    measure_view_intrinsic, normalize_view_for_render,
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
        AnyView::new(vstack((
            zstack((
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
            )),
        ))),
        env,
    )
}

impl HydroNativeView for WebView {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let surface_label = webview_surface_label(env);
        let mut render_env = env.clone();
        render_env.remove::<AccessibilityLabel>();
        render_env.remove::<AccessibilityRole>();
        let content = webview_content(&view, &surface_label, &render_env);
        ctx.dispatch_in_rect(&render_env, content, ctx.bounds);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let surface_label = webview_surface_label(env);
        let mut render_env = env.clone();
        render_env.remove::<AccessibilityLabel>();
        render_env.remove::<AccessibilityRole>();
        measure_view_intrinsic(&webview_content(view, &surface_label, &render_env), state, &render_env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let surface_label = webview_surface_label(env);
        let mut render_env = env.clone();
        render_env.remove::<AccessibilityLabel>();
        render_env.remove::<AccessibilityRole>();
        measure_view_dimensions_with_proposal(
            &webview_content(view, &surface_label, &render_env),
            proposal,
            state,
            &render_env,
        )
    }

    fn accessibility_is_render_driven() -> bool {
        true
    }
}
