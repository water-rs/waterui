//! Device management and application running utilities for `WaterUI` CLI.

use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
    pin::Pin,
};

use color_eyre::eyre;
use smol::{
    channel::{Receiver, Sender, unbounded},
    stream::Stream,
};

#[cfg(target_os = "macos")]
use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

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
    /// Android does not support environment variables yet.
    /// `iOS`/`macOS` support environment variables via `export SIMCTL_CHILD_KEY=Val`.
    ///
    /// As a workaround, on Android we pass values as Activity intent extras using the
    /// `waterui.env.<KEY>` namespace, and the app reads them on startup and calls `Os.setenv()`.
    env_vars: HashMap<String, String>,

    /// If set, stream device logs at or above this level.
    log_level: Option<LogLevel>,

    /// If true, stream all native platform logs (`NSLog`, `print`, etc.), not just `WaterUI` logs.
    /// This filters by process ID instead of subsystem, which is noisier but includes all output.
    native_logs: bool,

    /// If true, terminate existing local macOS app instances for the same executable before
    /// launching a new one. Preview support apps must disable this so multiple pooled instances
    /// can coexist across runtime fingerprints.
    replace_existing_macos_app_instances: bool,
}

impl RunOptions {
    /// Create new run options
    #[must_use]
    pub fn new() -> Self {
        Self {
            env_vars: HashMap::new(),
            log_level: None,
            native_logs: false,
            replace_existing_macos_app_instances: true,
        }
    }

    /// Insert an environment variable to be set when running the application
    pub fn insert_env_var(&mut self, key: String, value: String) {
        self.env_vars.insert(key, value);
    }

    /// Tells the application which project it came from and what it is called.
    ///
    /// A launched application knows neither. It has no working directory worth
    /// the name — a macOS bundle gets `/` — so nothing it starts on the
    /// developer's behalf could find the project, and nothing inside `WaterUI`
    /// knows the name the project gave itself, which is why a window with no
    /// title of its own has to be told what to fall back to.
    pub fn describe_project(&mut self, project: &crate::project::Project) {
        self.insert_env_var(
            String::from("WATERUI_PROJECT_DIR"),
            project.root().display().to_string(),
        );
        let name = project.manifest().package.name.clone();
        self.insert_env_var(String::from("WATERUI_APP_NAME"), name);
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

    /// Set whether to stream all native platform logs.
    pub const fn set_native_logs(&mut self, native_logs: bool) {
        self.native_logs = native_logs;
    }

    /// Get whether native logs are enabled.
    #[must_use]
    pub const fn native_logs(&self) -> bool {
        self.native_logs
    }

    /// Set whether launching a local macOS `.app` should replace existing instances of the same
    /// executable.
    pub const fn set_replace_existing_macos_app_instances(&mut self, replace: bool) {
        self.replace_existing_macos_app_instances = replace;
    }

    /// Get whether launching a local macOS `.app` should replace existing instances.
    #[must_use]
    pub const fn replace_existing_macos_app_instances(&self) -> bool {
        self.replace_existing_macos_app_instances
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

    /// Detach the running instance, preventing the app from being killed on drop.
    ///
    /// This is useful for long-running apps like the preview support app that should
    /// stay running after the CLI command completes.
    pub fn detach(self: Pin<&mut Self>) {
        // SAFETY: `on_drop` is not structurally pinned and clearing the vector does not move the
        // pinned `receiver` field.
        unsafe { self.get_unchecked_mut() }.on_drop.clear();
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
        // SAFETY: `receiver` is reached through a pinned `&mut self`, so it is already
        // pinned and this only re-states that; it is never moved out.
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

    /// Application crashed.
    #[error("Application crashed: {0}")]
    Crashed(String),
}

/// A clean application exit observed by the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationExit {
    reason: ApplicationExitReason,
}

impl ApplicationExit {
    /// The application process finished with a successful process status.
    #[must_use]
    pub const fn completed() -> Self {
        Self {
            reason: ApplicationExitReason::Completed,
        }
    }

    /// A GUI application window or process closed without crash evidence.
    #[must_use]
    pub const fn user_closed() -> Self {
        Self {
            reason: ApplicationExitReason::UserClosed,
        }
    }

    /// Human-readable message for terminal status output.
    #[must_use]
    pub const fn terminal_message(self) -> &'static str {
        match self.reason {
            ApplicationExitReason::Completed => "Application exited",
            ApplicationExitReason::UserClosed => "Application closed",
        }
    }

    /// Return the classified clean-exit reason.
    #[must_use]
    pub const fn reason(self) -> ApplicationExitReason {
        self.reason
    }
}

/// Reason attached to a clean application exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationExitReason {
    /// The launched process returned a successful exit status.
    Completed,
    /// The GUI app was closed and no crash report or panic log was found.
    UserClosed,
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

