
use crate::{
    apple::{device::AppleDevice, toolchain::AppleToolchain},
    platform::Platform,
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
        _options: crate::build::BuildOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        // RustBuild::new(path, triple)

        todo!()
    }

    fn toolchain(&self) -> &Self::Toolchain {
        todo!()
    }

    fn triple(&self) -> target_lexicon::Triple {
        todo!()
    }

    async fn clean(&self, _project: &crate::project::Project) -> color_eyre::eyre::Result<()> {
        // Clean build artifacts of a specific platform
        todo!()
    }

    async fn package(
        &self,
        _project: &crate::project::Project,
        _options: crate::platform::PackageOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        todo!()
    }
}
