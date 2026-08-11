//! Map component FFI bindings.
//!
//! This module provides FFI bindings for the Map component, allowing native backends
//! to render map views with coordinates, annotations, and user location.

use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiStr};
use alloc::vec::Vec;
use waterui_map::{Annotation, Coordinate, Location, MapConfig, MapStatus, MapStyle, Region};
use waterui_str::Str;

// =============================================================================
// Coordinate FFI
// =============================================================================

/// FFI representation of a geographic coordinate.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WuiCoordinate {
    /// Latitude in degrees (-90 to 90).
    pub latitude: f64,
    /// Longitude in degrees (-180 to 180).
    pub longitude: f64,
}

impl IntoFFI for Coordinate {
    type FFI = WuiCoordinate;
    fn into_ffi(self) -> Self::FFI {
        WuiCoordinate {
            latitude: self.latitude.get(),
            longitude: self.longitude.get(),
        }
    }
}

// =============================================================================
// Region FFI
// =============================================================================

/// FFI representation of a map region.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WuiRegion {
    /// The center coordinate of the region.
    pub center: WuiCoordinate,
    /// The north-to-south span in degrees.
    pub latitude_delta: f64,
    /// The east-to-west span in degrees.
    pub longitude_delta: f64,
}

impl IntoFFI for Region {
    type FFI = WuiRegion;
    fn into_ffi(self) -> Self::FFI {
        WuiRegion {
            center: self.center.into_ffi(),
            latitude_delta: self.latitude_delta,
            longitude_delta: self.longitude_delta,
        }
    }
}

// =============================================================================
// Annotation FFI
// =============================================================================

/// FFI representation of a map annotation (pin).
#[repr(C)]
#[derive(Debug)]
pub struct WuiAnnotation {
    /// The coordinate where the annotation is placed.
    pub coordinate: WuiCoordinate,
    /// The title text.
    pub title: WuiStr,
    /// The subtitle text (empty string if none).
    pub subtitle: WuiStr,
}

impl IntoFFI for Annotation {
    type FFI = WuiAnnotation;
    fn into_ffi(self) -> Self::FFI {
        WuiAnnotation {
            coordinate: self.coordinate.into_ffi(),
            title: self.title.into_ffi(),
            subtitle: self
                .subtitle
                .unwrap_or_else(|| Str::from_static(""))
                .into_ffi(),
        }
    }
}

// =============================================================================
// MapStyle FFI
// =============================================================================

/// FFI representation of map display style.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiMapStyle {
    /// Standard road map.
    Standard = 0,
    /// Satellite imagery.
    Satellite = 1,
    /// Hybrid of satellite and roads.
    Hybrid = 2,
}

impl IntoFFI for MapStyle {
    type FFI = WuiMapStyle;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Standard => WuiMapStyle::Standard,
            Self::Satellite => WuiMapStyle::Satellite,
            Self::Hybrid => WuiMapStyle::Hybrid,
        }
    }
}

// =============================================================================
// Location FFI
// =============================================================================

/// FFI representation of an optional device location reading.
///
/// A `WaterKit` location carries optional altitude and accuracy, which C cannot
/// express as `Option`. Each optional reading is paired with a `has_*` flag
/// instead of a sentinel value, so a real zero altitude stays distinguishable
/// from "no altitude".
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WuiLocation {
    /// Whether a location is present at all.
    pub has_location: bool,
    /// Where the device is.
    pub coordinate: WuiCoordinate,
    /// Altitude in metres above sea level, when known.
    pub altitude: f64,
    /// Whether `altitude` holds a reading.
    pub has_altitude: bool,
    /// Horizontal accuracy radius in metres, when known.
    pub horizontal_accuracy: f64,
    /// Whether `horizontal_accuracy` holds a reading.
    pub has_horizontal_accuracy: bool,
    /// When the reading was taken, in milliseconds since the Unix epoch.
    pub timestamp_ms: i64,
}

impl IntoFFI for Option<Location> {
    type FFI = WuiLocation;
    fn into_ffi(self) -> Self::FFI {
        let Some(location) = self else {
            return WuiLocation {
                has_location: false,
                coordinate: WuiCoordinate {
                    latitude: 0.0,
                    longitude: 0.0,
                },
                altitude: 0.0,
                has_altitude: false,
                horizontal_accuracy: 0.0,
                has_horizontal_accuracy: false,
                timestamp_ms: 0,
            };
        };
        WuiLocation {
            has_location: true,
            coordinate: Coordinate::from_location(&location).into_ffi(),
            altitude: location.altitude().unwrap_or(0.0),
            has_altitude: location.altitude().is_some(),
            horizontal_accuracy: location.horizontal_accuracy().unwrap_or(0.0),
            has_horizontal_accuracy: location.horizontal_accuracy().is_some(),
            timestamp_ms: location.timestamp().as_millisecond(),
        }
    }
}

