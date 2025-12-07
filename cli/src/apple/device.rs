use crate::device::Device;

pub struct AppleDevice {}

impl Device for AppleDevice {
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
