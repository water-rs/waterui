//! Android platform build and package utilities.
//!
//! This module provides utility functions for building and packaging Android apps.
//! These functions are used by `AndroidBackend` to implement the `Backend` trait.

use std::{
    env,
    path::{Path, PathBuf},
};

use askama::Template;
use color_eyre::eyre::{self, bail};
use smol::{fs, unblock};
use target_lexicon::{Aarch64Architecture, Architecture, Triple};

use tracing::{debug, info};

use std::str::FromStr;

use crate::{
    android::{
        ANDROID_MIN_API_LEVEL,
        backend::AndroidBackend,
        toolchain::{AndroidNdk, AndroidSdk, Java, Kotlin, java_proxy_properties_from_env},
    },
    assets::{self, ResolvedFont},
    build::{
        BuildOptions, RustBuild, RustDynamicLibraries, RustLinkage, shared_rust_runtime_fingerprint,
    },
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    templates::FontRegistrationTemplateEntry,
    toolchain::{ToolchainError, windows_arm64_llvm::WindowsArm64LlvmToolchain},
    utils::copy_file,
};

fn gradle_cmd(gradlew: &Path, backend_path: &Path, task: &str) -> smol::process::Command {
    let mut cmd = smol::process::Command::new(gradlew);
    cmd.arg(task).arg("--project-dir").arg(backend_path);
    cmd
}

