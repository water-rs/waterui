use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use crate::{
    backend::Backend,
    doctor::{AnyToolchainIssue, ToolchainIssue},
    impl_display,
    project::{Android, Project},
    util,
};
use color_eyre::eyre::{Context, Result, bail, eyre};
use thiserror::Error;
use tracing::{debug, info, warn};
use which::which;

// ============================================================================
// Target Configuration
// ============================================================================

struct AndroidTargetConfig {
    triple: &'static str,
    bin_prefix: &'static str,
    min_api: u32,
    abi: &'static str,
}

const ANDROID_TARGETS: &[AndroidTargetConfig] = &[
    AndroidTargetConfig {
        triple: "aarch64-linux-android",
        bin_prefix: "aarch64-linux-android",
        min_api: 21,
        abi: "arm64-v8a",
    },
    AndroidTargetConfig {
        triple: "x86_64-linux-android",
        bin_prefix: "x86_64-linux-android",
        min_api: 21,
        abi: "x86_64",
    },
    AndroidTargetConfig {
        triple: "armv7-linux-androideabi",
        bin_prefix: "armv7a-linux-androideabi",
        min_api: 19,
        abi: "armeabi-v7a",
    },
    AndroidTargetConfig {
        triple: "i686-linux-android",
        bin_prefix: "i686-linux-android",
        min_api: 19,
        abi: "x86",
    },
];

const MAX_API: u32 = 35;

// ============================================================================
// Backend Implementation
// ============================================================================

#[derive(Clone, Copy, Debug)]
pub struct AndroidBackend;

impl_display!(AndroidBackend, "android");

#[derive(Debug, Clone, Error)]
pub enum AndroidToolchainIssue {
    #[error("Android SDK tools (adb) were not found.")]
    SdkMissing,
    #[error("Android NDK was not detected.")]
    NdkMissing,
    #[error("CMake was not found.")]
    CmakeMissing,
}

impl ToolchainIssue for AndroidToolchainIssue {
    fn suggestion(&self) -> String {
        match self {
            Self::SdkMissing => {
                "Install Android Studio or add the command line tools to ANDROID_SDK_ROOT."
                    .to_string()
            }
            Self::NdkMissing => "Install the Android NDK via the SDK manager.".to_string(),
            Self::CmakeMissing => {
                "Install CMake via Android SDK Manager (SDK Tools → CMake) or run `brew install cmake`."
                    .to_string()
            }
        }
    }
}

impl Backend for AndroidBackend {
    type ToolchainIssue = AnyToolchainIssue;

    fn init(&self, _project: &Project, _dev: bool) -> Result<()> {
        Ok(())
    }

    fn is_existing(&self, project: &Project) -> bool {
        project.root().join("android").exists()
    }

    fn clean(&self, project: &Project) -> Result<()> {
        let gradle_dir = project.root().join("android");
        if !gradle_dir.exists() {
            return Ok(());
        }

        let gradlew = if cfg!(target_os = "windows") {
            "gradlew.bat"
        } else {
            "./gradlew"
        };

        let status = Command::new(gradlew)
            .arg("clean")
            .current_dir(&gradle_dir)
            .status()
            .context("failed to execute Gradle clean")?;

        if status.success() {
            Ok(())
        } else {
            Err(eyre!("Gradle clean failed with status {status}"))
        }
    }

    fn check_requirements(&self, _project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        let mut issues = Vec::new();

        if find_android_tool("adb").is_none() {
            issues.push(AndroidToolchainIssue::SdkMissing);
        }
        if resolve_ndk_path().is_none() {
            issues.push(AndroidToolchainIssue::NdkMissing);
        }
        if resolve_cmake_path().is_none() {
            issues.push(AndroidToolchainIssue::CmakeMissing);
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues
                .into_iter()
                .map(|issue| Box::new(issue) as AnyToolchainIssue)
                .collect())
        }
    }
}

// ============================================================================
// SDK/NDK Path Resolution
// ============================================================================

fn sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push = |path: PathBuf| {
        if path.exists() && !roots.contains(&path) {
            roots.push(path);
        }
    };

    if let Ok(path) = env::var("ANDROID_HOME") {
        push(PathBuf::from(path));
    }
    if let Ok(path) = env::var("ANDROID_SDK_ROOT") {
        push(PathBuf::from(path));
    }
    if let Ok(home) = env::var("HOME") {
        let home = PathBuf::from(home);
        push(home.join("Library/Android/sdk"));
        push(home.join("Android/Sdk"));
    }
    roots
}

#[must_use]
pub fn resolve_android_sdk_path() -> Option<PathBuf> {
    sdk_roots().into_iter().next()
}

