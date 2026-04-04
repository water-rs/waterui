//! # WaterUI Map Component
//!
//! This crate provides a declarative map component for the WaterUI framework.
//! It displays native maps (MKMapView on Apple platforms) with support for
//! annotations and user location.
//!
//! ## Example
//!
//! ```ignore
//! use waterui_map::{Map, Coordinate, Region, Annotation};
//!
//! // Display a map centered on San Francisco
//! let region = Region::new(
//!     Coordinate::new(37.7749, -122.4194),  // San Francisco
//!     0.1, 0.1  // span in degrees
//! );
//!
//! let map = Map::new(region)
//!     .annotations(vec![
//!         Annotation::new(Coordinate::new(37.7749, -122.4194), "San Francisco"),
//!     ])
//!     .shows_user_location(true);
//! ```

#![allow(clippy::module_name_repetitions)]

extern crate alloc;

use alloc::vec::Vec;
use nami::{impl_constant, signal::IntoComputed};
use waterui_core::{Computed, SignalExt, configurable, layout::StretchAxis};
use waterui_str::Str;

// Re-export waterkit-location for downstream convenience.
pub use waterkit_location as location;
// Commonly used location type re-export.
pub use waterkit_location::Location;

/// A geographic coordinate with latitude and longitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinate {
    /// Latitude in degrees (-90 to 90).
    pub latitude: f64,
    /// Longitude in degrees (-180 to 180).
    pub longitude: f64,
}

impl Coordinate {
    /// Creates a new coordinate.
    #[must_use]
    pub const fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    /// Creates a coordinate from a `waterkit_location::Location`.
    #[must_use]
    pub fn from_location(location: &Location) -> Self {
        Self {
            latitude: location.latitude(),
            longitude: location.longitude(),
        }
    }
}

impl From<Location> for Coordinate {
    fn from(value: Location) -> Self {
        Self::from_location(&value)
    }
}

impl From<&Location> for Coordinate {
    fn from(value: &Location) -> Self {
        Self::from_location(value)
    }
}

impl Default for Coordinate {
    fn default() -> Self {
        // Default to null island (0, 0)
        Self::new(0.0, 0.0)
    }
}

/// A map region defined by a center coordinate and span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// The center coordinate of the region.
    pub center: Coordinate,
    /// The north-to-south span in degrees.
    pub latitude_delta: f64,
    /// The east-to-west span in degrees.
    pub longitude_delta: f64,
}

impl Region {
    /// Creates a new region.
    #[must_use]
    pub const fn new(center: Coordinate, latitude_delta: f64, longitude_delta: f64) -> Self {
        Self {
            center,
            latitude_delta,
            longitude_delta,
        }
    }

    /// Creates a region from a coordinate with default zoom.
    #[must_use]
    pub fn from_coordinate(coordinate: Coordinate) -> Self {
        Self::new(coordinate, 0.05, 0.05)
    }
}

impl Default for Region {
    fn default() -> Self {
        Self::new(Coordinate::default(), 0.1, 0.1)
    }
}

impl From<Coordinate> for Region {
    fn from(coordinate: Coordinate) -> Self {
        Self::from_coordinate(coordinate)
    }
}

impl_constant!(Coordinate, Region, Annotation, MapStyle);

/// A map annotation (pin marker).
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// The coordinate where the annotation is placed.
    pub coordinate: Coordinate,
    /// The title text shown on the annotation.
    pub title: Str,
    /// Optional subtitle text.
    pub subtitle: Option<Str>,
}

impl Annotation {
    /// Creates a new annotation with a title.
    pub fn new(coordinate: Coordinate, title: impl Into<Str>) -> Self {
        Self {
            coordinate,
            title: title.into(),
            subtitle: None,
        }
    }

    /// Sets the subtitle for this annotation.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<Str>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

/// Map display style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapStyle {
    /// Standard road map.
    #[default]
    Standard,
    /// Satellite imagery.
    Satellite,
    /// Hybrid of satellite and roads.
    Hybrid,
}