fn apply_gradle_proxy_env(cmd: &mut smol::process::Command) -> eyre::Result<()> {
    let proxy_properties = java_proxy_properties_from_env()?;
    if proxy_properties.is_empty() {
        return Ok(());
    }

    cmd.args(&proxy_properties);

    let mut gradle_opts = proxy_properties.join(" ");
    if let Ok(existing) = env::var("GRADLE_OPTS")
        && !existing.trim().is_empty()
    {
        gradle_opts.push(' ');
        gradle_opts.push_str(&existing);
    }
    cmd.env("GRADLE_OPTS", gradle_opts);
    Ok(())
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
/// falling back to `darwin-x86_64` for older Android NDK releases (Rosetta).
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
/// NDK r23+ ships it under `sysroot/usr/lib/<triple>/`, while older Android
/// NDK releases used `sources/cxx-stl/llvm-libc++/libs/<abi>/`.
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

struct AndroidBuildContext {
    abi: AndroidAbi,
    ndk_path: PathBuf,
    linker: PathBuf,
    ar: PathBuf,
    cxx: PathBuf,
    target_underscore: String,
    target_upper: String,
    llvm_envs: Vec<(String, std::ffi::OsString)>,
    java_home: PathBuf,
    java_bin_dir: PathBuf,
    kotlin_compiler: PathBuf,
    kotlin_bin_dir: PathBuf,
    kotlin_home: PathBuf,
    sdk_path: PathBuf,
    android_jar: PathBuf,
    wrapper_toolchain: PathBuf,
    android_platform: String,
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
    pub const fn triple(&self) -> Triple {
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
        let build_context = resolve_android_build_context(abi, &triple).await?;
        let mut build = configure_android_rust_build(project, &triple, &build_context, &options)?;
        if options.linkage() == RustLinkage::SharedRuntime {
            let build_features = vec!["waterui-ffi/android-jni".to_string(), "dev".to_string()];
            let fingerprint = shared_rust_runtime_fingerprint(
                &project.ffi_crate_path().join("Cargo.toml"),
                &build_features,
                &triple,
            )
            .await?;
            build = build.with_target_dir(
                project
                    .shared_backend_target_dir("android", &fingerprint)
                    .await?,
            );
        }

        let lib_dir = build.build_lib(options.is_release()).await?;
        copy_android_build_outputs(project, &options, abi, &build_context.ndk_path, &lib_dir)
            .await?;
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
        if let Some(sdk_path) = AndroidSdk::detect_path() {
            cmd.env("ANDROID_HOME", &sdk_path)
                .env("ANDROID_SDK_ROOT", &sdk_path);
        }
        apply_gradle_proxy_env(&mut cmd)?;

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

async fn resolve_android_build_context(
    abi: AndroidAbi,
    triple: &Triple,
) -> eyre::Result<AndroidBuildContext> {
    let ndk_path = AndroidNdk::detect_path().ok_or_else(|| {
        eyre::eyre!("Android NDK not found. Please install it via Android Studio.")
    })?;
    let linker = ndk_linker_path(&ndk_path, abi);
    let ar = ndk_ar_path(&ndk_path);
    let cxx = ndk_cxx_path(&ndk_path, abi);
    let target_underscore = triple.to_string().replace('-', "_");
    let target_upper = target_underscore.to_uppercase();
    let llvm_envs = resolve_windows_arm64_llvm_envs().await?;
    let (java_home, java_bin_dir) = resolve_java_home().await?;
    let (kotlin_compiler, kotlin_bin_dir, kotlin_home) = resolve_kotlin_home().await?;
    let (sdk_path, android_jar) = resolve_android_sdk_paths().await?;
    let wrapper_toolchain = create_android_toolchain_wrapper(&ndk_path, abi).await?;

    Ok(AndroidBuildContext {
        abi,
        ndk_path,
        linker,
        ar,
        cxx,
        target_underscore,
        target_upper,
        llvm_envs,
        java_home,
        java_bin_dir,
        kotlin_compiler,
        kotlin_bin_dir,
        kotlin_home,
        sdk_path,
        android_jar,
        wrapper_toolchain,
        android_platform: format!("android-{ANDROID_MIN_API_LEVEL}"),
    })
}

async fn resolve_windows_arm64_llvm_envs() -> eyre::Result<Vec<(String, std::ffi::OsString)>> {
    WindowsArm64LlvmToolchain
        .cargo_envs()
        .await
        .map_err(|error| match error {
            ToolchainError::Fixable(_) => eyre::eyre!(
                "Windows ARM64 LLVM toolchain is missing. Run `water doctor --fix` to install it automatically."
            ),
            ToolchainError::Unfixable(unfixable) => {
                eyre::eyre!("Windows ARM64 LLVM toolchain check failed: {unfixable}")
            }
        })
}

async fn resolve_java_home() -> eyre::Result<(PathBuf, PathBuf)> {
    let java_home = Java::detect_home().await.ok_or_else(|| {
        eyre::eyre!(
            "Java runtime not found. Install a JDK (or Android Studio JBR), then re-run `water doctor --fix`."
        )
    })?;
    let java_bin_dir = java_home.join("bin");
    Ok((java_home, java_bin_dir))
}

async fn resolve_kotlin_home() -> eyre::Result<(PathBuf, PathBuf, PathBuf)> {
    let kotlin_compiler = Kotlin::detect_path().await.ok_or_else(|| {
        eyre::eyre!(
            "Kotlin compiler (kotlinc) not found. Install Android Studio or set `KOTLIN_HOME`, then re-run `water doctor`."
        )
    })?;
    let kotlin_bin_dir = kotlin_compiler.parent().map(PathBuf::from).ok_or_else(|| {
        eyre::eyre!(
            "Failed to determine Kotlin bin directory from `{}`.",
            kotlin_compiler.display()
        )
    })?;
    let kotlin_home = kotlin_bin_dir.parent().map(PathBuf::from).ok_or_else(|| {
        eyre::eyre!(
            "Failed to determine KOTLIN_HOME from `{}`.",
            kotlin_bin_dir.display()
        )
    })?;
    Ok((kotlin_compiler, kotlin_bin_dir, kotlin_home))
}

async fn resolve_android_sdk_paths() -> eyre::Result<(PathBuf, PathBuf)> {
    let sdk_path = AndroidSdk::detect_path().ok_or_else(|| {
        eyre::eyre!("Android SDK not found. Please install it via Android Studio.")
    })?;
    let android_jar = unblock(AndroidSdk::android_jar_path)
        .await
        .ok_or_else(|| {
            eyre::eyre!(
                "Android platforms not found in SDK at {}. Install an Android platform (SDK) in Android Studio.",
                sdk_path.display()
            )
        })?;
    Ok((sdk_path, android_jar))
}

fn configure_android_rust_build(
    project: &Project,
    triple: &Triple,
    context: &AndroidBuildContext,
    options: &BuildOptions,
) -> eyre::Result<RustBuild> {
    let mut build = RustBuild::new(project.ffi_crate_path(), triple.clone())
        .with_feature("waterui-ffi/android-jni");
    if options.linkage() == RustLinkage::SharedRuntime {
        build = build
            .with_feature("dev")
            .with_rustc_flag("-Cdebuginfo=0")
            .with_preferred_dynamic_linking();
    }
    if let Some(sccache_path) = options.sccache_path() {
        build = build.with_sccache(sccache_path.to_path_buf());
    }
    for (key, value) in &context.llvm_envs {
        build = build.with_env(key.clone(), value.clone());
    }

    build = build
        .with_env(
            format!("CARGO_TARGET_{}_LINKER", context.target_upper),
            context.linker.as_os_str(),
        )
        .with_env(
            format!("CARGO_TARGET_{}_AR", context.target_upper),
            context.ar.as_os_str(),
        )
        .with_env(
            format!("CC_{}", context.target_underscore),
            context.linker.as_os_str(),
        )
        .with_env(
            format!("CXX_{}", context.target_underscore),
            context.cxx.as_os_str(),
        )
        .with_env(
            format!("AR_{}", context.target_underscore),
            context.ar.as_os_str(),
        )
        .with_env("ANDROID_NDK", context.ndk_path.as_os_str())
        .with_env("ANDROID_NDK_HOME", context.ndk_path.as_os_str())
        .with_env("ANDROID_NDK_ROOT", context.ndk_path.as_os_str())
        .with_env("ANDROID_HOME", context.sdk_path.as_os_str())
        .with_env("ANDROID_SDK_ROOT", context.sdk_path.as_os_str())
        .with_env("ANDROID_JAR", context.android_jar.as_os_str())
        .with_env("JAVA_HOME", context.java_home.as_os_str())
        .with_env("KOTLIN_HOME", context.kotlin_home.as_os_str())
        .with_env("KOTLINC", context.kotlin_compiler.as_os_str())
        .with_env(
            "CMAKE_TOOLCHAIN_FILE",
            context.wrapper_toolchain.as_os_str(),
        )
        .with_env(
            format!("CMAKE_TOOLCHAIN_FILE_{}", context.target_underscore),
            context.wrapper_toolchain.as_os_str(),
        )
        .with_env("CMAKE_ASM_COMPILER", context.linker.as_os_str())
        .with_env("ANDROID_ABI", context.abi.as_str())
        .with_env("ANDROID_PLATFORM", &context.android_platform)
        .with_env("PKG_CONFIG_ALLOW_CROSS", "1")
        .with_env(
            format!("PKG_CONFIG_ALLOW_CROSS_{}", context.target_underscore),
            "1",
        )
        .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{triple}"), "1");

    let current_path = std::env::var_os("PATH")
        .ok_or_else(|| eyre::eyre!("PATH environment variable is not set"))?;
    let mut paths: Vec<PathBuf> = std::env::split_paths(&current_path).collect();
    paths.insert(0, context.java_bin_dir.clone());
    paths.insert(0, context.kotlin_bin_dir.clone());
    let new_path = std::env::join_paths(paths).map_err(|error| {
        eyre::eyre!("Failed to construct PATH for Java/Kotlin compiler resolution: {error}")
    })?;

    Ok(build.with_env("PATH", new_path))
}

async fn copy_android_build_outputs(
    project: &Project,
    options: &BuildOptions,
    abi: AndroidAbi,
    ndk_path: &Path,
    lib_dir: &Path,
) -> eyre::Result<()> {
    let lib_name = project.ffi_crate_name().replace('-', "_");
    let source_lib = lib_dir.join(format!("lib{lib_name}.so"));

    if !source_lib.exists() {
        bail!(
            "Rust shared library not found at {}. Did the build succeed?",
            source_lib.display()
        );
    }

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
    copy_file(&source_lib, &output_dir.join("libwaterui_app.so")).await?;

    let libcxx_path = ndk_libcxx_path(ndk_path, abi);
    if libcxx_path.exists() {
        copy_file(&libcxx_path, &output_dir.join("libc++_shared.so")).await?;
    }

    if options.linkage() == RustLinkage::SharedRuntime {
        let triple = AndroidPlatform::new(abi).triple();
        let libraries = RustDynamicLibraries::resolve(lib_dir, &triple).await?;
        libraries.stage(&output_dir).await?;
    } else {
        RustDynamicLibraries::remove_staged(&output_dir, &AndroidPlatform::new(abi).triple())
            .await?;
    }

    Ok(())
}

// ============================================================================
// Clean
// ============================================================================

/// Clean Gradle build artifacts for Android.
///
/// # Errors
/// Returns an error if the Gradle clean command fails.
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
    if let Some(sdk_path) = AndroidSdk::detect_path() {
        cmd.env("ANDROID_HOME", &sdk_path)
            .env("ANDROID_SDK_ROOT", &sdk_path);
    }
    apply_gradle_proxy_env(&mut cmd)?;

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
#[must_use]
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

#[derive(Template)]
#[template(
    path = "src/templates/android_dynamic/WaterUIFonts.kt.tpl",
    escape = "none"
)]
struct WaterUiFontsKotlinTemplate<'a> {
    namespace: &'a str,
    font_entries: &'a [FontRegistrationTemplateEntry],
}

