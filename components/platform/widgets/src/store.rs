//! Shared persistent state and timeline reload for home-screen widgets.
//!
//! Applications and their widget extensions run in separate processes on Apple
//! platforms (sharing an App Group container) and share private application
//! storage on Android. [`WidgetStore`] persists JSON-serializable values under
//! that container using atomic write-and-rename so readers never observe a
//! partial write; [`reload`] and [`reload_all`] ask the host system
//! (`WidgetKit` on Apple, `AppWidgetManager` on Android) to refresh visible
//! widgets after state changes.

use alloc::format;
use alloc::string::String;
use core::fmt::{self, Write as _};
use core::hash::{Hash as _, Hasher as _};
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Error produced when reading, writing, or locating a [`WidgetStore`].
#[derive(Debug)]
#[non_exhaustive]
pub enum WidgetStoreError {
    /// Filesystem I/O failed while accessing the store directory or key file.
    Io(io::Error),
    /// Serializing a value to JSON failed.
    Serialize(serde_json::Error),
    /// Deserializing a stored JSON value failed.
    Deserialize(serde_json::Error),
    /// No shared widget container is configured for the running process.
    Unavailable(String),
    /// The platform bridge failed to resolve the container.
    Platform(String),
    /// This target platform has no system widget container.
    UnsupportedPlatform,
}

impl fmt::Display for WidgetStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "widget store I/O error: {error}"),
            Self::Serialize(error) => write!(f, "widget store serialization error: {error}"),
            Self::Deserialize(error) => write!(f, "widget store deserialization error: {error}"),
            Self::Unavailable(reason) => write!(f, "widget shared store unavailable: {reason}"),
            Self::Platform(reason) => write!(f, "widget platform bridge error: {reason}"),
            Self::UnsupportedPlatform => {
                f.write_str("widget shared store is not supported on this platform")
            }
        }
    }
}

impl Error for WidgetStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serialize(error) | Self::Deserialize(error) => Some(error),
            Self::Unavailable(_) | Self::Platform(_) | Self::UnsupportedPlatform => None,
        }
    }
}

impl From<io::Error> for WidgetStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Persistent key-value store shared between an application and its home-screen
/// widgets.
///
/// Keys are arbitrary UTF-8 strings; values are serialized as JSON and written
/// atomically so a widget extension never observes a partially written entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WidgetStore {
    root: PathBuf,
}

impl WidgetStore {
    /// Opens a widget store backed by `root`.
    ///
    /// The directory is created on first write.
    #[must_use]
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Opens the platform-provisioned shared widget container for the running
    /// application or widget extension.
    ///
    /// - On Apple platforms, resolves the App Group container named by the
    ///   `WaterUIWidgetAppGroup` key in `Info.plist` (provisioned by `water`
    ///   whenever the project declares a `#[waterui::widget]`).
    /// - On Android, resolves `<filesDir>/waterui-widgets` through
    ///   `dev.waterui.widgets.WaterUIWidgetHost`.
    ///
    /// # Errors
    ///
    /// Returns [`WidgetStoreError::Unavailable`] when no App Group is
    /// configured, [`WidgetStoreError::Platform`] if the platform bridge fails,
    /// or [`WidgetStoreError::UnsupportedPlatform`] on platforms without a
    /// home-screen widget container.
    pub fn shared() -> Result<Self, WidgetStoreError> {
        let root = platform::shared_container_path()?;
        Ok(Self { root })
    }

    /// The directory backing this store.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reads and decodes the value stored under `key`, returning `None` when
    /// absent or unreadable.
    ///
    /// Use [`Self::try_get`] to distinguish missing keys from I/O or decode
    /// errors.
    #[must_use]
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.try_get(key).ok().flatten()
    }

    /// Reads and decodes the value stored under `key`.
    ///
    /// Returns `Ok(None)` when no entry exists for `key`.
    ///
    /// # Errors
    ///
    /// Returns [`WidgetStoreError::Io`] on filesystem failure or
    /// [`WidgetStoreError::Deserialize`] if the stored JSON does not decode
    /// into `T`.
    pub fn try_get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, WidgetStoreError> {
        let path = self.entry_path(key);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(WidgetStoreError::Io(error)),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(WidgetStoreError::Deserialize)
    }

    /// Stores `value` under `key`, replacing any prior value atomically.
    ///
    /// # Errors
    ///
    /// Returns [`WidgetStoreError::Serialize`] if `value` does not serialize to
    /// JSON, or [`WidgetStoreError::Io`] if the store directory or entry file
    /// cannot be written.
    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<(), WidgetStoreError> {
        let bytes = serde_json::to_vec(value).map_err(WidgetStoreError::Serialize)?;
        fs::create_dir_all(&self.root)?;
        let target = self.entry_path(key);
        let temp = self.root.join(unique_temp_filename(key));
        fs::write(&temp, bytes)?;
        if let Err(error) = fs::rename(&temp, &target) {
            let _ = fs::remove_file(&temp);
            return Err(WidgetStoreError::Io(error));
        }
        Ok(())
    }

    /// Removes `key` from the store if present.
    ///
    /// # Errors
    ///
    /// Returns [`WidgetStoreError::Io`] on filesystem failure other than a
    /// missing file.
    pub fn remove(&self, key: &str) -> Result<(), WidgetStoreError> {
        match fs::remove_file(self.entry_path(key)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(WidgetStoreError::Io(error)),
        }
    }

    /// Whether an entry currently exists for `key`.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.entry_path(key).is_file()
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        self.root.join(encoded_key_filename(key))
    }
}

