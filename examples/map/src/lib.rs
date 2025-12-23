//! Map Example - Demonstrates WaterUI's Map component
//!
//! This example showcases:
//! - Displaying a map centered on a coordinate
//! - Adding annotation pins
//! - Changing map styles
//! - Showing user location

use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_map::{Annotation, Coordinate, Map, MapStyle, Region};

/// Main view with map and controls
fn main_view() -> impl View {
    // San Francisco coordinates
    let sf = Coordinate::new(37.7749, -122.4194);
    let region = binding(Region::new(sf, 0.1, 0.1));
    let style = binding(MapStyle::Standard);
    let show_user_location = binding(false);

    // Sample annotations
    let annotations = binding(vec![
        Annotation::new(sf, "San Francisco").subtitle("California"),
        Annotation::new(
            Coordinate::new(37.8199, -122.4783),
            "Golden Gate Bridge",
        ),
        Annotation::new(
            Coordinate::new(37.7956, -122.3933),
            "Ferry Building",
        ),
    ]);

    let map = Map::new(region.clone())
        .annotations(annotations.clone())
        .shows_user_location(show_user_location.get())
        .shows_compass(true)
        .shows_scale(true);

    let controls = vstack((
        text("Map Example")
            .font(font::Title)
            .foreground(theme_color::Foreground),
        hstack((
            button("Standard").action({
                let style = style.clone();
                move || style.set(MapStyle::Standard)
            }),
            button("Satellite").action({
                let style = style.clone();
                move || style.set(MapStyle::Satellite)
            }),
            button("Hybrid").action({
                let style = style.clone();
                move || style.set(MapStyle::Hybrid)
            }),
        ))
        .spacing(8.0),
        Toggle::new(&show_user_location).label(
            text("Show User Location")
                .font(font::Body)
                .foreground(theme_color::Foreground),
        ),
        hstack((
            button("Zoom In").action({
                let region = region.clone();
                move || {
                    let current = region.get();
                    region.set(Region::new(
                        current.center,
                        current.latitude_delta * 0.5,
                        current.longitude_delta * 0.5,
                    ));
                }
            }),
            button("Zoom Out").action({
                let region = region.clone();
                move || {
                    let current = region.get();
                    region.set(Region::new(
                        current.center,
                        current.latitude_delta * 2.0,
                        current.longitude_delta * 2.0,
                    ));
                }
            }),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .padding()
    .width(250.0);

    hstack((map, controls))
}

pub fn app(env: Environment) -> App {
    App::new(main_view(), env)
}

waterui_ffi::export!();
