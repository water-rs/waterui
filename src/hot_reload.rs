#[cfg(waterui_enable_hot_reload)]
mod enabled {
    use async_channel::Sender;
    use executor_core::{Task, spawn_local};
    use libloading::Library;
    use log::{debug, warn};
    use std::{
        fs::File,
        io::Write,
        path::{Path, PathBuf},
        sync::{Mutex, OnceLock},
        thread::spawn,
    };
    use thiserror::Error;
    use tungstenite::connect;
    use waterui_core::{AnyView, Dynamic, View, event::Associated};

    /// A view that can be hot-reloaded at runtime.
    #[derive(Debug)]
    pub struct Hotreload<V> {
        initial: V,
    }

    impl<V: View> Hotreload<V> {
        /// Creates a new `Hotreload` view with the specified initial view.
        pub const fn new(initial: V) -> Self {
            Self { initial }
        }
    }

    impl<V: View> View for Hotreload<V> {
        fn body(self, _env: &waterui_core::Environment) -> impl View {
            let (sender, receiver) = async_channel::unbounded::<PathBuf>();
            let (handler, dynamic) = Dynamic::new();
            handler.set(self.initial);

            let trigger = HotReloadTrigger::new(sender);
            let endpoint = resolve_endpoint();

            spawn_local(async move {
                while let Ok(path) = receiver.recv().await {
                    let new_view = unsafe { reload("waterui_main", &path) };
                    handler.set(new_view);
                }
            })
            .detach();

            if let Some(endpoint) = endpoint {
                // Start the hot-reload daemon in a separate thread
                spawn(move || {
                    if let Err(err) = hot_reload_daemon(&trigger, &endpoint) {
                        warn!("Failed to launch hot reload daemon: {err}");
                    }
                });
            } else {
                debug!("Hot reload endpoint not available; running without watcher");
            }

            dynamic
        }
    }

    #[derive(Debug, Error)]
    pub enum Error {
        #[error("Failed to connect to hot-reload server: {0}")]
        FailedToConnect(#[from] tungstenite::Error),
        #[error("Hot reload port not set")]
        HotReloadPortNotSet,
    }

    // you must call this function on main thread, otherwise it is UB
    unsafe fn reload(symbol: &str, path: &Path) -> Associated<Library, AnyView> {
        let lib = unsafe { Library::new(path) }.unwrap();
        let func: libloading::Symbol<unsafe extern "C" fn() -> *mut waterui_core::AnyView> =
            unsafe { lib.get(symbol.as_bytes()) }.expect("Failed to load symbol");
        let view_ptr = unsafe { func() };
        let view = unsafe { Box::from_raw(view_ptr) };
        Associated::new(lib, *view)
    }

    #[derive(Debug, Clone)]
    struct HotReloadTrigger {
        sender: Sender<PathBuf>,
    }

    impl HotReloadTrigger {
        pub const fn new(sender: Sender<PathBuf>) -> Self {
            Self { sender }
        }

        pub fn trigger_reload(&self, path: PathBuf) {
            let _ = self.sender.send_blocking(path);
        }
    }

    fn hot_reload_daemon(
        trigger: &HotReloadTrigger,
        endpoint: &HotReloadEndpoint,
    ) -> Result<(), Error> {
        // The app is the client, connecting to the CLI.
        let url = format!("ws://{}:{}/hot-reload-native", endpoint.host, endpoint.port);

        let (mut socket, _) = connect(url)?;

        while let Ok(msg) = socket.read() {
            if msg.is_binary() {
                let data = msg.into_data();
                let lib_path = create_library(&data);
                trigger.trigger_reload(lib_path);
            }
        }

        Ok(())
    }

    fn library_name() -> String {
        // generate a uuid for the library name
        let uuid = uuid::Uuid::new_v4();
        format!("waterui_hot_{uuid}")
    }

    fn hot_reload_directory() -> PathBuf {
        HOT_RELOAD_DIR_OVERRIDE
            .get()
            .and_then(|slot| slot.lock().ok().and_then(|path| path.clone()))
            .or_else(|| std::env::var_os("WATERUI_HOT_RELOAD_DIR").map(PathBuf::from))
            .unwrap_or_else(|| std::env::temp_dir().join("waterui_hot_reload"))
    }