/// Opens the application's shared widget store.
///
/// Equivalent to [`WidgetStore::shared`].
///
/// # Errors
///
/// See [`WidgetStore::shared`].
pub fn shared_store() -> Result<WidgetStore, WidgetStoreError> {
    WidgetStore::shared()
}

/// Requests the host platform to reload timelines for the widget with `kind`.
///
/// - On Apple platforms, calls `WidgetCenter.shared.reloadTimelines(ofKind:)`.
/// - On Android, broadcasts an `AppWidgetManager.ACTION_APPWIDGET_UPDATE`
///   request to matching `AppWidgetProvider` and Glance receivers.
/// - On platforms without home-screen widgets, this is a no-op.
pub fn reload(kind: &str) {
    platform::reload(kind);
}

/// Requests the host platform to reload timelines for every widget declared by
/// the application.
///
/// - On Apple platforms, calls `WidgetCenter.shared.reloadAllTimelines()`.
/// - On Android, broadcasts an update to all installed widget providers for the
///   application.
/// - On platforms without home-screen widgets, this is a no-op.
pub fn reload_all() {
    platform::reload_all();
}

/// Generates a unique temporary sibling filename for atomic write-and-rename.
fn unique_temp_filename(key: &str) -> String {
    let mut hasher = DefaultHasher::new();
    process::id().hash(&mut hasher);
    thread::current().id().hash(&mut hasher);
    key.hash(&mut hasher);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    nanos.hash(&mut hasher);
    format!(".tmp-{}-{:016x}.json", process::id(), hasher.finish())
}

