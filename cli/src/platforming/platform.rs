//! Platform abstraction for `WaterUI` CLI.

use std::str::FromStr;

use crate::build::{BuildProfile, BuildProgress};
use eyre::bail;
use target_lexicon::{
    Aarch64Architecture, Architecture, DefaultToHost, Environment, OperatingSystem,
    Riscv32Architecture, Triple, Vendor,
};

// ============================================================================
// Target Platform Enum (New Architecture)
// ============================================================================

/// Target platform for building and running `WaterUI` apps.
///
/// This enum replaces the old `Platform` trait with a simpler, more explicit model.
/// Each variant represents a specific target platform that `WaterUI` can build for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetPlatform {
    // Apple platforms
    /// macOS (ARM64)
    MacOS,
    /// iOS (physical device, ARM64)
    IOS,
    /// iOS Simulator (ARM64)
    IOSSimulator,
    /// Mac Catalyst — the iOS runtime bridged onto macOS
    /// (`aarch64-apple-ios-macabi`). It compiles against the macOS SDK's
    /// `System/iOSSupport` frameworks, not the macOS or iOS SDKs directly.
    /// Internal to the target model for now; no `water` platform flag
    /// selects it (#2103).
    MacCatalyst,
    /// tvOS (physical device)
    TvOS,
    /// tvOS Simulator
    TvOSSimulator,
    /// watchOS (physical device)
    WatchOS,
    /// watchOS Simulator
    WatchOSSimulator,
    /// visionOS (physical device)
    VisionOS,
    /// visionOS Simulator
    VisionOSSimulator,

    // Other platforms
    /// Android
    Android,
    /// Linux (GTK4)
    Linux,
    /// Windows (Hydrolysis)
    Windows,
    /// Web (WASM + WebGPU)
    Web,
    /// ESP32-S3 (Xtensa, ESP-IDF firmware)
    Esp32S3,
    /// ESP32-C3 (RISC-V, ESP-IDF firmware)
    Esp32C3,
    /// ESP32-P4 (RISC-V with FPU, ESP-IDF firmware)
    Esp32P4,
}

/// Backend types available for building.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetBackend {
    /// Apple backend (Xcode, UIKit/AppKit)
    Apple,
    /// Android backend (Gradle, Android Views)
    Android,
    /// GTK4 backend (pure Rust binary)
    Gtk4,
    /// Hydrolysis backend (self-drawn renderer)
    Hydrolysis,
    /// `WinUI` backend (Windows App SDK / `WinUI` 3, pure Rust binary)
    WinUi,
}

