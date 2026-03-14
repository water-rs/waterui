//! Runtime-side Inspector endpoint for streaming diagnostics.

use std::backtrace::Backtrace;
use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use waterui_inspector_protocol::transport::{read_frame_blocking, write_frame_blocking};
use waterui_inspector_protocol::{
    InspectorClientMessage, InspectorEvent, InspectorEventEnvelope, InspectorServerMessage,
    MainThreadStallEvent, RuntimePollEvent, protocol_info,
};

use crate::task::{RuntimeProbe, TaskPollSample, install_runtime_probe};

const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const DEFAULT_PORT: u16 = 0;
const DEFAULT_MAX_EVENTS: usize = 4096;
const DEFAULT_WARN_RATIO: f64 = 0.90;
const DEFAULT_FLUSH_INTERVAL_MS: u64 = 50;

/// Resolved runtime endpoint info.
#[derive(Debug, Clone)]
pub struct InspectorEndpointInfo {
    /// Bound socket address.
    pub addr: SocketAddr,
    /// One-time token required by clients.
    pub token: String,
}

/// Runtime inspector server configuration.
#[derive(Debug, Clone)]
pub struct InspectorServerConfig {
    /// Host to bind.
    pub host: IpAddr,
    /// Port to bind. `0` means ephemeral.
    pub port: u16,
    /// One-time connection token.
    pub token: String,
    /// In-memory event ring buffer length.
    pub max_events: usize,
    /// Warn ratio threshold used to emit stall events.
    pub warn_ratio: f64,
    /// Event flush interval per client.
    pub flush_interval: Duration,
}

impl InspectorServerConfig {
    /// Build config from environment variables.
    ///
    /// Required:
    /// - `WATERUI_INSPECTOR_TOKEN`
    ///
    /// Optional:
    /// - `WATERUI_INSPECTOR_HOST` (default `127.0.0.1`)
    /// - `WATERUI_INSPECTOR_PORT` (default `0`)
    /// - `WATERUI_INSPECTOR_MAX_EVENTS` (default `4096`)
    /// - `WATERUI_INSPECTOR_WARN_RATIO` (default `0.90`)
    /// - `WATERUI_INSPECTOR_FLUSH_INTERVAL_MS` (default `50`)
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(token) = std::env::var("WATERUI_INSPECTOR_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };

        let host = match std::env::var("WATERUI_INSPECTOR_HOST") {
            Ok(raw) => raw
                .parse::<IpAddr>()
                .map_err(|error| format!("Invalid WATERUI_INSPECTOR_HOST `{raw}`: {error}"))?,
            Err(std::env::VarError::NotPresent) => DEFAULT_HOST,
            Err(error) => {
                return Err(format!(
                    "Invalid WATERUI_INSPECTOR_HOST environment value: {error}"
                ));
            }
        };

        let port = match std::env::var("WATERUI_INSPECTOR_PORT") {
            Ok(raw) => raw
                .parse::<u16>()
                .map_err(|error| format!("Invalid WATERUI_INSPECTOR_PORT `{raw}`: {error}"))?,
            Err(std::env::VarError::NotPresent) => DEFAULT_PORT,
            Err(error) => {
                return Err(format!(
                    "Invalid WATERUI_INSPECTOR_PORT environment value: {error}"
                ));
            }
        };

        let max_events = match std::env::var("WATERUI_INSPECTOR_MAX_EVENTS") {
            Ok(raw) => {
                let parsed = raw.parse::<usize>().map_err(|error| {
                    format!("Invalid WATERUI_INSPECTOR_MAX_EVENTS `{raw}`: {error}")
                })?;
                if parsed == 0 {
                    return Err("WATERUI_INSPECTOR_MAX_EVENTS must be > 0".to_string());
                }
                parsed
            }
            Err(std::env::VarError::NotPresent) => DEFAULT_MAX_EVENTS,
            Err(error) => {
                return Err(format!(
                    "Invalid WATERUI_INSPECTOR_MAX_EVENTS environment value: {error}"
                ));
            }
        };

        let warn_ratio = match std::env::var("WATERUI_INSPECTOR_WARN_RATIO") {
            Ok(raw) => {
                let parsed = raw.parse::<f64>().map_err(|error| {
                    format!("Invalid WATERUI_INSPECTOR_WARN_RATIO `{raw}`: {error}")
                })?;
                if !parsed.is_finite() || parsed <= 0.0 {
                    return Err("WATERUI_INSPECTOR_WARN_RATIO must be finite and > 0".to_string());
                }
                parsed
            }
            Err(std::env::VarError::NotPresent) => DEFAULT_WARN_RATIO,
            Err(error) => {
                return Err(format!(
                    "Invalid WATERUI_INSPECTOR_WARN_RATIO environment value: {error}"
                ));
            }
        };

        let flush_interval_ms = match std::env::var("WATERUI_INSPECTOR_FLUSH_INTERVAL_MS") {
            Ok(raw) => {
                let parsed = raw.parse::<u64>().map_err(|error| {
                    format!("Invalid WATERUI_INSPECTOR_FLUSH_INTERVAL_MS `{raw}`: {error}")
                })?;
                if parsed == 0 {
                    return Err("WATERUI_INSPECTOR_FLUSH_INTERVAL_MS must be > 0".to_string());
                }
                parsed
            }
            Err(std::env::VarError::NotPresent) => DEFAULT_FLUSH_INTERVAL_MS,
            Err(error) => {
                return Err(format!(
                    "Invalid WATERUI_INSPECTOR_FLUSH_INTERVAL_MS environment value: {error}"
                ));
            }
        };

        Ok(Some(Self {
            host,
            port,
            token,
            max_events,
            warn_ratio,
            flush_interval: Duration::from_millis(flush_interval_ms),
        }))
    }
}

