use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::jlong;
use jni::{JNIEnv, JavaVM};
use log::error;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{
    BeaconConstraint, HeadingConfig, LocationBackend, LocationDelegate, LocationError,
    LocationEvent, LocationResult, Region, SignificantUpdateConfig, StandardUpdateConfig,
};

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static DELEGATES: OnceLock<Mutex<HashMap<u64, Arc<dyn LocationDelegate>>>> = OnceLock::new();

fn delegates() -> &'static Mutex<HashMap<u64, Arc<dyn LocationDelegate>>> {
    DELEGATES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Backend implementation backed by an Android Java/Kotlin bridge via JNI.
pub struct AndroidLocationBackend {
    vm: JavaVM,
    manager: GlobalRef,
    handle: u64,
    delegate: Mutex<Option<Arc<dyn LocationDelegate>>>,
    standard_config: Mutex<Option<StandardUpdateConfig>>,
    significant_config: Mutex<Option<SignificantUpdateConfig>>,
    heading_config: Mutex<Option<HeadingConfig>>,
    monitored_regions: Mutex<Vec<Region>>,
    beacon_constraints: Mutex<Vec<BeaconConstraint>>,
}

impl fmt::Debug for AndroidLocationBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AndroidLocationBackend")
            .field("handle", &self.handle)
            .finish()
    }
}

impl AndroidLocationBackend {
    /// Create a new backend from an Android `LocationManager` bridge object.
    pub fn new(env: &JNIEnv<'_>, manager: JObject<'_>) -> LocationResult<Self> {
        let vm = env.get_java_vm().map_err(map_jni_error)?;
        let global = env.new_global_ref(manager).map_err(map_jni_error)?;
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);

        Ok(Self {
            vm,
            manager: global,
            handle,
            delegate: Mutex::new(None),
            standard_config: Mutex::new(None),
            significant_config: Mutex::new(None),
            heading_config: Mutex::new(None),
            monitored_regions: Mutex::new(Vec::new()),
            beacon_constraints: Mutex::new(Vec::new()),
        })
    }

    /// Native handle associated with this backend instance for callbacks from Java/Kotlin.
    pub fn handle(&self) -> u64 {
        self.handle
    }

    fn with_attached_env<F>(&self, action: F) -> LocationResult<()>
    where
        F: FnOnce(&mut JNIEnv<'_>, JObject<'_>) -> jni::errors::Result<()>,
    {
        let mut env = self.vm.attach_current_thread().map_err(map_jni_error)?;
        let manager = self.manager.as_obj();
        action(&mut env, manager).map_err(map_jni_error)
    }

    fn with_manager<F>(&self, action: F) -> LocationResult<()>
    where
        F: FnOnce(&mut JNIEnv<'_>, JObject<'_>) -> jni::errors::Result<()>,
    {
        if self
            .delegate
            .lock()
            .expect("delegate mutex poisoned")
            .is_none()
        {
            return Err(LocationError::DelegateMissing);
        }

        self.with_attached_env(action)
    }

    fn register_handle(&self) -> LocationResult<()> {
        self.with_attached_env(|env, manager| {
            let args = [JValue::Long(self.handle as jlong)];
            env.call_method(manager, "registerNativeHandle", "(J)V", &args)?;
            Ok(())
        })
    }

    fn restore_state(&self) -> LocationResult<()> {
        if let Some(config) = self
            .standard_config
            .lock()
            .expect("standard config mutex poisoned")
            .clone()
        {
            self.apply_standard(&config)?;
        }

        if let Some(config) = self
            .significant_config
            .lock()
            .expect("significant config mutex poisoned")
            .clone()
        {
            self.apply_significant(&config)?;
        }

        if let Some(config) = self
            .heading_config
            .lock()
            .expect("heading config mutex poisoned")
            .clone()
        {
            self.apply_heading(&config)?;
        }

        let regions = self
            .monitored_regions
            .lock()
            .expect("regions mutex poisoned")
            .clone();
        if !regions.is_empty() {
            self.apply_regions(&regions)?;
        }

        let beacons = self
            .beacon_constraints
            .lock()
            .expect("beacon mutex poisoned")
            .clone();
        if !beacons.is_empty() {
            self.apply_beacons(&beacons)?;
        }

        Ok(())
    }

    fn apply_standard(&self, config: &StandardUpdateConfig) -> LocationResult<()> {
        let json = to_json(config)?;
        self.with_manager(|env, manager| {
            let j_string = env.new_string(json.as_str())?;
            let args = [JValue::Object(JObject::from(j_string))];
            env.call_method(
                manager,
                "configureStandardUpdates",
                "(Ljava/lang/String;)V",
                &args,
            )?;
            Ok(())
        })
    }

    fn apply_significant(&self, config: &SignificantUpdateConfig) -> LocationResult<()> {
        let json = to_json(config)?;
        self.with_manager(|env, manager| {
            let j_string = env.new_string(json.as_str())?;
            let args = [JValue::Object(JObject::from(j_string))];
            env.call_method(
                manager,
                "configureSignificantUpdates",
                "(Ljava/lang/String;)V",
                &args,
            )?;
            Ok(())
        })
    }

    fn apply_heading(&self, config: &HeadingConfig) -> LocationResult<()> {
        let json = to_json(config)?;
        self.with_manager(|env, manager| {
            let j_string = env.new_string(json.as_str())?;
            let args = [JValue::Object(JObject::from(j_string))];
            env.call_method(
                manager,
                "configureHeadingUpdates",
                "(Ljava/lang/String;)V",
                &args,
            )?;
            Ok(())
        })
    }

    fn apply_regions(&self, regions: &[Region]) -> LocationResult<()> {
        let json = to_json(regions)?;
        self.with_manager(|env, manager| {
            let j_string = env.new_string(json.as_str())?;
            let args = [JValue::Object(JObject::from(j_string))];
            env.call_method(manager, "monitorRegions", "(Ljava/lang/String;)V", &args)?;
            Ok(())
        })
    }

    fn apply_beacons(&self, constraints: &[BeaconConstraint]) -> LocationResult<()> {
        let json = to_json(constraints)?;
        self.with_manager(|env, manager| {
            let j_string = env.new_string(json.as_str())?;
            let args = [JValue::Object(JObject::from(j_string))];
            env.call_method(manager, "rangeBeacons", "(Ljava/lang/String;)V", &args)?;
            Ok(())
        })
    }
}

