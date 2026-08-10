//! GPU-drawn vector map realization for platforms without a native map primitive.
//!
//! The semantic [`waterui_map::Map`] API stays platform-neutral. A self-drawn
//! backend installs this realization and applications provide a `MapLibre` style
//! through [`MapGpuOptions`]. Native backends do not install this hook.

pub(crate) mod network;
pub(crate) mod projection;
mod render;
pub(crate) mod style;
pub(crate) mod tile;

use std::{
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    rc::Rc,
    time::Duration,
};

use nami::{Binding, Computed, Signal as _, binding, watcher::BoxWatcherGuard};
use waterui_core::{
    AnyView, Environment, Metadata,
    animation::Animation,
    gesture::{
        DragEvent, DragGesture, GestureObserver, GesturePhase, MagnificationEvent,
        MagnificationGesture,
    },
};
use waterui_map::{Coordinate, MapConfig, Region};
use waterui_url::Url;

pub use render::{MapScene, map_surface};

/// Retry policy for transient style, source, and vector-tile request failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapNetworkRetryPolicy {
    maximum_attempts: NonZeroU32,
    initial_delay: Duration,
    maximum_delay: Duration,
}

impl MapNetworkRetryPolicy {
    const DEFAULT: Self = Self {
        maximum_attempts: NonZeroU32::new(4)
            .expect("static map retry attempt count must be non-zero"),
        initial_delay: Duration::from_millis(250),
        maximum_delay: Duration::from_secs(4),
    };

    /// Creates a bounded exponential-backoff retry policy.
    ///
    /// `maximum_attempts` includes the initial request.
    ///
    /// # Panics
    ///
    /// Panics if either delay is zero or `maximum_delay` is shorter than
    /// `initial_delay`.
    #[must_use]
    pub const fn new(
        maximum_attempts: NonZeroU32,
        initial_delay: Duration,
        maximum_delay: Duration,
    ) -> Self {
        assert!(
            !initial_delay.is_zero(),
            "GPU Map retry initial delay must be non-zero"
        );
        assert!(
            !maximum_delay.is_zero(),
            "GPU Map retry maximum delay must be non-zero"
        );
        assert!(
            maximum_delay.as_nanos() >= initial_delay.as_nanos(),
            "GPU Map retry maximum delay must not be shorter than the initial delay"
        );
        Self {
            maximum_attempts,
            initial_delay,
            maximum_delay,
        }
    }

    /// Returns the maximum number of request attempts, including the first.
    #[must_use]
    pub const fn maximum_attempts(self) -> NonZeroU32 {
        self.maximum_attempts
    }

    fn delay_after_failure(self, failed_attempt: u32) -> Duration {
        let exponent = failed_attempt.saturating_sub(1).min(31);
        self.initial_delay
            .saturating_mul(1_u32 << exponent)
            .min(self.maximum_delay)
    }
}

impl Default for MapNetworkRetryPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Resource and provider configuration for the GPU map realization.
#[derive(Debug, Clone)]
pub struct MapGpuOptions {
    style_url: Url,
    maximum_style_bytes: NonZeroUsize,
    maximum_tilejson_bytes: NonZeroUsize,
    maximum_tile_bytes: NonZeroUsize,
    tile_cache_bytes: NonZeroU64,
    maximum_in_flight_tile_requests: NonZeroUsize,
    request_timeout: Duration,
    network_retry_policy: MapNetworkRetryPolicy,
    camera_animation: Animation,
}

impl MapGpuOptions {
    /// Creates options for a `MapLibre` style URL.
    ///
    /// # Panics
    ///
    /// Panics if a compile-time response-size or cache-size constant is zero.
    #[must_use]
    pub const fn new(style_url: Url) -> Self {
        Self {
            style_url,
            maximum_style_bytes: NonZeroUsize::new(8 * 1024 * 1024)
                .expect("static style limit must be non-zero"),
            maximum_tilejson_bytes: NonZeroUsize::new(2 * 1024 * 1024)
                .expect("static TileJSON limit must be non-zero"),
            maximum_tile_bytes: NonZeroUsize::new(16 * 1024 * 1024)
                .expect("static vector tile limit must be non-zero"),
            tile_cache_bytes: NonZeroU64::new(128 * 1024 * 1024)
                .expect("static tile cache budget must be non-zero"),
            maximum_in_flight_tile_requests: NonZeroUsize::new(6)
                .expect("static tile request concurrency must be non-zero"),
            request_timeout: Duration::from_secs(20),
            network_retry_policy: MapNetworkRetryPolicy::DEFAULT,
            camera_animation: Animation::ease_in_out(core::time::Duration::from_millis(240)),
        }
    }

