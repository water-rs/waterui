use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

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
    debug,
    device::{Artifact, Device, DeviceEvent, FailToRun, LogLevel, Running},
    utils::{command, run_command, run_command_output},
};

/// Start streaming logs from a `WaterUI` app.
///
/// This uses `log stream` with a predicate to filter by the `WaterUI` subsystem ("dev.waterui").
/// This captures all tracing output from the Rust code via `tracing_oslog`.
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

    if let Ok(mut log_child) = log_cmd.spawn() {
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
}

/// Fetch recent panic logs from the unified logging system.
///
/// This uses `log show` to retrieve logs from the last few seconds that contain panic info.
/// Returns the panic message if found, along with location and payload.
async fn fetch_recent_panic_logs(started_at: Instant, pid: Option<u32>) -> Option<String> {
    let last = started_at.elapsed() + Duration::from_secs(2);
    let last_arg = format!("{}s", last.as_secs().max(5));

    let predicate = pid.map_or_else(|| "subsystem == \"dev.waterui\" AND eventMessage CONTAINS \"panic\"".to_string(), |pid| format!(
            "processID == {pid} AND subsystem == \"dev.waterui\" AND eventMessage CONTAINS \"panic\""
        ));

    let output = Command::new("log")
        .args(["show", "--predicate", &predicate, "--style", "compact"])
        .args(["--last", &last_arg])
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

async fn poll_for_crash_report(
    device_name: &str,
    device_identifier: &str,
    bundle_id: &str,
    process_name: &str,
    pid: Option<u32>,
    since: OffsetDateTime,
    timeout: Duration,
) -> Option<debug::CrashReport> {
    let deadline = Instant::now() + timeout;
    loop {
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
            return Some(report);
        }

        if Instant::now() >= deadline {
            return None;
        }

        Timer::after(Duration::from_millis(250)).await;
    }
}

fn parse_simctl_launch_pid(stdout: &str) -> Option<u32> {
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some((_, pid_part)) = line.rsplit_once(':') {
            if let Ok(pid) = pid_part.trim().parse::<u32>() {
                return Some(pid);
            }
        }

        if let Ok(pid) = line.parse::<u32>() {
            return Some(pid);
        }
    }
    None
}

async fn is_pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

