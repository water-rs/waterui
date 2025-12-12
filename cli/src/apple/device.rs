use std::{collections::HashMap, path::PathBuf, process::ExitStatus, time::Duration};

use color_eyre::eyre::{self, eyre};
use serde::Deserialize;
use smol::{
    Timer,
    channel::Sender,
    future::block_on,
    io::{AsyncBufReadExt, BufReader},
    process::{Command, Stdio},
    spawn,
    stream::StreamExt,
};
use time::OffsetDateTime;
use tracing::info;

use crate::{
    apple::platform::ApplePlatform,
    device::{Artifact, Device, DeviceEvent, FailToRun, LogLevel, Running},
    utils::{command, run_command},
};

/// Start streaming logs from a WaterUI app.
///
/// This uses `log stream` with a predicate to filter by the WaterUI subsystem ("dev.waterui").
/// This captures all tracing output from the Rust code via tracing_oslog.
///
/// If `log_level` is `None`, no log streaming is started.
fn start_log_stream(sender: Sender<DeviceEvent>, log_level: Option<LogLevel>) {
    let Some(level) = log_level else {
        return;
    };

    let mut log_cmd = Command::new("log");
    log_cmd
        .arg("stream")
        .arg("--predicate")
        .arg("subsystem == \"dev.waterui\"")
        .arg("--level")
        .arg(level.to_apple_level())
        .arg("--style")
        .arg("compact")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    match log_cmd.spawn() {
        Ok(mut log_child) => {
            if let Some(stdout) = log_child.stdout.take() {
                // Move log_child into the async task to keep it alive
                spawn(async move {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Some(Ok(line)) = lines.next().await {
                        // Skip header lines from `log stream`
                        if line.starts_with("Filtering") || line.starts_with("Timestamp") {
                            continue;
                        }

                        // Parse log level from compact format: "timestamp Ty Process..."
                        // Ty is: F (fault), E (error), W (warning), I (info), D (debug)
                        // Fault is Apple's highest severity - used by panic handler
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
                    // Keep log_child alive until stream ends, then let it drop to kill the process
                    drop(log_child);
                })
                .detach();
            }
        }
        Err(_) => {}
    }
}

/// Fetch recent panic logs from the unified logging system.
///
/// This uses `log show` to retrieve logs from the last few seconds that contain panic info.
/// Returns the panic message if found, along with location and payload.
async fn fetch_recent_panic_logs(since: OffsetDateTime) -> Option<String> {
    // Format time for log show: "YYYY-MM-DD HH:MM:SS"
    let time_format =
        time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").ok()?;
    let start_time = since.format(&time_format).ok()?;

    let output = Command::new("log")
        .args([
            "show",
            "--predicate",
            "subsystem == \"dev.waterui\" AND eventMessage CONTAINS \"panic\"",
            "--style",
            "compact",
            "--start",
            &start_time,
        ])
        .output()
        .await
        .ok()?;

    let stdout = String::from_utf8(output.stdout).ok()?;

    // Parse the log output to extract panic information
    for line in stdout.lines() {
        // Skip header lines
        if line.starts_with("Filtering") || line.starts_with("Timestamp") || line.is_empty() {
            continue;
        }

        // Extract panic.payload and panic.location from structured log fields
        // Format: ... panic.location="path:line:col" ... panic.payload="message"
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
            let mut msg = String::from("Panic occurred");
            if let Some(p) = payload {
                msg = format!("{msg}: {p}");
            }
            if let Some(l) = location {
                msg = format!("{msg}\n  at {l}");
            }
            return Some(msg);
        }
    }

    None
}