    /// Sets the maximum accepted `MapLibre` style response size.
    #[must_use]
    pub const fn maximum_style_bytes(mut self, value: NonZeroUsize) -> Self {
        self.maximum_style_bytes = value;
        self
    }

    /// Sets the maximum accepted `TileJSON` response size.
    #[must_use]
    pub const fn maximum_tilejson_bytes(mut self, value: NonZeroUsize) -> Self {
        self.maximum_tilejson_bytes = value;
        self
    }

    /// Sets the maximum accepted vector-tile response size.
    #[must_use]
    pub const fn maximum_tile_bytes(mut self, value: NonZeroUsize) -> Self {
        self.maximum_tile_bytes = value;
        self
    }

    /// Sets the in-memory decoded-tile cache budget.
    #[must_use]
    pub const fn tile_cache_bytes(mut self, value: NonZeroU64) -> Self {
        self.tile_cache_bytes = value;
        self
    }

    /// Sets the maximum number of vector-tile requests allowed in flight.
    #[must_use]
    pub const fn maximum_in_flight_tile_requests(mut self, value: NonZeroUsize) -> Self {
        self.maximum_in_flight_tile_requests = value;
        self
    }

    /// Sets the timeout applied to each style, `TileJSON`, and tile request.
    ///
    /// # Panics
    ///
    /// Panics if `value` is zero.
    #[must_use]
    pub const fn request_timeout(mut self, value: Duration) -> Self {
        assert!(!value.is_zero(), "GPU Map request timeout must be non-zero");
        self.request_timeout = value;
        self
    }

    /// Sets the bounded retry policy for transient network failures.
    #[must_use]
    pub const fn network_retry_policy(mut self, value: MapNetworkRetryPolicy) -> Self {
        self.network_retry_policy = value;
        self
    }

    /// Sets the transition used for programmatic camera changes.
    #[must_use]
    pub const fn camera_animation(mut self, value: Animation) -> Self {
        self.camera_animation = value;
        self
    }

    pub(crate) const fn style_url(&self) -> &Url {
        &self.style_url
    }

    pub(crate) const fn in_flight_tile_request_limit(&self) -> NonZeroUsize {
        self.maximum_in_flight_tile_requests
    }

    pub(crate) const fn network_request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub(crate) const fn retry_policy(&self) -> MapNetworkRetryPolicy {
        self.network_retry_policy
    }
}

/// Installs the GPU map hook for a self-drawn backend.
///
/// The application environment must contain [`MapGpuOptions`]. Keeping provider
/// configuration in the application prevents `WaterUI` from hard-coding a tile
/// service and lets native backends continue to use their platform map.
///
/// # Panics
///
/// The installed hook panics when a GPU map is realized without
/// [`MapGpuOptions`] in its environment.
pub fn install(env: &mut Environment) {
    env.insert_hook::<MapConfig, AnyView>(|env, mut config| {
        let options = env
            .get::<MapGpuOptions>()
            .expect("GPU Map requires MapGpuOptions in the application environment")
            .clone();
        if !config.interactivity.is_interactive() {
            return AnyView::new(render::map_surface(MapScene::new(config, options)));
        }

        let controller = Rc::new(MapGestureController::new(&config.region));
        config.region = Computed::new(controller.region.clone());
        let scene = render::map_surface_with_interaction(
            MapScene::with_viewport_signal(
                config,
                options,
                controller.viewport.clone(),
                Computed::new(controller.settled_region.clone()),
                Computed::new(controller.animate_camera_changes.clone()),
            ),
            Some(Rc::clone(&controller)),
        );
        let drag_controller = Rc::clone(&controller);
        let magnification_controller = controller;
        let scene = Metadata::new(
            scene,
            GestureObserver::new(
                DragGesture::new(MAP_DRAG_MINIMUM_DISTANCE),
                move |env: Environment| {
                    drag_controller.handle_drag(
                        env.get::<DragEvent>()
                            .expect("DragGesture observer must receive a DragEvent"),
                    );
                },
            ),
        );
        AnyView::new(Metadata::new(
            scene,
            GestureObserver::new(MagnificationGesture::new(1.0), move |env: Environment| {
                magnification_controller.handle_magnification(
                    env.get::<MagnificationEvent>()
                        .expect("MagnificationGesture observer must receive a MagnificationEvent"),
                );
            }),
        ))
    });
}

