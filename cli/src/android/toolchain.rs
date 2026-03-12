use std::{
    cmp::Ordering,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{self, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre;

use crate::{
    android::platform::{ALL_ABIS, AndroidAbi},
    brew::Brew,
    toolchain::{
        Installation, Toolchain, ToolchainError,
        cmake::Cmake,
        linux::{has_supported_package_manager, install_java_jdk},
        winget::{WingetInstallError, ensure_package_installed},
    },
    utils::{command, run_command, which},
};

/// Complete Android toolchain including SDK, platforms, NDK, Rust targets, platform-tools, Java, and CMake.
pub type AndroidToolchain = (
    AndroidSdk,
    AndroidSdkPlatforms,
    AndroidBuildTools,
    AndroidNdk,
    AndroidRustTargets,
    AndroidPlatformTools,
    Java,
    Cmake,
);

/// Android SDK toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidSdk;

/// Android Platform-Tools (`adb`) toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidPlatformTools;

/// Android SDK platform packages (`platforms/android-*`) used for compilation.
#[derive(Debug, Clone, Default)]
pub struct AndroidSdkPlatforms;

/// Android SDK build-tools packages (`build-tools;*`) used for D8/Kotlin dexing.
#[derive(Debug, Clone, Default)]
pub struct AndroidBuildTools;

/// Rust targets required for Android cross-compilation.
#[derive(Debug, Clone, Default)]
pub struct AndroidRustTargets;

/// An Android NDK toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidNdk;

/// Java toolchain component for Android development.
#[derive(Debug, Clone, Default)]
pub struct Java;

/// Kotlin toolchain component for Android development.
#[derive(Debug, Clone, Default)]
pub struct Kotlin;

/// Host-specific Android Studio installation guidance.
#[must_use]
pub const fn android_studio_install_suggestion() -> &'static str {
    if cfg!(target_os = "windows") {
        "Install Android Studio with winget: `winget install --id Google.AndroidStudio --exact`."
    } else if cfg!(target_os = "macos") {
        "Install Android Studio with Homebrew: `brew install --cask android-studio`, or download it from https://developer.android.com/studio."
    } else if cfg!(target_os = "linux") {
        "Install Android Studio from https://developer.android.com/studio."
    } else {
        "Install Android Studio from https://developer.android.com/studio."
    }
}

/// Android command-line tools guidance for headless/server environments.
#[must_use]
pub const fn android_cmdline_tools_suggestion() -> &'static str {
    "Install Android SDK command-line tools and ensure `sdkmanager` is available in PATH."
}

/// Host-specific Android SDK default path guidance.
#[must_use]
pub const fn android_sdk_path_suggestion() -> &'static str {
    if cfg!(target_os = "windows") {
        "Expected default SDK path is `%LOCALAPPDATA%\\Android\\Sdk`. Set `ANDROID_SDK_ROOT` to that path if needed."
    } else if cfg!(target_os = "macos") {
        "Expected default SDK path is `$HOME/Library/Android/sdk`. Set `ANDROID_SDK_ROOT` to that path if needed."
    } else if cfg!(target_os = "linux") {
        "Expected default SDK path is `$HOME/Android/Sdk`. Set `ANDROID_SDK_ROOT` to that path if needed."
    } else {
        "Set `ANDROID_SDK_ROOT` to your Android SDK path."
    }
}

/// Guidance for installing Android Platform-Tools (`adb`) without assuming Android Studio.
#[must_use]
pub const fn android_platform_tools_suggestion() -> &'static str {
    "Install Android Platform-Tools with `sdkmanager --install \"platform-tools\"` (or Android Studio SDK Manager), then ensure `ANDROID_SDK_ROOT` points to that SDK."
}

/// Guidance for installing Android NDK without assuming Android Studio.
#[must_use]
pub const fn android_ndk_install_suggestion() -> &'static str {
    "Install Android NDK with `sdkmanager --install \"ndk;<version>\"` (or Android Studio SDK Manager), then set `ANDROID_NDK_ROOT` if using a custom location."
}

/// Guidance for installing Android SDK platforms needed by build/package workflows.
#[must_use]
pub const fn android_platforms_install_suggestion() -> &'static str {
    "Install Android SDK platform packages with `sdkmanager --install \"platforms;android-<api>\"` (or Android Studio SDK Manager)."
}

/// Guidance for installing Android SDK Build-Tools needed by build/package workflows.
#[must_use]
pub const fn android_build_tools_install_suggestion() -> &'static str {
    "Install Android SDK Build-Tools with `sdkmanager --install \"build-tools;<version>\"` (or Android Studio SDK Manager)."
}

/// Guidance for installing Rust Android targets needed by Android build/package workflows.
#[must_use]
pub const fn android_rust_targets_install_suggestion() -> &'static str {
    "Install Rust Android targets with `rustup target add aarch64-linux-android x86_64-linux-android armv7-linux-androideabi i686-linux-android`."
}

/// Host-specific Java installation guidance for Android Gradle builds.
#[must_use]
pub const fn java_install_suggestion() -> &'static str {
    if cfg!(target_os = "windows") {
        "Install JDK with winget: `winget install --id Microsoft.OpenJDK.21 --exact`, or use Android Studio's bundled JBR."
    } else if cfg!(target_os = "macos") {
        "Install JDK with Homebrew: `brew install --cask temurin`, or use Android Studio's bundled JBR."
    } else if cfg!(target_os = "linux") {
        "Install JDK with your package manager (for example `openjdk-21-jdk`, `java-21-openjdk-devel`, or `jdk-openjdk`), or use Android Studio's bundled JBR."
    } else {
        "Install a JDK and set `JAVA_HOME`, or use Android Studio's bundled JBR."
    }
}

/// Host-specific Kotlin compiler guidance.
#[must_use]
pub const fn kotlin_install_suggestion() -> &'static str {
    if cfg!(target_os = "windows") {
        "Install Android Studio (includes Kotlin), or install Kotlin manually and set `KOTLIN_HOME`."
    } else if cfg!(target_os = "macos") {
        "Install Android Studio (includes Kotlin), or install Kotlin manually and ensure `kotlinc` is in PATH."
    } else if cfg!(target_os = "linux") {
        "Install Android Studio (includes Kotlin), or install Kotlin manually and ensure `kotlinc` is in PATH."
    } else {
        "Install Kotlin compiler (`kotlinc`) and set `KOTLIN_HOME` if needed."
    }
}

fn sdkmanager_search_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["sdkmanager.bat", "sdkmanager.exe", "sdkmanager"]
    } else {
        &["sdkmanager"]
    }
}

fn sdkmanager_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sdkmanager.bat"
    } else {
        "sdkmanager"
    }
}