/// Configuration for the Map component.
#[derive(Debug)]
pub struct MapConfig {
    /// The region to display.
    pub region: Computed<Region>,
    /// Annotations (pins) to display on the map.
    pub annotations: Computed<Vec<Annotation>>,
    /// The map display style.
    pub style: MapStyle,
    /// Whether to show the user's current location.
    pub shows_user_location: bool,
    /// Whether the map is interactive (pan/zoom enabled).
    pub is_interactive: bool,
    /// Whether to show the compass.
    pub shows_compass: bool,
    /// Whether to show the scale.
    pub shows_scale: bool,
}

// Use configurable! with StretchAxis::Both - this provides both NativeView and View impls
configurable!(
    #[doc = "A map view that displays a geographic region with optional annotations."]
    Map,
    MapConfig,
    StretchAxis::Both
);

impl Map {
    /// Creates a new Map displaying the specified region.
    ///
    /// # Arguments
    ///
    /// * `region` - The map region to display (can be reactive).
    pub fn new(region: impl IntoComputed<Region>) -> Self {
        let empty_annotations: Vec<Annotation> = Vec::new();
        Self(MapConfig {
            region: region.into_computed(),
            annotations: empty_annotations.into_computed(),
            style: MapStyle::default(),
            shows_user_location: false,
            is_interactive: true,
            shows_compass: true,
            shows_scale: true,
        })
    }

    /// Creates a new Map centered on the specified coordinate with default zoom.
    pub fn centered_on(coordinate: impl IntoComputed<Coordinate>) -> Self {
        let coord_signal = coordinate.into_computed();
        let region_signal: Computed<Region> = coord_signal.map(Region::from_coordinate).computed();
        Self::new(region_signal)
    }

    /// Creates a new Map centered on reactive `Location` values.
    pub fn centered_on_location(location: impl IntoComputed<Location>) -> Self {
        let location_signal = location.into_computed();
        let region_signal: Computed<Region> = location_signal
            .map(|location| Region::from_coordinate(Coordinate::from(location)))
            .computed();
        Self::new(region_signal)
    }

    /// Sets the annotations (pins) to display on the map.
    #[must_use]
    pub fn annotations(mut self, annotations: impl IntoComputed<Vec<Annotation>>) -> Self {
        self.0.annotations = annotations.into_computed();
        self
    }

    /// Sets the map display style.
    #[must_use]
    pub fn style(mut self, style: MapStyle) -> Self {
        self.0.style = style;
        self
    }

    /// Sets whether to show the user's current location on the map.
    #[must_use]
    pub fn shows_user_location(mut self, show: bool) -> Self {
        self.0.shows_user_location = show;
        self
    }

    /// Binds the map center to reactive `Location` updates and enables user-location display.
    #[must_use]
    pub fn follows_location(mut self, location: impl IntoComputed<Location>) -> Self {
        let location_signal = location.into_computed();
        self.0.region = location_signal
            .map(|location| Region::from_coordinate(Coordinate::from(location)))
            .into_computed();
        self.0.shows_user_location = true;
        self
    }

    /// Sets whether the map is interactive (pan/zoom enabled).
    #[must_use]
    pub fn is_interactive(mut self, interactive: bool) -> Self {
        self.0.is_interactive = interactive;
        self
    }

    /// Sets whether to show the compass.
    #[must_use]
    pub fn shows_compass(mut self, show: bool) -> Self {
        self.0.shows_compass = show;
        self
    }

    /// Sets whether to show the scale.
    #[must_use]
    pub fn shows_scale(mut self, show: bool) -> Self {
        self.0.shows_scale = show;
        self
    }
}

/// Convenience function to create a Map view.
pub fn map(region: impl IntoComputed<Region>) -> Map {
    Map::new(region)
}