    /// Clean exit of the application.
    Exited(ApplicationExit),

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

// =============================================================================
// macOS-specific crash detection and logging
// =============================================================================

#[cfg(target_os = "macos")]
use smol::{
    Timer,
    io::{AsyncBufReadExt, BufReader},
    process::{Command, Stdio},
    spawn,
    stream::StreamExt,
};

/// Panic information extracted from log stream.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct PanicInfo {
    /// The panic message payload
    pub payload: String,
    /// The source location where the panic occurred
    pub location: Option<String>,
}

#[cfg(target_os = "macos")]
struct MacosLogStream {
    task: smol::Task<()>,
    panic_rx: Receiver<String>,
}

/// Start streaming logs from a `WaterUI` app on macOS.
///
/// Uses `log stream` with a predicate to filter by the `WaterUI` subsystem (`dev.waterui`).
/// This captures all tracing output from the Rust code via `tracing_oslog`.
///
/// The returned task owns the `log stream` process so process monitoring can
/// stop it as soon as the application exits.
#[cfg(target_os = "macos")]
fn start_log_stream(
    sender: Sender<DeviceEvent>,
    log_level: Option<LogLevel>,
    pid: u32,
) -> Result<MacosLogStream, FailToRun> {
    // Bounded channel with capacity 1 acts as oneshot - only first panic is captured
    let (panic_tx, panic_rx) = smol::channel::bounded::<String>(1);

    // Always stream at default level to capture errors/faults, even if user didn't request logs
    let stream_level = log_level.map_or("default", |l| l.to_apple_level());

    let predicate = format!("processID == {pid} AND subsystem == \"dev.waterui\"");

    let mut log_cmd = smol::process::Command::new("log");
    log_cmd
        .arg("stream")
        .arg("--predicate")
        .arg(&predicate)
        .arg("--level")
        .arg(stream_level)
        .arg("--style")
        .arg("compact")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut log_child = log_cmd.spawn().map_err(|error| {
        FailToRun::Launch(eyre::eyre!("Failed to start macOS log stream: {error}"))
    })?;
    let stdout = log_child
        .stdout
        .take()
        .expect("stdout is piped for the macOS log stream");
    let task = spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(Ok(line)) = lines.next().await {
            if line.starts_with("Filtering") || line.starts_with("Timestamp") {
                continue;
            }

            if line.contains("panic.payload=")
                && let Some(info) = extract_panic_info_from_log(&line)
            {
                let _ = panic_tx.try_send(format_panic_message(
                    &info.payload,
                    info.location.as_deref(),
                ));
            }

            if log_level.is_some() {
                let level = if line.contains(" F ") || line.contains(" E ") {
                    tracing::Level::ERROR
                } else if line.contains(" W ") {
                    tracing::Level::WARN
                } else if line.contains(" D ") {
                    tracing::Level::DEBUG
                } else {
                    tracing::Level::INFO
                };

                if sender
                    .try_send(DeviceEvent::Log {
                        level,
                        message: line,
                    })
                    .is_err()
                {
                    break;
                }
            }
        }
        drop(log_child);
    });

    Ok(MacosLogStream { task, panic_rx })
}

/// Extract panic information from a log line containing panic.payload and panic.location fields.
#[cfg(target_os = "macos")]
fn extract_panic_info_from_log(line: &str) -> Option<PanicInfo> {
    let mut payload = None;
    let mut location = None;

    // Extract panic.payload="..."
    if let Some(start) = line.find("panic.payload=\"") {
        let start = start + 15;
        if let Some(end) = line[start..].find('"') {
            payload = Some(line[start..start + end].to_string());
        }
    }

    // Extract panic.location="..."
    if let Some(start) = line.find("panic.location=\"") {
        let start = start + 16;
        if let Some(end) = line[start..].find('"') {
            location = Some(line[start..start + end].to_string());
        }
    }

    payload.map(|p| PanicInfo {
        payload: p,
        location,
    })
}

