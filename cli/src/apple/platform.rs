use std::env;

use color_eyre::eyre::{self, bail};
use smol::fs;
use target_lexicon::{
    Aarch64Architecture, Architecture, DefaultToHost, Environment, OperatingSystem, Triple, Vendor,
};

use crate::{
    apple::{
        device::{AppleDevice, AppleSimulator},
        toolchain::{AppleSdk, AppleToolchain, Xcode},
    },
    build::{BuildOptions, RustBuild},
    device::Artifact,
    platform::Platform,
    project::Project,
    utils::{copy_file, run_command},
};

/// Represents an Apple platform (macOS, iOS, iOS Simulator, etc.)
#[derive(Debug, Clone)]
pub struct ApplePlatform {
    arch: Architecture,
    kind: ApplePlatformKind,
}

impl ApplePlatform {
    /// Create a new Apple platform with the specified architecture and kind.
    #[must_use]
    pub const fn new(arch: Architecture, kind: ApplePlatformKind) -> Self {
        Self { arch, kind }
    }

    /// Create an Apple platform for the current macOS host.
    #[must_use]
    pub fn macos() -> Self {
        Self {
            arch: DefaultToHost::default().0.architecture,
            kind: ApplePlatformKind::MacOS,
        }
    }

    /// Create an Apple platform for iOS (physical device).
    #[must_use]
    pub const fn ios() -> Self {
        Self {
            arch: Architecture::Aarch64(Aarch64Architecture::Aarch64),
            kind: ApplePlatformKind::Ios,
        }
    }

    /// Create an Apple platform for iOS Simulator.
    #[must_use]
    pub fn ios_simulator() -> Self {
        Self {
            arch: DefaultToHost::default().0.architecture,
            kind: ApplePlatformKind::IosSimulator,
        }
    }

    /// Parse a device type identifier to determine the platform.
    ///
    /// Device type identifiers look like:
    /// - `com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro`
    /// - `com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-10-46mm`
    /// - `com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K`
    /// - `com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro`
    #[must_use]
    pub fn from_device_type_identifier(id: &str) -> Self {
        // If it is a simulator, then it has the same architecture as the host machine
        // Otherwise, it is an actual device, which is always arm64
        let is_simulator = id.contains("CoreSimulator");
        let arch = if is_simulator {
            DefaultToHost::default().0.architecture
        } else {
            Architecture::Aarch64(Aarch64Architecture::Aarch64)
        };

        // Determine the platform kind based on device type identifier
        let kind = if id.contains("Apple-TV") {
            if is_simulator {
                ApplePlatformKind::TvOsSimulator
            } else {
                ApplePlatformKind::TvOs
            }
        } else if id.contains("Apple-Watch") {
            if is_simulator {
                ApplePlatformKind::WatchOsSimulator
            } else {
                ApplePlatformKind::WatchOs
            }
        } else if id.contains("Apple-Vision") {
            if is_simulator {
                ApplePlatformKind::VisionOsSimulator
            } else {
                ApplePlatformKind::VisionOs
            }
        } else if id.contains("Mac") {
            ApplePlatformKind::MacOS
        } else {
            // Default to iOS/iOS Simulator for iPhone, iPad, etc.
            if is_simulator {
                ApplePlatformKind::IosSimulator
            } else {
                ApplePlatformKind::Ios
            }
        };

        Self { arch, kind }
    }

    /// Get the platform kind.
    #[must_use]
    pub const fn kind(&self) -> &ApplePlatformKind {
        &self.kind
    }

    /// Get the architecture.
    #[must_use]
    pub const fn arch(&self) -> &Architecture {
        &self.arch
    }

    /// Get the SDK name for xcodebuild.
    #[must_use]
    pub const fn sdk_name(&self) -> &'static str {
        match self.kind {
            ApplePlatformKind::MacOS => "macosx",
            ApplePlatformKind::Ios => "iphoneos",
            ApplePlatformKind::IosSimulator => "iphonesimulator",
            ApplePlatformKind::TvOs => "appletvos",
            ApplePlatformKind::TvOsSimulator => "appletvsimulator",
            ApplePlatformKind::WatchOs => "watchos",
            ApplePlatformKind::WatchOsSimulator => "watchsimulator",
            ApplePlatformKind::VisionOs => "xros",
            ApplePlatformKind::VisionOsSimulator => "xrsimulator",
        }
    }

    /// Check if this platform is a simulator.
    #[must_use]
    pub const fn is_simulator(&self) -> bool {
        matches!(
            self.kind,
            ApplePlatformKind::IosSimulator
                | ApplePlatformKind::TvOsSimulator
                | ApplePlatformKind::WatchOsSimulator
                | ApplePlatformKind::VisionOsSimulator
        )
    }
}

