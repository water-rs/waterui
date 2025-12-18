use color_eyre::eyre::{self, eyre};
use smol::channel::{Receiver, Sender};
use smol::io::AsyncWriteExt;
use smol::process::Command;
use smol::spawn;
use tracing::error;

use std::path::Path;
use std::process::Stdio;

use crate::{
    android::toolchain::AndroidSdk,
    device::{Artifact, Device, DeviceEvent, FailToRun, LogLevel, RunOptions, Running},
    utils::{parse_whitespace_separated_u32s, run_command, run_command_output},
};

/// Panic information extracted from logcat.
#[derive(Debug, Clone)]
struct PanicInfo {
    payload: String,
    location: Option<String>,
}

/// Represents an Android device (physical or emulator).
#[derive(Debug)]
pub struct AndroidDevice {
    identifier: String,
    /// Primary ABI of the device (e.g., "arm64-v8a", "`x86_64`")
    abi: String,
}

impl AndroidDevice {
    /// Create a new Android device with the given identifier and ABI.
    #[must_use]
    pub const fn new(identifier: String, abi: String) -> Self {
        Self { identifier, abi }
    }

    /// Get the device identifier.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Get the device's primary ABI.
    #[must_use]
    pub fn abi(&self) -> &str {
        &self.abi
    }
}

impl Device for AndroidDevice {
    fn name(&self) -> &str {
        &self.identifier
    }

    async fn launch(&self) -> eyre::Result<()> {
        let adb = AndroidSdk::adb_path()
            .ok_or_else(|| eyre::eyre!("Android SDK not found or adb not installed"))?;
        run_command(
            adb.to_str().unwrap(),
            ["-s", &self.identifier, "wait-for-device"],
        )
        .await?;
        Ok(())
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        run_on_android(&self.identifier, artifact, options).await
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        let adb = AndroidSdk::adb_path()
            .ok_or_else(|| eyre::eyre!("Android SDK not found or adb not installed"))?;

        let output = run_command(adb.to_str().unwrap(), ["devices", "-l"])
            .await
            .map_err(|e| eyre!("Failed to list devices: {e}"))?;

        let mut devices = Vec::new();

        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "device" {
                let identifier = parts[0].to_string();

                // Get device ABI
                let abi = run_command(
                    adb.to_str().unwrap(),
                    ["-s", &identifier, "shell", "getprop", "ro.product.cpu.abi"],
                )
                .await
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "arm64-v8a".to_string());

                devices.push(Self::new(identifier, abi));
            }
        }

        Ok(devices)
    }
}

