use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, bail};
use smol::fs;
use target_lexicon::{Aarch64Architecture, Architecture, Triple};

use crate::{
    android::{
        device::AndroidDevice,
        toolchain::{AndroidNdk, AndroidToolchain},
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

    // Use API level 21 as minimum (Android 5.0)
    let api_level = 21;

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

/// Get the path to libc++_shared.so in the NDK.
fn ndk_libcxx_path(ndk_path: &Path, abi: &str) -> PathBuf {
    ndk_path
        .join("sources/cxx-stl/llvm-libc++/libs")
        .join(abi)
        .join("libc++_shared.so")
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

impl Platform for AndroidPlatform {
    type Device = AndroidDevice;
    type Toolchain = AndroidToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        // Use adb to list connected devices
        let output = run_command("adb", ["devices"]).await?;

        let mut devices = Vec::new();

        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "device" {
                let identifier = parts[0].to_string();

                // Query the device's primary ABI
                let abi = run_command(
                    "adb",
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
        let backend = project
            .android_backend()
            .ok_or_else(|| eyre::eyre!("Android backend must be configured"))?;

        let gradlew = backend.gradlew_path();
        let project_path = backend.project_path();

        if !gradlew.exists() {
            // No Android project to clean
            return Ok(());
        }

        run_command(
            gradlew.to_str().unwrap(),
            ["clean", "--project-dir", project_path.to_str().unwrap()],
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

        // Set environment variables for the linker
        let target_upper = self.triple().to_string().replace('-', "_").to_uppercase();

        // Build with RustBuild
        let build = RustBuild::new(project.root(), self.triple());

        // Set linker environment variables before building
        // SAFETY: CLI is single-threaded at this point
        unsafe {
            std::env::set_var(format!("CARGO_TARGET_{target_upper}_LINKER"), &linker);
            std::env::set_var(format!("CARGO_TARGET_{target_upper}_AR"), &ar);
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

        // Get the Android backend configuration
        let backend = project
            .android_backend()
            .ok_or_else(|| eyre::eyre!("Android backend must be configured"))?;

        // Copy to jniLibs directory
        let jni_libs_dir = project
            .root()
            .join(backend.project_path())
            .join("app/src/main/jniLibs")
            .join(self.abi());
        fs::create_dir_all(&jni_libs_dir).await?;

        // Copy with standardized name
        let dest_lib = jni_libs_dir.join("libwaterui_app.so");
        copy_file(&source_lib, &dest_lib).await?;

        // Also copy libc++_shared.so from NDK if it exists
        let libcxx_path = ndk_libcxx_path(&ndk_path, self.abi());
        if libcxx_path.exists() {
            let dest_libcxx = jni_libs_dir.join("libc++_shared.so");
            copy_file(&libcxx_path, &dest_libcxx).await?;
        }

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
        let backend = project
            .android_backend()
            .expect("Android backend must be configured");

        let project_path = backend.project_path();
        let gradlew = backend.gradlew_path();

        let (command_name, path) = if options.is_distribution() && !options.is_debug() {
            (
                "bundleRelease",
                project_path.join("app/build/outputs/bundle/release/app-release.aab"),
            )
        } else if !options.is_distribution() && !options.is_debug() {
            (
                "assembleRelease",
                project_path.join("app/build/outputs/apk/release/app-release.apk"),
            )
        } else if !options.is_distribution() && options.is_debug() {
            (
                "assembleDebug",
                project_path.join("app/build/outputs/apk/debug/app-debug.apk"),
            )
        } else if options.is_distribution() && options.is_debug() {
            (
                "bundleDebug",
                project_path.join("app/build/outputs/bundle/debug/app-debug.aab"),
            )
        } else {
            unreachable!()
        };

        run_command(
            gradlew.to_str().unwrap(),
            [
                command_name,
                "--project-dir",
                backend.project_path().to_str().unwrap(),
            ],
        )
        .await?;

        Ok(Artifact::new(project.bundle_identifier(), path))
    }
}
