use nami::SignalExt;
use waterui::ViewExt as _;
use waterui::accessibility::{AccessibilityLabel, AccessibilityRole};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::Gradient;
use waterui_graphics::color::Srgb;
use waterui_layout::stack::{VStack, vstack, zstack};
use waterui_map::{Annotation, MapConfig, MapStyle, Region};
use waterui_text::Text;

use crate::renderer::{
    HydroNativeView, HydroState, WidgetRenderContext, measure_view_dimensions_with_proposal,
    measure_view_intrinsic, normalize_view_for_render,
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
    let shows_location = if config.shows_user_location {
        "User location on"
    } else {
        "User location off"
    };
    let interactivity = if config.is_interactive {
        "Interactive"
    } else {
        "Read only"
    };
    let controls = format!(
        "Compass: {}  Scale: {}",
        if config.shows_compass { "on" } else { "off" },
        if config.shows_scale { "on" } else { "off" }
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

impl HydroNativeView for Native<MapConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let surface_label = map_surface_label(env);
        let mut render_env = env.clone();
        render_env.remove::<AccessibilityLabel>();
        render_env.remove::<AccessibilityRole>();
        let content = map_content(view.as_inner(), &surface_label, &render_env);
        ctx.dispatch_in_rect(&render_env, content, ctx.bounds);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let surface_label = map_surface_label(env);
        let mut render_env = env.clone();
        render_env.remove::<AccessibilityLabel>();
        render_env.remove::<AccessibilityRole>();
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
        let surface_label = map_surface_label(env);
        let mut render_env = env.clone();
        render_env.remove::<AccessibilityLabel>();
        render_env.remove::<AccessibilityRole>();
        measure_view_dimensions_with_proposal(
            &map_content(view.as_inner(), &surface_label, &render_env),
            proposal,
            state,
            &render_env,
        )
    }

    fn accessibility_is_render_driven() -> bool {
        true
    }
}
