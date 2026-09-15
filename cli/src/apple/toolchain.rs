//! Apple toolchain module

use std::convert::Infallible;
use std::ffi::OsString;

use eyre::Context as _;
use serde::{Deserialize, Serialize};

use crate::toolchain::{Host, Toolchain, ToolchainError};

/// Represents the complete Apple toolchain consisting of Xcode and an Apple SDK
pub type AppleToolchain = (Xcode, AppleSdk);

/// Represents the Xcode toolchain
#[derive(Debug, Clone, Default)]
pub struct Xcode;

impl Toolchain for Xcode {
    type Installation = Infallible;
    async fn check(
        &self,
        host: &Host,
    ) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if Xcode is installed and available
        if host.which("xcodebuild").await.is_ok() && host.which("xcode-select").await.is_ok() {
            Ok(())
        } else {
            Err(ToolchainError::unfixable(
                "Xcode is not installed or not found in PATH",
                "Please install Xcode from the App Store or the Apple Developer website and ensure it's available in your PATH.",
            ))
        }
    }
}

/// Represents an Apple SDK (e.g., iOS, macOS)
#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub enum AppleSdk {
    /// iOS SDK
    #[serde(rename = "iOS")]
    Ios,
    /// iOS Simulator SDK
    #[serde(rename = "iOS Simulator")]
    IosSimulator,
    /// macOS SDK
    #[serde(rename = "macOS")]
    Macos,
    /// tvOS SDK
    #[serde(rename = "tvOS")]
    TvOs,
    /// watchOS SDK
    #[serde(rename = "watchOS")]
    WatchOs,
    /// visionOS SDK
    #[serde(rename = "visionOS")]
    VisionOs,
}

impl AppleSdk {
    /// Get the SDK name as used by `xcrun`
    #[must_use]
    pub const fn sdk_name(&self) -> &str {
        match self {
            Self::Ios => "iphoneos",
            Self::IosSimulator => "iphonesimulator",
            Self::Macos => "macosx",
            Self::TvOs => "appletvos",
            Self::WatchOs => "watchos",
            Self::VisionOs => "xros",
        }
    }
}

/// The development team used to sign device builds.
///
/// Physical-device builds must be signed in every profile — iOS refuses
/// unsigned code outright — so `DEVELOPMENT_TEAM` cannot come from the Xcode
/// project (which does not know the developer's team) and is resolved here
/// instead. Preference order:
///
/// 1. The team Xcode last provisioned with
///    (`IDEProvisioningTeamManagerLastSelectedTeamID`), then any other team
///    Xcode knows an account for (`IDEProvisioningTeamByIdentifier`). A
///    signed-in account can mint both the provisioning profile and the
///    "Apple Development" certificate it needs, so these work even when the
///    keychain holds no matching identity yet.
/// 2. The team embedded in a keychain development certificate —
///    `security find-identity -v -p codesigning` prints identities as
///    `… "Apple Development: Liu Yuhao (6C5VGHHJ59)"` where the parenthesized
///    suffix is the team ID. Such a team is only usable when a matching
///    profile is already installed locally; without an Xcode account the
///    portal cannot mint one, which is why it ranks last.
///
/// # Errors
/// Fails when neither an Xcode account team nor a development certificate
/// exists — the error tells the user where to add an account.
pub async fn development_team_id(host: &Host) -> eyre::Result<String> {
    if let Some(team) = xcode_account_team(host).await {
        return Ok(team);
    }
    let output = host
        .output("security", ["find-identity", "-v", "-p", "codesigning"])
        .await
        .wrap_err("failed to run `security find-identity`")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_development_team(&stdout).ok_or_else(|| {
        eyre::eyre!(
            "No signing team found. Physical iOS builds must be signed: open \
             Xcode → Settings → Accounts and sign in an Apple ID (a free \
             account is enough), then re-run `water run`."
        )
    })
}

/// A team Xcode can provision for: the account's last-selected team, or any
/// team its accounts advertise. Reads Xcode's account registry from
/// `~/Library/Preferences/com.apple.dt.Xcode.plist`; a missing Xcode install
/// or unsigned-in state yields `None`.
async fn xcode_account_team(host: &Host) -> Option<String> {
    let plist = host
        .home_dir()?
        .join("Library/Preferences/com.apple.dt.Xcode.plist");

    let extract = |key: &str, format: &str| {
        let plist = plist.clone();
        let key = key.to_string();
        let format = format.to_string();
        async move {
            host.output(
                "plutil",
                [
                    OsString::from("-extract"),
                    OsString::from(key),
                    OsString::from(format),
                    OsString::from("-o"),
                    OsString::from("-"),
                    plist.into_os_string(),
                ],
            )
            .await
        }
    };

    if let Ok(output) = extract("IDEProvisioningTeamManagerLastSelectedTeamID", "raw").await
        && output.status.success()
    {
        let team = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !team.is_empty() {
            return Some(team);
        }
    }

    let output = extract("IDEProvisioningTeamByIdentifier", "json")
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let teams: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    teams.as_object()?.keys().next().cloned()
}

/// Extract the team ID from the first development identity in
/// `security find-identity -v -p codesigning` output.
fn parse_development_team(output: &str) -> Option<String> {
    for line in output.lines() {
        let is_development = line.contains("Apple Development:")
            || line.contains("iPhone Developer:")
            || line.contains("iOS Development:");
        if !is_development {
            continue;
        }
        if let Some(start) = line.rfind('(')
            && let Some(end) = line.rfind(')')
            && end > start
        {
            return Some(line[start + 1..end].to_string());
        }
    }
    None
}

impl std::fmt::Display for AppleSdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid panics in Display; serde is for config IO, not formatting.
        match self {
            Self::Ios => "iOS",
            Self::IosSimulator => "iOS Simulator",
            Self::Macos => "macOS",
            Self::TvOs => "tvOS",
            Self::WatchOs => "watchOS",
            Self::VisionOs => "visionOS",
        }
        .fmt(f)
    }
}

impl Toolchain for AppleSdk {
    type Installation = Infallible;
    async fn check(
        &self,
        host: &Host,
    ) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if the required Apple SDK is available
        let result = host
            .run("xcrun", ["--sdk", self.sdk_name(), "--show-sdk-path"])
            .await;

        if result.is_err() {
            return Err(ToolchainError::unfixable(
                format!("{self} SDK is not installed or not available"),
                format!(
                    "Please install {self} SDK through Xcode or use xcode-select to configure the active developer directory."
                ),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppleSdk, Xcode};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[test]
    fn xcode_ok_when_tools_on_path() {
        let machine = TestMachine::new();
        machine.install("xcodebuild");
        machine.install("xcode-select");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Xcode.check(&host)).expect("xcodebuild + xcode-select on PATH must be ok");
    }

    #[test]
    fn xcode_missing_is_unfixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Xcode.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "Xcode requires a manual App Store install: {result:?}"
        );
    }

    #[test]
    fn apple_sdk_ok_when_xcrun_reports_path() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        machine.respond("XCRUN_SDK_PATH", "/fake/SDKs/iPhoneOS.sdk\n");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(AppleSdk::Ios.check(&host))
            .expect("an SDK path from xcrun must satisfy the check");
    }

    #[test]
    fn apple_sdk_missing_is_unfixable() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AppleSdk::Ios.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "xcrun without an SDK path must be unfixable: {result:?}"
        );
    }
}
