#[cfg(waterui_enable_hot_reload)]
mod enabled {
    //! Hot reload client + log/panic forwarding.
    //!
    //! Panic/Tracing pipeline overview:
    //! - `ffi` installs `tracing_panic::panic_hook` so panics become tracing events (for stderr/logcat).
    //! - This module installs its own panic hook once (`PANIC_FORWARDER`), sending a structured
    //!   `NativeClientEvent::Panic` over the hot-reload websocket, then delegating to the previous
    //!   hook (tracing_panic) so logs still flow.
    //! - `PanicAwareFormatter` rewrites panic tracing events into a minimal, source-path-free
    //!   message + truncated stack snippet for app-side logs, while the CLI renders the rich view.
    //! - `CliForwardLayer` forwards tracing output (including sanitized panic logs) over the websocket.
    //!
    //! This keeps device logs clean while enabling the CLI to show a detailed, pretty panic report.
    use async_channel::{Receiver, Sender};
    use executor_core::spawn_local;
    use futures::{FutureExt, pin_mut};
    use libloading::Library;
    use serde_json::{Map, Number, Value};
    use std::panic::{self, PanicInfo};
    use std::{
        backtrace::Backtrace,
        fs::File,
        io::{self, Write},
        path::{Path, PathBuf},
        sync::{Mutex, Once, OnceLock},
        thread,
    };
    use thiserror::Error;
    use tracing::{Level, debug, warn};
    use tracing_subscriber::{
        EnvFilter, Layer,
        fmt::{
            self, FormatEvent, FormatFields,
            format::{DefaultFields, Format},
            writer::MakeWriter,
        },
        layer::Context,
        prelude::*,
        registry::LookupSpan,
        util::SubscriberInitExt,
    };
    use waterui_core::{AnyView, Dynamic, View, event::Associated};
    use zenwave::websocket::{self, WebSocketError, WebSocketMessage};

    const TRACING_PREFIX: &str = "[waterui::tracing]";

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
            install_tracing_forwarder();
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

    fn install_tracing_forwarder() {
        static TRACING_SETUP: Once = Once::new();
        TRACING_SETUP.call_once(|| {
            let filter =
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
            let event_formatter = PanicAwareFormatter::new();
            let console_layer = fmt::layer()
                .event_format(event_formatter.clone())
                .with_writer(PrefixedMakeWriter)
                .with_ansi(false)
                .without_time()
                .with_target(true);
            let cli_layer = CliForwardLayer::new(event_formatter);
            let mut registry = tracing_subscriber::registry()
                .with(filter)
                .with(cli_layer)
                .with(console_layer);
            #[cfg(target_os = "android")]
            {
                registry = registry.with(tracing_android::AndroidLayer::default());
            }
            if registry.try_init().is_err() {
                eprintln!("WaterUI tracing forwarder failed to initialize; logs may be missing.");
            }
        });
    }

    #[derive(Clone, Default)]
    struct PrefixedMakeWriter;

    impl<'a> MakeWriter<'a> for PrefixedMakeWriter {
        type Writer = PrefixedWriter<std::io::Stderr>;

        fn make_writer(&'a self) -> Self::Writer {
            PrefixedWriter::new(std::io::stderr())
        }
    }

    struct PrefixedWriter<W> {
        inner: W,
        wrote_prefix: bool,
    }

    impl<W> PrefixedWriter<W> {
        const fn new(inner: W) -> Self {
            Self {
                inner,
                wrote_prefix: false,
            }
        }

        fn write_prefix(&mut self) -> io::Result<()> {
            if self.wrote_prefix {
                return Ok(());
            }
            self.wrote_prefix = true;
            self.inner.write_all(TRACING_PREFIX.as_bytes())
        }
    }

    impl<W: Write> Write for PrefixedWriter<W> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.write_prefix()?;
            self.inner.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    #[derive(Clone)]
    struct PanicAwareFormatter {
        default: Format<DefaultFields>,
    }

    impl PanicAwareFormatter {
        fn new() -> Self {
            Self {
                default: fmt::format().with_ansi(false).without_time(),
            }
        }

        fn render_panic(&self, fields: &PanicFields) -> String {
            let mut parts = Vec::new();
            let message = fields
                .message
                .as_deref()
                .unwrap_or("panic occurred without message");
            parts.push(format!("PANIC: {message}"));
            if let Some(backtrace) = fields.backtrace.as_deref() {
                let mut lines = backtrace.lines().take(MAX_PANIC_LINES);
                let snippet: Vec<_> = lines.by_ref().collect();
                if !snippet.is_empty() {
                    parts.push("Stack:".to_string());
                    parts.extend(snippet.iter().map(|line| format!("  {line}")));
                    if backtrace.lines().count() > MAX_PANIC_LINES {
                        parts.push("  ... (truncated)".to_string());
                    }
                }
            }
            parts.join("\n")
        }
    }