/// Shared implementation for running an app on any Android device.
///
/// This handles:
/// - Passing environment variables as intent extras
/// - Uninstalling previous version (to avoid storage issues)
/// - Installing the APK
/// - Launching the app
/// - Monitoring process state
/// - Streaming logs
#[allow(clippy::too_many_lines)]
async fn run_on_android(
    device_id: &str,
    artifact: Artifact,
    options: RunOptions,
) -> Result<Running, FailToRun> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| FailToRun::Run(eyre!("Android SDK not found or adb not installed")))?;
    let adb_str = adb.to_str().unwrap();

    let env_vars: Vec<(String, String)> = options
        .env_vars()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // If hot reload is using localhost, set up adb reverse so the device can connect back
    // to the host's hot reload server (listening on 127.0.0.1:<port>).
    let reverse_port = env_vars
        .iter()
        .find(|(k, _)| k == "WATERUI_HOT_RELOAD_PORT")
        .and_then(|(_, v)| v.parse::<u16>().ok())
        .zip(
            env_vars
                .iter()
                .find(|(k, _)| k == "WATERUI_HOT_RELOAD_HOST")
                .map(|(_, v)| v.as_str()),
        )
        .and_then(|(port, host)| {
            if host == "127.0.0.1" || host == "localhost" {
                Some(port)
            } else {
                None
            }
        });

    if let Some(port) = reverse_port {
        let spec = format!("tcp:{port}");
        let output = Command::new(adb_str)
            .args(["-s", device_id, "reverse", &spec, &spec])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                tracing::warn!(
                    "Failed to set up adb reverse for hot reload ({}): stdout='{}' stderr='{}'",
                    spec,
                    String::from_utf8_lossy(&output.stdout).trim(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Err(e) => {
                tracing::warn!("Failed to set up adb reverse for hot reload ({spec}): {e}");
            }
        }
    }

    // Install the APK on the device with -r flag to replace existing installation
    // This handles both cases: fresh install and reinstall over existing app
    run_command(
        adb_str,
        [
            "-s",
            device_id,
            "install",
            "-r",
            artifact.path().to_str().unwrap(),
        ],
    )
    .await
    .map_err(|e| FailToRun::Install(eyre!("Failed to install APK: {e}")))?;

    // Launch the app (pass env vars as intent extras).
    //
    // We use the "waterui.env.<KEY>" namespace to avoid collisions.
    // MainActivity reads these extras and calls Os.setenv() before loading native libraries.
    let mut start_args = vec![
        "-s".to_string(),
        device_id.to_string(),
        "shell".to_string(),
        "am".to_string(),
        "start".to_string(),
        "-S".to_string(), // force-stop target app before starting (ensures env takes effect)
        "-n".to_string(),
        format!("{}/.MainActivity", artifact.bundle_id()),
    ];

    for (key, value) in &env_vars {
        start_args.push("--es".to_string());
        start_args.push(format!("waterui.env.{key}"));
        start_args.push(value.clone());
    }

    let output = Command::new(adb_str)
        .args(&start_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

    if !output.status.success() {
        return Err(FailToRun::Launch(eyre!(
            "Failed to launch app:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }

    // Wait for the process to start and get its PID
    let pid = wait_for_app_pid(adb_str, device_id, artifact.bundle_id()).await?;

    let adb_for_kill = adb.clone();
    let identifier_for_kill = device_id.to_string();
    let identifier_for_monitor = device_id.to_string();
    let bundle_id_for_kill = artifact.bundle_id().to_string();
    let bundle_id_for_monitor = artifact.bundle_id().to_string();
    let log_level = options.log_level();
    let reverse_port_for_drop = reverse_port;

    let (running, sender) = Running::new(move || {
        // Use std::process::Command for synchronous execution in Drop context
        let result = std::process::Command::new(&adb_for_kill)
            .args([
                "-s",
                &identifier_for_kill,
                "shell",
                "am",
                "force-stop",
                &bundle_id_for_kill,
            ])
            .output();

        match result {
            Ok(output) => {
                tracing::debug!(
                    "Force-stop command executed: status={}, stdout={}, stderr={}",
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                error!("Failed to stop app {}: {}", bundle_id_for_kill, e);
            }
        }

        if let Some(port) = reverse_port_for_drop {
            let spec = format!("tcp:{port}");
            let _ = std::process::Command::new(&adb_for_kill)
                .args(["-s", &identifier_for_kill, "reverse", "--remove", &spec])
                .output();
        }
    });

    // Clone sender for different tasks before moving
    let sender_for_monitor = sender.clone();
    let sender_for_panic = sender.clone();
    let sender_for_logs = sender;

    // Spawn a background task to monitor the process
    let adb_for_monitor = adb.clone();
    smol::spawn(async move {
        monitor_android_process(
            adb_for_monitor,
            &identifier_for_monitor,
            &bundle_id_for_monitor,
            pid,
            sender_for_monitor,
        )
        .await;
    })
    .detach();

    // Always stream logs at fatal level to capture panics, with optional display
    let adb_for_logs = adb;
    let identifier_for_logs = device_id.to_string();
    let panic_rx = start_android_log_stream(
        adb_for_logs,
        identifier_for_logs,
        pid,
        log_level,
        sender_for_logs,
    );

    // Listen for panic info and send as crash event
    spawn(async move {
        if let Ok(info) = panic_rx.recv().await {
            // Format panic message for panic_report() display
            let mut msg = format!("Panic: {}", info.payload);
            if let Some(loc) = info.location {
                msg = format!("{msg}\n  at {loc}");
            }
            let _ = sender_for_panic.send(DeviceEvent::Crashed(msg)).await;
        }
    })
    .detach();

    Ok(running)
}

/// Wait for an app to start and return its PID.
async fn wait_for_app_pid(
    adb_str: &str,
    device_id: &str,
    bundle_id: &str,
) -> Result<u32, FailToRun> {
    for _ in 0..10 {
        smol::Timer::after(std::time::Duration::from_millis(200)).await;
        if let Ok(output) =
            run_command(adb_str, ["-s", device_id, "shell", "pidof", bundle_id]).await
        {
            if let Some(pid) = parse_whitespace_separated_u32s(&output).into_iter().next() {
                return Ok(pid);
            }
        }
    }

    // App likely crashed on startup - fetch logcat for crash info
    let crash_info = run_command(
        adb_str,
        [
            "-s",
            device_id,
            "logcat",
            "-d",
            "-t",
            "100",
            "-s",
            "AndroidRuntime:E",
            "DEBUG:*",
            "WaterUI:*",
        ],
    )
    .await
    .unwrap_or_default();

    let mut error_msg = format!("App {bundle_id} crashed on startup (process not found).\n\n");

    if !crash_info.trim().is_empty() {
        error_msg.push_str("=== Crash Log ===\n");
        error_msg.push_str(&crash_info);
    }

    Err(FailToRun::Launch(eyre!("{}", error_msg)))
}

/// Find the running emulator's device identifier.
async fn find_emulator_identifier() -> Result<String, FailToRun> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| FailToRun::Run(eyre!("Android SDK not found or adb not installed")))?;

    let output = run_command(adb.to_str().unwrap(), ["devices"])
        .await
        .map_err(|e| FailToRun::Run(eyre!("Failed to list devices: {e}")))?;

    output
        .lines()
        .skip(1)
        .find_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0].starts_with("emulator-") && parts[1] == "device" {
                Some(parts[0].to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| FailToRun::Run(eyre!("Emulator not running")))
}

/// Monitor an Android process and send events when it crashes or exits.
async fn monitor_android_process(
    adb: std::path::PathBuf,
    device_id: &str,
    bundle_id: &str,
    pid: u32,
    sender: smol::channel::Sender<DeviceEvent>,
) {
    let adb_str = adb.to_str().unwrap_or_default();

    // Check process status periodically
    loop {
        smol::Timer::after(std::time::Duration::from_secs(1)).await;

        // Check if process is still running using pidof
        // Note: We use pidof instead of kill -0 because kill -0 returns "Operation not permitted"
        // when the shell user doesn't have permission to send signals to the app process
        let result = run_command(adb_str, ["-s", device_id, "shell", "pidof", bundle_id]).await;

        // Check if the process with the same PID is still running
        let still_running = result
            .as_ref()
            .ok()
            .map(|output| parse_whitespace_separated_u32s(output))
            .is_some_and(|pids| pids.contains(&pid));

        if !still_running {
            // Give crash reporting a brief moment to flush logs.
            smol::Timer::after(std::time::Duration::from_millis(500)).await;

            // Try to fetch logs for this PID (best signal for distinguishing crash vs normal exit).
            let pid_arg = format!("--pid={pid}");
            let pid_log_args = vec![
                "-s".to_string(),
                device_id.to_string(),
                "logcat".to_string(),
                "-v".to_string(),
                "threadtime".to_string(),
                "-d".to_string(),
                "-t".to_string(),
                "200".to_string(),
                pid_arg,
                "*:V".to_string(),
            ];
            let pid_log = run_command_output(adb_str, pid_log_args.iter().map(String::as_str))
                .await
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();

            // Fallback for older logcat versions that don't support --pid.
            let fallback_log = if pid_log.trim().is_empty() {
                let fallback_args = vec![
                    "-s".to_string(),
                    device_id.to_string(),
                    "logcat".to_string(),
                    "-v".to_string(),
                    "threadtime".to_string(),
                    "-d".to_string(),
                    "-t".to_string(),
                    "200".to_string(),
                    "-s".to_string(),
                    "AndroidRuntime:E".to_string(),
                    "DEBUG:*".to_string(),
                    "libc:F".to_string(),
                ];
                run_command_output(adb_str, fallback_args.iter().map(String::as_str))
                    .await
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let pid_filtered = !pid_log.trim().is_empty();
            let log_for_detection = if pid_filtered {
                pid_log.as_str()
            } else {
                fallback_log.as_str()
            };

            if android_log_looks_like_crash(log_for_detection, bundle_id, pid, pid_filtered) {
                let crash_log = if pid_log.trim().is_empty() {
                    fallback_log
                } else {
                    pid_log
                };

                let error_msg = if crash_log.trim().is_empty() {
                    format!("Process {bundle_id} crashed.")
                } else {
                    format!("Process {bundle_id} crashed.\n\n=== Crash Log ===\n{crash_log}")
                };

                let _ = sender.send(DeviceEvent::Crashed(error_msg)).await;
            } else {
                let _ = sender.send(DeviceEvent::Exited).await;
            }
            break;
        }
    }
}

fn log_mentions_pid(log: &str, pid: u32) -> bool {
    let pid_str = pid.to_string();
    let pid_lower = format!("pid: {pid}");
    let pid_upper = format!("PID: {pid}");

    log.lines().any(|line| {
        line.split_whitespace().any(|part| part == pid_str)
            || line.contains(&pid_lower)
            || line.contains(&pid_upper)
    })
}

fn android_log_looks_like_crash(log: &str, bundle_id: &str, pid: u32, pid_filtered: bool) -> bool {
    if log.trim().is_empty() {
        return false;
    }

    // When we don't have a PID-filtered dump (older logcat), ensure we don't accidentally pick up
    // crashes from unrelated processes.
    let relevant = pid_filtered || log.contains(bundle_id) || log_mentions_pid(log, pid);
    if !relevant {
        return false;
    }

    // Common Java crash markers (AndroidRuntime).
    if log.contains("FATAL EXCEPTION") {
        return true;
    }

    // Common native crash markers (tombstone / debuggerd / libc).
    if log.contains("Fatal signal") {
        return true;
    }
    if log.contains("SIGSEGV")
        || log.contains("SIGABRT")
        || log.contains("SIGBUS")
        || log.contains("SIGILL")
        || log.contains("SIGFPE")
    {
        return true;
    }
    if log.contains("Abort message:") || log.contains("backtrace:") {
        return true;
    }

    // If we only have a global log fallback (no --pid), make sure it actually mentions this app
    // and includes an error marker to avoid false positives from unrelated processes.
    if !log.contains(bundle_id) {
        return false;
    }

    // Heuristic: treat AndroidRuntime errors for this process as crash.
    log.contains("AndroidRuntime")
        && (log.contains("E AndroidRuntime") || log.contains("Exception"))
}

/// Start log streaming from an Android process using logcat.
///
/// Always streams at minimum fatal level to capture panics.
/// Returns a receiver for panic info that fires if a panic is detected.
fn start_android_log_stream(
    adb: std::path::PathBuf,
    device_id: String,
    pid: u32,
    log_level: Option<LogLevel>,
    sender: Sender<DeviceEvent>,
) -> Receiver<PanicInfo> {
    use futures::io::{AsyncBufReadExt, BufReader};
    use futures::StreamExt;

    // Bounded channel with capacity 1 acts as oneshot - only first panic is captured
    let (panic_tx, panic_rx) = smol::channel::bounded::<PanicInfo>(1);

    // Always stream at fatal level to capture panics, even if user didn't request logs
    let priority = log_level.map_or('F', |l| l.to_android_priority());

    // Build logcat command with PID filter and minimum priority
    let pid_arg = format!("--pid={pid}");
    let mut cmd = Command::new(&adb);
    cmd.args(["-s", &device_id, "logcat", "-v", "threadtime"])
        .arg(pid_arg)
        .arg(format!("*:{priority}"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to spawn logcat: {e}");
            return panic_rx;
        }
    };

    let Some(stdout) = child.stdout.take() else {
        return panic_rx;
    };

    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    spawn(async move {
        // Parse logcat output and send as DeviceEvent::Log
        // Logcat format: "MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG: message"
        while let Some(result) = lines.next().await {
            let Ok(line) = result else { break };

            // Extract panic info from log line if present (only first panic via try_send)
            if line.contains("panic.payload=") {
                if let Some(info) = extract_panic_info_from_log(&line) {
                    let _ = panic_tx.try_send(info);
                }
            }

            // Only send log events to display if user requested logs
            if log_level.is_some() {
                let (parsed_level, message) = parse_logcat_line(&line);

                if sender
                    .try_send(DeviceEvent::Log {
                        level: parsed_level,
                        message,
                    })
                    .is_err()
                {
                    break;
                }
            }
        }

        // Clean up child process
        let _ = child.kill();
    })
    .detach();

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

/// Parsed logcat line with level, tag, and message.
struct LogcatParsed {
    level: tracing::Level,
    tag: String,
    message: String,
}

/// Parse a logcat line into level, tag, and message.
/// Logcat threadtime format: "MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG: message"
fn parse_logcat_line(line: &str) -> (tracing::Level, String) {
    // Try to parse the structured format
    if let Some(parsed) = try_parse_logcat(line) {
        let formatted = format!("[{}] {}", parsed.tag, parsed.message);
        return (parsed.level, formatted);
    }

    // Fallback: return raw line with default level
    (tracing::Level::INFO, line.to_string())
}

/// Try to parse a logcat line. Returns None if parsing fails.
fn try_parse_logcat(line: &str) -> Option<LogcatParsed> {
    // Logcat threadtime format: "MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG: message"
    // Example: "12-10 23:04:40.190 28184 28184 D WaterUI : Touch..."

    // Split by whitespace, but we need to be careful about the message part
    let parts: Vec<&str> = line.splitn(7, char::is_whitespace).collect();

    // We need at least: date, time, pid, tid, level, tag, message
    if parts.len() < 6 {
        return None;
    }

    // Find the level character (should be single char: V, D, I, W, E, F)
    let mut level_idx = None;
    for (i, part) in parts.iter().enumerate() {
        if part.len() == 1 {
            let c = part.chars().next()?;
            if matches!(c, 'V' | 'D' | 'I' | 'W' | 'E' | 'F') {
                level_idx = Some(i);
                break;
            }
        }
    }

    let level_idx = level_idx?;
    if level_idx + 1 >= parts.len() {
        return None;
    }

    let level = match parts[level_idx] {
        "E" | "F" => tracing::Level::ERROR,
        "W" => tracing::Level::WARN,
        "D" => tracing::Level::DEBUG,
        "V" => tracing::Level::TRACE,
        _ => tracing::Level::INFO,
    };

    // The rest after level is "TAG: message" or "TAG     : message"
    // Find the position of the level character in the original line (after timestamp)
    // Skip past timestamp "MM-DD HH:MM:SS.mmm" which is about 18 chars
    let level_char = parts[level_idx].chars().next()?;
    let search_start = 18.min(line.len());
    let level_pos = line[search_start..]
        .find(level_char)
        .map(|p| p + search_start)?;

    let after_level = line.get(level_pos + 1..)?.trim_start();

    // Split by ": " to get tag and message
    after_level.find(": ").map_or_else(
        || {
            Some(LogcatParsed {
                level,
                tag: "unknown".to_string(),
                message: after_level.to_string(),
            })
        },
        |colon_pos| {
            let tag = after_level[..colon_pos].trim();
            let message = after_level[colon_pos + 2..].to_string();
            Some(LogcatParsed {
                level,
                tag: tag.to_string(),
                message,
            })
        },
    )
}

/// Android emulator (AVD) that needs to be launched.
///
/// Unlike `AndroidDevice` which represents an already-connected device,
/// `AndroidEmulator` represents an AVD that will be launched when `launch()` is called.
#[derive(Debug)]
pub struct AndroidEmulator {
    /// AVD name.
    avd_name: String,
}

impl AndroidEmulator {
    /// Create a new Android emulator with the given AVD name.
    #[must_use]
    pub const fn new(avd_name: String) -> Self {
        Self { avd_name }
    }

    /// Get the AVD name.
    #[must_use]
    pub fn avd_name(&self) -> &str {
        &self.avd_name
    }
}

impl Device for AndroidEmulator {
    fn name(&self) -> &str {
        &self.avd_name
    }

    async fn launch(&self) -> eyre::Result<()> {
        let emulator_path =
            AndroidSdk::emulator_path().ok_or_else(|| eyre::eyre!("Android emulator not found"))?;

        // Start the emulator process (don't wait for it here, we'll poll for readiness)
        Command::new(&emulator_path)
            .arg("-avd")
            .arg(&self.avd_name)
            .arg("-no-snapshot-load")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        // Wait for the emulator to boot by polling adb devices
        let adb_path =
            AndroidSdk::adb_path().ok_or_else(|| eyre::eyre!("Android adb not found"))?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(120);

        loop {
            if start.elapsed() > timeout {
                eyre::bail!("Emulator launch timed out after 120 seconds");
            }

            // Check for booted emulator via adb
            if let Ok(output) = Command::new(&adb_path).arg("devices").output().await {
                if let Ok(stdout) = String::from_utf8(output.stdout) {
                    for line in stdout.lines().skip(1) {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2
                            && parts[0].starts_with("emulator-")
                            && parts[1] == "device"
                        {
                            // Emulator is ready
                            return Ok(());
                        }
                    }
                }
            }

            smol::Timer::after(std::time::Duration::from_secs(2)).await;
        }
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        let identifier = find_emulator_identifier().await?;
        run_on_android(&identifier, artifact, options).await
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        // List available AVDs using avdmanager or emulator -list-avds
        let emulator_path = AndroidSdk::emulator_path()
            .ok_or_else(|| eyre::eyre!("Android emulator not found"))?;

        let output = Command::new(&emulator_path)
            .arg("-list-avds")
            .output()
            .await
            .map_err(|e| eyre!("Failed to list AVDs: {e}"))?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let avds: Vec<Self> = stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|name| Self::new(name.trim().to_string()))
            .collect();

        Ok(avds)
    }
}

/// Capture a screenshot from an Android device.
///
/// Uses `adb exec-out screencap -p` to capture the screen and writes
/// the PNG data directly to the output file.
///
/// # Errors
///
/// Returns an error if the screenshot command fails, the device is not
/// available, or the output file cannot be written.
pub async fn screenshot(device_id: &str, output: &Path) -> eyre::Result<()> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| eyre!("Android SDK not found or adb not installed"))?;

    let child = Command::new(&adb)
        .args(["-s", device_id, "exec-out", "screencap", "-p"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let output_result = child.output().await?;

    if !output_result.status.success() {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        eyre::bail!("Failed to capture screenshot: {}", stderr.trim());
    }

    // Write the PNG data to the output file
    let mut file = smol::fs::File::create(output).await?;
    file.write_all(&output_result.stdout).await?;
    file.flush().await?;

    Ok(())
}

/// Perform a tap gesture on an Android device at the specified coordinates.
///
/// Uses `adb shell input tap <x> <y>` to simulate a touch event.
///
/// # Errors
///
/// Returns an error if the tap command fails or the device is not available.
pub async fn tap(device_id: &str, x: u32, y: u32) -> eyre::Result<()> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| eyre!("Android SDK not found or adb not installed"))?;

    run_command(
        adb.to_str().unwrap(),
        ["-s", device_id, "shell", "input", "tap", &x.to_string(), &y.to_string()],
    )
    .await?;

    Ok(())
}

/// Perform a swipe gesture on an Android device.
///
/// Uses `adb shell input swipe <x1> <y1> <x2> <y2> [duration_ms]` to simulate a swipe.
///
/// # Arguments
///
/// * `device_id` - The Android device identifier
/// * `from` - Starting coordinates (x, y)
/// * `to` - Ending coordinates (x, y)
/// * `duration_ms` - Optional duration in milliseconds (default ~300ms if not specified)
///
/// # Errors
///
/// Returns an error if the swipe command fails or the device is not available.
pub async fn swipe(
    device_id: &str,
    from: (u32, u32),
    to: (u32, u32),
    duration_ms: Option<u32>,
) -> eyre::Result<()> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| eyre!("Android SDK not found or adb not installed"))?;

    let mut args = vec![
        "-s",
        device_id,
        "shell",
        "input",
        "swipe",
    ];

    let x1 = from.0.to_string();
    let y1 = from.1.to_string();
    let x2 = to.0.to_string();
    let y2 = to.1.to_string();
    let duration = duration_ms.map(|d| d.to_string());

    args.push(&x1);
    args.push(&y1);
    args.push(&x2);
    args.push(&y2);

    if let Some(ref d) = duration {
        args.push(d);
    }

    run_command(adb.to_str().unwrap(), args).await?;

    Ok(())
}

/// Input text on an Android device.
///
/// Uses `adb shell input text "<string>"` to type text.
/// Note: Special characters may need escaping.
///
/// # Errors
///
/// Returns an error if the text input command fails or the device is not available.
pub async fn text(device_id: &str, input: &str) -> eyre::Result<()> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| eyre!("Android SDK not found or adb not installed"))?;

    // Escape special characters for shell
    let escaped = input
        .replace('\\', "\\\\")
        .replace(' ', "%s")
        .replace('"', "\\\"")
        .replace('\'', "\\'")
        .replace('&', "\\&")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('|', "\\|")
        .replace(';', "\\;")
        .replace('(', "\\(")
        .replace(')', "\\)");

    run_command(
        adb.to_str().unwrap(),
        ["-s", device_id, "shell", "input", "text", &escaped],
    )
    .await?;

    Ok(())
}

