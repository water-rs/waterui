//! Swift/Xcode toolchain implementation.

use smol::process::Command;
use which::which;

use crate::{
    toolchain::{Installation, Toolchain},
    utils::run_command,
};

// Tip: We cannot fix apple toolchain
pub enum AppleToolchain {
    MacOS,
    Ios,
    IosSimulator,
}

impl Toolchain for AppleToolchain {
    type Installation = AppleInstallation;
    fn name(&self) -> &'static str {
        "Apple Toolchain"
    }
    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError> {
        todo!()
    }

    async fn fix(&self) -> Result<Self::Installation, crate::toolchain::ToolchainError> {
        todo!()
    }
}

impl AppleToolchain {
    fn has_xcode_cli_tools() -> bool {
        which("xcode-select").is_ok()
    }

    fn has_swift() -> bool {
        which("swift").is_ok()
    }

    fn has_xcodebuild() -> bool {
        which("xcodebuild").is_ok()
    }

    fn has_cmake() -> bool {
        which("cmake").is_ok()
    }

    async fn check_sdk(&self) -> bool {
        let sdk = match self {
            AppleToolchain::MacOS => "macosx",
            AppleToolchain::Ios => "iphoneos",
            AppleToolchain::IosSimulator => "iphonesimulator",
        };

        let cmd = Command::new("xcrun").args(["--sdk", sdk, "--show-sdk-path"]);

        run_command(&mut cmd.clone()).await.is_ok()
    }
}

pub struct AppleInstallation {}

impl Installation for AppleInstallation {
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