fn cmdline_tools_host_tag() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        Some("win")
    } else if cfg!(target_os = "macos") {
        Some("mac")
    } else if cfg!(target_os = "linux") {
        Some("linux")
    } else {
        None
    }
}

fn default_android_sdk_path() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let localappdata = env::var("LOCALAPPDATA").ok()?;
        return Some(PathBuf::from(localappdata).join("Android/Sdk"));
    }

    if cfg!(target_os = "macos") {
        let home = env::var("HOME").ok()?;
        return Some(PathBuf::from(home).join("Library/Android/sdk"));
    }

    if cfg!(target_os = "linux") {
        let home = env::var("HOME").ok()?;
        return Some(PathBuf::from(home).join("Android/Sdk"));
    }

    None
}

fn configured_android_sdk_path() -> Option<PathBuf> {
    for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Ok(raw) = env::var(key) {
            return Some(PathBuf::from(raw));
        }
    }
    default_android_sdk_path()
}

fn sdkmanager_candidates_under_sdk_root(sdk_root: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        vec![
            sdk_root.join("cmdline-tools/latest/bin/sdkmanager.bat"),
            sdk_root.join("cmdline-tools/bin/sdkmanager.bat"),
            sdk_root.join("tools/bin/sdkmanager.bat"),
        ]
    } else {
        vec![
            sdk_root.join("cmdline-tools/latest/bin/sdkmanager"),
            sdk_root.join("cmdline-tools/bin/sdkmanager"),
            sdk_root.join("tools/bin/sdkmanager"),
        ]
    }
}

fn parse_latest_cmdline_tools_archive(repository_xml: &str) -> Option<String> {
    let host_tag = cmdline_tools_host_tag()?;
    let prefix = format!("commandlinetools-{host_tag}-");
    let suffix = "_latest.zip";

    let mut cursor = 0usize;
    let mut best: Option<(u64, String)> = None;

    while let Some(offset) = repository_xml[cursor..].find(&prefix) {
        let start = cursor + offset + prefix.len();
        let remainder = &repository_xml[start..];
        let Some(suffix_offset) = remainder.find(suffix) else {
            cursor = start;
            continue;
        };

        let build_id = &remainder[..suffix_offset];
        let filename = format!("{prefix}{build_id}{suffix}");
        cursor = start + suffix_offset + suffix.len();

        if build_id.is_empty() || !build_id.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }

        let Ok(build_id) = build_id.parse::<u64>() else {
            continue;
        };

        match best {
            Some((current, _)) if build_id <= current => {}
            _ => best = Some((build_id, filename)),
        }
    }

    best.map(|(_, filename)| filename)
}

