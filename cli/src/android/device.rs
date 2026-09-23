use color_eyre::eyre::{self, eyre};
use smol::channel::{Receiver, Sender};
use smol::io::AsyncWriteExt;
use smol::process::Command;
use smol::spawn;
use tracing::{debug, error};

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use crate::{
    android::platform::AndroidAbi,
    android::toolchain::AndroidSdk,
    device::{
        ApplicationExit, Artifact, Device, DeviceEvent, FailToRun, LogLevel, RunOptions, Running,
    },
    utils::{parse_whitespace_separated_u32s, run_command_os, run_command_output_os},
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
    abi: AndroidAbi,
}

impl AndroidDevice {
    /// Create a new Android device with the given identifier and ABI.
    #[must_use]
    pub const fn new(identifier: String, abi: AndroidAbi) -> Self {
        Self { identifier, abi }
    }

    /// Get the device identifier.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Get the device's primary ABI.
    #[must_use]
    pub const fn abi(&self) -> AndroidAbi {
        self.abi
    }
}

impl Device for AndroidDevice {
    fn name(&self) -> &str {
        &self.identifier
    }

    async fn launch(&self) -> eyre::Result<()> {
        let adb = AndroidSdk::adb_path()
            .ok_or_else(|| eyre::eyre!("Android SDK not found or adb not installed"))?;
        run_command_os(&adb, ["-s", &self.identifier, "wait-for-device"]).await?;
        Ok(())
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        run_on_android(&self.identifier, artifact, options).await
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        let adb = AndroidSdk::adb_path()
            .ok_or_else(|| eyre::eyre!("Android SDK not found or adb not installed"))?;
        Self::scan_with_adb(&adb).await
    }
}

impl AndroidDevice {
    async fn scan_with_adb(adb: &Path) -> eyre::Result<Vec<Self>> {
        let output = run_command_os(adb, ["devices", "-l"])
            .await
            .map_err(|e| eyre!("Failed to list devices: {e}"))?;

        let mut devices = Vec::new();

        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "device" {
                let identifier = parts[0].to_string();

                // Get device ABI
                let abi = run_command_os(
                    adb,
                    ["-s", &identifier, "shell", "getprop", "ro.product.cpu.abi"],
                )
                .await
                .map_err(|e| eyre!("Failed to get device ABI: {e}"))?;
                let abi = abi
                    .trim()
                    .parse::<AndroidAbi>()
                    .map_err(|e| eyre!("Unsupported device ABI: {e}"))?;

                devices.push(Self::new(identifier, abi));
            }
        }

        Ok(devices)
    }
}

/// Provides the Android ABI required to build/package for a target.
pub trait AndroidAbiProvider {
    /// Return the device ABI used for building and packaging.
    fn android_abi(&self) -> AndroidAbi;
}

impl AndroidAbiProvider for AndroidDevice {
    fn android_abi(&self) -> AndroidAbi {
        self.abi()
    }
}

impl AndroidAbiProvider for AndroidEmulator {
    fn android_abi(&self) -> AndroidAbi {
        self.expected_abi()
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
async fn run_on_android(
    device_id: &str,
    artifact: Artifact,
    options: RunOptions,
) -> Result<Running, FailToRun> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| FailToRun::Run(eyre!("Android SDK not found or adb not installed")))?;
    let env_vars = collect_android_env_vars(&options);

    install_android_artifact(&adb, device_id, artifact.path()).await?;
    launch_android_app(
        &adb,
        build_android_start_args(device_id, &artifact, &env_vars),
    )
    .await?;

    // Wait for the process to start and get its PID
    let pid = wait_for_app_pid(&adb, device_id, artifact.bundle_id()).await?;

    let adb_for_kill = adb.clone();
    let device_id_for_kill = device_id.to_string();
    let device_id_for_monitor = device_id.to_string();
    let bundle_id_for_kill = artifact.bundle_id().to_string();
    let bundle_id_for_monitor = artifact.bundle_id().to_string();
    let log_level = options.log_level();

    let (running, sender) = Running::new(move || {
        spawn_android_force_stop(adb_for_kill, device_id_for_kill, bundle_id_for_kill);
    });

    spawn_android_runtime_tasks(
        &adb,
        device_id,
        device_id_for_monitor,
        bundle_id_for_monitor,
        pid,
        log_level,
        sender,
    );

    Ok(running)
}

