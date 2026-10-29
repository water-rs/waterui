//! The `[apple.deployment-targets]` section of `Water.toml`: the Apple OS
//! versions a project declares it runs on.
//!
//! The framework's `apple-deployment-targets` metadata is the floor; a project
//! that needs a newer OS (an API gated on it only compiles at that deployment
//! target) raises it here. Everything the CLI derives from a deployment
//! target — the `*_DEPLOYMENT_TARGET` environment of a build, the project's
//! `.cargo/config.toml`, `Package.swift`, `Info.plist` minimums, simulator
//! selection — reads the *effective* target this module resolves, never the
//! framework's floor directly.

use std::fmt;

use color_eyre::eyre::{Result, bail};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::framework::ResolvedFramework;
use crate::runtime::utils::{VersionParseError, parse_semver_version};

/// The platform keys of the framework's deployment-target tables and of
/// `[apple.deployment-targets]`.
pub const APPLE_PLATFORMS: [&str; 5] = ["macos", "ios", "tvos", "watchos", "visionos"];

/// A deployment-target version as written: `27.0`, with the parsed form kept
/// for comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AppleVersion {
    text: String,
    parsed: Version,
}

impl AppleVersion {
    /// Parses `text`, which may omit the minor and patch components.
    ///
    /// # Errors
    /// Returns the parse error when `text` is not a numeric version.
    pub fn parse(text: &str) -> Result<Self, VersionParseError> {
        let parsed = parse_semver_version(text)?;
        Ok(Self {
            text: text.trim().to_owned(),
            parsed,
        })
    }

    /// The version as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl TryFrom<String> for AppleVersion {
    type Error = VersionParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<AppleVersion> for String {
    fn from(version: AppleVersion) -> Self {
        version.text
    }
}

impl fmt::Display for AppleVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The `[apple]` section of `Water.toml`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppleConfig {
    /// `[apple.deployment-targets]`.
    #[serde(
        default,
        rename = "deployment-targets",
        skip_serializing_if = "AppleDeploymentTargets::is_empty"
    )]
    deployment_targets: AppleDeploymentTargets,
}

impl AppleConfig {
    /// A section declaring `deployment_targets`.
    #[must_use]
    pub const fn new(deployment_targets: AppleDeploymentTargets) -> Self {
        Self { deployment_targets }
    }

    /// `[apple.deployment-targets]`.
    #[must_use]
    pub const fn deployment_targets(&self) -> &AppleDeploymentTargets {
        &self.deployment_targets
    }

    /// Whether the section declares nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.deployment_targets.is_empty()
    }
}

/// The deployment targets a project declares, per platform. A platform left
/// out runs on the framework's floor.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppleDeploymentTargets {
    /// macOS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    macos: Option<AppleVersion>,
    /// iOS, Mac Catalyst and the iOS simulator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ios: Option<AppleVersion>,
    /// tvOS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tvos: Option<AppleVersion>,
    /// watchOS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    watchos: Option<AppleVersion>,
    /// visionOS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visionos: Option<AppleVersion>,
}

impl AppleDeploymentTargets {
    /// Whether no platform is declared.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.macos.is_none()
            && self.ios.is_none()
            && self.tvos.is_none()
            && self.watchos.is_none()
            && self.visionos.is_none()
    }

    /// The declaration for `platform`, one of [`APPLE_PLATFORMS`].
    ///
    /// # Panics
    /// When `platform` is not an Apple platform key.
    #[must_use]
    pub fn declared(&self, platform: &str) -> Option<&AppleVersion> {
        match platform {
            "macos" => self.macos.as_ref(),
            "ios" => self.ios.as_ref(),
            "tvos" => self.tvos.as_ref(),
            "watchos" => self.watchos.as_ref(),
            "visionos" => self.visionos.as_ref(),
            other => panic!("`{other}` is not an Apple platform key"),
        }
    }

    /// Declares `version` for `platform`.
    ///
    /// # Panics
    /// When `platform` is not an Apple platform key.
    pub fn declare(&mut self, platform: &str, version: AppleVersion) {
        let slot = match platform {
            "macos" => &mut self.macos,
            "ios" => &mut self.ios,
            "tvos" => &mut self.tvos,
            "watchos" => &mut self.watchos,
            "visionos" => &mut self.visionos,
            other => panic!("`{other}` is not an Apple platform key"),
        };
        *slot = Some(version);
    }

    /// The deployment target a build for `platform` runs with: the project's
    /// declaration, else the framework's floor.
    ///
    /// # Errors
    /// Returns an error when the framework declares no floor for `platform`,
    /// or the project declares a version below it.
    pub fn effective(&self, framework: &ResolvedFramework, platform: &str) -> Result<String> {
        let floor = framework.apple_deployment_target(platform)?;
        let Some(declared) = self.declared(platform) else {
            return Ok(floor.to_owned());
        };
        let floor_version = AppleVersion::parse(floor)?;
        if declared.parsed < floor_version.parsed {
            bail!(
                "Water.toml declares apple.deployment-targets.{platform} = \"{declared}\", below \
                 the framework's {platform} deployment floor {floor} (apple-deployment-targets \
                 of the selected framework); raise it to {floor} or newer"
            );
        }
        Ok(declared.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::test_fixtures::stable_framework;

    fn declared(text: &str) -> AppleDeploymentTargets {
        toml::from_str(text).expect("a valid deployment-targets table")
    }

    #[test]
    fn an_undeclared_platform_runs_on_the_framework_floor() {
        let framework = stable_framework();
        let targets = declared("ios = \"27.0\"");
        assert_eq!(targets.effective(&framework, "macos").unwrap(), "26.0");
        assert_eq!(targets.effective(&framework, "visionos").unwrap(), "2.5");
    }

    #[test]
    fn a_declaration_at_or_above_the_floor_wins() {
        let framework = stable_framework();
        let targets = declared("ios = \"27.0\"\nmacos = \"26.0\"\nvisionos = \"2.6\"");
        assert_eq!(targets.effective(&framework, "ios").unwrap(), "27.0");
        assert_eq!(targets.effective(&framework, "macos").unwrap(), "26.0");
        assert_eq!(targets.effective(&framework, "visionos").unwrap(), "2.6");
    }

    #[test]
    fn a_declaration_below_the_floor_names_both_versions() {
        let framework = stable_framework();
        let error = declared("ios = \"25.4\"")
            .effective(&framework, "ios")
            .unwrap_err()
            .to_string();
        assert!(error.contains("\"25.4\""), "{error}");
        assert!(error.contains("floor 26.0"), "{error}");
    }

    #[test]
    fn an_unknown_platform_or_malformed_version_is_rejected_on_parse() {
        assert!(toml::from_str::<AppleDeploymentTargets>("windows = \"1.0\"").is_err());
        assert!(toml::from_str::<AppleDeploymentTargets>("ios = \"latest\"").is_err());
        assert!(toml::from_str::<AppleConfig>("unknown = 1").is_err());
    }

    #[test]
    fn the_section_round_trips_through_water_toml() {
        let text = "[apple.deployment-targets]\nios = \"27.0\"\n";
        let manifest: toml::Table = toml::from_str(text).unwrap();
        let config: AppleConfig = manifest["apple"].clone().try_into().unwrap();
        assert_eq!(
            config.deployment_targets.ios.as_ref().unwrap().as_str(),
            "27.0"
        );
        assert!(AppleConfig::default().is_empty());
    }
}