async fn latest_cmdline_tools_archive_url() -> eyre::Result<String> {
    use zenwave::{Client, Method, redirect::FollowRedirect};

    const REPOSITORY_URL: &str = "https://dl.google.com/android/repository/repository2-3.xml";
    const REPOSITORY_PREFIX: &str = "https://dl.google.com/android/repository/";

    let mut client = FollowRedirect::new(zenwave::client());
    let response = client.method(Method::GET, REPOSITORY_URL).await?;
    if !response.status().is_success() {
        return Err(eyre::eyre!(
            "Failed to query Android SDK repository metadata: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.into_body().into_bytes().await?;
    let repository_xml = String::from_utf8_lossy(&bytes).into_owned();
    let archive_name = parse_latest_cmdline_tools_archive(&repository_xml)
        .ok_or_else(|| eyre::eyre!("Could not locate Android command-line tools archive"))?;
    Ok(format!("{REPOSITORY_PREFIX}{archive_name}"))
}

async fn download_file_with_redirect(url: &str, destination: &Path) -> eyre::Result<()> {
    use zenwave::{Client, Method, redirect::FollowRedirect};

    let mut client = FollowRedirect::new(zenwave::client());
    let response = client.method(Method::GET, url).await?;
    if !response.status().is_success() {
        return Err(eyre::eyre!(
            "Failed to download {url}: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.into_body().into_bytes().await?;
    let destination = destination.to_path_buf();
    smol::unblock(move || std::fs::write(destination, &bytes)).await?;
    Ok(())
}

fn find_cmdline_tools_dir(root: &Path) -> eyre::Result<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let sdkmanager_name = sdkmanager_binary_name();

    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let is_sdkmanager = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(sdkmanager_name));
            if !is_sdkmanager {
                continue;
            }

            let bin_dir = path.parent().ok_or_else(|| {
                eyre::eyre!(
                    "Invalid Android command-line tools archive layout (missing bin directory)"
                )
            })?;
            let cmdline_tools_dir = bin_dir.parent().ok_or_else(|| {
                eyre::eyre!(
                    "Invalid Android command-line tools archive layout (missing cmdline-tools root)"
                )
            })?;
            return Ok(cmdline_tools_dir.to_path_buf());
        }
    }

    Err(eyre::eyre!(
        "Android command-line tools archive does not contain sdkmanager"
    ))
}

async fn ensure_cmdline_tools_available(sdk_root: &Path) -> eyre::Result<()> {
    let latest_dir = sdk_root.join("cmdline-tools/latest");
    let sdkmanager = latest_dir.join("bin").join(sdkmanager_binary_name());
    if sdkmanager.exists() {
        return Ok(());
    }

    let cmdline_tools_root = sdk_root.join("cmdline-tools");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
    let temp_dir =
        cmdline_tools_root.join(format!(".water-cmdline-tools-{}-{nonce}", process::id()));
    let extract_dir = temp_dir.join("extract");
    let archive_path = temp_dir.join("commandline-tools.zip");

    {
        let cmdline_tools_root = cmdline_tools_root.clone();
        let extract_dir = extract_dir.clone();
        smol::unblock(move || {
            std::fs::create_dir_all(&cmdline_tools_root)?;
            std::fs::create_dir_all(&extract_dir)?;
            Ok::<_, eyre::Report>(())
        })
        .await?;
    }

    let archive_url = latest_cmdline_tools_archive_url().await?;
    download_file_with_redirect(&archive_url, &archive_path).await?;

    {
        let archive_path = archive_path.clone();
        let extract_dir = extract_dir.clone();
        smol::unblock(move || {
            let archive_file = std::fs::File::open(&archive_path)?;
            let mut archive = zip::ZipArchive::new(archive_file)?;
            archive.extract(&extract_dir)?;
            Ok::<_, eyre::Report>(())
        })
        .await?;
    }

    let extracted_cmdline_dir = {
        let extract_dir = extract_dir.clone();
        smol::unblock(move || find_cmdline_tools_dir(&extract_dir)).await?
    };

    if latest_dir.exists() {
        let latest_dir = latest_dir.clone();
        smol::unblock(move || std::fs::remove_dir_all(latest_dir)).await?;
    }

    {
        let extracted_cmdline_dir = extracted_cmdline_dir.clone();
        let latest_dir = latest_dir.clone();
        smol::unblock(move || std::fs::rename(extracted_cmdline_dir, latest_dir)).await?;
    }

    if sdkmanager.exists() {
        let temp_dir = temp_dir.clone();
        let _ = smol::unblock(move || std::fs::remove_dir_all(temp_dir)).await;
        Ok(())
    } else {
        Err(eyre::eyre!(
            "Android command-line tools were extracted but sdkmanager is still missing"
        ))
    }
}

fn looks_like_android_sdk_root(path: &Path) -> bool {
    path.join("cmdline-tools").exists()
        || path.join("platform-tools").exists()
        || path.join("platforms").exists()
        || path.join("ndk").exists()
}

fn find_android_jar_in_sdk(sdk_root: &Path) -> Option<PathBuf> {
    let platforms_dir = sdk_root.join("platforms");
    if !platforms_dir.exists() {
        return None;
    }

    let mut platforms = std::fs::read_dir(&platforms_dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    platforms.sort_by(|left, right| {
        let left_api = left
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("android-"))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let right_api = right
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("android-"))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        right_api.cmp(&left_api)
    });

    for platform in platforms {
        let android_jar = platform.join("android.jar");
        if android_jar.exists() {
            return Some(android_jar);
        }
    }
    None
}

fn derive_sdk_root_from_sdkmanager_path(path: &Path) -> Option<PathBuf> {
    let bin_dir = path.parent()?;
    if !bin_dir
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("bin")
    {
        return None;
    }

    let parent = bin_dir.parent()?;
    if parent
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("tools")
        || parent
            .file_name()?
            .to_string_lossy()
            .eq_ignore_ascii_case("cmdline-tools")
    {
        return Some(parent.parent()?.to_path_buf());
    }

    let maybe_cmdline_tools = parent.parent()?;
    if maybe_cmdline_tools
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("cmdline-tools")
    {
        return Some(maybe_cmdline_tools.parent()?.to_path_buf());
    }

    None
}

fn find_sdkmanager_on_path_env() -> Option<PathBuf> {
    let path_env = env::var_os("PATH")?;
    for path_dir in env::split_paths(&path_env) {
        for candidate_name in sdkmanager_search_names() {
            let candidate = path_dir.join(candidate_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn parse_sdkmanager_package_id(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (first_column, _) = trimmed.split_once('|')?;
    let package_id = first_column.trim();
    if package_id.is_empty() || package_id == "Path" || package_id.starts_with('-') {
        return None;
    }
    Some(package_id)
}

fn parse_android_platform_api_level(package_id: &str) -> Option<u32> {
    package_id
        .strip_prefix("platforms;android-")?
        .parse::<u32>()
        .ok()
}

fn parse_android_build_tools_version(package_id: &str) -> Option<&str> {
    package_id.strip_prefix("build-tools;")
}

fn parse_numeric_prefix(segment: &str) -> u64 {
    let digits: String = segment
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().unwrap_or(0)
}

fn compare_version_segments(left: &[u64], right: &[u64]) -> Ordering {
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let l = left.get(idx).copied().unwrap_or(0);
        let r = right.get(idx).copied().unwrap_or(0);
        match l.cmp(&r) {
            Ordering::Equal => continue,
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

fn compare_sdk_package_ids(left: &str, right: &str) -> Ordering {
    let left_version = left
        .split_once(';')
        .map_or("", |(_, version)| version)
        .split('.')
        .map(parse_numeric_prefix)
        .collect::<Vec<_>>();
    let right_version = right
        .split_once(';')
        .map_or("", |(_, version)| version)
        .split('.')
        .map(parse_numeric_prefix)
        .collect::<Vec<_>>();

    match compare_version_segments(&left_version, &right_version) {
        Ordering::Equal => left.cmp(right),
        ordering => ordering,
    }
}

fn find_d8_jar_in_sdk(sdk_root: &Path) -> Option<PathBuf> {
    let build_tools_dir = sdk_root.join("build-tools");
    if !build_tools_dir.exists() {
        return None;
    }

    let mut build_tools_versions = std::fs::read_dir(&build_tools_dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let version = path.file_name()?.to_str()?;
            Some((format!("build-tools;{version}"), path))
        })
        .collect::<Vec<_>>();
    build_tools_versions.sort_by(|(left, _), (right, _)| compare_sdk_package_ids(left, right));

    while let Some((_, version_dir)) = build_tools_versions.pop() {
        let d8_jar = version_dir.join("lib/d8.jar");
        if d8_jar.exists() {
            return Some(d8_jar);
        }
    }

    None
}

async fn resolve_sdkmanager_and_root() -> eyre::Result<(PathBuf, PathBuf)> {
    let sdkmanager_path = AndroidSdk::sdkmanager_path()
        .await
        .ok_or_else(|| eyre::eyre!("Android SDK command-line tools (`sdkmanager`) not found"))?;
    let sdk_root = AndroidSdk::detect_path()
        .or_else(|| derive_sdk_root_from_sdkmanager_path(&sdkmanager_path))
        .ok_or_else(|| {
            eyre::eyre!(
                "Android SDK root could not be determined from environment or sdkmanager path"
            )
        })?;
    Ok((sdkmanager_path, sdk_root))
}

fn prepend_path_entry(entry: &Path, existing: Option<OsString>) -> eyre::Result<OsString> {
    let mut entries = vec![entry.to_path_buf()];
    if let Some(existing) = existing {
        entries.extend(env::split_paths(&existing));
    }
    env::join_paths(entries).map_err(|error| {
        eyre::eyre!(
            "Failed to construct PATH with required entry '{}': {error}",
            entry.display()
        )
    })
}

fn sdkmanager_combined_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("stdout: {} stderr: {}", stdout.trim(), stderr.trim())
}

fn sdkmanager_requires_license_acceptance(output: &Output) -> bool {
    let lower = sdkmanager_combined_output(output).to_ascii_lowercase();
    lower.contains("license is not accepted")
        || lower.contains("licenses or those of the packages they depend on were not accepted")
        || lower.contains("accept? (y/n):")
}

async fn run_sdkmanager_output_with_java(
    args: Vec<OsString>,
    stdin_payload: Option<&str>,
) -> eyre::Result<Output> {
    let (sdkmanager_path, sdk_root) = resolve_sdkmanager_and_root().await?;
    let java_home = Java::detect_home()
        .await
        .ok_or_else(|| eyre::eyre!("Java runtime not found while invoking sdkmanager"))?;
    let java_bin = java_home.join("bin");
    let path_env = prepend_path_entry(&java_bin, env::var_os("PATH"))?;

    let mut sdk_root_arg = OsString::from("--sdk_root=");
    sdk_root_arg.push(&sdk_root);
    let mut full_args = vec![sdk_root_arg];
    full_args.extend(args);

    let mut cmd = smol::process::Command::new(&sdkmanager_path);
    cmd.args(full_args)
        .env("ANDROID_SDK_ROOT", &sdk_root)
        .env("ANDROID_HOME", &sdk_root)
        .env("JAVA_HOME", &java_home)
        .env("PATH", path_env);

    if let Some(stdin_payload) = stdin_payload {
        use smol::io::AsyncWriteExt;
        use std::process::Stdio;

        cmd.stdin(Stdio::piped());
        let mut child = command(&mut cmd).spawn().map_err(eyre::Report::from)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(stdin_payload.as_bytes())
                .await
                .map_err(eyre::Report::from)?;
            stdin.flush().await.map_err(eyre::Report::from)?;
        }
        child.output().await.map_err(eyre::Report::from)
    } else {
        command(&mut cmd).output().await.map_err(eyre::Report::from)
    }
}

async fn accept_sdkmanager_licenses() -> eyre::Result<()> {
    let license_input = "y\n".repeat(128);
    let output =
        run_sdkmanager_output_with_java(vec![OsString::from("--licenses")], Some(&license_input))
            .await?;
    if output.status.success() {
        return Ok(());
    }
    Err(eyre::eyre!(
        "Failed to accept Android SDK licenses. {}",
        sdkmanager_combined_output(&output)
    ))
}

async fn install_android_sdk_package(package_id: &str) -> eyre::Result<()> {
    let install_args = vec![OsString::from("--install"), OsString::from(package_id)];
    let mut output = run_sdkmanager_output_with_java(install_args.clone(), None).await?;
    if sdkmanager_requires_license_acceptance(&output) {
        accept_sdkmanager_licenses().await?;
        output = run_sdkmanager_output_with_java(install_args, None).await?;
    }
    if output.status.success() {
        return Ok(());
    }

    Err(eyre::eyre!(
        "Failed to install package `{package_id}` via sdkmanager. {}",
        sdkmanager_combined_output(&output)
    ))
}

async fn list_sdk_package_ids() -> eyre::Result<Vec<String>> {
    let output = run_sdkmanager_output_with_java(vec![OsString::from("--list")], None).await?;
    if !output.status.success() {
        return Err(eyre::eyre!(
            "Failed to list Android SDK packages via sdkmanager. {}",
            sdkmanager_combined_output(&output)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(parse_sdkmanager_package_id)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>())
}

async fn latest_ndk_package_id() -> eyre::Result<String> {
    let mut ndk_packages = list_sdk_package_ids()
        .await?
        .into_iter()
        .filter(|package| package.starts_with("ndk;"))
        .collect::<Vec<_>>();
    ndk_packages.sort_by(|left, right| compare_sdk_package_ids(left, right));
    ndk_packages.dedup();

    ndk_packages.pop().ok_or_else(|| {
        eyre::eyre!("No installable Android NDK package found via `sdkmanager --list`")
    })
}

async fn latest_android_platform_package_id() -> eyre::Result<String> {
    list_sdk_package_ids()
        .await?
        .into_iter()
        .filter_map(|package_id| {
            parse_android_platform_api_level(&package_id).map(|api_level| (api_level, package_id))
        })
        .max_by_key(|(api_level, _)| *api_level)
        .map(|(_, package_id)| package_id)
        .ok_or_else(|| {
            eyre::eyre!("No installable Android platform package found via `sdkmanager --list`")
        })
}

async fn latest_android_build_tools_package_id() -> eyre::Result<String> {
    let mut build_tools_packages = list_sdk_package_ids()
        .await?
        .into_iter()
        .filter(|package_id| parse_android_build_tools_version(package_id).is_some())
        .collect::<Vec<_>>();
    build_tools_packages.sort_by(|left, right| compare_sdk_package_ids(left, right));
    build_tools_packages.dedup();

    build_tools_packages.pop().ok_or_else(|| {
        eyre::eyre!("No installable Android build-tools package found via `sdkmanager --list`")
    })
}

const fn rust_target_for_android_abi(abi: AndroidAbi) -> &'static str {
    match abi {
        AndroidAbi::Arm64V8a => "aarch64-linux-android",
        AndroidAbi::X86_64 => "x86_64-linux-android",
        AndroidAbi::ArmeabiV7a => "armv7-linux-androideabi",
        AndroidAbi::X86 => "i686-linux-android",
    }
}

fn required_android_rust_targets() -> Vec<&'static str> {
    let mut targets = ALL_ABIS
        .iter()
        .map(|abi| rust_target_for_android_abi(*abi))
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();
    targets
}

async fn installed_rustup_targets() -> eyre::Result<Vec<String>> {
    let installed = run_command("rustup", ["target", "list", "--installed"]).await?;
    Ok(installed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn missing_android_rust_targets(installed_targets: &[String]) -> Vec<String> {
    required_android_rust_targets()
        .into_iter()
        .filter(|target| {
            !installed_targets
                .iter()
                .any(|installed| installed == target)
        })
        .map(ToOwned::to_owned)
        .collect()
}

impl AndroidSdk {
    /// Detect the path to the Android SDK installation.
    #[must_use]
    pub fn detect_path() -> Option<PathBuf> {
        if let Some(configured) = configured_android_sdk_path()
            && configured.exists()
            && looks_like_android_sdk_root(&configured)
        {
            return Some(configured);
        }

        if let Some(sdkmanager_path) = find_sdkmanager_on_path_env()
            && let Some(sdk_root) = derive_sdk_root_from_sdkmanager_path(&sdkmanager_path)
            && sdk_root.exists()
            && looks_like_android_sdk_root(&sdk_root)
        {
            return Some(sdk_root);
        }

        None
    }

    /// Detect the highest available `android.jar` from installed SDK platforms.
    #[must_use]
    pub fn android_jar_path() -> Option<PathBuf> {
        let sdk_root = Self::detect_path()?;
        find_android_jar_in_sdk(&sdk_root)
    }

    /// Detect the highest available `d8.jar` from installed SDK build-tools.
    #[must_use]
    pub fn d8_jar_path() -> Option<PathBuf> {
        let sdk_root = Self::detect_path()?;
        find_d8_jar_in_sdk(&sdk_root)
    }

    /// Detect sdkmanager executable path.
    pub async fn sdkmanager_path() -> Option<PathBuf> {
        if let Some(sdk_root) = Self::detect_path() {
            for candidate in sdkmanager_candidates_under_sdk_root(&sdk_root) {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }

        for name in sdkmanager_search_names() {
            if let Ok(path) = which(name).await {
                return Some(path);
            }
        }

        find_sdkmanager_on_path_env()
    }

    /// Get the path to the `adb` executable.
    #[must_use]
    pub fn adb_path() -> Option<PathBuf> {
        let sdk_path = Self::detect_path()?;
        let adb = sdk_path
            .join("platform-tools")
            .join(if cfg!(target_os = "windows") {
                "adb.exe"
            } else {
                "adb"
            });
        if adb.exists() { Some(adb) } else { None }
    }

    /// Get the path to the `emulator` executable.
    #[must_use]
    pub fn emulator_path() -> Option<PathBuf> {
        let sdk_path = Self::detect_path()?;
        let emulator = sdk_path
            .join("emulator")
            .join(if cfg!(target_os = "windows") {
                "emulator.exe"
            } else {
                "emulator"
            });
        if emulator.exists() {
            Some(emulator)
        } else {
            None
        }
    }
}

/// Installation procedure for the Android SDK.
#[derive(Debug, Clone, Default)]
pub struct AndroidSdkInstallation;

/// Errors that can occur when installing the Android SDK.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidSdk {
    #[error("Homebrew not found. Install Homebrew first, then retry `water doctor --fix`.")]
    BrewNotFound,
    #[error(
        "winget is required for automatic Android Studio installation on Windows. Install App Installer and retry."
    )]
    WingetNotFound,
    #[error("Failed to install Android Studio via winget: {0}")]
    WingetInstallFailed(String),
    #[error("Failed to install Android SDK prerequisites: {0}")]
    InstallFailed(eyre::Report),
    #[error(
        "Android SDK setup completed, but SDK root is still not detectable. Install Android command-line tools and set `ANDROID_SDK_ROOT`."
    )]
    PostInstallSetupRequired,
    #[error(
        "Automatic Android SDK installation is only supported on macOS and Windows. On Linux, install Android SDK command-line tools manually."
    )]
    UnsupportedPlatform,
}

