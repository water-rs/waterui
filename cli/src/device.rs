//! Device management and application running utilities for `WaterUI` CLI.

use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use color_eyre::eyre;
use smol::{
    channel::{Receiver, Sender, unbounded},
    stream::Stream,
};

#[cfg(target_os = "macos")]
use jiff::Timestamp;

#[cfg(target_os = "macos")]
use tracing::debug as trace_debug;

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

    /// Set whether to stream all native platform logs.
    pub const fn set_native_logs(&mut self, native_logs: bool) {
        self.native_logs = native_logs;
    }

    /// Get whether native logs are enabled.
    #[must_use]
    pub const fn native_logs(&self) -> bool {
        self.native_logs
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
    /// This is useful for long-running apps like the preview daemon that should
    /// stay running after the CLI command completes.
    pub fn detach(&mut self) {
        self.on_drop.clear();
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

// =============================================================================
// macOS-specific crash detection and logging
// =============================================================================

#[cfg(target_os = "macos")]
use crate::debug;

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

/// Start streaming logs from a `WaterUI` app on macOS.
///
/// Uses `log stream` with a predicate to filter by the `WaterUI` subsystem (`dev.waterui`).
/// This captures all tracing output from the Rust code via `tracing_oslog`.
///
/// Returns a receiver for panic info that fires if a panic is detected.
#[cfg(target_os = "macos")]
fn start_log_stream(
    sender: Sender<DeviceEvent>,
    log_level: Option<LogLevel>,
    pid: u32,
) -> Receiver<PanicInfo> {
    // Bounded channel with capacity 1 acts as oneshot - only first panic is captured
    let (panic_tx, panic_rx) = smol::channel::bounded::<PanicInfo>(1);

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

    if let Ok(mut log_child) = log_cmd.spawn()
        && let Some(stdout) = log_child.stdout.take()
    {
        spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(Ok(line)) = lines.next().await {
                if line.starts_with("Filtering") || line.starts_with("Timestamp") {
                    continue;
                }

                // Extract panic info from log line if present (only first panic via try_send)
                if line.contains("panic.payload=")
                    && let Some(info) = extract_panic_info_from_log(&line)
                {
                    let _ = panic_tx.try_send(info);
                }

                // Only send log events to display if user requested logs
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
        })
        .detach();
    }

    panic_rx
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

/// Poll for crash reports on macOS.
#[cfg(target_os = "macos")]
async fn poll_for_crash_report(
    device_name: &str,
    device_identifier: &str,
    bundle_id: &str,
    process_name: &str,
    pid: Option<u32>,
    since: Timestamp,
    timeout: Duration,
) -> Option<debug::CrashReport> {
    trace_debug!(
        "Polling for crash report: bundle_id={}, process_name={}, pid={:?}, timeout={:?}",
        bundle_id,
        process_name,
        pid,
        timeout
    );

    let deadline = Instant::now() + timeout;
    let mut poll_count = 0;
    loop {
        poll_count += 1;
        if let Some(report) = debug::find_macos_ips_crash_report_since(
            device_name,
            device_identifier,
            bundle_id,
            process_name,
            pid,
            since,
        )
        .await
        {
            trace_debug!(
                "Found crash report after {} polls: {}",
                poll_count,
                report.summary()
            );
            return Some(report);
        }

        if Instant::now() >= deadline {
            trace_debug!(
                "No crash report found after {} polls within {:?}",
                poll_count,
                timeout
            );
            return None;
        }

        Timer::after(Duration::from_millis(250)).await;
    }
}

// =============================================================================
// Local Device
// =============================================================================

/// Local device representing the current machine.
///
/// This is a shared device that works with ANY backend:
/// - Apple backend: runs `.app` bundles via `open` command (with crash detection on macOS)
/// - GTK4 backend: runs cargo binaries directly
///
/// The artifact type determines how it's executed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Local;

impl Device for Local {
    fn name(&self) -> &'static str {
        "Local Machine"
    }

    async fn launch(&self) -> eyre::Result<()> {
        // No-op - local machine is always "launched"
        Ok(())
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        let artifact_path = artifact.path();

        // Dispatch based on artifact type
        match artifact_path.extension().and_then(|e| e.to_str()) {
            Some("app") => {
                // macOS .app bundle - use `open` command
                run_macos_app(artifact, options).await
            }
            _ => {
                // Binary executable - run directly
                run_binary(&artifact, &options)
            }
        }
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        // Local machine is always available - just return a single instance
        Ok(vec![Self])
    }
}

#[cfg(target_os = "macos")]
async fn list_matching_pids(executable_path: &std::path::Path) -> Result<Vec<u32>, FailToRun> {
    let executable = executable_path.to_string_lossy();
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
    let mut pids = Vec::new();
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
        let command = command.trim_start();
        if command == executable || command.starts_with(&format!("{executable} ")) {
            let pid = pid_str.parse::<u32>().map_err(|e| {
                FailToRun::Launch(eyre::eyre!(
                    "Failed to parse process id '{pid_str}' from ps output: {e}"
                ))
            })?;
            pids.push(pid);
        }
    }

    Ok(pids)
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
async fn detect_new_pid(
    executable_path: &std::path::Path,
    existing_pids: &[u32],
) -> Result<u32, FailToRun> {
    let existing: std::collections::HashSet<u32> = existing_pids.iter().copied().collect();
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let candidates = list_matching_pids(executable_path).await?;
        if let Some(pid) = candidates.into_iter().find(|pid| !existing.contains(pid)) {
            return Ok(pid);
        }

        if Instant::now() >= deadline {
            return Err(FailToRun::Launch(eyre::eyre!(
                "Failed to determine launched PID for app executable '{}'",
                executable_path.display()
            )));
        }

        Timer::after(Duration::from_millis(100)).await;
    }
}

