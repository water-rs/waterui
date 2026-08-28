//! # Safety
//!
//! Loading CEF means `dlopen` plus symbol lookup, so the `unsafe` here asserts
//! that the framework exports those entry points at the declared signatures.
//! `libloading` keeps the framework mapped for as long as the `Library` lives.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use cef::args::Args;
use cef::{Settings, api_hash, execute_process, initialize, shutdown, sys};
use serde::Deserialize;
#[cfg(feature = "chromium")]
use waterui_chromium::ChromiumController;
#[cfg(feature = "webview")]
use waterui_webview::WebViewController;

use crate::app::new_app;
#[cfg(any(feature = "chromium", feature = "webview"))]
use crate::page::CefController;

const MAXIMUM_PUMP_INTERVAL: Duration = Duration::from_millis(1000 / 30);

#[derive(Deserialize)]
struct RuntimeIdentity {
    engine: String,
    version: String,
    platform: String,
    architecture: String,
}

#[cfg(target_os = "macos")]
struct PlatformSandbox {
    library: libloading::Library,
    context: std::ptr::NonNull<std::ffi::c_void>,
}

#[cfg(target_os = "macos")]
impl PlatformSandbox {
    fn initialize(paths: &CefRuntimePaths, args: &cef::MainArgs) -> Self {
        let library_path = paths.framework().join("Libraries/libcef_sandbox.dylib");
        // SAFETY: CEF framework load or symbol resolution; see the module safety
        // note.
        let library = unsafe { libloading::Library::new(&library_path) }.unwrap_or_else(|error| {
            panic!(
                "failed to load CEF sandbox library {}: {error}",
                library_path.display()
            )
        });
        // SAFETY: CEF framework load or symbol resolution; see the module safety
        // note.
        let initialize = unsafe {
            library
                .get::<extern "C" fn(
                    std::os::raw::c_int,
                    *mut *mut std::os::raw::c_char,
                ) -> *mut std::ffi::c_void>(b"cef_sandbox_initialize")
                .expect("CEF sandbox library has no cef_sandbox_initialize")
        };
        let context = std::ptr::NonNull::new(initialize(args.argc, args.argv))
            .expect("CEF sandbox initialization failed");
        Self { library, context }
    }

    const fn info() -> *mut u8 {
        core::ptr::null_mut()
    }
}

#[cfg(target_os = "macos")]
impl Drop for PlatformSandbox {
    fn drop(&mut self) {
        // SAFETY: CEF framework load or symbol resolution; see the module safety
        // note.
        let destroy = unsafe {
            self.library
                .get::<extern "C" fn(*mut std::ffi::c_void)>(b"cef_sandbox_destroy")
                .expect("CEF sandbox library has no cef_sandbox_destroy")
        };
        destroy(self.context.as_ptr());
    }
}

#[cfg(target_os = "windows")]
struct PlatformSandbox(std::ptr::NonNull<std::ffi::c_void>);

#[cfg(target_os = "windows")]
unsafe extern "C" {
    fn waterui_cef_windows_sandbox_create() -> *mut std::ffi::c_void;
    fn waterui_cef_windows_sandbox_info(sandbox: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn waterui_cef_windows_sandbox_destroy(sandbox: *mut std::ffi::c_void);
}

#[cfg(target_os = "windows")]
impl PlatformSandbox {
    fn initialize(_paths: &CefRuntimePaths, _args: &cef::MainArgs) -> Self {
        let sandbox = unsafe { waterui_cef_windows_sandbox_create() };
        Self(
            std::ptr::NonNull::new(sandbox)
                .expect("failed to allocate Windows CEF sandbox information"),
        )
    }

    fn info(&self) -> *mut u8 {
        unsafe { waterui_cef_windows_sandbox_info(self.0.as_ptr()) }.cast()
    }
}

#[cfg(target_os = "windows")]
impl Drop for PlatformSandbox {
    fn drop(&mut self) {
        unsafe { waterui_cef_windows_sandbox_destroy(self.0.as_ptr()) };
    }
}

#[cfg(target_os = "linux")]
struct PlatformSandbox;

#[cfg(target_os = "linux")]
impl PlatformSandbox {
    const fn initialize(_paths: &CefRuntimePaths, _args: &cef::MainArgs) -> Self {
        Self
    }

    const fn info() -> *mut u8 {
        core::ptr::null_mut()
    }
}

/// A CEF-requested deadline for the next external message-pump iteration.
#[derive(Debug, Clone, Copy)]
pub struct PumpDeadline(Instant);

impl PumpDeadline {
    pub(crate) fn after_millis(delay_ms: i64) -> Self {
        let delay =
            u64::try_from(delay_ms.max(0)).expect("non-negative CEF pump delay must fit into u64");
        Self(Instant::now() + Duration::from_millis(delay))
    }

    /// Returns the requested instant.
    #[must_use]
    pub const fn instant(self) -> Instant {
        self.0
    }
}

/// Resolved locations inside one staged CEF runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CefRuntimePaths {
    root: PathBuf,
    #[cfg(target_os = "macos")]
    framework: PathBuf,
}

