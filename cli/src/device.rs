//! Device management and application running utilities for `WaterUI` CLI.

use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
};

use color_eyre::eyre;
use smol::{
    channel::{Receiver, Sender, unbounded},
    stream::Stream,
};

/// Minimum log level for streaming device logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Only errors
    Error,
    /// Warnings and errors
    Warn,
    /// Info, warnings, and errors
    #[default]
    Info,
    /// Debug and above
    Debug,
    /// All logs including verbose
    Verbose,
}

impl LogLevel {
    /// Convert to Android logcat priority character.
    #[must_use]
    pub const fn to_android_priority(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warn => 'W',
            Self::Info => 'I',
            Self::Debug => 'D',
            Self::Verbose => 'V',
        }
    }

    /// Convert to iOS/macOS `log stream --level` argument.
    ///
    /// Apple's unified logging `log stream --level` accepts: default, info, debug
    /// - `debug` includes all messages (debug, info, default, error, fault)
    /// - `info` includes info and above
    /// - `default` includes default (notice) and above
    ///
    /// Since we want to capture errors/warnings, we need at least `default` level.
    #[must_use]
    pub const fn to_apple_level(self) -> &'static str {
        match self {
            Self::Error | Self::Warn | Self::Info => "default",
            Self::Debug | Self::Verbose => "debug",
        }
    }
}

/// Options for running an application on a device
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// # Note
    ///
    /// Android do not support environment variables yet.
    /// iOS/macOS support environment variables via `export SIMCTL_CHILD_KEY=Val`
    ///
    /// As a workaround, on Android we pass values as Activity intent extras using the
    /// `waterui.env.<KEY>` namespace, and the app reads them on startup and calls `Os.setenv()`.
    env_vars: HashMap<String, String>,

    /// If set, stream device logs at or above this level.
    log_level: Option<LogLevel>,
}

impl RunOptions {
    /// Create new run options
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an environment variable to be set when running the application
    pub fn insert_env_var(&mut self, key: String, value: String) {
        self.env_vars.insert(key, value);
    }

    /// Get an iterator over the environment variables
    pub fn env_vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Set the minimum log level to stream.
    pub const fn set_log_level(&mut self, level: LogLevel) {
        self.log_level = Some(level);
    }

    /// Get the log level if set.
    #[must_use]
    pub const fn log_level(&self) -> Option<LogLevel> {
        self.log_level
    }
}

/// Represents a build artifact to be run on a device
#[derive(Debug)]
pub struct Artifact {
    bundle_id: String,
    path: PathBuf,
}

impl Artifact {
    /// Create a new artifact
    #[must_use]
    pub fn new(bundle_id: impl Into<String>, path: PathBuf) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            path,
        }
    }

    /// Get the bundle identifier of the artifact
    #[must_use]
    pub const fn bundle_id(&self) -> &str {
        self.bundle_id.as_str()
    }

    /// Get the path to the artifact
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Trait representing a device (e.g., emulator, simulator, physical device)
///
/// Devices are decoupled from platforms - a device just knows how to execute artifacts.
/// The same device can be used with different backends (e.g., Local device works with
/// both Apple and GTK4 backends on macOS).
///
/// Each device type knows how to scan for available devices of its kind via the
/// associated `scan()` function.
pub trait Device: Sized + Send {
    /// Human-readable name for display purposes.
    fn name(&self) -> &str;

    /// Launch the device emulator or simulator.
    ///
    /// If the device is a physical device or local machine, this should do nothing.
    fn launch(&self) -> impl Future<Output = eyre::Result<()>> + Send;

    /// Run the given artifact on the device with the specified options.
    fn run(
        &self,
        artifact: Artifact,
        options: RunOptions,
    ) -> impl Future<Output = Result<Running, FailToRun>> + Send;

    /// Scan for available devices of this type.
    ///
    /// Each device type knows how to discover its own kind:
    /// - `Local::scan()` → always returns `vec![Local]`
    /// - `AppleSimulator::scan()` → uses `simctl list devices`
    /// - `AndroidDevice::scan()` → uses `adb devices`
    fn scan() -> impl Future<Output = eyre::Result<Vec<Self>>> + Send;
}

/// Represents a running application on a device.
///
/// Drop the `Running` to terminate the application
pub struct Running {
    sender: Sender<DeviceEvent>,
    receiver: Receiver<DeviceEvent>,
    on_drop: Vec<Box<dyn FnOnce() + Send>>,
}

