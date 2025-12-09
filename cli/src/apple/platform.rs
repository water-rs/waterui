//! Apple platform implementations for macOS, iOS, iOS Simulator, etc.

use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, bail};
use smol::fs;
use target_lexicon::{
    Aarch64Architecture, Architecture, DefaultToHost, Environment, OperatingSystem, Triple, Vendor,
};

use crate::{
    apple::{
        backend::AppleBackend,
        device::{AppleDevice, AppleSimulator},
        toolchain::{AppleSdk, AppleToolchain, Xcode},
    },
    build::{BuildOptions, RustBuild},
    device::Artifact,
    platform::{PackageOptions, Platform},
    project::{PackageType, Project},
    templates::{self, TemplateContext},
    utils::{copy_file, run_command},
};

// ============================================================================
// Common Apple Platform Trait
// ============================================================================

/// Trait for Apple-specific platform functionality.
///
/// This trait provides Apple-specific methods that are shared across all Apple platforms.
/// Each platform type (MacOS, Ios, IosSimulator, etc.) implements this trait.
pub trait ApplePlatformExt: Platform {
    /// Get the SDK name for xcodebuild (e.g., "macosx", "iphoneos", "iphonesimulator").
    fn sdk_name(&self) -> &'static str;

    /// Check if this platform is a simulator.
    fn is_simulator(&self) -> bool;

    /// Get the architecture for this platform.
    fn arch(&self) -> Architecture;
}

// ============================================================================
// Shared Implementation Helpers
// ============================================================================

/// Initialize the Apple backend for a playground project.
async fn init_playground_backend(project: &Project) -> eyre::Result<AppleBackend> {
    let manifest = project.manifest();

    // Derive app name from the display name (remove spaces for filesystem)
    let app_name = manifest
        .package
        .name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();

    let ctx = TemplateContext {
        app_display_name: manifest.package.name.clone(),
        app_name: app_name.clone(),
        crate_name: project.crate_name().to_string(),
        bundle_identifier: manifest.package.bundle_identifier.clone(),
        author: String::new(),
        android_backend_path: None,
        use_remote_dev_backend: manifest.waterui_path.is_none(),
        waterui_path: manifest.waterui_path.as_ref().map(PathBuf::from),
    };

    let project_path = PathBuf::from("apple");
    let output_dir = project.root().join(&project_path);

    templates::apple::scaffold(&output_dir, &ctx).await?;

    Ok(AppleBackend::new(app_name)
        .with_project_path(project_path)
        .with_backend_path(manifest.waterui_path.clone().unwrap_or_default()))
}

/// Get or initialize the Apple backend for a project.
async fn get_or_init_backend(project: &Project) -> eyre::Result<AppleBackend> {
    match project.apple_backend() {
        Some(backend) => Ok(backend.clone()),
        None => {
            if project.manifest().package.package_type == PackageType::Playground {
                init_playground_backend(project).await
            } else {
                bail!("Apple backend must be configured")
            }
        }
    }
}

/// Build Rust library for an Apple platform.
async fn build_rust_lib(
    project: &Project,
    triple: Triple,
    options: BuildOptions,
) -> eyre::Result<PathBuf> {
    let build = RustBuild::new(project.root(), triple);
    Ok(build.build_lib(options.is_release()).await?)
}

/// Clean Xcode build artifacts for an Apple platform.
async fn clean_apple(project: &Project) -> eyre::Result<()> {
    let backend = match project.apple_backend() {
        Some(backend) => backend.clone(),
        None => {
            if project.manifest().package.package_type == PackageType::Playground {
                return Ok(());
            }
            bail!("Apple backend must be configured")
        }
    };

    let project_path = project.root().join(backend.project_path());
    let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

    if !xcodeproj.exists() {
        return Ok(());
    }

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

    let build_dir = project_path.join("build");
    if build_dir.exists() {
        fs::remove_dir_all(&build_dir).await?;
    }

    Ok(())
}