#[must_use]
pub fn find_android_tool(tool: &str) -> Option<PathBuf> {
    if let Ok(path) = which(tool) {
        return Some(path);
    }

    let suffixes: &[&str] = match tool {
        "adb" => &["platform-tools/adb", "platform-tools/adb.exe"],
        "emulator" => &["emulator/emulator", "emulator/emulator.exe"],
        _ => return None,
    };

    for root in sdk_roots() {
        for suffix in suffixes {
            let candidate = root.join(suffix);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolve the Android NDK path, preferring ANDROID_NDK_HOME if set.
#[must_use]
pub fn resolve_ndk_path() -> Option<PathBuf> {
    // Check environment variable first
    if let Ok(path) = env::var("ANDROID_NDK_HOME") {
        let path = PathBuf::from(path);
        if path.exists() && ndk_toolchain_bin(&path).is_some() {
            return Some(path);
        }
    }

    // Search in SDK roots
    for sdk_root in sdk_roots() {
        // Check ndk-bundle (legacy location)
        let ndk_bundle = sdk_root.join("ndk-bundle");
        if ndk_bundle.exists() && ndk_toolchain_bin(&ndk_bundle).is_some() {
            return Some(ndk_bundle);
        }

        // Check versioned NDK directories (prefer newest)
        let ndk_dir = sdk_root.join("ndk");
        if let Ok(entries) = fs::read_dir(&ndk_dir) {
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            candidates.sort_by(|a, b| b.cmp(a)); // Newest first

            for candidate in candidates {
                if ndk_toolchain_bin(&candidate).is_some() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Resolve the NDK path, setting environment variables as needed.
fn resolve_and_configure_ndk() -> Result<PathBuf> {
    let ndk_path = resolve_ndk_path()
        .ok_or_else(|| eyre!("Android NDK not found. Install it and set ANDROID_NDK_HOME."))?;

    let was_set = env::var("ANDROID_NDK_HOME").is_ok();
    // SAFETY: environment mutation happens on the main CLI thread before other threads run.
    unsafe {
        env::set_var("ANDROID_NDK_HOME", &ndk_path);
        env::set_var("ANDROID_NDK_ROOT", &ndk_path);
        env::set_var("ANDROID_NDK", &ndk_path);
    }

    if !was_set {
        info!(
            "ANDROID_NDK_HOME not set; using auto-detected NDK at {}",
            ndk_path.display()
        );
    }

    Ok(ndk_path)
}

pub(crate) fn ndk_toolchain_bin(ndk_root: &Path) -> Option<(String, PathBuf)> {
    let host_tags: &[&str] = if cfg!(target_os = "macos") {
        &["darwin-arm64", "darwin-aarch64", "darwin-x86_64"]
    } else if cfg!(target_os = "linux") {
        &["linux-aarch64", "linux-x86_64"]
    } else if cfg!(target_os = "windows") {
        &["windows-x86_64"]
    } else {
        &[]
    };

    let prebuilt = ndk_root.join("toolchains/llvm/prebuilt");

    // Try known host tags first
    for tag in host_tags {
        let bin = prebuilt.join(tag).join("bin");
        if bin.exists() {
            return Some(((*tag).to_string(), bin));
        }
    }

    // Fallback: scan directory
    if let Ok(entries) = fs::read_dir(&prebuilt) {
        for entry in entries.flatten() {
            let bin = entry.path().join("bin");
            if bin.exists() {
                let tag = entry
                    .file_name()
                    .into_string()
                    .unwrap_or_else(|_| "unknown".to_string());
                return Some((tag, bin));
            }
        }
    }
    None
}

// ============================================================================
// CMake Resolution (Prefer SDK CMake)
// ============================================================================

/// Find CMake, preferring Android SDK cmake over system cmake.
#[must_use]
pub fn resolve_cmake_path() -> Option<PathBuf> {
    // Priority 1: Android SDK cmake (includes Ninja, designed for cross-compilation)
    for sdk_root in sdk_roots() {
        if let Some(path) = find_sdk_cmake(&sdk_root) {
            debug!("Found SDK CMake at {}", path.display());
            return Some(path);
        }
    }

    // Priority 2: Check NDK parent for SDK cmake
    if let Some(ndk_root) = resolve_ndk_path() {
        if let Some(sdk_root) = ndk_root.parent().and_then(Path::parent) {
            if let Some(path) = find_sdk_cmake(sdk_root) {
                debug!("Found SDK CMake via NDK at {}", path.display());
                return Some(path);
            }
        }
    }

    // Priority 3: System cmake (fallback)
    if let Ok(path) = which("cmake") {
        debug!("Using system CMake at {}", path.display());
        return Some(path);
    }

    None
}

fn find_sdk_cmake(sdk_root: &Path) -> Option<PathBuf> {
    let cmake_dir = sdk_root.join("cmake");
    let mut candidates: Vec<PathBuf> = fs::read_dir(&cmake_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();

    // Sort descending to get newest version first
    candidates.sort_by(|a, b| b.cmp(a));

    for dir in candidates {
        let binary = if cfg!(windows) {
            dir.join("bin/cmake.exe")
        } else {
            dir.join("bin/cmake")
        };
        if binary.exists() {
            return Some(binary);
        }
    }
    None
}

/// Prepare CMake environment for Android builds.
pub fn prepare_cmake_env(targets: &[&str]) -> Result<PathBuf> {
    let cmake_path = resolve_cmake_path().ok_or_else(|| {
        eyre!(
            "CMake not found. Install via Android SDK Manager (SDK Tools → CMake) or `brew install cmake`."
        )
    })?;

    info!("Using CMake from {}", cmake_path.display());

    let bin_dir = cmake_path.parent();
    let ninja_available = bin_dir.is_some_and(|d| d.join("ninja").exists());

    // Add cmake bin to PATH
    if let Some(bin_dir) = bin_dir {
        prepend_to_path(bin_dir);
    }

    // Set cmake environment variables
    // SAFETY: main CLI thread mutates environment before spawning children.
    unsafe {
        env::set_var("CMAKE", &cmake_path);
        env::set_var("AWS_LC_SYS_CMAKE", &cmake_path);

        if ninja_available {
            env::set_var("CMAKE_GENERATOR", "Ninja");
            env::set_var("AWS_LC_SYS_CMAKE_GENERATOR", "Ninja");
        }

        // Set target-specific variables
        for triple in targets {
            let key = triple.replace('-', "_");
            env::set_var(format!("CMAKE_{key}"), &cmake_path);
            env::set_var(format!("AWS_LC_SYS_CMAKE_{key}"), &cmake_path);
            if ninja_available {
                env::set_var(format!("CMAKE_GENERATOR_{key}"), "Ninja");
                env::set_var(format!("AWS_LC_SYS_CMAKE_GENERATOR_{key}"), "Ninja");
            }
        }
    }

    Ok(cmake_path)
}

// ============================================================================
// Environment Helpers
// ============================================================================

fn prepend_to_path(dir: &Path) {
    let mut entries =
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    if !entries.iter().any(|e| e == dir) {
        entries.insert(0, dir.to_path_buf());
        if let Ok(joined) = env::join_paths(&entries) {
            // SAFETY: main CLI thread mutates environment.
            unsafe {
                env::set_var("PATH", joined);
            }
        }
    }
}

fn installed_rust_targets() -> Option<HashSet<String>> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
    )
}

// ============================================================================
// Target Resolution
// ============================================================================

fn target_by_triple(triple: &str) -> Option<&'static AndroidTargetConfig> {
    ANDROID_TARGETS.iter().find(|t| t.triple == triple)
}

fn target_for_abi(abi: &str) -> Option<&'static AndroidTargetConfig> {
    match abi {
        "arm64-v8a" | "arm64" => target_by_triple("aarch64-linux-android"),
        "armeabi-v7a" | "armeabi" => target_by_triple("armv7-linux-androideabi"),
        "x86_64" => target_by_triple("x86_64-linux-android"),
        "x86" => target_by_triple("i686-linux-android"),
        _ => None,
    }
}

fn resolve_android_target(triple: &str) -> Result<&'static AndroidTargetConfig> {
    target_by_triple(triple).ok_or_else(|| {
        let supported = ANDROID_TARGETS
            .iter()
            .map(|t| t.triple)
            .collect::<Vec<_>>()
            .join(", ");
        eyre!("Unsupported Android target '{triple}'. Supported: {supported}")
    })
}

fn determine_build_targets(explicit: Option<&[&str]>) -> Result<Vec<&'static AndroidTargetConfig>> {
    // Priority 1: Explicitly requested targets
    if let Some(list) = explicit {
        return list.iter().map(|t| resolve_android_target(t)).collect();
    }

    // Priority 2: ANDROID_BUILD_TARGETS environment variable
    if let Ok(raw) = env::var("ANDROID_BUILD_TARGETS") {
        let targets: Result<Vec<_>> = raw
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(resolve_android_target)
            .collect();
        if let Ok(t) = targets {
            if !t.is_empty() {
                return Ok(t);
            }
        }
    }

    // Priority 3: All installed Android targets
    if let Some(installed) = installed_rust_targets() {
        let selected: Vec<_> = ANDROID_TARGETS
            .iter()
            .filter(|t| installed.contains(t.triple))
            .collect();
        if !selected.is_empty() {
            return Ok(selected);
        }
    }

    // Default: arm64 only
    Ok(vec![target_by_triple("aarch64-linux-android").unwrap()])
}

/// Determine the preferred Rust targets for the connected Android device.
pub fn device_preferred_targets(
    adb_path: &Path,
    identifier: Option<&str>,
) -> Result<Vec<&'static str>> {
    let abis = query_device_abis(adb_path, identifier)?;
    if abis.is_empty() {
        bail!("Unable to determine Android device ABI via adb.");
    }

    let mut seen = HashSet::new();
    let targets: Vec<_> = abis
        .iter()
        .filter_map(|abi| target_for_abi(abi))
        .filter(|t| seen.insert(t.triple))
        .map(|t| t.triple)
        .collect();

    if targets.is_empty() {
        bail!(
            "No supported Rust targets match device ABIs: {}",
            abis.join(", ")
        );
    }
    Ok(targets)
}

fn query_device_abis(adb_path: &Path, identifier: Option<&str>) -> Result<Vec<String>> {
    for prop in [
        "ro.product.cpu.abilist",
        "ro.product.cpu.abi",
        "ro.product.cpu.abi2",
    ] {
        let output = adb_command(adb_path, identifier)
            .args(["shell", "getprop", prop])
            .output()
            .with_context(|| format!("failed to query device property {prop}"))?;

        if !output.status.success() {
            continue;
        }

        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            continue;
        }

        let abis: Vec<String> = value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if !abis.is_empty() {
            return Ok(abis);
        }
    }
    Ok(Vec::new())
}