#[cfg(target_os = "macos")]
async fn resolve_macos_bundle_executable_path(artifact_path: &Path) -> Result<PathBuf, FailToRun> {
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
    app_name: String,
    artifact_path: PathBuf,
    executable_path: PathBuf,
}

#[cfg(target_os = "macos")]
struct MacosRunningProcess {
    child: smol::process::Child,
    app_pid: u32,
    start_time: Timestamp,
    start_instant: Instant,
}

#[cfg(target_os = "macos")]
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
    let app_name = artifact_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or(FailToRun::InvalidArtifact)?
        .to_string();
    let executable_path = resolve_macos_bundle_executable_path(&artifact_path).await?;

    Ok(MacosBundleLaunchContext {
        bundle_id: artifact.bundle_id().to_string(),
        app_name,
        artifact_path,
        executable_path,
    })
}

#[cfg(target_os = "macos")]
fn build_macos_open_command(artifact_path: &Path, options: &RunOptions) -> smol::process::Command {
    let mut cmd = smol::process::Command::new("open");
    cmd.arg("-W").arg("-n").kill_on_drop(true);
    for (key, value) in options.env_vars() {
        cmd.arg("--env").arg(format!("{key}={value}"));
    }
    cmd.arg(artifact_path);
    cmd
}

#[cfg(target_os = "macos")]
async fn launch_macos_bundle_process(
    launch: &MacosBundleLaunchContext,
    options: &RunOptions,
) -> Result<MacosRunningProcess, FailToRun> {
    use tracing::info;

    let existing_pids = list_matching_pids(&launch.executable_path).await?;
    terminate_pids(&existing_pids).await?;

    info!("Launching app on macOS: {}", launch.artifact_path.display());

    let start_time = Timestamp::now();
    let start_instant = Instant::now();
    let child = build_macos_open_command(&launch.artifact_path, options)
        .spawn()
        .map_err(|error| FailToRun::Launch(eyre::eyre!("Failed to launch app: {error}")))?;

    Timer::after(Duration::from_millis(500)).await;
    let app_pid = detect_new_pid(&launch.executable_path, &existing_pids).await?;

    trace_debug!(
        "App launched: name={}, bundle_id={}, pid={:?}",
        launch.app_name,
        launch.bundle_id,
        app_pid
    );

    Ok(MacosRunningProcess {
        child,
        app_pid,
        start_time,
        start_instant,
    })
}

