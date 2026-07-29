use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use color_eyre::eyre::{self, eyre};
use jiff::Timestamp;
use serde::Deserialize;
use smol::{
    Timer,
    channel::Sender,
    io::{AsyncBufReadExt, BufReader},
    process::{Command, Stdio},
    spawn,
    stream::StreamExt,
};
use tracing::{debug as trace_debug, info, warn};

use std::path::Path;

use crate::{
    debug,
    device::{
        ApplicationExit, Artifact, Device, DeviceEvent, FailToRun, Local, LogLevel, Running,
        format_panic_message,
    },
    utils::run_command,
};

use smol::channel::Receiver;

/// Panic information extracted from log stream.
#[derive(Debug, Clone)]
struct PanicInfo {
    /// The panic message payload
    payload: String,
    /// The source location where the panic occurred
    location: Option<String>,
}

async fn install_simulator_artifact(udid: &str, artifact_path: &Path) -> Result<(), FailToRun> {
    let install_output = Command::new("xcrun")
        .args(["simctl", "install", udid])
        .arg(artifact_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| FailToRun::Install(eyre!("Failed to install app: {error}")))?;
    if install_output.status.success() {
        return Ok(());
    }

    Err(FailToRun::Install(eyre!(
        "Failed to install app:\n{}\n{}",
        String::from_utf8_lossy(&install_output.stdout).trim(),
        String::from_utf8_lossy(&install_output.stderr).trim(),
    )))
}

fn simulator_process_name(artifact: &Artifact) -> Result<String, FailToRun> {
    artifact
        .path()
        .file_stem()
        .ok_or_else(|| {
            FailToRun::Run(eyre!(
                "Artifact path has no filename: {}",
                artifact.path().display()
            ))
        })?
        .to_str()
        .ok_or_else(|| {
            FailToRun::Run(eyre!(
                "Artifact filename is not valid UTF-8: {}",
                artifact.path().display()
            ))
        })
        .map(std::string::ToString::to_string)
}

fn simulator_env_vars(options: &crate::device::RunOptions) -> Vec<(String, String)> {
    options
        .env_vars()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

async fn launch_simulator_app(
    udid: &str,
    bundle_id: &str,
    env_vars: &[(String, String)],
) -> Result<u32, FailToRun> {
    let mut launch = Command::new("xcrun");
    launch
        .arg("simctl")
        .arg("launch")
        .arg("--terminate-running-process")
        .arg(udid)
        .arg(bundle_id);

    for (key, value) in env_vars {
        launch.env(format!("SIMCTL_CHILD_{key}"), value);
    }

    let launch_output = launch
        .output()
        .await
        .map_err(|error| FailToRun::Launch(eyre!("Failed to launch app: {error}")))?;
    if !launch_output.status.success() {
        return Err(FailToRun::Launch(eyre!(
            "Failed to launch app:\n{}\n{}",
            String::from_utf8_lossy(&launch_output.stdout).trim(),
            String::from_utf8_lossy(&launch_output.stderr).trim(),
        )));
    }

    parse_simctl_launch_pid(&String::from_utf8_lossy(&launch_output.stdout)).ok_or_else(|| {
        FailToRun::Launch(eyre!(
            "Failed to parse PID from simctl launch output: {}",
            String::from_utf8_lossy(&launch_output.stdout).trim()
        ))
    })
}

fn spawn_simulator_termination(udid: String, bundle_id: String) {
    let spawn_result = std::thread::Builder::new()
        .name("waterui-simctl-terminate".to_string())
        .spawn(move || {
            match std::process::Command::new("xcrun")
                .args(["simctl", "terminate", &udid, &bundle_id])
                .output()
            {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    tracing::error!(
                        "Failed to terminate app on simulator: status={}, stdout={}, stderr={}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout).trim(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                Err(error) => {
                    tracing::error!("Failed to terminate app on simulator: {error}");
                }
            }
        });

    if let Err(error) = spawn_result {
        tracing::error!("Failed to spawn simulator termination thread: {error}");
    }
}

struct SimulatorExitContext {
    device_name: String,
    device_identifier: String,
    bundle_id: String,
    process_name: String,
    pid: u32,
    start_time: Timestamp,
    start_instant: Instant,
}

fn spawn_simulator_exit_monitor(
    sender: Sender<DeviceEvent>,
    panic_rx: Receiver<PanicInfo>,
    context: SimulatorExitContext,
) {
    spawn(async move {
        wait_for_pid_exit(context.pid).await;

        if let Ok(info) = panic_rx.try_recv() {
            let _ = sender.try_send(DeviceEvent::Crashed(format_panic_message(
                &info.payload,
                info.location.as_deref(),
            )));
            return;
        }

        if let Some(report) = poll_for_crash_report(
            &context.device_name,
            &context.device_identifier,
            &context.bundle_id,
            &context.process_name,
            Some(context.pid),
            context.start_time,
            Duration::from_secs(10),
        )
        .await
        {
            let _ = sender.try_send(DeviceEvent::Crashed(report.to_string()));
            return;
        }

        if let Some(panic_msg) =
            fetch_recent_panic_logs(context.start_instant, Some(context.pid)).await
        {
            let _ = sender.try_send(DeviceEvent::Crashed(panic_msg));
            return;
        }

        let _ = sender.try_send(DeviceEvent::Exited(ApplicationExit::user_closed()));
    })
    .detach();
}

/// Start streaming logs from a `WaterUI` app.
///
/// This uses `log stream` with a predicate to filter logs.
/// - By default, filters by the `WaterUI` subsystem ("dev.waterui").
/// - If `native_logs` is true, filters by process ID instead to capture all native output.
///
/// Returns a receiver for panic info that fires if a panic is detected.
fn start_log_stream(
    sender: Sender<DeviceEvent>,
    log_level: Option<LogLevel>,
    pid: u32,
    native_logs: bool,
) -> Receiver<PanicInfo> {
    // Bounded channel with capacity 1 acts as oneshot - only first panic is captured
    let (panic_tx, panic_rx) = smol::channel::bounded::<PanicInfo>(1);

    // Always stream at default level to capture errors/faults, even if user didn't request logs
    let stream_level = log_level.map_or("default", |l| l.to_apple_level());

    // Build predicate: use processID for native logs, subsystem for WaterUI-only logs
    let predicate = if native_logs {
        format!("processID == {pid}")
    } else {
        format!("processID == {pid} AND subsystem == \"dev.waterui\"")
    };

    let mut log_cmd = Command::new("log");
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
        // Move log_child into the async task to keep it alive
        spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(Ok(line)) = lines.next().await {
                // Skip header lines from `log stream`
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
            }
            // Keep log_child alive until stream ends, then let it drop to kill the process
            drop(log_child);
        })
        .detach();
    }

    panic_rx
}

