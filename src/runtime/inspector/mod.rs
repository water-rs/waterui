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
#[cfg(not(target_arch = "wasm32"))]
use std::net::TcpListener;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
use std::time::Duration;

use waterui_core::Environment;
#[cfg(not(target_arch = "wasm32"))]
use waterui_inspector_protocol::discovery::Advertisement;
#[cfg(not(target_arch = "wasm32"))]
use waterui_inspector_protocol::{ChannelSet, TargetInfo};

use crate::task::RuntimeProbe;

mod hub;
mod logs;
mod recorder;
#[cfg(not(target_arch = "wasm32"))]
mod server;
/// The reactive-graph bridge only exists when `nami` is built with graph
/// observability; without it there is no observer trait to implement.
#[cfg(feature = "inspector-signals")]
mod signals;
mod tasks;

use hub::EventHub;
/// The wire protocol this endpoint speaks.
///
/// Re-exported because the public API is stated in its terms — inspecting an
/// element names a [`protocol::NodeId`] — and a caller cannot depend on the
/// protocol crate just to say which node it means.
pub use waterui_inspector_protocol as protocol;

pub use logs::InspectorLayer;
pub use recorder::{FrameRecorder, TreeRecorder};

/// How long a connecting peer has to complete the handshake.
#[cfg(not(target_arch = "wasm32"))]
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

    /// The configuration a debug build inspects itself with.
    ///
    /// Returns `None` in a release build: inspection there stays opt-in through
    /// [`Self::from_env`].
    ///
    /// The token is generated per process, so an endpoint is never open to
    /// whoever happens to reach the port. Where the endpoint listens depends on
    /// who needs to reach it: a desktop application is inspected from the same
    /// machine and binds loopback, while a phone or a simulator is inspected
    /// from the developer's computer and has to be reachable from it.
    #[must_use]
    pub fn for_debug_build() -> Option<Self> {
        if !cfg!(debug_assertions) {
            return None;
        }
        Some(Self {
            host: debug_host(),
            port: DEFAULT_PORT,
            token: generate_token(),
            task_window: Duration::from_millis(DEFAULT_TASK_WINDOW_MS),
            stall_ratio: DEFAULT_STALL_RATIO,
            app_name: std::env::var("WATERUI_INSPECTOR_APP_NAME")
                .unwrap_or_else(|_| String::from("WaterUI application")),
        })
    }
}

/// Where a debug build listens.
///
/// A mobile target is inspected from a computer, so it has to accept a
/// connection that did not come from itself; everything else is inspected from
/// the machine it runs on.
const fn debug_host() -> IpAddr {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        DEFAULT_HOST
    }
}

