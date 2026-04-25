//! # `WaterUI` Map Component
//!
//! This crate provides a declarative map component for the `WaterUI` framework.
//! It displays native maps (`MKMapView` on Apple platforms) with support for
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
    pub const fn from_location(location: &Location) -> Self {
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
    pub const fn from_coordinate(coordinate: Coordinate) -> Self {
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

/// Generic visibility toggle for optional map chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapVisibility {
    /// Hide the feature.
    Hidden,
    /// Show the feature.
    Visible,
}

impl MapVisibility {
    const fn from_bool(value: bool) -> Self {
        if value { Self::Visible } else { Self::Hidden }
    }

    /// Returns whether the feature should be visible.
    #[must_use]
    pub const fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// Controls whether the user can pan and zoom the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapInteractivity {
    /// Disable direct map interaction.
    ReadOnly,
    /// Enable direct map interaction.
    Interactive,
}

impl MapInteractivity {
    const fn from_bool(value: bool) -> Self {
        if value {
            Self::Interactive
        } else {
            Self::ReadOnly
        }
    }

    /// Returns whether gestures are enabled.
    #[must_use]
    pub const fn is_interactive(self) -> bool {
        matches!(self, Self::Interactive)
    }
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
    pub user_location_visibility: MapVisibility,
    /// Whether the map is interactive (pan/zoom enabled).
    pub interactivity: MapInteractivity,
    /// Whether to show the compass.
    pub compass_visibility: MapVisibility,
    /// Whether to show the scale.
    pub scale_visibility: MapVisibility,
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
            user_location_visibility: MapVisibility::Hidden,
            interactivity: MapInteractivity::Interactive,
            compass_visibility: MapVisibility::Visible,
            scale_visibility: MapVisibility::Visible,
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
    pub const fn style(mut self, style: MapStyle) -> Self {
        self.0.style = style;
        self
    }

    /// Sets whether to show the user's current location on the map.
    #[must_use]
    pub const fn shows_user_location(mut self, show: bool) -> Self {
        self.0.user_location_visibility = MapVisibility::from_bool(show);
        self
    }

    /// Binds the map center to reactive `Location` updates and enables user-location display.
    #[must_use]
    pub fn follows_location(mut self, location: impl IntoComputed<Location>) -> Self {
        let location_signal = location.into_computed();
        self.0.region = location_signal
            .map(|location| Region::from_coordinate(Coordinate::from(location)))
            .into_computed();
        self.0.user_location_visibility = MapVisibility::Visible;
        self
    }

    /// Sets whether the map is interactive (pan/zoom enabled).
    #[must_use]
    pub const fn is_interactive(mut self, interactive: bool) -> Self {
        self.0.interactivity = MapInteractivity::from_bool(interactive);
        self
    }

    /// Sets whether to show the compass.
    #[must_use]
    pub const fn shows_compass(mut self, show: bool) -> Self {
        self.0.compass_visibility = MapVisibility::from_bool(show);
        self
    }

    /// Sets whether to show the scale.
    #[must_use]
    pub const fn shows_scale(mut self, show: bool) -> Self {
        self.0.scale_visibility = MapVisibility::from_bool(show);
        self
    }
}

/// Convenience function to create a Map view.
pub fn map(region: impl IntoComputed<Region>) -> Map {
    Map::new(region)
}
