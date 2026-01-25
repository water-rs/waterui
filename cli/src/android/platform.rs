//! Android platform build and package utilities.
//!
//! This module provides utility functions for building and packaging Android apps.
//! These functions are used by `AndroidBackend` to implement the `Backend` trait.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, bail};
use smol::fs;
use target_lexicon::{Aarch64Architecture, Architecture, Triple};

use tracing::{debug, info};

use std::fmt::Write;

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
fn ndk_host_tag() -> &'static str {
    use target_lexicon::{Architecture, OperatingSystem, Triple};

    let host = Triple::host();

    // TODO: Better ARM support
    match (&host.operating_system, &host.architecture) {
        (OperatingSystem::Darwin(_), Architecture::Aarch64(_) | _) => "darwin-x86_64", // NDK uses x86_64 even on ARM Macs (Rosetta)
        (OperatingSystem::Windows, _) => "windows-x86_64",
        // NDK doesn't have native ARM64 Linux builds
        (OperatingSystem::Linux, _) => "linux-x86_64",
        _ => unimplemented!(),
    }
}

/// Get the NDK clang linker path for the given ABI.
fn ndk_linker_path(ndk_path: &Path, abi: &str) -> PathBuf {
    let target = match abi {
        "arm64-v8a" => "aarch64-linux-android",
        "x86_64" => "x86_64-linux-android",
        "armeabi-v7a" => "armv7a-linux-androideabi",
        "x86" => "i686-linux-android",
        _ => unimplemented!(),
    };

    // Use API level 24 as minimum (Android 7.0)
    let api_level = 24;

    ndk_path
        .join("toolchains/llvm/prebuilt")
        .join(ndk_host_tag())
        .join("bin")
        .join(format!("{target}{api_level}-clang"))
}

/// Get the NDK ar path.
fn ndk_ar_path(ndk_path: &Path) -> PathBuf {
    ndk_path
        .join("toolchains/llvm/prebuilt")
        .join(ndk_host_tag())
        .join("bin/llvm-ar")
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
fn create_android_toolchain_wrapper(ndk_path: &Path, abi: &str) -> eyre::Result<PathBuf> {
    use std::io::Write;

    // Create wrapper in a temp directory that persists for the build
    let wrapper_dir = std::env::temp_dir().join("waterui-cmake-toolchains");
    std::fs::create_dir_all(&wrapper_dir)?;

    let wrapper_path = wrapper_dir.join(format!("android-{abi}.cmake"));
    let ndk_toolchain = ndk_path.join("build/cmake/android.toolchain.cmake");

    let content = format!(
        r#"# Auto-generated wrapper toolchain for WaterUI Android builds
# Sets ANDROID_ABI before including the NDK toolchain to fix cmake-rs cross-compilation
set(ANDROID_ABI "{abi}")
set(ANDROID_PLATFORM "android-24")
include("{ndk_toolchain}")
"#,
        abi = abi,
        ndk_toolchain = ndk_toolchain.display()
    );

    let mut file = std::fs::File::create(&wrapper_path)?;
    file.write_all(content.as_bytes())?;

    Ok(wrapper_path)
}

/// Get the NDK clang++ (C++ compiler) path for the given ABI.
fn ndk_cxx_path(ndk_path: &Path, abi: &str) -> PathBuf {
    let target = match abi {
        "arm64-v8a" => "aarch64-linux-android",
        "x86_64" => "x86_64-linux-android",
        "armeabi-v7a" => "armv7a-linux-androideabi",
        "x86" => "i686-linux-android",
        _ => unimplemented!(),
    };

    // Use API level 24 as minimum (Android 7.0)
    let api_level = 24;

    ndk_path
        .join("toolchains/llvm/prebuilt")
        .join(ndk_host_tag())
        .join("bin")
        .join(format!("{target}{api_level}-clang++"))
}

/// Represents an Android platform for a specific architecture.
#[derive(Debug, Clone)]
pub struct AndroidPlatform {
    architecture: Architecture,
}

impl AndroidPlatform {
    /// Create a new Android platform with the specified architecture.
    #[must_use]
    pub const fn new(architecture: Architecture) -> Self {
        Self { architecture }
    }

    /// Create an Android platform for arm64-v8a (most common modern Android devices).
    #[must_use]
    pub const fn arm64() -> Self {
        Self {
            architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
        }
    }

    /// Create an Android platform for `x86_64` (emulators on Intel/AMD).
    #[must_use]
    pub const fn x86_64() -> Self {
        Self {
            architecture: Architecture::X86_64,
        }
    }

    /// Get the Android ABI name for this architecture.
    #[must_use]
    pub const fn abi(&self) -> &'static str {
        match self.architecture {
            Architecture::Aarch64(_) => "arm64-v8a",
            Architecture::X86_64 => "x86_64",
            Architecture::Arm(_) => "armeabi-v7a",
            Architecture::X86_32(_) => "x86",
            _ => unimplemented!(),
        }
    }

    /// Get the architecture from an Android ABI name.
    #[must_use]
    pub fn from_abi(abi: &str) -> Self {
        let architecture = match abi {
            "arm64-v8a" => Architecture::Aarch64(Aarch64Architecture::Aarch64),
            "x86_64" => Architecture::X86_64,
            "armeabi-v7a" => Architecture::Arm(target_lexicon::ArmArchitecture::Armv7),
            "x86" => Architecture::X86_32(target_lexicon::X86_32Architecture::I686),
            _ => unimplemented!(),
        };
        Self { architecture }
    }
}

