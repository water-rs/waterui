//! Log and panic forwarding to CLI.

use alloc::string::String;
use async_channel::Sender;
use serde_json::{Map, Number, Value};
use std::backtrace::Backtrace;
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::str::FromStr;
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::thread;
use tracing::Level;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::fmt::{self, FormatEvent, FormatFields};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const TRACING_PREFIX: &str = "[waterui::tracing]";
const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::INFO;
const MAX_PANIC_LINES: usize = 8;

// ============================================================================
// Global State
// ============================================================================

static PANIC_HOOK_INSTALLED: Once = Once::new();
static TRACING_INSTALLED: Once = Once::new();
static OUTBOUND_SENDER: OnceLock<Mutex<Option<Sender<String>>>> = OnceLock::new();
static LOG_LEVEL: OnceLock<Arc<Mutex<LevelFilter>>> = OnceLock::new();

/// Register the outbound sender for log/panic forwarding.
pub fn register_sender(sender: Sender<String>) {
    let slot = OUTBOUND_SENDER.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(sender);
    }
}

/// Clear the outbound sender.
pub fn clear_sender() {
    if let Some(slot) = OUTBOUND_SENDER.get()
        && let Ok(mut guard) = slot.lock()
    {
        guard.take();
    }
}

fn get_sender() -> Option<Sender<String>> {
    OUTBOUND_SENDER
        .get()
        .and_then(|slot| slot.lock().ok().and_then(|s| s.clone()))
}

fn get_log_level_handle() -> Arc<Mutex<LevelFilter>> {
    LOG_LEVEL
        .get_or_init(|| Arc::new(Mutex::new(DEFAULT_LOG_LEVEL)))
        .clone()
}

/// Update the CLI log filter level.
pub fn set_log_level(level: &str) {
    let parsed = LevelFilter::from_str(level).unwrap_or(DEFAULT_LOG_LEVEL);
    if let Ok(mut guard) = get_log_level_handle().lock() {
        *guard = parsed;
    }
}

// ============================================================================
// Installation
// ============================================================================

/// Install the panic forwarder (idempotent).
pub fn install_panic_forwarder() {
    PANIC_HOOK_INSTALLED.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            forward_panic(info);
            previous(info);
        }));
    });
}

/// Install the tracing forwarder (idempotent).
pub fn install_tracing_forwarder() {
    TRACING_INSTALLED.call_once(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let formatter = PanicAwareFormatter;

        let console = fmt::layer()
            .event_format(formatter.clone())
            .with_writer(PrefixedWriter)
            .with_ansi(false)
            .with_filter(filter);

        let cli = CliForwardLayer::new(formatter);

        #[cfg(target_os = "android")]
        let result = {
            let registry = tracing_subscriber::registry().with(cli).with(console);
            if let Ok(android) = tracing_android::layer("WaterUI") {
                registry.with(android).try_init()
            } else {
                registry.try_init()
            }
        };

        #[cfg(not(target_os = "android"))]
        let result = tracing_subscriber::registry()
            .with(cli)
            .with(console)
            .try_init();

        if result.is_err() {
            eprintln!("WaterUI tracing forwarder failed to initialize");
        }
    });
}

// ============================================================================
// Panic Forwarding
// ============================================================================

fn forward_panic(info: &PanicHookInfo) {
    // Log via tracing
    let message = extract_panic_message(info);
    let thread_name = thread::current().name().unwrap_or("unnamed").to_string();
    let backtrace = Backtrace::force_capture().to_string();

    tracing::error!(
        target: "waterui::panic",
        message = %message,
        thread = %thread_name,
        backtrace = %backtrace
    );

    // Send to CLI
    if let Some(sender) = get_sender() {
        let report = PanicReport::from_info(info);
        let json = report.to_json();
        let _ = sender.send_blocking(json);
    }
}

fn extract_panic_message(info: &PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown panic".to_string()
    }
}

struct PanicReport {
    message: String,
    file: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    thread: Option<String>,
    backtrace: String,
}

impl PanicReport {
    fn from_info(info: &PanicHookInfo<'_>) -> Self {
        let loc = info.location();
        Self {
            message: extract_panic_message(info),
            file: loc.map(|l| l.file().to_string()),
            line: loc.map(std::panic::Location::line),
            column: loc.map(std::panic::Location::column),
            thread: thread::current().name().map(String::from),
            backtrace: Backtrace::force_capture().to_string(),
        }
    }

