use std::convert::Infallible;

use serde::{Deserialize, Serialize};

use crate::{
    toolchain::{Toolchain, ToolchainError},
    utils::{run_command, which},
};

pub type AppleToolchain = (Xcode, AppleSdk);

pub struct Xcode;

impl Toolchain for Xcode {
    type Installation = Infallible;
    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if Xcode is installed and available
        if which("xcodebuild").await && which("xcode-select").await {
            Ok(())
        } else {
            Err(ToolchainError::unfixable(
                "Xcode is not installed or not found in PATH",
                "Please install Xcode from the App Store or the Apple Developer website and ensure it's available in your PATH.",
            ))
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum AppleSdk {
    #[serde(rename = "iOS")]
    Ios,
    #[serde(rename = "MacOS")]
    Macos,
    // TODO: more SDKs
}

impl AppleSdk {
    pub fn sdk_name(&self) -> &str {
        match self {
            Self::Ios => "iphoneos",
            Self::Macos => "macosx",
        }
    }
}

impl std::fmt::Display for AppleSdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        serde_json::to_value(self).unwrap().as_str().unwrap().fmt(f)
    }
}

impl Toolchain for AppleSdk {
    type Installation = Infallible;
    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if the required Apple SDK is available
        let result = run_command("xcrun", &["--sdk", self.sdk_name(), "--show-sdk-path"]).await;

        if result.is_err() {
            return Err(ToolchainError::unfixable(
                format!("{} SDK is not installed or not available", self),
                format!(
                    "Please install {} SDK through Xcode or use xcode-select to configure the active developer directory.",
                    self
                ),
            ));
        }

        Ok(())
    }
}