impl TargetBackend {
    /// The framework scaffold packages the backend's generated crate links —
    /// the names a `framework.json` `scaffold` (or `experimental-packages`)
    /// table keys on. A withheld package means the selected channel cannot
    /// scaffold the backend at all.
    #[must_use]
    pub const fn scaffold_packages(&self) -> &'static [&'static str] {
        match self {
            Self::Apple | Self::Android => &[],
            Self::Gtk4 => &["waterui-gtk"],
            // `hydrolysis` itself is an in-tree framework member resolved
            // through `hydrolysis-path`, not a scaffold package (#1635).
            Self::Hydrolysis => &["hydrolysis-m3"],
            Self::WinUi => &["waterui-winui"],
        }
    }

    /// Whether this host can build `platform` with this backend.
    ///
    /// Desktop backends only build for their own host OS; the Android
    /// backends and the web frontend cross-compile from any host, and so
    /// does the Hydrolysis Android path. Every `water`
    /// command that builds gates on this one check so a forbidden
    /// combination fails identically in `build`, `run` and `package`.
    ///
    /// # Errors
    /// Returns an error naming the required host or `--platform`.
    pub fn validate_host_support(&self, platform: TargetPlatform) -> eyre::Result<()> {
        if platform == TargetPlatform::Web {
            return Ok(());
        }
        match self {
            Self::Gtk4 => {
                #[cfg(target_os = "linux")]
                if platform != TargetPlatform::Linux {
                    bail!("GTK4 backend on Linux host requires --platform linux");
                }
                #[cfg(not(target_os = "linux"))]
                bail!("GTK4 backend is only supported on Linux hosts");
            }
            Self::Hydrolysis => {
                // The Hydrolysis Android path cross-compiles from any host.
                if platform == TargetPlatform::Android {
                    return Ok(());
                }
                #[cfg(target_os = "macos")]
                if platform != TargetPlatform::MacOS {
                    bail!("Hydrolysis backend on macOS host requires --platform macos");
                }
                #[cfg(target_os = "linux")]
                if platform != TargetPlatform::Linux {
                    bail!("Hydrolysis backend on Linux host requires --platform linux");
                }
                #[cfg(target_os = "windows")]
                if platform != TargetPlatform::Windows {
                    bail!("Hydrolysis backend on Windows host requires --platform windows");
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
                bail!("Hydrolysis backend is only supported on macOS, Linux, or Windows hosts");
            }
            Self::WinUi => {
                #[cfg(target_os = "windows")]
                if platform != TargetPlatform::Windows {
                    bail!("WinUI backend on Windows host requires --platform windows");
                }
                #[cfg(not(target_os = "windows"))]
                bail!("WinUI backend is only supported on Windows hosts");
            }
            Self::Apple => {
                #[cfg(not(target_os = "macos"))]
                bail!("Apple backend requires a macOS host");
            }
            Self::Android => {}
        }
        Ok(())
    }

    /// The [`BuildProfile`] a development `water build` or `water run` uses
    /// for this backend when the user passes no profile flag.
    ///
    /// Self-drawn backends spend their per-frame budget in the rendering
    /// stack, so a Hydrolysis development build lifts the `dev` profile to a
    /// light optimization level rather than paying debug-code frame times;
    /// every other backend builds the declared `dev` profile. The two
    /// commands must agree on this default: generated backend crates share
    /// one Cargo target directory, and a profile mismatch re-fingerprints
    /// every dependency unit — the build's artifacts then warm nothing the
    /// run reuses.
    #[must_use]
    pub const fn default_development_profile(&self) -> BuildProfile {
        match self {
            Self::Hydrolysis => BuildProfile::Optimized,
            _ => BuildProfile::Debug,
        }
    }
}