// ============================================================================
// Toolchain Configuration
// ============================================================================

/// Configure the environment so Cargo can link against the requested Android targets.
pub fn configure_rust_android_linker_env(desired_triples: &[&str]) -> Result<()> {
    let ndk_path = resolve_and_configure_ndk()?;
    let (_host_tag, toolchain_bin) = ndk_toolchain_bin(&ndk_path)
        .ok_or_else(|| eyre!("Unable to locate NDK LLVM toolchain binaries."))?;

    prepend_to_path(&toolchain_bin);

    let installed = installed_rust_targets();
    let mut configured = Vec::new();

    for &triple in desired_triples {
        let target = target_by_triple(triple)
            .ok_or_else(|| eyre!("Unsupported Android target `{triple}`"))?;

        // Verify target is installed
        if let Some(ref installed) = installed {
            if !installed.contains(target.triple) {
                bail!(
                    "Rust target `{}` is not installed. Run `rustup target add {}` and retry.",
                    target.triple,
                    target.triple
                );
            }
        }

        configure_target_toolchain_env(target, &toolchain_bin)?;
        configured.push(target.triple.to_string());
    }

    if configured.is_empty() {
        bail!("No Android Rust targets were configured.");
    }

    info!("Configured Android Rust targets: {}", configured.join(", "));

    // SAFETY: single-threaded mutation of process environment in CLI.
    unsafe {
        env::set_var("ANDROID_BUILD_TARGETS", configured.join(","));
    }

    Ok(())
}