/// A token for one process, from the system's random source.
fn generate_token() -> String {
    use std::fmt::Write as _;
    use std::hash::{BuildHasher, Hasher, RandomState};

    let mut token = String::with_capacity(32);
    for _ in 0..2 {
        let value = RandomState::new().build_hasher().finish();
        write!(token, "{value:016x}").expect("writing to a String cannot fail");
    }
    token
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
    #[cfg(not(target_arch = "wasm32"))]
    advertisement: Advertisement,
    hub: Arc<EventHub>,
    task_probe: Arc<tasks::TaskProbe>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for InspectorRuntime {
    fn drop(&mut self) {
        // A file naming an endpoint that no longer exists sends the next reader
        // to a closed port.
        if let Err(error) = self.advertisement.withdraw() {
            tracing::warn!(
                target: "waterui::inspector",
                error = %error,
                "Could not withdraw the inspector advertisement"
            );
        }
    }
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

    /// Reveals `node` in an inspector, opening one if none is attached.
    ///
    /// This is what "inspect this element" does. When an inspector is already
    /// attached it simply jumps to the node; otherwise one is launched and the
    /// node waits for it, so the element the user pointed at is what they see
    /// when the window appears.
    ///
    /// On a phone or a simulator there is nothing to launch — the inspector
    /// runs on the developer's computer — so the endpoint is reported instead
    /// and the node is revealed whenever that inspector attaches.
    pub fn inspect_node(&self, node: waterui_inspector_protocol::NodeId) {
        self.hub.select_node(node);
        if self.hub.has_clients() {
            return;
        }
        self.launch_inspector();
    }

    /// Opens an inspector on this application, if one is not already attached.
    ///
    /// Reveals nothing in particular; [`Self::inspect_node`] is what an
    /// "inspect this element" gesture wants.
    pub fn open(&self) {
        if self.hub.has_clients() {
            return;
        }
        self.launch_inspector();
    }

    /// Starts an inspector pointed at this endpoint.
    fn launch_inspector(&self) {
        if cfg!(any(target_os = "android", target_os = "ios")) {
            tracing::info!(
                target: "waterui::inspector",
                inspector_addr = %self.endpoint.addr,
                token = %self.endpoint.token,
                "Inspect requested; attach from your computer with `water inspect`"
            );
            return;
        }

        // The CLI owns knowing how to build and run the inspector application
        // for this platform, so this asks for it by name rather than
        // reimplementing any of that here.
        let launched = std::process::Command::new("water")
            .arg("inspect")
            .arg("--target")
            .arg(self.endpoint.addr.to_string())
            .arg("--token")
            .arg(&self.endpoint.token)
            .spawn();

        match launched {
            Ok(_) => tracing::info!(
                target: "waterui::inspector",
                inspector_addr = %self.endpoint.addr,
                "Launching the inspector"
            ),
            Err(error) => tracing::warn!(
                target: "waterui::inspector",
                error = %error,
                inspector_addr = %self.endpoint.addr,
                token = %self.endpoint.token,
                "Could not launch `water inspect`; attach manually"
            ),
        }
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
        nami::observe::ObserverScope::install(Rc::new(signals::SignalBridge::new(Arc::clone(
            &self.hub,
        ))))
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
#[cfg(not(target_arch = "wasm32"))]
fn available_channels() -> ChannelSet {
    let mut available =
        ChannelSet::FRAMES | ChannelSet::TREE | ChannelSet::TASKS | ChannelSet::LOGS;
    if cfg!(feature = "inspector-signals") {
        available |= ChannelSet::SIGNALS;
    }
    available
}

/// Starts the endpoint from the environment, or automatically in a debug build.
///
/// A debug build listens without being asked, so that any `WaterUI` application
/// a developer runs can be inspected without deciding in advance that it should
/// be. A release build inspects only when the environment says so.
#[must_use]
pub fn maybe_init_from_env() -> Option<InspectorRuntime> {
    let config = match InspectorServerConfig::from_env() {
        Ok(Some(config)) => config,
        Ok(None) => InspectorServerConfig::for_debug_build()?,
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
    #[cfg(target_arch = "wasm32")]
    {
        let _ = config;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the TCP inspector endpoint is unavailable in browsers",
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
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

        // Publishing is what lets a tool attach without being told a port. A
        // failure here costs discovery, not inspection, so it is reported and
        // the endpoint keeps running.
        let advertisement = Advertisement {
            pid: target.pid,
            app_name: target.name.clone(),
            backend: target.backend.clone(),
            addr,
            token: config.token.clone(),
        };
        match advertisement.publish() {
            Ok(path) => tracing::info!(
                target: "waterui::inspector",
                path = %path.display(),
                "Inspector endpoint advertised"
            ),
            Err(error) => tracing::warn!(
                target: "waterui::inspector",
                error = %error,
                "Could not advertise the inspector endpoint; attach by address instead"
            ),
        }

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
            advertisement,
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
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> io::Result<()> {
    thread::Builder::new()
        .name(name.to_string())
        .spawn(body)
        .map(drop)
        .map_err(|error| io::Error::other(format!("failed to spawn {name} thread: {error}")))
}

/// Best available description of the rendering backend, for the handshake.
#[cfg(not(target_arch = "wasm32"))]
const fn backend_name() -> &'static str {
    if cfg!(target_vendor = "apple") {
        "apple"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "hydrolysis"
    }
}