impl CefRuntimePaths {
    /// Creates paths for an explicitly staged CEF runtime root.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        #[cfg(target_os = "macos")]
        let framework = root.join("Chromium Embedded Framework.framework");
        Self {
            root,
            #[cfg(target_os = "macos")]
            framework,
        }
    }

    #[cfg(target_os = "macos")]
    fn macos_app(contents: &Path) -> Self {
        Self {
            root: contents.join("Resources/waterui-browser/cef"),
            framework: contents.join("Frameworks/Chromium Embedded Framework.framework"),
        }
    }

    /// Resolves the runtime staged next to the current executable.
    ///
    /// `water package` and `water run` place the selected runtime at
    /// `waterui-browser/cef` relative to the executable.
    ///
    /// # Panics
    ///
    /// Panics when the current executable path cannot be resolved.
    #[must_use]
    pub fn packaged() -> Self {
        let executable = std::env::current_exe()
            .unwrap_or_else(|error| panic!("failed to resolve executable for CEF: {error}"));
        #[cfg(target_os = "macos")]
        {
            let contents = executable
                .ancestors()
                .filter(|ancestor| ancestor.file_name().is_some_and(|name| name == "Contents"))
                .find(|contents| {
                    contents
                        .join("Resources/waterui-browser/cef/runtime.json")
                        .is_file()
                })
                .unwrap_or_else(|| {
                    panic!(
                        "CEF executable {} is not inside a packaged WaterUI macOS application",
                        executable.display()
                    )
                });
            return Self::macos_app(contents);
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let executable_dir = executable
                .parent()
                .expect("CEF executable path must have a parent directory");
            return Self::new(executable_dir);
        }
        #[allow(unreachable_code)]
        Self::new(executable)
    }

    /// Returns the staged runtime root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn library(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            return self.framework.join("Chromium Embedded Framework");
        }
        #[cfg(target_os = "linux")]
        {
            return self.root.join("libcef.so");
        }
        #[cfg(target_os = "windows")]
        {
            return self.root.join("libcef.dll");
        }
        #[allow(unreachable_code)]
        self.root.join("unsupported")
    }

    fn resources(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            return self.framework.join("Resources");
        }
        #[allow(unreachable_code)]
        self.root.clone()
    }

    #[cfg(target_os = "macos")]
    fn framework(&self) -> PathBuf {
        self.framework.clone()
    }

    #[cfg(not(target_os = "macos"))]
    const fn framework() -> Option<PathBuf> {
        None
    }

    fn manifest(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            return self.root.join("runtime.json");
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            return self.root.join("waterui-browser/cef/runtime.json");
        }
        #[allow(unreachable_code)]
        self.root.join("runtime.json")
    }

    fn validate(&self) {
        let library = self.library();
        assert!(
            library.is_file(),
            "CEF runtime library is missing at {}. Package or run the app through `water` so the exact runtime is staged.",
            library.display()
        );
        let resources = self.resources();
        assert!(
            resources.is_dir(),
            "CEF runtime resources are missing at {}",
            resources.display()
        );
        #[cfg(target_os = "macos")]
        {
            let locale = resources.join("en.lproj/locale.pak");
            assert!(
                locale.is_file(),
                "CEF macOS runtime locale pack is missing at {}",
                locale.display()
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            let locales = resources.join("locales");
            assert!(
                locales.is_dir(),
                "CEF runtime locales are missing at {}",
                locales.display()
            );
        }
        let icu = resources.join("icudtl.dat");
        assert!(
            icu.is_file(),
            "CEF ICU data is missing at {}",
            icu.display()
        );
        let manifest = self.manifest();
        let identity: RuntimeIdentity =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap_or_else(|error| {
                panic!(
                    "failed to read CEF runtime manifest {}: {error}",
                    manifest.display()
                )
            }))
            .unwrap_or_else(|error| {
                panic!(
                    "invalid CEF runtime manifest {}: {error}",
                    manifest.display()
                )
            });
        let expected_version = format!(
            "{}.{}.{}",
            sys::CEF_VERSION_MAJOR,
            sys::CEF_VERSION_MINOR,
            sys::CEF_VERSION_PATCH
        );
        assert_eq!(identity.engine, "cef", "bundled runtime engine mismatch");
        assert_eq!(
            identity.version, expected_version,
            "bundled CEF version mismatch"
        );
        assert_eq!(
            identity.platform,
            std::env::consts::OS,
            "bundled CEF platform mismatch"
        );
        assert_eq!(
            identity.architecture,
            std::env::consts::ARCH,
            "bundled CEF architecture mismatch"
        );
    }
}

