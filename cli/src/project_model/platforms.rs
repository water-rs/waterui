//! Project backend declarations and backend resolution.

use std::collections::BTreeMap;

use eyre::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize};

use crate::platform::{TargetBackend, TargetPlatform};
use crate::project::Manifest;

/// Platform names accepted by the CLI and by `[platforms.<platform>]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlatformName {
    /// iOS device.
    Ios,
    /// iOS simulator.
    IosSimulator,
    /// macOS.
    Macos,
    /// Android.
    Android,
    /// Linux.
    Linux,
    /// Windows.
    Windows,
    /// Web.
    Web,
    /// ESP32-S3.
    Esp32s3,
    /// ESP32-C3.
    Esp32c3,
    /// ESP32-P4.
    Esp32p4,
}

impl PlatformName {
    /// The build platform selected by this manifest key.
    #[must_use]
    pub const fn target(self) -> TargetPlatform {
        match self {
            Self::Ios => TargetPlatform::IOS,
            Self::IosSimulator => TargetPlatform::IOSSimulator,
            Self::Macos => TargetPlatform::MacOS,
            Self::Android => TargetPlatform::Android,
            Self::Linux => TargetPlatform::Linux,
            Self::Windows => TargetPlatform::Windows,
            Self::Web => TargetPlatform::Web,
            Self::Esp32s3 => TargetPlatform::Esp32S3,
            Self::Esp32c3 => TargetPlatform::Esp32C3,
            Self::Esp32p4 => TargetPlatform::Esp32P4,
        }
    }
}

/// Backend selection for a single platform.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformConfig {
    /// Backend this project requires for the platform.
    pub backend: TargetBackend,
}

pub(super) fn deserialize<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<PlatformName, PlatformConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let platforms = BTreeMap::<PlatformName, PlatformConfig>::deserialize(deserializer)?;
    for (platform, config) in &platforms {
        validate_backend(platform.target(), config.backend).map_err(serde::de::Error::custom)?;
    }
    Ok(platforms)
}

fn validate_backend(platform: TargetPlatform, backend: TargetBackend) -> Result<()> {
    let available = platform.available_backends();
    if available.is_empty() {
        bail!(
            "Platform {platform:?} has no supported backend until Hydrolysis's embedded host lands \
             (water-rs/waterui#1601); found backend {backend:?}"
        );
    }
    if !available.contains(&backend) {
        bail!(
            "Backend {backend:?} does not support platform {platform:?}. Valid backends: {available:?}"
        );
    }
    Ok(())
}

