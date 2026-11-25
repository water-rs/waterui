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
//! use waterui::reactive::binding;
//! use waterui_map::{Annotation, Coordinate, Map, Region};
//!
//! // Display a map centered on San Francisco with selectable pins
//! # fn example() -> Result<(), waterui_map::OutOfRange> {
//! let san_francisco = Coordinate::from_degrees(37.7749, -122.4194)?;
//! let camera = binding(Region::new(san_francisco, 0.1, 0.1));
//! let selected = binding(None);
//!
//! let map = Map::new(&camera)
//!     .annotations(vec![Annotation::new("sf", san_francisco, "San Francisco")])
//!     .selection(&selected)
//!     .shows_user_location(true);
//! # let _ = map;
//! # Ok(())
//! # }
//! ```

extern crate alloc;

use alloc::vec::Vec;
use nami::{Binding, Signal, impl_constant, signal::IntoComputed};
use suiteki::Str;
use waterui_core::id::{Id, Mapping};
use waterui_core::{
    Computed, Environment, Metadata, Native, NativeView, OneOf2, Retain, SignalExt, View,
    layout::StretchAxis,
};

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

impl_constant!(Coordinate, Region, MapStyle, MapStatus);

/// A map annotation (pin marker) carrying an application-defined identifier.
///
/// The type parameter defaults to [`Id`], the erased identifier type backends
/// receive inside [`MapConfig`]. Application code constructs annotations with
/// any `T: Ord + Clone + 'static` (such as an enum, integer, or string) and
/// binds selection through [`Map::selection`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Annotation<T = Id> {
    /// Application-defined identity for reconciliation and selection.
    id: T,
    /// The coordinate where the annotation is placed.
    coordinate: Coordinate,
    /// The title text shown on the annotation.
    title: Str,
    /// Optional subtitle text.
    subtitle: Option<Str>,
}

impl<T> Annotation<T> {
    /// The application-defined identifier of this annotation.
    #[must_use]
    pub const fn id(&self) -> &T {
        &self.id
    }

    /// The coordinate where the annotation is placed.
    #[must_use]
    pub const fn coordinate(&self) -> Coordinate {
        self.coordinate
    }

    /// The title text shown on the annotation.
    #[must_use]
    pub const fn title(&self) -> &Str {
        &self.title
    }

    /// Optional subtitle text.
    #[must_use]
    pub const fn get_subtitle(&self) -> Option<&Str> {
        self.subtitle.as_ref()
    }