/// Configuration required to initialize a sandboxed CEF runtime.
#[derive(Debug, Clone)]
pub struct CefRuntimeConfiguration {
    /// Staged runtime layout.
    pub paths: CefRuntimePaths,
    /// Writable root for Chromium profiles and caches.
    pub cache_root: PathBuf,
}

impl CefRuntimeConfiguration {
    /// Creates a configuration from staged runtime and writable cache roots.
    #[must_use]
    pub fn new(paths: CefRuntimePaths, cache_root: impl Into<PathBuf>) -> Self {
        Self {
            paths,
            cache_root: cache_root.into(),
        }
    }

    /// Resolves the runtime staged by `water` and a persistent per-executable cache.
    ///
    /// # Panics
    ///
    /// Panics when the executable path, executable name, or platform cache
    /// directory cannot be resolved.
    #[must_use]
    pub fn packaged() -> Self {
        let executable = std::env::current_exe()
            .unwrap_or_else(|error| panic!("failed to resolve executable for CEF cache: {error}"));
        let name = executable
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .filter(|name| !name.is_empty())
            .expect("CEF executable name must be valid UTF-8");
        let cache_root = dirs::cache_dir()
            .expect("platform does not expose a writable cache directory")
            .join("waterui")
            .join(name)
            .join("cef");
        Self::new(CefRuntimePaths::packaged(), cache_root)
    }
}

struct LoadedCefLibrary;

impl LoadedCefLibrary {
    #[cfg(target_os = "macos")]
    fn load(path: &Path) -> Self {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;

        let encoded = path.as_os_str().as_bytes().to_vec();
        let encoded = CString::new(encoded)
            .unwrap_or_else(|_| panic!("CEF runtime path contains NUL: {}", path.display()));
        assert_eq!(
            // SAFETY: CEF framework load or symbol resolution; see the module safety
            // note.
            cef::load_library(Some(unsafe { &*encoded.as_ptr() })),
            1,
            "failed to load CEF runtime library from {}",
            path.display()
        );
        Self
    }

    #[cfg(not(target_os = "macos"))]
    const fn load(_path: &Path) -> Self {
        Self
    }
}

impl Drop for LoadedCefLibrary {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        assert_eq!(cef::unload_library(), 1, "failed to unload CEF runtime");
    }
}

struct CefBootstrap {
    args: Args,
    sandbox: PlatformSandbox,
    library: LoadedCefLibrary,
    app: cef::App,
    pump_requests: Receiver<PumpDeadline>,
}

fn bootstrap(paths: &CefRuntimePaths) -> Result<CefBootstrap, i32> {
    let args = Args::new();
    let sandbox = PlatformSandbox::initialize(paths, args.as_main_args());
    #[cfg(target_os = "windows")]
    let sandbox_info = sandbox.info();
    #[cfg(not(target_os = "windows"))]
    let sandbox_info = PlatformSandbox::info();
    let library = LoadedCefLibrary::load(&paths.library());
    let hash = api_hash(sys::CEF_API_VERSION_LAST, 0);
    assert!(
        !hash.is_null(),
        "loaded CEF runtime has no compatible API hash"
    );

    let (schedule, pump_requests) = channel();
    let mut app = new_app(schedule);
    let subprocess_exit = execute_process(Some(args.as_main_args()), Some(&mut app), sandbox_info);
    if subprocess_exit >= 0 {
        return Err(subprocess_exit);
    }

    Ok(CefBootstrap {
        args,
        sandbox,
        library,
        app,
        pump_requests,
    })
}