impl TargetPlatform {
    /// Get the target triple for this platform.
    ///
    /// # Panics
    /// May panic if `target_lexicon` cannot resolve the current host triple for host-native targets.
    #[must_use]
    pub fn triple(&self) -> Triple {
        match self {
            Self::MacOS => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::Darwin(None),
                environment: Environment::Unknown,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::IOS => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::IOS(None),
                environment: Environment::Unknown,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::IOSSimulator => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::IOS(None),
                environment: Environment::Sim,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::MacCatalyst => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::IOS(None),
                environment: Environment::Macabi,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::TvOS => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::TvOS(None),
                environment: Environment::Unknown,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::TvOSSimulator => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::TvOS(None),
                environment: Environment::Sim,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::WatchOS => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::WatchOS(None),
                environment: Environment::Unknown,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::WatchOSSimulator => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::WatchOS(None),
                environment: Environment::Sim,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::VisionOS => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::VisionOS(None),
                environment: Environment::Unknown,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::VisionOSSimulator => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::VisionOS(None),
                environment: Environment::Sim,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::Android => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Android,
                binary_format: target_lexicon::BinaryFormat::Elf,
            },
            Self::Linux | Self::Windows => Triple::host(),
            Self::Web => Triple::from_str("wasm32-unknown-unknown")
                .expect("web target triple must remain valid"),
            Self::Esp32S3 => Triple::from_str("xtensa-esp32s3-espidf")
                .expect("esp32s3 target triple must remain valid"),
            Self::Esp32C3 => Triple::from_str("riscv32imc-esp-espidf")
                .expect("esp32c3 target triple must remain valid"),
            Self::Esp32P4 => Triple::from_str("riscv32imafc-esp-espidf")
                .expect("esp32p4 target triple must remain valid"),
        }
    }

    /// Get available backends for this platform.
    #[must_use]
    pub const fn available_backends(&self) -> &[TargetBackend] {
        match self {
            Self::MacOS => &[TargetBackend::Apple, TargetBackend::Hydrolysis],
            Self::IOS
            | Self::IOSSimulator
            | Self::MacCatalyst
            | Self::TvOS
            | Self::TvOSSimulator
            | Self::WatchOS
            | Self::WatchOSSimulator
            | Self::VisionOS
            | Self::VisionOSSimulator => &[TargetBackend::Apple],
            Self::Android => &[TargetBackend::Hydrolysis, TargetBackend::Android],
            Self::Linux => &[TargetBackend::Gtk4, TargetBackend::Hydrolysis],
            Self::Windows => &[TargetBackend::Hydrolysis, TargetBackend::WinUi],
            Self::Web => &[TargetBackend::Hydrolysis],
            // No backend serves ESP32 targets yet: Dew is archived and
            // Hydrolysis's embedded host lands with #1601.
            Self::Esp32S3 | Self::Esp32C3 | Self::Esp32P4 => &[],
        }
    }

    /// Get the default backend for this platform — `None` for a platform no
    /// backend serves yet (the ESP32 targets, until #1601).
    #[must_use]
    pub const fn default_backend(&self) -> Option<TargetBackend> {
        match self {
            Self::MacOS
            | Self::IOS
            | Self::IOSSimulator
            | Self::MacCatalyst
            | Self::TvOS
            | Self::TvOSSimulator
            | Self::WatchOS
            | Self::WatchOSSimulator
            | Self::VisionOS
            | Self::VisionOSSimulator => Some(TargetBackend::Apple),
            Self::Android | Self::Windows | Self::Web => Some(TargetBackend::Hydrolysis),
            Self::Linux => Some(TargetBackend::Gtk4),
            Self::Esp32S3 | Self::Esp32C3 | Self::Esp32P4 => None,
        }
    }

    /// Whether the host builds for this platform with its own toolchain —
    /// the platform is one of the host's defaults.
    ///
    /// A macOS host builds every Apple platform through its one Xcode
    /// toolchain; a Linux or Windows host builds its own desktop. Android,
    /// the web and the ESP32 firmware are cross-compiled targets a project
    /// selects — the host never targets them on its own.
    #[must_use]
    pub const fn host_builds(&self) -> bool {
        match self {
            Self::MacOS
            | Self::IOS
            | Self::IOSSimulator
            | Self::MacCatalyst
            | Self::TvOS
            | Self::TvOSSimulator
            | Self::WatchOS
            | Self::WatchOSSimulator
            | Self::VisionOS
            | Self::VisionOSSimulator => cfg!(target_os = "macos"),
            Self::Linux => cfg!(target_os = "linux"),
            Self::Windows => cfg!(target_os = "windows"),
            Self::Android | Self::Web | Self::Esp32S3 | Self::Esp32C3 | Self::Esp32P4 => false,
        }
    }

    /// Check if this platform is a simulator/emulator.
    #[must_use]
    pub const fn is_simulator(&self) -> bool {
        matches!(
            self,
            Self::IOSSimulator
                | Self::TvOSSimulator
                | Self::WatchOSSimulator
                | Self::VisionOSSimulator
        )
    }

    /// Get the SDK name for Apple platforms.
    #[must_use]
    pub const fn sdk_name(&self) -> Option<&'static str> {
        match self {
            // Catalyst links the iOS frameworks the macOS SDK ships under
            // `System/iOSSupport`, so its SDK is macosx too.
            Self::MacOS | Self::MacCatalyst => Some("macosx"),
            Self::IOS => Some("iphoneos"),
            Self::IOSSimulator => Some("iphonesimulator"),
            Self::TvOS => Some("appletvos"),
            Self::TvOSSimulator => Some("appletvsimulator"),
            Self::WatchOS => Some("watchos"),
            Self::WatchOSSimulator => Some("watchsimulator"),
            Self::VisionOS => Some("xros"),
            Self::VisionOSSimulator => Some("xrsimulator"),
            Self::Android
            | Self::Linux
            | Self::Windows
            | Self::Web
            | Self::Esp32S3
            | Self::Esp32C3
            | Self::Esp32P4 => None,
        }
    }

    /// The platform name an xcodebuild `-destination` specifier uses
    /// (`platform=iOS,id=…`, `generic/platform=tvOS`) for this platform's
    /// device family.
    #[must_use]
    pub const fn xcode_destination_name(&self) -> Option<&'static str> {
        match self {
            Self::MacOS => Some("macOS"),
            Self::IOS | Self::IOSSimulator => Some("iOS"),
            Self::TvOS | Self::TvOSSimulator => Some("tvOS"),
            Self::WatchOS | Self::WatchOSSimulator => Some("watchOS"),
            Self::VisionOS | Self::VisionOSSimulator => Some("visionOS"),
            Self::MacCatalyst
            | Self::Android
            | Self::Linux
            | Self::Windows
            | Self::Web
            | Self::Esp32S3
            | Self::Esp32C3
            | Self::Esp32P4 => None,
        }
    }

    /// The `TARGETED_DEVICE_FAMILY` value Xcode projects for this platform
    /// declare (`1,2` = iPhone+iPad, `3` = Apple TV, `4` = Apple Watch,
    /// `7` = Apple Vision).
    #[must_use]
    pub const fn targeted_device_family(&self) -> Option<&'static str> {
        match self {
            Self::IOS | Self::IOSSimulator => Some("1,2"),
            Self::TvOS | Self::TvOSSimulator => Some("3"),
            Self::WatchOS | Self::WatchOSSimulator => Some("4"),
            Self::VisionOS | Self::VisionOSSimulator => Some("7"),
            Self::MacCatalyst
            | Self::MacOS
            | Self::Android
            | Self::Linux
            | Self::Windows
            | Self::Web
            | Self::Esp32S3
            | Self::Esp32C3
            | Self::Esp32P4 => None,
        }
    }

    /// The `*_DEPLOYMENT_TARGET` Xcode build setting this platform uses.
    #[must_use]
    pub const fn deployment_target_setting(&self) -> Option<&'static str> {
        match self {
            Self::MacOS => Some("MACOSX_DEPLOYMENT_TARGET"),
            Self::IOS | Self::IOSSimulator | Self::MacCatalyst => {
                Some("IPHONEOS_DEPLOYMENT_TARGET")
            }
            Self::TvOS | Self::TvOSSimulator => Some("TVOS_DEPLOYMENT_TARGET"),
            Self::WatchOS | Self::WatchOSSimulator => Some("WATCHOS_DEPLOYMENT_TARGET"),
            Self::VisionOS | Self::VisionOSSimulator => Some("XROS_DEPLOYMENT_TARGET"),
            Self::Android
            | Self::Linux
            | Self::Windows
            | Self::Web
            | Self::Esp32S3
            | Self::Esp32C3
            | Self::Esp32P4 => None,
        }
    }

    /// Get the architecture for this platform.
    #[must_use]
    pub fn arch(&self) -> Architecture {
        match self {
            Self::MacOS
            | Self::IOSSimulator
            | Self::MacCatalyst
            | Self::TvOSSimulator
            | Self::WatchOSSimulator
            | Self::VisionOSSimulator
            | Self::IOS
            | Self::TvOS
            | Self::WatchOS
            | Self::VisionOS
            | Self::Android => Architecture::Aarch64(Aarch64Architecture::Aarch64),
            Self::Linux | Self::Windows => DefaultToHost::default().0.architecture,
            Self::Web => Architecture::Wasm32,
            Self::Esp32S3 => Architecture::XTensa,
            Self::Esp32C3 => Architecture::Riscv32(Riscv32Architecture::Riscv32imc),
            Self::Esp32P4 => Architecture::Riscv32(Riscv32Architecture::Riscv32imafc),
        }
    }
}

