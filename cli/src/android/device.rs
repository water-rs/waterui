use color_eyre::eyre::{self, eyre};
use smol::{future::block_on, process::Command};
use tracing::error;

use crate::{
    android::platform::AndroidPlatform,
    device::{Artifact, Device, FailToRun, RunOptions, Running},
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
        run_command("adb", ["-s", &self.identifier, "wait-for-device"]).await?;
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
        // Set environment variables as system properties
        // Android doesn't support direct environment variable passing, so we use
        // system properties with prefix "waterui.env." which the app reads at startup
        for (key, value) in options.env_vars() {
            let prop_key = format!("waterui.env.{key}");
            // Note: setprop requires root or adb shell access, but adb shell grants this
            let result = run_command(
                "adb",
                ["-s", &self.identifier, "shell", "setprop", &prop_key, value],
            )
            .await;
            if let Err(e) = result {
                tracing::warn!("Failed to set system property {prop_key}: {e}");
            }
        }

        // Install the APK on the device
        run_command(
            "adb",
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
            "adb",
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

        let pid = run_command(
            "adb",
            [
                "-s",
                &self.identifier,
                "shell",
                "pidof",
                artifact.bundle_id(),
            ],
        )
        .await
        .map_err(|e| FailToRun::Launch(eyre!("Failed to get PID: {e}")))?;

        let pid = pid
            .trim()
            .parse::<u32>()
            .map_err(|e| FailToRun::Launch(eyre!("Failed to parse PID from adb output: {e}")))?;

        let (running, _sender) = Running::new(move || {
            let result = block_on(async move {
                let mut cmd = Command::new("adb");
                command(cmd.args(["shell", "kill", &pid.to_string()]))
                    .output()
                    .await
            });

            if let Err(e) = result {
                error!("Failed to kill process {}: {}", pid, e);
            }
        });

        Ok(running)
    }
}

/// Android emulator/simulator device.
#[derive(Debug)]
pub struct AndroidSimulator {
    /// Name of the simulator.
    pub name: String,
    /// Unique identifier.
    pub id: String,
}
