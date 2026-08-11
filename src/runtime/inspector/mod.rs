//! Runtime endpoint that streams diagnostics to the Inspector app.
//!
//! # Nothing is produced unobserved
//!
//! The endpoint is subscription-driven. Every producer — the task probe, the
//! rendering backend, the reactive-graph bridge, the log layer — asks whether
//! anyone wants its channel before it spends anything. With no inspector
//! attached the cost is one relaxed atomic load per would-be event.
//!
//! # Wiring
//!
//! A backend takes the pieces it can feed and ignores the rest:
//!
//! ```ignore
//! let Some(inspector) = waterui::inspector::maybe_init_from_env() else { return };
//! let probe = inspector.runtime_probe();          // main-thread occupancy
//! let frames = inspector.frame_recorder();        // per-frame timings
//! let trees = inspector.tree_recorder();          // accessibility tree
//! let layer = inspector.tracing_layer();          // tracing records
//! # #[cfg(feature = "inspector-signals")]
//! let _scope = inspector.observe_signals();       // reactive graph, main thread
//! ```

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use waterui_core::Environment;
use waterui_inspector_protocol::{ChannelSet, TargetInfo};

use crate::task::RuntimeProbe;

mod hub;
mod logs;
mod recorder;
mod server;
mod tasks;
/// The reactive-graph bridge only exists when `nami` is built with graph
/// observability; without it there is no observer trait to implement.
#[cfg(feature = "inspector-signals")]
mod signals;

use hub::EventHub;
pub use logs::InspectorLayer;
pub use recorder::{FrameRecorder, TreeRecorder};

/// How long a connecting peer has to complete the handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const DEFAULT_PORT: u16 = 0;
const DEFAULT_TASK_WINDOW_MS: u64 = 100;
const DEFAULT_STALL_RATIO: f64 = 0.90;

/// Where the endpoint is listening and the token required to talk to it.
#[derive(Debug, Clone)]
pub struct InspectorEndpointInfo {
    /// Bound socket address.
    pub addr: SocketAddr,
    /// One-time token clients must present.
    pub token: String,
}

/// Runtime endpoint configuration.
#[derive(Debug, Clone)]
pub struct InspectorServerConfig {
    /// Address to bind.
    pub host: IpAddr,
    /// Port to bind; `0` picks an ephemeral one.
    pub port: u16,
    /// One-time connection token.
    pub token: String,
    /// How often task occupancy is aggregated and published.
    pub task_window: Duration,
    /// Fraction of the frame budget above which a poll is reported as a stall.
    pub stall_ratio: f64,
    /// Name reported to the inspector.
    pub app_name: String,
}

impl InspectorServerConfig {
    /// Builds a configuration from the environment, if inspection is enabled.
    ///
    /// Inspection is enabled by `WATERUI_INSPECTOR_TOKEN`. Optional settings:
    /// `WATERUI_INSPECTOR_HOST`, `WATERUI_INSPECTOR_PORT`,
    /// `WATERUI_INSPECTOR_TASK_WINDOW_MS`, `WATERUI_INSPECTOR_STALL_RATIO`.
    ///
    /// # Errors
    ///
    /// Returns an error when a variable is present but unparseable. A malformed
    /// setting is a mistake worth reporting rather than quietly ignoring.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(token) = std::env::var("WATERUI_INSPECTOR_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };

        Ok(Some(Self {
            host: parse_env("WATERUI_INSPECTOR_HOST")?.unwrap_or(DEFAULT_HOST),
            port: parse_env("WATERUI_INSPECTOR_PORT")?.unwrap_or(DEFAULT_PORT),
            token,
            task_window: Duration::from_millis(
                parse_env("WATERUI_INSPECTOR_TASK_WINDOW_MS")?.unwrap_or(DEFAULT_TASK_WINDOW_MS),
            ),
            stall_ratio: parse_env("WATERUI_INSPECTOR_STALL_RATIO")?.unwrap_or(DEFAULT_STALL_RATIO),
            app_name: std::env::var("WATERUI_INSPECTOR_APP_NAME")
                .unwrap_or_else(|_| String::from("WaterUI application")),
        }))
    }
}

fn parse_env<T>(name: &str) -> Result<Option<T>, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(raw) => raw
            .parse::<T>()
            .map(Some)
            .map_err(|error| format!("Invalid {name} `{raw}`: {error}")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(format!("Invalid {name} environment value: {error}")),
    }
}

/// A listening inspector endpoint and the producers that feed it.
pub struct InspectorRuntime {
    endpoint: InspectorEndpointInfo,
    hub: Arc<EventHub>,
    task_probe: Arc<tasks::TaskProbe>,
}

