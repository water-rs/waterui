//! Cross-platform abstractions for location services used throughout `WaterKit`.

#![deny(missing_debug_implementations)]

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Reactive location manager for working with location services.
pub mod reactive;
pub use reactive::{LocationSignals, ReactiveLocationManager};

/// Result type used by the location crate.
pub type LocationResult<T> = Result<T, LocationError>;

/// Latitude/longitude coordinate pair.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Coordinate {
    /// Latitude in degrees.
    pub latitude: f64,
    /// Longitude in degrees.
    pub longitude: f64,
}

/// Sample representing a single location update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocationSample {
    /// The coordinate of the user.
    pub coordinate: Coordinate,
    /// Altitude in meters above sea level, when known.
    pub altitude: Option<f64>,
    /// Horizontal accuracy in meters.
    pub horizontal_accuracy: Option<f64>,
    /// Vertical accuracy in meters.
    pub vertical_accuracy: Option<f64>,
    /// Course in degrees relative to true north.
    pub course: Option<f64>,
    /// Speed in meters per second.
    pub speed: Option<f64>,
    /// Timestamp associated with the reading.
    #[serde(with = "serde_time")]
    pub timestamp: SystemTime,
}

/// Requested accuracy bucket for updates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocationAccuracy {
    /// Best possible accuracy, suitable for navigation.
    Navigation,
    /// Prioritize battery by delivering only high accuracy samples.
    Best,
    /// Approximately ten meter accuracy.
    TenMeters,
    /// Approximately one hundred meter accuracy.
    HundredMeters,
    /// Approximately one kilometer accuracy.
    Kilometer,
    /// Approximately three kilometer accuracy.
    ThreeKilometers,
}

/// Activity hint to influence the hardware on Apple platforms.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    /// General navigation when the activity is unknown.
    Other,
    /// Automotive navigation.
    AutomotiveNavigation,
    /// Fitness workout.
    Fitness,
    /// Other vehicular navigation such as trains or buses.
    OtherNavigation,
}

/// Configuration describing standard location updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandardUpdateConfig {
    /// Desired accuracy.
    pub accuracy: LocationAccuracy,
    /// Minimum distance in meters before the delegate is notified.
    pub distance_filter_meters: Option<f64>,
    /// Allow the backend to deliver updates while the app is in background.
    pub allow_background_updates: bool,
    /// Hint for the system on expected movement activity.
    pub activity_type: ActivityType,
    /// When true the system can pause location updates automatically.
    pub pause_automatically: bool,
}

impl Default for StandardUpdateConfig {
    fn default() -> Self {
        Self {
            accuracy: LocationAccuracy::Best,
            distance_filter_meters: Some(5.0),
            allow_background_updates: false,
            activity_type: ActivityType::Other,
            pause_automatically: true,
        }
    }
}

/// Configuration describing significant change monitoring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignificantUpdateConfig {
    /// Desired accuracy for the significant change monitor.
    pub accuracy: LocationAccuracy,
    /// Whether updates are permitted in the background.
    pub allow_background_updates: bool,
}

impl Default for SignificantUpdateConfig {
    fn default() -> Self {
        Self {
            accuracy: LocationAccuracy::HundredMeters,
            allow_background_updates: true,
        }
    }
}

/// Circular region that can be monitored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Region {
    /// Unique identifier for the region.
    pub identifier: String,
    /// Region center.
    pub center: Coordinate,
    /// Region radius in meters.
    pub radius: f64,
    /// Whether the device should notify on entry.
    pub notify_on_entry: bool,
    /// Whether the device should notify on exit.
    pub notify_on_exit: bool,
}

/// Event type describing entry or exit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegionEventKind {
    /// The device entered the monitored region.
    Enter,
    /// The device exited the monitored region.
    Exit,
}

/// Event triggered when the device crosses a monitored region boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegionEvent {
    /// Region that triggered the event.
    pub region: Region,
    /// Whether the event was entry or exit.
    pub kind: RegionEventKind,
    /// Timestamp for the event.
    #[serde(with = "serde_time")]
    pub timestamp: SystemTime,
}

/// Constraint used to range iBeacons.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeaconConstraint {
    /// Beacon UUID.
    pub uuid: String,
    /// Optional major value.
    pub major: Option<u16>,
    /// Optional minor value.
    pub minor: Option<u16>,
}

/// Qualitative proximity of a ranged beacon.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BeaconProximity {
    /// Device is in close range of the beacon.
    Immediate,
    /// Device is near the beacon.
    Near,
    /// Device is far from the beacon.
    Far,
    /// Unable to determine proximity.
    Unknown,
}