impl Toolchain for AndroidSdk {
    type Installation = AndroidSdkInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if Self::detect_path().is_some() {
            Ok(())
        } else if cfg!(target_os = "windows") {
            if which("winget").await.is_ok() {
                Err(ToolchainError::fixable(AndroidSdkInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Android SDK not found and winget is unavailable",
                    format!(
                        "Install Microsoft App Installer to provide winget, then retry `water doctor --fix`. {} {}",
                        android_cmdline_tools_suggestion(),
                        android_sdk_path_suggestion()
                    ),
                ))
            }
        } else if cfg!(target_os = "macos") {
            if which("brew").await.is_ok() {
                Err(ToolchainError::fixable(AndroidSdkInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Android SDK not found and Homebrew is unavailable",
                    format!(
                        "Install Homebrew to enable automatic fixes, or install Android SDK manually. {} {}",
                        android_cmdline_tools_suggestion(),
                        android_sdk_path_suggestion()
                    ),
                ))
            }
        } else {
            Err(ToolchainError::unfixable(
                "Android SDK not found",
                format!(
                    "{} {}",
                    android_cmdline_tools_suggestion(),
                    android_sdk_path_suggestion()
                ),
            ))
        }
    }
}

impl Installation for AndroidSdkInstallation {
    type Error = FailToInstallAndroidSdk;

