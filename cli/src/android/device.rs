use target_lexicon::Triple;

use crate::{android::platform::AndroidPlatform, device::Device};

pub struct AndroidDevice {}

impl Device for AndroidDevice {
    async fn launch(&self) -> color_eyre::eyre::Result<()> {
        todo!()
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