fn configure_target_toolchain_env(
    target: &AndroidTargetConfig,
    toolchain_bin: &Path,
) -> Result<()> {
    let api_levels: Vec<u32> = (target.min_api..=MAX_API).rev().collect();

    let clang = find_toolchain_binary(toolchain_bin, target, &api_levels, "clang")
        .ok_or_else(|| eyre!("Unable to find clang for target {} in NDK", target.triple))?;

    let clang_pp = find_toolchain_binary(toolchain_bin, target, &api_levels, "clang++")
        .or_else(|| {
            clang
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| clang.with_file_name(format!("{n}++")))
                .filter(|p| p.exists())
        })
        .ok_or_else(|| eyre!("Unable to find clang++ for target {} in NDK", target.triple))?;

    let ar = toolchain_bin.join("llvm-ar");
    let ranlib = toolchain_bin.join("llvm-ranlib");
    let env_key = target.triple.replace('-', "_");
    let env_key_upper = env_key.to_uppercase();

    // SAFETY: CLI configures environment on main thread before parallel work.
    unsafe {
        env::set_var(format!("CC_{env_key}"), &clang);
        env::set_var(format!("CXX_{env_key}"), &clang_pp);
        env::set_var(format!("CARGO_TARGET_{env_key_upper}_LINKER"), &clang);

        if ar.exists() {
            env::set_var(format!("AR_{env_key}"), &ar);
            env::set_var(format!("CARGO_TARGET_{env_key_upper}_AR"), &ar);
        }
        if ranlib.exists() {
            env::set_var(format!("RANLIB_{env_key}"), &ranlib);
        }
    }

    Ok(())
}