/// Handle app exit: check for panics, then check exit status.
///
/// This is the main exit handler shared between macOS and iOS simulator.
/// It first checks for Rust panic logs, then falls back to signal/exit code analysis.
async fn handle_app_exit(
    exit_status: ExitStatus,
    start_time: OffsetDateTime,
    sender: &Sender<DeviceEvent>,
) {
    // First, check for Rust panic logs (structured tracing output)
    if let Some(panic_msg) = fetch_recent_panic_logs(start_time).await {
        let _ = sender.try_send(DeviceEvent::Crashed(panic_msg));
        return;
    }

    // Fall back to signal/exit code analysis
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = exit_status.signal() {
            // SIGINT(2), SIGKILL(9), and SIGTERM(15) are normal termination signals
            match signal {
                2 | 9 | 15 => {
                    let _ = sender.try_send(DeviceEvent::Exited);
                }
                6 => {
                    let _ = sender.try_send(DeviceEvent::Crashed(
                        "App terminated by SIGABRT".to_string(),
                    ));
                }
                10 => {
                    let _ = sender
                        .try_send(DeviceEvent::Crashed("App terminated by SIGBUS".to_string()));
                }
                11 => {
                    let _ = sender.try_send(DeviceEvent::Crashed(
                        "App terminated by SIGSEGV".to_string(),
                    ));
                }
                _ => {
                    let _ = sender.try_send(DeviceEvent::Crashed(format!(
                        "App terminated by signal {signal}"
                    )));
                }
            }
            return;
        }
    }

    if exit_status.success() {
        let _ = sender.try_send(DeviceEvent::Exited);
    } else {
        let _ = sender.try_send(DeviceEvent::Crashed(format!(
            "App exited with code {:?}",
            exit_status.code()
        )));
    }
}

/// Find a crash report for the given app name created since the specified time.
///
/// Searches `~/Library/Logs/DiagnosticReports/` for crash reports created after `since`.
async fn find_crash_report_since(app_name: &str, since: OffsetDateTime) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let crash_dir = PathBuf::from(home).join("Library/Logs/DiagnosticReports");

    if !crash_dir.exists() {
        return None;
    }

    // List crash reports for this app
    let pattern = format!("{app_name}*.ips");
    let output = Command::new("find")
        .args([
            crash_dir.to_str()?,
            "-name",
            &pattern,
            "-type",
            "f",
            "-mmin",
            "-2", // Modified within last 2 minutes (generous window)
        ])
        .output()
        .await
        .ok()?;

    let paths = String::from_utf8(output.stdout).ok()?;
    let paths: Vec<&str> = paths.lines().collect();

    // Find crash reports created after `since`
    let mut most_recent: Option<(PathBuf, OffsetDateTime)> = None;

    for path_str in paths {
        let path = PathBuf::from(path_str);
        if let Ok(metadata) = std::fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                let modified_time: OffsetDateTime = modified.into();
                // Only consider reports created after we started the app
                if modified_time > since {
                    if most_recent
                        .as_ref()
                        .map_or(true, |(_, t)| modified_time > *t)
                    {
                        most_recent = Some((path, modified_time));
                    }
                }
            }
        }
    }

    let (crash_path, _) = most_recent?;

    // Read and extract crash info
    extract_crash_summary(&crash_path).await
}

/// Extract a summary from a crash report file (.ips format)
async fn extract_crash_summary(path: &PathBuf) -> Option<String> {
    let content = smol::fs::read_to_string(path).await.ok()?;

    // .ips files have two JSON objects:
    // 1. First line: metadata (app_name, timestamp, etc.)
    // 2. Rest: detailed crash report with exception and termination info
    let mut lines = content.lines();
    let _header = lines.next()?;

    // Join the rest to form the detailed crash JSON (need newlines for valid JSON)
    let crash_json: String = lines.collect::<Vec<_>>().join("\n");

    // Parse the crash report JSON
    let crash: serde_json::Value = serde_json::from_str(&crash_json).ok()?;

    let mut parts = Vec::new();

    // Extract exception type and signal
    if let Some(exception) = crash.get("exception") {
        if let Some(exc_type) = exception.get("type").and_then(|v| v.as_str()) {
            parts.push(format!("Exception: {exc_type}"));
        }
        if let Some(signal) = exception.get("signal").and_then(|v| v.as_str()) {
            parts.push(format!("Signal: {signal}"));
        }
    }

    // Extract termination reason
    if let Some(termination) = crash.get("termination") {
        if let Some(indicator) = termination.get("indicator").and_then(|v| v.as_str()) {
            parts.push(format!("Reason: {indicator}"));
        }
    }

    // Build summary
    let summary = if parts.is_empty() {
        "App crashed".to_string()
    } else {
        parts.join(", ")
    };

    Some(format!("{summary}\n\nCrash report: {}", path.display()))
}

