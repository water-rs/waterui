use target_lexicon::Triple;

use crate::{
    android::{device::AndroidDevice, toolchain::AndroidToolchain},
    platform::Platform,
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

    fn toolchain(&self) -> &Self::Toolchain {
        todo!()
    }

    async fn clean(&self, _project: &crate::project::Project) -> color_eyre::eyre::Result<()> {
        todo!()
    }

    async fn build(
        &self,
        _options: crate::build::BuildOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        //RustBuild::new(path, triple)
        todo!()
    }

    fn triple(&self) -> target_lexicon::Triple {
        self.triple.clone()
    }

    async fn package(
        &self,
        _project: &crate::project::Project,
        _options: crate::platform::PackageOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        todo!()
    }
}