fn collect_android_env_vars(options: &RunOptions) -> Vec<(String, String)> {
    options
        .env_vars()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

async fn install_android_artifact(
    adb: &Path,
    device_id: &str,
    artifact_path: &Path,
) -> Result<(), FailToRun> {
    let install_output = Command::new(adb)
        .args(["-s", device_id, "install", "-r"])
        .arg(artifact_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| FailToRun::Install(eyre!("Failed to install APK: {error}")))?;

    if install_output.status.success() {
        return Ok(());
    }

    Err(FailToRun::Install(eyre!(
        "Failed to install APK:\n{}\n{}",
        String::from_utf8_lossy(&install_output.stdout).trim(),
        String::from_utf8_lossy(&install_output.stderr).trim(),
    )))
}

fn build_android_start_args(
    device_id: &str,
    artifact: &Artifact,
    env_vars: &[(String, String)],
) -> Vec<String> {
    let mut start_args = vec![
        "-s".to_string(),
        device_id.to_string(),
        "shell".to_string(),
        "am".to_string(),
        "start".to_string(),
        "-S".to_string(),
        "-n".to_string(),
        format!("{}/.MainActivity", artifact.bundle_id()),
    ];

    for (key, value) in env_vars {
        start_args.push("--es".to_string());
        start_args.push(format!("waterui.env.{key}"));
        start_args.push(value.clone());
    }

    start_args
}

async fn launch_android_app(adb: &Path, start_args: Vec<String>) -> Result<(), FailToRun> {
    let output = Command::new(adb)
        .args(&start_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| FailToRun::Launch(eyre!("Failed to launch app: {error}")))?;

    if output.status.success() {
        return Ok(());
    }

    Err(FailToRun::Launch(eyre!(
        "Failed to launch app:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim(),
    )))
}

fn spawn_android_force_stop(adb: PathBuf, device_id: String, bundle_id: String) {
    let spawn_result = std::thread::Builder::new()
        .name("waterui-android-force-stop".to_string())
        .spawn(move || {
            let result = std::process::Command::new(&adb)
                .args(["-s", &device_id, "shell", "am", "force-stop", &bundle_id])
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
                Err(error) => {
                    error!("Failed to stop app {}: {}", bundle_id, error);
                }
            }
        });

    if let Err(error) = spawn_result {
        error!("Failed to spawn force-stop worker thread: {error}");
    }
}

fn spawn_android_runtime_tasks(
    adb: &Path,
    device_id: &str,
    device_id_for_monitor: String,
    bundle_id_for_monitor: String,
    pid: u32,
    log_level: Option<LogLevel>,
    sender: Sender<DeviceEvent>,
) {
    let sender_for_monitor = sender.clone();
    let sender_for_panic = sender.clone();
    let sender_for_logs = sender;
    let adb_for_monitor = adb.to_path_buf();

    smol::spawn(async move {
        monitor_android_process(
            adb_for_monitor,
            &device_id_for_monitor,
            &bundle_id_for_monitor,
            pid,
            sender_for_monitor,
        )
        .await;
    })
    .detach();

    let panic_rx = start_android_log_stream(adb, device_id, pid, log_level, sender_for_logs);
    spawn(async move {
        if let Ok(info) = panic_rx.recv().await {
            let _ = sender_for_panic
                .send(DeviceEvent::Crashed(format_android_panic(&info)))
                .await;
        }
    })
    .detach();
}

fn format_android_panic(info: &PanicInfo) -> String {
    let mut msg = format!("Panic: {}", info.payload);
    if let Some(location) = &info.location {
        msg.push('\n');
        msg.push_str("  at ");
        msg.push_str(location);
    }
    msg
}