/// Fetch recent panic logs from macOS unified logging system.
///
/// Uses `log show` to retrieve logs that contain panic info.
/// Returns the panic message if found, along with location and payload.
#[cfg(target_os = "macos")]
async fn fetch_recent_panic_logs(started_at: Instant, pid: Option<u32>) -> Option<String> {
    let last = started_at.elapsed() + Duration::from_secs(2);
    let last_arg = format!("{}s", last.as_secs().max(5));

    let predicate = pid.map_or_else(
        || "subsystem == \"dev.waterui\" AND eventMessage CONTAINS \"panic\"".to_string(),
        |pid| {
            format!(
                "processID == {pid} AND subsystem == \"dev.waterui\" AND eventMessage CONTAINS \"panic\""
            )
        },
    );

    let output = Command::new("log")
        .args(["show", "--predicate", &predicate, "--style", "compact"])
        .args(["--last", &last_arg])
        .output()
        .await
        .ok()?;

    let stdout = String::from_utf8(output.stdout).ok()?;

    for line in stdout.lines() {
        if line.starts_with("Filtering") || line.starts_with("Timestamp") || line.is_empty() {
            continue;
        }

        let mut location = None;
        let mut payload = None;

        if let Some(loc_start) = line.find("panic.location=\"") {
            let start = loc_start + 16;
            if let Some(end) = line[start..].find('"') {
                location = Some(&line[start..start + end]);
            }
        }

        if let Some(pay_start) = line.find("panic.payload=\"") {
            let start = pay_start + 15;
            if let Some(end) = line[start..].find('"') {
                payload = Some(&line[start..start + end]);
            }
        }

        if payload.is_some() || location.is_some() {
            let mut msg = String::from("Panic:");
            if let Some(p) = payload {
                msg = format!("{msg} {p}");
            }
            if let Some(l) = location {
                msg = format!("{msg}\n  at {l}");
            }
            return Some(msg);
        }
    }

    None
}

// =============================================================================
// Local Device
// =============================================================================

/// Local device representing the current machine.
///
/// This is a shared device that works with ANY backend:
/// - Apple backend: runs the executable inside a macOS `.app` bundle
/// - GTK4 backend: runs cargo binaries directly
///
/// The artifact type determines how it's executed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Local;

impl Device for Local {
    fn name(&self) -> &'static str {
        "Local Machine"
    }

    fn launch(&self) -> impl Future<Output = eyre::Result<()>> + Send {
        // No-op - local machine is always "launched"
        std::future::ready(Ok(()))
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        let artifact_path = artifact.path();

        // Dispatch based on artifact type
        match artifact_path.extension().and_then(|e| e.to_str()) {
            Some("app") => {
                // macOS .app bundle - supervise its real executable
                run_macos_app(artifact, options).await
            }
            _ => {
                // Binary executable - run directly
                run_binary(&artifact, &options)
            }
        }
    }

    fn scan() -> impl Future<Output = eyre::Result<Vec<Self>>> + Send {
        // Local machine is always available - just return a single instance
        std::future::ready(Ok(vec![Self]))
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacosProcess {
    pid: u32,
    command: String,
}

#[cfg(target_os = "macos")]
async fn list_macos_processes() -> Result<Vec<MacosProcess>, FailToRun> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .await
        .map_err(|e| FailToRun::Launch(eyre::eyre!("Failed to list local processes: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(FailToRun::Launch(eyre::eyre!(
            "Failed to list local processes with ps: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }

        let mut fields = trimmed.splitn(2, char::is_whitespace);
        let Some(pid_str) = fields.next() else {
            continue;
        };
        let Some(command) = fields.next() else {
            continue;
        };

        let pid = pid_str.parse::<u32>().map_err(|e| {
            FailToRun::Launch(eyre::eyre!(
                "Failed to parse process id '{pid_str}' from ps output: {e}"
            ))
        })?;
        processes.push(MacosProcess {
            pid,
            command: command.trim_start().to_string(),
        });
    }

    Ok(processes)
}

#[cfg(target_os = "macos")]
fn command_runs_executable(command: &str, executable_path: &Path) -> bool {
    let executable = executable_path.to_string_lossy();
    command == executable || command.starts_with(&format!("{executable} "))
}

#[cfg(target_os = "macos")]
async fn read_macos_bundle_identifier(app_path: &Path) -> Result<String, FailToRun> {
    let plist_path = app_path.join("Contents").join("Info.plist");
    smol::unblock({
        let plist_path = plist_path.clone();
        move || -> eyre::Result<String> {
            let plist = plist::Value::from_file(&plist_path).map_err(|error| {
                eyre::eyre!(
                    "Failed to read bundle Info.plist at '{}': {error}",
                    plist_path.display()
                )
            })?;
            let dictionary = plist.into_dictionary().ok_or_else(|| {
                eyre::eyre!(
                    "Bundle Info.plist at '{}' must contain a dictionary root",
                    plist_path.display()
                )
            })?;
            dictionary
                .get("CFBundleIdentifier")
                .and_then(plist::Value::as_string)
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    eyre::eyre!(
                        "Bundle Info.plist at '{}' is missing CFBundleIdentifier",
                        plist_path.display()
                    )
                })
        }
    })
    .await
    .map_err(FailToRun::Launch)
}