fn find_toolchain_binary(
    toolchain_bin: &Path,
    target: &AndroidTargetConfig,
    api_levels: &[u32],
    tool: &str,
) -> Option<PathBuf> {
    // Try versioned binaries first (e.g., aarch64-linux-android35-clang)
    for level in api_levels {
        let candidate = toolchain_bin.join(format!("{}{level}-{tool}", target.bin_prefix));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Fallback to unversioned
    let fallback = toolchain_bin.join(format!("{}-{tool}", target.bin_prefix));
    fallback.exists().then_some(fallback)
}

// ============================================================================
// Java Detection
// ============================================================================

fn detect_java_major(home: &Path) -> Option<u32> {
    let java_bin = home.join("bin/java");
    if !java_bin.exists() {
        return None;
    }

    let output = Command::new(java_bin).arg("-version").output().ok()?;
    let text = if output.stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout)
    } else {
        String::from_utf8_lossy(&output.stderr)
    };

    // Parse version from output like: openjdk version "17.0.1" or java version "1.8.0_321"
    for line in text.lines() {
        if let Some(start) = line.find('"') {
            let rest = &line[start + 1..];
            if let Some(end) = rest.find('"') {
                let version = &rest[..end];
                let major = version
                    .strip_prefix("1.")
                    .unwrap_or(version)
                    .split(|c: char| !c.is_ascii_digit())
                    .next()?;
                return major.parse().ok();
            }
        }
    }
    None
}

fn prefer_java_home() -> Option<PathBuf> {
    const MAX_SUPPORTED: u32 = 21;

    let check = |path: Option<PathBuf>| -> Option<PathBuf> {
        let path = path?;
        match detect_java_major(&path) {
            Some(v) if v <= MAX_SUPPORTED => Some(path),
            _ => None,
        }
    };

    // Check environment variables
    if let Some(home) = check(env::var_os("JAVA_HOME").map(PathBuf::from)) {
        return Some(home);
    }
    if let Some(home) = check(env::var_os("ANDROID_JAVA_HOME").map(PathBuf::from)) {
        return Some(home);
    }

    #[cfg(target_os = "macos")]
    {
        // Check Android Studio JBR
        for path in [
            "/Applications/Android Studio.app/Contents/jbr/Contents/Home",
            "/Applications/Android Studio Preview.app/Contents/jbr/Contents/Home",
        ] {
            if let Some(home) = check(Some(PathBuf::from(path))) {
                return Some(home);
            }
        }

        // Try java_home utility
        for version in ["17", "21", "20", "19", "18"] {
            if let Ok(output) = Command::new("/usr/libexec/java_home")
                .args(["-v", version])
                .output()
            {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        let candidate = PathBuf::from(path);
                        if detect_java_major(&candidate).is_some() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }
    }

    None
}

// ============================================================================
// Package Name Utilities
// ============================================================================

/// Convert arbitrary bundle identifiers into valid Java package names.
#[must_use]
pub fn sanitize_package_name(input: &str) -> String {
    let parts: Vec<String> = input
        .split('.')
        .filter(|s| !s.is_empty())
        .map(|raw| {
            let mut segment = String::new();
            for ch in raw.chars() {
                match ch {
                    'a'..='z' | '0'..='9' => segment.push(ch),
                    'A'..='Z' => segment.push(ch.to_ascii_lowercase()),
                    '_' | '-' => segment.push('_'),
                    _ if ch.is_alphanumeric() => segment.push(ch.to_ascii_lowercase()),
                    _ => segment.push('_'),
                }
            }

            if segment.is_empty() {
                segment.push('a');
            } else if !segment.chars().next().unwrap().is_ascii_alphabetic()
                && !segment.starts_with('_')
            {
                segment.insert(0, 'a');
            }
            segment
        })
        .collect();

    if parts.is_empty() {
        "com.waterui.app".to_string()
    } else {
        parts.join(".")
    }
}

fn prepare_android_package(project_dir: &Path, bundle_identifier: &str) -> Result<String> {
    let sanitized = sanitize_package_name(bundle_identifier);
    if sanitized == bundle_identifier {
        return Ok(sanitized);
    }

    let android_dir = project_dir.join("android");
    let app_dir = android_dir.join("app");

    // Update build.gradle.kts
    let gradle_file = app_dir.join("build.gradle.kts");
    if gradle_file.exists() {
        let contents = fs::read_to_string(&gradle_file)?;
        if contents.contains(bundle_identifier) {
            fs::write(
                &gradle_file,
                contents.replace(bundle_identifier, &sanitized),
            )?;
        }
    }

    // Move Java/Kotlin source directories
    let java_dir = app_dir.join("src/main/java");
    let original_path = java_dir.join(bundle_identifier.replace('.', "/"));
    let sanitized_path = java_dir.join(sanitized.replace('.', "/"));

    if original_path.exists() && original_path != sanitized_path {
        move_dir_contents(&original_path, &sanitized_path)?;
        fs::remove_dir_all(&original_path).ok();
    }

    // Update MainActivity.kt
    let activity_path = if sanitized_path.exists() {
        sanitized_path.join("MainActivity.kt")
    } else {
        original_path.join("MainActivity.kt")
    };

    if activity_path.exists() {
        let contents = fs::read_to_string(&activity_path)?;
        if contents.contains(bundle_identifier) {
            fs::write(
                &activity_path,
                contents.replace(bundle_identifier, &sanitized),
            )?;
        }
    }

    info!(
        "Adjusted Android bundle identifier from '{}' to '{}'",
        bundle_identifier, sanitized
    );

    Ok(sanitized)
}

fn move_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            move_dir_contents(&src_path, &dst_path)?;
            if src_path != dst_path {
                fs::remove_dir_all(&src_path).ok();
            }
        } else if let Err(err) = fs::rename(&src_path, &dst_path) {
            // Handle cross-device moves
            if err.kind() == std::io::ErrorKind::Unsupported || err.raw_os_error() == Some(18)
            // EXDEV
            {
                fs::copy(&src_path, &dst_path)?;
                fs::remove_file(&src_path)?;
            } else {
                return Err(err).context("failed to move file");
            }
        }
    }
    Ok(())
}