/// Reading captured during beacon ranging.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeaconReading {
    /// Constraint that describes the beacon.
    pub constraint: BeaconConstraint,
    /// Estimated distance classification.
    pub proximity: BeaconProximity,
    /// Accuracy estimate in meters.
    pub accuracy: Option<f64>,
    /// RSSI strength reported by `CoreLocation`.
    pub rssi: Option<i16>,
    /// Timestamp for when the reading was captured.
    #[serde(with = "serde_time")]
    pub timestamp: SystemTime,
}

/// Whether the beacon was detected or lost.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BeaconEventKind {
    /// Beacon was detected or updated.
    Ranged,
    /// Beacon is no longer detected.
    Lost,
}

/// Event produced while ranging beacons.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeaconEvent {
    /// The ranged beacon information.
    pub reading: BeaconReading,
    /// Kind of event that occurred.
    pub kind: BeaconEventKind,
}

/// Configuration for heading updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeadingConfig {
    /// Minimum heading delta in degrees before a delegate is notified.
    pub minimum_delta_degrees: Option<f64>,
    /// Allow calibration UI prompts if necessary.
    pub allow_calibration: bool,
}

impl Default for HeadingConfig {
    fn default() -> Self {
        Self {
            minimum_delta_degrees: Some(5.0),
            allow_calibration: true,
        }
    }
}

/// Compass heading sample.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Heading {
    /// Direction relative to magnetic north in degrees.
    pub magnetic_heading: f64,
    /// Optional direction relative to true north.
    pub true_heading: Option<f64>,
    /// Accuracy in degrees.
    pub accuracy: f64,
    /// Timestamp for the heading update.
    #[serde(with = "serde_time")]
    pub timestamp: SystemTime,
}

/// High level events delivered to the delegate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocationEvent {
    /// Standard location update.
    StandardUpdate {
        /// The location sample.
        sample: LocationSample,
    },
    /// Significant change in location was detected.
    SignificantUpdate {
        /// The location sample.
        sample: LocationSample,
    },
    /// Region entry event.
    RegionEvent {
        /// The region event.
        event: RegionEvent,
    },
    /// Beacon ranging event.
    BeaconEvent {
        /// The beacon event.
        event: BeaconEvent,
    },
    /// Updated heading sample.
    Heading {
        /// The heading information.
        heading: Heading,
    },
    /// Backend error surfaced to the delegate.
    Error {
        /// The error that occurred.
        error: LocationError,
    },
}

/// Errors produced by backends and exposed to consumers.
#[derive(Debug, Clone, Serialize, Deserialize, Error, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocationError {
    /// Delegate has not been set on the backend.
    #[error("delegate has not been registered")]
    DelegateMissing,
    /// Backend is unavailable for the current platform.
    #[error("backend is unavailable")]
    BackendUnavailable,
    /// Serialization/deserialization failure.
    #[error("serialization error: {message}")]
    Serialization {
        /// The error message.
        message: String,
    },
    /// Underlying platform reported an error.
    #[error("platform error: {message}")]
    Platform {
        /// The error message.
        message: String,
    },
}

impl From<serde_json::Error> for LocationError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization {
            message: err.to_string(),
        }
    }
}

/// Delegate that receives location events from the backend.
pub trait LocationDelegate: Send + Sync + 'static {
    /// Handle a new event dispatched by the backend.
    fn on_event(&self, event: LocationEvent);
}

/// Trait implemented by platform backends that provide location information.
pub trait LocationBackend: Send + Sync + 'static {
    /// Register the delegate that should receive events.
    fn set_delegate(&self, delegate: Arc<dyn LocationDelegate>);
    /// Configure standard location updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    fn configure_standard_updates(&self, config: StandardUpdateConfig) -> LocationResult<()>;
    /// Start delivering standard updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    fn start_standard_updates(&self) -> LocationResult<()>;
    /// Stop delivering standard updates.
    fn stop_standard_updates(&self);
    /// Configure significant location updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    fn configure_significant_updates(&self, config: SignificantUpdateConfig) -> LocationResult<()>;
    /// Start significant location updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    fn start_significant_updates(&self) -> LocationResult<()>;
    /// Stop significant location updates.
    fn stop_significant_updates(&self);
    /// Begin monitoring a set of regions.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start monitoring.
    fn monitor_regions(&self, regions: Vec<Region>) -> LocationResult<()>;
    /// Stop monitoring all regions.
    fn stop_monitoring_regions(&self);
    /// Begin ranging beacons that satisfy the provided constraints.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start ranging.
    fn range_beacons(&self, constraints: Vec<BeaconConstraint>) -> LocationResult<()>;
    /// Stop ranging beacons.
    fn stop_ranging_beacons(&self);
    /// Configure heading updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    fn configure_heading_updates(&self, config: HeadingConfig) -> LocationResult<()>;
    /// Start heading updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    fn start_heading_updates(&self) -> LocationResult<()>;
    /// Stop heading updates.
    fn stop_heading_updates(&self);
}