    /// Creates a new annotation with an identifier, coordinate, and title.
    pub fn new(id: T, coordinate: Coordinate, title: impl Into<Str>) -> Self {
        Self {
            id,
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

    /// Transforms this annotation's identifier, preserving coordinate and text.
    #[must_use]
    pub fn map_id<U>(self, f: impl FnOnce(T) -> U) -> Annotation<U> {
        Annotation {
            id: f(self.id),
            coordinate: self.coordinate,
            title: self.title,
            subtitle: self.subtitle,
        }
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
#[expect(
    clippy::exhaustive_enums,
    reason = "the map realizations load each style variant by variant; a new style must fail their build instead of falling into a wildcard arm"
)]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct MapConfig {
    /// Two-way camera region: app updates move the map, user gestures write back.
    camera: Binding<Region>,
    /// Annotations (pins) to display on the map, with erased [`Id`]s.
    annotations: Computed<Vec<Annotation<Id>>>,
    /// Optional two-way selection of the currently selected annotation [`Id`].
    selection: Option<Binding<Option<Id>>>,
    /// The map display style.
    style: MapStyle,
    /// Whether to show the user's current location.
    user_location_visibility: MapVisibility,
    /// `WaterKit` location values rendered by portable map realizations.
    ///
    /// Native realizations may use the platform location service when this is
    /// absent. Supplying it keeps camera following, the location marker, and
    /// horizontal-accuracy visualization driven by one reactive source.
    user_location: Option<Computed<Option<Location>>>,
    /// Whether the map is interactive (pan/zoom enabled).
    interactivity: MapInteractivity,
    /// Whether to show the compass.
    compass_visibility: MapVisibility,
    /// Whether to show the scale.
    scale_visibility: MapVisibility,
    /// Caller-owned sink the realization reports its load lifecycle into.
    ///
    /// `None` means the application is not observing status, and a realization
    /// must not allocate one on its behalf.
    status: Option<Binding<MapStatus>>,
}

impl MapConfig {
    /// Two-way camera region binding.
    ///
    /// Changes written by the application move the camera; user pan and zoom
    /// gestures write the settled region back into this binding.
    #[must_use]
    pub const fn camera(&self) -> &Binding<Region> {
        &self.camera
    }

    /// Annotations (pins) to display on the map, carrying erased [`Id`]s.
    #[must_use]
    pub const fn annotations(&self) -> &Computed<Vec<Annotation<Id>>> {
        &self.annotations
    }

    /// Optional two-way binding for the currently selected annotation [`Id`].
    #[must_use]
    pub const fn selection(&self) -> Option<&Binding<Option<Id>>> {
        self.selection.as_ref()
    }

    /// The map display style.
    #[must_use]
    pub const fn style(&self) -> MapStyle {
        self.style
    }

    /// Whether to show the user's current location.
    #[must_use]
    pub const fn user_location_visibility(&self) -> MapVisibility {
        self.user_location_visibility
    }

    /// `WaterKit` location values rendered by portable map realizations.
    ///
    /// Native realizations may use the platform location service when this is
    /// absent. Supplying it keeps camera following, the location marker, and
    /// horizontal-accuracy visualization driven by one reactive source.
    #[must_use]
    pub const fn user_location(&self) -> Option<&Computed<Option<Location>>> {
        self.user_location.as_ref()
    }

    /// Whether the map is interactive (pan/zoom enabled).
    #[must_use]
    pub const fn interactivity(&self) -> MapInteractivity {
        self.interactivity
    }

    /// Whether to show the compass.
    #[must_use]
    pub const fn compass_visibility(&self) -> MapVisibility {
        self.compass_visibility
    }

    /// Whether to show the scale.
    #[must_use]
    pub const fn scale_visibility(&self) -> MapVisibility {
        self.scale_visibility
    }

    /// Caller-owned sink the realization reports its load lifecycle into.
    ///
    /// `None` means the application is not observing status, and a realization
    /// must not allocate one on its behalf.
    #[must_use]
    pub const fn status(&self) -> Option<&Binding<MapStatus>> {
        self.status.as_ref()
    }
}

impl NativeView for MapConfig {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Marker type for a [`Map`] that has not yet been given typed annotations or
/// an annotation selection binding.
#[derive(Debug, Clone, Copy)]
pub struct NoAnnotations;

impl sealed::Sealed for NoAnnotations {}
impl<T: Ord + Clone + 'static> sealed::Sealed for T {}

/// Erased annotation list and optional selection binding produced by [`MapAnnotations`].
#[doc(hidden)]
#[derive(Debug)]
pub struct ErasedMapAnnotations {
    annotations: Computed<Vec<Annotation<Id>>>,
    selection: Option<Binding<Option<Id>>>,
}

/// Erases a [`Map`]'s typed annotation identifiers and selection binding into
/// the [`Id`]-keyed backend contract.
pub trait MapAnnotations: sealed::Sealed + Sized + 'static {
    /// Converts typed annotations and optional selection into erased [`Id`]
    /// signals sharing one [`Mapping`].
    #[doc(hidden)]
    fn erase(
        annotations: Option<Computed<Vec<Annotation<Self>>>>,
        selection: Option<Binding<Option<Self>>>,
    ) -> ErasedMapAnnotations;
}

impl MapAnnotations for NoAnnotations {
    fn erase(
        _annotations: Option<Computed<Vec<Annotation<Self>>>>,
        _selection: Option<Binding<Option<Self>>>,
    ) -> ErasedMapAnnotations {
        ErasedMapAnnotations {
            annotations: Computed::new(Vec::new()),
            selection: None,
        }
    }
}

impl<T: Ord + Clone + 'static> MapAnnotations for T {
    fn erase(
        annotations: Option<Computed<Vec<Annotation<Self>>>>,
        selection: Option<Binding<Option<Self>>>,
    ) -> ErasedMapAnnotations {
        let mapping: Mapping<T> = Mapping::new();
        let erased_annotations = annotations.map_or_else(
            || Computed::new(Vec::new()),
            |annotations| {
                let mapping = mapping.clone();
                annotations
                    .map(move |items| {
                        items
                            .into_iter()
                            .map(|annotation| annotation.map_id(|id| mapping.to_id(id)))
                            .collect::<Vec<_>>()
                    })
                    .computed()
            },
        );
        let erased_selection = selection.map(|selection| mapping.optional_binding(&selection));
        ErasedMapAnnotations {
            annotations: erased_annotations,
            selection: erased_selection,
        }
    }
}

/// A map view that displays a geographic region with optional selectable
/// annotations.
#[derive(Debug)]
#[must_use]
pub struct Map<T: 'static = NoAnnotations> {
    camera: Binding<Region>,
    follows_region: Option<Computed<Region>>,
    annotations: Option<Computed<Vec<Annotation<T>>>>,
    selection: Option<Binding<Option<T>>>,
    style: MapStyle,
    user_location_visibility: MapVisibility,
    user_location: Option<Computed<Option<Location>>>,
    interactivity: MapInteractivity,
    compass_visibility: MapVisibility,
    scale_visibility: MapVisibility,
    status: Option<Binding<MapStatus>>,
}

