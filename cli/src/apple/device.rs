use std::{collections::HashMap, path::PathBuf, process::Stdio};

use color_eyre::eyre::{self, eyre};
use serde::Deserialize;
use smol::{
    Task,
    future::zip,
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    spawn,
    stream::StreamExt,
};
use target_lexicon::OperatingSystem;
use tracing::info;

use crate::{
    apple::platform::ApplePlatform,
    device::{Artifact, Device, DeviceEvent, FailToRun, Running},
    utils::run_command,
};

/// Represents an Apple device (simulator or physical)
#[derive(Debug)]
pub enum AppleDevice {
    /// An Apple Simulator device
    Simulator(AppleSimulator),

    Current(MacOS),
}

/// Local `MacOS` device (current physical machine)
#[derive(Debug)]
pub struct MacOS;

/// Represents a running application on an Apple device
///
/// Drop the `AppleRunning` to terminate the application
///
/// Using `log stream --predicate` to track logs and events
pub struct AppleRunning {
    app: Task<()>, // Drop the task to terminate the app
}

impl Drop for AppleRunning {
    fn drop(&mut self) {
        todo!()
    }
}

impl Device for MacOS {
    type Platform = ApplePlatform;
    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        // No need to launch anything for MacOS physical device
        // This is the current machine
        Ok(())
    }

    fn platform(&self) -> Self::Platform {
        todo!()
    }

    async fn run(
        &self,
        artifact: Artifact,
        options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        // No need to install, let's run it directly

        // `launchctl` and `open` require to set up environment variables for all GUI apps
        // So we execute the binary directly
        // However, some functionalities may be limited without going through `open`
        // For instance, URL schemes may not work properly
        // In the future, we may consider to support `open` with proper environment setup

        // Artifact must end with `.app` for MacOS
        let artifact_path = artifact.path();

        if artifact_path.extension().and_then(|e| e.to_str()) != Some("app") {
            return Err(FailToRun::InvalidArtifact);
        }

        let app_name = artifact_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(FailToRun::InvalidArtifact)?;

        info!("Launching app on MacOS: {}", artifact.path().display());
        let executable = artifact_path.join("Contents/MacOS").join(app_name);

        // Configure environment variables and launch the app
        let mut command = Command::new(executable);
        for (key, value) in options.env_vars() {
            command.env(key, value);
        }

        command.kill_on_drop(true);

        let (running, sender) = Running::new(|| {
            // no-op, since kill_on_drop is set
        });

        let mut child = command
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        let mut stdout =
            BufReader::new(child.stdout.take().expect("Failed to take stdout")).lines();

        let mut stderr =
            BufReader::new(child.stderr.take().expect("Failed to take stderr")).lines();

        spawn(async move {
            zip(
                {
                    let sender = sender.clone();
                    async move {
                        while let Ok(Some(line)) = stdout.try_next().await {
                            if sender
                                .try_send(DeviceEvent::Stdout { message: line })
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                },
                async move {
                    while let Ok(Some(line)) = stderr.try_next().await {
                        if sender
                            .try_send(DeviceEvent::Stderr { message: line })
                            .is_err()
                        {
                            break;
                        }
                    }
                },
            )
            .await;
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
        }
    }

    fn platform(&self) -> Self::Platform {
        match self {
            Self::Simulator(simulator) => simulator.platform(),
            Self::Current(mac_os) => mac_os.platform(),
        }
    }

    async fn run(
        &self,
        artifact: Artifact,
        options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        match self {
            Self::Simulator(simulator) => simulator.run(artifact, options).await,
            Self::Current(_mac_os) => todo!(),
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
    pub async fn scan() -> eyre::Result<Vec<Self>> {
        let content = run_command("xcrun", ["simctl", "list", "devices"]).await?;

        #[derive(Deserialize)]
        struct Root {
            devices: HashMap<String, Vec<AppleSimulator>>,
        }

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
    /// Launch the Apple device
    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        run_command("xcrun", ["simctl", "boot", &self.udid]).await?;
        Ok(())
    }

    fn platform(&self) -> Self::Platform {
        // Simulator must have a same architecture triple as the host machine
        let host_triple = target_lexicon::Triple::host();

        // it looks like this: com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro
        // So we have to use keyword matching to determine the OS, quite hacky but works for now
        let device_type_id = &self.device_type_identifier;

        todo!()
    }

    /// Run an artifact on the Apple device
    ///
    /// Please lanuch the device before calling this method
    async fn run(
        &self,
        artifact: Artifact,
        _options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        // Fail to run an artifact on an Apple device

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
        .map_err(|e| crate::device::FailToRun::Install(eyre!("Failed to install app: {e}")))?;

        info!("Launching app on apple simulator {}", self.name);

        run_command(
            "xcrun",
            ["simctl", "launch", &self.udid, artifact.bundle_id()],
        )
        .await
        .map_err(|e| crate::device::FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        let (running, _sender) = Running::new(|| todo!());

        // TODO: Track the launched app process and return a Running handler
        Ok(running)
    }
}