// =============================================================================
// MapStatus FFI
// =============================================================================

/// FFI representation of the map's load lifecycle.
#[repr(C)]
#[derive(Debug)]
pub struct WuiMapStatus {
    /// Which lifecycle state this is.
    pub kind: WuiMapStatusKind,
    /// Failure reason; empty unless `kind` is `Failed`.
    pub reason: WuiStr,
}

/// Discriminant of [`WuiMapStatus`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiMapStatusKind {
    /// The map is loading what it needs to draw.
    Loading = 0,
    /// The map has everything it needs.
    Ready = 1,
    /// The map failed to load.
    Failed = 2,
}

impl crate::IntoRust for WuiMapStatus {
    type Rust = MapStatus;
    /// # Safety
    ///
    /// `reason` must be a `WuiStr` this process still owns; it is consumed.
    unsafe fn into_rust(self) -> Self::Rust {
        match self.kind {
            WuiMapStatusKind::Loading => MapStatus::Loading,
            WuiMapStatusKind::Ready => MapStatus::Ready,
            WuiMapStatusKind::Failed => MapStatus::Failed(
                // SAFETY: the caller contract requires a live `WuiStr`.
                unsafe { crate::IntoRust::into_rust(self.reason) },
            ),
        }
    }
}

impl IntoFFI for MapStatus {
    type FFI = WuiMapStatus;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Loading => WuiMapStatus {
                kind: WuiMapStatusKind::Loading,
                reason: Str::from_static("").into_ffi(),
            },
            Self::Ready => WuiMapStatus {
                kind: WuiMapStatusKind::Ready,
                reason: Str::from_static("").into_ffi(),
            },
            Self::Failed(reason) => WuiMapStatus {
                kind: WuiMapStatusKind::Failed,
                reason: reason.into_ffi(),
            },
        }
    }
}

// =============================================================================
// Map View FFI
// =============================================================================

/// FFI representation of the Map component.
#[repr(C)]
#[derive(Debug)]
pub struct WuiMap {
    /// The region to display (reactive).
    pub region: *mut WuiComputed<Region>,
    /// Annotations to display (reactive).
    pub annotations: *mut WuiComputed<Vec<Annotation>>,
    /// The map display style.
    pub style: WuiMapStyle,
    /// Whether to show the user's current location.
    pub shows_user_location: bool,
    /// Reactive `WaterKit` location supplied by the application.
    ///
    /// Present whenever the application drove the location itself; a backend
    /// must prefer this over the platform's own location service so both agree
    /// on one source.
    /// Null when the application did not supply one, in which case a native
    /// backend may fall back to the platform's own location service.
    pub user_location: *mut WuiComputed<Option<Location>>,
    /// Sink the backend reports its load lifecycle into, or null when the
    /// application is not observing status.
    pub status: *mut WuiBinding<MapStatus>,
    /// Whether the map is interactive (pan/zoom enabled).
    pub is_interactive: bool,
    /// Whether to show the compass.
    pub shows_compass: bool,
    /// Whether to show the scale.
    pub shows_scale: bool,
}

impl IntoFFI for MapConfig {
    type FFI = WuiMap;
    fn into_ffi(self) -> Self::FFI {
        WuiMap {
            region: self.region.into_ffi(),
            annotations: self.annotations.into_ffi(),
            style: self.style.into_ffi(),
            shows_user_location: self.user_location_visibility.is_visible(),
            user_location: self
                .user_location
                .map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
            status: self
                .status
                .map_or(core::ptr::null_mut(), crate::IntoFFI::into_ffi),
            is_interactive: self.interactivity.is_interactive(),
            shows_compass: self.compass_visibility.is_visible(),
            shows_scale: self.scale_visibility.is_visible(),
        }
    }
}

// =============================================================================
// FFI view binding
// =============================================================================

#[cfg(feature = "c-api")]
ffi_view!(MapConfig, WuiMap, map);

// =============================================================================
// Computed types for watchers
// =============================================================================

#[cfg(feature = "c-api")]
crate::ffi_computed!(Region, WuiRegion, region);
#[cfg(feature = "c-api")]
crate::ffi_computed!(
    Vec<Annotation>,
    crate::array::WuiArray<WuiAnnotation>,
    annotations
);
#[cfg(feature = "c-api")]
crate::ffi_computed!(Option<Location>, WuiLocation, user_location);
#[cfg(feature = "c-api")]
crate::ffi_binding!(MapStatus, WuiMapStatus, map_status);
