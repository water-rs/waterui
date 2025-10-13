use std::fmt;
use std::sync::{Arc, Mutex};

use async_channel::{Receiver, Sender, unbounded};
use executor_core::{DefaultExecutor, LocalExecutor, Task};
use log::warn;
use nami::Signal;
use nami::binding::{Container, CustomBinding};

use crate::{
    BeaconConstraint, BeaconEvent, Heading, HeadingConfig, LocationBackend, LocationDelegate,
    LocationError, LocationEvent, LocationManager, LocationResult, LocationSample, Region,
    RegionEvent, SignificantUpdateConfig, StandardUpdateConfig,
};

const MAX_EVENT_HISTORY: usize = 256;
const MAX_ERROR_HISTORY: usize = 64;

/// Reactive signals for location events.
#[derive(Debug, Clone, Default)]
pub struct LocationSignals {
    history: Container<Vec<LocationEvent>>,
    latest_event: Container<Option<LocationEvent>>,
    latest_standard: Container<Option<LocationSample>>,
    latest_significant: Container<Option<LocationSample>>,
    latest_region: Container<Option<RegionEvent>>,
    latest_beacon: Container<Option<BeaconEvent>>,
    latest_heading: Container<Option<Heading>>,
    last_error: Container<Option<LocationError>>,
    error_history: Container<Vec<LocationError>>,
}

impl LocationSignals {
    /// Returns a signal for the event history.
    #[must_use]
    pub fn history(&self) -> impl Signal<Output = Vec<LocationEvent>> {
        self.history.clone()
    }

    /// Returns a signal for the latest event.
    #[must_use]
    pub fn latest_event(&self) -> impl Signal<Output = Option<LocationEvent>> {
        self.latest_event.clone()
    }

    /// Returns a signal for the latest standard location sample.
    #[must_use]
    pub fn latest_standard(&self) -> impl Signal<Output = Option<LocationSample>> {
        self.latest_standard.clone()
    }

    /// Returns a signal for the latest significant location sample.
    #[must_use]
    pub fn latest_significant(&self) -> impl Signal<Output = Option<LocationSample>> {
        self.latest_significant.clone()
    }

    /// Returns a signal for the latest region event.
    #[must_use]
    pub fn latest_region(&self) -> impl Signal<Output = Option<RegionEvent>> {
        self.latest_region.clone()
    }

    /// Returns a signal for the latest beacon event.
    #[must_use]
    pub fn latest_beacon(&self) -> impl Signal<Output = Option<BeaconEvent>> {
        self.latest_beacon.clone()
    }

    /// Returns a signal for the latest heading.
    #[must_use]
    pub fn latest_heading(&self) -> impl Signal<Output = Option<Heading>> {
        self.latest_heading.clone()
    }

    /// Returns a signal for the last error.
    #[must_use]
    pub fn last_error(&self) -> impl Signal<Output = Option<LocationError>> {
        self.last_error.clone()
    }

    /// Returns a signal for the error history.
    #[must_use]
    pub fn error_history(&self) -> impl Signal<Output = Vec<LocationError>> {
        self.error_history.clone()
    }

    /// Clears all signals and history.
    pub fn clear(&self) {
        self.history.set(Vec::new());
        self.error_history.set(Vec::new());
        self.latest_event.set(None);
        self.latest_standard.set(None);
        self.latest_significant.set(None);
        self.latest_region.set(None);
        self.latest_beacon.set(None);
        self.latest_heading.set(None);
        self.last_error.set(None);
    }

    fn record_event(&self, event: &LocationEvent) {
        let event_clone = event.clone();
        self.latest_event.set(Some(event_clone.clone()));

        match event {
            LocationEvent::StandardUpdate { sample } => {
                self.latest_standard.set(Some(sample.clone()));
            }
            LocationEvent::SignificantUpdate { sample } => {
                self.latest_significant.set(Some(sample.clone()));
            }
            LocationEvent::RegionEvent { event } => {
                self.latest_region.set(Some(event.clone()));
            }
            LocationEvent::BeaconEvent { event } => {
                self.latest_beacon.set(Some(event.clone()));
            }
            LocationEvent::Heading { heading } => {
                self.latest_heading.set(Some(heading.clone()));
            }
            LocationEvent::Error { error } => {
                let mut errors = self.error_history.get();
                let error_clone = error.clone();
                errors.push(error_clone.clone());
                if errors.len() > MAX_ERROR_HISTORY {
                    let overflow = errors.len() - MAX_ERROR_HISTORY;
                    errors.drain(0..overflow);
                }
                self.error_history.set(errors);
                self.last_error.set(Some(error_clone));
            }
        }

        let mut history = self.history.get();
        history.push(event_clone);
        if history.len() > MAX_EVENT_HISTORY {
            let overflow = history.len() - MAX_EVENT_HISTORY;
            history.drain(0..overflow);
        }
        self.history.set(history);
    }
}

