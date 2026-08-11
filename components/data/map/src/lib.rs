//! # `WaterUI` Map Component
//!
//! This crate provides a declarative map component for the `WaterUI` framework.
//! It displays native maps where a platform has a suitable primitive and a
//! shared GPU-drawn vector map elsewhere, with annotations and `WaterKit`
//! location integration.
//!
//! ## Example
//!
//! ```ignore
//! use waterui_map::{Annotation, Coordinate, Map, Region};
//!
//! // Display a map centered on San Francisco
//! # fn example() -> Result<(), waterui_map::OutOfRange> {
//! let san_francisco = Coordinate::from_degrees(37.7749, -122.4194)?;
//! let region = Region::new(san_francisco, 0.1, 0.1);
//!
//! let map = Map::new(region)
//!     .annotations(vec![Annotation::new(san_francisco, "San Francisco")])
//!     .shows_user_location(true);
//! # let _ = map;
//! # Ok(())
//! # }
//! ```

extern crate alloc;

use alloc::vec::Vec;
use nami::{Binding, impl_constant, signal::IntoComputed};
use waterui_core::{Computed, SignalExt, configurable, layout::StretchAxis};
use waterui_str::Str;

// Re-export waterkit-location for downstream convenience.
pub use waterkit_location as location;
// Commonly used location types re-export.
pub use waterkit_location::{Latitude, Location, Longitude, OutOfRange, Timestamp};

/// A geographic coordinate with latitude and longitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinate {
    /// Latitude in degrees (-90 to 90).
    pub latitude: Latitude,
    /// Longitude in degrees (-180 to 180).
    pub longitude: Longitude,
}

impl Coordinate {
    /// Creates a new coordinate.
    #[must_use]
    pub const fn new(latitude: Latitude, longitude: Longitude) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    /// Creates a coordinate from raw degree values.
    ///
    /// # Errors
    ///
    /// Returns [`OutOfRange`] if either coordinate is `NaN` or outside its
    /// valid range.
    pub fn from_degrees(latitude: f64, longitude: f64) -> Result<Self, OutOfRange> {
        Ok(Self::new(
            Latitude::new(latitude)?,
            Longitude::new(longitude)?,
        ))
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
        Self::new(Latitude::new_unchecked(0.0), Longitude::new_unchecked(0.0))
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

impl_constant!(Coordinate, Region, Annotation, MapStyle, MapStatus);

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
///
/// Native realizations resolve these against the platform map's own imagery.
/// The portable GPU realization has no imagery of its own, so an application
/// asking for [`Self::Satellite`] or [`Self::Hybrid`] must also supply the
/// matching provider style (`MapGpuOptions::satellite_style_url` and
/// `hybrid_style_url`). The map reports a load failure when it did not, rather
/// than silently drawing the standard style.
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

/// Lifecycle of the imagery backing a map.
///
/// A realization that fetches its own tiles reports progress here so the
/// application can show a spinner or an error instead of an empty rectangle.
/// Realizations that draw from a platform map report the platform's own
/// load callbacks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MapStatus {
    /// The map is fetching the data it needs to draw.
    #[default]
    Loading,
    /// The map has everything it needs for the current camera.
    Ready,
    /// The map could not load, carrying a human-readable reason.
    ///
    /// The map keeps drawing whatever it last had, so a failure that arrives
    /// after a successful load leaves the previous imagery on screen.
    Failed(Str),
}

impl MapStatus {
    /// Returns whether the map is still loading.
    #[must_use]
    pub const fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// Returns the failure reason, if the map failed to load.
    #[must_use]
    pub const fn failure(&self) -> Option<&Str> {
        match self {
            Self::Failed(reason) => Some(reason),
            Self::Loading | Self::Ready => None,
        }
    }
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
    /// `WaterKit` location values rendered by portable map realizations.
    ///
    /// Native realizations may use the platform location service when this is
    /// absent. Supplying it keeps camera following, the location marker, and
    /// horizontal-accuracy visualization driven by one reactive source.
    pub user_location: Option<Computed<Option<Location>>>,
    /// Whether the map is interactive (pan/zoom enabled).
    pub interactivity: MapInteractivity,
    /// Whether to show the compass.
    pub compass_visibility: MapVisibility,
    /// Whether to show the scale.
    pub scale_visibility: MapVisibility,
    /// Caller-owned sink the realization reports its load lifecycle into.
    ///
    /// `None` means the application is not observing status, and a realization
    /// must not allocate one on its behalf.
    pub status: Option<Binding<MapStatus>>,
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
            user_location: None,
            interactivity: MapInteractivity::Interactive,
            compass_visibility: MapVisibility::Visible,
            scale_visibility: MapVisibility::Visible,
            status: None,
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