impl LocationBackend for AndroidLocationBackend {
    fn set_delegate(&self, delegate: Arc<dyn LocationDelegate>) {
        {
            let mut guard = self.delegate.lock().expect("delegate mutex poisoned");
            *guard = Some(delegate.clone());
        }

        {
            let mut map = delegates().lock().expect("delegate map mutex poisoned");
            map.insert(self.handle, delegate);
        }

        if let Err(err) = self.register_handle() {
            error!("failed to register Android location handle: {err}");
        }

        if let Err(err) = self.restore_state() {
            error!("failed to restore Android location state: {err}");
        }
    }

    fn configure_standard_updates(&self, config: StandardUpdateConfig) -> LocationResult<()> {
        {
            let mut guard = self
                .standard_config
                .lock()
                .expect("standard config mutex poisoned");
            *guard = Some(config.clone());
        }

        self.apply_standard(&config)
    }

    fn start_standard_updates(&self) -> LocationResult<()> {
        self.with_manager(|env, manager| {
            env.call_method(manager, "startStandardUpdates", "()V", &[])?;
            Ok(())
        })
    }

    fn stop_standard_updates(&self) {
        if let Err(err) = self.with_manager(|env, manager| {
            env.call_method(manager, "stopStandardUpdates", "()V", &[])?;
            Ok(())
        }) {
            error!("failed to stop Android standard updates: {err}");
        }
    }

    fn configure_significant_updates(&self, config: SignificantUpdateConfig) -> LocationResult<()> {
        {
            let mut guard = self
                .significant_config
                .lock()
                .expect("significant config mutex poisoned");
            *guard = Some(config.clone());
        }

        self.apply_significant(&config)
    }

    fn start_significant_updates(&self) -> LocationResult<()> {
        self.with_manager(|env, manager| {
            env.call_method(manager, "startSignificantUpdates", "()V", &[])?;
            Ok(())
        })
    }