mod private {
    pub trait SealedCenter {}
    pub trait SealedRegion<M> {}
    #[derive(Debug)]
    pub struct TwoWayRegion;
    #[derive(Debug)]
    pub struct SignalCenter;
}

/// Target center value carried by a one-way signal passed to [`IntoMapRegion`].
pub trait MapCenter: private::SealedCenter + Clone + 'static {
    #[doc(hidden)]
    fn into_region(self) -> Region;
}

impl private::SealedCenter for Coordinate {}
impl private::SealedCenter for Location {}

impl MapCenter for Coordinate {
    fn into_region(self) -> Region {
        Region::from_coordinate(self)
    }
}

impl MapCenter for Location {
    fn into_region(self) -> Region {
        Region::from_coordinate(Coordinate::from(self))
    }
}

/// Center coordinates or camera regions accepted by [`Map::new`] and [`map`].
///
/// Implemented for `&Binding<Region>` (two-way camera binding) and for any
/// reactive signal or constant producing [`Coordinate`] or [`Location`]
/// (one-way centered camera).
pub trait IntoMapRegion<M = private::SignalCenter>: private::SealedRegion<M> {
    #[doc(hidden)]
    fn into_map_camera(self) -> (Binding<Region>, Option<Computed<Region>>);
}

impl private::SealedRegion<private::TwoWayRegion> for &Binding<Region> {}

impl IntoMapRegion<private::TwoWayRegion> for &Binding<Region> {
    fn into_map_camera(self) -> (Binding<Region>, Option<Computed<Region>>) {
        (self.clone(), None)
    }
}

impl<S> private::SealedRegion<private::SignalCenter> for S
where
    S: Signal + Clone + 'static,
    S::Output: MapCenter,
{
}

impl<S> IntoMapRegion<private::SignalCenter> for S
where
    S: Signal + Clone + 'static,
    S::Output: MapCenter,
{
    fn into_map_camera(self) -> (Binding<Region>, Option<Computed<Region>>) {
        let region_signal = self.map(MapCenter::into_region).computed();
        let camera = Binding::container(region_signal.snapshot());
        (camera, Some(region_signal))
    }
}

impl Map<NoAnnotations> {
    /// Creates a new `Map` centered on `center`.
    ///
    /// Pass `&Binding<Region>` for a two-way camera binding (pan and zoom
    /// gestures write back into the binding), or any [`Coordinate`] /
    /// [`Location`] value or reactive signal for a one-way centered camera.
    pub fn new<M>(center: impl IntoMapRegion<M>) -> Self {
        let (camera, follows_region) = center.into_map_camera();
        Self {
            camera,
            follows_region,
            annotations: None,
            selection: None,
            style: MapStyle::default(),
            user_location_visibility: MapVisibility::Hidden,
            user_location: None,
            interactivity: MapInteractivity::Interactive,
            compass_visibility: MapVisibility::Visible,
            scale_visibility: MapVisibility::Visible,
            status: None,
        }
    }