/// Package an Apple app using xcodebuild.
async fn package_apple<P: ApplePlatformExt>(
    platform: &P,
    project: &Project,
    options: PackageOptions,
) -> eyre::Result<Artifact> {
    let backend = get_or_init_backend(project).await?;

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

    let derived_data = project_path.join(".water/DerivedData");

    // Copy the built Rust library to where Xcode expects it
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let lib_dir = project
        .root()
        .join("target")
        .join(platform.triple().to_string())
        .join(profile);
    let lib_name = project.crate_name().replace('-', "_");
    let source_lib = lib_dir.join(format!("lib{lib_name}.a"));

    let products_dir = derived_data.join("Build/Products").join(configuration);
    fs::create_dir_all(&products_dir).await?;
    let dest_lib = products_dir.join("libwaterui_app.a");
    copy_file(&source_lib, &dest_lib).await?;

    // Build with xcodebuild
    let mut args = vec![
        "-project",
        xcodeproj.to_str().unwrap_or_default(),
        "-scheme",
        &backend.scheme,
        "-configuration",
        configuration,
        "-sdk",
        platform.sdk_name(),
        "-derivedDataPath",
        derived_data.to_str().unwrap_or_default(),
        "build",
    ];

    if platform.is_simulator() || options.is_debug() {
        args.extend([
            "CODE_SIGNING_ALLOWED=NO",
            "CODE_SIGNING_REQUIRED=NO",
            "CODE_SIGN_IDENTITY=-",
        ]);
    }

    run_command("xcodebuild", args.iter().copied()).await?;

    // Reset the environment variable
    unsafe {
        env::set_var("WATERUI_SKIP_RUST_BUILD", "0");
    }

    let app_path = products_dir.join(format!("{}.app", backend.scheme));

    if !app_path.exists() {
        bail!(
            "Built app not found at {}. Check xcodebuild output for errors.",
            app_path.display()
        );
    }

    Ok(Artifact::new(project.bundle_identifier(), app_path))
}

// ============================================================================
// macOS Platform
// ============================================================================

/// macOS platform for building and running on the current Mac.
#[derive(Debug, Clone, Copy, Default)]
pub struct MacOS;

impl MacOS {
    /// Create a new macOS platform instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn arch(&self) -> Architecture {
        DefaultToHost::default().0.architecture
    }
}

impl Platform for MacOS {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        // macOS doesn't have simulators to scan
        Ok(vec![])
    }

    async fn build(&self, project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
        build_rust_lib(project, self.triple(), options).await
    }

    fn toolchain(&self) -> Self::Toolchain {
        (Xcode, AppleSdk::Macos)
    }

    fn triple(&self) -> Triple {
        Triple {
            architecture: self.arch(),
            vendor: Vendor::Apple,
            operating_system: OperatingSystem::Darwin(None),
            environment: Environment::Unknown,
            binary_format: target_lexicon::BinaryFormat::Macho,
        }
    }

    async fn clean(&self, project: &Project) -> eyre::Result<()> {
        clean_apple(project).await
    }

    async fn package(&self, project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
        package_apple(self, project, options).await
    }
}

impl ApplePlatformExt for MacOS {
    fn sdk_name(&self) -> &'static str {
        "macosx"
    }

    fn is_simulator(&self) -> bool {
        false
    }

    fn arch(&self) -> Architecture {
        self.arch()
    }
}

// ============================================================================
// iOS Platform (Physical Device)
// ============================================================================

/// iOS platform for building and running on physical iOS devices.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ios;

impl Ios {
    /// Create a new iOS platform instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for Ios {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        // TODO: Scan for physical iOS devices
        Ok(vec![])
    }

    async fn build(&self, project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
        build_rust_lib(project, self.triple(), options).await
    }

    fn toolchain(&self) -> Self::Toolchain {
        (Xcode, AppleSdk::Ios)
    }

    fn triple(&self) -> Triple {
        Triple {
            architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
            vendor: Vendor::Apple,
            operating_system: OperatingSystem::IOS(None),
            environment: Environment::Unknown,
            binary_format: target_lexicon::BinaryFormat::Macho,
        }
    }

    async fn clean(&self, project: &Project) -> eyre::Result<()> {
        clean_apple(project).await
    }

    async fn package(&self, project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
        package_apple(self, project, options).await
    }
}

impl ApplePlatformExt for Ios {
    fn sdk_name(&self) -> &'static str {
        "iphoneos"
    }

    fn is_simulator(&self) -> bool {
        false
    }

    fn arch(&self) -> Architecture {
        Architecture::Aarch64(Aarch64Architecture::Aarch64)
    }
}

// ============================================================================
// iOS Simulator Platform
// ============================================================================

/// iOS Simulator platform for building and running on the iOS Simulator.
#[derive(Debug, Clone, Copy, Default)]
pub struct IosSimulator;

impl IosSimulator {
    /// Create a new iOS Simulator platform instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn arch(&self) -> Architecture {
        DefaultToHost::default().0.architecture
    }
}

impl Platform for IosSimulator {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        let simulators = AppleSimulator::scan().await?;

        let filtered: Vec<AppleDevice> = simulators
            .into_iter()
            .filter(|sim| {
                // Filter to only iOS simulators (iPhone, iPad)
                !sim.device_type_identifier.contains("Apple-TV")
                    && !sim.device_type_identifier.contains("Apple-Watch")
                    && !sim.device_type_identifier.contains("Apple-Vision")
            })
            .map(AppleDevice::Simulator)
            .collect();

