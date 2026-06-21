use std::cell::RefCell;
use std::rc::Rc;

use nami::SignalExt;
use waterui::ViewExt as _;
use waterui::accessibility::{AccessibilityLabel, AccessibilityRole};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::Gradient;
use waterui_graphics::color::Srgb;
use waterui_layout::stack::{vstack, zstack};
use waterui_map::{Annotation, MapConfig, MapStyle, Region};
use waterui_text::Text;

use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RetainedSubview, WidgetRenderContext,
    measure_view_dimensions_with_proposal, measure_view_intrinsic, normalize_view_for_render,
};

const MAP_SURFACE_HEIGHT: f32 = 184.0;

fn map_surface_label(env: &Environment) -> String {
    env.get::<AccessibilityLabel>()
        .map(|label| label.as_str().as_str().to_owned())
        .unwrap_or_else(|| String::from("Map viewport"))
}

fn map_style_name(style: MapStyle) -> &'static str {
    match style {
        MapStyle::Standard => "Standard",
        MapStyle::Satellite => "Satellite",
        MapStyle::Hybrid => "Hybrid",
    }
}

fn map_region_summary(region: Region) -> String {
    format!(
        "Center: {:.4}, {:.4} span {:.3} x {:.3}",
        region.center.latitude,
        region.center.longitude,
        region.latitude_delta,
        region.longitude_delta
    )
}

fn annotation_summary(annotation: Annotation) -> String {
    let title = annotation.title.as_str();
    match annotation.subtitle {
        Some(subtitle) => format!("{title} ({subtitle})"),
        None => title.to_owned(),
    }
}

fn annotation_list_text(annotations: Vec<Annotation>) -> String {
    if annotations.is_empty() {
        return String::from("Annotations: none");
    }
    let joined = annotations
        .into_iter()
        .map(annotation_summary)
        .collect::<Vec<_>>()
        .join(" | ");
    format!("Annotations: {joined}")
}

fn map_content(config: &MapConfig, surface_label: &str, env: &Environment) -> AnyView {
    let style = map_style_name(config.style);
    let shows_location = if config.user_location_visibility.is_visible() {
        "User location on"
    } else {
        "User location off"
    };
    let interactivity = if config.interactivity.is_interactive() {
        "Interactive"
    } else {
        "Read only"
    };
    let controls = format!(
        "Compass: {}  Scale: {}",
        if config.compass_visibility.is_visible() {
            "on"
        } else {
            "off"
        },
        if config.scale_visibility.is_visible() {
            "on"
        } else {
            "off"
        }
    );

    let region_summary = Text::display(config.region.map(map_region_summary));
    let annotation_count = Text::display(
        config
            .annotations
            .clone()
            .map(|annotations: Vec<Annotation>| format!("Pins: {}", annotations.len())),
    );
    let annotation_list = Text::display(config.annotations.clone().map(annotation_list_text));

    normalize_view_for_render(
        AnyView::new(
            vstack((
                zstack((
                    Gradient::linear(
                        vec![
                            (0.0, Srgb::new(0.10, 0.42, 0.72).resolve()),
                            (0.5, Srgb::new(0.19, 0.60, 0.35).resolve()),
                            (1.0, Srgb::new(0.85, 0.91, 0.95).resolve()),
                        ],
                        [0.0, 0.0],
                        [1.0, 1.0],
                    )
                    .height(MAP_SURFACE_HEIGHT)
                    .a11y_role(AccessibilityRole::Image)
                    .a11y_label(surface_label.to_owned()),
                    vstack((
                        Text::new(style),
                        Text::new(interactivity),
                        Text::new(shows_location),
                        Text::new(controls),
                    ))
                    .spacing(6.0)
                    .padding_with(16.0),
                )),
                region_summary,
                annotation_count,
                annotation_list,
            ))
            .spacing(8.0),
        ),
        env,
    )
}

/// Environment with the map's own accessibility label/role removed, so the inner
/// content's render-driven a11y emits the surface label rather than inheriting the
/// outer wrapper's. The map a11y is render-driven, so the inner content owns it.
fn map_render_env(env: &Environment) -> Environment {
    let mut render_env = env.clone();
    render_env.remove::<AccessibilityLabel>();
    render_env.remove::<AccessibilityRole>();
    render_env
}

/// The retained render state of a map: the composed content (a `vstack` of a
/// gradient surface plus reactive region/annotation `Text`s built from the config's
/// `Computed` signals) is a move-only `AnyView`, so the persistent `Widget` node
/// holds it as a [`RetainedSubview`] built once and re-flushed each frame. The
/// region/annotation reactivity is carried by the inner `Text` nodes
/// (`config.region.map(...)`), which become live `Dynamic`/`Text` nodes inside the
/// sub-view — a region or annotation change updates without rebuilding the node.
pub(crate) struct MapRenderState {
    /// The scoped environment the content was built under (map a11y label/role
    /// removed), reused at flush so the inner render-driven a11y matches the build.
    render_env: Environment,
    content: RetainedSubview,
}

impl MapRenderState {
    pub(crate) fn from_config(config: MapConfig, env: &Environment) -> Self {
        let render_env = map_render_env(env);
        let surface_label = map_surface_label(env);
        let content = map_content(&config, &surface_label, &render_env);
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

impl HydroNativeView for Native<MapConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let render_env = map_render_env(env);
        let surface_label = map_surface_label(env);
        measure_view_intrinsic(
            &map_content(view.as_inner(), &surface_label, &render_env),
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
        let render_env = map_render_env(env);
        let surface_label = map_surface_label(env);
        measure_view_dimensions_with_proposal(
            &map_content(view.as_inner(), &surface_label, &render_env),
            proposal,
            state,
            &render_env,
        )
    }
}

/// Measures a retained map leaf from its [`MapRenderState`]: the map sizes itself
/// to its composed content, mirroring the dispatch path's `dimensions`.
pub(crate) fn measure_map_node(
    state: &MapRenderState,
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

/// Renders a retained map leaf every flush: flushes the composed content sub-view
/// at the node's bounds. The content's own dispatch drives accessibility (map a11y
/// is render-driven) and its inner reactive `Text` nodes stay live.
pub(crate) fn render_map_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<MapRenderState>>,
    env: &Environment,
) {
    render_map_parts(ctx, state, env);
}

pub(crate) fn render_map_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<MapRenderState>>,
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
