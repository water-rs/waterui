use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;

use serde::Deserialize;

use crate::abi::{ABI_VERSION, WaterWpeRuntime, WpeApi};

/// Exact WPE `WebKit` line used by the bundled runtime.
pub const WPE_WEBKIT_VERSION: &str = "2.52.5";

#[derive(Deserialize)]
struct RuntimeIdentity {
    engine: String,
    version: String,
    platform: String,
    architecture: String,
}

/// Resolved locations inside one staged WPE runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpeRuntimePaths {
    root: PathBuf,
}

impl WpeRuntimePaths {
    /// Creates paths for an explicitly staged WPE runtime root.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolves the runtime staged next to the executable by `water`.
    ///
    /// # Panics
    ///
    /// Panics when the current executable path cannot be resolved.
    #[must_use]
    pub fn packaged() -> Self {
        let executable = std::env::current_exe()
            .unwrap_or_else(|error| panic!("failed to resolve executable for WPE: {error}"));
        let executable_dir = executable
            .parent()
            .expect("WPE executable path must have a parent directory");
        Self::new(executable_dir.join("waterui-browser/wpe"))
    }

    /// Returns the runtime root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn bridge(&self) -> PathBuf {
        self.root.join("lib/libwaterui_wpe.so")
    }

    fn validate(&self) {
        let bridge = self.bridge();
        assert!(
            bridge.is_file(),
            "bundled WPE bridge is missing at {}. Package or run the app through `water` so WPE WebKit {WPE_WEBKIT_VERSION} is staged",
            bridge.display()
        );
        let manifest = self.root.join("runtime.json");
        assert!(
            manifest.is_file(),
            "bundled WPE runtime manifest is missing at {}",
            manifest.display()
        );
        let identity: RuntimeIdentity =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap_or_else(|error| {
                panic!(
                    "failed to read bundled WPE runtime manifest {}: {error}",
                    manifest.display()
                )
            }))
            .unwrap_or_else(|error| {
                panic!(
                    "invalid bundled WPE runtime manifest {}: {error}",
                    manifest.display()
                )
            });
        assert_eq!(identity.engine, "wpe", "bundled runtime engine mismatch");
        assert_eq!(
            identity.version, WPE_WEBKIT_VERSION,
            "bundled WPE WebKit version mismatch"
        );
        assert_eq!(
            identity.platform,
            std::env::consts::OS,
            "bundled WPE platform mismatch"
        );
        assert_eq!(
            identity.architecture,
            std::env::consts::ARCH,
            "bundled WPE architecture mismatch"
        );
        let licenses = self.root.join("licenses");
        assert!(
            licenses.is_dir(),
            "bundled WPE license directory is missing at {}",
            licenses.display()
        );
        for (tool, package) in [
            ("/usr/bin/bwrap", "bubblewrap >= 0.3.1"),
            ("/usr/bin/xdg-dbus-proxy", "xdg-dbus-proxy"),
        ] {
            assert!(
                Path::new(tool).is_file(),
                "bundled WPE sandbox requires {package} at {tool}"
            );
        }
    }
}

pub struct RuntimeApi {
    pub api: WpeApi,
    _library: libloading::Library,
}

impl RuntimeApi {
    fn load(paths: &WpeRuntimePaths) -> Arc<Self> {
        paths.validate();
        let bridge = paths.bridge();
        let library = unsafe { libloading::Library::new(&bridge) }.unwrap_or_else(|error| {
            panic!(
                "failed to load bundled WPE bridge {}: {error}",
                bridge.display()
            )
        });
        let api = unsafe { WpeApi::load(&library) };
        let version = unsafe { (api.abi_version)() };
        assert_eq!(
            version, ABI_VERSION,
            "bundled WPE bridge ABI mismatch: runtime={version}, WaterUI={ABI_VERSION}"
        );
        Arc::new(Self {
            api,
            _library: library,
        })
    }
}

// SAFETY: the bridge ABI explicitly permits calling the frame-completion
// functions from wgpu's completion thread, and libloading keeps the module mapped
// until the final `Arc` drops, so the function pointers stay valid.
unsafe impl Send for RuntimeApi {}
// SAFETY: see the `Send` impl — the API holds only function pointers into a module
// that stays mapped, with no interior mutability.
unsafe impl Sync for RuntimeApi {}

struct RuntimeInner {
    api: Arc<RuntimeApi>,
    raw: NonNull<WaterWpeRuntime>,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        unsafe { (self.api.api.runtime_free)(self.raw.as_ptr()) };
    }
}

/// Main-thread WPE `WebKit` runtime.
#[derive(Clone)]
pub struct WpeRuntime {
    inner: std::rc::Rc<RuntimeInner>,
}

impl core::fmt::Debug for WpeRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("WpeRuntime").finish_non_exhaustive()
    }
}

impl WpeRuntime {
    /// Loads and initializes the exact staged WPE runtime.
    ///
    /// # Panics
    ///
    /// Panics when the runtime is missing, invalid, ABI-incompatible, or fails
    /// to initialize.
    #[must_use]
    pub fn initialize(paths: &WpeRuntimePaths) -> Self {
        let api = RuntimeApi::load(paths);
        let mut error = std::ptr::null_mut::<c_char>();
        let raw = unsafe { (api.api.runtime_new)(&raw mut error) };
        let raw = NonNull::new(raw).unwrap_or_else(|| {
            let message = take_error(&api, error);
            panic!("failed to initialize bundled WPE WebKit {WPE_WEBKIT_VERSION}: {message}")
        });
        assert!(
            error.is_null(),
            "WPE runtime initialized while also returning an error"
        );
        Self {
            inner: std::rc::Rc::new(RuntimeInner { api, raw }),
        }
    }

    /// Dispatches every currently-ready GLib/WebKit task without blocking.
    ///
    /// Backends call this from their native event loop before rendering.
    #[must_use]
    pub fn iteration(&self) -> bool {
        unsafe { (self.inner.api.api.runtime_iteration)(self.inner.raw.as_ptr()) }
    }

    pub(super) fn raw(&self) -> NonNull<WaterWpeRuntime> {
        self.inner.raw
    }

    pub(super) fn api(&self) -> &Arc<RuntimeApi> {
        &self.inner.api
    }
}

pub fn take_error(api: &RuntimeApi, error: *mut c_char) -> String {
    assert!(!error.is_null(), "WPE failed without an error message");
    let message = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    unsafe { (api.api.string_free)(error) };
    message
}