        Ok(filtered)
    }

    async fn build(&self, project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
        build_rust_lib(project, self.triple(), options).await
    }

    fn toolchain(&self) -> Self::Toolchain {
        (Xcode, AppleSdk::Ios)
    }

    fn triple(&self) -> Triple {
        let arch = self.arch();
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

    async fn clean(&self, project: &Project) -> eyre::Result<()> {
        clean_apple(project).await
    }

    async fn package(&self, project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
        package_apple(self, project, options).await
    }
}

impl ApplePlatformExt for IosSimulator {
    fn sdk_name(&self) -> &'static str {
        "iphonesimulator"
    }

    fn is_simulator(&self) -> bool {
        true
    }

    fn arch(&self) -> Architecture {
        self.arch()
    }
}

// ============================================================================
// Legacy ApplePlatform (for backwards compatibility)
// ============================================================================

/// Legacy Apple platform enum for backwards compatibility.
///
/// This struct wraps the platform kind and delegates to the appropriate
/// platform implementation. New code should use `MacOS`, `Ios`, or `IosSimulator` directly.
#[derive(Debug, Clone)]
pub struct ApplePlatform {
    arch: Architecture,
    kind: ApplePlatformKind,
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
    #[must_use]
    pub fn from_device_type_identifier(id: &str) -> Self {
        let is_simulator = id.contains("CoreSimulator");
        let arch = if is_simulator {
            DefaultToHost::default().0.architecture
        } else {
            Architecture::Aarch64(Aarch64Architecture::Aarch64)
        };

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
        } else if is_simulator {
            ApplePlatformKind::IosSimulator
        } else {
            ApplePlatformKind::Ios
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

impl Platform for ApplePlatform {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        let simulators = AppleSimulator::scan().await?;

        let filtered: Vec<AppleDevice> = simulators
            .into_iter()
            .filter(|sim| {
                let sim_platform = Self::from_device_type_identifier(&sim.device_type_identifier);
                matches!(
                    (&self.kind, &sim_platform.kind),
                    (
                        ApplePlatformKind::IosSimulator,
                        ApplePlatformKind::IosSimulator
                    ) | (
                        ApplePlatformKind::TvOsSimulator,
                        ApplePlatformKind::TvOsSimulator
                    ) | (
                        ApplePlatformKind::WatchOsSimulator,
                        ApplePlatformKind::WatchOsSimulator
                    ) | (
                        ApplePlatformKind::VisionOsSimulator,
                        ApplePlatformKind::VisionOsSimulator,
                    )
                )
            })
            .map(AppleDevice::Simulator)
            .collect();

        Ok(filtered)
    }

    async fn build(&self, project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
        build_rust_lib(project, self.triple(), options).await
    }

    fn toolchain(&self) -> Self::Toolchain {
        let sdk = match self.kind {
            ApplePlatformKind::MacOS => AppleSdk::Macos,
            ApplePlatformKind::Ios | ApplePlatformKind::IosSimulator => AppleSdk::Ios,
            ApplePlatformKind::TvOs | ApplePlatformKind::TvOsSimulator => AppleSdk::TvOs,
            ApplePlatformKind::WatchOs | ApplePlatformKind::WatchOsSimulator => AppleSdk::WatchOs,
            ApplePlatformKind::VisionOs | ApplePlatformKind::VisionOsSimulator => {
                AppleSdk::VisionOs
            }
        };
        (Xcode, sdk)
    }

    fn triple(&self) -> Triple {
        let (os, env) = match self.kind {
            ApplePlatformKind::MacOS => (OperatingSystem::Darwin(None), Environment::Unknown),
            ApplePlatformKind::Ios => (OperatingSystem::IOS(None), Environment::Unknown),
            ApplePlatformKind::IosSimulator => match self.arch {
                Architecture::X86_64 => (OperatingSystem::IOS(None), Environment::Unknown),
                _ => (OperatingSystem::IOS(None), Environment::Sim),
            },
            ApplePlatformKind::TvOs => (OperatingSystem::TvOS(None), Environment::Unknown),
            ApplePlatformKind::TvOsSimulator => (OperatingSystem::TvOS(None), Environment::Sim),
            ApplePlatformKind::WatchOs => (OperatingSystem::WatchOS(None), Environment::Unknown),
            ApplePlatformKind::WatchOsSimulator => {
                (OperatingSystem::WatchOS(None), Environment::Sim)
            }
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
        clean_apple(project).await
    }

    async fn package(&self, project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
        package_apple(self, project, options).await
    }
}

impl ApplePlatformExt for ApplePlatform {
    fn sdk_name(&self) -> &'static str {
        self.sdk_name()
    }

    fn is_simulator(&self) -> bool {
        self.is_simulator()
    }

    fn arch(&self) -> Architecture {
        self.arch
    }
}