    /// Displays reactive `WaterKit` location values and their horizontal accuracy.
    #[must_use]
    pub fn user_location(mut self, location: impl IntoComputed<Location>) -> Self {
        self.0.user_location = Some(location.into_computed().map(Some).computed());
        self.0.user_location_visibility = MapVisibility::Visible;
        self
    }

    /// Displays an optional reactive `WaterKit` location.
    ///
    /// This is useful while an asynchronous location permission/request flow is
    /// pending: `None` draws no location marker and the first `Some` value
    /// updates the existing map without replacing its view identity.
    #[must_use]
    pub fn optional_user_location(mut self, location: impl IntoComputed<Option<Location>>) -> Self {
        self.0.user_location = Some(location.into_computed());
        self.0.user_location_visibility = MapVisibility::Visible;
        self
    }

    /// Binds the map center to reactive `Location` updates and enables user-location display.
    #[must_use]
    pub fn follows_location(mut self, location: impl IntoComputed<Location>) -> Self {
        let location_signal = location.into_computed();
        self.0.region = location_signal
            .map(|location| Region::from_coordinate(Coordinate::from(location)))
            .into_computed();
        self.0.user_location = Some(location_signal.map(Some).computed());
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

    /// Observes the map's load lifecycle through a caller-owned binding.
    ///
    /// The map writes [`MapStatus`] into `status` as it loads, fails, or
    /// becomes ready, so an application can show its own loading and error
    /// affordances over the map.
    #[must_use]
    pub fn status(mut self, status: &Binding<MapStatus>) -> Self {
        self.0.status = Some(status.clone());
        self
    }
}

/// Convenience function to create a Map view.
pub fn map(region: impl IntoComputed<Region>) -> Map {
    Map::new(region)
}

/// Convenience constructor for [`Map::centered_on`].
pub fn map_centered_on(coordinate: impl IntoComputed<Coordinate>) -> Map {
    Map::centered_on(coordinate)
}

/// Convenience constructor for [`Map::centered_on_location`].
pub fn map_centered_on_location(location: impl IntoComputed<Location>) -> Map {
    Map::centered_on_location(location)
}

#[cfg(test)]
mod tests {
    use super::{Coordinate, Map, MapStatus, Region};
    use nami::Binding;

    #[test]
    fn a_map_reports_status_into_the_caller_owned_binding() {
        let status = Binding::container(MapStatus::Loading);
        let map = Map::new(Region::from_coordinate(
            Coordinate::from_degrees(37.7749, -122.4194).expect("valid coordinate"),
        ))
        .status(&status);

        let sink = map.0.status.expect("status sink must be retained");
        sink.set(MapStatus::Failed("style unreachable".into()));

        assert_eq!(
            status.get().failure().map(ToString::to_string),
            Some(String::from("style unreachable"))
        );
        assert!(!status.get().is_loading());
    }

    #[test]
    fn a_map_without_an_observer_allocates_no_status_sink() {
        let map = Map::new(Region::default());

        assert!(map.0.status.is_none());
    }
}