#[derive(Debug)]
struct InspectorServer {
    endpoint: InspectorEndpointInfo,
}

static SERVER: OnceLock<InspectorServer> = OnceLock::new();
static SERVER_INIT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Returns the current inspector endpoint if initialized.
#[must_use]
pub fn endpoint() -> Option<InspectorEndpointInfo> {
    SERVER.get().map(|s| s.endpoint.clone())
}

/// Initializes inspector runtime endpoint from environment when configured.
#[must_use]
pub fn maybe_init_from_env() -> Option<InspectorEndpointInfo> {
    let config = match InspectorServerConfig::from_env() {
        Ok(Some(config)) => config,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(
                target: "waterui::inspector",
                error = %error,
                "Invalid inspector runtime environment configuration"
            );
            return None;
        }
    };
    match init_with_config(config) {
        Ok(endpoint) => Some(endpoint),
        Err(error) => {
            tracing::warn!(
                target: "waterui::inspector",
                error = %error,
                "Failed to initialize inspector runtime endpoint"
            );
            None
        }
    }
}

/// Initializes the inspector runtime endpoint.
///
/// If already initialized, returns the existing endpoint.
pub fn init_with_config(config: InspectorServerConfig) -> io::Result<InspectorEndpointInfo> {
    let init_lock = SERVER_INIT_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = match init_lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    if let Some(existing) = SERVER.get() {
        return Ok(existing.endpoint.clone());
    }

    let listener = TcpListener::bind(SocketAddr::new(config.host, config.port))?;
    listener.set_nonblocking(false)?;
    let addr = listener.local_addr()?;

    let state = Arc::new(SharedEvents::new(config.max_events));
    let runtime_probe = Arc::new(InspectorRuntimeProbe {
        warn_ratio: config.warn_ratio,
        state: Arc::clone(&state),
    });

    let token = config.token.clone();
    let flush_interval = config.flush_interval;
    thread::Builder::new()
        .name("waterui-inspector-server".to_string())
        .spawn(move || run_server(listener, state, token, flush_interval))
        .map_err(|e| io::Error::other(format!("failed to spawn inspector server thread: {e}")))?;

    let endpoint = InspectorEndpointInfo {
        addr,
        token: config.token,
    };

    let _ = SERVER.set(InspectorServer {
        endpoint: endpoint.clone(),
    });
    install_runtime_probe(runtime_probe);

    tracing::info!(
        target: "waterui::inspector",
        inspector_addr = %addr,
        "Inspector runtime endpoint listening"
    );

    Ok(endpoint)
}

#[derive(Debug)]
struct SharedEvents {
    max_events: usize,
    inner: Mutex<EventBuffer>,
}

#[derive(Debug, Default)]
struct EventBuffer {
    next_seq: u64,
    events: VecDeque<InspectorEventEnvelope>,
}

impl SharedEvents {
    fn new(max_events: usize) -> Self {
        Self {
            max_events,
            inner: Mutex::new(EventBuffer::default()),
        }
    }

    fn push_event(&self, event: InspectorEvent) {
        let timestamp_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System clock is before UNIX_EPOCH")
            .as_millis()
            .try_into()
            .expect("UNIX millisecond timestamp overflowed u64");

        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let seq = guard.next_seq;
        guard.next_seq = guard.next_seq.saturating_add(1);
        guard.events.push_back(InspectorEventEnvelope {
            seq,
            timestamp_unix_ms,
            event,
        });

        while guard.events.len() > self.max_events {
            guard.events.pop_front();
        }
    }