    /// Sets the annotations (pins) to display on the map.
    pub fn annotations<T: Ord + Clone + 'static>(
        self,
        annotations: impl IntoComputed<Vec<Annotation<T>>>,
    ) -> Map<T> {
        Map {
            camera: self.camera,
            follows_region: self.follows_region,
            annotations: Some(annotations.into_computed()),
            selection: None,
            style: self.style,
            user_location_visibility: self.user_location_visibility,
            user_location: self.user_location,
            interactivity: self.interactivity,
            compass_visibility: self.compass_visibility,
            scale_visibility: self.scale_visibility,
            status: self.status,
        }
    }

    /// Binds annotation selection two-way to `selection`.
    pub fn selection<T: Ord + Clone + 'static>(self, selection: &Binding<Option<T>>) -> Map<T> {
        Map {
            camera: self.camera,
            follows_region: self.follows_region,
            annotations: None,
            selection: Some(selection.clone()),
            style: self.style,
            user_location_visibility: self.user_location_visibility,
            user_location: self.user_location,
            interactivity: self.interactivity,
            compass_visibility: self.compass_visibility,
            scale_visibility: self.scale_visibility,
            status: self.status,
        }
    }
}

impl<T: Ord + Clone + 'static> Map<T> {
    /// Sets the annotations (pins) to display on the map.
    pub fn annotations(mut self, annotations: impl IntoComputed<Vec<Annotation<T>>>) -> Self {
        self.annotations = Some(annotations.into_computed());
        self
    }

    /// Binds annotation selection two-way to `selection`.
    pub fn selection(mut self, selection: &Binding<Option<T>>) -> Self {
        self.selection = Some(selection.clone());
        self
    }
}

impl<T: 'static> Map<T> {
    /// Sets the map display style.
    pub const fn style(mut self, style: MapStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets whether to show the user's current location on the map.
    pub const fn shows_user_location(mut self, show: bool) -> Self {
        self.user_location_visibility = MapVisibility::from_bool(show);
        self
    }

    /// Displays reactive `WaterKit` location values and their horizontal accuracy.
    pub fn user_location(mut self, location: impl IntoComputed<Location>) -> Self {
        self.user_location = Some(location.into_computed().map(Some).computed());
        self.user_location_visibility = MapVisibility::Visible;
        self
    }

    /// Displays an optional reactive `WaterKit` location.
    ///
    /// This is useful while an asynchronous location permission/request flow is
    /// pending: `None` draws no location marker and the first `Some` value
    /// updates the existing map without replacing its view identity.
    pub fn optional_user_location(mut self, location: impl IntoComputed<Option<Location>>) -> Self {
        self.user_location = Some(location.into_computed());
        self.user_location_visibility = MapVisibility::Visible;
        self
    }

    /// Binds the map center to reactive `Location` updates and enables user-location display.
    pub fn follows_location(mut self, location: impl IntoComputed<Location>) -> Self {
        let location_signal = location.into_computed();
        let region_signal = location_signal
            .map(|location| Region::from_coordinate(Coordinate::from(location)))
            .computed();
        self.camera.set(region_signal.snapshot());
        self.follows_region = Some(region_signal);
        self.user_location = Some(location_signal.map(Some).computed());
        self.user_location_visibility = MapVisibility::Visible;
        self
    }

    /// Sets whether the map is interactive (pan/zoom enabled).
    pub const fn is_interactive(mut self, interactive: bool) -> Self {
        self.interactivity = MapInteractivity::from_bool(interactive);
        self
    }

    /// Sets whether to show the compass.
    pub const fn shows_compass(mut self, show: bool) -> Self {
        self.compass_visibility = MapVisibility::from_bool(show);
        self
    }

    /// Sets whether to show the scale.
    pub const fn shows_scale(mut self, show: bool) -> Self {
        self.scale_visibility = MapVisibility::from_bool(show);
        self
    }

    /// Observes the map's load lifecycle through a caller-owned binding.
    ///
    /// The map writes [`MapStatus`] into `status` as it loads, fails, or
    /// becomes ready, so an application can show its own loading and error
    /// affordances over the map.
    pub fn status(mut self, status: &Binding<MapStatus>) -> Self {
        self.status = Some(status.clone());
        self
    }
}

impl<T: MapAnnotations> Map<T> {
    fn into_config(self) -> (MapConfig, Option<Computed<Region>>) {
        let erased = T::erase(self.annotations, self.selection);
        let config = MapConfig {
            camera: self.camera,
            annotations: erased.annotations,
            selection: erased.selection,
            style: self.style,
            user_location_visibility: self.user_location_visibility,
            user_location: self.user_location,
            interactivity: self.interactivity,
            compass_visibility: self.compass_visibility,
            scale_visibility: self.scale_visibility,
            status: self.status,
        };
        (config, self.follows_region)
    }
}