/// The Apple platforms the generated manifests' `cfg(target_vendor =
/// "apple")` tables serve — every Apple platform this CLI produces.
const APPLE_PLATFORMS: &[TargetPlatform] = &[
    TargetPlatform::MacOS,
    TargetPlatform::IOS,
    TargetPlatform::IOSSimulator,
    TargetPlatform::MacCatalyst,
    TargetPlatform::TvOS,
    TargetPlatform::TvOSSimulator,
    TargetPlatform::WatchOS,
    TargetPlatform::WatchOSSimulator,
    TargetPlatform::VisionOS,
    TargetPlatform::VisionOSSimulator,
];

/// The Apple triples a `cfg(target_vendor = "apple")` table of a generated
/// manifest serves — every Apple platform a `water` build can produce,
/// device and simulator alike.
#[must_use]
pub(crate) fn apple_target_triples() -> Vec<Triple> {
    APPLE_PLATFORMS.iter().map(TargetPlatform::triple).collect()
}

/// A desktop operating system a generated manifest can answer for
/// separately. macOS, Linux and Windows graph answers legitimately differ
/// — `waterui-browser-wpe` enters only on Linux — so a `[target.*]` table
/// that spans more than one of them may carry only what resolves
/// identically for all; the OS-dependent pieces get a table per OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NativeOs {
    /// `cfg(target_os = "macos")`
    MacOs,
    /// `cfg(target_os = "linux")`
    Linux,
    /// `cfg(windows)`
    Windows,
}