/// All supported Android ABIs.
pub const ALL_ABIS: &[&str] = &["arm64-v8a", "x86_64", "armeabi-v7a", "x86"];

impl AndroidPlatform {
    /// Returns all supported Android platforms (all architectures).
    #[must_use]
    pub fn all() -> Vec<Self> {
        ALL_ABIS.iter().map(|abi| Self::from_abi(abi)).collect()
    }

    /// Get the target triple for this Android platform.
    #[must_use]
    pub fn triple(&self) -> Triple {
        Triple {
            architecture: self.architecture.clone(),
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

        // Set environment variables for the linker
        let target_upper = triple.to_string().replace('-', "_").to_uppercase();

        // Build with RustBuild
        let build = RustBuild::new(project.root(), triple.clone(), options.is_hot_reload());

        // Detect Kotlin path before entering unsafe block (detect_path is async)
        let kotlin_bin_dir = Kotlin::detect_path().await.and_then(|p| p.parent().map(PathBuf::from));

        // Set environment variables for cargo, cc-rs, and cmake before building
        // SAFETY: CLI is single-threaded at this point
        unsafe {
            // For cargo/rustc linker
            std::env::set_var(format!("CARGO_TARGET_{target_upper}_LINKER"), &linker);
            std::env::set_var(format!("CARGO_TARGET_{target_upper}_AR"), &ar);

            // For cc-rs crate (used by ring, aws-lc-sys, etc.) - uses underscore format
            let target_underscore = triple.to_string().replace('-', "_");
            std::env::set_var(format!("CC_{target_underscore}"), &linker);
            std::env::set_var(format!("CXX_{target_underscore}"), &cxx);
            std::env::set_var(format!("AR_{target_underscore}"), &ar);

            // For CMake-based builds (aws-lc-sys, etc.)
            std::env::set_var("ANDROID_NDK", &ndk_path);
            std::env::set_var("ANDROID_NDK_HOME", &ndk_path);
            std::env::set_var("ANDROID_NDK_ROOT", &ndk_path);

            // Set Android SDK environment variables (needed by waterkit and other crates)
            if let Some(sdk_path) = AndroidSdk::detect_path() {
                std::env::set_var("ANDROID_HOME", &sdk_path);
                std::env::set_var("ANDROID_SDK_ROOT", &sdk_path);

                // Set ANDROID_JAR path from highest installed API level
                if let Some(android_jar) = find_android_jar(&sdk_path) {
                    std::env::set_var("ANDROID_JAR", &android_jar);
                }
            }

            // Add kotlinc to PATH (needed by waterkit and other crates that compile Kotlin)
            if let Some(kotlin_bin) = &kotlin_bin_dir {
                let current_path = std::env::var("PATH").unwrap_or_default();
                let new_path = format!("{}:{}", kotlin_bin.display(), current_path);
                std::env::set_var("PATH", new_path);
            }

            // Create a wrapper CMake toolchain file
            let wrapper_toolchain = create_android_toolchain_wrapper(&ndk_path, abi)?;
            std::env::set_var("CMAKE_TOOLCHAIN_FILE", &wrapper_toolchain);
            std::env::set_var(
                format!("CMAKE_TOOLCHAIN_FILE_{target_underscore}"),
                &wrapper_toolchain,
            );

            std::env::set_var("ANDROID_ABI", abi);
            std::env::set_var("ANDROID_PLATFORM", "android-24");

            if which::which("ninja").is_ok() {
                std::env::set_var("CMAKE_GENERATOR", "Ninja");
            }
        }

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
                    .join(abi)
            },
            std::path::Path::to_path_buf,
        );
        fs::create_dir_all(&output_dir).await?;