/// Executes one CEF subprocess from the packaged helper application.
///
/// # Panics
///
/// Panics when called without a CEF subprocess command-line type or when the
/// packaged runtime is malformed.
#[must_use]
pub fn run_packaged_subprocess() -> i32 {
    let paths = CefRuntimePaths::packaged();
    paths.validate();
    bootstrap(&paths).map_or_else(
        |exit_code| exit_code,
        |_| panic!("the packaged CEF helper requires a Chromium subprocess type"),
    )
}

struct RuntimeInner {
    _app: cef::App,
    _library: LoadedCefLibrary,
    pump_requests: Receiver<PumpDeadline>,
    next_pump: Cell<Instant>,
    /// Whether [`CefRuntime::start_message_pump`] already spawned this runtime's
    /// pump task. The `WebView` and Chromium installs share one runtime and both
    /// need it running, so the second call must not start a second loop.
    pump_running: Cell<bool>,
    #[cfg(any(feature = "chromium", feature = "webview"))]
    cache_root: PathBuf,
    _sandbox: PlatformSandbox,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        shutdown();
    }
}

/// One process-owned CEF runtime shared by standard `WebView` and Chromium pages.
#[derive(Clone)]
pub struct CefRuntime {
    inner: Rc<RuntimeInner>,
}

impl core::fmt::Debug for CefRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("CefRuntime").finish_non_exhaustive()
    }
}

impl CefRuntime {
    /// Loads, validates, executes subprocess dispatch, and initializes CEF.
    ///
    /// CEF's OS sandbox remains enabled. A missing helper or malformed runtime
    /// therefore fails during CEF initialization instead of silently disabling
    /// isolation.
    ///
    /// # Panics
    ///
    /// Panics when configuration, runtime identity, sandbox initialization,
    /// subprocess dispatch, or CEF initialization fails.
    #[must_use]
    pub fn initialize(configuration: CefRuntimeConfiguration) -> Self {
        let CefRuntimeConfiguration { paths, cache_root } = configuration;
        #[cfg(target_os = "macos")]
        crate::application_mac::assert_macos_application_active();
        paths.validate();
        assert!(
            cache_root.is_absolute(),
            "CEF cache root must be absolute: {}",
            cache_root.display()
        );
        std::fs::create_dir_all(&cache_root).unwrap_or_else(|error| {
            panic!(
                "failed to create CEF cache root {}: {error}",
                cache_root.display()
            )
        });

        let CefBootstrap {
            args,
            sandbox,
            library,
            mut app,
            pump_requests,
        } = bootstrap(&paths).unwrap_or_else(|exit_code| {
            std::process::exit(exit_code);
        });
        #[cfg(target_os = "windows")]
        let sandbox_info = sandbox.info();
        #[cfg(not(target_os = "windows"))]
        let sandbox_info = PlatformSandbox::info();

        let resources = paths.resources();
        #[cfg(target_os = "macos")]
        let framework = Some(paths.framework());
        #[cfg(not(target_os = "macos"))]
        let framework = CefRuntimePaths::framework();
        #[cfg(target_os = "macos")]
        let locales_dir_path = cef::CefString::default();
        #[cfg(not(target_os = "macos"))]
        let locales_dir_path = resources.join("locales").to_string_lossy().as_ref().into();
        #[cfg(target_os = "macos")]
        let browser_subprocess_path = cef::CefString::default();
        #[cfg(not(target_os = "macos"))]
        let browser_subprocess_path = std::env::current_exe()
            .expect("failed to resolve current executable for CEF subprocesses")
            .to_string_lossy()
            .as_ref()
            .into();
        let settings = Settings {
            no_sandbox: 0,
            browser_subprocess_path,
            windowless_rendering_enabled: 1,
            external_message_pump: 1,
            disable_signal_handlers: 1,
            root_cache_path: cache_root.to_string_lossy().as_ref().into(),
            resources_dir_path: resources.to_string_lossy().as_ref().into(),
            locales_dir_path,
            framework_dir_path: framework
                .as_deref()
                .map_or_else(cef::CefString::default, |path| {
                    path.to_string_lossy().as_ref().into()
                }),
            remote_debugging_port: 0,
            ..Default::default()
        };
        assert_eq!(
            initialize(
                Some(args.as_main_args()),
                Some(&settings),
                Some(&mut app),
                sandbox_info,
            ),
            1,
            "CEF initialization failed with sandbox enabled and runtime {}",
            paths.root().display()
        );

        Self {
            inner: Rc::new(RuntimeInner {
                _app: app,
                _library: library,
                pump_requests,
                next_pump: Cell::new(Instant::now()),
                pump_running: Cell::new(false),
                #[cfg(any(feature = "chromium", feature = "webview"))]
                cache_root,
                _sandbox: sandbox,
            }),
        }
    }