impl NativeOs {
    /// Every desktop OS, in table order.
    pub(crate) const ALL: [Self; 3] = [Self::MacOs, Self::Linux, Self::Windows];

    /// The `[target.*]` table key this OS's section is written under.
    #[must_use]
    pub(crate) const fn cfg(self) -> &'static str {
        match self {
            Self::MacOs => "cfg(target_os = \"macos\")",
            Self::Linux => "cfg(target_os = \"linux\")",
            Self::Windows => "cfg(windows)",
        }
    }

    /// The `cfg` predicate — inside `#[cfg(...)]` — selecting this OS.
    #[must_use]
    pub(crate) const fn cfg_predicate(self) -> &'static str {
        match self {
            Self::MacOs => "target_os = \"macos\"",
            Self::Linux => "target_os = \"linux\"",
            Self::Windows => "windows",
        }
    }

    /// The triples this OS's generated-manifest table serves — every
    /// target a `water` build produces for it.
    ///
    /// macOS builds aarch64 only, so its set is [`TargetPlatform::MacOS`]'s
    /// own triple. A Linux or Windows build produces the host's own
    /// triple — the `Triple::host()` [`TargetPlatform::triple`] resolves —
    /// so the served set is every host triple of that OS the released CLI
    /// runs on: the `[package.metadata.dist] targets` list `dist` ships the
    /// CLI for, which `build.rs` embeds, including its
    /// `x86_64-unknown-linux-musl` build.
    ///
    /// The running host's own triple rides along whenever the host's OS is
    /// the served one: `Triple::host()` names the toolchain this CLI was
    /// built with, so a `water` built from source on a triple `dist` never
    /// shipped still checks the table the host itself compiles — it is not
    /// a `cfg` answer to take at face value.
    #[must_use]
    pub(crate) fn serving_triples(self) -> Vec<Triple> {
        let mut triples = match self {
            Self::MacOs => vec![TargetPlatform::MacOS.triple()],
            Self::Linux => released_host_triples(OperatingSystem::Linux),
            Self::Windows => released_host_triples(OperatingSystem::Windows),
        };
        let host = Triple::host();
        if self == Self::running_host() && !triples.contains(&host) {
            triples.push(host);
        }
        triples
    }

    /// The `NativeOs` this CLI build runs on — the OS `Triple::host()`
    /// names.
    const fn running_host() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The host triples on `os` the released CLI ships for — the
/// `[package.metadata.dist] targets` list `build.rs` embeds from the CLI's
/// manifest, the list `dist` builds from.
///
/// # Panics
/// Panics when an embedded target is not a target triple — `dist` cannot
/// build such a list either.
fn released_host_triples(os: OperatingSystem) -> Vec<Triple> {
    env!("WATERUI_CLI_DIST_TARGETS")
        .split_whitespace()
        .map(|target| {
            target.parse::<Triple>().unwrap_or_else(|error| {
                panic!("the dist target `{target}` is not a target triple: {error}")
            })
        })
        .filter(|triple| triple.operating_system == os)
        .collect()
}

/// The desktop triples a generated manifest's
/// `cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))`
/// table serves — the macOS, Linux and Windows triples `water` builds
/// produce on every host they run on.
#[must_use]
pub(crate) fn native_target_triples() -> Vec<Triple> {
    NativeOs::ALL
        .into_iter()
        .flat_map(NativeOs::serving_triples)
        .collect()
}