#[cfg(target_os = "macos")]
fn command_app_bundle_path_for_executable(command: &str, executable_name: &str) -> Option<PathBuf> {
    const BUNDLE_SUFFIX: &str = ".app";
    const EXECUTABLE_MARKER: &str = ".app/Contents/MacOS/";

    let command = command.trim_start();
    if !command.starts_with('/') {
        return None;
    }
    let marker_start = command.find(EXECUTABLE_MARKER)?;
    let executable_start = marker_start + EXECUTABLE_MARKER.len();
    let executable_end = executable_start.checked_add(executable_name.len())?;
    if !command[executable_start..].starts_with(executable_name) {
        return None;
    }
    if command
        .as_bytes()
        .get(executable_end)
        .is_some_and(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
    {
        return None;
    }

    let app_end = marker_start + BUNDLE_SUFFIX.len();
    Some(PathBuf::from(&command[..app_end]))
}

#[cfg(target_os = "macos")]
async fn list_conflicting_macos_app_pids(
    launch: &MacosBundleLaunchContext,
) -> Result<Vec<u32>, FailToRun> {
    let executable_name = launch
        .executable_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            FailToRun::Launch(eyre::eyre!(
                "Failed to determine executable name for '{}'",
                launch.executable_path.display()
            ))
        })?;

    let mut pids = BTreeSet::new();
    for process in list_macos_processes().await? {
        if command_runs_executable(&process.command, &launch.executable_path) {
            pids.insert(process.pid);
            continue;
        }

        let Some(app_path) =
            command_app_bundle_path_for_executable(&process.command, executable_name)
        else {
            continue;
        };
        // A running process whose bundle can no longer be identified (its
        // build directory was deleted after launch) cannot be an instance of
        // the bundle being launched; it must not fail this launch.
        match read_macos_bundle_identifier(&app_path).await {
            Ok(bundle_id) if bundle_id == launch.bundle_id => {
                pids.insert(process.pid);
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    pid = process.pid,
                    path = %app_path.display(),
                    "Skipping running app with unreadable bundle: {error:?}"
                );
            }
        }
    }

    Ok(pids.into_iter().collect())
}

#[cfg(target_os = "macos")]
fn quiet_kill_command(signal: &str, pid: &str) -> Command {
    let mut command = Command::new("kill");
    command
        .arg(signal)
        .arg(pid)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(target_os = "macos")]
async fn is_pid_alive(pid: u32) -> bool {
    let pid = pid.to_string();
    quiet_kill_command("-0", &pid)
        .status()
        .await
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "macos")]
async fn terminate_pids(pids: &[u32]) -> Result<(), FailToRun> {
    if pids.is_empty() {
        return Ok(());
    }

    for &pid in pids {
        let pid = pid.to_string();
        let status = quiet_kill_command("-TERM", &pid)
            .status()
            .await
            .map_err(|e| {
                FailToRun::Launch(eyre::eyre!(
                    "Failed to terminate existing app process {pid}: {e}"
                ))
            })?;
        if !status.success() {
            return Err(FailToRun::Launch(eyre::eyre!(
                "Failed to terminate existing app process {pid} before relaunch"
            )));
        }
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let mut alive = false;
        for &pid in pids {
            if is_pid_alive(pid).await {
                alive = true;
                break;
            }
        }
        if !alive {
            return Ok(());
        }
        Timer::after(Duration::from_millis(80)).await;
    }

    Err(FailToRun::Launch(eyre::eyre!(
        "Timed out waiting for previous app instance(s) to terminate before relaunch"
    )))
}

#[cfg(target_os = "macos")]
pub(crate) async fn resolve_macos_bundle_executable_path(
    artifact_path: &Path,
) -> Result<PathBuf, FailToRun> {
    let plist_path = artifact_path.join("Contents").join("Info.plist");
    let executable_name = smol::unblock({
        let plist_path = plist_path.clone();
        move || -> eyre::Result<String> {
            let plist = plist::Value::from_file(&plist_path).map_err(|error| {
                eyre::eyre!(
                    "Failed to read bundle Info.plist at '{}': {error}",
                    plist_path.display()
                )
            })?;
            let dictionary = plist.into_dictionary().ok_or_else(|| {
                eyre::eyre!(
                    "Bundle Info.plist at '{}' must contain a dictionary root",
                    plist_path.display()
                )
            })?;
            let executable = dictionary
                .get("CFBundleExecutable")
                .and_then(plist::Value::as_string)
                .ok_or_else(|| {
                    eyre::eyre!(
                        "Bundle Info.plist at '{}' is missing CFBundleExecutable",
                        plist_path.display()
                    )
                })?;
            Ok(executable.to_string())
        }
    })
    .await
    .map_err(FailToRun::Launch)?;

    Ok(artifact_path
        .join("Contents")
        .join("MacOS")
        .join(executable_name))
}

#[cfg(target_os = "macos")]
struct MacosBundleLaunchContext {
    bundle_id: String,
    artifact_path: PathBuf,
    executable_path: PathBuf,
}

pub(crate) fn format_panic_message(payload: &str, location: Option<&str>) -> String {
    let mut msg = format!("Panic: {payload}");
    if let Some(location) = location {
        msg.push('\n');
        msg.push_str("  at ");
        msg.push_str(location);
    }
    msg
}