    impl<S, N> FormatEvent<S, N> for PanicAwareFormatter
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span>,
        N: for<'writer> FormatFields<'writer> + 'static,
    {
        fn format_event(
            &self,
            ctx: &FmtContext<'_, S, N>,
            writer: &mut dyn Write,
            event: &tracing::Event<'_>,
        ) -> fmt::Result {
            if is_panic_event(event.metadata()) {
                let mut visitor = PanicVisitor::default();
                event.record(&mut visitor);
                let rendered = self.render_panic(&visitor.fields);
                write!(writer, "{rendered}")
            } else {
                self.default.format_event(ctx, writer, event)
            }
        }
    }

    #[derive(Default)]
    struct PanicFields {
        message: Option<String>,
        backtrace: Option<String>,
    }

    #[derive(Default)]
    struct PanicVisitor {
        fields: PanicFields,
    }

    impl tracing::field::Visit for PanicVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            match field.name() {
                "message" => self.fields.message = Some(format!("{value:?}")),
                "backtrace" => self.fields.backtrace = Some(format!("{value:?}")),
                _ => {}
            }
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            match field.name() {
                "message" => self.fields.message = Some(value.to_string()),
                "backtrace" => self.fields.backtrace = Some(value.to_string()),
                _ => {}
            }
        }
    }

    const MAX_PANIC_LINES: usize = 8;

    fn is_panic_event(metadata: &tracing::Metadata<'_>) -> bool {
        let target = metadata.target();
        target.contains("panic")
    }

    #[derive(Clone)]
    struct CliForwardLayer {
        formatter: PanicAwareFormatter,
    }

    impl CliForwardLayer {
        fn new(formatter: PanicAwareFormatter) -> Self {
            Self { formatter }
        }
    }

    impl<S> Layer<S> for CliForwardLayer
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span>,
    {
        fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
            let Some(sender) = outbound_sender() else {
                return;
            };
            let mut rendered = Vec::new();
            if self
                .formatter
                .format_event(&FmtContext::new(&ctx, event), &mut rendered, event)
                .is_err()
            {
                return;
            }
            let rendered = String::from_utf8_lossy(&rendered);
            let message = rendered.trim_end_matches('\n').to_string();
            let event = ClientEvent::log(
                LogEvent::new(
                    message,
                    event.metadata().level(),
                    Some(event.metadata().target().to_string()),
                ),
                TRACING_PREFIX,
            );
            let text = event.into_json_string();
            let _ = sender.send_blocking(text);
        }
    }

    fn report_panic(info: &PanicInfo) {
        log_panic_via_tracing(info);
        if let Some(sender) = outbound_sender() {
            let event = ClientEvent::from_panic(info);
            let text = event.into_json_string();
            let _ = sender.send_blocking(text);
        }
    }

    fn log_panic_via_tracing(info: &PanicInfo) {
        let message = panic_message(info);
        let thread_name = thread::current().name().unwrap_or("unnamed");
        let backtrace = Backtrace::force_capture().to_string();
        tracing::error!(
            target: "waterui::panic",
            message = %message,
            thread = thread_name,
            backtrace = %backtrace
        );
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
        Log(LogEvent),
    }

    impl ClientEvent {
        fn from_panic(info: &PanicInfo) -> Self {
            Self::Panic(PanicReport::from(info))
        }

        fn log(event: LogEvent, prefix: &str) -> Self {
            let mut event = event;
            event.apply_prefix(prefix);
            Self::Log(event)
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
                Self::Log(log) => {
                    event.insert("type".into(), Value::String("log".into()));
                    event.insert("log".into(), log.into_json_value());
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

    #[derive(Debug)]
    struct LogEvent {
        message: String,
        level: Level,
        target: Option<String>,
    }

    impl LogEvent {
        fn new(message: String, level: &Level, target: Option<String>) -> Self {
            Self {
                message,
                level: *level,
                target,
            }
        }

        fn apply_prefix(&mut self, prefix: &str) {
            if self.message.starts_with(prefix) {
                return;
            }
            self.message = format!("{prefix} {}", self.message);
        }

        fn into_json_value(self) -> Value {
            let mut map = Map::with_capacity(3);
            map.insert("message".into(), Value::String(self.message));
            map.insert(
                "level".into(),
                Value::String(self.level.as_str().to_string()),
            );
            if let Some(target) = self.target {
                map.insert("target".into(), Value::String(target));
            }
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