/// Resolve the backend before opening managed projects or starting a build.
///
/// An explicit override may confirm a declaration, but cannot contradict it.
/// With neither, the platform's library default applies.
///
/// # Errors
/// Returns an error for a conflicting override, an unsupported combination,
/// or a platform with no backend.
pub fn resolve_backend(
    project: &Manifest,
    platform: TargetPlatform,
    explicit: Option<TargetBackend>,
) -> Result<TargetBackend> {
    let declared = project
        .platforms
        .iter()
        .find(|(name, _)| name.target() == platform)
        .map(|(_, config)| config.backend);
    if let (Some(explicit), Some(declared)) = (explicit, declared)
        && explicit != declared
    {
        bail!(
            "Explicit backend {explicit:?} conflicts with Water.toml backend {declared:?} \
             for platform {platform:?}"
        );
    }
    let backend = explicit.or(declared).or_else(|| platform.default_backend())
        .ok_or_else(|| eyre::eyre!(
            "Platform {platform:?} has no supported backend until Hydrolysis's embedded host lands \
             (water-rs/waterui#1601)"
        ))?;
    validate_backend(platform, backend)?;
    Ok(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE: &str = "[package]\nname = 'Demo'\nbundle_identifier = 'dev.example.demo'\n";

    fn manifest(declaration: &str) -> Manifest {
        Manifest::parse(&format!("{PACKAGE}{declaration}")).unwrap()
    }

    #[test]
    fn backend_resolution_precedence() {
        let declared = manifest("[platforms.linux]\nbackend = 'hydrolysis'");
        let plain = manifest("");
        for (project, explicit, expected) in [
            (&declared, None, TargetBackend::Hydrolysis),
            (
                &declared,
                Some(TargetBackend::Hydrolysis),
                TargetBackend::Hydrolysis,
            ),
            (
                &plain,
                Some(TargetBackend::Hydrolysis),
                TargetBackend::Hydrolysis,
            ),
            (&plain, None, TargetBackend::Gtk4),
        ] {
            assert_eq!(
                resolve_backend(project, TargetPlatform::Linux, explicit).unwrap(),
                expected
            );
        }
        let error = resolve_backend(&declared, TargetPlatform::Linux, Some(TargetBackend::Gtk4))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Gtk4") && error.contains("Hydrolysis"),
            "{error}"
        );
    }

    #[test]
    fn unknown_platform_names_list_valid_names() {
        let error = Manifest::parse(&format!(
            "{PACKAGE}[platforms.linuz]\nbackend = 'hydrolysis'"
        ))
        .unwrap_err()
        .to_string();
        for name in [
            "linuz",
            "ios",
            "ios-simulator",
            "macos",
            "android",
            "linux",
            "windows",
            "web",
            "esp32s3",
            "esp32c3",
            "esp32p4",
        ] {
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn unknown_backend_names_list_valid_names() {
        let error = Manifest::parse(&format!("{PACKAGE}[platforms.linux]\nbackend = 'typo'"))
            .unwrap_err()
            .to_string();
        for name in ["typo", "apple", "android", "gtk4", "hydrolysis", "winui"] {
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn unsupported_backend_is_a_parse_error() {
        let text = format!("{PACKAGE}[platforms.linux]\nbackend = 'apple'");
        assert!(
            Manifest::parse(&text)
                .unwrap_err()
                .to_string()
                .contains("does not support")
        );
        assert!(toml::from_str::<Manifest>(&text).is_err());
    }

    #[test]
    fn absent_declarations_stay_absent_when_serialized() {
        let project = manifest("");
        assert!(project.platforms.is_empty());
        assert!(
            !toml::to_string(&Manifest::new(project.package))
                .unwrap()
                .contains("platforms")
        );
    }

    #[test]
    fn declarations_round_trip() {
        let project = manifest(
            "[platforms.windows]\nbackend = 'winui'\n[platforms.ios-simulator]\nbackend = 'apple'",
        );
        let parsed = Manifest::parse(&toml::to_string(&project).unwrap()).unwrap();
        assert_eq!(
            resolve_backend(&parsed, TargetPlatform::Windows, None).unwrap(),
            TargetBackend::WinUi
        );
        assert_eq!(
            resolve_backend(&parsed, TargetPlatform::IOSSimulator, None).unwrap(),
            TargetBackend::Apple
        );
    }

    #[test]
    fn unsupported_overrides_and_unserved_platforms_fail() {
        let project = manifest("");
        assert!(
            resolve_backend(&project, TargetPlatform::Linux, Some(TargetBackend::WinUi)).is_err()
        );
        for platform in [
            TargetPlatform::Esp32S3,
            TargetPlatform::Esp32C3,
            TargetPlatform::Esp32P4,
        ] {
            for explicit in [None, Some(TargetBackend::Hydrolysis)] {
                assert!(
                    resolve_backend(&project, platform, explicit)
                        .unwrap_err()
                        .to_string()
                        .contains("#1601")
                );
            }
        }
    }

    #[test]
    fn undeclared_platforms_use_library_defaults() {
        let project = manifest("");
        for platform in [
            TargetPlatform::IOS,
            TargetPlatform::IOSSimulator,
            TargetPlatform::MacOS,
            TargetPlatform::Android,
            TargetPlatform::Linux,
            TargetPlatform::Windows,
            TargetPlatform::Web,
        ] {
            assert_eq!(
                Some(resolve_backend(&project, platform, None).unwrap()),
                platform.default_backend()
            );
        }
    }
}