/// The Linux triples the GTK4 backend crate serves — the Linux targets a
/// `water` build produces on any Linux host.
#[must_use]
pub(crate) fn linux_target_triples() -> Vec<Triple> {
    NativeOs::Linux.serving_triples()
}

/// The Windows triples the `WinUI` backend crate serves.
#[must_use]
pub(crate) fn windows_target_triples() -> Vec<Triple> {
    NativeOs::Windows.serving_triples()
}

/// The WebAssembly triple a generated manifest's `cfg(target_arch =
/// "wasm32")` table serves.
#[must_use]
pub(crate) fn wasm_target_triples() -> Vec<Triple> {
    vec![TargetPlatform::Web.triple()]
}

/// Reject a desktop platform label that does not name the host OS.
///
/// `macos`, `linux` and `windows` platforms require a matching host OS.
/// macOS targets ARM64; Linux and Windows use `Triple::host()`.
/// `water run` and `water preview` funnel through this check
/// so the gate is identical for both.
///
/// `desktop_os` is `Some` only for host-native platform labels; mobile, web
/// and embedded platforms carry their own triples and pass `None`.
///
/// # Errors
/// Returns an error when `desktop_os` names a different OS than `host_os`
/// (typically `std::env::consts::OS`).
pub fn ensure_desktop_platform_is_host(
    desktop_os: Option<&str>,
    host_os: &str,
) -> eyre::Result<()> {
    match desktop_os {
        Some(os) if os != host_os => {
            bail!("`--platform {os}` targets the host; this host is {host_os}.");
        }
        _ => Ok(()),
    }
}

// ============================================================================
// Package Options
// ============================================================================

/// Who a packaged application is for.
///
/// The audience decides how the artifact is sealed for delivery: a
/// distribution package must survive delivery to a machine that never saw
/// the developer's keychain, while a development package only has to run
/// where it was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackageAudience {
    /// A package for the developer's own machine: ad hoc signature on macOS,
    /// a direct-install artifact on Android.
    #[default]
    Development,
    /// A package for users elsewhere: Developer ID signature, hardened
    /// runtime and notarization on macOS; a store-upload artifact on Android.
    Distribution,
}

/// Configuration options for packaging the application.
///
/// This struct contains settings that control how the application
/// is packaged for distribution across different platforms.
#[derive(Debug, Clone)]
pub struct PackageOptions {
    /// Who the package is for.
    ///
    /// A distribution package is configured for delivery outside the
    /// developer's machine (notarized Developer ID signature on macOS, an
    /// upload artifact for the App Store or Play Store); a development
    /// package is for direct use on the machine that built it.
    ///
    /// # Warning
    ///
    /// The audience only changes the packaging format, not the build
    /// configuration. For a real world distribution build, you may also want
    /// to disable `debug` in `BuildOptions`.
    audience: PackageAudience,

    /// Whether to enable debug mode in the packaged application.
    ///
    /// When `true`, the application will include additional debug information
    /// and logging capabilities to facilitate troubleshooting during development.
    ///
    /// When `false`, the application will be optimized for release with
    /// minimal debug information.
    ///
    /// This flag is not conflict with `distribution`, since `distribution` decide the package format,
    /// while `debug` decide the build configuration.
    debug: bool,

    /// How a build for a physical device is code-signed.
    device_signing: DeviceSigning,

    /// Hardware UDID of the physical Apple device the package targets, when
    /// packaging for one (`water run`); `None` for a device-agnostic
    /// `water package`, which skips the profile's `ProvisionedDevices`
    /// check and provisions with a generic iOS destination.
    device_udid: Option<String>,

    /// Whether the package embeds the shared `WaterUI` Rust runtime.
    shared_rust_runtime: bool,

    /// How `include_web!` mounts reach the packaged app.
    web_frontend: WebFrontendMode,

    /// Sink compile progress is reported to while packaging runs cargo —
    /// asset-manifest planning compiles the project rlib for its symbol table.
    progress: Option<BuildProgress>,
}

/// How a build for a physical device is code-signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceSigning {
    /// Automatic signing with the developer team resolved on the host.
    #[default]
    Automatic,
    /// No code signing, for a host without a signing identity (CI, a build
    /// VM). The artifact keeps the shipped linkage and profile and is signed
    /// before it is installed.
    Unsigned,
}