/// Apple platform kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplePlatformKind {
    /// macOS
    MacOS,
    /// iOS (physical device)
    Ios,
    /// iOS Simulator
    IosSimulator,
    /// tvOS (physical device)
    TvOs,
    /// tvOS Simulator
    TvOsSimulator,
    /// watchOS (physical device)
    WatchOs,
    /// watchOS Simulator
    WatchOsSimulator,
    /// visionOS (physical device)
    VisionOs,
    /// visionOS Simulator
    VisionOsSimulator,
}

impl Platform for ApplePlatform {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        // Scan for simulators matching this platform
        let simulators = AppleSimulator::scan().await?;

        // Filter simulators by platform kind
        let filtered: Vec<AppleDevice> = simulators
            .into_iter()
            .filter(|sim| {
                let sim_platform = Self::from_device_type_identifier(&sim.device_type_identifier);
                // Match simulator platforms
                match (&self.kind, &sim_platform.kind) {
                    (ApplePlatformKind::IosSimulator, ApplePlatformKind::IosSimulator) => true,
                    (ApplePlatformKind::TvOsSimulator, ApplePlatformKind::TvOsSimulator) => true,
                    (ApplePlatformKind::WatchOsSimulator, ApplePlatformKind::WatchOsSimulator) => {
                        true
                    }
                    (
                        ApplePlatformKind::VisionOsSimulator,
                        ApplePlatformKind::VisionOsSimulator,
                    ) => true,
                    _ => false,
                }
            })
            .map(AppleDevice::Simulator)
            .collect();