/// Represents a physical Apple device
#[derive(Debug)]
pub struct ApplePhysicalDevice {}

/// Represents an Apple device (simulator or physical)
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum AppleDevice {
    /// An Apple Simulator device
    Simulator(AppleSimulator),

    /// A physical Apple device
    Physical(ApplePhysicalDevice),

    /// The current physical `macOS` device
    ///
    /// Apple do not provide macOS simulator, so this represents the current physical machine
    Current(MacOS),
}

/// Local `MacOS` device (current physical machine)
#[derive(Debug)]
pub struct MacOS;

impl Device for MacOS {
    type Platform = ApplePlatform;
    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        // No need to launch anything for MacOS physical device
        // This is the current machine
        Ok(())
    }

    fn platform(&self) -> Self::Platform {
        ApplePlatform::macos()
    }

    async fn run(
        &self,
        artifact: Artifact,
        options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        // Artifact must end with `.app` for MacOS
        let artifact_path = artifact.path();

        if artifact_path.extension().and_then(|e| e.to_str()) != Some("app") {
            return Err(FailToRun::InvalidArtifact);
        }

        let app_name = artifact_path
            .file_stem()
            .and_then(|n| n.to_str())
            .ok_or(FailToRun::InvalidArtifact)?
            .to_string();

        info!("Launching app on MacOS: {}", artifact.path().display());

        // Build the `open` command
        let mut cmd = Command::new("open");
        command(&mut cmd);
        cmd.arg("-W") // Wait for app to exit
            .arg("-n"); // Open a new instance

        // Add environment variables
        for (key, value) in options.env_vars() {
            cmd.arg("--env").arg(format!("{key}={value}"));
        }

        cmd.arg(artifact_path);

        // Spawn the open command
        let mut child = cmd
            .spawn()
            .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        // Give the app a moment to start, then get its PID
        smol::Timer::after(std::time::Duration::from_millis(500)).await;

        // Get the PID of the launched app using pgrep
        let app_pid = Command::new("pgrep")
            .arg("-n") // Newest matching process
            .arg("-x") // Exact match
            .arg(&app_name)
            .output()
            .await
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u32>().ok());

        // Create Running instance - kill the app process on drop
        let pid_for_termination = app_pid;
        let app_name_for_termination = app_name.clone();
        let (running, sender) = Running::new(move || {
            // pkill the app by name to ensure it's terminated
            if pid_for_termination.is_some() {
                let _ = std::process::Command::new("pkill")
                    .arg("-x")
                    .arg(&app_name_for_termination)
                    .status();
            }
        });

        // Start log streaming (uses WaterUI subsystem predicate)
        start_log_stream(sender.clone(), options.log_level());

        // Spawn a task to wait for the app to exit and detect crashes.
        // We monitor two things in parallel:
        // 1. The `open -W` command completing (normal exit)
        // 2. A crash report appearing (app crashed but may be stuck)
        //
        // `open -W` exits with code 0 regardless of how the app terminated,
        // so we check for crash reports to detect crashes.
        let app_name_for_crash = app_name.clone();
        let app_name_for_kill = app_name;
        spawn(async move {
            // Record the start time to filter crash reports
            let start_time = OffsetDateTime::now_utc();

            // Race between: open command exiting vs crash report appearing
            let crash_check_sender = sender.clone();
            let crash_app_name = app_name_for_crash.clone();

            // Task 1: Wait for `open` command to exit
            let open_task = async {
                let _ = child.status().await;
                // Give crash reporter a moment to write
                Timer::after(Duration::from_millis(500)).await;
            };

            // Task 2: Poll for crash reports (check every 500ms)
            let crash_poll_task = async {
                loop {
                    Timer::after(Duration::from_millis(500)).await;
                    if let Some(crash_msg) =
                        find_crash_report_since(&crash_app_name, start_time).await
                    {
                        return Some(crash_msg);
                    }
                }
            };

            // Race the two tasks
            let open_task = std::pin::pin!(open_task);
            let crash_poll_task = std::pin::pin!(crash_poll_task);
            let result = futures::future::select(open_task, crash_poll_task).await;

            match result {
                // open exited first - check for crash report or panic logs
                futures::future::Either::Left(((), crash_future)) => {
                    // Check for crash report first (macOS .ips files)
                    if let Some(msg) =
                        find_crash_report_since(&app_name_for_crash, start_time).await
                    {
                        let _ = sender.try_send(DeviceEvent::Crashed(msg));
                    } else if let Some(panic_msg) = fetch_recent_panic_logs(start_time).await {
                        // Check for Rust panic logs
                        let _ = sender.try_send(DeviceEvent::Crashed(panic_msg));
                    } else {
                        let _ = sender.try_send(DeviceEvent::Exited);
                    }
                    drop(crash_future);
                }
                // Crash report appeared first - kill the app and report
                futures::future::Either::Right((Some(crash_msg), _open_future)) => {
                    // Kill the stuck app
                    let _ = std::process::Command::new("pkill")
                        .arg("-9") // Force kill
                        .arg("-x")
                        .arg(&app_name_for_kill)
                        .status();

                    let _ = crash_check_sender.try_send(DeviceEvent::Crashed(crash_msg));
                }
                futures::future::Either::Right((None, _)) => {
                    // Should never happen (crash_poll_task loops forever)
                    let _ = sender.try_send(DeviceEvent::Exited);
                }
            }
        })
        .detach();

        Ok(running)
    }
}

