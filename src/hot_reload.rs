#[cfg(waterui_enable_hot_reload)]
mod enabled {
    use async_channel::{Receiver, Sender};
    use executor_core::spawn_local;
    use futures::{FutureExt, pin_mut};
    use libloading::Library;
    use log::{debug, warn};
    use serde_json::{Map, Number, Value};
    use std::panic::{self, PanicInfo};
    use std::{
        backtrace::Backtrace,
        fs::File,
        io::Write,
        path::{Path, PathBuf},
        sync::{Mutex, Once, OnceLock},
        thread,
    };
    use thiserror::Error;
    use waterui_core::{AnyView, Dynamic, View, event::Associated};
    use zenwave::websocket::{self, WebSocketError, WebSocketMessage};

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

            install_panic_forwarder();
            if let Some(endpoint) = endpoint {
                let daemon_trigger = trigger.clone();
                let daemon_endpoint = endpoint.clone();
                let (outbound_tx, outbound_rx) = async_channel::unbounded();
                register_outbound_sender(outbound_tx);
                spawn_local(async move {
                    let result =
                        hot_reload_daemon(daemon_trigger, daemon_endpoint, outbound_rx).await;
                    clear_outbound_sender();
                    if let Err(err) = result {
                        warn!("Failed to launch hot reload daemon: {err}");
                    }
                })
                .detach();
            } else {
                debug!("Hot reload endpoint not available; running without watcher");
            }

            dynamic
        }
    }

    #[derive(Debug, Error)]
    pub enum Error {
        #[error("Failed to connect to hot-reload server: {0}")]
        FailedToConnect(#[from] WebSocketError),
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

    async fn hot_reload_daemon(
        trigger: HotReloadTrigger,
        endpoint: HotReloadEndpoint,
        mut outbound_rx: Receiver<String>,
    ) -> Result<(), Error> {
        // The app is the client, connecting to the CLI.
        let url = format!("ws://{}:{}/hot-reload-native", endpoint.host, endpoint.port);

        let mut socket = websocket::connect(url).await?;
        let mut outbound_closed = false;

        loop {
            if !outbound_closed && !outbound_rx.is_closed() {
                while let Ok(text) = outbound_rx.try_recv() {
                    if let Err(err) = socket.send_text(text).await {
                        warn!("Failed to forward message to CLI: {err}");
                        return Err(err.into());
                    }
                }

                if outbound_rx.is_closed() {
                    outbound_closed = true;
                }
            }

            match socket.recv().await {
                Ok(Some(WebSocketMessage::Binary(data))) => {
                    let lib_path = create_library(&data);
                    trigger.trigger_reload(lib_path);
                }
                Ok(Some(WebSocketMessage::Text(_))) => {}
                Ok(None) => {
                    break;
                }
                Err(err) => {
                    return Err(err.into());
                }
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
    static OUTBOUND_CHANNEL: OnceLock<Mutex<Option<Sender<String>>>> = OnceLock::new();
    static PANIC_FORWARDER: Once = Once::new();

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

    fn register_outbound_sender(sender: Sender<String>) {
        let slot = OUTBOUND_CHANNEL.get_or_init(|| Mutex::new(None));
        let mut guard = slot.lock().expect("outbound sender mutex poisoned");
        *guard = Some(sender);
    }

    fn clear_outbound_sender() {
        if let Some(slot) = OUTBOUND_CHANNEL.get() {
            if let Ok(mut guard) = slot.lock() {
                guard.take();
            }
        }
    }

    fn outbound_sender() -> Option<Sender<String>> {
        OUTBOUND_CHANNEL
            .get()
            .and_then(|slot| slot.lock().ok().and_then(|sender| sender.clone()))
    }

    fn install_panic_forwarder() {
        PANIC_FORWARDER.call_once(|| {
            let previous_hook = panic::take_hook();
            panic::set_hook(Box::new(move |info| {
                report_panic(info);
                previous_hook(info);
            }));
        });
    }

    fn report_panic(info: &PanicInfo) {
        if let Some(sender) = outbound_sender() {
            let event = ClientEvent::from_panic(info);
            let text = event.into_json_string();
            let _ = sender.send_blocking(text);
        }
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

    #[derive(Debug)]
    enum ClientEvent {
        Panic(PanicReport),
    }

    impl ClientEvent {
        fn from_panic(info: &PanicInfo) -> Self {
            Self::Panic(PanicReport::from(info))
        }

        fn into_json_string(self) -> String {
            self.into_json_value().to_string()
        }

        fn into_json_value(self) -> Value {
            let mut event = Map::with_capacity(2);
            match self {
                Self::Panic(report) => {
                    event.insert("type".into(), Value::String("panic".into()));
                    event.insert("panic".into(), report.into_json_value());
                }
            }
            Value::Object(event)
        }
    }

    #[derive(Debug)]
    struct PanicReport {
        message: String,
        location: Option<PanicLocation>,
        thread: Option<String>,
        backtrace: Option<String>,
    }

    impl From<&PanicInfo<'_>> for PanicReport {
        fn from(info: &PanicInfo<'_>) -> Self {
            Self {
                message: panic_message(info),
                location: info.location().map(|loc| PanicLocation {
                    file: loc.file().to_string(),
                    line: loc.line(),
                    column: loc.column(),
                }),
                thread: thread::current().name().map(ToString::to_string),
                backtrace: Some(Backtrace::force_capture().to_string()),
            }
        }
    }

    impl PanicReport {
        fn into_json_value(self) -> Value {
            let mut panic_obj = Map::with_capacity(4);
            panic_obj.insert("message".into(), Value::String(self.message));
            if let Some(location) = self.location {
                panic_obj.insert("location".into(), location.into_json_value());
            }
            if let Some(thread) = self.thread {
                panic_obj.insert("thread".into(), Value::String(thread));
            }
            if let Some(backtrace) = self.backtrace {
                panic_obj.insert("backtrace".into(), Value::String(backtrace));
            }
            Value::Object(panic_obj)
        }
    }

    #[derive(Debug)]
    struct PanicLocation {
        file: String,
        line: u32,
        column: u32,
    }

    impl PanicLocation {
        fn into_json_value(self) -> Value {
            let mut map = Map::with_capacity(3);
            map.insert("file".into(), Value::String(self.file));
            map.insert("line".into(), Value::Number(Number::from(self.line)));
            map.insert("column".into(), Value::Number(Number::from(self.column)));
            Value::Object(map)
        }
    }

    fn panic_message(info: &PanicInfo<'_>) -> String {
        if let Some(message) = info.payload().downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            "Unknown panic payload".to_string()
        }
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