// ============================================================================
// Native Library Build
// ============================================================================

#[derive(Debug)]
pub struct AndroidNativeBuildOptions<'a> {
    pub project_dir: &'a Path,
    pub android_config: &'a Android,
    pub crate_name: &'a str,
    pub release: bool,
    pub requested_triples: Option<Vec<String>>,
    pub enable_sccache: bool,
    pub enable_mold: bool,
    pub hot_reload: bool,
}

#[derive(Debug)]
pub struct AndroidNativeBuildReport {
    pub profile: String,
    pub targets: Vec<String>,
    pub jni_libs_dir: PathBuf,
}

pub fn build_android_native_libraries(
    opts: AndroidNativeBuildOptions<'_>,
) -> Result<AndroidNativeBuildReport> {
    let profile = if opts.release { "release" } else { "debug" };

    // Determine targets
    let requested_refs: Option<Vec<&str>> = opts
        .requested_triples
        .as_ref()
        .map(|v| v.iter().map(String::as_str).collect());
    let targets = determine_build_targets(requested_refs.as_deref())?;

    if targets.is_empty() {
        bail!("No Android Rust targets selected for build.");
    }

    // Configure environment
    let triples: Vec<&str> = targets.iter().map(|t| t.triple).collect();
    configure_rust_android_linker_env(&triples)?;
    let cmake_path = prepare_cmake_env(&triples)?;

    let ndk_path = env::var("ANDROID_NDK_HOME")
        .map(PathBuf::from)
        .map_err(|_| eyre!("ANDROID_NDK_HOME is not set"))?;
    let (host_tag, _) = ndk_toolchain_bin(&ndk_path)
        .ok_or_else(|| eyre!("Unable to locate NDK toolchain binaries."))?;

    // Prepare output directory
    let android_root = opts.project_dir.join(&opts.android_config.project_path);
    let jni_dir = android_root.join("app/src/main/jniLibs");
    util::ensure_directory(&jni_dir)?;

    let crate_file = opts.crate_name.replace('-', "_");
    let mut built = Vec::new();

    // Build each target
    for target in &targets {
        info!(
            "Building Rust crate '{}' for {}",
            opts.crate_name, target.triple
        );

        clean_aws_lc_cmake_cache(opts.project_dir, target.triple, profile);

        let build_result = build_single_target(
            opts.project_dir,
            opts.crate_name,
            target,
            profile,
            &cmake_path,
            &ndk_path,
            opts.hot_reload,
            opts.enable_sccache,
            opts.enable_mold,
        )?;

        if !build_result {
            bail!("cargo build failed for target {}", target.triple);
        }

        // Copy built library
        let source = opts
            .project_dir
            .join("target")
            .join(target.triple)
            .join(profile)
            .join(format!("lib{crate_file}.so"));

        if !source.exists() {
            bail!(
                "Rust shared library not found at {}. Did the build succeed?",
                source.display()
            );
        }

        let abi_dir = jni_dir.join(target.abi);
        util::ensure_directory(&abi_dir)?;

        let dest = abi_dir.join(format!("lib{crate_file}.so"));
        fs::copy(&source, &dest)
            .with_context(|| format!("failed to copy library to {}", dest.display()))?;

        info!("→ Copied {}", dest.display());
        built.push(target.triple.to_string());
    }

    if built.is_empty() {
        bail!("No Android Rust targets were built.");
    }

    // Copy libc++_shared.so for each target
    let libcxx_root = ndk_path
        .join("toolchains/llvm/prebuilt")
        .join(&host_tag)
        .join("sysroot/usr/lib");

    for target in &targets {
        let src = libcxx_root.join(target.triple).join("libc++_shared.so");
        let dst = jni_dir.join(target.abi).join("libc++_shared.so");

        if src.exists() {
            fs::copy(&src, &dst)
                .with_context(|| format!("failed to copy libc++_shared.so for {}", target.abi))?;
            info!("  → Copied libc++_shared.so for {}", target.abi);
        } else {
            warn!(
                "libc++_shared.so not found for {} at {}",
                target.triple,
                src.display()
            );
        }
    }

    info!("Rust libraries copied to {}", jni_dir.display());

    Ok(AndroidNativeBuildReport {
        profile: profile.to_string(),
        targets: built,
        jni_libs_dir: jni_dir,
    })
}

