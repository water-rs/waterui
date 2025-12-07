use color_eyre::eyre;
use target_lexicon::Triple;

use crate::{
    android::platform::AndroidPlatform,
    device::{Device, DeviceKind, DeviceState},
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
        artifact: &std::path::Path,
        options: crate::device::RunOptions,
    ) -> Result<crate::device::Running, crate::device::FailToRun> {
        todo!()
    }
}

pub struct AndroidSimulator {
    pub name: String,
    pub id: String,
}