async fn wait_for_pid_exit(pid: u32) {
    while is_pid_alive(pid).await {
        Timer::after(Duration::from_millis(200)).await;
    }
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

    #[allow(clippy::too_many_lines)]
    async fn run(
        &self,
        artifact: Artifact,
        options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        let bundle_id = artifact.bundle_id().to_string();

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
        let start_time = OffsetDateTime::now_utc();
        let start_instant = Instant::now();
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
            let device_name = "macOS";
            let device_identifier =
                whoami::fallible::hostname().unwrap_or_else(|_| "unknown".into());

            // Race between: open command exiting vs crash report appearing
            let crash_check_sender = sender.clone();
            let crash_app_name = app_name_for_crash.clone();
            let bundle_id_for_crash = bundle_id.clone();
            let pid_for_crash = app_pid;

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
                    if let Some(report) = debug::find_macos_ips_crash_report_since(
                        device_name,
                        &device_identifier,
                        &bundle_id_for_crash,
                        &crash_app_name,
                        pid_for_crash,
                        start_time,
                    )
                    .await
                    {
                        return Some(report);
                    }
                }
            };

            // Race the two tasks
            let open_task = std::pin::pin!(open_task);
            let crash_poll_task = std::pin::pin!(crash_poll_task);
            let result = futures::future::select(open_task, crash_poll_task).await;

            match result {
                // open exited first - check for crash report or panic logs
                futures::future::Either::Left(_) => {
                    // Check for crash report first (macOS .ips files)
                    if let Some(report) = poll_for_crash_report(
                        device_name,
                        &device_identifier,
                        &bundle_id,
                        &app_name_for_crash,
                        app_pid,
                        start_time,
                        Duration::from_secs(5),
                    )
                    .await
                    {
                        let _ = sender.try_send(DeviceEvent::Crashed(report.to_string()));
                    } else if let Some(panic_msg) =
                        fetch_recent_panic_logs(start_instant, app_pid).await
                    {
                        // Check for Rust panic logs
                        let _ = sender.try_send(DeviceEvent::Crashed(panic_msg));
                    } else {
                        let _ = sender.try_send(DeviceEvent::Exited);
                    }
                }
                // Crash report appeared first - kill the app and report
                futures::future::Either::Right((Some(report), _open_future)) => {
                    // Kill the stuck app
                    let _ = std::process::Command::new("pkill")
                        .arg("-9") // Force kill
                        .arg("-x")
                        .arg(&app_name_for_kill)
                        .status();

                    let _ = crash_check_sender.try_send(DeviceEvent::Crashed(report.to_string()));
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

        let start_time = OffsetDateTime::now_utc();
        let start_instant = Instant::now();

        let bundle_id = artifact.bundle_id().to_string();
        let process_name = artifact
            .path()
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or(bundle_id.as_str())
            .to_string();

        let log_level = options.log_level();
        let env_vars: Vec<(String, String)> = options
            .env_vars()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let mut launch_args = vec![
            "simctl".to_string(),
            "launch".to_string(),
            "--terminate-running-process".to_string(),
        ];
        for (key, value) in &env_vars {
            launch_args.push("--env".to_string());
            launch_args.push(format!("{key}={value}"));
        }
        launch_args.push(self.udid.clone());
        launch_args.push(bundle_id.clone());

        let launch_output = run_command_output("xcrun", launch_args.iter().map(String::as_str))
            .await
            .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        if !launch_output.status.success() {
            return Err(FailToRun::Launch(eyre!(
                "Failed to launch app:\n{}\n{}",
                String::from_utf8_lossy(&launch_output.stdout).trim(),
                String::from_utf8_lossy(&launch_output.stderr).trim(),
            )));
        }

        let pid = parse_simctl_launch_pid(&String::from_utf8_lossy(&launch_output.stdout))
            .ok_or_else(|| {
                FailToRun::Launch(eyre!(
                    "Failed to parse PID from simctl launch output: {}",
                    String::from_utf8_lossy(&launch_output.stdout).trim()
                ))
            })?;

        // Create a Running instance - termination will use simctl terminate
        let udid = self.udid.clone();
        let bundle_id_for_termination = bundle_id.clone();
        let (running, sender) = Running::new(move || {
            // Terminate the app when Running is dropped
            let fut = run_command(
                "xcrun",
                ["simctl", "terminate", &udid, &bundle_id_for_termination],
            );
            if let Err(err) = block_on(fut) {
                tracing::error!("Failed to terminate app on simulator: {err}");
            }
        });

        // Start log streaming (uses WaterUI subsystem predicate)
        start_log_stream(sender.clone(), log_level);

        // Monitor the actual app process and classify crash vs normal exit.
        let device_name = self.name.clone();
        let device_identifier = self.udid.clone();
        let sender_for_exit = sender;
        spawn(async move {
            wait_for_pid_exit(pid).await;

            if let Some(report) = poll_for_crash_report(
                &device_name,
                &device_identifier,
                &bundle_id,
                &process_name,
                Some(pid),
                start_time,
                Duration::from_secs(8),
            )
            .await
            {
                let _ = sender_for_exit.try_send(DeviceEvent::Crashed(report.to_string()));
                return;
            }

            if let Some(panic_msg) = fetch_recent_panic_logs(start_instant, Some(pid)).await {
                let _ = sender_for_exit.try_send(DeviceEvent::Crashed(panic_msg));
                return;
            }

            let _ = sender_for_exit.try_send(DeviceEvent::Exited);
        })
        .detach();

        Ok(running)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_simctl_launch_pid;

    #[test]
    fn parses_simctl_launch_pid_from_bundle_prefix() {
        let stdout = "com.example.app: 12345\n";
        assert_eq!(parse_simctl_launch_pid(stdout), Some(12345));
    }

    #[test]
    fn parses_simctl_launch_pid_from_plain_pid() {
        let stdout = "12345\n";
        assert_eq!(parse_simctl_launch_pid(stdout), Some(12345));
    }

    #[test]
    fn returns_none_when_no_pid_present() {
        let stdout = "com.example.app: not-a-pid\n";
        assert_eq!(parse_simctl_launch_pid(stdout), None);
    }
}