/// Whether an `include_web!` mount is staged from a frontend build or served
/// by a running dev server.
///
/// In dev-server mode staging skips a web mount entirely — no frontend build
/// and no copied output — since the app opens the bundler's dev-server URL
/// instead of the staged bundle. `water run` selects it in debug mode;
/// packaging and `--release` runs never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebFrontendMode {
    /// Build the frontend and stage its output into the bundle.
    #[default]
    Stage,
    /// A running dev server serves the mount; nothing is staged for it.
    DevServer,
}

impl PackageOptions {
    /// Create options for installing and running a development build.
    #[must_use]
    pub const fn development() -> Self {
        Self {
            audience: PackageAudience::Development,
            debug: true,
            device_signing: DeviceSigning::Automatic,
            device_udid: None,
            shared_rust_runtime: true,
            web_frontend: WebFrontendMode::Stage,
            progress: None,
        }
    }

    /// Create options for a self-contained package artifact.
    #[must_use]
    pub const fn packaging(audience: PackageAudience, debug: bool) -> Self {
        Self {
            audience,
            debug,
            device_signing: DeviceSigning::Automatic,
            device_udid: None,
            shared_rust_runtime: false,
            web_frontend: WebFrontendMode::Stage,
            progress: None,
        }
    }

    /// Choose how a build for a physical device is code-signed.
    #[must_use]
    pub const fn with_device_signing(mut self, device_signing: DeviceSigning) -> Self {
        self.device_signing = device_signing;
        self
    }

    /// How a build for a physical device is code-signed.
    #[must_use]
    pub const fn device_signing(&self) -> DeviceSigning {
        self.device_signing
    }

    /// Bind the package to a physical Apple device by hardware UDID, so
    /// provisioning registers the device and the selected profile must list
    /// it. `None` keeps the package device-agnostic.
    #[must_use]
    pub fn with_device_udid(mut self, device_udid: Option<String>) -> Self {
        self.device_udid = device_udid;
        self
    }

    /// The hardware UDID of the device the package targets, if any.
    #[must_use]
    pub fn device_udid(&self) -> Option<&str> {
        self.device_udid.as_deref()
    }

    /// Override the debug flag without changing the runtime linkage.
    #[must_use]
    pub const fn with_debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }

    /// Mark web mounts as dev-server-served for this packaging pass.
    #[must_use]
    pub const fn with_dev_server(mut self, dev_server: bool) -> Self {
        self.web_frontend = if dev_server {
            WebFrontendMode::DevServer
        } else {
            WebFrontendMode::Stage
        };
        self
    }

    /// Who the package is for.
    #[must_use]
    pub const fn audience(&self) -> PackageAudience {
        self.audience
    }

    /// Whether to package in distribution mode
    #[must_use]
    pub const fn is_distribution(&self) -> bool {
        matches!(self.audience, PackageAudience::Distribution)
    }

    /// Whether to package in debug mode
    #[must_use]
    pub const fn is_debug(&self) -> bool {
        self.debug
    }

    /// Whether the package must embed the shared `WaterUI` Rust runtime.
    #[must_use]
    pub const fn uses_shared_rust_runtime(&self) -> bool {
        self.shared_rust_runtime
    }

    /// Whether web mounts are dev-server-served and skipped during staging.
    #[must_use]
    pub const fn uses_dev_server(&self) -> bool {
        matches!(self.web_frontend, WebFrontendMode::DevServer)
    }

    /// Attach a compile-progress sink the cargo invocations this packaging
    /// pass performs report to.
    #[must_use]
    pub fn with_progress(mut self, progress: BuildProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    /// The compile-progress sink, when one is attached.
    #[must_use]
    pub const fn progress(&self) -> Option<&BuildProgress> {
        self.progress.as_ref()
    }
}

#[cfg(test)]
mod host_support_tests {
    use super::{
        Aarch64Architecture, Architecture, Environment, OperatingSystem, TargetBackend,
        TargetPlatform,
    };

