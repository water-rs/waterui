use crate::toolchain::Installation;

pub struct Brew {
    package_name: String,
}

impl Installation for Brew {
    async fn install(
        self,
        progress: crate::utils::task::Progress,
    ) -> Result<(), crate::toolchain::ToolchainError> {
        todo!()
    }

    fn description(&self) -> String {
        todo!()
    }
}