impl Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running").finish_non_exhaustive()
    }
}

impl Running {
    /// Create a new `Running` instance
    #[allow(clippy::missing_panics_doc)]
    pub fn new(on_drop: impl FnOnce() + Send + 'static) -> (Self, Sender<DeviceEvent>) {
        let (sender, receiver) = unbounded();
        sender.try_send(DeviceEvent::Started).unwrap(); // `unwrap` is safe here, as we just created the channel
        (
            Self {
                sender: sender.clone(),
                receiver,
                on_drop: vec![Box::new(on_drop)],
            },
            sender,
        )
    }

    /// Retain a value for the lifetime of the `Running` instance.
    pub fn retain<T: Send + 'static>(&mut self, value: T) {
        self.on_drop.push(Box::new(move || {
            drop(value);
        }));
    }
}

impl Stream for Running {
    type Item = DeviceEvent;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // SAFETY: We only project to the `receiver` field, which is safe to pin
        // because we never move out of it and the other fields don't affect pinning
        let receiver = unsafe { &mut self.get_unchecked_mut().receiver };
        unsafe { std::pin::Pin::new_unchecked(receiver) }.poll_next(cx)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.sender.try_send(DeviceEvent::Stopped);
        for f in self.on_drop.drain(..) {
            f();
        }
    }
}

/// Errors that can occur when running an application on a device
#[derive(Debug, thiserror::Error)]
pub enum FailToRun {
    /// Invalid artifact provided.
    #[error("Invalid artifact")]
    InvalidArtifact,

    /// Failed to install the application on the device.
    #[error("Failed to install application on device: {0}")]
    Install(eyre::Report),

    /// Failed to launch the device.
    #[error("Failed to launch device: {0}")]
    Launch(eyre::Report),
    /// Failed to run the application on the device.
    #[error("Failed to run application on device: {0}")]
    Run(eyre::Report),

    /// Failed to package the artifacts.
    #[error("Failed to package the artifacts: {0}")]
    Package(eyre::Report),

    /// Failed to build the project.
    #[error("Failed to build the project: {0}")]
    Build(eyre::Report),

    /// Failed to start hot reload server.
    #[error("Failed to start hot reload server: {0}")]
    HotReload(crate::debug::hot_reload::FailToLaunch),

    /// Application crashed.
    #[error("Application crashed: {0}")]
    Crashed(String),
}

/// Events emitted by a running application on a device
#[derive(Debug)]
pub enum DeviceEvent {
    /// Application has started
    Started,
    /// Application has stopped by CLI
    Stopped,
    /// Standard output from the application
    Stdout {
        /// The output message
        message: String,
    },

    /// Standard error from the application
    Stderr {
        /// The error message
        message: String,
    },
    /// Standard log from the application
    Log {
        /// The log level
        level: tracing::Level,
        /// The log message
        message: String,
    },

    /// Unexpected exit of the application, may triggered by user quitting
    Exited,

    /// Application crashed with error message
    Crashed(String),
}

/// Represents the kind of device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// Simulator device
    Simulator,
    /// Physical device
    Physical,
}

/// Represents the state of a device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Device is booted and ready
    Booted,
    /// Device is shutdown
    Shutdown,
    /// Device is disconnected (e.g., physical device unplugged)
    Disconnected,
}

/// Local device representing the current machine.
///
/// This is a shared device that works with ANY backend:
/// - Apple backend: runs `.app` bundles via `open` command
/// - GTK4 backend: runs cargo binaries directly
///
/// The artifact type determines how it's executed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Local;

impl Device for Local {
    fn name(&self) -> &str {
        "Local Machine"
    }

    async fn launch(&self) -> eyre::Result<()> {
        // No-op - local machine is always "launched"
        Ok(())
    }

    async fn run(
        &self,
        artifact: Artifact,
        options: RunOptions,
    ) -> Result<Running, FailToRun> {
        let artifact_path = artifact.path();

        // Dispatch based on artifact type
        match artifact_path.extension().and_then(|e| e.to_str()) {
            Some("app") => {
                // macOS .app bundle - use `open` command
                run_macos_app(artifact, options).await
            }
            _ => {
                // Binary executable - run directly
                run_binary(artifact, options).await
            }
        }
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        // Local machine is always available - just return a single instance
        Ok(vec![Self])
    }
}