#[cfg(target_os = "macos")]
async fn prepare_macos_bundle_launch(
    artifact: Artifact,
) -> Result<MacosBundleLaunchContext, FailToRun> {
    let artifact_path = artifact.path().to_path_buf();
    let executable_path = resolve_macos_bundle_executable_path(&artifact_path).await?;

    Ok(MacosBundleLaunchContext {
        bundle_id: artifact.bundle_id().to_string(),
        artifact_path,
        executable_path,
    })
}

/// A backstop against a wedged `LaunchServices` only, never a judgement about
/// how fast a launch "should" be: readiness is the app's process appearing,
/// failure is `open` exiting, and this bound is sized so it can never lose a
/// race against a slow-but-healthy launch (Gatekeeper's first-run scan of a
/// freshly built binary alone can take well past five seconds).
#[cfg(target_os = "macos")]
const MACOS_LAUNCH_BACKSTOP: Duration = Duration::from_secs(120);

#[cfg(target_os = "macos")]
async fn launch_macos_bundle_process(
    launch: &MacosBundleLaunchContext,
    options: &RunOptions,
) -> Result<(smol::process::Child, u32), FailToRun> {
    use tracing::info;

    if options.replace_existing_macos_app_instances() {
        let existing_pids = list_conflicting_macos_app_pids(launch).await?;
        terminate_pids(&existing_pids).await?;
    }

    let existing_pids = list_conflicting_macos_app_pids(launch)
        .await?
        .into_iter()
        .collect::<BTreeSet<_>>();
    info!("Launching app on macOS: {}", launch.artifact_path.display());
    let mut command = Command::new("open");
    command.arg("-W").arg("-n");
    for (key, value) in options.env_vars() {
        command.arg("--env").arg(format!("{key}={value}"));
    }
    command
        .arg(&launch.artifact_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        FailToRun::Launch(eyre::eyre!(
            "Failed to launch macOS app bundle '{}': {error}",
            launch.artifact_path.display()
        ))
    })?;

    // Readiness is decided by real signals, not a stopwatch: the app's process
    // appearing means the launch succeeded, and `open` exiting before that
    // means it failed — its status and stderr say why. A fixed five-second
    // deadline used to stand in for both, and it killed launches that were
    // about to work; see [`MACOS_LAUNCH_BACKSTOP`].
    let deadline = Instant::now() + MACOS_LAUNCH_BACKSTOP;
    while Instant::now() < deadline {
        let new_pid = list_conflicting_macos_app_pids(launch)
            .await?
            .into_iter()
            .find(|pid| !existing_pids.contains(pid));
        if let Some(app_pid) = new_pid {
            return Ok((child, app_pid));
        }

        // `open -W` outlives the app, so any exit before the process appeared
        // is a launch that did not happen — report LaunchServices' own words
        // instead of a timeout.
        match child.try_status() {
            Ok(Some(status)) => {
                let mut stderr_text = String::new();
                if let Some(stderr) = child.stderr.as_mut() {
                    use smol::io::AsyncReadExt as _;
                    let _ = stderr.read_to_string(&mut stderr_text).await;
                }
                let stderr_text = stderr_text.trim();
                return Err(FailToRun::Launch(eyre::eyre!(
                    "LaunchServices failed to start '{}': `open` exited with {status}{}{}",
                    launch.artifact_path.display(),
                    if stderr_text.is_empty() { "" } else { ": " },
                    stderr_text,
                )));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(FailToRun::Launch(eyre::eyre!(
                    "Failed to supervise the `open` process for '{}': {error}",
                    launch.artifact_path.display()
                )));
            }
        }

        Timer::after(Duration::from_millis(80)).await;
    }

    let _ = child.kill();
    let _ = child.status().await;
    Err(FailToRun::Launch(eyre::eyre!(
        "LaunchServices neither started '{}' nor failed within {MACOS_LAUNCH_BACKSTOP:?}; \
         `open` is still running with no matching app process",
        launch.artifact_path.display()
    )))
}

/// Run a macOS `.app` bundle through `LaunchServices`.
///
/// `open -W -n` gives the CLI a supervised proxy while launching through the
/// bundle preserves the process identity required by macOS privacy, lifecycle,
/// and application services. App logs are captured from unified logging by PID.
#[cfg(target_os = "macos")]
async fn run_macos_app(artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
    let launch = prepare_macos_bundle_launch(artifact).await?;
    let started_at = Instant::now();
    let (child, app_pid) = launch_macos_bundle_process(&launch, &options).await?;
    let (cancel_tx, cancel_rx) = smol::channel::bounded(1);
    let (running, sender) = Running::new(move || {
        let pid = nix::unistd::Pid::from_raw(
            i32::try_from(app_pid).expect("macOS process identifiers fit in i32"),
        );
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
        let _ = cancel_tx.try_send(());
    });
    let log_stream = start_log_stream(sender.clone(), options.log_level(), app_pid)?;
    let monitor = ChildMonitor::new(child, sender.clone(), cancel_rx);
    spawn_macos_app_exit_monitor(monitor, log_stream, sender, started_at, app_pid);

    Ok(running)
}