    #[test]
    fn apple_defaults_are_arm64_for_devices_and_simulators() {
        for platform in [
            TargetPlatform::MacOS,
            TargetPlatform::IOS,
            TargetPlatform::IOSSimulator,
            TargetPlatform::MacCatalyst,
            TargetPlatform::TvOS,
            TargetPlatform::TvOSSimulator,
            TargetPlatform::WatchOS,
            TargetPlatform::WatchOSSimulator,
            TargetPlatform::VisionOS,
            TargetPlatform::VisionOSSimulator,
        ] {
            assert_eq!(
                platform.arch(),
                Architecture::Aarch64(Aarch64Architecture::Aarch64)
            );
            assert_eq!(platform.triple().architecture, platform.arch());
        }
        assert_eq!(
            TargetPlatform::IOSSimulator.triple().environment,
            Environment::Sim
        );
    }

    #[test]
    fn mac_catalyst_is_the_ios_triple_on_the_macabi_environment() {
        let triple = TargetPlatform::MacCatalyst.triple();
        assert_eq!(triple.operating_system, OperatingSystem::IOS(None));
        assert_eq!(triple.environment, Environment::Macabi);
        assert_eq!(triple.to_string(), "aarch64-apple-ios-macabi");
        // Catalyst builds against the macOS SDK's bridged iOS frameworks.
        assert_eq!(TargetPlatform::MacCatalyst.sdk_name(), Some("macosx"));
        assert_eq!(
            TargetPlatform::MacCatalyst.deployment_target_setting(),
            Some("IPHONEOS_DEPLOYMENT_TARGET")
        );
        // Packaging answers stay unanswered — Xcode destinations and device
        // families are #2103's job, so Catalyst fails fast on both.
        assert_eq!(TargetPlatform::MacCatalyst.xcode_destination_name(), None);
        assert_eq!(TargetPlatform::MacCatalyst.targeted_device_family(), None);
    }

    #[test]
    fn cross_compiling_backends_accept_any_platform_pairing() {
        // Android targets and the web frontend cross-compile — every
        // `water` command gates on this same check.
        assert!(
            TargetBackend::Hydrolysis
                .validate_host_support(TargetPlatform::Android)
                .is_ok()
        );
        assert!(
            TargetBackend::Hydrolysis
                .validate_host_support(TargetPlatform::Web)
                .is_ok()
        );
        assert!(
            TargetBackend::Android
                .validate_host_support(TargetPlatform::Android)
                .is_ok()
        );
    }

    #[test]
    fn hydrolysis_still_rejects_a_foreign_desktop_target() {
        #[cfg(target_os = "linux")]
        let foreign = TargetPlatform::MacOS;
        #[cfg(target_os = "macos")]
        let foreign = TargetPlatform::Linux;
        #[cfg(target_os = "windows")]
        let foreign = TargetPlatform::Linux;
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let foreign = TargetPlatform::Linux;

        assert!(
            TargetBackend::Hydrolysis
                .validate_host_support(foreign)
                .is_err()
        );
    }
}

#[cfg(test)]
mod package_options_tests {
    use super::PackageOptions;

    #[test]
    fn development_embeds_shared_runtime_and_packaging_does_not() {
        let development = PackageOptions::development();
        assert!(development.is_debug());
        assert!(!development.is_distribution());
        assert!(development.uses_shared_rust_runtime());

        for options in [
            PackageOptions::packaging(super::PackageAudience::Development, true),
            PackageOptions::packaging(super::PackageAudience::Distribution, false),
        ] {
            assert!(!options.uses_shared_rust_runtime());
        }
    }
}

#[cfg(test)]
mod tests {
    use target_lexicon::Triple;

    use super::NativeOs;

    /// Every host `dist` ships the CLI for is served by one desktop OS's
    /// generated-manifest table: a shipped target on an OS no table models
    /// would run a `water` whose generated manifests no section covers.
    #[test]
    fn every_shipped_host_is_served_by_a_native_table() {
        for target in env!("WATERUI_CLI_DIST_TARGETS").split_whitespace() {
            let triple: Triple = target.parse().expect("a dist target is a target triple");
            assert!(
                NativeOs::ALL
                    .into_iter()
                    .any(|os| os.serving_triples().contains(&triple)),
                "the shipped host {target} is served by no native table"
            );
        }
    }
}
