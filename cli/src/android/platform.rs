use target_lexicon::Triple;

use crate::{
    android::{device::AndroidDevice, toolchain::AndroidToolchain},
    build::BuildOptions,
    device::Artifact,
    platform::Platform,
    project::Project,
};

pub struct AndroidPlatform {
    triple: Triple,
}

impl AndroidPlatform {
    #[must_use]
    pub const fn new(triple: Triple) -> Self {
        Self { triple }
    }
}

impl Platform for AndroidPlatform {
    type Device = AndroidDevice;
    type Toolchain = AndroidToolchain;
    async fn scan(&self) -> color_eyre::eyre::Result<Vec<Self::Device>> {
        todo!()
    }

    fn toolchain(&self) -> Self::Toolchain {
        AndroidToolchain::default()
    }

    async fn clean(&self, _project: &crate::project::Project) -> color_eyre::eyre::Result<()> {
        todo!()
    }

    async fn build(
        &self,
        _project: &Project,
        _options: BuildOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        todo!()
    }

    fn triple(&self) -> target_lexicon::Triple {
        self.triple.clone()
    }

    async fn package(
        &self,
        _project: &Project,
        _options: crate::platform::PackageOptions,
    ) -> color_eyre::eyre::Result<Artifact> {
        todo!()
    }
}