/// Extract panic information from a log line containing panic.payload and panic.location fields.
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

fn parse_simctl_launch_pid(stdout: &str) -> Option<u32> {
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some((_, pid_part)) = line.rsplit_once(':')
            && let Ok(pid) = pid_part.trim().parse::<u32>()
        {
            return Some(pid);
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

async fn wait_for_pid_exit(pid: u32) {
    while is_pid_alive(pid).await {
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Represents an Apple device available to the CLI.
#[derive(Debug)]
pub enum AppleDevice {
    /// An Apple Simulator device
    Simulator(Box<AppleSimulator>),

    /// The current physical `macOS` device
    ///
    /// Apple do not provide macOS simulator, so this represents the current physical machine.
    /// Uses the shared `Local` device which handles both `.app` bundles and binaries.
    Current(Local),
}

impl Device for AppleDevice {
    fn name(&self) -> &str {
        match self {
            Self::Simulator(simulator) => simulator.name(),
            Self::Current(mac_os) => mac_os.name(),
        }
    }

    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        match self {
            Self::Simulator(simulator) => simulator.launch().await,
            Self::Current(_) => {
                // No need to launch anything for MacOS physical device
                // This is the current machine
                Ok(())
            }
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
        }
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        // Aggregate all available Apple devices: simulators + local
        let mut devices = Vec::new();

        // Add available simulators
        let simulators = AppleSimulator::scan().await?;
        for sim in simulators {
            devices.push(Self::Simulator(Box::new(sim)));
        }

        // Add local machine
        devices.push(Self::Current(Local));

        Ok(devices)
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

    /// Runtime identifier key from `simctl` (e.g. `com.apple.CoreSimulator.SimRuntime.iOS-26-2`).
    ///
    /// This is not part of the simulator device object itself; it comes from the map key in
    /// `xcrun simctl list devices --json`.
    #[serde(skip)]
    pub runtime_identifier: Option<String>,
}

impl Device for AppleSimulator {
    fn name(&self) -> &str {
        &self.name
    }

    /// Launch the Apple simulator (boot it)
    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        // Only boot if not already booted
        if self.state != "Booted" {
            run_command("xcrun", ["simctl", "boot", &self.udid]).await?;
        }
        Ok(())
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
        install_simulator_artifact(&self.udid, artifact.path()).await?;

        info!("Launching app on apple simulator {}", self.name);

        let start_time = Timestamp::now();
        let start_instant = Instant::now();
        let bundle_id = artifact.bundle_id().to_string();
        let process_name = simulator_process_name(&artifact)?;
        let log_level = options.log_level();
        let native_logs = options.native_logs();
        let env_vars = simulator_env_vars(&options);
        let pid = launch_simulator_app(&self.udid, &bundle_id, &env_vars).await?;

        // Create a Running instance - termination will use simctl terminate
        let udid = self.udid.clone();
        let bundle_id_for_termination = bundle_id.clone();
        let (running, sender) = Running::new(move || {
            spawn_simulator_termination(udid, bundle_id_for_termination);
        });

        // Start log streaming and get panic info receiver
        // Uses WaterUI subsystem predicate by default, or processID if native_logs is enabled
        let panic_rx = start_log_stream(sender.clone(), log_level, pid, native_logs);

        // Monitor the actual app process and classify crash vs normal exit.
        spawn_simulator_exit_monitor(
            sender,
            panic_rx,
            SimulatorExitContext {
                device_name: self.name.clone(),
                device_identifier: self.udid.clone(),
                bundle_id,
                process_name,
                pid,
                start_time,
                start_instant,
            },
        );

        Ok(running)
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        #[derive(Deserialize)]
        struct Root {
            devices: HashMap<String, Vec<AppleSimulator>>,
        }

        let content = run_command("xcrun", ["simctl", "list", "devices", "--json"]).await?;

        let root = serde_json::from_str::<Root>(&content)?;
        let mut simulators = Vec::new();
        for (runtime_identifier, sims) in root.devices {
            for mut sim in sims {
                sim.runtime_identifier = Some(runtime_identifier.clone());
                simulators.push(sim);
            }
        }

        Ok(simulators)
    }
}

impl AppleSimulator {
    /// Scan iOS simulators only.
    ///
    /// # Errors
    /// Returns an error if `simctl` cannot be queried for available simulators.
    pub async fn scan_ios() -> eyre::Result<Vec<Self>> {
        let ios_filter = |s: &Self| {
            s.is_available
                && s.runtime_identifier
                    .as_deref()
                    .is_some_and(|r| r.contains("SimRuntime.iOS-"))
        };

        let simulators = Self::scan().await?;
        let mut ios_sims: Vec<Self> = simulators.into_iter().filter(ios_filter).collect();
        let mut healthy: Vec<Self> = ios_sims
            .iter()
            .filter(|s| s.data_path.exists())
            .cloned()
            .collect();
        if !healthy.is_empty() {
            return Ok(healthy);
        }

        if ios_sims.is_empty() {
            return Ok(Vec::new());
        }

        warn!(
            "No healthy iOS simulators found (missing data paths). Attempting automatic simulator repair."
        );

        // Best-effort cleanup first: remove stale entries from unavailable runtimes.
        if let Err(error) = run_command("xcrun", ["simctl", "delete", "unavailable"]).await {
            warn!("Failed to delete unavailable simulators: {error}");
        }

        // Re-scan after cleanup.
        ios_sims = Self::scan().await?.into_iter().filter(ios_filter).collect();
        healthy = ios_sims
            .iter()
            .filter(|s| s.data_path.exists())
            .cloned()
            .collect();
        if !healthy.is_empty() {
            return Ok(healthy);
        }

        // If still broken, create a fresh simulator from a template.
        if let Some(template) = ios_sims
            .iter()
            .find(|s| s.device_type_identifier.contains("iPhone"))
            .or_else(|| ios_sims.first())
            .cloned()
            && let Some(runtime) = template.runtime_identifier.as_deref()
        {
            let generated_name = format!("{} (WaterUI)", template.name);
            match run_command(
                "xcrun",
                [
                    "simctl",
                    "create",
                    &generated_name,
                    &template.device_type_identifier,
                    runtime,
                ],
            )
            .await
            {
                Ok(udid) => {
                    info!(
                        "Created replacement iOS simulator: {} ({})",
                        generated_name,
                        udid.trim()
                    );
                }
                Err(error) => {
                    warn!("Failed to create replacement iOS simulator: {error}");
                }
            }
        }

        // Final re-scan: return only healthy simulators.
        Ok(Self::scan()
            .await?
            .into_iter()
            .filter(ios_filter)
            .filter(|s| s.data_path.exists())
            .collect())
    }
}

/// Capture a screenshot from an iOS simulator.
///
/// Uses `xcrun simctl io <udid> screenshot <output_path>` to capture
/// the current screen of the simulator.
///
/// # Errors
///
/// Returns an error if the screenshot command fails or the simulator
/// is not available.
pub async fn screenshot(udid: &str, output: &Path) -> eyre::Result<()> {
    run_command(
        "xcrun",
        [
            "simctl",
            "io",
            udid,
            "screenshot",
            output
                .to_str()
                .ok_or_else(|| eyre!("Invalid output path"))?,
        ],
    )
    .await?;
    Ok(())
}

/// Capture a screenshot from an iOS simulator and return the raw PNG bytes.
///
/// This is used for the diff workflow where we need in-memory screenshots.
///
/// # Errors
///
/// Returns an error if the screenshot command fails or the simulator is not available.
pub async fn screenshot_bytes(udid: &str) -> eyre::Result<Vec<u8>> {
    // Use "-" to output to stdout
    let output = Command::new("xcrun")
        .args(["simctl", "io", udid, "screenshot", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to capture screenshot: {}", stderr.trim());
    }

    Ok(output.stdout)
}

/// Check if IDB (iOS Development Bridge) is installed.
///
/// IDB is required for gesture automation on iOS simulators.
async fn check_idb_installed() -> eyre::Result<()> {
    let output = Command::new("which").arg("idb").output().await?;

    if !output.status.success() {
        eyre::bail!(
            "IDB (iOS Development Bridge) is not installed.\n\n\
            Gesture commands require IDB for iOS simulator automation.\n\n\
            To install IDB:\n\
            \x20 brew tap facebook/fb && brew install idb-companion\n\
            \x20 pipx install fb-idb --python python3.12\n\n\
            For more information: https://fbidb.io/"
        );
    }

    Ok(())
}

/// Perform a tap gesture on an iOS simulator at the specified coordinates.
///
/// Uses IDB (iOS Development Bridge) to send touch events to the simulator.
///
/// # Arguments
///
/// * `udid` - The simulator's unique device identifier
/// * `x` - X coordinate within the simulator screen
/// * `y` - Y coordinate within the simulator screen
///
/// # Errors
///
/// Returns an error if IDB is not installed or the tap fails.
pub async fn tap(udid: &str, x: u32, y: u32) -> eyre::Result<()> {
    check_idb_installed().await?;

    let output = Command::new("idb")
        .args(["ui", "tap", "--udid", udid, &x.to_string(), &y.to_string()])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to tap: {}", stderr.trim());
    }

    Ok(())
}

/// Perform a swipe gesture on an iOS simulator.
///
/// Uses IDB (iOS Development Bridge) to send swipe events to the simulator.
///
/// # Arguments
///
/// * `udid` - The simulator's unique device identifier
/// * `from` - Starting coordinates (x, y)
/// * `to` - Ending coordinates (x, y)
/// * `duration_ms` - Duration of the swipe in milliseconds (optional)
///
/// # Errors
///
/// Returns an error if IDB is not installed or the swipe fails.
pub async fn swipe(
    udid: &str,
    from: (u32, u32),
    to: (u32, u32),
    duration_ms: Option<u32>,
) -> eyre::Result<()> {
    check_idb_installed().await?;

    let mut args = vec![
        "ui".to_string(),
        "swipe".to_string(),
        "--udid".to_string(),
        udid.to_string(),
        from.0.to_string(),
        from.1.to_string(),
        to.0.to_string(),
        to.1.to_string(),
    ];

    if let Some(duration) = duration_ms {
        // IDB uses duration in seconds as a float
        let duration_sec = f64::from(duration) / 1000.0;
        args.push("--duration".to_string());
        args.push(format!("{duration_sec:.2}"));
    }

    let output = Command::new("idb").args(&args).output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to swipe: {}", stderr.trim());
    }

    Ok(())
}

/// Input text on an iOS simulator.
///
/// Uses IDB (iOS Development Bridge) to send text input to the simulator.
///
/// # Errors
///
/// Returns an error if IDB is not installed or the text input fails.
pub async fn text(udid: &str, input: &str) -> eyre::Result<()> {
    check_idb_installed().await?;

    let output = Command::new("idb")
        .args(["ui", "text", "--udid", udid, input])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to input text: {}", stderr.trim());
    }

    Ok(())
}

/// Describe UI elements on the screen.
///
/// Uses IDB to get accessibility information about all UI elements.
/// Returns JSON string with element details (frame, label, type, etc.).
///
/// # Errors
///
/// Returns an error if IDB is not installed or the command fails.
pub async fn describe(udid: &str) -> eyre::Result<String> {
    check_idb_installed().await?;

    let output = Command::new("idb")
        .args(["ui", "describe-all", "--udid", udid, "--json"])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to describe UI: {}", stderr.trim());
    }

    let json = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(json)
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
