use std::{collections::HashMap, path::PathBuf};

use color_eyre::eyre::{self, eyre};
use serde::Deserialize;
use smol::{future::block_on, process::Command, spawn};
use tracing::info;

use crate::{
    apple::platform::ApplePlatform,
    device::{Artifact, Device, DeviceEvent, FailToRun, Running},
    utils::run_command,
};

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

        info!("Launching app on MacOS: {}", artifact.path().display());

        // Build the `open` command
        // Note: We can't easily capture stdout/stderr with `open`, so we just launch the app
        // For debugging output, use Console.app or run the executable directly
        let mut command = Command::new("open");
        command
            .arg("-W") // Wait for app to exit
            .arg("-n"); // Open a new instance

        // Add environment variables
        for (key, value) in options.env_vars() {
            command.arg("--env").arg(format!("{key}={value}"));
        }

        command.arg(artifact_path);
        command.kill_on_drop(true);

        let (running, sender) = Running::new(|| {
            // no-op, since kill_on_drop is set
        });

        // Spawn the open command
        let child = command
            .spawn()
            .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        // Spawn a task to wait for the child to exit and send Exited event
        spawn(async move {
            let _ = child.output().await;
            let _ = sender.try_send(DeviceEvent::Exited);
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

        let content = run_command("xcrun", ["simctl", "list", "devices"]).await?;

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
        _options: crate::device::RunOptions,
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

        run_command(
            "xcrun",
            ["simctl", "launch", &self.udid, artifact.bundle_id()],
        )
        .await
        .map_err(|e| FailToRun::Launch(eyre!("Failed to launch app: {e}")))?;

        // Create a Running instance - termination will use simctl terminate
        let udid = self.udid.clone();
        let bundle_id = artifact.bundle_id().to_string();
        let (running, _sender) = Running::new(move || {
            // Terminate the app when Running is dropped
            // This runs synchronously in drop, so we use std::process::Command

            let fut = run_command("xcrun", ["simctl", "terminate", &udid, &bundle_id]);
            if let Err(err) = block_on(fut) {
                tracing::error!("Failed to terminate app on simulator: {err}");
            }
        });

        Ok(running)
    }
}