    async fn install(&self) -> Result<(), Self::Error> {
        if cfg!(target_os = "windows") {
            ensure_package_installed("Google.AndroidStudio")
                .await
                .map_err(map_winget_error_for_android_sdk)?;
        } else if cfg!(target_os = "macos") {
            let brew = Brew::default();
            brew.check()
                .await
                .map_err(|_| FailToInstallAndroidSdk::BrewNotFound)?;
            brew.install_cask("android-studio")
                .await
                .map_err(FailToInstallAndroidSdk::InstallFailed)?;
        } else {
            return Err(FailToInstallAndroidSdk::UnsupportedPlatform);
        }

        let sdk_root = configured_android_sdk_path()
            .ok_or_else(|| eyre::eyre!("Android SDK root cannot be determined on this host"))
            .map_err(FailToInstallAndroidSdk::InstallFailed)?;
        {
            let sdk_root = sdk_root.clone();
            smol::unblock(move || std::fs::create_dir_all(&sdk_root))
                .await
                .map_err(eyre::Report::from)
                .map_err(FailToInstallAndroidSdk::InstallFailed)?;
        }
        ensure_cmdline_tools_available(&sdk_root)
            .await
            .map_err(FailToInstallAndroidSdk::InstallFailed)?;

        if AndroidSdk::detect_path().is_some() {
            Ok(())
        } else {
            Err(FailToInstallAndroidSdk::PostInstallSetupRequired)
        }
    }
}

fn map_winget_error_for_android_sdk(error: WingetInstallError) -> FailToInstallAndroidSdk {
    match error {
        WingetInstallError::WingetNotFound => FailToInstallAndroidSdk::WingetNotFound,
        WingetInstallError::CommandFailed(err) => {
            FailToInstallAndroidSdk::WingetInstallFailed(err.to_string())
        }
        WingetInstallError::NotInstalled { package_id } => {
            FailToInstallAndroidSdk::WingetInstallFailed(format!(
                "Package `{package_id}` is still missing after winget install; verify winget sources and retry."
            ))
        }
    }
}

/// Installation procedure for Android Platform-Tools.
#[derive(Debug, Clone, Default)]
pub struct AndroidPlatformToolsInstallation;

/// Errors that can occur when installing Android Platform-Tools.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidPlatformTools {
    #[error("Android SDK command-line tools (`sdkmanager`) not found.")]
    SdkManagerNotFound,
    #[error("Failed to install Android Platform-Tools via sdkmanager: {0}")]
    InstallFailed(eyre::Report),
    #[error("Android Platform-Tools (`adb`) is still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidPlatformTools {
    type Installation = AndroidPlatformToolsInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if AndroidSdk::adb_path().is_some() {
            return Ok(());
        }

        if AndroidSdk::sdkmanager_path().await.is_some() {
            Err(ToolchainError::fixable(AndroidPlatformToolsInstallation))
        } else {
            Err(ToolchainError::unfixable(
                "Android Platform-Tools (`adb`) not found",
                format!(
                    "{} {}",
                    android_platform_tools_suggestion(),
                    android_cmdline_tools_suggestion()
                ),
            ))
        }
    }
}

impl Installation for AndroidPlatformToolsInstallation {
    type Error = FailToInstallAndroidPlatformTools;

    async fn install(&self) -> Result<(), Self::Error> {
        if AndroidSdk::sdkmanager_path().await.is_none() {
            return Err(FailToInstallAndroidPlatformTools::SdkManagerNotFound);
        }

        install_android_sdk_package("platform-tools")
            .await
            .map_err(FailToInstallAndroidPlatformTools::InstallFailed)?;

        if AndroidSdk::adb_path().is_some() {
            Ok(())
        } else {
            Err(FailToInstallAndroidPlatformTools::StillMissing)
        }
    }
}