        Ok(filtered)
    }

    async fn build(
        &self,
        project: &Project,
        options: BuildOptions,
    ) -> eyre::Result<std::path::PathBuf> {
        let build = RustBuild::new(project.root(), self.triple());
        // build_lib now returns the directory containing the built library (target/{triple}/{profile}/)
        let lib_dir = build.build_lib(options.is_release()).await?;

        // Get the crate name and convert to library filename
        let lib_name = project.crate_name().replace('-', "_");
        let source_lib = lib_dir.join(format!("lib{lib_name}.a"));

        if !source_lib.exists() {
            bail!(
                "Rust static library not found at {}. Did the build succeed?",
                source_lib.display()
            );
        }

        // Get the Apple backend configuration
        let backend = project
            .apple_backend()
            .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;

        // Create the output directory in the Apple project
        let output_dir = project.root().join(backend.project_path()).join("build");
        fs::create_dir_all(&output_dir).await?;

        // Copy with reflink when available for efficiency
        let dest_lib = output_dir.join("libwaterui_app.a");
        copy_file(&source_lib, &dest_lib).await?;

        Ok(lib_dir)
    }

    fn toolchain(&self) -> Self::Toolchain {
        let sdk = match self.kind {
            ApplePlatformKind::MacOS => AppleSdk::Macos,
            ApplePlatformKind::Ios | ApplePlatformKind::IosSimulator => AppleSdk::Ios,
            ApplePlatformKind::TvOs | ApplePlatformKind::TvOsSimulator => AppleSdk::TvOs,
            ApplePlatformKind::WatchOs | ApplePlatformKind::WatchOsSimulator => AppleSdk::WatchOs,
            ApplePlatformKind::VisionOs | ApplePlatformKind::VisionOsSimulator => AppleSdk::VisionOs,
        };
        (Xcode, sdk)
    }

    fn triple(&self) -> Triple {
        let (os, env) = match self.kind {
            ApplePlatformKind::MacOS => (OperatingSystem::Darwin(None), Environment::Unknown),
            ApplePlatformKind::Ios => (OperatingSystem::IOS(None), Environment::Unknown),
            ApplePlatformKind::IosSimulator => {
                // iOS simulator uses a different triple suffix
                match self.arch {
                    Architecture::X86_64 => {
                        // x86_64-apple-ios (simulator uses same as device for x86_64)
                        (OperatingSystem::IOS(None), Environment::Unknown)
                    }
                    _ => {
                        // aarch64-apple-ios-sim
                        (OperatingSystem::IOS(None), Environment::Sim)
                    }
                }
            }
            ApplePlatformKind::TvOs => (OperatingSystem::TvOS(None), Environment::Unknown),
            ApplePlatformKind::TvOsSimulator => (OperatingSystem::TvOS(None), Environment::Sim),
            ApplePlatformKind::WatchOs => (OperatingSystem::WatchOS(None), Environment::Unknown),
            ApplePlatformKind::WatchOsSimulator => (OperatingSystem::WatchOS(None), Environment::Sim),
            ApplePlatformKind::VisionOs => (OperatingSystem::VisionOS(None), Environment::Unknown),
            ApplePlatformKind::VisionOsSimulator => {
                (OperatingSystem::VisionOS(None), Environment::Sim)
            }
        };

        Triple {
            architecture: self.arch,
            vendor: Vendor::Apple,
            operating_system: os,
            environment: env,
            binary_format: target_lexicon::BinaryFormat::Macho,
        }
    }

    async fn clean(&self, project: &Project) -> eyre::Result<()> {
        let backend = project
            .apple_backend()
            .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;

        let project_path = project.root().join(backend.project_path());

        // Find the Xcode project file
        let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

        if !xcodeproj.exists() {
            // No Xcode project to clean
            return Ok(());
        }

        // Run xcodebuild clean
        run_command(
            "xcodebuild",
            [
                "-project",
                xcodeproj.to_str().unwrap_or_default(),
                "-scheme",
                &backend.scheme,
                "clean",
            ],
        )
        .await?;

        // Also clean the build directory
        let build_dir = project_path.join("build");
        if build_dir.exists() {
            fs::remove_dir_all(&build_dir).await?;
        }

        Ok(())
    }

    async fn package(
        &self,
        project: &Project,
        options: crate::platform::PackageOptions,
    ) -> eyre::Result<Artifact> {
        let backend = project
            .apple_backend()
            .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;

        let project_path = project.root().join(backend.project_path());
        let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

        if !xcodeproj.exists() {
            bail!(
                "Xcode project not found at {}. Did you run 'water create'?",
                xcodeproj.display()
            );
        }

        // Tell Xcode not to call `water build` again (we already built)
        // SAFETY: CLI runs on main thread before spawning build processes
        unsafe {
            env::set_var("WATERUI_SKIP_RUST_BUILD", "1");
        }

        let configuration = if options.is_debug() {
            "Debug"
        } else {
            "Release"
        };

        // Determine derived data path for finding the built product
        let derived_data = project_path.join(".water/DerivedData");

        // Build with xcodebuild
        let mut args = vec![
            "-project",
            xcodeproj.to_str().unwrap_or_default(),
            "-scheme",
            &backend.scheme,
            "-configuration",
            configuration,
            "-sdk",
            self.sdk_name(),
            "-derivedDataPath",
            derived_data.to_str().unwrap_or_default(),
            "build",
        ];

        // Disable code signing for simulators and debug builds
        if self.is_simulator() || options.is_debug() {
            args.extend([
                "CODE_SIGNING_ALLOWED=NO",
                "CODE_SIGNING_REQUIRED=NO",
                "CODE_SIGN_IDENTITY=-",
            ]);
        }

        run_command("xcodebuild", args.iter().copied()).await?;

        // Reset the environment variable
        // SAFETY: CLI runs on main thread
        unsafe {
            env::set_var("WATERUI_SKIP_RUST_BUILD", "0");
        }

        // Find the built .app bundle
        let products_dir = derived_data.join("Build/Products").join(format!(
            "{}-{}",
            configuration,
            self.sdk_name()
        ));

        let app_path = products_dir.join(format!("{}.app", backend.scheme));

        if !app_path.exists() {
            bail!(
                "Built app not found at {}. Check xcodebuild output for errors.",
                app_path.display()
            );
        }

        Ok(Artifact::new(project.bundle_identifier(), app_path))
    }
}