/// Capture a screenshot from an Android device and return the raw PNG bytes.
///
/// This is used for the diff workflow where we need in-memory screenshots.
///
/// # Errors
///
/// Returns an error if the screenshot command fails or the device is not available.
pub async fn screenshot_bytes(device_id: &str) -> eyre::Result<Vec<u8>> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| eyre!("Android SDK not found or adb not installed"))?;

    let child = Command::new(&adb)
        .args(["-s", device_id, "exec-out", "screencap", "-p"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let output = child.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to capture screenshot: {}", stderr.trim());
    }

    Ok(output.stdout)
}

/// Describe UI elements on the screen.
///
/// Uses `uiautomator dump` to get UI hierarchy as XML, then converts to JSON.
///
/// # Errors
///
/// Returns an error if adb is not available or the command fails.
pub async fn describe(device_id: &str) -> eyre::Result<String> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| eyre!("Android SDK not found or adb not installed"))?;

    // Dump UI hierarchy to a temp file on device
    let dump_path = "/sdcard/window_dump.xml";
    run_command(
        adb.to_str().unwrap(),
        ["-s", device_id, "shell", "uiautomator", "dump", dump_path],
    )
    .await?;

    // Read the dump file
    let output = Command::new(adb.to_str().unwrap())
        .args(["-s", device_id, "shell", "cat", dump_path])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to read UI dump: {}", stderr.trim());
    }

    let xml = String::from_utf8_lossy(&output.stdout).to_string();

    // Convert XML to simplified JSON format
    let json = xml_to_ui_json(&xml)?;

    // Clean up
    let _ = run_command(
        adb.to_str().unwrap(),
        ["-s", device_id, "shell", "rm", dump_path],
    )
    .await;

    Ok(json)
}