/// Installation procedure for Android SDK platform packages.
#[derive(Debug, Clone, Default)]
pub struct AndroidSdkPlatformsInstallation;

/// Errors that can occur when installing Android SDK platform packages.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidSdkPlatforms {
    #[error("Android SDK command-line tools (`sdkmanager`) not found.")]
    SdkManagerNotFound,
    #[error("Failed to install Android SDK platform package via sdkmanager: {0}")]
    InstallFailed(eyre::Report),
    #[error("Android SDK platforms are still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidSdkPlatforms {
    type Installation = AndroidSdkPlatformsInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if smol::unblock(AndroidSdk::android_jar_path).await.is_some() {
            return Ok(());
        }

        if AndroidSdk::sdkmanager_path().await.is_some() {
            Err(ToolchainError::fixable(AndroidSdkPlatformsInstallation))
        } else {
            Err(ToolchainError::unfixable(
                "Android SDK platforms are missing",
                format!(
                    "{} {}",
                    android_platforms_install_suggestion(),
                    android_cmdline_tools_suggestion()
                ),
            ))
        }
    }
}

impl Installation for AndroidSdkPlatformsInstallation {
    type Error = FailToInstallAndroidSdkPlatforms;

    async fn install(&self) -> Result<(), Self::Error> {
        if AndroidSdk::sdkmanager_path().await.is_none() {
            return Err(FailToInstallAndroidSdkPlatforms::SdkManagerNotFound);
        }

        let platform_package = latest_android_platform_package_id()
            .await
            .map_err(FailToInstallAndroidSdkPlatforms::InstallFailed)?;
        install_android_sdk_package(&platform_package)
            .await
            .map_err(FailToInstallAndroidSdkPlatforms::InstallFailed)?;

        if smol::unblock(AndroidSdk::android_jar_path).await.is_some() {
            Ok(())
        } else {
            Err(FailToInstallAndroidSdkPlatforms::StillMissing)
        }
    }
}

/// Installation procedure for Android SDK build-tools packages.
#[derive(Debug, Clone, Default)]
pub struct AndroidBuildToolsInstallation;

/// Errors that can occur when installing Android SDK build-tools packages.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidBuildTools {
    #[error("Android SDK command-line tools (`sdkmanager`) not found.")]
    SdkManagerNotFound,
    #[error("Failed to install Android SDK build-tools package via sdkmanager: {0}")]
    InstallFailed(eyre::Report),
    #[error("Android SDK build-tools are still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidBuildTools {
    type Installation = AndroidBuildToolsInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if smol::unblock(AndroidSdk::d8_jar_path).await.is_some() {
            return Ok(());
        }

        if AndroidSdk::sdkmanager_path().await.is_some() {
            Err(ToolchainError::fixable(AndroidBuildToolsInstallation))
        } else {
            Err(ToolchainError::unfixable(
                "Android SDK build-tools are missing",
                format!(
                    "{} {}",
                    android_build_tools_install_suggestion(),
                    android_cmdline_tools_suggestion()
                ),
            ))
        }
    }
}

impl Installation for AndroidBuildToolsInstallation {
    type Error = FailToInstallAndroidBuildTools;

    async fn install(&self) -> Result<(), Self::Error> {
        if AndroidSdk::sdkmanager_path().await.is_none() {
            return Err(FailToInstallAndroidBuildTools::SdkManagerNotFound);
        }

        let build_tools_package = latest_android_build_tools_package_id()
            .await
            .map_err(FailToInstallAndroidBuildTools::InstallFailed)?;
        install_android_sdk_package(&build_tools_package)
            .await
            .map_err(FailToInstallAndroidBuildTools::InstallFailed)?;

        if smol::unblock(AndroidSdk::d8_jar_path).await.is_some() {
            Ok(())
        } else {
            Err(FailToInstallAndroidBuildTools::StillMissing)
        }
    }
}

/// Installation procedure for Rust Android targets.
#[derive(Debug, Clone)]
pub struct AndroidRustTargetsInstallation {
    missing_targets: Vec<String>,
}

impl AndroidRustTargetsInstallation {
    fn new(missing_targets: Vec<String>) -> Self {
        assert!(
            !missing_targets.is_empty(),
            "AndroidRustTargetsInstallation requires at least one missing target"
        );
        Self { missing_targets }
    }
}

/// Errors that can occur when installing Rust Android targets.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidRustTargets {
    #[error("rustup is required to install Android Rust targets but was not found in PATH.")]
    RustupNotFound,
    #[error("Failed to install Rust Android target `{target}`: {source}")]
    AddTarget {
        /// Target triple that failed to install.
        target: String,
        /// Underlying command error.
        source: eyre::Report,
    },
    #[error("Failed to list installed Rust targets after installation: {0}")]
    QueryTargets(eyre::Report),
    #[error("Android Rust targets are still missing after installation: {missing_targets}")]
    StillMissing {
        /// Comma-separated missing targets.
        missing_targets: String,
    },
}

impl Toolchain for AndroidRustTargets {
    type Installation = AndroidRustTargetsInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if which("rustup").await.is_err() {
            return Err(ToolchainError::unfixable(
                "rustup is not available, so Android Rust targets cannot be managed automatically",
                "Install rustup from https://rustup.rs, then run `water doctor --fix`.",
            ));
        }

        let installed_targets = installed_rustup_targets().await.map_err(|error| {
            ToolchainError::unfixable(
                format!("Failed to query installed Rust targets: {error}"),
                "Run `rustup target list --installed`; if it fails, repair rustup with `rustup self update` or reinstall rustup.",
            )
        })?;

        let missing_targets = missing_android_rust_targets(&installed_targets);
        if missing_targets.is_empty() {
            Ok(())
        } else {
            Err(ToolchainError::fixable(
                AndroidRustTargetsInstallation::new(missing_targets),
            ))
        }
    }
}

impl Installation for AndroidRustTargetsInstallation {
    type Error = FailToInstallAndroidRustTargets;

    async fn install(&self) -> Result<(), Self::Error> {
        if which("rustup").await.is_err() {
            return Err(FailToInstallAndroidRustTargets::RustupNotFound);
        }

        for target in &self.missing_targets {
            run_command("rustup", ["target", "add", target.as_str()])
                .await
                .map_err(|source| FailToInstallAndroidRustTargets::AddTarget {
                    target: target.clone(),
                    source,
                })?;
        }

        let installed_targets = installed_rustup_targets()
            .await
            .map_err(FailToInstallAndroidRustTargets::QueryTargets)?;
        let still_missing = missing_android_rust_targets(&installed_targets);
        if still_missing.is_empty() {
            Ok(())
        } else {
            Err(FailToInstallAndroidRustTargets::StillMissing {
                missing_targets: still_missing.join(", "),
            })
        }
    }
}

