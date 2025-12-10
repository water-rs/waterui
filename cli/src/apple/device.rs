use std::{collections::HashMap, path::PathBuf, process::ExitStatus};

use color_eyre::eyre::{self, eyre};
use serde::Deserialize;
use smol::{
    channel::Sender,
    future::block_on,
    io::{AsyncBufReadExt, BufReader},
    process::{Command, Stdio},
    spawn,
    stream::StreamExt,
};
use tracing::info;

use crate::{
    apple::platform::ApplePlatform,
    device::{Artifact, Device, DeviceEvent, FailToRun, LogLevel, Running},
    utils::{command, run_command},
};

/// Start streaming logs from an Apple device process.
///
/// This works for both macOS apps and iOS simulators by using the `log stream` command.
/// For simulators, use `--predicate` with the subsystem; for macOS, use `--process` with PID.
///
/// If `log_level` is `None`, no log streaming is started.
fn start_log_stream(sender: Sender<DeviceEvent>, args: Vec<String>, log_level: Option<LogLevel>) {
    let Some(level) = log_level else {
        return;
    };

    let mut log_cmd = Command::new("log");
    log_cmd
        .arg("stream")
        .args(&args)
        .arg("--level")
        .arg(level.to_apple_level())
        .arg("--style")
        .arg("compact")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    if let Ok(mut log_child) = log_cmd.spawn() {
        if let Some(stdout) = log_child.stdout.take() {
            spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Some(Ok(line)) = lines.next().await {
                    // Parse log level from the line (compact format: timestamp level ...)
                    let level = if line.contains(" Error ") || line.contains(" Fault ") {
                        tracing::Level::ERROR
                    } else if line.contains(" Warning ") {
                        tracing::Level::WARN
                    } else if line.contains(" Debug ") {
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
            })
            .detach();
        }
    }
}

/// Handle exit status and send appropriate event.
///
/// This is shared between macOS and iOS simulator to ensure consistent crash detection.
fn handle_exit_status(exit_status: ExitStatus, sender: &Sender<DeviceEvent>) {
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
        cmd.arg("-W") // Wait for app to exit
            .arg("-n"); // Open a new instance

        // Add environment variables
        for (key, value) in options.env_vars() {
            cmd.arg("--env").arg(format!("{key}={value}"));
        }

        cmd.arg(artifact_path);
        cmd.kill_on_drop(true);

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
        let (running, sender) = Running::new(move || {
            // pkill the app by name to ensure it's terminated
            if pid_for_termination.is_some() {
                let _ = std::process::Command::new("pkill")
                    .arg("-x")
                    .arg(&app_name)
                    .status();
            }
        });

        // Start log streaming if we got a PID and log level is set
        if let Some(pid) = app_pid {
            start_log_stream(
                sender.clone(),
                vec!["--process".to_string(), pid.to_string()],
                options.log_level(),
            );
        }

        // Spawn a task to wait for the app to exit and detect crashes
        spawn(async move {
            match child.status().await {
                Ok(exit_status) => handle_exit_status(exit_status, &sender),
                Err(e) => {
                    let _ = sender.try_send(DeviceEvent::Crashed(format!("Failed to wait: {e}")));
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

        // Start log streaming if we got a PID and log level is set
        if let Some(pid) = app_pid {
            start_log_stream(
                sender.clone(),
                vec!["--process".to_string(), pid.to_string()],
                options.log_level(),
            );
        }

        // Spawn a task to wait for the app to exit
        spawn(async move {
            match child.status().await {
                Ok(exit_status) => handle_exit_status(exit_status, &sender),
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