fn build_single_target(
    project_dir: &Path,
    crate_name: &str,
    target: &AndroidTargetConfig,
    profile: &str,
    cmake_path: &Path,
    ndk_path: &Path,
    hot_reload: bool,
    enable_sccache: bool,
    enable_mold: bool,
) -> Result<bool> {
    let make_command = || {
        let mut cmd = Command::new("cargo");
        cmd.arg("build")
            .arg("--package")
            .arg(crate_name)
            .arg("--target")
            .arg(target.triple);

        if profile == "release" {
            cmd.arg("--release");
        }

        cmd.current_dir(project_dir);

        // Set CMake environment
        let key = target.triple.replace('-', "_");
        let ninja_available = cmake_path
            .parent()
            .is_some_and(|d| d.join("ninja").exists());

        cmd.env("CMAKE", cmake_path);
        cmd.env("AWS_LC_SYS_CMAKE", cmake_path);
        cmd.env(format!("CMAKE_{key}"), cmake_path);
        cmd.env(format!("AWS_LC_SYS_CMAKE_{key}"), cmake_path);

        if ninja_available {
            cmd.env("CMAKE_GENERATOR", "Ninja");
            cmd.env("AWS_LC_SYS_CMAKE_GENERATOR", "Ninja");
            cmd.env(format!("CMAKE_GENERATOR_{key}"), "Ninja");
            cmd.env(format!("AWS_LC_SYS_CMAKE_GENERATOR_{key}"), "Ninja");
        }

        // Add cmake bin to PATH
        if let Some(bin_dir) = cmake_path.parent() {
            let mut path =
                env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
            if !path.iter().any(|e| e == bin_dir) {
                path.insert(0, bin_dir.to_path_buf());
            }
            if let Ok(joined) = env::join_paths(&path) {
                cmd.env("PATH", joined);
            }
        }

        cmd.env("ANDROID_NDK_HOME", ndk_path);
        cmd.env("ANDROID_NDK_ROOT", ndk_path);
        cmd.env("ANDROID_NDK", ndk_path);

        util::configure_hot_reload_env(&mut cmd, hot_reload, None);
        cmd
    };

    // First attempt
    let mut cmd = make_command();
    let sccache_enabled = util::configure_build_speedups(&mut cmd, enable_sccache, enable_mold);

    let status = cmd
        .status()
        .with_context(|| format!("failed to compile {} for {}", crate_name, target.triple))?;

    if status.success() {
        return Ok(true);
    }

    // Retry without sccache if it was enabled
    if sccache_enabled {
        warn!(
            "cargo build failed for {} with sccache; retrying without cache",
            target.triple
        );

        let mut retry_cmd = make_command();
        util::configure_build_speedups(&mut retry_cmd, false, enable_mold);

        let retry_status = retry_cmd.status().with_context(|| {
            format!(
                "failed to re-run cargo build for {} without sccache",
                target.triple
            )
        })?;

        return Ok(retry_status.success());
    }

    Ok(false)
}

/// Clean aws-lc-sys CMake cache to prevent stale build issues.
pub fn clean_aws_lc_cmake_cache(project_dir: &Path, target: &str, profile: &str) {
    let build_root = project_dir
        .join("target")
        .join(target)
        .join(profile)
        .join("build");

    if let Ok(entries) = fs::read_dir(&build_root) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("aws-lc-sys-")
            {
                if let Err(err) = fs::remove_dir_all(entry.path()) {
                    warn!(
                        "Failed to clear aws-lc-sys cache {}: {err}",
                        entry.path().display()
                    );
                }
            }
        }
    }
}

// ============================================================================
// APK Build
// ============================================================================