fn windows_jdk_candidates_from_root(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut candidates = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
            if !name.starts_with("jdk") {
                return None;
            }
            let java_path = path.join("bin/java.exe");
            if java_path.exists() {
                Some(java_path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
}

fn detect_windows_jdk_java_path() -> Option<PathBuf> {
    let program_files = env::var("ProgramFiles").ok()?;
    let roots = [
        PathBuf::from(&program_files).join("Microsoft"),
        PathBuf::from(&program_files).join("Eclipse Adoptium"),
        PathBuf::from(&program_files).join("Java"),
    ];

    let mut matches = roots
        .iter()
        .flat_map(|root| windows_jdk_candidates_from_root(root))
        .collect::<Vec<_>>();
    matches.sort();
    matches.pop()
}

impl Java {
    /// Detect the path to the Java installation for Android development.
    ///
    /// Priority order:
    /// 1. Android Studio's bundled JBR (guaranteed compatible with AGP)
    /// 2. JAVA_HOME environment variable (may be incompatible)
    /// 3. Java from PATH (fallback)
    pub async fn detect_path() -> Option<PathBuf> {
        if cfg!(target_os = "macos") {
            const ANDROID_STUDIO_JBRS: &[&str] = &[
                "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java",
                "/Applications/Android Studio Preview.app/Contents/jbr/Contents/Home/bin/java",
            ];
            for path in ANDROID_STUDIO_JBRS {
                let java_path = PathBuf::from(path);
                if java_path.exists() {
                    return Some(java_path);
                }
            }
        }

        if cfg!(target_os = "linux") {
            if let Ok(home) = env::var("HOME") {
                let paths = [
                    format!(
                        "{home}/.local/share/JetBrains/Toolbox/apps/android-studio/jbr/bin/java"
                    ),
                    format!("{home}/android-studio/jbr/bin/java"),
                ];
                for path in paths {
                    let java_path = PathBuf::from(&path);
                    if java_path.exists() {
                        return Some(java_path);
                    }
                }
            }
        }

        if cfg!(target_os = "windows") {
            if let Ok(program_files) = env::var("ProgramFiles") {
                let java_path =
                    PathBuf::from(&program_files).join("Android/Android Studio/jbr/bin/java.exe");
                if java_path.exists() {
                    return Some(java_path);
                }
            }

            if let Some(java_path) = detect_windows_jdk_java_path() {
                return Some(java_path);
            }
        }

        if let Ok(home) = env::var("JAVA_HOME") {
            let java_path = PathBuf::from(home)
                .join("bin")
                .join(if cfg!(target_os = "windows") {
                    "java.exe"
                } else {
                    "java"
                });
            if java_path.exists() {
                return Some(java_path);
            }
        }

        which("java").await.ok()
    }

    /// Get the JAVA_HOME directory (parent of bin/).
    pub async fn detect_home() -> Option<PathBuf> {
        let java_path = Self::detect_path().await?;
        java_path.parent()?.parent().map(PathBuf::from)
    }
}

/// Java installation handler.
#[derive(Debug, Clone, Default)]
pub struct JavaInstallation;

/// Errors that can occur when installing Java.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallJava {
    #[error("Homebrew not found. Install Homebrew first, then retry `water doctor --fix`.")]
    BrewNotFound,
    #[error(
        "winget is required for automatic Java installation on Windows. Install App Installer and retry."
    )]
    WingetNotFound,
    #[error("Failed to install Java via winget: {0}")]
    WingetInstallFailed(String),
    #[error(
        "No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk). Install Java manually."
    )]
    UnsupportedPackageManager,
    #[error("Failed to install Java: {0}")]
    InstallFailed(eyre::Report),
    #[error(
        "Automatic Java installation is not supported on this host. Install a JDK manually and set `JAVA_HOME`."
    )]
    UnsupportedPlatform,
}

impl Toolchain for Java {
    type Installation = JavaInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if Self::detect_path().await.is_some() {
            Ok(())
        } else if cfg!(target_os = "windows") {
            if which("winget").await.is_ok() {
                Err(ToolchainError::fixable(JavaInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Java runtime not found and winget is unavailable",
                    "Install Microsoft App Installer to provide winget, or install a JDK manually and set `JAVA_HOME`.",
                ))
            }
        } else if cfg!(target_os = "macos") {
            if which("brew").await.is_ok() {
                Err(ToolchainError::fixable(JavaInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Java runtime not found and Homebrew is unavailable",
                    "Install Homebrew to enable automatic fixes, or install a JDK manually and set `JAVA_HOME`.",
                ))
            }
        } else if cfg!(target_os = "linux") {
            if has_supported_package_manager().await {
                Err(ToolchainError::fixable(JavaInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Java runtime not found and no supported package manager was detected",
                    "Install a JDK manually and set `JAVA_HOME`, then retry.",
                ))
            }
        } else {
            Err(ToolchainError::unfixable(
                "Java runtime not found",
                "Install a JDK manually and set `JAVA_HOME`, then retry.",
            ))
        }
    }
}

impl Installation for JavaInstallation {
    type Error = FailToInstallJava;

    async fn install(&self) -> Result<(), Self::Error> {
        if cfg!(target_os = "windows") {
            ensure_package_installed("Microsoft.OpenJDK.21")
                .await
                .map_err(map_winget_error_for_java)
        } else if cfg!(target_os = "macos") {
            let brew = Brew::default();
            brew.check()
                .await
                .map_err(|_| FailToInstallJava::BrewNotFound)?;
            brew.install_cask("temurin")
                .await
                .map_err(FailToInstallJava::InstallFailed)
        } else if cfg!(target_os = "linux") {
            install_java_jdk().await.map_err(map_linux_error_for_java)
        } else {
            Err(FailToInstallJava::UnsupportedPlatform)
        }
    }
}

fn map_linux_error_for_java(error: eyre::Report) -> FailToInstallJava {
    let message = error.to_string();
    if message.contains("No supported Linux package manager found") {
        FailToInstallJava::UnsupportedPackageManager
    } else {
        FailToInstallJava::InstallFailed(error)
    }
}

fn map_winget_error_for_java(error: WingetInstallError) -> FailToInstallJava {
    match error {
        WingetInstallError::WingetNotFound => FailToInstallJava::WingetNotFound,
        WingetInstallError::CommandFailed(err) => {
            FailToInstallJava::WingetInstallFailed(err.to_string())
        }
        WingetInstallError::NotInstalled { package_id } => {
            FailToInstallJava::WingetInstallFailed(format!(
                "Package `{package_id}` is still missing after winget install; verify winget sources and retry."
            ))
        }
    }
}

