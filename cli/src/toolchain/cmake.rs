use crate::toolchain::{Installation, Toolchain};

pub struct Cmake {}

impl Toolchain for Cmake {
    type Installation = CmakeInstallation;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        todo!()
    }

    async fn fix(&self) -> Result<Self::Installation, <Self::Installation as Installation>::Error> {
        todo!()
    }
}

pub struct CmakeInstallation {}

#[derive(Debug, thiserror::Error)]
pub enum FailToInstallCmake {}

impl Installation for CmakeInstallation {
    type Error = FailToInstallCmake;

    async fn install(&self) -> Result<(), Self::Error> {
        todo!()
    }
}