impl std::fmt::Debug for InspectorRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InspectorRuntime")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl InspectorRuntime {
    /// Where this endpoint is listening.
    #[must_use]
    pub const fn endpoint(&self) -> &InspectorEndpointInfo {
        &self.endpoint
    }

    /// The probe that reports main-thread occupancy.
    #[must_use]
    pub fn runtime_probe(&self) -> Arc<dyn RuntimeProbe> {
        Arc::clone(&self.task_probe) as Arc<dyn RuntimeProbe>
    }

    /// A handle the rendering backend uses to report frame timings.
    #[must_use]
    pub fn frame_recorder(&self) -> FrameRecorder {
        FrameRecorder::new(Arc::clone(&self.hub))
    }

    /// A handle the rendering backend uses to report accessibility trees.
    #[must_use]
    pub fn tree_recorder(&self) -> TreeRecorder {
        TreeRecorder::new(Arc::clone(&self.hub))
    }

    /// A `tracing` layer that forwards records to the inspector.
    #[must_use]
    pub fn tracing_layer(&self) -> InspectorLayer {
        InspectorLayer::new(Arc::clone(&self.hub))
    }

    /// Observes the reactive graph for as long as the returned scope lives.
    ///
    /// Must be called on the thread that owns the UI: the reactive graph is
    /// thread-confined, and so is its observer.
    #[cfg(feature = "inspector-signals")]
    #[must_use]
    pub fn observe_signals(&self) -> nami::observe::ObserverScope {
        use std::rc::Rc;
        nami::observe::ObserverScope::install(Rc::new(signals::SignalBridge::new(
            Arc::clone(&self.hub),
        )))
    }
}

/// Publishes the inspector's recorders through the environment.
///
/// Frame timings and accessibility trees are backend knowledge, so a backend has
/// to be able to reach the recorders. The environment is the context every
/// backend already receives, which makes it the right carrier — no process-wide
/// state, and a backend running without an inspector simply finds nothing.
///
/// Passing `None` is the normal case for an app that was not launched for
/// inspection, and does nothing.
pub fn install(environment: &mut Environment, inspector: Option<InspectorRuntime>) {
    let Some(inspector) = inspector else {
        return;
    };
    environment.insert(inspector.frame_recorder());
    environment.insert(inspector.tree_recorder());
    environment.insert(inspector);
}

/// Which channels this build can actually produce.
///
/// Advertised in the handshake so an inspector can grey out what it will never
/// receive instead of waiting forever for it.
fn available_channels() -> ChannelSet {
    let mut available = ChannelSet::FRAMES | ChannelSet::TREE | ChannelSet::TASKS | ChannelSet::LOGS;
    if cfg!(feature = "inspector-signals") {
        available |= ChannelSet::SIGNALS;
    }
    available
}

/// Starts the endpoint from the environment, if inspection is enabled.
#[must_use]
pub fn maybe_init_from_env() -> Option<InspectorRuntime> {
    let config = match InspectorServerConfig::from_env() {
        Ok(Some(config)) => config,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(
                target: "waterui::inspector",
                error = %error,
                "Invalid inspector configuration; inspection disabled"
            );
            return None;
        }
    };

    match init_with_config(config) {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            tracing::warn!(
                target: "waterui::inspector",
                error = %error,
                "Failed to start the inspector endpoint"
            );
            None
        }
    }
}

/// Starts the endpoint.
///
/// # Errors
///
/// Returns an I/O error when the socket cannot be bound or a thread cannot be
/// spawned.
pub fn init_with_config(config: InspectorServerConfig) -> io::Result<InspectorRuntime> {
    let listener = TcpListener::bind(SocketAddr::new(config.host, config.port))?;
    listener.set_nonblocking(false)?;
    let addr = listener.local_addr()?;

    let (hub, receiver) = EventHub::new();
    let available = available_channels();
    let target = TargetInfo {
        pid: std::process::id(),
        name: config.app_name.clone(),
        backend: backend_name().to_string(),
    };

    spawn("waterui-inspector-dispatch", {
        let hub = Arc::clone(&hub);
        move || server::dispatch_loop(&hub, &receiver, available)
    })?;

    spawn("waterui-inspector-accept", {
        let hub = Arc::clone(&hub);
        let token = config.token.clone();
        move || server::accept_loop(&listener, &hub, token.as_str(), &target, available)
    })?;

    tracing::info!(
        target: "waterui::inspector",
        inspector_addr = %addr,
        "Inspector endpoint listening"
    );

    Ok(InspectorRuntime {
        endpoint: InspectorEndpointInfo {
            addr,
            token: config.token,
        },
        task_probe: Arc::new(tasks::TaskProbe::new(
            Arc::clone(&hub),
            config.task_window,
            config.stall_ratio,
        )),
        hub,
    })
}

fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> io::Result<()> {
    thread::Builder::new()
        .name(name.to_string())
        .spawn(body)
        .map(drop)
        .map_err(|error| io::Error::other(format!("failed to spawn {name} thread: {error}")))
}

/// Best available description of the rendering backend, for the handshake.
const fn backend_name() -> &'static str {
    if cfg!(target_vendor = "apple") {
        "apple"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "hydrolysis"
    }
}