impl Kotlin {
    /// Detect the path to the kotlinc compiler.
    pub async fn detect_path() -> Option<PathBuf> {
        if let Ok(home) = env::var("KOTLIN_HOME") {
            let kotlinc_path = PathBuf::from(&home).join("bin/kotlinc");
            if kotlinc_path.exists() {
                return Some(kotlinc_path);
            }
        }

        if let Ok(path) = which("kotlinc").await {
            return Some(path);
        }

        if cfg!(target_os = "macos") {
            const ANDROID_STUDIO_KOTLINS: &[&str] = &[
                "/Applications/Android Studio.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
                "/Applications/Android Studio Preview.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
            ];
            for path in ANDROID_STUDIO_KOTLINS {
                let kotlinc_path = PathBuf::from(path);
                if kotlinc_path.exists() {
                    return Some(kotlinc_path);
                }
            }
        }

        if cfg!(target_os = "linux") {
            if let Ok(home) = env::var("HOME") {
                let paths = [
                    format!(
                        "{home}/.local/share/JetBrains/Toolbox/apps/android-studio/plugins/Kotlin/kotlinc/bin/kotlinc"
                    ),
                    format!("{home}/android-studio/plugins/Kotlin/kotlinc/bin/kotlinc"),
                ];
                for path in paths {
                    let kotlinc_path = PathBuf::from(&path);
                    if kotlinc_path.exists() {
                        return Some(kotlinc_path);
                    }
                }
            }
        }

        if cfg!(target_os = "windows") {
            if let Ok(program_files) = env::var("ProgramFiles") {
                let kotlinc_path = PathBuf::from(&program_files)
                    .join("Android/Android Studio/plugins/Kotlin/kotlinc/bin/kotlinc.bat");
                if kotlinc_path.exists() {
                    return Some(kotlinc_path);
                }
            }
        }

        None
    }

    /// Get the KOTLIN_HOME directory (parent of bin/).
    pub async fn detect_home() -> Option<PathBuf> {
        let kotlinc_path = Self::detect_path().await?;
        kotlinc_path.parent()?.parent().map(PathBuf::from)
    }
}

/// Kotlin installation handler.
#[derive(Debug)]
pub struct KotlinInstallation;

/// Errors that can occur when installing Kotlin.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallKotlin {}

impl Toolchain for Kotlin {
    type Installation = KotlinInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        let kotlinc_path = Self::detect_path().await.ok_or_else(|| {
            ToolchainError::unfixable(
                "Kotlin compiler (kotlinc) not found",
                kotlin_install_suggestion(),
            )
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = smol::unblock({
                let kotlinc_path = kotlinc_path.clone();
                move || std::fs::metadata(&kotlinc_path)
            })
            .await
            {
                let permissions = metadata.permissions();
                if permissions.mode() & 0o111 == 0 {
                    return Err(ToolchainError::unfixable(
                        "Kotlin compiler (kotlinc) is not executable",
                        format!(
                            "The kotlinc script at '{}' does not have execute permission. Fix it with: sudo chmod +x '{}'",
                            kotlinc_path.display(),
                            kotlinc_path.display()
                        ),
                    ));
                }
            }
        }

        Ok(())
    }
}

impl Installation for KotlinInstallation {
    type Error = FailToInstallKotlin;

    async fn install(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl AndroidNdk {
    /// Detect the Android NDK path from environment variables or standard locations.
    #[must_use]
    pub fn detect_path() -> Option<PathBuf> {
        if let Ok(ndk_root) = env::var("ANDROID_NDK_ROOT") {
            let ndk_path = PathBuf::from(ndk_root);
            if ndk_path.exists() {
                return Some(ndk_path);
            }
        }

        if let Ok(ndk_home) = env::var("ANDROID_NDK_HOME") {
            let ndk_path = PathBuf::from(ndk_home);
            if ndk_path.exists() {
                return Some(ndk_path);
            }
        }

        let sdk_path = AndroidSdk::detect_path()?;
        let ndk_dir = sdk_path.join("ndk");
        if ndk_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&ndk_dir) {
                let mut versions: Vec<PathBuf> = entries
                    .filter_map(std::result::Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .collect();
                versions.sort();
                if let Some(latest) = versions.last() {
                    return Some(latest.clone());
                }
            }
        }

        None
    }
}

/// Android NDK installation handler.
#[derive(Debug, Clone, Default)]
pub struct AndroidNdkInstallation;

/// Errors that can occur when installing the Android NDK.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidNdk {
    #[error("Android SDK command-line tools (`sdkmanager`) not found.")]
    SdkManagerNotFound,
    #[error("Failed to install Android NDK via sdkmanager: {0}")]
    InstallFailed(eyre::Report),
    #[error("Android NDK is still missing after installation.")]
    StillMissing,
    #[error("Android NDK is installed but incomplete (`toolchains/llvm/prebuilt` is missing).")]
    Incomplete,
}

impl Toolchain for AndroidNdk {
    type Installation = AndroidNdkInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if let Some(ndk_path) = Self::detect_path() {
            let llvm_dir = ndk_path.join("toolchains/llvm/prebuilt");
            if llvm_dir.exists() {
                return Ok(());
            }

            if AndroidSdk::sdkmanager_path().await.is_some() {
                return Err(ToolchainError::fixable(AndroidNdkInstallation));
            }

            return Err(ToolchainError::unfixable(
                "Android NDK is installed but incomplete",
                android_ndk_install_suggestion(),
            ));
        }

        if AndroidSdk::sdkmanager_path().await.is_some() {
            Err(ToolchainError::fixable(AndroidNdkInstallation))
        } else {
            Err(ToolchainError::unfixable(
                "Android NDK not found",
                format!(
                    "{} {}",
                    android_ndk_install_suggestion(),
                    android_cmdline_tools_suggestion()
                ),
            ))
        }
    }
}

impl Installation for AndroidNdkInstallation {
    type Error = FailToInstallAndroidNdk;

    async fn install(&self) -> Result<(), Self::Error> {
        if AndroidSdk::sdkmanager_path().await.is_none() {
            return Err(FailToInstallAndroidNdk::SdkManagerNotFound);
        }

        let ndk_package = latest_ndk_package_id()
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        install_android_sdk_package(&ndk_package)
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;

        let ndk_path = AndroidNdk::detect_path().ok_or(FailToInstallAndroidNdk::StillMissing)?;
        let llvm_dir = ndk_path.join("toolchains/llvm/prebuilt");
        if llvm_dir.exists() {
            Ok(())
        } else {
            Err(FailToInstallAndroidNdk::Incomplete)
        }
    }
}
