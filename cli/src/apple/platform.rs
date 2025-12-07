use crate::{
    apple::{device::AppleDevice, toolchain::AppleToolchain},
    platform::Platform,
    toolchain::Toolchain,
};

pub struct ApplePlatform {}

impl Platform for ApplePlatform {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;
    async fn scan(&self) -> color_eyre::eyre::Result<Vec<Self::Device>> {
        todo!()
    }

    async fn build(
        &self,
        options: crate::build::BuildOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        todo!()
    }

    fn toolchain(&self) -> &Self::Toolchain {
        todo!()
    }

    fn triple(&self) -> target_lexicon::Triple {
        todo!()
    }

    async fn clean(&self, project: &crate::project::Project) -> color_eyre::eyre::Result<()> {
        // Clean build artifacts of a specific platform
        todo!()
    }

    async fn package(
        &self,
        project: &crate::project::Project,
        options: crate::platform::PackageOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        todo!()
    }
}