/// Run a macOS .app bundle on non-macOS platforms (not supported).
#[cfg(not(target_os = "macos"))]
fn run_macos_app(
    _artifact: Artifact,
    _options: RunOptions,
) -> impl std::future::Future<Output = Result<Running, FailToRun>> {
    std::future::ready(Err(FailToRun::InvalidArtifact)) // .app bundles only work on macOS
}

/// Run a binary executable directly.
///
/// Captures stdout/stderr and extracts panic messages from stderr.
fn run_binary(artifact: &Artifact, options: &RunOptions) -> Result<Running, FailToRun> {
    let binary_path = artifact.path();
    if !binary_path.exists() {
        return Err(FailToRun::InvalidArtifact);
    }

    let child = spawn_local_child(binary_path, options)?;
    let (cancel_tx, cancel_rx) = smol::channel::bounded(1);
    let (running, sender) = Running::new(move || {
        let _ = cancel_tx.try_send(());
    });
    let monitor = ChildMonitor::new(child, sender.clone(), cancel_rx);
    spawn_binary_exit_monitor(monitor, sender);

    Ok(running)
}

fn spawn_local_child(
    executable_path: &Path,
    options: &RunOptions,
) -> Result<smol::process::Child, FailToRun> {
    use smol::process::{Command, Stdio};

    let mut cmd = Command::new(executable_path);
    for (key, value) in options.env_vars() {
        cmd.env(key, value);
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    cmd.spawn().map_err(|error| {
        FailToRun::Launch(eyre::eyre!(
            "Failed to launch '{}': {error}",
            executable_path.display()
        ))
    })
}

fn spawn_stdout_forwarder(
    stdout: smol::process::ChildStdout,
    sender: Sender<DeviceEvent>,
) -> smol::Task<()> {
    use smol::io::{AsyncBufReadExt, BufReader};
    use smol::spawn;
    use smol::stream::StreamExt;

    spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Some(result) = lines.next().await {
            let Ok(line) = result else { break };
            if sender
                .try_send(DeviceEvent::Log {
                    level: parse_log_level(&line),
                    message: line,
                })
                .is_err()
            {
                break;
            }
        }
    })
}

fn spawn_stderr_forwarder(
    stderr: smol::process::ChildStderr,
    sender: Sender<DeviceEvent>,
    panic_tx: Sender<String>,
) -> smol::Task<()> {
    use smol::io::{AsyncBufReadExt, BufReader};
    use smol::spawn;
    use smol::stream::StreamExt;

    spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        let mut panic_lines = Vec::new();
        let mut capturing_panic = false;

        while let Some(result) = lines.next().await {
            let Ok(line) = result else { break };

            if starts_panic_capture(&line) {
                capturing_panic = true;
                panic_lines.clear();
            }

            if capturing_panic {
                panic_lines.push(line.clone());
                if should_flush_panic_capture(&panic_lines, &line) {
                    capturing_panic = false;
                    try_send_panic_message(&panic_tx, &panic_lines);
                }
            }

            if sender
                .try_send(DeviceEvent::Stderr { message: line })
                .is_err()
            {
                break;
            }
        }

        if capturing_panic && !panic_lines.is_empty() {
            try_send_panic_message(&panic_tx, &panic_lines);
        }
    })
}

fn starts_panic_capture(line: &str) -> bool {
    line.contains("panicked at") || line.starts_with("thread '") && line.contains("panic")
}

fn should_flush_panic_capture(panic_lines: &[String], line: &str) -> bool {
    panic_lines.len() > 10 || panic_lines.len() > 2 && line.trim().is_empty()
}

fn try_send_panic_message(panic_tx: &Sender<String>, panic_lines: &[String]) {
    if let Some(message) = extract_panic_message(panic_lines) {
        let _ = panic_tx.try_send(message);
    }
}

struct ChildMonitor {
    child: smol::process::Child,
    stdout_task: Option<smol::Task<()>>,
    stderr_task: Option<smol::Task<()>>,
    panic_rx: Receiver<String>,
    cancel_rx: Receiver<()>,
}

impl ChildMonitor {
    fn new(
        mut child: smol::process::Child,
        sender: Sender<DeviceEvent>,
        cancel_rx: Receiver<()>,
    ) -> Self {
        let (panic_tx, panic_rx) = smol::channel::unbounded::<String>();
        let stdout_task = child
            .stdout
            .take()
            .map(|stdout| spawn_stdout_forwarder(stdout, sender.clone()));
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| spawn_stderr_forwarder(stderr, sender, panic_tx));

