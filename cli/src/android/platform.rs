//! Android platform build and package utilities.
//!
//! This module provides utility functions for building and packaging Android apps.
//! These functions are used by `AndroidBackend` to implement the `Backend` trait.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, bail};
use smol::{fs, unblock};
use target_lexicon::{Aarch64Architecture, Architecture, Triple};

use tracing::{debug, info};

use std::str::FromStr;

use crate::{
    android::{
        backend::AndroidBackend,
        toolchain::{AndroidNdk, AndroidSdk, Java, Kotlin},
    },
    assets::{self, ResolvedFont},
    build::{BuildOptions, RustBuild},
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::copy_file,
};

const ANDROID_MIN_API_LEVEL: u32 = 24;

fn gradle_cmd(gradlew: &Path, backend_path: &Path, task: &str) -> smol::process::Command {
    let mut cmd = smol::process::Command::new(gradlew);
    cmd.arg(task).arg("--project-dir").arg(backend_path);
    cmd
}

fn validate_android_package_name(package: &str) -> eyre::Result<()> {
    if package.is_empty() {
        bail!("Android package name is empty (set `[package].bundle_identifier` in `Water.toml`).");
    }

    if package.contains('-') {
        bail!(
            "Invalid Android package name: '{package}' (hyphens are not allowed). \
Set `[package].bundle_identifier` in `Water.toml` to a valid Java package name (e.g. replace '-' with '_')."
        );
    }

    for segment in package.split('.') {
        if segment.is_empty() {
            bail!("Invalid Android package name: '{package}' (empty segment).");
        }

        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            bail!("Invalid Android package name: '{package}' (empty segment).");
        };

        if !(first.is_ascii_alphabetic() || first == '_') {
            bail!(
                "Invalid Android package name: '{package}' (segment '{segment}' must start with a letter or underscore)."
            );
        }

        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!(
                "Invalid Android package name: '{package}' (segment '{segment}' contains invalid characters)."
            );
        }
    }

    Ok(())
}

/// Get the NDK host tag based on the current machine's OS and architecture.
///
/// On Apple Silicon, prefer the native `darwin-arm64` toolchain when present,
/// falling back to `darwin-x86_64` for older NDKs (Rosetta).
fn ndk_host_tag(ndk_path: &Path) -> &'static str {
    use target_lexicon::{Architecture, OperatingSystem, Triple};

    let host = Triple::host();

    match (&host.operating_system, &host.architecture) {
        (OperatingSystem::Darwin(_), Architecture::Aarch64(_)) => {
            let native = ndk_path
                .join("toolchains/llvm/prebuilt")
                .join("darwin-arm64");
            if native.exists() {
                "darwin-arm64"
            } else {
                "darwin-x86_64"
            }
        }
        (OperatingSystem::Darwin(_), _) => "darwin-x86_64",
        (OperatingSystem::Windows, _) => "windows-x86_64",
        // NDK doesn't have native ARM64 Linux builds
        (OperatingSystem::Linux, _) => "linux-x86_64",
        _ => panic!("Unsupported host triple for Android NDK: {host}"),
    }
}

fn ndk_bin_dir(ndk_path: &Path) -> PathBuf {
    ndk_path
        .join("toolchains/llvm/prebuilt")
        .join(ndk_host_tag(ndk_path))
        .join("bin")
}

/// Get the NDK ar path.
fn ndk_ar_path(ndk_path: &Path) -> PathBuf {
    ndk_bin_dir(ndk_path).join("llvm-ar")
}

fn ndk_clang_path(ndk_path: &Path, abi: AndroidAbi, cxx: bool) -> PathBuf {
    let suffix = if cxx { "clang++" } else { "clang" };
    ndk_bin_dir(ndk_path).join(format!(
        "{}{ANDROID_MIN_API_LEVEL}-{suffix}",
        abi.ndk_target()
    ))
}

/// Get the NDK clang linker path for the given ABI.
fn ndk_linker_path(ndk_path: &Path, abi: AndroidAbi) -> PathBuf {
    ndk_clang_path(ndk_path, abi, false)
}