/// Wait for an app to start and return its PID.
async fn wait_for_app_pid(adb: &Path, device_id: &str, bundle_id: &str) -> Result<u32, FailToRun> {
    for _ in 0..10 {
        smol::Timer::after(std::time::Duration::from_millis(200)).await;
        if let Ok(output) =
            run_command_os(adb, ["-s", device_id, "shell", "pidof", bundle_id]).await
            && let Some(pid) = parse_whitespace_separated_u32s(&output).into_iter().next()
        {
            return Ok(pid);
        }
    }

    // App likely crashed on startup - fetch logcat for crash info
    let crash_info = match run_command_os(
        adb,
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
    {
        Ok(output) => output,
        Err(err) => format!("(failed to collect logcat crash info: {err})"),
    };

    let mut error_msg = format!("App {bundle_id} crashed on startup (process not found).\n\n");

    if !crash_info.trim().is_empty() {
        error_msg.push_str("=== Crash Log ===\n");
        error_msg.push_str(&crash_info);
    }

    Err(FailToRun::Launch(eyre!("{}", error_msg)))
}

/// Query the AVD name for a running emulator device via `adb emu avd name`.
///
/// # Errors
/// Returns an error if the device isn't an emulator or adb doesn't return a name.
pub async fn emulator_avd_name_with_adb(adb: &Path, emulator_id: &str) -> eyre::Result<String> {
    if !emulator_id.starts_with("emulator-") {
        eyre::bail!("Not an Android emulator identifier: {emulator_id}");
    }

    let output = run_command_os(adb, ["-s", emulator_id, "emu", "avd", "name"]).await?;
    let name = output.lines().next().unwrap_or_default().trim();
    if name.is_empty() {
        eyre::bail!("Failed to query AVD name for {emulator_id}: empty response");
    }
    Ok(name.to_string())
}

/// Resolve the AVD name for a running emulator device.
///
/// # Errors
/// Returns an error if adb isn't available or the emulator doesn't return a name.
pub async fn emulator_avd_name(emulator_id: &str) -> eyre::Result<String> {
    let adb = AndroidSdk::adb_path()
        .ok_or_else(|| eyre!("Android SDK not found or adb not installed"))?;
    emulator_avd_name_with_adb(&adb, emulator_id).await
}

async fn try_find_running_emulator_for_avd(
    adb: &Path,
    avd_name: &str,
) -> eyre::Result<Option<AndroidDevice>> {
    let devices = AndroidDevice::scan_with_adb(adb).await?;

    for device in devices {
        let id = device.identifier();
        if !id.starts_with("emulator-") {
            continue;
        }

        match emulator_avd_name_with_adb(adb, id).await {
            Ok(name) if name == avd_name => return Ok(Some(device)),
            Ok(_) => {}
            Err(e) => {
                // Some emulator builds may not respond until fully booted.
                debug!("Failed to query AVD name for {id}: {e}");
            }
        }
    }

    Ok(None)
}

async fn adb_emulator_states(adb: &Path) -> eyre::Result<String> {
    let output = run_command_os(adb, ["devices", "-l"]).await?;
    let states: Vec<String> = output
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| line.starts_with("emulator-"))
        .map(ToOwned::to_owned)
        .collect();

    if states.is_empty() {
        return Ok(String::new());
    }

    Ok(states.join("; "))
}

async fn adb_emulator_boot_completed(adb: &Path, emulator_id: &str) -> bool {
    run_command_os(
        adb,
        ["-s", emulator_id, "shell", "getprop", "sys.boot_completed"],
    )
    .await
    .is_ok_and(|value| value.trim() == "1")
}

async fn adb_package_manager_ready(adb: &Path, emulator_id: &str) -> bool {
    run_command_os(adb, ["-s", emulator_id, "shell", "pm", "path", "android"])
        .await
        .is_ok_and(|output| {
            output
                .lines()
                .any(|line| line.trim().starts_with("package:"))
        })
}