const MAX_MERCATOR_LATITUDE: f64 = 85.051_128_779_806_6;
const MAX_LATITUDE_DELTA: f64 = MAX_MERCATOR_LATITUDE * 2.0;
const MAX_LONGITUDE_DELTA: f64 = 360.0;
const MAP_ZOOM_LEVELS: u32 = 22;
const MAP_ZOOM_SCALE: f64 = (1_u32 << MAP_ZOOM_LEVELS) as f64;
const MIN_LATITUDE_DELTA: f64 = MAX_LATITUDE_DELTA / MAP_ZOOM_SCALE;
const MIN_LONGITUDE_DELTA: f64 = MAX_LONGITUDE_DELTA / MAP_ZOOM_SCALE;
const MAP_DRAG_MINIMUM_DISTANCE: f32 = 4.0;

pub(crate) struct MapGestureController {
    region: Binding<Region>,
    viewport: Binding<Option<(f32, f32)>>,
    gesture_origin: Binding<Region>,
    settled_region: Binding<Region>,
    animate_camera_changes: Binding<bool>,
    _source_guard: BoxWatcherGuard,
}

impl MapGestureController {
    fn new(source: &Computed<Region>) -> Self {
        let initial = source.get();
        let region = binding(initial);
        let settled_region = binding(initial);
        let animate_camera_changes = binding(false);
        let source_guard = source.watch({
            let region = region.clone();
            let settled_region = settled_region.clone();
            let animate_camera_changes = animate_camera_changes.clone();
            move |context| {
                let value = context.into_value();
                tracing::debug!(
                    latitude = value.center.latitude.get(),
                    longitude = value.center.longitude.get(),
                    latitude_delta = value.latitude_delta,
                    longitude_delta = value.longitude_delta,
                    "GPU Map received a programmatic camera update"
                );
                animate_camera_changes.set(true);
                region.set(value);
                settled_region.set(value);
            }
        });
        Self {
            region,
            viewport: binding(None),
            gesture_origin: binding(initial),
            settled_region,
            animate_camera_changes,
            _source_guard: source_guard,
        }
    }

    fn set_live_region(&self, region: Region) {
        self.animate_camera_changes.set(false);
        self.region.set(region);
    }

    fn settle_region(&self, region: Region) {
        self.set_live_region(region);
        self.settled_region.set(region);
    }

    fn handle_drag(&self, event: &DragEvent) {
        match event.phase {
            GesturePhase::Started => {
                self.animate_camera_changes.set(false);
                self.gesture_origin.set(self.region.get());
            }
            GesturePhase::Updated | GesturePhase::Ended => {
                let Some((width, height)) = self.viewport.get() else {
                    return;
                };
                let origin = self.gesture_origin.get();
                let region = translated_region(
                    origin,
                    event.translation.x,
                    event.translation.y,
                    f64::from(width),
                    f64::from(height),
                );
                match event.phase {
                    GesturePhase::Updated => self.set_live_region(region),
                    GesturePhase::Ended => self.settle_region(region),
                    _ => unreachable!("drag region is produced only while updating or ending"),
                }
            }
            GesturePhase::Cancelled => self.settle_region(self.region.get()),
        }
    }