/// Find the android.jar from any installed Android platform.
/// Returns the path to android.jar from the highest installed API level.
fn find_android_jar(sdk_path: &Path) -> Option<PathBuf> {
    let platforms_dir = sdk_path.join("platforms");
    if !platforms_dir.exists() {
        return None;
    }

    // Find all installed platforms and sort by API level descending
    let mut platforms: Vec<PathBuf> = std::fs::read_dir(&platforms_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();

    // Sort by API level (android-XX format) - highest first
    platforms.sort_by(|a, b| {
        let get_api_level = |p: &Path| -> u32 {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("android-"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        get_api_level(b).cmp(&get_api_level(a))
    });

    // Find first platform with android.jar
    for platform in platforms {
        let android_jar = platform.join("android.jar");
        if android_jar.exists() {
            return Some(android_jar);
        }
    }

    None
}

/// Create a wrapper `CMake` toolchain file that sets `ANDROID_ABI` before including
/// the NDK's toolchain. This is required because cmake-rs doesn't pass `ANDROID_ABI`
/// as a -D define, causing the NDK toolchain to default to armeabi-v7a.
///
/// Returns the path to the created wrapper toolchain file.
async fn create_android_toolchain_wrapper(
    ndk_path: &Path,
    abi: AndroidAbi,
) -> eyre::Result<PathBuf> {
    // Create wrapper in a temp directory that persists for the build
    let wrapper_dir = std::env::temp_dir().join("waterui-cmake-toolchains");
    fs::create_dir_all(&wrapper_dir).await?;

    let wrapper_path = wrapper_dir.join(format!("android-{}.cmake", abi.as_str()));
    let ndk_toolchain = ndk_path.join("build/cmake/android.toolchain.cmake");

    let content = format!(
        include_str!("android_toolchain_wrapper.cmake.tpl"),
        abi = abi.as_str(),
        api_level = ANDROID_MIN_API_LEVEL,
        ndk_toolchain = ndk_toolchain.display(),
        asm_compiler = ndk_clang_path(ndk_path, abi, false).display(),
    );
    fs::write(&wrapper_path, content).await?;

    Ok(wrapper_path)
}

/// Get the NDK clang++ (C++ compiler) path for the given ABI.
fn ndk_cxx_path(ndk_path: &Path, abi: AndroidAbi) -> PathBuf {
    ndk_clang_path(ndk_path, abi, true)
}

/// Get the path to `libc++_shared.so` in the NDK.
///
/// NDK r23+ ships it under `sysroot/usr/lib/<triple>/`, while older NDKs
/// used `sources/cxx-stl/llvm-libc++/libs/<abi>/`.
fn ndk_libcxx_path(ndk_path: &Path, abi: AndroidAbi) -> PathBuf {
    let new_path = ndk_path
        .join("toolchains/llvm/prebuilt")
        .join(ndk_host_tag(ndk_path))
        .join("sysroot/usr/lib")
        .join(abi.ndk_libcxx_triple())
        .join("libc++_shared.so");

    if new_path.exists() {
        return new_path;
    }

    ndk_path
        .join("sources/cxx-stl/llvm-libc++/libs")
        .join(abi.as_str())
        .join("libc++_shared.so")
}

/// Represents an Android platform for a specific architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AndroidAbi {
    /// ARM64 (arm64-v8a) - modern Android devices
    Arm64V8a,
    /// `x86_64` - emulators on Intel/AMD
    X86_64,
    /// `ARMv7` (armeabi-v7a) - older 32-bit devices
    ArmeabiV7a,
    /// x86 - older 32-bit emulators
    X86,
}

/// Error returned when parsing an unsupported Android ABI string.
#[derive(Debug, thiserror::Error)]
#[error("Unsupported Android ABI: {abi}")]
pub struct UnsupportedAndroidAbi {
    abi: String,
}

impl FromStr for AndroidAbi {
    type Err = UnsupportedAndroidAbi;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "arm64-v8a" => Ok(Self::Arm64V8a),
            "x86_64" => Ok(Self::X86_64),
            "armeabi-v7a" => Ok(Self::ArmeabiV7a),
            "x86" => Ok(Self::X86),
            other => Err(UnsupportedAndroidAbi {
                abi: other.to_string(),
            }),
        }
    }
}