/// Generate WaterUIFonts.kt file for registering custom fonts.
async fn generate_font_registration_kotlin(
    project: &Project,
    fonts: &[ResolvedFont],
    java_dir: &Path,
) -> eyre::Result<()> {
    // Get the package namespace from the project
    let namespace = project
        .bundle_identifier()
        .android_package_name()
        .map_err(|error| eyre::eyre!("{error}"))?;

    // Clean up legacy layout: older CLI versions wrote `WaterUIFonts.kt` directly under
    // `app/src/main/java/` (but still declared the app package), which can cause
    // Kotlin redeclaration errors after we started generating into the package dir.
    let legacy_path = java_dir.join("WaterUIFonts.kt");
    let _ = fs::remove_file(&legacy_path).await;

    // Build font entries
    let font_entries = fonts
        .iter()
        .map(|font| FontRegistrationTemplateEntry {
            family_name: font.name.clone(),
            file_name: font
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect::<Vec<_>>();

    let content = WaterUiFontsKotlinTemplate {
        namespace: namespace.as_str(),
        font_entries: &font_entries,
    }
    .render()
    .map_err(|error| eyre::eyre!("Failed to render WaterUIFonts.kt template: {error}"))?;

    // Create the package directory structure
    let package_dir = java_dir.join(namespace.as_str().replace('.', "/"));
    fs::create_dir_all(&package_dir).await?;

    let kotlin_path = package_dir.join("WaterUIFonts.kt");
    fs::write(&kotlin_path, content).await?;

    debug!("Generated {}", kotlin_path.display());

    Ok(())
}