    fn stop_significant_updates(&self) {
        if let Err(err) = self.with_manager(|env, manager| {
            env.call_method(manager, "stopSignificantUpdates", "()V", &[])?;
            Ok(())
        }) {
            error!("failed to stop Android significant updates: {err}");
        }
    }

    fn monitor_regions(&self, regions: Vec<Region>) -> LocationResult<()> {
        {
            let mut guard = self
                .monitored_regions
                .lock()
                .expect("regions mutex poisoned");
            *guard = regions.clone();
        }

        self.apply_regions(&regions)
    }

    fn stop_monitoring_regions(&self) {
        {
            let mut guard = self
                .monitored_regions
                .lock()
                .expect("regions mutex poisoned");
            guard.clear();
        }

        if let Err(err) = self.with_manager(|env, manager| {
            env.call_method(manager, "stopMonitoringRegions", "()V", &[])?;
            Ok(())
        }) {
            error!("failed to stop monitoring Android regions: {err}");
        }
    }

    fn range_beacons(&self, constraints: Vec<BeaconConstraint>) -> LocationResult<()> {
        {
            let mut guard = self
                .beacon_constraints
                .lock()
                .expect("beacon mutex poisoned");
            *guard = constraints.clone();
        }

        self.apply_beacons(&constraints)
    }

    fn stop_ranging_beacons(&self) {
        {
            let mut guard = self
                .beacon_constraints
                .lock()
                .expect("beacon mutex poisoned");
            guard.clear();
        }

        if let Err(err) = self.with_manager(|env, manager| {
            env.call_method(manager, "stopRangingBeacons", "()V", &[])?;
            Ok(())
        }) {
            error!("failed to stop Android beacon ranging: {err}");
        }
    }

    fn configure_heading_updates(&self, config: HeadingConfig) -> LocationResult<()> {
        {
            let mut guard = self
                .heading_config
                .lock()
                .expect("heading config mutex poisoned");
            *guard = Some(config.clone());
        }

        self.apply_heading(&config)
    }

    fn start_heading_updates(&self) -> LocationResult<()> {
        self.with_manager(|env, manager| {
            env.call_method(manager, "startHeadingUpdates", "()V", &[])?;
            Ok(())
        })
    }

    fn stop_heading_updates(&self) {
        if let Err(err) = self.with_manager(|env, manager| {
            env.call_method(manager, "stopHeadingUpdates", "()V", &[])?;
            Ok(())
        }) {
            error!("failed to stop Android heading updates: {err}");
        }
    }
}

impl Drop for AndroidLocationBackend {
    fn drop(&mut self) {
        if let Some(map) = DELEGATES.get() {
            let mut guard = map.lock().expect("delegate map mutex poisoned");
            guard.remove(&self.handle);
        }
    }
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn Java_com_waterkit_location_LocationBridge_dispatchEvent(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    json_event: JString<'_>,
) {
    let handle = handle as u64;
    let json = match env.get_string(&json_event) {
        Ok(value) => value.to_string_lossy().into_owned(),
        Err(err) => {
            error!("failed to read Android event payload: {err}");
            return;
        }
    };

    dispatch_event(handle, json);
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn Java_com_waterkit_location_LocationBridge_dispatchError(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    message: JString<'_>,
) {
    let handle = handle as u64;
    let message = match env.get_string(&message) {
        Ok(value) => value.to_string_lossy().into_owned(),
        Err(err) => {
            error!("failed to read Android error payload: {err}");
            return;
        }
    };

    let error = LocationError::Platform { message };
    emit_event(handle, LocationEvent::Error { error });
}

fn dispatch_event(handle: u64, json: String) {
    match from_json::<LocationEvent>(&json) {
        Ok(event) => emit_event(handle, event),
        Err(err) => {
            let error = LocationError::Serialization {
                message: err.to_string(),
            };
            emit_event(handle, LocationEvent::Error { error });
        }
    }
}

fn emit_event(handle: u64, event: LocationEvent) {
    let delegate = {
        let map = delegates().lock().expect("delegate map mutex poisoned");
        map.get(&handle).cloned()
    };

    if let Some(delegate) = delegate {
        delegate.on_event(event);
    } else {
        error!("received Android location event for unknown handle {handle}");
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

fn map_jni_error(err: jni::errors::Error) -> LocationError {
    LocationError::Platform {
        message: err.to_string(),
    }
}