impl Device for AppleDevice {
    type Platform = ApplePlatform;
    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        match self {
            Self::Simulator(simulator) => simulator.launch().await,
            Self::Current(_) => {
                // No need to launch anything for MacOS physical device
                // This is the current machine
                Ok(())
            }
            Self::Physical(_) => {
                // Physical devices don't need to be "launched" - they're already running
                // Connection is handled during run()
                Ok(())
            }
        }
    }

    fn platform(&self) -> Self::Platform {
        match self {
            Self::Simulator(simulator) => simulator.platform(),
            Self::Current(mac_os) => mac_os.platform(),
            Self::Physical(_) => ApplePlatform::ios(), // Physical devices are iOS
        }
    }

    async fn run(
        &self,
        artifact: Artifact,
        options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        match self {
            Self::Simulator(simulator) => simulator.run(artifact, options).await,
            Self::Current(mac_os) => mac_os.run(artifact, options).await,
            Self::Physical(_) => {
                // Physical device deployment requires ios-deploy or similar tooling
                // For now, return an error indicating this is not yet implemented
                Err(FailToRun::Run(eyre!(
                    "Physical iOS device deployment is not yet implemented. \
                     Please use a simulator or deploy manually via Xcode."
                )))
            }
        }
    }
}

/// Represents an Apple Simulator device
///
/// Fields are deserialized from `xcrun simctl list devices --json` output
#[derive(Debug, Deserialize, Clone)]
pub struct AppleSimulator {
    /// Path to the simulator data directory
    #[serde(rename = "dataPath")]
    pub data_path: PathBuf,

    /// Size of the simulator data directory in bytes
    #[serde(rename = "dataPathSize")]
    pub data_path_size: Option<u64>,

    /// Path to the simulator log directory
    #[serde(rename = "logPath")]
    pub log_path: PathBuf,

    /// Size of the simulator log directory in bytes
    #[serde(rename = "logPathSize")]
    pub log_path_size: Option<u64>,

    /// Unique device identifier
    ///
    /// Note: not `uuid` but `udid`!
    pub udid: String,

    /// Indicates if the simulator is available
    #[serde(rename = "isAvailable")]
    pub is_available: bool,

    /// Device type identifier
    #[serde(rename = "deviceTypeIdentifier")]
    pub device_type_identifier: String,

    /// Current state of the simulator (e.g., Shutdown, Booted)
    pub state: String,
    /// Name of the simulator device
    pub name: String,

