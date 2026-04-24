//! Map component FFI bindings.
//!
//! This module provides FFI bindings for the Map component, allowing native backends
//! to render map views with coordinates, annotations, and user location.

use crate::reactive::WuiComputed;
use crate::{IntoFFI, WuiStr};
use alloc::vec::Vec;
use waterui_map::{Annotation, Coordinate, MapConfig, MapStyle, Region};
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
            latitude: self.latitude,
            longitude: self.longitude,
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
            MapStyle::Standard => WuiMapStyle::Standard,
            MapStyle::Satellite => WuiMapStyle::Satellite,
            MapStyle::Hybrid => WuiMapStyle::Hybrid,
        }
    }
}

// =============================================================================
// Map View FFI
// =============================================================================

/// FFI representation of the Map component.
#[repr(C)]
pub struct WuiMap {
    /// The region to display (reactive).
    pub region: *mut WuiComputed<Region>,
    /// Annotations to display (reactive).
    pub annotations: *mut WuiComputed<Vec<Annotation>>,
    /// The map display style.
    pub style: WuiMapStyle,
    /// Whether to show the user's current location.
    pub shows_user_location: bool,
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
            is_interactive: self.interactivity.is_interactive(),
            shows_compass: self.compass_visibility.is_visible(),
            shows_scale: self.scale_visibility.is_visible(),
        }
    }
}

// =============================================================================
// FFI view binding
// =============================================================================

ffi_view!(MapConfig, WuiMap, map);

// =============================================================================
// Computed types for watchers
// =============================================================================

crate::ffi_computed!(Region, WuiRegion, region);
crate::ffi_computed!(
    Vec<Annotation>,
    crate::array::WuiArray<WuiAnnotation>,
    annotations
);