/// Run a macOS .app bundle using the `open` command.
async fn run_macos_app(artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
    use smol::process::{Command, Stdio};
    use smol::spawn;

    let artifact_path = artifact.path();

    // Build the `open` command
    let mut cmd = Command::new("open");
    cmd.arg("-W") // Wait for app to exit
        .arg("-n") // Open a new instance
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    // Add environment variables
    for (key, value) in options.env_vars() {
        cmd.arg("--env").arg(format!("{key}={value}"));
    }

    cmd.arg(artifact_path);

    // Spawn the open command
    let mut child = cmd
        .spawn()
        .map_err(|e| FailToRun::Launch(eyre::eyre!("Failed to launch app: {e}")))?;

    // Create Running instance
    let (running, sender) = Running::new(move || {
        // Process will be killed on drop
    });

    // Monitor the process for exit
    spawn(async move {
        let status = child.status().await;
        match status {
            Ok(exit_status) if exit_status.success() => {
                let _ = sender.try_send(DeviceEvent::Exited);
            }
            Ok(exit_status) => {
                let code = exit_status.code().unwrap_or(-1);
                let _ = sender.try_send(DeviceEvent::Crashed(format!("Exit code: {code}")));
            }
            Err(e) => {
                let _ = sender.try_send(DeviceEvent::Crashed(format!("Process error: {e}")));
            }
        }
    })
    .detach();

    Ok(running)
}

/// Run a binary executable directly.
async fn run_binary(artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
    use smol::io::{AsyncBufReadExt, BufReader};
    use smol::process::{Command, Stdio};
    use smol::spawn;
    use smol::stream::StreamExt;

    let binary_path = artifact.path();

    // Verify the binary exists
    if !binary_path.exists() {
        return Err(FailToRun::InvalidArtifact);
    }

    // Build the command to run the binary
    let mut cmd = Command::new(binary_path);

    // Set environment variables
    for (key, value) in options.env_vars() {
        cmd.env(key, value);
    }

    // Capture stdout/stderr for logging
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    // Spawn the process
    let mut child = cmd
        .spawn()
        .map_err(|e| FailToRun::Launch(eyre::eyre!("Failed to launch binary: {e}")))?;

    // Create Running instance
    let (running, sender) = Running::new(move || {
        // Process will be killed on drop due to kill_on_drop(true)
    });

    // Capture stdout
    if let Some(stdout) = child.stdout.take() {
        let stdout_sender = sender.clone();
        spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Some(result) = lines.next().await {
                let Ok(line) = result else { break };
                let level = parse_log_level(&line);
                if stdout_sender
                    .try_send(DeviceEvent::Log { level, message: line })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    // Capture stderr
    if let Some(stderr) = child.stderr.take() {
        let stderr_sender = sender.clone();
        spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Some(result) = lines.next().await {
                let Ok(line) = result else { break };
                if stderr_sender
                    .try_send(DeviceEvent::Stderr { message: line })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    // Monitor the process for exit
    let exit_sender = sender;
    spawn(async move {
        let status = child.status().await;
        match status {
            Ok(exit_status) if exit_status.success() => {
                let _ = exit_sender.try_send(DeviceEvent::Exited);
            }
            Ok(exit_status) => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(signal) = exit_status.signal() {
                        let crash_msg = match signal {
                            6 => "Process aborted (SIGABRT)".to_string(),
                            11 => "Segmentation fault (SIGSEGV)".to_string(),
                            _ => format!("Terminated by signal {signal}"),
                        };
                        let _ = exit_sender.try_send(DeviceEvent::Crashed(crash_msg));
                        return;
                    }
                }
                let code = exit_status.code().unwrap_or(-1);
                let _ = exit_sender.try_send(DeviceEvent::Crashed(format!("Exit code: {code}")));
            }
            Err(e) => {
                let _ = exit_sender.try_send(DeviceEvent::Crashed(format!("Process error: {e}")));
            }
        }
    })
    .detach();

    Ok(running)
}

/// Parse log level from a line of output.
fn parse_log_level(line: &str) -> tracing::Level {
    let line_lower = line.to_lowercase();
    if line_lower.contains("error") || line_lower.contains("fatal") || line_lower.contains("panic") {
        tracing::Level::ERROR
    } else if line_lower.contains("warn") {
        tracing::Level::WARN
    } else if line_lower.contains("debug") {
        tracing::Level::DEBUG
    } else if line_lower.contains("trace") {
        tracing::Level::TRACE
    } else {
        tracing::Level::INFO
    }
}