#[cfg(target_os = "macos")]
fn spawn_macos_bundle_monitor(
    mut child: smol::process::Child,
    sender: Sender<DeviceEvent>,
    panic_rx: Receiver<PanicInfo>,
    launch: MacosBundleLaunchContext,
    app_pid: u32,
    start_time: Timestamp,
    start_instant: Instant,
) -> Result<(), FailToRun> {
    use smol::spawn;

    let pid_for_crash_kill = app_pid.to_string();
    let pid_for_crash = Some(app_pid);
    let device_identifier = whoami::fallible::hostname().map_err(|error| {
        FailToRun::Launch(eyre::eyre!(
            "Failed to determine hostname for crash reports: {error}"
        ))
    })?;

    spawn(async move {
        let device_name = "macOS";
        let crash_check_sender = sender.clone();
        let bundle_id_for_crash = launch.bundle_id.clone();
        let app_name_for_crash = launch.app_name.clone();
        let device_identifier_for_poll = device_identifier.clone();

        let open_task = async {
            let _ = child.status().await;
            Timer::after(Duration::from_millis(500)).await;
        };

        let crash_poll_task = async {
            loop {
                Timer::after(Duration::from_millis(500)).await;
                if let Some(report) = debug::find_macos_ips_crash_report_since(
                    device_name,
                    &device_identifier_for_poll,
                    &bundle_id_for_crash,
                    &app_name_for_crash,
                    pid_for_crash,
                    start_time,
                )
                .await
                {
                    return Some(report);
                }
            }
        };

        let open_task = std::pin::pin!(open_task);
        let crash_poll_task = std::pin::pin!(crash_poll_task);
        let result = futures::future::select(open_task, crash_poll_task).await;
        match result {
            futures::future::Either::Left(_) => {
                handle_macos_open_exit(
                    sender,
                    panic_rx,
                    MacosExitContext {
                        device_identifier,
                        bundle_id: launch.bundle_id,
                        app_name: launch.app_name,
                        pid: pid_for_crash,
                        start_time,
                        start_instant,
                    },
                )
                .await;
            }
            futures::future::Either::Right((Some(report), _)) => {
                let _ = quiet_kill_command("-9", &pid_for_crash_kill).spawn();
                if let Ok(info) = panic_rx.try_recv() {
                    let _ = crash_check_sender.try_send(DeviceEvent::Crashed(
                        format_panic_message(&info.payload, info.location.as_deref()),
                    ));
                } else {
                    let _ = crash_check_sender.try_send(DeviceEvent::Crashed(report.to_string()));
                }
            }
            futures::future::Either::Right((None, _)) => {
                let _ = sender.try_send(DeviceEvent::Exited);
            }
        }
    })
    .detach();

    Ok(())
}

#[cfg(target_os = "macos")]
struct MacosExitContext {
    device_identifier: String,
    bundle_id: String,
    app_name: String,
    pid: Option<u32>,
    start_time: Timestamp,
    start_instant: Instant,
}

#[cfg(target_os = "macos")]
async fn handle_macos_open_exit(
    sender: Sender<DeviceEvent>,
    panic_rx: Receiver<PanicInfo>,
    context: MacosExitContext,
) {
    if let Ok(info) = panic_rx.try_recv() {
        let _ = sender.try_send(DeviceEvent::Crashed(format_panic_message(
            &info.payload,
            info.location.as_deref(),
        )));
        return;
    }

    if let Some(report) = poll_for_crash_report(
        "macOS",
        &context.device_identifier,
        &context.bundle_id,
        &context.app_name,
        context.pid,
        context.start_time,
        Duration::from_secs(10),
    )
    .await
    {
        let _ = sender.try_send(DeviceEvent::Crashed(report.to_string()));
        return;
    }

    if let Some(panic_msg) = fetch_recent_panic_logs(context.start_instant, context.pid).await {
        let _ = sender.try_send(DeviceEvent::Crashed(panic_msg));
        return;
    }

    let _ = sender.try_send(DeviceEvent::Exited);
}