impl<T: MapAnnotations> View for Map<T> {
    fn body(self, _env: &Environment) -> impl View {
        let (config, follows_region) = self.into_config();
        match follows_region {
            Some(region_signal) => {
                let camera = config.camera.clone();
                let guard = region_signal.watch(move |context| {
                    camera.set(context.into_value());
                });
                OneOf2::B(Metadata::new(
                    Native::new(config),
                    Retain::new((guard, region_signal)),
                ))
            }
            None => OneOf2::A(Native::new(config)),
        }
    }
}

/// Convenience function to create a `Map` view centered on `center`.
pub fn map<M>(center: impl IntoMapRegion<M>) -> Map {
    Map::new(center)
}

#[cfg(test)]
mod tests {
    use super::{Annotation, Coordinate, Map, MapStatus, Region};
    use nami::{Binding, Signal};

    #[test]
    fn a_map_reports_status_into_the_caller_owned_binding() {
        let status = Binding::container(MapStatus::Loading);
        let camera = Binding::container(Region::from_coordinate(
            Coordinate::from_degrees(37.7749, -122.4194).expect("valid coordinate"),
        ));
        let (config, _) = Map::new(&camera).status(&status).into_config();

        let sink = config.status.expect("status sink must be retained");
        sink.set(MapStatus::Failed("style unreachable".into()));

        assert_eq!(
            status.snapshot().failure().map(ToString::to_string),
            Some(String::from("style unreachable"))
        );
        assert!(!status.snapshot().is_loading());
    }

    #[test]
    fn a_map_without_an_observer_allocates_no_status_sink() {
        let camera = Binding::container(Region::default());
        let (config, _) = Map::new(&camera).into_config();

        assert!(config.status.is_none());
    }

    #[test]
    fn camera_binding_is_two_way() {
        let sf = Coordinate::from_degrees(37.7749, -122.4194).expect("valid sf");
        let nyc = Coordinate::from_degrees(40.7128, -74.0060).expect("valid nyc");
        let camera = Binding::container(Region::from_coordinate(sf));
        let (config, _) = Map::new(&camera).into_config();

        // Backend writes back panned region into caller's camera binding.
        config.camera().set(Region::from_coordinate(nyc));
        assert_eq!(camera.snapshot(), Region::from_coordinate(nyc));

        // Caller recentering back to sf updates backend binding.
        camera.set(Region::from_coordinate(sf));
        assert_eq!(config.camera().snapshot(), Region::from_coordinate(sf));
    }

    #[test]
    fn annotations_and_selection_erase_through_shared_mapping() {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Pin {
            Ferry,
            Bridge,
        }

        let sf = Coordinate::from_degrees(37.7955, -122.3937).expect("valid ferry");
        let bridge = Coordinate::from_degrees(37.8199, -122.4783).expect("valid bridge");
        let camera = Binding::container(Region::from_coordinate(sf));
        let selected = Binding::container(None::<Pin>);
        let pins = vec![
            Annotation::new(Pin::Ferry, sf, "Ferry Building"),
            Annotation::new(Pin::Bridge, bridge, "Golden Gate Bridge"),
        ];

        let (config, _) = Map::new(&camera)
            .annotations(pins)
            .selection(&selected)
            .into_config();

        let erased = config.annotations().snapshot();
        assert_eq!(erased.len(), 2);
        assert_ne!(erased[0].id(), erased[1].id());

        let selection = config.selection().expect("selection binding retained");
        assert_eq!(selection.snapshot(), None);

        // Backend taps second pin -> caller selection becomes Some(Pin::Bridge).
        selection.set(Some(*erased[1].id()));
        assert_eq!(selected.snapshot(), Some(Pin::Bridge));

        // Caller changes selection programmatically -> backend sees first pin's Id.
        selected.set(Some(Pin::Ferry));
        assert_eq!(selection.snapshot(), Some(*erased[0].id()));

        // Deselecting clears both sides.
        selection.set(None);
        assert_eq!(selected.snapshot(), None);
    }
}
