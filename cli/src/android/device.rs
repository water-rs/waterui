use color_eyre::eyre::{self, eyre};
use smol::process::Command;
use tracing::error;

use crate::{
    android::{platform::AndroidPlatform, toolchain::AndroidSdk},
    device::{Artifact, Device, DeviceEvent, FailToRun, LogLevel, RunOptions, Running},
    utils::run_command,
};

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
    type Platform = AndroidPlatform;

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

    fn platform(&self) -> Self::Platform {
        AndroidPlatform::from_abi(&self.abi)
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        run_on_android(&self.identifier, artifact, options).await
    }
}

/// Shared implementation for running an app on any Android device.
///
/// This handles:
/// - Setting environment variables as system properties
/// - Uninstalling previous version (to avoid storage issues)
/// - Installing the APK
/// - Launching the app
/// - Monitoring process state
/// - Streaming logs
async fn run_on_android(
    device_id: &str,
    artifact: Artifact,
    options: RunOptions,
) -> Result<Running, FailToRun> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| FailToRun::Run(eyre!("Android SDK not found or adb not installed")))?;
    let adb_str = adb.to_str().unwrap();

    // Set environment variables as system properties
    // Android doesn't support direct environment variable passing, so we use
    // system properties with prefix "waterui.env." which the app reads at startup
    for (key, value) in options.env_vars() {
        let prop_key = format!("waterui.env.{key}");
        let result = run_command(
            adb_str,
            ["-s", device_id, "shell", "setprop", &prop_key, value],
        )
        .await;
        if let Err(e) = result {
            tracing::warn!("Failed to set system property {prop_key}: {e}");
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

    // Launch the app
    run_command(
        adb_str,
        [
            "-s",
            device_id,
            "shell",
            "am",
            "start",
            "-n",
            &format!("{}/.MainActivity", artifact.bundle_id()),
        ],
    )
    .await
    .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

    // Wait for the process to start and get its PID
    let pid = wait_for_app_pid(adb_str, device_id, artifact.bundle_id()).await?;

    let adb_for_kill = adb.clone();
    let identifier_for_kill = device_id.to_string();
    let identifier_for_monitor = device_id.to_string();
    let bundle_id_for_kill = artifact.bundle_id().to_string();
    let bundle_id_for_monitor = artifact.bundle_id().to_string();
    let log_level = options.log_level();

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
    });

    // Spawn a background task to monitor the process
    let adb_for_monitor = adb.clone();
    let sender_for_monitor = sender.clone();
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

    // Spawn a background task to stream logs if log_level is set
    if let Some(level) = log_level {
        let adb_for_logs = adb;
        let identifier_for_logs = device_id.to_string();
        smol::spawn(async move {
            stream_android_logs(adb_for_logs, &identifier_for_logs, pid, level, sender).await;
        })
        .detach();
    }

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
            if let Ok(pid) = output.trim().parse::<u32>() {
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
            .and_then(|output| output.trim().parse::<u32>().ok())
            .is_some_and(|current_pid| current_pid == pid);

        if !still_running {
            // Process is no longer running - fetch crash logs
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

            let error_msg = if !crash_info.trim().is_empty() {
                format!(
                    "Process {} exited.\n\n=== Crash Log ===\n{}",
                    bundle_id, crash_info
                )
            } else {
                // Try to get more general logs
                let general_log =
                    run_command(adb_str, ["-s", device_id, "logcat", "-d", "-t", "50"])
                        .await
                        .unwrap_or_default();

                if !general_log.trim().is_empty() {
                    format!(
                        "Process {} exited.\n\n=== Recent Log ===\n{}",
                        bundle_id, general_log
                    )
                } else {
                    format!("Process {} exited unexpectedly.", bundle_id)
                }
            };

            let _ = sender.send(DeviceEvent::Crashed(error_msg)).await;
            break;
        }
    }
}

/// Stream logs from an Android process using logcat.
async fn stream_android_logs(
    adb: std::path::PathBuf,
    device_id: &str,
    pid: u32,
    level: LogLevel,
    sender: smol::channel::Sender<DeviceEvent>,
) {
    use futures::io::{AsyncBufReadExt, BufReader};
    use smol::process::Command;

    let priority = level.to_android_priority();

    // Build logcat command with PID filter and minimum priority
    // Format: `adb -s <device> logcat --pid=<pid> *:<priority>`
    let mut cmd = Command::new(&adb);
    cmd.args(["-s", device_id, "logcat", "--pid", &pid.to_string()])
        .arg(format!("*:{priority}"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to spawn logcat: {e}");
            return;
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return,
    };

    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    // Parse logcat output and send as DeviceEvent::Log
    // Logcat format: "MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG: message"
    use futures::StreamExt;
    while let Some(result) = lines.next().await {
        let line = match result {
            Ok(l) => l,
            Err(_) => break,
        };

        let (parsed_level, message) = parse_logcat_line(&line);

        let _ = sender
            .send(DeviceEvent::Log {
                level: parsed_level,
                message,
            })
            .await;
    }

    // Clean up child process
    let _ = child.kill();
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
        "I" => tracing::Level::INFO,
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
    if let Some(colon_pos) = after_level.find(": ") {
        let tag = after_level[..colon_pos].trim();
        let message = after_level[colon_pos + 2..].to_string();
        Some(LogcatParsed {
            level,
            tag: tag.to_string(),
            message,
        })
    } else {
        // No colon found, use everything as message
        Some(LogcatParsed {
            level,
            tag: "unknown".to_string(),
            message: after_level.to_string(),
        })
    }
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
    pub fn new(avd_name: String) -> Self {
        Self { avd_name }
    }

    /// Get the AVD name.
    #[must_use]
    pub fn avd_name(&self) -> &str {
        &self.avd_name
    }
}

impl Device for AndroidEmulator {
    type Platform = AndroidPlatform;

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

    fn platform(&self) -> Self::Platform {
        // Default to arm64 for emulators - most common architecture
        AndroidPlatform::arm64()
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        let identifier = find_emulator_identifier().await?;
        run_on_android(&identifier, artifact, options).await
    }
}