    fn create_library(data: &[u8]) -> PathBuf {
        let hot_reload_dir = hot_reload_directory();
        if !hot_reload_dir.exists() {
            std::fs::create_dir_all(&hot_reload_dir)
                .expect("Failed to create hot_reload directory");
        }

        let lib_name = library_name();
        let mut lib_path = hot_reload_dir.join(lib_name);
        // do not forget to add the proper extension for the library
        if cfg!(target_os = "windows") {
            lib_path.set_extension("dll");
        } else if cfg!(target_os = "macos") {
            lib_path.set_extension("dylib");
        } else {
            lib_path.set_extension("so");
        }

        let mut file = File::create(&lib_path).expect("Failed to create hot reload library");
        file.write_all(data)
            .expect("Failed to write to hot reload library");
        lib_path
    }

    #[derive(Clone, Debug)]
    struct HotReloadEndpoint {
        host: String,
        port: u16,
    }

    static HOT_RELOAD_OVERRIDE: OnceLock<Mutex<Option<HotReloadEndpoint>>> = OnceLock::new();
    static HOT_RELOAD_DIR_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

    /// Configure the hot reload endpoint programmatically.
    ///
    /// If called, the override takes precedence over environment variables.
    #[allow(clippy::missing_panics_doc)]
    pub fn configure_hot_reload_endpoint(host: impl Into<String>, port: u16) {
        let slot = HOT_RELOAD_OVERRIDE.get_or_init(|| Mutex::new(None));
        let mut guard = slot
            .lock()
            .expect("configure_hot_reload_endpoint mutex poisoned");
        *guard = Some(HotReloadEndpoint {
            host: host.into(),
            port,
        });
    }

    /// Override the directory hotspots use to persist new shared libraries.
    ///
    /// Hosts such as Android can point this to their code cache to ensure the filesystem allows
    /// execution.
    #[allow(clippy::missing_panics_doc)]
    pub fn configure_hot_reload_directory(path: impl Into<PathBuf>) {
        let slot = HOT_RELOAD_DIR_OVERRIDE.get_or_init(|| Mutex::new(None));
        let mut guard = slot
            .lock()
            .expect("configure_hot_reload_directory mutex poisoned");
        *guard = Some(path.into());
    }

    fn resolve_endpoint() -> Option<HotReloadEndpoint> {
        if hot_reload_disabled() {
            return None;
        }

        HOT_RELOAD_OVERRIDE
            .get()
            .and_then(|slot| slot.lock().ok().and_then(|cfg| cfg.clone()))
            .or_else(endpoint_from_env)
    }

    fn endpoint_from_env() -> Option<HotReloadEndpoint> {
        let port = std::env::var("WATERUI_HOT_RELOAD_PORT")
            .ok()?
            .parse::<u16>()
            .ok()?;
        let host =
            std::env::var("WATERUI_HOT_RELOAD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        Some(HotReloadEndpoint { host, port })
    }

    fn hot_reload_disabled() -> bool {
        std::env::var("WATERUI_DISABLE_HOT_RELOAD")
            .map(|value| value == "1")
            .unwrap_or(false)
    }
}

#[cfg(waterui_enable_hot_reload)]
pub use enabled::{Hotreload, configure_hot_reload_directory, configure_hot_reload_endpoint};

#[cfg(not(waterui_enable_hot_reload))]
mod disabled {
    use std::path::PathBuf;
    use waterui_core::{Environment, View};

    /// Stub hot reload view used when compile-time support is disabled.
    #[derive(Debug)]
    pub struct Hotreload<V> {
        initial: V,
    }

    impl<V: View> Hotreload<V> {
        /// Creates a pass-through wrapper for the provided view.
        pub const fn new(initial: V) -> Self {
            Self { initial }
        }
    }

    impl<V: View> View for Hotreload<V> {
        fn body(self, _env: &Environment) -> impl View {
            self.initial
        }
    }

    /// No-op when hot reload is disabled.
    pub fn configure_hot_reload_endpoint(_host: impl Into<String>, _port: u16) {}

    /// No-op when hot reload is disabled.
    pub fn configure_hot_reload_directory(_path: impl Into<PathBuf>) {}
}

#[cfg(not(waterui_enable_hot_reload))]
pub use disabled::{Hotreload, configure_hot_reload_directory, configure_hot_reload_endpoint};
