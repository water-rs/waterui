//! Platform abstraction for `WaterUI` CLI.

use std::str::FromStr;

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
    /// macOS (current machine architecture)
    MacOS,
    /// iOS (physical device, ARM64)
    IOS,
    /// iOS Simulator (host machine architecture)
    IOSSimulator,
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
    /// ESP32-S3 (Xtensa, ESP-IDF firmware via the Dew backend)
    Esp32S3,
    /// ESP32-C3 (RISC-V, ESP-IDF firmware via the Dew backend)
    Esp32C3,
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
    /// Dew backend (embedded-first CPU renderer for ESP32-class chips)
    Dew,
}

impl TargetPlatform {
    /// Get the target triple for this platform.
    ///
    /// # Panics
    /// May panic if `target_lexicon` cannot resolve the current host triple for simulator or host-native targets.
    #[must_use]
    pub fn triple(&self) -> Triple {
        match self {
            Self::MacOS => Triple {
                architecture: DefaultToHost::default().0.architecture,
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
            Self::IOSSimulator => {
                let arch = DefaultToHost::default().0.architecture;
                let env = match arch {
                    Architecture::X86_64 => Environment::Unknown,
                    _ => Environment::Sim,
                };
                Triple {
                    architecture: arch,
                    vendor: Vendor::Apple,
                    operating_system: OperatingSystem::IOS(None),
                    environment: env,
                    binary_format: target_lexicon::BinaryFormat::Macho,
                }
            }
            Self::TvOS => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::TvOS(None),
                environment: Environment::Unknown,
                binary_format: target_lexicon::BinaryFormat::Macho,
            },
            Self::TvOSSimulator => Triple {
                architecture: DefaultToHost::default().0.architecture,
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
                architecture: DefaultToHost::default().0.architecture,
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
                architecture: DefaultToHost::default().0.architecture,
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
        }
    }

    /// Get available backends for this platform.
    #[must_use]
    pub const fn available_backends(&self) -> &[TargetBackend] {
        match self {
            Self::MacOS => &[TargetBackend::Apple, TargetBackend::Hydrolysis],
            Self::IOS
            | Self::IOSSimulator
            | Self::TvOS
            | Self::TvOSSimulator
            | Self::WatchOS
            | Self::WatchOSSimulator
            | Self::VisionOS
            | Self::VisionOSSimulator => &[TargetBackend::Apple],
            Self::Android => &[TargetBackend::Android],
            Self::Linux => &[TargetBackend::Gtk4, TargetBackend::Hydrolysis],
            Self::Windows | Self::Web => &[TargetBackend::Hydrolysis],
            Self::Esp32S3 | Self::Esp32C3 => &[TargetBackend::Dew],
        }
    }

    /// Get the default backend for this platform.
    #[must_use]
    pub const fn default_backend(&self) -> TargetBackend {
        match self {
            Self::MacOS
            | Self::IOS
            | Self::IOSSimulator
            | Self::TvOS
            | Self::TvOSSimulator
            | Self::WatchOS
            | Self::WatchOSSimulator
            | Self::VisionOS
            | Self::VisionOSSimulator => TargetBackend::Apple,
            Self::Android => TargetBackend::Android,
            Self::Linux => TargetBackend::Gtk4,
            Self::Windows | Self::Web => TargetBackend::Hydrolysis,
            Self::Esp32S3 | Self::Esp32C3 => TargetBackend::Dew,
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
            Self::MacOS => Some("macosx"),
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
            | Self::Esp32C3 => None,
        }
    }

    /// Get the architecture for this platform.
    #[must_use]
    pub fn arch(&self) -> Architecture {
        match self {
            Self::MacOS
            | Self::IOSSimulator
            | Self::TvOSSimulator
            | Self::WatchOSSimulator
            | Self::VisionOSSimulator
            | Self::Linux
            | Self::Windows => DefaultToHost::default().0.architecture,
            Self::IOS | Self::TvOS | Self::WatchOS | Self::VisionOS | Self::Android => {
                Architecture::Aarch64(Aarch64Architecture::Aarch64)
            }
            Self::Web => Architecture::Wasm32,
            Self::Esp32S3 => Architecture::XTensa,
            Self::Esp32C3 => Architecture::Riscv32(Riscv32Architecture::Riscv32imc),
        }
    }
}

// ============================================================================
// Package Options
// ============================================================================

/// Configuration options for packaging the application.
///
/// This struct contains settings that control how the application
/// is packaged for distribution across different platforms.
#[derive(Debug, Clone)]
pub struct PackageOptions {
    /// Whether to prepare the package for store distribution.
    ///
    /// When `true`, the package will be configured for submission to
    /// official app stores (App Store for iOS/macOS or Play Store for Android).
    ///
    /// When `false`, the package will be prepared for direct distribution
    /// or development purposes.
    ///
    /// # Warning
    ///
    /// Enable this option only change your packaging format, it does not change your build configuration.
    /// For a real world distribution build, you may also want to disable `debug` in `BuildOptions`.
    distribution: bool,

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
}

impl PackageOptions {
    /// Create new package options
    #[must_use]
    pub const fn new(distribution: bool, debug: bool) -> Self {
        Self {
            distribution,
            debug,
        }
    }

    /// Whether to package in distribution mode
    #[must_use]
    pub const fn is_distribution(&self) -> bool {
        self.distribution
    }

    /// Whether to package in debug mode
    #[must_use]
    pub const fn is_debug(&self) -> bool {
        self.debug
    }
}