    fn handle_magnification(&self, event: &MagnificationEvent) {
        match event.phase {
            GesturePhase::Started => {
                self.animate_camera_changes.set(false);
                self.gesture_origin.set(self.region.get());
            }
            GesturePhase::Updated | GesturePhase::Ended => {
                let Some((width, height)) = self.viewport.get() else {
                    return;
                };
                let origin = self.gesture_origin.get();
                let scale = f64::from(event.scale).max(f64::EPSILON);
                let latitude_delta =
                    (origin.latitude_delta / scale).clamp(MIN_LATITUDE_DELTA, MAX_LATITUDE_DELTA);
                let longitude_delta = (origin.longitude_delta / scale)
                    .clamp(MIN_LONGITUDE_DELTA, MAX_LONGITUDE_DELTA);
                let focal_x = f64::from(event.center.x / width) - 0.5;
                let focal_y = f64::from(event.center.y / height) - 0.5;
                let longitude = focal_x.mul_add(
                    origin.longitude_delta - longitude_delta,
                    origin.center.longitude.get(),
                );
                let latitude = (-focal_y).mul_add(
                    origin.latitude_delta - latitude_delta,
                    origin.center.latitude.get(),
                );
                let region = Region::new(
                    map_coordinate(latitude, longitude, latitude_delta),
                    latitude_delta,
                    longitude_delta,
                );
                match event.phase {
                    GesturePhase::Updated => self.set_live_region(region),
                    GesturePhase::Ended => self.settle_region(region),
                    _ => {
                        unreachable!(
                            "magnification region is produced only while updating or ending"
                        )
                    }
                }
            }
            GesturePhase::Cancelled => self.settle_region(self.region.get()),
        }
    }
}

fn translated_region(
    origin: Region,
    translation_x: f32,
    translation_y: f32,
    width: f64,
    height: f64,
) -> Region {
    let longitude = (f64::from(translation_x) / width)
        .mul_add(-origin.longitude_delta, origin.center.longitude.get());
    let latitude = (f64::from(translation_y) / height)
        .mul_add(origin.latitude_delta, origin.center.latitude.get());
    Region::new(
        map_coordinate(latitude, longitude, origin.latitude_delta),
        origin.latitude_delta,
        origin.longitude_delta,
    )
}

fn map_coordinate(latitude: f64, longitude: f64, latitude_delta: f64) -> Coordinate {
    let latitude_limit = latitude_delta.mul_add(-0.5, MAX_MERCATOR_LATITUDE).max(0.0);
    let latitude = latitude.clamp(-latitude_limit, latitude_limit);
    let longitude = (longitude + 180.0).rem_euclid(360.0) - 180.0;
    Coordinate::from_degrees(latitude, longitude)
        .expect("clamped map interaction coordinate must be valid")
}

/// Error returned while resolving styles, sources, or vector tiles.
#[derive(Debug, thiserror::Error)]
pub enum MapLoadError {
    /// A provider URL was invalid.
    #[error("invalid map URL {url}: {source}")]
    InvalidUrl {
        /// URL that failed to parse or resolve.
        url: String,
        /// URL parser error.
        source: url::ParseError,
    },
    /// An HTTP request failed.
    #[error("map request failed for {url}: {message}")]
    Request {
        /// Requested URL.
        url: String,
        /// Transport or HTTP failure.
        message: String,
    },
    /// A map provider returned an unsuccessful HTTP status.
    #[error("map request failed for {url}: HTTP {status}")]
    Http {
        /// Requested URL.
        url: String,
        /// HTTP response status.
        status: u16,
    },
    /// A response exceeded its configured bound.
    #[error("map response exceeded its configured limit for {url}: {message}")]
    ResponseLimit {
        /// Requested URL.
        url: String,
        /// Size-limit failure.
        message: String,
    },
    /// JSON was invalid.
    #[error("invalid map JSON from {url}: {source}")]
    Json {
        /// Requested URL.
        url: String,
        /// JSON parser error.
        source: serde_json::Error,
    },
    /// A `MapLibre` style expression was invalid.
    #[error("invalid expression for layer {layer} property {property}: {message}")]
    Expression {
        /// Style layer id.
        layer: String,
        /// Style property name.
        property: String,
        /// Parser or type-check failure.
        message: String,
    },
    /// A style or source violates the supported vector-map contract.
    #[error("unsupported map style construct: {0}")]
    Unsupported(String),
    /// A vector tile could not be decoded.
    #[error("invalid vector tile {z}/{x}/{y}: {message}")]
    VectorTile {
        /// Tile zoom.
        z: u8,
        /// Tile column.
        x: u32,
        /// Tile row.
        y: u32,
        /// Decoder failure.
        message: String,
    },
}