/// Convert Android UI XML dump to a JSON format similar to iOS IDB output.
fn xml_to_ui_json(xml: &str) -> eyre::Result<String> {
    let mut elements = Vec::new();

    // Simple XML parsing - find all <node> elements
    for line in xml.lines() {
        if !line.contains("<node") {
            continue;
        }

        let mut element = serde_json::Map::new();

        // Extract bounds attribute: bounds="[left,top][right,bottom]"
        if let Some(bounds_start) = line.find("bounds=\"[") {
            let bounds_str = &line[bounds_start + 8..];
            if let Some(bounds_end) = bounds_str.find('"') {
                let bounds = &bounds_str[..bounds_end];
                // Parse [left,top][right,bottom]
                let parts: Vec<&str> = bounds
                    .trim_matches(|c| c == '[' || c == ']')
                    .split("][")
                    .collect();
                if parts.len() == 2 {
                    let lt: Vec<i32> = parts[0]
                        .split(',')
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    let rb: Vec<i32> = parts[1]
                        .split(',')
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    if lt.len() == 2 && rb.len() == 2 {
                        let mut frame = serde_json::Map::new();
                        frame.insert("x".to_string(), serde_json::Value::Number(lt[0].into()));
                        frame.insert("y".to_string(), serde_json::Value::Number(lt[1].into()));
                        frame.insert(
                            "width".to_string(),
                            serde_json::Value::Number((rb[0] - lt[0]).into()),
                        );
                        frame.insert(
                            "height".to_string(),
                            serde_json::Value::Number((rb[1] - lt[1]).into()),
                        );
                        element.insert("frame".to_string(), serde_json::Value::Object(frame));
                    }
                }
            }
        }

        // Extract common attributes
        for attr in ["text", "content-desc", "class", "resource-id"] {
            let search = format!("{attr}=\"");
            if let Some(start) = line.find(&search) {
                let value_start = start + search.len();
                let rest = &line[value_start..];
                if let Some(end) = rest.find('"') {
                    let value = &rest[..end];
                    if !value.is_empty() {
                        let key = match attr {
                            "content-desc" => "AXLabel",
                            "class" => "type",
                            "resource-id" => "AXUniqueId",
                            "text" => "AXValue",
                            _ => attr,
                        };
                        element.insert(key.to_string(), serde_json::Value::String(value.to_string()));
                    }
                }
            }
        }

        // Extract clickable/enabled attributes
        if line.contains("clickable=\"true\"") {
            element.insert("clickable".to_string(), serde_json::Value::Bool(true));
        }
        if line.contains("enabled=\"true\"") {
            element.insert("enabled".to_string(), serde_json::Value::Bool(true));
        }

        if !element.is_empty() {
            elements.push(serde_json::Value::Object(element));
        }
    }

    serde_json::to_string(&elements).map_err(|e| eyre!("Failed to serialize UI elements: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{android_log_looks_like_crash, log_mentions_pid};

    #[test]
    fn detects_pid_mentions_in_threadtime_lines() {
        let log = "12-10 23:04:40.190 28184 28184 F libc    : Fatal signal 11 (SIGSEGV)\n";
        assert!(log_mentions_pid(log, 28184));
        assert!(!log_mentions_pid(log, 12345));
    }

    #[test]
    fn avoids_false_positive_from_unrelated_fatal_signal_in_global_dump() {
        let unrelated = "12-10 23:04:40.190 999 999 F libc    : Fatal signal 11 (SIGSEGV)\n";
        assert!(!android_log_looks_like_crash(
            unrelated,
            "com.example.app",
            28184,
            false
        ));
    }

    #[test]
    fn detects_native_crash_when_pid_is_mentioned() {
        let log = "I DEBUG : Fatal signal 11 (SIGSEGV), code 1, fault addr 0x0 in tid 1 (main) pid: 28184\n";
        assert!(android_log_looks_like_crash(
            log,
            "com.example.app",
            28184,
            false
        ));
    }

    #[test]
    fn detects_java_crash_for_app() {
        let log = "E AndroidRuntime: FATAL EXCEPTION: main\nE AndroidRuntime: Process: com.example.app, PID: 28184\n";
        assert!(android_log_looks_like_crash(
            log,
            "com.example.app",
            28184,
            false
        ));
    }
}