/// Run a macOS .app bundle using the `open` command.
///
/// On macOS, this includes crash detection via .ips files and panic log fetching.
#[cfg(target_os = "macos")]
async fn run_macos_app(artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
    let launch = prepare_macos_bundle_launch(artifact).await?;
    let process = launch_macos_bundle_process(&launch, &options).await?;
    let pid_for_termination = process.app_pid.to_string();
    let (running, sender) = Running::new(move || {
        let _ = quiet_kill_command("-TERM", &pid_for_termination).spawn();
    });
    let panic_rx = start_log_stream(sender.clone(), options.log_level(), process.app_pid);
    spawn_macos_bundle_monitor(
        process.child,
        sender,
        panic_rx,
        launch,
        process.app_pid,
        process.start_time,
        process.start_instant,
    )?;

    Ok(running)
}

/// Run a macOS .app bundle on non-macOS platforms (not supported).
#[cfg(not(target_os = "macos"))]
async fn run_macos_app(_artifact: Artifact, _options: RunOptions) -> Result<Running, FailToRun> {
    Err(FailToRun::InvalidArtifact) // .app bundles only work on macOS
}

/// Run a binary executable directly.
///
/// Captures stdout/stderr and extracts panic messages from stderr.
fn run_binary(artifact: &Artifact, options: &RunOptions) -> Result<Running, FailToRun> {
    let binary_path = artifact.path();
    if !binary_path.exists() {
        return Err(FailToRun::InvalidArtifact);
    }

    let mut child = spawn_binary_child(binary_path, options)?;
    let (running, sender) = Running::new(move || {
        // Process will be killed on drop due to kill_on_drop(true).
    });
    let (panic_tx, panic_rx) = smol::channel::unbounded::<String>();
    let stdout_task = child
        .stdout
        .take()
        .map(|stdout| spawn_stdout_forwarder(stdout, sender.clone()));
    let stderr_task = child
        .stderr
        .take()
        .map(|stderr| spawn_stderr_forwarder(stderr, sender.clone(), panic_tx));
    spawn_binary_exit_monitor(child, stdout_task, stderr_task, panic_rx, sender);

    Ok(running)
}

fn spawn_binary_child(
    binary_path: &Path,
    options: &RunOptions,
) -> Result<smol::process::Child, FailToRun> {
    use smol::process::{Command, Stdio};

    let mut cmd = Command::new(binary_path);
    for (key, value) in options.env_vars() {
        cmd.env(key, value);
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    cmd.spawn()
        .map_err(|error| FailToRun::Launch(eyre::eyre!("Failed to launch binary: {error}")))
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

fn spawn_binary_exit_monitor(
    mut child: smol::process::Child,
    stdout_task: Option<smol::Task<()>>,
    stderr_task: Option<smol::Task<()>>,
    panic_rx: Receiver<String>,
    sender: Sender<DeviceEvent>,
) {
    use smol::spawn;

    spawn(async move {
        let status = child.status().await;

        if let Some(task) = stdout_task {
            task.await;
        }
        if let Some(task) = stderr_task {
            task.await;
        }

        let panic_message = latest_panic_message(&panic_rx);
        emit_binary_exit_event(&sender, status, panic_message);
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

fn emit_binary_exit_event(
    sender: &Sender<DeviceEvent>,
    status: std::io::Result<std::process::ExitStatus>,
    panic_message: Option<String>,
) {
    match status {
        Ok(exit_status) if exit_status.success() => {
            let _ = sender.try_send(DeviceEvent::Exited);
        }
        Ok(exit_status) => {
            let _ = sender.try_send(DeviceEvent::Crashed(binary_crash_message(
                exit_status,
                panic_message,
            )));
        }
        Err(error) => {
            let _ = sender.try_send(DeviceEvent::Crashed(format!("Process error: {error}")));
        }
    }
}

fn binary_crash_message(
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

            return panic_message.map_or_else(
                || {
                    if signal_name.is_empty() {
                        format!("Terminated by signal {signal}")
                    } else {
                        format!("Process crashed ({signal_name})")
                    }
                },
                |panic| {
                    if signal_name.is_empty() {
                        format!("Panic: {panic}")
                    } else {
                        format!("Panic ({signal_name}): {panic}")
                    }
                },
            );
        }
    }

    let code = exit_status.code().unwrap_or(-1);
    panic_message.map_or_else(
        || format!("Exit code: {code}"),
        |panic| format!("Panic (exit code {code}): {panic}"),
    )
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