impl AndroidAbi {
    #[must_use]
    /// Android ABI string used by the Android toolchain (e.g. `arm64-v8a`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Arm64V8a => "arm64-v8a",
            Self::X86_64 => "x86_64",
            Self::ArmeabiV7a => "armeabi-v7a",
            Self::X86 => "x86",
        }
    }

    #[must_use]
    /// Target triple prefix used by the NDK toolchain binaries (clang, clang++).
    pub const fn ndk_target(self) -> &'static str {
        match self {
            Self::Arm64V8a => "aarch64-linux-android",
            Self::X86_64 => "x86_64-linux-android",
            Self::ArmeabiV7a => "armv7a-linux-androideabi",
            Self::X86 => "i686-linux-android",
        }
    }

    #[must_use]
    /// Target triple used by NDK sysroot libc++ paths.
    pub const fn ndk_libcxx_triple(self) -> &'static str {
        match self {
            Self::Arm64V8a => "aarch64-linux-android",
            Self::X86_64 => "x86_64-linux-android",
            Self::ArmeabiV7a => "arm-linux-androideabi",
            Self::X86 => "i686-linux-android",
        }
    }
}

/// Represents an Android platform for a specific ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AndroidPlatform {
    abi: AndroidAbi,
}

impl AndroidPlatform {
    /// Create a new Android platform with the specified ABI.
    #[must_use]
    pub const fn new(abi: AndroidAbi) -> Self {
        Self { abi }
    }

    /// Create an Android platform for arm64-v8a (most common modern Android devices).
    #[must_use]
    pub const fn arm64() -> Self {
        Self {
            abi: AndroidAbi::Arm64V8a,
        }
    }

    /// Create an Android platform for `x86_64` (emulators on Intel/AMD).
    #[must_use]
    pub const fn x86_64() -> Self {
        Self {
            abi: AndroidAbi::X86_64,
        }
    }

    #[must_use]
    /// Return the ABI for this platform.
    pub const fn abi(&self) -> AndroidAbi {
        self.abi
    }

    #[must_use]
    /// Return the ABI string for this platform.
    pub const fn abi_str(&self) -> &'static str {
        self.abi.as_str()
    }

    /// Create an Android platform from an ABI string.
    ///
    /// # Errors
    /// Returns an error if the ABI is not supported.
    pub fn try_from_abi(abi: &str) -> eyre::Result<Self> {
        let abi = AndroidAbi::from_str(abi).map_err(|e| eyre::eyre!(e))?;
        Ok(Self { abi })
    }
}

/// All supported Android ABIs.
pub const ALL_ABIS: &[AndroidAbi] = &[
    AndroidAbi::Arm64V8a,
    AndroidAbi::X86_64,
    AndroidAbi::ArmeabiV7a,
    AndroidAbi::X86,
];

impl AndroidPlatform {
    /// Returns all supported Android platforms (all architectures).
    #[must_use]
    pub fn all() -> Vec<Self> {
        ALL_ABIS.iter().copied().map(Self::new).collect()
    }

    /// Get the target triple for this Android platform.
    #[must_use]
    pub fn triple(&self) -> Triple {
        let architecture = match self.abi {
            AndroidAbi::Arm64V8a => Architecture::Aarch64(Aarch64Architecture::Aarch64),
            AndroidAbi::X86_64 => Architecture::X86_64,
            AndroidAbi::ArmeabiV7a => Architecture::Arm(target_lexicon::ArmArchitecture::Armv7),
            AndroidAbi::X86 => Architecture::X86_32(target_lexicon::X86_32Architecture::I686),
        };
        Triple {
            architecture,
            vendor: target_lexicon::Vendor::Unknown,
            operating_system: target_lexicon::OperatingSystem::Linux,
            environment: target_lexicon::Environment::Android,
            binary_format: target_lexicon::BinaryFormat::Elf,
        }
    }