    /// Timestamp of the last boot time
    #[serde(rename = "lastBootedAt")]
    pub last_booted_at: Option<String>,
}

impl AppleSimulator {
    /// Scan for available simulators using `simctl`.
    ///
    /// # Errors
    ///
    /// Returns an error if `simctl` command fails or output cannot be parsed.
    pub async fn scan() -> eyre::Result<Vec<Self>> {
        #[derive(Deserialize)]
        struct Root {
            devices: HashMap<String, Vec<AppleSimulator>>,
        }

        let content = run_command("xcrun", ["simctl", "list", "devices", "--json"]).await?;

        let simulators = serde_json::from_str::<Root>(&content)?
            .devices
            .into_values()
            .flatten()
            .collect();

        Ok(simulators)
    }

    /// Get the PID of an app running in this simulator by bundle ID.
    async fn get_app_pid(&self, bundle_id: &str) -> Option<u32> {
        // Use simctl spawn to run pgrep inside the simulator
        let output = Command::new("xcrun")
            .args(["simctl", "spawn", &self.udid, "launchctl", "list"])
            .output()
            .await
            .ok()?;

        let stdout = String::from_utf8(output.stdout).ok()?;

        // Look for a line containing the bundle ID and extract the PID
        for line in stdout.lines() {
            if line.contains(bundle_id) {
                // Format: PID  Status  Label
                let pid_str = line.split_whitespace().next()?;
                if pid_str != "-" {
                    return pid_str.parse().ok();
                }
            }
        }
        None
    }
}

impl Device for AppleSimulator {
    type Platform = ApplePlatform;

    /// Launch the Apple simulator (boot it)
    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        // Only boot if not already booted
        if self.state != "Booted" {
            run_command("xcrun", ["simctl", "boot", &self.udid]).await?;
        }
        Ok(())
    }

    fn platform(&self) -> Self::Platform {
        // Parse device type identifier to determine platform
        ApplePlatform::from_device_type_identifier(&self.device_type_identifier)
    }

    /// Run an artifact on the Apple simulator
    ///
    /// Please launch the device before calling this method
    async fn run(
        &self,
        artifact: Artifact,
        options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        info!("Installing app on apple simulator {}", self.name);
        run_command(
            "xcrun",
            [
                "simctl",
                "install",
                &self.udid,
                artifact.path().to_str().unwrap(),
            ],
        )
        .await
        .map_err(|e| FailToRun::Install(eyre!("Failed to install app: {e}")))?;

        info!("Launching app on apple simulator {}", self.name);

        // Use --console to block until app exits and pass signals through
        let mut child = command(Command::new("xcrun").args([
            "simctl",
            "launch",
            "--console",
            "--terminate-running-process",
            &self.udid,
            artifact.bundle_id(),
        ]))
        .spawn()
        .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        // Give the app a moment to start, then get its PID for log streaming
        smol::Timer::after(std::time::Duration::from_millis(500)).await;
        let app_pid = self.get_app_pid(artifact.bundle_id()).await;

        // Create a Running instance - termination will use simctl terminate
        let udid = self.udid.clone();
        let bundle_id = artifact.bundle_id().to_string();
        let (running, sender) = Running::new(move || {
            // Terminate the app when Running is dropped
            let fut = run_command("xcrun", ["simctl", "terminate", &udid, &bundle_id]);
            if let Err(err) = block_on(fut) {
                tracing::error!("Failed to terminate app on simulator: {err}");
            }
        });

        // Record start time for panic log lookup
        let start_time = OffsetDateTime::now_utc();

        // Start log streaming (uses WaterUI subsystem predicate)
        start_log_stream(sender.clone(), options.log_level());

        // Spawn a task to wait for the app to exit and check for panics
        spawn(async move {
            match child.status().await {
                Ok(exit_status) => handle_app_exit(exit_status, start_time, &sender).await,
                Err(e) => {
                    let _ =
                        sender.try_send(DeviceEvent::Crashed(format!("Failed to get status: {e}")));
                }
            }
        })
        .detach();

        Ok(running)
    }
}
