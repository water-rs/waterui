//! Map Example - Demonstrates WaterUI's Map component
//!
//! This example showcases:
//! - Displaying a map centered on a coordinate
//! - Zoom controls with reactive updates
//! - Map style configuration
//! - User location display

use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui_map::{Coordinate, Map, MapStyle, Region};

fn coordinate(latitude: f64, longitude: f64) -> Coordinate {
    Coordinate::from_degrees(latitude, longitude).expect("example coordinates should be valid")
}

/// Demo view with map and controls
#[preview]
pub fn demo() -> impl View {
    // San Francisco coordinates
    let sf = coordinate(37.7749, -122.4194);
    let region: Binding<Region> = binding(Region::new(sf, 0.1, 0.1));

    // Create map with reactive region
    let map = Map::new(region.clone())
        .style(MapStyle::Standard)
        .shows_user_location(true)
        .shows_compass(true)
        .shows_scale(true);

    let controls = vstack((
        text("Map Example").title(),
        hstack((
            button("Zoom In")
                .action(|State(r): State<Binding<Region>>| {
                    let current = r.get();
                    r.set(Region::new(
                        current.center,
                        current.latitude_delta * 0.5,
                        current.longitude_delta * 0.5,
                    ));
                })
                .state(&region)
                .width(112.0),
            button("Zoom Out")
                .action(|State(r): State<Binding<Region>>| {
                    let current = r.get();
                    r.set(Region::new(
                        current.center,
                        current.latitude_delta * 2.0,
                        current.longitude_delta * 2.0,
                    ));
                })
                .state(&region)
                .width(112.0),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .padding()
    .width(264.0);

    hstack((map, controls))
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