    /// Build Rust library for this Android platform.
    ///
    /// # Errors
    /// Returns an error if the build fails.
    pub async fn build(&self, project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
        // Resolve fonts BEFORE cargo build - this ensures icons.json is downloaded
        // for crates like fontawesome7 that need it during build.rs
        let font_declarations = crate::assets::scan_fonts(project).await?;
        let _resolved_fonts = crate::assets::resolve_fonts(font_declarations).await?;

        let abi = self.abi();
        let triple = self.triple();

        // Get NDK path for configuring the linker
        let ndk_path = AndroidNdk::detect_path().ok_or_else(|| {
            eyre::eyre!("Android NDK not found. Please install it via Android Studio.")
        })?;

        // Configure NDK environment for cargo
        let linker = ndk_linker_path(&ndk_path, abi);
        let ar = ndk_ar_path(&ndk_path);
        let cxx = ndk_cxx_path(&ndk_path, abi);

        let target_underscore = triple.to_string().replace('-', "_");
        let target_upper = target_underscore.to_uppercase();

        // Build with RustBuild
        // Enable android-jni feature for waterui-ffi to generate JNI bindings in Rust
        let mut build =
            RustBuild::new(project.root(), triple.clone()).with_feature("waterui-ffi/android-jni");
        if let Some(sccache_path) = options.sccache_path() {
            build = build.with_sccache(sccache_path.to_path_buf());
        }

        // Detect Kotlin path before entering unsafe block (detect_path is async)
        let kotlin_bin_dir = Kotlin::detect_path()
            .await
            .and_then(|p| p.parent().map(PathBuf::from));

        let sdk_path = AndroidSdk::detect_path().ok_or_else(|| {
            eyre::eyre!("Android SDK not found. Please install it via Android Studio.")
        })?;
        let sdk_path_for_jar = sdk_path.clone();
        let android_jar = unblock(move || find_android_jar(&sdk_path_for_jar))
            .await
            .ok_or_else(|| {
                eyre::eyre!(
                    "Android platforms not found in SDK at {}. Install an Android platform (SDK) in Android Studio.",
                    sdk_path.display()
                )
            })?;

        let wrapper_toolchain = create_android_toolchain_wrapper(&ndk_path, abi).await?;
        let android_platform = format!("android-{ANDROID_MIN_API_LEVEL}");

        build = build
            // For cargo/rustc linker
            .with_env(
                format!("CARGO_TARGET_{target_upper}_LINKER"),
                linker.as_os_str(),
            )
            .with_env(format!("CARGO_TARGET_{target_upper}_AR"), ar.as_os_str())
            // For cc-rs crate (used by ring, aws-lc-sys, etc.) - uses underscore format
            .with_env(format!("CC_{target_underscore}"), linker.as_os_str())
            .with_env(format!("CXX_{target_underscore}"), cxx.as_os_str())
            .with_env(format!("AR_{target_underscore}"), ar.as_os_str())
            // Keep generic host compiler variables untouched so host build scripts
            // continue to use the native compiler during cross-compiles.
            .with_env("ASM", linker.as_os_str())
            // For CMake-based builds (aws-lc-sys, etc.)
            .with_env("ANDROID_NDK", ndk_path.as_os_str())
            .with_env("ANDROID_NDK_HOME", ndk_path.as_os_str())
            .with_env("ANDROID_NDK_ROOT", ndk_path.as_os_str())
            // Android SDK environment variables (needed by waterkit and other crates)
            .with_env("ANDROID_HOME", sdk_path.as_os_str())
            .with_env("ANDROID_SDK_ROOT", sdk_path.as_os_str())
            .with_env("ANDROID_JAR", android_jar.as_os_str())
            // CMake toolchain wrapper
            .with_env("CMAKE_TOOLCHAIN_FILE", wrapper_toolchain.as_os_str())
            .with_env(
                format!("CMAKE_TOOLCHAIN_FILE_{target_underscore}"),
                wrapper_toolchain.as_os_str(),
            )
            .with_env("CMAKE_ASM_COMPILER", linker.as_os_str())
            .with_env("ANDROID_ABI", abi.as_str())
            .with_env("ANDROID_PLATFORM", android_platform)
            // Allow pkg-config probes that are intentionally scoped to cross targets.
            // This is required by cross-target native dependency build scripts.
            .with_env("PKG_CONFIG_ALLOW_CROSS", "1")
            .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{target_underscore}"), "1")
            .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{}", triple), "1");

        // Add required toolchains to PATH for transitive native builds (CMake/asm/Kotlin).
        let current_path = std::env::var_os("PATH")
            .ok_or_else(|| eyre::eyre!("PATH environment variable is not set"))?;
        let mut paths: Vec<PathBuf> = std::env::split_paths(&current_path).collect();
        if let Some(kotlin_bin) = &kotlin_bin_dir {
            paths.insert(0, kotlin_bin.clone());
        }
        let new_path = std::env::join_paths(paths)
            .map_err(|e| eyre::eyre!("Failed to construct PATH for Android build tools: {e}"))?;
        build = build.with_env("PATH", new_path);

        let lib_dir = build.build_lib(options.is_release()).await?;

        // Get the crate name and find the built .so file
        let lib_name = project.crate_name().replace('-', "_");
        let source_lib = lib_dir.join(format!("lib{lib_name}.so"));

        if !source_lib.exists() {
            bail!(
                "Rust shared library not found at {}. Did the build succeed?",
                source_lib.display()
            );
        }

        // Determine output directory: use specified output_dir or default to jniLibs
        let output_dir = options.output_dir().map_or_else(
            || {
                project
                    .backend_path::<AndroidBackend>()
                    .join("app/src/main/jniLibs")
                    .join(abi.as_str())
            },
            std::path::Path::to_path_buf,
        );
        fs::create_dir_all(&output_dir).await?;

        // Copy with standardized name
        let dest_lib = output_dir.join("libwaterui_app.so");
        copy_file(&source_lib, &dest_lib).await?;

        // Some dependencies (e.g., C++-backed crates) dynamically link against
        // `libc++_shared.so`. Bundle it so apps don't fail at dlopen time.
        let libcxx_path = ndk_libcxx_path(&ndk_path, abi);
        if libcxx_path.exists() {
            let dest_libcxx = output_dir.join("libc++_shared.so");
            copy_file(&libcxx_path, &dest_libcxx).await?;
        }

        Ok(lib_dir)
    }