/// Monitor an Android process and send events when it crashes or exits.
async fn monitor_android_process(
    adb: PathBuf,
    device_id: &str,
    bundle_id: &str,
    pid: u32,
    sender: smol::channel::Sender<DeviceEvent>,
) {
    // Check process status periodically
    loop {
        smol::Timer::after(std::time::Duration::from_secs(1)).await;

        // Check if process is still running using pidof
        // Note: We use pidof instead of kill -0 because kill -0 returns "Operation not permitted"
        // when the shell user doesn't have permission to send signals to the app process
        let output = match run_command_os(&adb, ["-s", device_id, "shell", "pidof", bundle_id])
            .await
        {
            Ok(output) => output,
            Err(err) => {
                debug!(
                    "Failed to query process state via pidof for {bundle_id} on {device_id}: {err}"
                );
                continue;
            }
        };

        // Check if the process with the same PID is still running.
        let still_running = parse_whitespace_separated_u32s(&output).contains(&pid);

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
            let pid_log = run_command_output_os(
                &adb,
                pid_log_args
                    .iter()
                    .map(|s| std::ffi::OsStr::new(s.as_str())),
            )
            .await
            .map_or_else(
                |err| {
                    debug!("Failed to fetch PID-filtered logcat: {err}");
                    String::new()
                },
                |output| {
                    if output.status.success() {
                        String::from_utf8_lossy(&output.stdout).to_string()
                    } else {
                        debug!("PID-filtered logcat exited with status {}", output.status);
                        String::new()
                    }
                },
            );

            assert!(
                !(pid_log.trim().is_empty()),
                "PID-filtered logcat returned empty output for {bundle_id} (pid={pid}); --pid support is required"
            );

            if android_log_looks_like_crash(&pid_log, bundle_id, pid) {
                let crash_log = pid_log;

                let error_msg = if crash_log.trim().is_empty() {
                    format!("Process {bundle_id} crashed.")
                } else {
                    format!("Process {bundle_id} crashed.\n\n=== Crash Log ===\n{crash_log}")
                };

                let _ = sender.send(DeviceEvent::Crashed(error_msg)).await;
            } else {
                let _ = sender
                    .send(DeviceEvent::Exited(ApplicationExit::user_closed()))
                    .await;
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

fn android_log_looks_like_crash(log: &str, bundle_id: &str, pid: u32) -> bool {
    if log.trim().is_empty() {
        return false;
    }

    let relevant = log.contains(bundle_id) || log_mentions_pid(log, pid);
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

    // Ensure log lines mention this app before applying AndroidRuntime heuristics.
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
    adb: &Path,
    device_id: &str,
    pid: u32,
    log_level: Option<LogLevel>,
    sender: Sender<DeviceEvent>,
) -> Receiver<PanicInfo> {
    use futures::StreamExt;
    use futures::io::{AsyncBufReadExt, BufReader};

    // Bounded channel with capacity 1 acts as oneshot - only first panic is captured
    let (panic_tx, panic_rx) = smol::channel::bounded::<PanicInfo>(1);

    // Always stream at fatal level to capture panics, even if user didn't request logs
    let priority = log_level.map_or('F', super::super::device::LogLevel::to_android_priority);

    // Build logcat command with PID filter and minimum priority
    let pid_arg = format!("--pid={pid}");
    let mut cmd = Command::new(adb);
    cmd.args(["-s", device_id, "logcat", "-v", "threadtime"])
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
            if line.contains("panic.payload=")
                && let Some(info) = extract_panic_info_from_log(&line)
            {
                let _ = panic_tx.try_send(info);
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
    expected_abi: AndroidAbi,
    device: OnceLock<AndroidDevice>,
}

impl AndroidEmulator {
    /// Open an Android emulator definition by AVD name (reads config to determine ABI).
    ///
    /// # Errors
    /// Returns an error if the AVD config is missing or malformed.
    pub async fn open(avd_name: String) -> eyre::Result<Self> {
        let expected_abi = read_avd_abi(&avd_name).await?;
        Ok(Self {
            avd_name,
            expected_abi,
            device: OnceLock::new(),
        })
    }

    /// Get the AVD name.
    #[must_use]
    pub fn avd_name(&self) -> &str {
        &self.avd_name
    }

    #[must_use]
    /// ABI expected for this AVD (from its config).
    pub const fn expected_abi(&self) -> AndroidAbi {
        self.expected_abi
    }
}

impl Device for AndroidEmulator {
    fn name(&self) -> &str {
        &self.avd_name
    }

    async fn launch(&self) -> eyre::Result<()> {
        let emulator_path =
            AndroidSdk::emulator_path().ok_or_else(|| eyre::eyre!("Android emulator not found"))?;

        // Start the emulator process (don't wait for it here, we'll poll for readiness).
        // Use std::process::Command so we can isolate process-group behavior.
        let mut emulator_cmd = std::process::Command::new(&emulator_path);
        emulator_cmd
            .arg("-avd")
            .arg(&self.avd_name)
            .arg("-no-snapshot-load")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            // Move emulator into its own process group so Ctrl+C in `water run`
            // only stops the CLI/app and doesn't terminate the emulator process.
            emulator_cmd.process_group(0);
        }

        let mut emulator_process = smol::unblock(move || emulator_cmd.spawn()).await?;

        // Wait for the emulator to boot (and match the requested AVD).
        let adb_path = AndroidSdk::adb_path()
            .ok_or_else(|| eyre::eyre!("Android SDK not found or adb not installed"))?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(300);
        let mut last_emulator_states = String::new();

        loop {
            if let Some(status) = emulator_process.try_wait()? {
                eyre::bail!(
                    "Emulator process exited before becoming ready (status: {status}). Check AVD configuration and run `emulator -avd {}` manually for details.",
                    self.avd_name
                );
            }

            if start.elapsed() > timeout {
                let states = if last_emulator_states.is_empty() {
                    "no emulator device reported by adb".to_string()
                } else {
                    last_emulator_states.clone()
                };

                eyre::bail!(
                    "Emulator launch timed out after 300 seconds (ADB state: {}).",
                    states
                );
            }

            last_emulator_states = match adb_emulator_states(&adb_path).await {
                Ok(states) => states,
                Err(err) => format!("failed to query emulator state via adb: {err}"),
            };

            if let Some(device) =
                try_find_running_emulator_for_avd(&adb_path, &self.avd_name).await?
            {
                if device.abi() != self.expected_abi {
                    eyre::bail!(
                        "AVD '{}' expected ABI {}, but running emulator reports {}",
                        self.avd_name,
                        self.expected_abi.as_str(),
                        device.abi().as_str()
                    );
                }

                let emulator_id = device.identifier().to_string();
                let boot_completed = adb_emulator_boot_completed(&adb_path, &emulator_id).await;
                let package_ready = adb_package_manager_ready(&adb_path, &emulator_id).await;

                if !boot_completed || !package_ready {
                    debug!(
                        "Emulator {} detected but not fully ready yet (boot_completed={}, package_ready={})",
                        emulator_id, boot_completed, package_ready
                    );
                    smol::Timer::after(std::time::Duration::from_secs(2)).await;
                    continue;
                }

                self.device
                    .set(device)
                    .map_err(|_| eyre::eyre!("Emulator device already initialized"))?;
                return Ok(());
            }

            smol::Timer::after(std::time::Duration::from_secs(2)).await;
        }
    }

    async fn run(&self, artifact: Artifact, options: RunOptions) -> Result<Running, FailToRun> {
        let device = self.device.get().ok_or_else(|| {
            FailToRun::Run(eyre!(
                "Android emulator '{}' is not launched. Launch it before running.",
                self.avd_name
            ))
        })?;
        run_on_android(device.identifier(), artifact, options).await
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        // List available AVDs using avdmanager or emulator -list-avds
        let emulator_path =
            AndroidSdk::emulator_path().ok_or_else(|| eyre::eyre!("Android emulator not found"))?;

        let output = Command::new(&emulator_path)
            .arg("-list-avds")
            .output()
            .await
            .map_err(|e| eyre!("Failed to list AVDs: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eyre::bail!("Failed to list AVDs: {}", stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut avds = Vec::new();
        for name in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
            avds.push(Self::open(name.to_string()).await?);
        }

        Ok(avds)
    }
}

async fn read_avd_abi(avd_name: &str) -> eyre::Result<AndroidAbi> {
    let home = dirs::home_dir().ok_or_else(|| eyre!("Failed to resolve home directory"))?;
    let config_path = home
        .join(".android/avd")
        .join(format!("{avd_name}.avd"))
        .join("config.ini");

    let content = smol::fs::read_to_string(&config_path)
        .await
        .map_err(|e| eyre!("Failed to read AVD config {}: {e}", config_path.display()))?;

    let abi_value = content
        .lines()
        .filter_map(|line| line.split_once('=').map(|(k, v)| (k.trim(), v.trim())))
        .find_map(|(k, v)| (k == "abi.type").then_some(v))
        .ok_or_else(|| eyre!("AVD config {} missing key abi.type", config_path.display()))?;

    abi_value
        .parse::<AndroidAbi>()
        .map_err(|e| eyre!("Unsupported AVD ABI '{abi_value}': {e}"))
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

    run_command_os(
        &adb,
        [
            "-s",
            device_id,
            "shell",
            "input",
            "tap",
            &x.to_string(),
            &y.to_string(),
        ],
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

    let mut args = vec!["-s", device_id, "shell", "input", "swipe"];

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

    run_command_os(&adb, args).await?;

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

    run_command_os(&adb, ["-s", device_id, "shell", "input", "text", &escaped]).await?;

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
    run_command_os(
        &adb,
        ["-s", device_id, "shell", "uiautomator", "dump", dump_path],
    )
    .await?;

    // Read the dump file
    let output = Command::new(&adb)
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
    let _ = run_command_os(&adb, ["-s", device_id, "shell", "rm", dump_path]).await;

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
                    let lt: Vec<i32> = parts[0].split(',').filter_map(|s| s.parse().ok()).collect();
                    let rb: Vec<i32> = parts[1].split(',').filter_map(|s| s.parse().ok()).collect();
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
                        element.insert(
                            key.to_string(),
                            serde_json::Value::String(value.to_string()),
                        );
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
            28184
        ));
    }

    #[test]
    fn detects_native_crash_when_pid_is_mentioned() {
        let log = "I DEBUG : Fatal signal 11 (SIGSEGV), code 1, fault addr 0x0 in tid 1 (main) pid: 28184\n";
        assert!(android_log_looks_like_crash(log, "com.example.app", 28184));
    }

    #[test]
    fn detects_java_crash_for_app() {
        let log = "E AndroidRuntime: FATAL EXCEPTION: main\nE AndroidRuntime: Process: com.example.app, PID: 28184\n";
        assert!(android_log_looks_like_crash(log, "com.example.app", 28184));
    }
}