/// Encodes an arbitrary UTF-8 key into a collision-free, case-insensitive-FS
/// safe filename (`<lowercase-hex>.json`).
fn encoded_key_filename(key: &str) -> String {
    let mut name = String::with_capacity(key.len() * 2 + 5);
    for byte in key.as_bytes() {
        write!(name, "{byte:02x}").expect("writing to a String never fails");
    }
    name.push_str(".json");
    name
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod platform {
    use alloc::string::ToString as _;
    use core::ffi::c_void;
    use core::slice;
    use core::str;
    use std::path::PathBuf;

    use super::WidgetStoreError;

    type PathCallback = extern "C" fn(*const u8, usize, *mut c_void);

    unsafe extern "C" {
        fn waterui_widgets_apple_reload(kind_ptr: *const u8, kind_len: usize);
        fn waterui_widgets_apple_reload_all();
        fn waterui_widgets_apple_shared_container_path(
            callback: PathCallback,
            context: *mut c_void,
        );
    }

    extern "C" fn collect_path(ptr: *const u8, len: usize, context: *mut c_void) {
        if ptr.is_null() || len == 0 || context.is_null() {
            return;
        }
        // SAFETY: `AppleBridge.swift` passes a valid UTF-8 byte slice for the
        // duration of this synchronous callback and `context` points to our
        // stack-allocated `Option<PathBuf>`.
        let bytes = unsafe { slice::from_raw_parts(ptr, len) };
        if let Ok(path) = str::from_utf8(bytes) {
            // SAFETY: `shared_container_path` passes `&raw mut resolved` and
            // awaits this callback synchronously on the same stack frame.
            let slot = unsafe { &mut *context.cast::<Option<PathBuf>>() };
            *slot = Some(PathBuf::from(path));
        }
    }

    pub(super) fn shared_container_path() -> Result<PathBuf, WidgetStoreError> {
        let mut resolved: Option<PathBuf> = None;
        let context = (&raw mut resolved).cast::<c_void>();
        // SAFETY: `collect_path` runs synchronously before this FFI call
        // returns, so `resolved` stays alive for the entire call.
        unsafe {
            waterui_widgets_apple_shared_container_path(collect_path, context);
        }
        resolved.ok_or_else(|| {
            WidgetStoreError::Unavailable(
                "missing `WaterUIWidgetAppGroup` in Info.plist or App Group container entitlement"
                    .to_string(),
            )
        })
    }

    pub(super) fn reload(kind: &str) {
        let bytes = kind.as_bytes();
        // SAFETY: `bytes.as_ptr()` is valid for `bytes.len()` bytes for the
        // duration of the synchronous FFI call.
        unsafe {
            waterui_widgets_apple_reload(bytes.as_ptr(), bytes.len());
        }
    }

    pub(super) fn reload_all() {
        // SAFETY: invokes `WidgetCenter.shared.reloadAllTimelines()`.
        unsafe {
            waterui_widgets_apple_reload_all();
        }
    }
}

#[cfg(target_os = "android")]
mod platform {
    use alloc::string::ToString as _;
    use std::path::PathBuf;

    use jni::objects::JValue;
    use jni::{jni_sig, jni_str};
    use waterkit_build::{
        AndroidError, DexHelper, decode_string, dex_helper, with_android_context,
    };
    use waterui_contract::contract_violation;

    use super::WidgetStoreError;

    static HOST: DexHelper = dex_helper!("dev.waterui.widgets.WaterUIWidgetHost");

    impl From<AndroidError> for WidgetStoreError {
        fn from(error: AndroidError) -> Self {
            Self::Platform(error.to_string())
        }
    }

    pub(super) fn shared_container_path() -> Result<PathBuf, WidgetStoreError> {
        with_android_context(|env, context| {
            let class = HOST.class(env, context)?;
            let path = env
                .call_static_method(
                    class,
                    jni_str!("sharedStorePath"),
                    jni_sig!("(Landroid/content/Context;)Ljava/lang/String;"),
                    &[JValue::Object(context)],
                )
                .map_err(AndroidError::from)?
                .l()
                .map_err(AndroidError::from)?;
            let path = decode_string(env, &path)?;
            Ok(PathBuf::from(path))
        })
    }

    pub(super) fn reload(kind: &str) {
        let result = with_android_context::<(), WidgetStoreError, _>(|env, context| {
            let class = HOST.class(env, context)?;
            let kind_str = env.new_string(kind).map_err(AndroidError::from)?;
            env.call_static_method(
                class,
                jni_str!("reload"),
                jni_sig!("(Landroid/content/Context;Ljava/lang/String;)V"),
                &[JValue::Object(context), JValue::Object(&kind_str)],
            )
            .map_err(AndroidError::from)?;
            Ok(())
        });
        if let Err(error) = result {
            contract_violation!("failed to reload Android widget `{kind}`: {error}");
        }
    }

    pub(super) fn reload_all() {
        let result = with_android_context::<(), WidgetStoreError, _>(|env, context| {
            let class = HOST.class(env, context)?;
            env.call_static_method(
                class,
                jni_str!("reloadAll"),
                jni_sig!("(Landroid/content/Context;)V"),
                &[JValue::Object(context)],
            )
            .map_err(AndroidError::from)?;
            Ok(())
        });
        if let Err(error) = result {
            contract_violation!("failed to reload all Android widgets: {error}");
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
mod platform {
    use std::path::PathBuf;

    use super::WidgetStoreError;

    pub(super) const fn shared_container_path() -> Result<PathBuf, WidgetStoreError> {
        Err(WidgetStoreError::UnsupportedPlatform)
    }

    pub(super) const fn reload(_kind: &str) {}

    pub(super) const fn reload_all() {}
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use std::fs;

    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    use super::{WidgetStore, WidgetStoreError};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Goal {
        label: String,
        target: u32,
    }

    #[test]
    fn round_trips_primitives_and_structs() {
        let dir = tempdir().expect("tempdir");
        let store = WidgetStore::open(dir.path().join("nested/store"));

        assert_eq!(store.get::<u32>("steps"), None);
        assert!(!store.contains_key("steps"));

        store.set("steps", &6_241_u32).expect("write steps");
        assert!(store.contains_key("steps"));
        assert_eq!(store.get::<u32>("steps"), Some(6_241));

        let goal = Goal {
            label: "歩数 / 걸음 / 步数".into(),
            target: 10_000,
        };
        store.set("goal/daily", &goal).expect("write goal");
        assert_eq!(store.get::<Goal>("goal/daily"), Some(goal));

        store.set("steps", &8_500_u32).expect("overwrite steps");
        assert_eq!(store.get::<u32>("steps"), Some(8_500));

        store.remove("steps").expect("remove steps");
        assert!(!store.contains_key("steps"));
        assert_eq!(store.get::<u32>("steps"), None);
        store.remove("steps").expect("removing absent key succeeds");
    }

    #[test]
    fn preserves_case_distinction_and_special_key_characters() {
        let dir = tempdir().expect("tempdir");
        let store = WidgetStore::open(dir.path());

        store.set("Steps", &1_u32).expect("write upper");
        store.set("steps", &2_u32).expect("write lower");
        store
            .set("../escape/日本語/한국어", &3_u32)
            .expect("write special");

        assert_eq!(store.get::<u32>("Steps"), Some(1));
        assert_eq!(store.get::<u32>("steps"), Some(2));
        assert_eq!(store.get::<u32>("../escape/日本語/한국어"), Some(3));
    }

    #[test]
    fn try_get_reports_deserialization_failure() {
        let dir = tempdir().expect("tempdir");
        let store = WidgetStore::open(dir.path());
        store.set("steps", &"not-a-number").expect("write string");

        assert_eq!(store.get::<u32>("steps"), None);
        assert!(matches!(
            store.try_get::<u32>("steps"),
            Err(WidgetStoreError::Deserialize(_))
        ));

        fs::write(store.entry_path("corrupt"), b"{not json").expect("write corrupt");
        assert!(matches!(
            store.try_get::<u32>("corrupt"),
            Err(WidgetStoreError::Deserialize(_))
        ));
    }
}