    /// Clean all jniLibs directories to remove stale libraries from previous builds.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be removed.
    pub async fn clean_jni_libs(project: &Project) -> eyre::Result<()> {
        let jni_libs_dir = project
            .backend_path::<AndroidBackend>()
            .join("app/src/main/jniLibs");

        if jni_libs_dir.exists() {
            fs::remove_dir_all(&jni_libs_dir).await?;
        }
        Ok(())
    }

    /// Package the Android app with specific ABIs.
    ///
    /// This is used when building for multiple architectures. The ABIs parameter
    /// controls which native libraries are included in the final APK.
    ///
    /// # Errors
    /// Returns an error if Gradle build fails.
    pub async fn package_with_abis(
        project: &Project,
        options: PackageOptions,
        abis: &[AndroidAbi],
    ) -> eyre::Result<Artifact> {
        validate_android_package_name(project.bundle_identifier())?;

        let backend_path = project.backend_path::<AndroidBackend>();

        // Copy project assets and dependency fonts
        copy_assets_and_fonts(project, &backend_path).await?;

        let gradlew = backend_path.join(if cfg!(windows) {
            "gradlew.bat"
        } else {
            "gradlew"
        });

        let (command_name, path) = if options.is_distribution() && !options.is_debug() {
            (
                "bundleRelease",
                backend_path.join("app/build/outputs/bundle/release/app-release.aab"),
            )
        } else if !options.is_distribution() && !options.is_debug() {
            (
                "assembleRelease",
                backend_path.join("app/build/outputs/apk/release/app-release.apk"),
            )
        } else if !options.is_distribution() && options.is_debug() {
            (
                "assembleDebug",
                backend_path.join("app/build/outputs/apk/debug/app-debug.apk"),
            )
        } else if options.is_distribution() && options.is_debug() {
            (
                "bundleDebug",
                backend_path.join("app/build/outputs/bundle/debug/app-debug.aab"),
            )
        } else {
            unreachable!()
        };

        // Join ABIs with comma for the environment variable
        let abis_str = abis
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(",");

        // Set JAVA_HOME to Android Studio's bundled JDK to avoid JDK version conflicts
        // (e.g., Homebrew's JDK 25 is not supported by Android Gradle Plugin)
        let mut cmd = gradle_cmd(&gradlew, &backend_path, command_name);
        cmd.env("WATERUI_SKIP_RUST_BUILD", "1")
            .env("WATERUI_ANDROID_ABIS", &abis_str);

        if let Some(java_home) = Java::detect_home().await {
            cmd.env("JAVA_HOME", java_home);
        }

        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!("Gradle build failed:\n{}\n{}", stdout.trim(), stderr.trim());
        }