    /// Executes due CEF message-loop work and returns the next requested deadline.
    ///
    /// # Panics
    ///
    /// Panics when the browser-process pump channel disconnects.
    #[must_use]
    pub fn pump(&self) -> PumpDeadline {
        let mut requested = None;
        loop {
            match self.inner.pump_requests.try_recv() {
                Ok(deadline) => requested = Some(deadline.instant()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    panic!("CEF browser-process message pump disconnected")
                }
            }
        }
        if let Some(deadline) = requested {
            self.inner.next_pump.set(deadline);
        }

        let now = Instant::now();
        if now >= self.inner.next_pump.get() {
            cef::do_message_loop_work();
            self.inner.next_pump.set(now + MAXIMUM_PUMP_INTERVAL);
        }
        PumpDeadline(self.inner.next_pump.get())
    }

    /// Drives this runtime's external message pump from `WaterUI`'s UI executor.
    ///
    /// CEF's browser-process loop runs on the thread that initialized it — the
    /// UI thread — and it has to keep running whether or not anything is being
    /// drawn. A page only produces frames while Chromium is pumped, and a
    /// renderer that skips idle frames stops drawing exactly when a page has
    /// nothing new to show, so a pump that lives inside rendering deadlocks the
    /// two against each other: no frame, no pump, no frame. This task is
    /// therefore independent of every surface, and a page that wakes up asks
    /// for a redraw through the frame sink's waker.
    ///
    /// The loop is paced by CEF itself: [`Self::pump`] returns the instant
    /// Chromium asked to be called back at, and the task sleeps exactly that
    /// long. Calling this more than once for one runtime is a no-op — the
    /// `WebView` and Chromium installs share a runtime and both ask for it.
    ///
    /// The task holds no strong reference to the runtime and ends when the last
    /// one is dropped.
    pub fn start_message_pump(&self) {
        if self.inner.pump_running.replace(true) {
            return;
        }
        let runtime = Rc::downgrade(&self.inner);
        executor_core::spawn_local(async move {
            loop {
                let Some(inner) = runtime.upgrade() else {
                    return;
                };
                // The strong reference must not be held across the await, or
                // this task would keep CEF alive for the life of the process.
                let deadline = Self { inner }.pump().instant();
                futures_timer::Delay::new(deadline.saturating_duration_since(Instant::now())).await;
            }
        })
        .detach();
    }

    /// Creates the standard `WebView` controller backed by this runtime.
    #[cfg(feature = "webview")]
    #[must_use]
    pub fn webview_controller(&self) -> WebViewController {
        WebViewController::new(CefController::new(self.clone()))
    }

    /// Creates the full Chromium controller backed by this runtime.
    #[cfg(feature = "chromium")]
    #[must_use]
    pub fn chromium_controller(&self) -> ChromiumController {
        ChromiumController::new(CefController::new(self.clone()))
    }

    #[cfg(any(feature = "chromium", feature = "webview"))]
    pub(crate) fn cache_root(&self) -> &Path {
        &self.inner.cache_root
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::path::Path;

    use super::CefRuntimePaths;

    #[test]
    fn macos_app_uses_canonical_resource_and_framework_locations() {
        let contents = Path::new("/Applications/Browser.app/Contents");
        let paths = CefRuntimePaths::macos_app(contents);

        assert_eq!(paths.root(), contents.join("Resources/waterui-browser/cef"));
        assert_eq!(
            paths.framework(),
            contents.join("Frameworks/Chromium Embedded Framework.framework")
        );
    }
}
