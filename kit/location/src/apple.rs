use std::fmt;
use std::sync::{Arc, Mutex};

use log::error;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{
    BeaconConstraint, HeadingConfig, LocationBackend, LocationDelegate, LocationError,
    LocationEvent, LocationResult, Region, SignificantUpdateConfig, StandardUpdateConfig,
};

#[swift_bridge::bridge]
mod ffi {
    extern "Rust" {
        type EventRelay;

        fn dispatch_event(self: &EventRelay, json_event: String);
    }

    extern "Swift" {
        type AppleLocationManager;

        #[swift_bridge(init)]
        fn new(callback: Box<EventRelay>) -> AppleLocationManager;

        fn configure_standard(self: &AppleLocationManager, config_json: String);
        fn configure_significant(self: &AppleLocationManager, config_json: String);
        fn configure_heading(self: &AppleLocationManager, config_json: String);
        fn start_standard(self: &AppleLocationManager);
        fn stop_standard(self: &AppleLocationManager);
        fn start_significant(self: &AppleLocationManager);
        fn stop_significant(self: &AppleLocationManager);
        fn monitor_regions(self: &AppleLocationManager, regions_json: String);
        fn stop_monitoring_regions(self: &AppleLocationManager);
        fn range_beacons(self: &AppleLocationManager, constraints_json: String);
        fn stop_ranging_beacons(self: &AppleLocationManager);
        fn start_heading_updates(self: &AppleLocationManager);
        fn stop_heading_updates(self: &AppleLocationManager);
    }
}

/// Backend implementation backed by the Apple CoreLocation stack via swift-bridge.
#[derive(Default)]
pub struct AppleLocationBackend {
    manager: Mutex<Option<ffi::AppleLocationManager>>,
    delegate: Mutex<Option<Arc<dyn LocationDelegate>>>,
    standard_config: Mutex<Option<StandardUpdateConfig>>,
    significant_config: Mutex<Option<SignificantUpdateConfig>>,
    heading_config: Mutex<Option<HeadingConfig>>,
    monitored_regions: Mutex<Vec<Region>>,
    beacon_constraints: Mutex<Vec<BeaconConstraint>>,
}

impl fmt::Debug for AppleLocationBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppleLocationBackend").finish()
    }
}

impl AppleLocationBackend {
    /// Construct a backend without a delegate. A delegate must be registered before starting updates.
    pub fn new() -> Self {
        Self::default()
    }

    fn with_manager<F>(&self, action: F) -> LocationResult<()>
    where
        F: FnOnce(&ffi::AppleLocationManager) -> LocationResult<()>,
    {
        let guard = self.manager.lock().expect("poisoned manager mutex");
        if let Some(manager) = guard.as_ref() {
            action(manager)
        } else {
            Err(LocationError::DelegateMissing)
        }
    }

    fn restore_state(&self) {
        let standard = self
            .standard_config
            .lock()
            .expect("poisoned standard config")
            .clone();
        let significant = self
            .significant_config
            .lock()
            .expect("poisoned significant config")
            .clone();
        let heading = self
            .heading_config
            .lock()
            .expect("poisoned heading config")
            .clone();
        let regions = self
            .monitored_regions
            .lock()
            .expect("poisoned regions")
            .clone();
        let beacons = self
            .beacon_constraints
            .lock()
            .expect("poisoned beacons")
            .clone();

        if let Err(err) = self.with_manager(|manager| {
            if let Some(config) = &standard {
                self.apply_standard(manager, config)?;
            }
            if let Some(config) = &significant {
                self.apply_significant(manager, config)?;
            }
            if let Some(config) = &heading {
                self.apply_heading(manager, config)?;
            }
            if !regions.is_empty() {
                self.apply_regions(manager, &regions)?;
            }
            if !beacons.is_empty() {
                self.apply_beacons(manager, &beacons)?;
            }
            Ok(())
        }) {
            error!("failed to restore Apple location backend state: {err}");
        }
    }

    fn apply_standard(
        &self,
        manager: &ffi::AppleLocationManager,
        config: &StandardUpdateConfig,
    ) -> LocationResult<()> {
        let json = to_json(config)?;
        manager.configure_standard(json);
        Ok(())
    }

    fn apply_significant(
        &self,
        manager: &ffi::AppleLocationManager,
        config: &SignificantUpdateConfig,
    ) -> LocationResult<()> {
        let json = to_json(config)?;
        manager.configure_significant(json);
        Ok(())
    }

    fn apply_heading(
        &self,
        manager: &ffi::AppleLocationManager,
        config: &HeadingConfig,
    ) -> LocationResult<()> {
        let json = to_json(config)?;
        manager.configure_heading(json);
        Ok(())
    }

    fn apply_regions(
        &self,
        manager: &ffi::AppleLocationManager,
        regions: &[Region],
    ) -> LocationResult<()> {
        let json = to_json(regions)?;
        manager.monitor_regions(json);
        Ok(())
    }

    fn apply_beacons(
        &self,
        manager: &ffi::AppleLocationManager,
        constraints: &[BeaconConstraint],
    ) -> LocationResult<()> {
        let json = to_json(constraints)?;
        manager.range_beacons(json);
        Ok(())
    }
}