        Ok(Artifact::new(project.bundle_identifier(), path))
    }

    /// List available Android Virtual Devices (emulators).
    ///
    /// # Errors
    /// Returns an error if the emulator tool is not found.
    pub async fn list_avds() -> eyre::Result<Vec<String>> {
        let emulator_path =
            AndroidSdk::emulator_path().ok_or_else(|| eyre::eyre!("Android emulator not found"))?;

        let output = smol::process::Command::new(&emulator_path)
            .arg("-list-avds")
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let avds: Vec<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect();

        Ok(avds)
    }
}

// ============================================================================
// Clean
// ============================================================================

/// Clean Gradle build artifacts for Android.
pub async fn clean_android(project: &Project) -> eyre::Result<()> {
    let backend_path = project.backend_path::<AndroidBackend>();
    let gradlew = backend_path.join(if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    });

    if !gradlew.exists() {
        // No Android project to clean
        return Ok(());
    }

    // Set JAVA_HOME to Android Studio's bundled JDK to avoid JDK version conflicts
    let mut cmd = gradle_cmd(&gradlew, &backend_path, "clean");

    if let Some(java_home) = Java::detect_home().await {
        cmd.env("JAVA_HOME", java_home);
    }

    let output = cmd.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Gradle clean failed: {}", stderr.trim());
    }

    Ok(())
}

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the Android backend.
pub const fn is_android_platform(platform: TargetPlatform) -> bool {
    matches!(platform, TargetPlatform::Android)
}

// ============================================================================
// Asset and Font Handling
// ============================================================================

/// Copy project assets and dependency fonts to the Android assets directory.
async fn copy_assets_and_fonts(project: &Project, backend_path: &Path) -> eyre::Result<()> {
    let assets_dir = backend_path.join("app/src/main/assets");

    // Stage project assets using platform-native conventions.
    assets::stage_project_assets_for_android(project, backend_path).await?;

    // Scan and resolve dependency fonts
    let font_declarations = assets::scan_fonts(project).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(project)?);

    if !resolved_fonts.is_empty() {
        // Copy fonts to assets/fonts/
        let fonts_dest = assets_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;

        info!("Copied {} fonts to Android app", resolved_fonts.len());
    }

    // Always generate WaterUIFonts.kt (even if empty) since MainActivity references it
    let java_dir = backend_path.join("app/src/main/java");
    generate_font_registration_kotlin(project, &resolved_fonts, &java_dir).await?;

    Ok(())
}

/// Template for WaterUIFonts.kt (in android_dynamic/ to avoid scaffold auto-copy)
const WATERUI_FONTS_TEMPLATE: &str =
    include_str!("../templates/android_dynamic/WaterUIFonts.kt.tpl");

/// Generate WaterUIFonts.kt file for registering custom fonts.
async fn generate_font_registration_kotlin(
    project: &Project,
    fonts: &[ResolvedFont],
    java_dir: &Path,
) -> eyre::Result<()> {
    // Get the package namespace from the project
    let namespace = project.bundle_identifier().replace('-', "_");

    // Clean up legacy layout: older CLI versions wrote `WaterUIFonts.kt` directly under
    // `app/src/main/java/` (but still declared the app package), which can cause
    // Kotlin redeclaration errors after we started generating into the package dir.
    let legacy_path = java_dir.join("WaterUIFonts.kt");
    let _ = fs::remove_file(&legacy_path).await;

    // Build font entries
    let mut font_entries = String::new();
    for font in fonts {
        let file_name = font
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        writeln!(
            &mut font_entries,
            "        fonts[\"{}\"] = Typeface.createFromAsset(context.assets, \"fonts/{}\")",
            font.name, file_name
        )
        .unwrap();
    }

    // Render template
    let content = WATERUI_FONTS_TEMPLATE
        .replace("__ANDROID_NAMESPACE__", &namespace)
        .replace("__FONT_ENTRIES__", font_entries.trim_end());

    // Create the package directory structure
    let package_dir = java_dir.join(namespace.replace('.', "/"));
    fs::create_dir_all(&package_dir).await?;

    let kotlin_path = package_dir.join("WaterUIFonts.kt");
    fs::write(&kotlin_path, content).await?;

    debug!("Generated {}", kotlin_path.display());

    Ok(())
}