        // Copy with standardized name
        let dest_lib = output_dir.join("libwaterui_app.so");
        copy_file(&source_lib, &dest_lib).await?;

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
    ///
    /// # Panics
    ///
    /// Panics if an unsupported ABI is provided.
    pub async fn package_with_abis(
        project: &Project,
        options: PackageOptions,
        abis: &[&str],
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
        let abis_str = abis.join(",");

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
// Build Utilities
// ============================================================================

/// Build Rust library for Android platform.
pub async fn build_android(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<PathBuf> {
    let abi = platform_to_abi(platform);
    let triple = android_triple(platform);

    // Get NDK path for configuring the linker
    let ndk_path = AndroidNdk::detect_path().ok_or_else(|| {
        eyre::eyre!("Android NDK not found. Please install it via Android Studio.")
    })?;

    // Configure NDK environment for cargo
    let linker = ndk_linker_path(&ndk_path, abi);
    let ar = ndk_ar_path(&ndk_path);
    let cxx = ndk_cxx_path(&ndk_path, abi);

    // Set environment variables for the linker
    let target_upper = triple.to_string().replace('-', "_").to_uppercase();

    // Build with RustBuild
    let build = RustBuild::new(project.root(), triple.clone(), options.is_hot_reload());

    // Detect Kotlin path before entering unsafe block (detect_path is async)
    let kotlin_bin_dir = Kotlin::detect_path().await.and_then(|p| p.parent().map(PathBuf::from));

    // Set environment variables for cargo, cc-rs, and cmake before building
    // SAFETY: CLI is single-threaded at this point
    unsafe {
        // For cargo/rustc linker
        std::env::set_var(format!("CARGO_TARGET_{target_upper}_LINKER"), &linker);
        std::env::set_var(format!("CARGO_TARGET_{target_upper}_AR"), &ar);

        // For cc-rs crate (used by ring, aws-lc-sys, etc.) - uses underscore format
        let target_underscore = triple.to_string().replace('-', "_");
        std::env::set_var(format!("CC_{target_underscore}"), &linker);
        std::env::set_var(format!("CXX_{target_underscore}"), &cxx);
        std::env::set_var(format!("AR_{target_underscore}"), &ar);

        // For CMake-based builds (aws-lc-sys, etc.)
        // Set all variants as different crates check different env vars
        std::env::set_var("ANDROID_NDK", &ndk_path);
        std::env::set_var("ANDROID_NDK_HOME", &ndk_path);
        std::env::set_var("ANDROID_NDK_ROOT", &ndk_path);

        // Set Android SDK environment variables (needed by waterkit and other crates)
        if let Some(sdk_path) = AndroidSdk::detect_path() {
            std::env::set_var("ANDROID_HOME", &sdk_path);
            std::env::set_var("ANDROID_SDK_ROOT", &sdk_path);

            // Set ANDROID_JAR path from highest installed API level
            if let Some(android_jar) = find_android_jar(&sdk_path) {
                std::env::set_var("ANDROID_JAR", &android_jar);
            }
        }

        // Add kotlinc to PATH (needed by waterkit and other crates that compile Kotlin)
        if let Some(kotlin_bin) = &kotlin_bin_dir {
            let current_path = std::env::var("PATH").unwrap_or_default();
            let new_path = format!("{}:{}", kotlin_bin.display(), current_path);
            std::env::set_var("PATH", new_path);
        }

        // Create a wrapper CMake toolchain file that sets ANDROID_ABI before
        // including the NDK toolchain. This is required because cmake-rs doesn't
        // pass ANDROID_ABI as a -D define, causing the NDK toolchain to default
        // to armeabi-v7a (32-bit ARM) instead of the correct architecture.
        let wrapper_toolchain = create_android_toolchain_wrapper(&ndk_path, abi)?;

        std::env::set_var("CMAKE_TOOLCHAIN_FILE", &wrapper_toolchain);
        std::env::set_var(
            format!("CMAKE_TOOLCHAIN_FILE_{target_underscore}"),
            &wrapper_toolchain,
        );

        // Also set these for other tools that might check them
        std::env::set_var("ANDROID_ABI", abi);
        std::env::set_var("ANDROID_PLATFORM", "android-24");

        // Use Ninja generator if available to avoid Xcode/Make conflicts on macOS
        // The system Make on macOS can inject -arch and -isysroot flags that break Android builds
        if which::which("ninja").is_ok() {
            std::env::set_var("CMAKE_GENERATOR", "Ninja");
        }
    }

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
                .join(abi)
        },
        std::path::Path::to_path_buf,
    );
    fs::create_dir_all(&output_dir).await?;

    // Copy with standardized name
    let dest_lib = output_dir.join("libwaterui_app.so");
    copy_file(&source_lib, &dest_lib).await?;

    Ok(lib_dir)
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
// Package
// ============================================================================

/// Package an Android app using Gradle.
pub async fn package_android(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> eyre::Result<Artifact> {
    validate_android_package_name(project.bundle_identifier())?;

    let abi = platform_to_abi(platform);
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

    // Skip Rust build in Gradle - we already built the library via `water build`
    // The Gradle build.gradle.kts checks this env var and skips its buildRust tasks
    //
    // Also pass the target ABI to filter which native libraries are included
    // This ensures only the architectures we built are packaged in the APK
    //
    // Set JAVA_HOME to Android Studio's bundled JDK to avoid JDK version conflicts
    // (e.g., Homebrew's JDK 25 is not supported by Android Gradle Plugin)
    let mut cmd = gradle_cmd(&gradlew, &backend_path, command_name);
    cmd.env("WATERUI_SKIP_RUST_BUILD", "1")
        .env("WATERUI_ANDROID_ABIS", abi);

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

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the Android backend.
pub const fn is_android_platform(platform: TargetPlatform) -> bool {
    matches!(platform, TargetPlatform::Android)
}

/// Get the Android ABI for a platform.
fn platform_to_abi(platform: TargetPlatform) -> &'static str {
    match platform {
        TargetPlatform::Android => "arm64-v8a", // Default to arm64
        _ => unreachable!("Not an Android platform"),
    }
}

/// Get the target triple for Android.
fn android_triple(platform: TargetPlatform) -> Triple {
    match platform {
        TargetPlatform::Android => Triple {
            architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
            vendor: target_lexicon::Vendor::Unknown,
            operating_system: target_lexicon::OperatingSystem::Linux,
            environment: target_lexicon::Environment::Android,
            binary_format: target_lexicon::BinaryFormat::Elf,
        },
        _ => unreachable!("Not an Android platform"),
    }
}

// ============================================================================
// Asset and Font Handling
// ============================================================================

/// Copy project assets and dependency fonts to the Android assets directory.
async fn copy_assets_and_fonts(project: &Project, backend_path: &Path) -> eyre::Result<()> {
    let assets_dir = backend_path.join("app/src/main/assets");

    // Copy project assets
    let project_assets_dest = assets_dir.clone();
    assets::copy_project_assets(project, &project_assets_dest).await?;

    // Scan and resolve dependency fonts
    let font_declarations = assets::scan_fonts(project).await?;
    let resolved_fonts = assets::resolve_fonts(font_declarations).await?;

    if !resolved_fonts.is_empty() {
        // Copy fonts to assets/fonts/
        let fonts_dest = assets_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;

        // Generate WaterUIFonts.kt for font registration
        let java_dir = backend_path.join("app/src/main/java");
        generate_font_registration_kotlin(project, &resolved_fonts, &java_dir).await?;

        info!("Copied {} fonts to Android app", resolved_fonts.len());
    }

    Ok(())
}

/// Template for WaterUIFonts.kt
const WATERUI_FONTS_TEMPLATE: &str =
    include_str!("../templates/android/app/src/main/java/WaterUIFonts.kt.tpl");

/// Generate WaterUIFonts.kt file for registering custom fonts.
async fn generate_font_registration_kotlin(
    project: &Project,
    fonts: &[ResolvedFont],
    java_dir: &Path,
) -> eyre::Result<()> {
    // Get the package namespace from the project
    let namespace = project.bundle_identifier().replace('-', "_");

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