        Self {
            child,
            stdout_task,
            stderr_task,
            panic_rx,
            cancel_rx,
        }
    }

    async fn wait(mut self) -> Option<ChildExit> {
        let status = {
            let wait = self.child.status();
            let cancel = self.cancel_rx.recv();
            let wait = std::pin::pin!(wait);
            let cancel = std::pin::pin!(cancel);

            match futures::future::select(wait, cancel).await {
                futures::future::Either::Left((status, _)) => Some(status),
                futures::future::Either::Right(_) => None,
            }
        };

        if status.is_none() {
            let _ = self.child.kill();
            let _ = self.child.status().await;
        }

        if let Some(task) = self.stdout_task {
            task.await;
        }
        if let Some(task) = self.stderr_task {
            task.await;
        }

        status.map(|status| ChildExit {
            status,
            panic_message: latest_panic_message(&self.panic_rx),
        })
    }
}

struct ChildExit {
    status: std::io::Result<std::process::ExitStatus>,
    panic_message: Option<String>,
}

fn spawn_binary_exit_monitor(monitor: ChildMonitor, sender: Sender<DeviceEvent>) {
    use smol::spawn;

    spawn(async move {
        let Some(exit) = monitor.wait().await else {
            return;
        };
        emit_process_exit_event(
            &sender,
            exit.status,
            exit.panic_message,
            ApplicationExit::completed(),
        );
    })
    .detach();
}

#[cfg(target_os = "macos")]
fn spawn_macos_app_exit_monitor(
    monitor: ChildMonitor,
    log_stream: MacosLogStream,
    sender: Sender<DeviceEvent>,
    started_at: Instant,
    pid: u32,
) {
    use smol::spawn;

    spawn(async move {
        let Some(exit) = monitor.wait().await else {
            return;
        };

        let mut panic_message = exit
            .panic_message
            .or_else(|| latest_panic_message(&log_stream.panic_rx));
        drop(log_stream.task);

        if panic_message.is_none()
            && matches!(&exit.status, Ok(exit_status) if !exit_status.success())
        {
            panic_message = fetch_recent_panic_logs(started_at, Some(pid)).await;
        }

        emit_process_exit_event(
            &sender,
            exit.status,
            panic_message,
            ApplicationExit::user_closed(),
        );
    })
    .detach();
}

fn latest_panic_message(panic_rx: &Receiver<String>) -> Option<String> {
    let mut panic_message = None;
    while let Ok(message) = panic_rx.try_recv() {
        panic_message = Some(message);
    }
    panic_message
}

fn emit_process_exit_event(
    sender: &Sender<DeviceEvent>,
    status: std::io::Result<std::process::ExitStatus>,
    panic_message: Option<String>,
    successful_exit: ApplicationExit,
) {
    match status {
        Ok(exit_status) if exit_status.success() => {
            let _ = sender.try_send(DeviceEvent::Exited(successful_exit));
        }
        Ok(exit_status) => {
            let _ = sender.try_send(DeviceEvent::Crashed(process_crash_message(
                exit_status,
                panic_message,
            )));
        }
        Err(error) => {
            let _ = sender.try_send(DeviceEvent::Crashed(format!("Process error: {error}")));
        }
    }
}

fn process_crash_message(
    exit_status: std::process::ExitStatus,
    panic_message: Option<String>,
) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = exit_status.signal() {
            let signal_name = match signal {
                6 => "SIGABRT",
                11 => "SIGSEGV",
                _ => "",
            };

            let termination = if signal_name.is_empty() {
                format!("signal {signal}")
            } else {
                format!("signal {signal} ({signal_name})")
            };

            return panic_message.map_or_else(
                || {
                    if signal_name.is_empty() {
                        format!("Terminated by signal {signal}")
                    } else {
                        format!("Process crashed ({signal_name})")
                    }
                },
                |panic| panic_process_message(&panic, &termination),
            );
        }
    }

    let code = exit_status.code().unwrap_or(-1);
    panic_message.map_or_else(
        || format!("Exit code: {code}"),
        |panic| panic_process_message(&panic, &format!("exit code {code}")),
    )
}

fn panic_process_message(panic: &str, termination: &str) -> String {
    let panic = panic.strip_prefix("Panic:").map_or(panic, str::trim_start);
    format!("Panic: {panic}\n  process terminated with {termination}")
}

