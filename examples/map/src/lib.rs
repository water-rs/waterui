//! Map Example - Demonstrates WaterUI's Map component
//!
//! This example showcases:
//! - Displaying a vector map centered on Manhattan
//! - Zoom controls with reactive updates
//! - Native-first map configuration with a GPU provider for self-drawn backends
//! - WaterKit location permission and reactive user-location display

use waterkit_location::{Location, Permission, PermissionStatus};
use waterkit_permission::request;
use waterui::app::App;
use waterui::env::use_env;
use waterui::prelude::theme_color::Surface;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui_icons_lucide as lucide;
use waterui_map::{Coordinate, Map, MapStyle, Region};
use waterui_map_gpu::MapGpuOptions;
use waterui_url::Url;

const MAP_CONTROL_SPACING: f32 = 10.0;
const MAP_CONTROL_INSET: f32 = 18.0;
const STATUS_PANEL_WIDTH: f32 = 264.0;
const STATUS_PANEL_HEIGHT: f32 = 72.0;
const STATUS_PANEL_INSET: f32 = 16.0;

fn coordinate(latitude: f64, longitude: f64) -> Coordinate {
    Coordinate::from_degrees(latitude, longitude).expect("example coordinates should be valid")
}

fn zoom_region(region: &Binding<Region>, scale: f64) {
    let current = region.get();
    let target = Region::new(
        current.center,
        current.latitude_delta * scale,
        current.longitude_delta * scale,
    );
    tracing::debug!(
        latitude_delta = target.latitude_delta,
        longitude_delta = target.longitude_delta,
        "map example requested a camera zoom"
    );
    region.set(target);
}

#[derive(Clone)]
struct MapExampleState {
    region: Binding<Region>,
    user_location: Binding<Option<Location>>,
    location_status: Binding<Str>,
    locating: Binding<bool>,
}

impl MapExampleState {
    fn new() -> Self {
        let manhattan = coordinate(40.7580, -73.9855);
        Self {
            region: binding(Region::new(manhattan, 0.030, 0.050)),
            user_location: binding(None),
            location_status: binding("Location is not requested"),
            locating: binding(false),
        }
    }
}

/// Demo view with map and controls
fn content(state: MapExampleState) -> impl View {
    let region = state.region;
    let user_location = state.user_location;
    let location_status = state.location_status;
    let locating = state.locating;
    let map = Map::new(region.clone())
        .style(MapStyle::Standard)
        .optional_user_location(user_location.clone())
        .shows_compass(true)
        .shows_scale(true);

    let location_control = button(label("Use My Location").icon(lucide::locate_fixed()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action_async(
            |State(location): State<Binding<Option<Location>>>,
             State(region): State<Binding<Region>>,
             State(status): State<Binding<Str>>,
             State(locating): State<Binding<bool>>| async move {
                locating.set(true);
                status.set("Requesting WaterKit location…".into());
                match request(Permission::Location).await {
                    Ok(PermissionStatus::Granted) => match Location::get().await {
                        Ok(value) => {
                            region.set(Region::from_coordinate(Coordinate::from(&value)));
                            location.set(Some(value));
                            status.set("Following WaterKit location".into());
                        }
                        Err(error) => {
                            tracing::error!("WaterKit location request failed: {error}");
                            status.set(format!("Location failed: {error}").into());
                        }
                    },
                    Ok(permission) => {
                        status.set(format!("Location permission: {permission:?}").into());
                    }
                    Err(error) => {
                        tracing::error!("Location permission request failed: {error}");
                        status.set(format!("Permission failed: {error}").into());
                    }
                }
                locating.set(false);
            },
        )
        .state(&user_location)
        .state(&region)
        .state(&location_status)
        .state(&locating)
        .floating()
        .disabled(locating.clone());

    let zoom_in = button(label("Zoom In").icon(lucide::plus()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action(|State(region): State<Binding<Region>>| zoom_region(&region, 0.5))
        .state(&region);
    let zoom_out = button(label("Zoom Out").icon(lucide::minus()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action(|State(region): State<Binding<Region>>| zoom_region(&region, 2.0))
        .state(&region);

    let status_panel = vstack((
        text("Manhattan").title(),
        text!("{location_status}").caption(),
    ))
    .spacing(4.0)
    .padding_with(EdgeInsets::symmetric(10.0, 14.0))
    .background(RoundedRectangle::new(0.18).fill(Surface))
    .size(STATUS_PANEL_WIDTH, STATUS_PANEL_HEIGHT)
    .position_in_offset(
        UnitPoint::TOP_LEADING,
        UnitPoint::TOP_LEADING,
        STATUS_PANEL_INSET,
        STATUS_PANEL_INSET,
    );

    let controls = use_env(move |env: Environment| {
        // Ambient theme data: a theme that styles floating surfaces supplies
        // these, and the framework default applies on backends that do not.
        let style = env.get::<FloatingStyle>().cloned().unwrap_or_default();
        let control_width = style.minimum_width as f32;
        let zoom_controls = vstack((zoom_in, Divider, zoom_out))
            .width(control_width)
            .floating_with(style);
        vstack((location_control, zoom_controls))
            .spacing(MAP_CONTROL_SPACING)
            .width(control_width)
            .position_in_offset(
                UnitPoint::BOTTOM_TRAILING,
                UnitPoint::BOTTOM_TRAILING,
                -MAP_CONTROL_INSET,
                -MAP_CONTROL_INSET,
            )
    });

    absolute((map.pin(PinConstraints::all(0.0)), status_panel, controls))
}

#[preview]
pub fn demo() -> impl View {
    content(MapExampleState::new())
}

/// Builds the example with whichever map realization this platform needs.
///
/// Selecting the map realization is the application's job, not the framework's:
/// `waterui-map` is the semantic component, `waterui-map-gpu` is one way to draw
/// it. Apple platforms bridge `MapKit`, so nothing is installed there and the
/// same `Map` view above draws through the native bridge. Every other platform
/// draws the map itself, and the tile source below is what it draws from.
pub fn app(mut env: Environment) -> App {
    env.insert(MapGpuOptions::new(Url::new(
        "https://tiles.openfreemap.org/styles/positron",
    )));
    waterui_map_gpu::install(&mut env);
    let state = MapExampleState::new();
    App::new(move || content(state.clone()), env)
}