impl MapLoadError {
    const fn is_retryable(&self) -> bool {
        match self {
            Self::Request { .. } => true,
            Self::Http { status, .. } => {
                matches!(*status, 408 | 425 | 429) || (*status >= 500 && *status <= 599)
            }
            Self::InvalidUrl { .. }
            | Self::ResponseLimit { .. }
            | Self::Json { .. }
            | Self::Expression { .. }
            | Self::Unsupported(_)
            | Self::VectorTile { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::gesture::GesturePoint;

    fn manhattan_region() -> Region {
        Region::new(
            Coordinate::from_degrees(40.7580, -73.9855)
                .expect("Manhattan coordinate must be valid"),
            0.030,
            0.050,
        )
    }

    #[test]
    fn drag_updates_the_internal_camera_without_replacing_the_map() {
        let controller = MapGestureController::new(&Computed::constant(manhattan_region()));
        controller.viewport.set(Some((1_000.0, 500.0)));
        controller.handle_drag(&DragEvent {
            phase: GesturePhase::Started,
            location: GesturePoint::new(500.0, 250.0),
            translation: GesturePoint::new(0.0, 0.0),
            velocity: GesturePoint::new(0.0, 0.0),
        });
        controller.handle_drag(&DragEvent {
            phase: GesturePhase::Updated,
            location: GesturePoint::new(600.0, 300.0),
            translation: GesturePoint::new(100.0, 50.0),
            velocity: GesturePoint::new(0.0, 0.0),
        });

        let region = controller.region.get();
        assert!((region.center.latitude.get() - 40.761).abs() < 1e-9);
        assert!((region.center.longitude.get() + 73.9905).abs() < 1e-9);
        assert_eq!(controller.settled_region.get(), manhattan_region());

        controller.handle_drag(&DragEvent {
            phase: GesturePhase::Ended,
            location: GesturePoint::new(600.0, 300.0),
            translation: GesturePoint::new(100.0, 50.0),
            velocity: GesturePoint::new(0.0, 0.0),
        });
        assert_eq!(controller.settled_region.get(), controller.region.get());
        assert!(!controller.animate_camera_changes.get());
    }

    #[test]
    fn focal_zoom_preserves_the_center_and_halves_the_span() {
        let controller = MapGestureController::new(&Computed::constant(manhattan_region()));
        controller.viewport.set(Some((1_000.0, 500.0)));
        controller.handle_magnification(&MagnificationEvent {
            phase: GesturePhase::Started,
            center: GesturePoint::new(500.0, 250.0),
            scale: 1.0,
            velocity: 0.0,
        });
        controller.handle_magnification(&MagnificationEvent {
            phase: GesturePhase::Updated,
            center: GesturePoint::new(500.0, 250.0),
            scale: 2.0,
            velocity: 0.0,
        });

        let region = controller.region.get();
        assert_eq!(region.center, manhattan_region().center);
        assert!((region.latitude_delta - 0.015).abs() < 1e-12);
        assert!((region.longitude_delta - 0.025).abs() < 1e-12);
    }

    #[test]
    fn cancelled_magnification_keeps_the_last_visible_camera() {
        let controller = MapGestureController::new(&Computed::constant(manhattan_region()));
        controller.viewport.set(Some((1_000.0, 500.0)));
        controller.handle_magnification(&MagnificationEvent {
            phase: GesturePhase::Started,
            center: GesturePoint::new(500.0, 250.0),
            scale: 1.0,
            velocity: 0.0,
        });
        controller.handle_magnification(&MagnificationEvent {
            phase: GesturePhase::Updated,
            center: GesturePoint::new(500.0, 250.0),
            scale: 2.0,
            velocity: 0.0,
        });
        let visible = controller.region.get();

        controller.handle_magnification(&MagnificationEvent {
            phase: GesturePhase::Cancelled,
            center: GesturePoint::new(500.0, 250.0),
            scale: 2.0,
            velocity: 0.0,
        });

        assert_eq!(controller.region.get(), visible);
        assert_eq!(controller.settled_region.get(), visible);
    }

    #[test]
    fn source_camera_changes_animate_and_settle_immediately() {
        let source = binding(manhattan_region());
        let controller = MapGestureController::new(&Computed::new(source.clone()));
        let target = Region::new(manhattan_region().center, 0.015, 0.025);

        source.set(target);

        assert_eq!(controller.region.get(), target);
        assert_eq!(controller.settled_region.get(), target);
        assert!(controller.animate_camera_changes.get());
    }
}