/// Build the Android APK using the generated Gradle project.
pub fn build_android_apk(
    project_dir: &Path,
    android_config: &Android,
    release: bool,
    skip_native: bool,
    hot_reload_enabled: bool,
    bundle_identifier: &str,
    crate_name: &str,
    enable_sccache: bool,
    mold_requested: bool,
) -> Result<PathBuf> {
    prepare_android_package(project_dir, bundle_identifier)?;

    if skip_native {
        info!("Skipping Android native build (requested via --skip-native)");
    } else {
        build_android_native_libraries(AndroidNativeBuildOptions {
            project_dir,
            android_config,
            crate_name,
            release,
            requested_triples: None,
            enable_sccache,
            enable_mold: mold_requested,
            hot_reload: hot_reload_enabled,
        })?;
    }

    info!("Building Android app with Gradle...");

    let android_dir = project_dir.join(&android_config.project_path);

    // Ensure local.properties exists
    let local_properties = android_dir.join("local.properties");
    if !local_properties.exists() {
        let sdk_path = resolve_android_sdk_path().ok_or_else(|| {
            eyre!(
                "Android SDK not found. Install Android Studio or set ANDROID_HOME/ANDROID_SDK_ROOT."
            )
        })?;

        fs::write(
            &local_properties,
            format!("sdk.dir={}\n", sdk_path.to_string_lossy()),
        )
        .context("failed to write local.properties")?;

        info!(
            "Wrote SDK location {} to {}",
            sdk_path.display(),
            local_properties.display()
        );
    }

    // Run Gradle
    let gradlew = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "./gradlew"
    };
    let mut cmd = Command::new(gradlew);

    util::configure_hot_reload_env(&mut cmd, hot_reload_enabled, None);

    // Configure JVM options
    let ipv4_flag = "-Djava.net.preferIPv4Stack=true";
    cmd.env(
        "GRADLE_OPTS",
        ensure_jvm_flag(env::var("GRADLE_OPTS").ok(), ipv4_flag),
    );
    cmd.env(
        "JAVA_TOOL_OPTIONS",
        ensure_jvm_flag(env::var("JAVA_TOOL_OPTIONS").ok(), ipv4_flag),
    );

    // Set Java home
    if let Some(java_home) = prefer_java_home() {
        info!("Using Java from {}", java_home.display());
        cmd.env("JAVA_HOME", &java_home);

        let java_bin = java_home.join("bin");
        if java_bin.exists() {
            let mut path =
                env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
            if !path.iter().any(|e| e == &java_bin) {
                path.insert(0, java_bin);
                if let Ok(joined) = env::join_paths(&path) {
                    cmd.env("PATH", joined);
                }
            }
        }
    }

    let task = if release {
        "assembleRelease"
    } else {
        "assembleDebug"
    };
    cmd.arg(task).current_dir(&android_dir);

    debug!("Running command: {:?}", cmd);

    let status = cmd.status().context("failed to run gradlew")?;
    if !status.success() {
        bail!("Gradle build failed");
    }

    // Locate APK
    let profile = if release { "release" } else { "debug" };
    let apk_name = if release {
        "app-release.apk"
    } else {
        "app-debug.apk"
    };
    let apk_path = android_dir.join(format!("app/build/outputs/apk/{profile}/{apk_name}"));

    if !apk_path.exists() {
        bail!("APK not found at {}", apk_path.display());
    }

    info!("Generated {} APK at {}", profile, apk_path.display());
    Ok(apk_path)
}

fn ensure_jvm_flag(existing: Option<String>, flag: &str) -> String {
    match existing {
        None => flag.to_string(),
        Some(current) => {
            let trimmed = current.trim();
            if trimmed.split_whitespace().any(|t| t == flag) {
                trimmed.to_string()
            } else if trimmed.is_empty() {
                flag.to_string()
            } else {
                format!("{trimmed} {flag}")
            }
        }
    }
}

// ============================================================================
// ADB Utilities
// ============================================================================

/// Block until an Android device reports as ready via `adb`.
pub fn wait_for_android_device(adb_path: &Path, identifier: Option<&str>) -> Result<()> {
    let status = adb_command(adb_path, identifier)
        .arg("wait-for-device")
        .status()
        .context("failed to run adb wait-for-device")?;

    if !status.success() {
        bail!("'adb wait-for-device' failed. Is the device/emulator running correctly?");
    }

    // Wait for boot completion
    loop {
        let output = adb_command(adb_path, identifier)
            .args(["shell", "getprop", "sys.boot_completed"])
            .output()?;

        if String::from_utf8_lossy(&output.stdout).trim() == "1" {
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }

    Ok(())
}

#[must_use]
pub fn adb_command(adb_path: &Path, identifier: Option<&str>) -> Command {
    let mut cmd = Command::new(adb_path);
    if let Some(id) = identifier {
        cmd.arg("-s").arg(id);
    }
    cmd
}
