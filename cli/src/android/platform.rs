use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, bail};
use smol::fs;
use target_lexicon::{Aarch64Architecture, Architecture, Triple};

use crate::{
    android::{
        backend::AndroidBackend,
        device::AndroidDevice,
        toolchain::{AndroidNdk, AndroidSdk, AndroidToolchain},
    },
    build::{BuildOptions, RustBuild},
    device::Artifact,
    platform::{PackageOptions, Platform},
    project::Project,
    utils::{copy_file, run_command},
};

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
        abis: &[&str],
    ) -> eyre::Result<Artifact> {
        let backend_path = project.backend_path::<AndroidBackend>();
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

        let output = smol::process::Command::new(gradlew.to_str().unwrap())
            .args([
                command_name,
                "--project-dir",
                backend_path.to_str().unwrap(),
            ])
            .env("WATERUI_SKIP_RUST_BUILD", "1")
            .env("WATERUI_ANDROID_ABIS", &abis_str)
            .output()
            .await?;

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

impl Platform for AndroidPlatform {
    type Device = AndroidDevice;
    type Toolchain = AndroidToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        let adb = AndroidSdk::adb_path()
            .ok_or_else(|| eyre::eyre!("Android SDK not found or adb not installed"))?;

        // Use adb to list connected devices
        let output = run_command(adb.to_str().unwrap(), ["devices"]).await?;

        let mut devices = Vec::new();

        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "device" {
                let identifier = parts[0].to_string();

                // Query the device's primary ABI
                let abi = run_command(
                    adb.to_str().unwrap(),
                    ["-s", &identifier, "shell", "getprop", "ro.product.cpu.abi"],
                )
                .await
                .map_or_else(|_| "arm64-v8a".to_string(), |abi| abi.trim().to_string());

                devices.push(AndroidDevice::new(identifier, abi));
            }
        }

        Ok(devices)
    }

    fn toolchain(&self) -> Self::Toolchain {
        AndroidToolchain::default()
    }

    async fn clean(&self, project: &Project) -> eyre::Result<()> {
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

        run_command(
            gradlew.to_str().unwrap(),
            ["clean", "--project-dir", backend_path.to_str().unwrap()],
        )
        .await?;

        Ok(())
    }

    async fn build(
        &self,
        project: &Project,
        options: BuildOptions,
    ) -> eyre::Result<std::path::PathBuf> {
        // Get NDK path for configuring the linker
        let ndk_path = AndroidNdk::detect_path().ok_or_else(|| {
            eyre::eyre!("Android NDK not found. Please install it via Android Studio.")
        })?;

        // Configure NDK environment for cargo
        let linker = ndk_linker_path(&ndk_path, self.abi());
        let ar = ndk_ar_path(&ndk_path);
        let cxx = ndk_cxx_path(&ndk_path, self.abi());

        // Set environment variables for the linker
        let target_upper = self.triple().to_string().replace('-', "_").to_uppercase();

        // Build with RustBuild
        let build = RustBuild::new(project.root(), self.triple());

        // Set environment variables for cargo, cc-rs, and cmake before building
        // SAFETY: CLI is single-threaded at this point
        unsafe {
            // For cargo/rustc linker
            std::env::set_var(format!("CARGO_TARGET_{target_upper}_LINKER"), &linker);
            std::env::set_var(format!("CARGO_TARGET_{target_upper}_AR"), &ar);

            // For cc-rs crate (used by ring, aws-lc-sys, etc.) - uses underscore format
            let target_underscore = self.triple().to_string().replace('-', "_");
            std::env::set_var(format!("CC_{target_underscore}"), &linker);
            std::env::set_var(format!("CXX_{target_underscore}"), &cxx);
            std::env::set_var(format!("AR_{target_underscore}"), &ar);

            // For CMake-based builds (aws-lc-sys, etc.)
            // Set both variants as different crates check different env vars
            std::env::set_var("ANDROID_NDK_HOME", &ndk_path);
            std::env::set_var("ANDROID_NDK_ROOT", &ndk_path);

            // Set CMake toolchain file for proper Android cross-compilation
            // This is required for crates like aws-lc-sys that use CMake
            let cmake_toolchain = ndk_path.join("build/cmake/android.toolchain.cmake");
            if cmake_toolchain.exists() {
                std::env::set_var("CMAKE_TOOLCHAIN_FILE", &cmake_toolchain);
                // Also set target-specific variant
                std::env::set_var(
                    format!("CMAKE_TOOLCHAIN_FILE_{target_underscore}"),
                    &cmake_toolchain,
                );
            }

            // Set Android ABI for CMake
            let android_abi = self.abi();
            std::env::set_var("ANDROID_ABI", android_abi);
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
        let output_dir = if let Some(dir) = options.output_dir() {
            dir.to_path_buf()
        } else {
            project
                .backend_path::<AndroidBackend>()
                .join("app/src/main/jniLibs")
                .join(self.abi())
        };
        fs::create_dir_all(&output_dir).await?;

        // Copy with standardized name
        let dest_lib = output_dir.join("libwaterui_app.so");
        copy_file(&source_lib, &dest_lib).await?;

        Ok(lib_dir)
    }

    fn triple(&self) -> Triple {
        Triple {
            architecture: self.architecture,
            vendor: target_lexicon::Vendor::Unknown,
            operating_system: target_lexicon::OperatingSystem::Linux,
            environment: target_lexicon::Environment::Android,
            binary_format: target_lexicon::BinaryFormat::Elf,
        }
    }

    async fn package(
        &self,
        project: &Project,
        options: PackageOptions,
    ) -> color_eyre::eyre::Result<Artifact> {
        let backend_path = project.backend_path::<AndroidBackend>();
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
        let output = smol::process::Command::new(gradlew.to_str().unwrap())
            .args([
                command_name,
                "--project-dir",
                backend_path.to_str().unwrap(),
            ])
            .env("WATERUI_SKIP_RUST_BUILD", "1")
            .env("WATERUI_ANDROID_ABIS", self.abi())
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!("Gradle build failed:\n{}\n{}", stdout.trim(), stderr.trim());
        }

        Ok(Artifact::new(project.bundle_identifier(), path))
    }
}