    fn to_json(&self) -> String {
        let mut panic = Map::new();
        panic.insert("message".into(), Value::String(self.message.clone()));

        if let Some(ref file) = self.file {
            let mut loc = Map::new();
            loc.insert("file".into(), Value::String(file.clone()));
            if let Some(line) = self.line {
                loc.insert("line".into(), Value::Number(Number::from(line)));
            }
            if let Some(col) = self.column {
                loc.insert("column".into(), Value::Number(Number::from(col)));
            }
            panic.insert("location".into(), Value::Object(loc));
        }

        if let Some(ref thread) = self.thread {
            panic.insert("thread".into(), Value::String(thread.clone()));
        }

        panic.insert("backtrace".into(), Value::String(self.backtrace.clone()));

        let mut event = Map::new();
        event.insert("type".into(), Value::String("panic".into()));
        event.insert("panic".into(), Value::Object(panic));

        Value::Object(event).to_string()
    }
}

// ============================================================================
// Tracing Layer
// ============================================================================

#[derive(Clone)]
struct CliForwardLayer {
    #[allow(dead_code)]
    formatter: PanicAwareFormatter,
    level: Arc<Mutex<LevelFilter>>,
}

impl CliForwardLayer {
    fn new(formatter: PanicAwareFormatter) -> Self {
        Self {
            formatter,
            level: get_log_level_handle(),
        }
    }
}

impl<S> Layer<S> for CliForwardLayer
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let Some(sender) = get_sender() else { return };

        let filter = self.level.lock().map(|g| *g).unwrap_or(DEFAULT_LOG_LEVEL);
        if !level_allows(filter, *event.metadata().level()) {
            return;
        }

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.unwrap_or_default();

        let json = log_to_json(
            &message,
            event.metadata().level(),
            event.metadata().target(),
        );
        let _ = sender.send_blocking(json);
    }
}

const fn level_allows(filter: LevelFilter, level: Level) -> bool {
    match filter {
        LevelFilter::OFF => false,
        LevelFilter::ERROR => matches!(level, Level::ERROR),
        LevelFilter::WARN => matches!(level, Level::ERROR | Level::WARN),
        LevelFilter::INFO => matches!(level, Level::ERROR | Level::WARN | Level::INFO),
        LevelFilter::DEBUG => matches!(
            level,
            Level::ERROR | Level::WARN | Level::INFO | Level::DEBUG
        ),
        LevelFilter::TRACE => true,
    }
}

fn log_to_json(message: &str, level: &Level, target: &str) -> String {
    let mut log = Map::new();
    log.insert(
        "message".into(),
        Value::String(format!("{TRACING_PREFIX} {message}")),
    );
    log.insert("level".into(), Value::String(level.as_str().to_string()));
    log.insert("target".into(), Value::String(target.to_string()));

    let mut event = Map::new();
    event.insert("type".into(), Value::String("log".into()));
    event.insert("log".into(), Value::Object(log));

    Value::Object(event).to_string()
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

// ============================================================================
// Console Output Formatter
// ============================================================================

#[derive(Clone, Default)]
struct PrefixedWriter;

impl<'a> MakeWriter<'a> for PrefixedWriter {
    type Writer = PrefixedWriterInner<std::io::Stderr>;

    fn make_writer(&'a self) -> Self::Writer {
        PrefixedWriterInner {
            inner: std::io::stderr(),
            wrote_prefix: false,
        }
    }
}

struct PrefixedWriterInner<W> {
    inner: W,
    wrote_prefix: bool,
}

impl<W: Write> Write for PrefixedWriterInner<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.wrote_prefix {
            self.inner.write_all(TRACING_PREFIX.as_bytes())?;
            self.wrote_prefix = true;
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Clone, Default)]
struct PanicAwareFormatter;

impl<S, N> FormatEvent<S, N> for PanicAwareFormatter
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let target = event.metadata().target();

        if target.contains("panic") {
            // Render panic with truncated backtrace
            let mut visitor = PanicFieldVisitor::default();
            event.record(&mut visitor);

            let msg = visitor.message.as_deref().unwrap_or("panic");
            write!(writer, "PANIC: {msg}")?;

            if let Some(bt) = visitor.backtrace.as_deref() {
                writeln!(writer)?;
                write!(writer, "Stack:")?;
                for line in bt.lines().take(MAX_PANIC_LINES) {
                    write!(writer, "\n  {line}")?;
                }
                if bt.lines().count() > MAX_PANIC_LINES {
                    write!(writer, "\n  ... (truncated)")?;
                }
            }
            Ok(())
        } else {
            // Standard format
            let level = event.metadata().level();
            write!(writer, "{level} {target}: ")?;
            ctx.field_format().format_fields(writer.by_ref(), event)?;
            Ok(())
        }
    }
}

#[derive(Default)]
struct PanicFieldVisitor {
    message: Option<String>,
    backtrace: Option<String>,
}

impl tracing::field::Visit for PanicFieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = Some(format!("{value:?}")),
            "backtrace" => self.backtrace = Some(format!("{value:?}")),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.message = Some(value.to_string()),
            "backtrace" => self.backtrace = Some(value.to_string()),
            _ => {}
        }
    }
}
