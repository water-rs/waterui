//! Project backend declarations and backend resolution.

use std::collections::BTreeMap;

use eyre::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize};

use crate::platform::{TargetBackend, TargetPlatform};
use crate::project::Manifest;

/// Platform names accepted by the CLI and by `[platforms.<platform>]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
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
        }
    }
}

/// Backend selection for a single platform.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformConfig {
    /// Backend this project requires for the platform.
    backend: TargetBackend,
}
impl PlatformConfig {
    /// A `PlatformConfig` carrying each of its fields.
    #[must_use]
    pub const fn new(backend: TargetBackend) -> Self {
        Self { backend }
    }

    /// Backend this project requires for the platform.
    #[must_use]
    pub const fn backend(&self) -> &TargetBackend {
        &self.backend
    }
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
/// Returns an error for a conflicting override or an unsupported
/// combination.
pub fn resolve_backend(
    project: &Manifest,
    platform: TargetPlatform,
    explicit: Option<TargetBackend>,
) -> Result<TargetBackend> {
    let declared = project
        .platforms()
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
    let backend = explicit
        .or(declared)
        .unwrap_or_else(|| platform.default_backend());
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
        ] {
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn unknown_backend_names_list_valid_names() {
        let error = Manifest::parse(&format!("{PACKAGE}[platforms.linux]\nbackend = 'typo'"))
            .unwrap_err()
            .to_string();
        for name in ["typo", "apple", "gtk4", "hydrolysis", "winui"] {
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
        assert!(project.platforms().is_empty());
        assert!(
            !toml::to_string(&Manifest::new(project.into_package()))
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
    fn unsupported_overrides_fail() {
        let project = manifest("");
        assert!(
            resolve_backend(&project, TargetPlatform::Linux, Some(TargetBackend::WinUi)).is_err()
        );
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
                resolve_backend(&project, platform, None).unwrap(),
                platform.default_backend()
            );
        }
    }
}