/// Convenience wrapper for working with a backend implementation.
#[derive(Clone)]
pub struct LocationManager {
    backend: Arc<dyn LocationBackend>,
}

impl LocationManager {
    /// Create a new manager with the specified backend.
    pub fn new(backend: Arc<dyn LocationBackend>) -> Self {
        Self { backend }
    }

    /// Access the underlying backend.
    #[must_use] 
    pub fn backend(&self) -> &Arc<dyn LocationBackend> {
        &self.backend
    }

    /// Register the delegate.
    pub fn set_delegate(&self, delegate: Arc<dyn LocationDelegate>) {
        self.backend.set_delegate(delegate);
    }

    /// Configure standard updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    pub fn configure_standard_updates(&self, config: StandardUpdateConfig) -> LocationResult<()> {
        self.backend.configure_standard_updates(config)
    }

    /// Start standard updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    pub fn start_standard_updates(&self) -> LocationResult<()> {
        self.backend.start_standard_updates()
    }

    /// Stop standard updates.
    pub fn stop_standard_updates(&self) {
        self.backend.stop_standard_updates();
    }

    /// Configure significant updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    pub fn configure_significant_updates(
        &self,
        config: SignificantUpdateConfig,
    ) -> LocationResult<()> {
        self.backend.configure_significant_updates(config)
    }

    /// Start significant updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    pub fn start_significant_updates(&self) -> LocationResult<()> {
        self.backend.start_significant_updates()
    }

    /// Stop significant updates.
    pub fn stop_significant_updates(&self) {
        self.backend.stop_significant_updates();
    }

    /// Monitor regions.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start monitoring.
    pub fn monitor_regions(&self, regions: Vec<Region>) -> LocationResult<()> {
        self.backend.monitor_regions(regions)
    }

    /// Stop monitoring regions.
    pub fn stop_monitoring_regions(&self) {
        self.backend.stop_monitoring_regions();
    }

    /// Start ranging beacons.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start ranging.
    pub fn range_beacons(&self, constraints: Vec<BeaconConstraint>) -> LocationResult<()> {
        self.backend.range_beacons(constraints)
    }

    /// Stop ranging beacons.
    pub fn stop_ranging_beacons(&self) {
        self.backend.stop_ranging_beacons();
    }

    /// Configure heading updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    pub fn configure_heading_updates(&self, config: HeadingConfig) -> LocationResult<()> {
        self.backend.configure_heading_updates(config)
    }

    /// Start heading updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    pub fn start_heading_updates(&self) -> LocationResult<()> {
        self.backend.start_heading_updates()
    }

    /// Stop heading updates.
    pub fn stop_heading_updates(&self) {
        self.backend.stop_heading_updates();
    }
}

impl fmt::Debug for LocationManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocationManager").finish()
    }
}

mod serde_time {
    use super::{Deserialize, Duration, SystemTime, UNIX_EPOCH};
    use serde::{Deserializer, Serializer};

    const MICROS_PER_SECOND: i128 = 1_000_000;

    #[allow(clippy::cast_possible_wrap)]
    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match time.duration_since(UNIX_EPOCH) {
            Ok(duration) => serializer.serialize_i128(duration.as_micros() as i128),
            Err(err) => serializer.serialize_i128(-(err.duration().as_micros() as i128)),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let micros = i128::deserialize(deserializer)?;
        if micros >= 0 {
            Ok(UNIX_EPOCH + micros_to_duration(micros))
        } else {
            Ok(UNIX_EPOCH - micros_to_duration(-micros))
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    const fn micros_to_duration(micros: i128) -> Duration {
        let seconds = (micros / MICROS_PER_SECOND) as u64;
        let remainder = micros % MICROS_PER_SECOND;
        let nanos = (remainder.unsigned_abs() as u32) * 1_000;
        Duration::new(seconds, nanos)
    }
}

#[cfg(feature = "apple")]
mod apple;

#[cfg(feature = "apple")]
pub use apple::AppleLocationBackend;

#[cfg(feature = "android")]
mod android;

#[cfg(feature = "android")]
pub use android::AndroidLocationBackend;