struct ChannelLocationDelegate {
    sender: Sender<LocationEvent>,
    forward: Mutex<Option<Arc<dyn LocationDelegate>>>,
}

impl ChannelLocationDelegate {
    fn new(sender: Sender<LocationEvent>) -> Self {
        Self {
            sender,
            forward: Mutex::new(None),
        }
    }

    fn set_forward_delegate(&self, delegate: Option<Arc<dyn LocationDelegate>>) {
        let mut guard = self
            .forward
            .lock()
            .expect("forward delegate mutex poisoned");
        *guard = delegate;
    }
}

impl fmt::Debug for ChannelLocationDelegate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelLocationDelegate").finish()
    }
}

impl LocationDelegate for ChannelLocationDelegate {
    fn on_event(&self, event: LocationEvent) {
        if let Err(err) = self.sender.try_send(event.clone()) {
            warn!("dropping location event: {err}");
        }

        let forward = {
            let guard = self
                .forward
                .lock()
                .expect("forward delegate mutex poisoned");
            guard.clone()
        };

        if let Some(delegate) = forward {
            delegate.on_event(event);
        }
    }
}

/// A reactive wrapper around `LocationManager` that provides reactive signals.
#[derive(Debug, Clone)]
pub struct ReactiveLocationManager {
    manager: LocationManager,
    delegate: Arc<ChannelLocationDelegate>,
    signals: LocationSignals,
}

impl ReactiveLocationManager {
    /// Creates a new reactive location manager with the given backend.
    pub fn new(backend: Arc<dyn LocationBackend>) -> Self {
        let manager = LocationManager::new(backend);
        let (sender, receiver) = unbounded();
        let delegate = Arc::new(ChannelLocationDelegate::new(sender));
        manager.set_delegate(delegate.clone());

        let signals = LocationSignals::default();
        spawn_signal_pump(receiver, signals.clone());

        Self {
            manager,
            delegate,
            signals,
        }
    }

    /// Returns the location signals.
    #[must_use]
    pub fn signals(&self) -> LocationSignals {
        self.signals.clone()
    }

    /// Clears all signals and history.
    pub fn clear_signals(&self) {
        self.signals.clear();
    }

    /// Sets a delegate to receive location events.
    pub fn set_delegate(&self, delegate: Arc<dyn LocationDelegate>) {
        self.delegate.set_forward_delegate(Some(delegate));
    }

    /// Clears the delegate.
    pub fn clear_delegate(&self) {
        self.delegate.set_forward_delegate(None);
    }

    /// Configures standard location updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    pub fn configure_standard_updates(&self, config: StandardUpdateConfig) -> LocationResult<()> {
        self.manager.configure_standard_updates(config)
    }

    /// Starts standard location updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    pub fn start_standard_updates(&self) -> LocationResult<()> {
        self.manager.start_standard_updates()
    }

    /// Stops standard location updates.
    pub fn stop_standard_updates(&self) {
        self.manager.stop_standard_updates();
    }

    /// Configures significant location updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    pub fn configure_significant_updates(
        &self,
        config: SignificantUpdateConfig,
    ) -> LocationResult<()> {
        self.manager.configure_significant_updates(config)
    }

    /// Starts significant location updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    pub fn start_significant_updates(&self) -> LocationResult<()> {
        self.manager.start_significant_updates()
    }

    /// Stops significant location updates.
    pub fn stop_significant_updates(&self) {
        self.manager.stop_significant_updates();
    }

    /// Begins monitoring regions.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start monitoring.
    pub fn monitor_regions(&self, regions: Vec<Region>) -> LocationResult<()> {
        self.manager.monitor_regions(regions)
    }

    /// Stops monitoring regions.
    pub fn stop_monitoring_regions(&self) {
        self.manager.stop_monitoring_regions();
    }

    /// Begins ranging beacons.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start ranging.
    pub fn range_beacons(&self, constraints: Vec<BeaconConstraint>) -> LocationResult<()> {
        self.manager.range_beacons(constraints)
    }

    /// Stops ranging beacons.
    pub fn stop_ranging_beacons(&self) {
        self.manager.stop_ranging_beacons();
    }

    /// Configures heading updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to configure updates.
    pub fn configure_heading_updates(&self, config: HeadingConfig) -> LocationResult<()> {
        self.manager.configure_heading_updates(config)
    }

    /// Starts heading updates.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend fails to start updates.
    pub fn start_heading_updates(&self) -> LocationResult<()> {
        self.manager.start_heading_updates()
    }

    /// Stops heading updates.
    pub fn stop_heading_updates(&self) {
        self.manager.stop_heading_updates();
    }
}

fn spawn_signal_pump(receiver: Receiver<LocationEvent>, signals: LocationSignals) {
    DefaultExecutor
        .spawn_local(async move {
            while let Ok(event) = receiver.recv().await {
                signals.record_event(&event);
            }
        })
        .detach();
}
