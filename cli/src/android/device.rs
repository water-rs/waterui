
use color_eyre::eyre::{self, eyre};
use tracing::error;

use crate::{
    device::{
        Artifact, Device, DeviceKind, DeviceState, FailToRun, RunOptions, Running,
    },
    utils::run_command,
};

pub struct AndroidDevice {
    name: String,
    identifier: String,
    kind: DeviceKind,
    state: DeviceState,
}

impl Device for AndroidDevice {
    async fn launch(&self) -> eyre::Result<()> {
        run_command("adb", ["-s", &self.identifier, "wait-for-device"]).await?;

        Ok(())
    }

    async fn run(
        &self,
        artifact: Artifact,
        _options: RunOptions,
    ) -> Result<Running, crate::device::FailToRun> {
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
            let result = std::process::Command::new("adb")
                .args(["shell", "kill", &pid.to_string()])
                .output();

            if let Err(e) = result {
                error!("Failed to kill process {}: {}", pid, e);
            }
        });

        Ok(running)
    }
}

pub struct AndroidSimulator {
    pub name: String,
    pub id: String,
}
