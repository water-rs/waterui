use color_eyre::eyre;

use crate::{device::Device, project::Project, toolchain::Toolchain};

pub struct BuildOptions {}

pub struct PackageOptions {}

pub trait Platform {
    type Toolchain: Toolchain;
    type Device: Device;

    /// Clean build artifacts for this platform
    fn clean(&self, project: &Project) -> impl Future<Output = eyre::Result<()>> + Send;

    fn package(
        &self,
        project: &Project,
        options: &PackageOptions,
    ) -> impl Future<Output = eyre::Result<()>> + Send;

    fn toolchain(&self) -> &Self::Toolchain;

    fn scan(&self) -> impl Future<Output = eyre::Result<Vec<Self::Device>>> + Send;

    fn description(&self) -> String;
}