/// Extract panic message from captured stderr lines.
fn extract_panic_message(lines: &[String]) -> Option<String> {
    for line in lines {
        // Format: "thread 'main' panicked at 'message', file.rs:123:45"
        // Or: "thread 'main' panicked at file.rs:123:45:\nmessage"
        if let Some(idx) = line.find("panicked at") {
            let after = &line[idx + 11..].trim_start();

            // Try to extract message in quotes: panicked at 'message'
            if after.starts_with('\'')
                && let Some(end) = after[1..].find('\'')
            {
                let message = &after[1..=end];
                // Also try to get location
                let location = after[end + 2..].trim_start_matches(", ").trim();
                if location.is_empty() {
                    return Some(message.to_string());
                }
                return Some(format!("{message}\n  at {location}"));
            }

            // Try newer format: panicked at file.rs:123:45:
            // Message is on the next line
            if after.ends_with(':') {
                let location = after.trim_end_matches(':');
                // Find message in next lines
                for next_line in lines.iter().skip(1) {
                    let msg = next_line.trim();
                    if !msg.is_empty()
                        && !msg.starts_with("note:")
                        && !msg.starts_with("stack backtrace:")
                    {
                        return Some(format!("{msg}\n  at {location}"));
                    }
                }
                return Some(format!("panic at {location}"));
            }

            // Fallback: return everything after "panicked at"
            return Some(after.to_string());
        }
    }
    None
}

/// Parse log level from a line of output.
fn parse_log_level(line: &str) -> tracing::Level {
    let line_lower = line.to_lowercase();
    if line_lower.contains("error") || line_lower.contains("fatal") || line_lower.contains("panic")
    {
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

#[cfg(test)]
mod tests {
    use std::process::ExitStatus;

    use smol::channel::unbounded;

    use super::{
        ApplicationExit, ApplicationExitReason, DeviceEvent, emit_process_exit_event,
        parse_log_level,
    };
    #[cfg(target_os = "macos")]
    use super::{command_app_bundle_path_for_executable, command_runs_executable};

    #[cfg(unix)]
    fn successful_exit_status() -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(0)
    }

    #[cfg(windows)]
    fn successful_exit_status() -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(0)
    }

    #[cfg(unix)]
    fn failing_exit_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn failing_exit_status(code: u32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(code)
    }

    #[test]
    fn application_exit_messages_are_reason_specific() {
        assert_eq!(
            ApplicationExit::completed().reason(),
            ApplicationExitReason::Completed
        );
        assert_eq!(
            ApplicationExit::completed().terminal_message(),
            "Application exited"
        );
        assert_eq!(
            ApplicationExit::user_closed().reason(),
            ApplicationExitReason::UserClosed
        );
        assert_eq!(
            ApplicationExit::user_closed().terminal_message(),
            "Application closed"
        );
    }

    #[test]
    fn successful_binary_status_emits_completed_exit() {
        let (sender, receiver) = unbounded();
        emit_process_exit_event(
            &sender,
            Ok(successful_exit_status()),
            None,
            ApplicationExit::completed(),
        );

        let event = receiver
            .try_recv()
            .expect("successful status should emit an event");
        let DeviceEvent::Exited(exit) = event else {
            panic!("successful status should emit a clean exit");
        };
        assert_eq!(exit.reason(), ApplicationExitReason::Completed);
    }

    #[test]
    fn successful_gui_status_emits_user_closed_exit() {
        let (sender, receiver) = unbounded();
        emit_process_exit_event(
            &sender,
            Ok(successful_exit_status()),
            None,
            ApplicationExit::user_closed(),
        );

        let event = receiver
            .try_recv()
            .expect("successful status should emit an event");
        let DeviceEvent::Exited(exit) = event else {
            panic!("successful status should emit a clean exit");
        };
        assert_eq!(exit.reason(), ApplicationExitReason::UserClosed);
    }

    #[test]
    fn failing_binary_status_emits_crash_message() {
        let (sender, receiver) = unbounded();
        emit_process_exit_event(
            &sender,
            Ok(failing_exit_status(7)),
            Some("backend panic".to_string()),
            ApplicationExit::completed(),
        );

        let event = receiver
            .try_recv()
            .expect("failing status should emit an event");
        let DeviceEvent::Crashed(message) = event else {
            panic!("failing status should emit a crash event");
        };
        assert!(message.starts_with("Panic:"));
        assert!(message.contains("backend panic"));
        assert!(message.contains('7'));
    }

    #[test]
    fn parse_log_level_detects_panic_as_error() {
        assert_eq!(
            parse_log_level("thread panicked at app.rs"),
            tracing::Level::ERROR
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_command_extracts_app_path_with_spaces() {
        let app_path = command_app_bundle_path_for_executable(
            "/tmp/water build/My App.app/Contents/MacOS/my-app --flag",
            "my-app",
        )
        .expect("app path should be extracted");
        assert_eq!(
            app_path,
            std::path::PathBuf::from("/tmp/water build/My App.app")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_command_rejects_nonmatching_executable_prefix() {
        assert!(
            command_app_bundle_path_for_executable(
                "/tmp/My App.app/Contents/MacOS/my-app-helper",
                "my-app",
            )
            .is_none()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_command_matches_exact_executable_path() {
        let executable = std::path::Path::new("/tmp/My App.app/Contents/MacOS/my-app");
        assert!(command_runs_executable(
            "/tmp/My App.app/Contents/MacOS/my-app --flag",
            executable,
        ));
    }
}
