//! Map Example - Demonstrates `WaterUI`'s Map component
//!
//! This example showcases:
//! - Displaying a vector map centered on Manhattan
//! - Zoom controls with reactive updates
//! - Native-first map configuration with a GPU provider for self-drawn backends
//! - `WaterKit` location permission and reactive user-location display

use lucide::locate_fixed;
use lucide::minus;
use lucide::plus;
use waterkit_location::{Location, Permission, PermissionStatus};
use waterkit_permission::request;
use waterui::app::{App, Windows};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::Signal;
use waterui::reactive::binding;
use waterui::shape::CornerRadius;
use waterui::theme::color::Surface;
use waterui_icons_lucide as lucide;
use waterui_map::{Coordinate, Map, MapStyle, Region};
use waterui_map_gpu::MapGpu;
use waterui_url::Url;

const MAP_CONTROL_WIDTH: f32 = 44.0;
const MAP_CONTROL_SPACING: f32 = 10.0;
const MAP_CONTROL_INSET: f32 = 18.0;
const STATUS_PANEL_WIDTH: f32 = 264.0;
const STATUS_PANEL_HEIGHT: f32 = 72.0;
const STATUS_PANEL_INSET: f32 = 16.0;

fn coordinate(latitude: f64, longitude: f64) -> Coordinate {
    Coordinate::from_degrees(latitude, longitude).expect("example coordinates should be valid")
}

fn zoom_region(region: &Binding<Region>, scale: f64) {
    let current = region.snapshot();
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
}

impl MapExampleState {
    fn new() -> Self {
        let manhattan = coordinate(40.7580, -73.9855);
        Self {
            region: binding(Region::new(manhattan, 0.030, 0.050)),
            user_location: binding(None),
            location_status: binding(Str::from("Location is not requested")),
        }
    }
}

/// Demo view with map and controls
fn content(state: MapExampleState) -> impl View {
    let region = state.region;
    let user_location = state.user_location;
    let location_status = state.location_status;
    let map = Map::new(&region)
        .style(MapStyle::Standard)
        .optional_user_location(user_location.clone())
        .shows_compass(true)
        .shows_scale(true);

    let location_control = button(label("Use My Location").icon(locate_fixed()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action_async(
            |State(location): State<Binding<Option<Location>>>,
             State(region): State<Binding<Region>>,
             State(status): State<Binding<Str>>| async move {
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
            },
        )
        .state(&user_location)
        .state(&region)
        .state(&location_status)
        .floating();

    let zoom_in = button(label("Zoom In").icon(plus()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action(|State(region): State<Binding<Region>>| zoom_region(&region, 0.5))
        .state(&region);
    let zoom_out = button(label("Zoom Out").icon(minus()))
        .label_style(LabelDisplayMode::IconOnly)
        .plain()
        .action(|State(region): State<Binding<Region>>| zoom_region(&region, 2.0))
        .state(&region);

    let status_panel = vstack((
        text("Manhattan").title(),
        text!("{location_status}").caption(),
    ))
    .spacing(4.0)
    .padding_with(EdgeInsets::vertical(10.0).with_horizontal(14.0))
    .background(RoundedRectangle::new(CornerRadius::fraction(0.18)).fill(Surface))
    .size(STATUS_PANEL_WIDTH, STATUS_PANEL_HEIGHT)
    .position_in(UnitPoint::TOP_LEADING)
    .anchor(UnitPoint::TOP_LEADING)
    .offset(STATUS_PANEL_INSET, STATUS_PANEL_INSET);

    let zoom_controls = vstack((zoom_in, Divider, zoom_out))
        .width(MAP_CONTROL_WIDTH)
        .floating();
    let controls = vstack((location_control, zoom_controls))
        .spacing(MAP_CONTROL_SPACING)
        .width(MAP_CONTROL_WIDTH)
        .position_in(UnitPoint::BOTTOM_TRAILING)
        .anchor(UnitPoint::BOTTOM_TRAILING)
        .offset(-MAP_CONTROL_INSET, -MAP_CONTROL_INSET);

    absolute((map.pin(PinConstraints::all(0.0)), status_panel, controls))
}

/// The map demo: a full-screen map with a status panel and zoom and location controls.
#[preview]
pub fn demo() -> impl View {
    with_tiles(content(MapExampleState::new()))
}

/// `view` with the tile source the GPU map draws from.
///
/// `waterui-map` is the semantic component. The Apple backend bridges
/// `MapKit`; Hydrolysis bridges no platform map and draws it with
/// `waterui-map-gpu`, which this example links, from the tile source below.
fn with_tiles(view: impl View) -> impl View {
    view.install(MapGpu::new(Url::new(
        "https://tiles.openfreemap.org/styles/positron",
    )))
}

/// Builds the example.
#[must_use]
pub fn app(env: Environment) -> App<impl Windows> {
    let state = MapExampleState::new();
    App::new(env).window(Window::new("", move || with_tiles(content(state.clone()))))
}