    fn collect_after(&self, seq: u64) -> Vec<InspectorEventEnvelope> {
        let guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        guard
            .events
            .iter()
            .filter(|envelope| envelope.seq > seq)
            .cloned()
            .collect()
    }
}

#[derive(Debug)]
struct InspectorRuntimeProbe {
    warn_ratio: f64,
    state: Arc<SharedEvents>,
}

impl RuntimeProbe for InspectorRuntimeProbe {
    fn on_poll_sample(&self, sample: &TaskPollSample) {
        let poll = RuntimePollEvent {
            task_type: sample.task_type.to_string(),
            poll_ready: sample.poll_ready,
            wall_us: duration_to_us(sample.wall),
            cpu_us: duration_to_us(sample.cpu),
            budget_us: duration_to_us(sample.frame_budget),
            refresh_hz: sample.refresh_hz,
        };
        self.state.push_event(InspectorEvent::RuntimePoll(poll));

        let budget_secs = sample.frame_budget.as_secs_f64();
        if budget_secs <= 0.0 {
            return;
        }

        let usage_ratio = sample.wall.as_secs_f64() / budget_secs;
        if usage_ratio < self.warn_ratio {
            return;
        }

        let stall = MainThreadStallEvent {
            task_type: sample.task_type.to_string(),
            poll_ready: sample.poll_ready,
            wall_us: duration_to_us(sample.wall),
            cpu_us: duration_to_us(sample.cpu),
            budget_us: duration_to_us(sample.frame_budget),
            overrun_us: duration_to_us(sample.wall.saturating_sub(sample.frame_budget)),
            usage_pct: usage_ratio * 100.0,
            backtrace: Some(Backtrace::force_capture().to_string()),
        };
        self.state
            .push_event(InspectorEvent::MainThreadStall(stall));
    }
}

fn duration_to_us(duration: Duration) -> u64 {
    duration
        .as_micros()
        .try_into()
        .expect("Duration microseconds overflowed u64")
}

fn run_server(
    listener: TcpListener,
    state: Arc<SharedEvents>,
    token: String,
    flush_interval: Duration,
) {
    for connection in listener.incoming() {
        let stream = match connection {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(
                    target: "waterui::inspector",
                    error = %error,
                    "Failed to accept inspector connection"
                );
                continue;
            }
        };

        let peer = stream.peer_addr().ok();
        tracing::debug!(
            target: "waterui::inspector",
            peer = ?peer,
            "Inspector client connected"
        );

        let state = Arc::clone(&state);
        let token = token.clone();
        let spawn_result = thread::Builder::new()
            .name("waterui-inspector-client".to_string())
            .spawn(move || {
                if let Err(error) =
                    handle_connection(stream, &state, token.as_str(), flush_interval)
                {
                    tracing::warn!(
                        target: "waterui::inspector",
                        peer = ?peer,
                        error = %error,
                        "Inspector client disconnected"
                    );
                }
            });

        if let Err(error) = spawn_result {
            tracing::warn!(
                target: "waterui::inspector",
                peer = ?peer,
                error = %error,
                "Failed to spawn inspector client worker"
            );
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    state: &SharedEvents,
    expected_token: &str,
    flush_interval: Duration,
) -> io::Result<()> {
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

    let hello: InspectorClientMessage = read_frame_blocking(&mut stream)?;
    let from_seq = match hello {
        InspectorClientMessage::Hello { token, from_seq } => {
            if token != expected_token {
                let _ = write_frame_blocking(
                    &mut stream,
                    &InspectorServerMessage::Error {
                        message: "invalid inspector token".to_string(),
                    },
                );
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "invalid inspector token",
                ));
            }
            from_seq.unwrap_or(0)
        }
        InspectorClientMessage::Ping => {
            let _ = write_frame_blocking(&mut stream, &InspectorServerMessage::Pong);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected hello message",
            ));
        }
    };

    stream.set_read_timeout(None).ok();

    write_frame_blocking(
        &mut stream,
        &InspectorServerMessage::HelloAck {
            protocol: protocol_info(),
            app_pid: std::process::id(),
        },
    )?;

    let mut cursor = from_seq;
    loop {
        let pending = state.collect_after(cursor);
        if pending.is_empty() {
            thread::sleep(flush_interval);
            continue;
        }

        for envelope in pending {
            cursor = envelope.seq;
            write_frame_blocking(&mut stream, &InspectorServerMessage::Event { envelope })?;
        }
    }
}