impl LocationBackend for AppleLocationBackend {
    fn set_delegate(&self, delegate: Arc<dyn LocationDelegate>) {
        {
            let mut guard = self.delegate.lock().expect("poisoned delegate mutex");
            *guard = Some(delegate.clone());
        }

        let relay = EventRelay::new(delegate);
        let manager = ffi::AppleLocationManager::new(Box::new(relay));

        {
            let mut guard = self.manager.lock().expect("poisoned manager mutex");
            *guard = Some(manager);
        }

        self.restore_state();
    }

    fn configure_standard_updates(&self, config: StandardUpdateConfig) -> LocationResult<()> {
        {
            let mut guard = self
                .standard_config
                .lock()
                .expect("poisoned standard config");
            *guard = Some(config.clone());
        }

        self.with_manager(|manager| self.apply_standard(manager, &config))
    }

    fn start_standard_updates(&self) -> LocationResult<()> {
        self.with_manager(|manager| {
            manager.start_standard();
            Ok(())
        })
    }

    fn stop_standard_updates(&self) {
        if let Err(err) = self.with_manager(|manager| {
            manager.stop_standard();
            Ok(())
        }) {
            error!("failed to stop standard updates: {err}");
        }
    }

    fn configure_significant_updates(&self, config: SignificantUpdateConfig) -> LocationResult<()> {
        {
            let mut guard = self
                .significant_config
                .lock()
                .expect("poisoned significant config");
            *guard = Some(config.clone());
        }

        self.with_manager(|manager| self.apply_significant(manager, &config))
    }

    fn start_significant_updates(&self) -> LocationResult<()> {
        self.with_manager(|manager| {
            manager.start_significant();
            Ok(())
        })
    }

    fn stop_significant_updates(&self) {
        if let Err(err) = self.with_manager(|manager| {
            manager.stop_significant();
            Ok(())
        }) {
            error!("failed to stop significant updates: {err}");
        }
    }

    fn monitor_regions(&self, regions: Vec<Region>) -> LocationResult<()> {
        {
            let mut guard = self
                .monitored_regions
                .lock()
                .expect("poisoned regions mutex");
            *guard = regions.clone();
        }

        self.with_manager(|manager| self.apply_regions(manager, &regions))
    }

    fn stop_monitoring_regions(&self) {
        {
            let mut guard = self
                .monitored_regions
                .lock()
                .expect("poisoned regions mutex");
            guard.clear();
        }

        if let Err(err) = self.with_manager(|manager| {
            manager.stop_monitoring_regions();
            Ok(())
        }) {
            error!("failed to stop monitoring regions: {err}");
        }
    }

    fn range_beacons(&self, constraints: Vec<BeaconConstraint>) -> LocationResult<()> {
        {
            let mut guard = self
                .beacon_constraints
                .lock()
                .expect("poisoned beacon mutex");
            *guard = constraints.clone();
        }

        self.with_manager(|manager| self.apply_beacons(manager, &constraints))
    }

    fn stop_ranging_beacons(&self) {
        {
            let mut guard = self
                .beacon_constraints
                .lock()
                .expect("poisoned beacon mutex");
            guard.clear();
        }

        if let Err(err) = self.with_manager(|manager| {
            manager.stop_ranging_beacons();
            Ok(())
        }) {
            error!("failed to stop ranging beacons: {err}");
        }
    }

    fn configure_heading_updates(&self, config: HeadingConfig) -> LocationResult<()> {
        {
            let mut guard = self.heading_config.lock().expect("poisoned heading config");
            *guard = Some(config.clone());
        }

        self.with_manager(|manager| self.apply_heading(manager, &config))
    }

    fn start_heading_updates(&self) -> LocationResult<()> {
        self.with_manager(|manager| {
            manager.start_heading_updates();
            Ok(())
        })
    }

    fn stop_heading_updates(&self) {
        if let Err(err) = self.with_manager(|manager| {
            manager.stop_heading_updates();
            Ok(())
        }) {
            error!("failed to stop heading updates: {err}");
        }
    }
}

struct EventRelay {
    delegate: Arc<dyn LocationDelegate>,
}

impl EventRelay {
    fn new(delegate: Arc<dyn LocationDelegate>) -> Self {
        Self { delegate }
    }

    fn dispatch_event(&self, json_event: String) {
        match from_json::<LocationEvent>(&json_event) {
            Ok(event) => self.delegate.on_event(event),
            Err(err) => {
                let error = LocationError::Serialization {
                    message: err.to_string(),
                };
                self.delegate.on_event(LocationEvent::Error { error });
            }
        }
    }
}

impl fmt::Debug for EventRelay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventRelay").finish()
    }
}

fn to_json<T: Serialize>(value: &T) -> LocationResult<String> {
    serde_json::to_string(value).map_err(|err| LocationError::Serialization {
        message: err.to_string(),
    })
}

fn from_json<T: DeserializeOwned>(value: &str) -> LocationResult<T> {
    serde_json::from_str(value).map_err(|err| LocationError::Serialization {
        message: err.to_string(),
    })
}
