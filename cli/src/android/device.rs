use color_eyre::eyre::{self, eyre};
use smol::{future::block_on, process::Command};
use tracing::error;

use crate::{
    android::{platform::AndroidPlatform, toolchain::AndroidSdk},
    device::{Artifact, Device, DeviceEvent, FailToRun, RunOptions, Running},
    utils::{command, run_command},
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

    async fn run(
        &self,
        artifact: Artifact,
        options: RunOptions,
    ) -> Result<Running, crate::device::FailToRun> {
        let adb = AndroidSdk::adb_path()
            .ok_or_else(|| FailToRun::Run(eyre!("Android SDK not found or adb not installed")))?;
        let adb_str = adb.to_str().unwrap();

        // Set environment variables as system properties
        // Android doesn't support direct environment variable passing, so we use
        // system properties with prefix "waterui.env." which the app reads at startup
        for (key, value) in options.env_vars() {
            let prop_key = format!("waterui.env.{key}");
            // Note: setprop requires root or adb shell access, but adb shell grants this
            let result = run_command(
                adb_str,
                ["-s", &self.identifier, "shell", "setprop", &prop_key, value],
            )
            .await;
            if let Err(e) = result {
                tracing::warn!("Failed to set system property {prop_key}: {e}");
            }
        }

        // Install the APK on the device
        run_command(
            adb_str,
            [
                "-s",
                &self.identifier,
                "install",
                artifact.path().to_str().unwrap(),
            ],
        )
        .await
        .map_err(|e| FailToRun::Install(eyre!("Failed to install APK: {e}")))?;

        run_command(
            adb_str,
            [
                "-s",
                &self.identifier,
                "shell",
                "am",
                "start",
                "-n",
                &format!("{}/.MainActivity", artifact.bundle_id()),
            ],
        )
        .await
        .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        let _identifier = self.identifier.clone();

        let _bundle_id = artifact.bundle_id().to_string();

        // Wait a moment for the process to start, then get its PID
        // Retry a few times since the process might not be immediately visible
        let mut pid = None;
        for _ in 0..10 {
            smol::Timer::after(std::time::Duration::from_millis(200)).await;
            if let Ok(output) = run_command(
                adb_str,
                [
                    "-s",
                    &self.identifier,
                    "shell",
                    "pidof",
                    artifact.bundle_id(),
                ],
            )
            .await
            {
                if let Ok(p) = output.trim().parse::<u32>() {
                    pid = Some(p);
                    break;
                }
            }
        }

        let pid = match pid {
            Some(p) => p,
            None => {
                // App likely crashed on startup - fetch logcat for crash info
                let crash_log = run_command(
                    adb_str,
                    [
                        "-s",
                        &self.identifier,
                        "logcat",
                        "-d",    // dump and exit
                        "-t",    // last N lines
                        "50",    // get last 50 lines
                        "--pid", // filter is not useful if process died
                        "0",     // so we use a broader filter below
                    ],
                )
                .await
                .ok();

                // Try to get crash-specific logs
                let crash_info = run_command(
                    adb_str,
                    [
                        "-s",
                        &self.identifier,
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

                let mut error_msg = format!(
                    "App {} crashed on startup (process not found).\n\n",
                    artifact.bundle_id()
                );

                if !crash_info.trim().is_empty() {
                    error_msg.push_str("=== Crash Log ===\n");
                    error_msg.push_str(&crash_info);
                } else if let Some(log) = crash_log {
                    error_msg.push_str("=== Recent Logcat ===\n");
                    error_msg.push_str(&log);
                }

                return Err(FailToRun::Launch(eyre!("{}", error_msg)));
            }
        };

        let adb_for_kill = adb.clone();
        let identifier_for_monitor = self.identifier.clone();
        let bundle_id_for_monitor = artifact.bundle_id().to_string();

        let (running, sender) = Running::new(move || {
            let result = block_on(async move {
                let mut cmd = Command::new(&adb_for_kill);
                command(cmd.args(["shell", "kill", &pid.to_string()]))
                    .output()
                    .await
            });

            if let Err(e) = result {
                error!("Failed to kill process {}: {}", pid, e);
            }
        });

        // Spawn a background task to monitor the process
        let adb_for_monitor = adb;
        smol::spawn(async move {
            monitor_android_process(
                adb_for_monitor,
                &identifier_for_monitor,
                &bundle_id_for_monitor,
                pid,
                sender,
            )
            .await;
        })
        .detach();

        Ok(running)
    }
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

        // Check if process is still running
        let result = run_command(
            adb_str,
            ["-s", device_id, "shell", "kill", "-0", &pid.to_string()],
        )
        .await;

        if result.is_err() {
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

    async fn run(
        &self,
        artifact: Artifact,
        options: RunOptions,
    ) -> Result<Running, crate::device::FailToRun> {
        let adb = AndroidSdk::adb_path()
            .ok_or_else(|| FailToRun::Run(eyre!("Android SDK not found or adb not installed")))?;
        let adb_str = adb.to_str().unwrap();

        // Find the running emulator identifier
        let output = run_command(adb_str, ["devices"])
            .await
            .map_err(|e| FailToRun::Run(eyre!("Failed to list devices: {e}")))?;

        let identifier = output
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
            .ok_or_else(|| FailToRun::Run(eyre!("Emulator not running")))?;

        // Set environment variables as system properties
        for (key, value) in options.env_vars() {
            let prop_key = format!("waterui.env.{key}");
            let result = run_command(
                adb_str,
                ["-s", &identifier, "shell", "setprop", &prop_key, value],
            )
            .await;
            if let Err(e) = result {
                tracing::warn!("Failed to set system property {prop_key}: {e}");
            }
        }

        // Install the APK on the emulator
        run_command(
            adb_str,
            [
                "-s",
                &identifier,
                "install",
                artifact.path().to_str().unwrap(),
            ],
        )
        .await
        .map_err(|e| FailToRun::Install(eyre!("Failed to install APK: {e}")))?;

        run_command(
            adb_str,
            [
                "-s",
                &identifier,
                "shell",
                "am",
                "start",
                "-n",
                &format!("{}/.MainActivity", artifact.bundle_id()),
            ],
        )
        .await
        .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        // Wait a moment for the process to start, then get its PID
        // Retry a few times since the process might not be immediately visible
        let mut pid = None;
        for _ in 0..10 {
            smol::Timer::after(std::time::Duration::from_millis(200)).await;
            if let Ok(output) = run_command(
                adb_str,
                ["-s", &identifier, "shell", "pidof", artifact.bundle_id()],
            )
            .await
            {
                if let Ok(p) = output.trim().parse::<u32>() {
                    pid = Some(p);
                    break;
                }
            }
        }

        let pid = match pid {
            Some(p) => p,
            None => {
                // App likely crashed on startup - fetch logcat for crash info
                let crash_info = run_command(
                    adb_str,
                    [
                        "-s",
                        &identifier,
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

                let mut error_msg = format!(
                    "App {} crashed on startup (process not found).\n\n",
                    artifact.bundle_id()
                );

                if !crash_info.trim().is_empty() {
                    error_msg.push_str("=== Crash Log ===\n");
                    error_msg.push_str(&crash_info);
                }

                return Err(FailToRun::Launch(eyre!("{}", error_msg)));
            }
        };

        let adb_for_kill = adb.clone();
        let identifier_for_kill = identifier.clone();
        let identifier_for_monitor = identifier;
        let bundle_id_for_monitor = artifact.bundle_id().to_string();

        let (running, sender) = Running::new(move || {
            let result = block_on(async move {
                let mut cmd = Command::new(&adb_for_kill);
                command(cmd.args([
                    "-s",
                    &identifier_for_kill,
                    "shell",
                    "kill",
                    &pid.to_string(),
                ]))
                .output()
                .await
            });

            if let Err(e) = result {
                error!("Failed to kill process {}: {}", pid, e);
            }
        });

        // Spawn a background task to monitor the process
        let adb_for_monitor = adb;
        smol::spawn(async move {
            monitor_android_process(
                adb_for_monitor,
                &identifier_for_monitor,
                &bundle_id_for_monitor,
                pid,
                sender,
            )
            .await;
        })
        .detach();

        Ok(running)
    }
}
