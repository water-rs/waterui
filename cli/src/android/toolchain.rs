use std::{
    cmp::Ordering,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Output,
};

use color_eyre::eyre::{self, WrapErr};
use url::Url;
use walkdir::WalkDir;
use waterui_assets::{download_remote_bytes, write_bytes_atomically};

use crate::{
    android::platform::{ALL_ABIS, AndroidAbi},
    brew::Brew,
    build_info,
    toolchain::{
        Installation, Toolchain, ToolchainError,
        linux::{has_supported_package_manager, install_java_jdk, install_named_packages},
        winget::{WingetInstallError, ensure_package_installed},
    },
    utils::{command, run_command, run_command_output_os, which},
    water_dir::water_home_dir,
};

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
#[derive(Debug, Clone)]
pub struct AndroidRustTargets {
    required_targets: Vec<String>,
}

impl AndroidRustTargets {
    /// Build the Rust-target requirement set for the requested Android ABIs.
    ///
    /// # Panics
    ///
    /// Panics when `abis` is empty. Android packaging always needs at least one ABI.
    #[must_use]
    pub fn for_abis(abis: &[AndroidAbi]) -> Self {
        assert!(
            !abis.is_empty(),
            "AndroidRustTargets::for_abis requires at least one ABI"
        );
        Self {
            required_targets: required_android_rust_targets_for_abis(abis),
        }
    }
}

impl Default for AndroidRustTargets {
    fn default() -> Self {
        Self::for_abis(ALL_ABIS)
    }
}

/// An Android NDK toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidNdk;

/// Java toolchain component for Android development.
#[derive(Debug, Clone, Default)]
pub struct Java;

/// Kotlin toolchain component for Android development.
#[derive(Debug, Clone, Default)]
pub struct Kotlin;

const ANDROID_LINUX_X86_64_HOST_TOOLS_COMPAT_PACKAGES: &[&str] =
    &["libc6:amd64", "libstdc++6:amd64", "zlib1g:amd64"];

const fn is_linux_arm_host() -> bool {
    cfg!(target_os = "linux")
        && (cfg!(target_arch = "aarch64")
            || cfg!(target_arch = "arm")
            || cfg!(target_arch = "arm64ec"))
}

fn needs_linux_x86_64_host_tools_compat(detail: &str) -> bool {
    is_linux_arm_host() && detail.contains("ld-linux-x86-64.so.2")
}

async fn install_android_linux_x86_64_host_tools_compat() -> eyre::Result<()> {
    install_named_packages(ANDROID_LINUX_X86_64_HOST_TOOLS_COMPAT_PACKAGES).await
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

const fn sdkmanager_search_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["sdkmanager.bat", "sdkmanager.exe", "sdkmanager"]
    } else {
        &["sdkmanager"]
    }
}

const fn sdkmanager_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sdkmanager.bat"
    } else {
        "sdkmanager"
    }
}

const fn cmdline_tools_host_tag() -> Option<&'static str> {
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

const ANDROID_RUNTIME_BUILD_GRADLE_RELATIVE_PATH: &str =
    "backends/android/runtime/build.gradle.kts";

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

    let mut client = FollowRedirect::new(zenwave::raw_client());
    let response = client.method(Method::GET, REPOSITORY_URL)?.await?;
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
    let bytes = download_remote_bytes(url)
        .await
        .map_err(eyre::Report::new)
        .wrap_err_with(|| format!("Failed to download {url}"))?;
    write_bytes_atomically(destination, &bytes)
        .await
        .map_err(eyre::Report::new)
        .wrap_err_with(|| {
            format!(
                "Failed to write downloaded archive to {}",
                destination.display()
            )
        })?;
    Ok(())
}

fn find_cmdline_tools_dir(root: &Path) -> eyre::Result<PathBuf> {
    let sdkmanager_name = sdkmanager_binary_name();

    for entry in WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let is_sdkmanager = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(sdkmanager_name));
        if !is_sdkmanager {
            continue;
        }

        let bin_dir = path.parent().ok_or_else(|| {
            eyre::eyre!("Invalid Android command-line tools archive layout (missing bin directory)")
        })?;
        let cmdline_tools_dir = bin_dir.parent().ok_or_else(|| {
            eyre::eyre!(
                "Invalid Android command-line tools archive layout (missing cmdline-tools root)"
            )
        })?;
        return Ok(cmdline_tools_dir.to_path_buf());
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
    let temp_dir = {
        let cmdline_tools_root = cmdline_tools_root.clone();
        smol::unblock(move || {
            std::fs::create_dir_all(&cmdline_tools_root)?;
            tempfile::Builder::new()
                .prefix(".water-cmdline-tools-")
                .tempdir_in(&cmdline_tools_root)
                .map_err(eyre::Report::from)
        })
        .await?
    };
    let extract_dir = temp_dir.path().join("extract");
    let archive_path = temp_dir.path().join("commandline-tools.zip");

    {
        let extract_dir = extract_dir.clone();
        smol::unblock(move || {
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
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().unwrap_or(0)
}

fn compare_version_segments(left: &[u64], right: &[u64]) -> Ordering {
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let l = left.get(idx).copied().unwrap_or(0);
        let r = right.get(idx).copied().unwrap_or(0);
        match l.cmp(&r) {
            Ordering::Equal => {}
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

fn sdkmanager_confirmation_input() -> String {
    "y\n".repeat(128)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SdkManagerProxyType {
    Http,
    Socks,
}

impl SdkManagerProxyType {
    const fn as_flag(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Socks => "socks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SdkManagerProxyConfig {
    proxy_type: SdkManagerProxyType,
    host: String,
    port: u16,
}

fn proxy_env_value() -> Option<String> {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .into_iter()
    .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
}

fn parse_sdkmanager_proxy_config(proxy: &str) -> eyre::Result<SdkManagerProxyConfig> {
    let trimmed = proxy.trim();
    let normalized = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let url = Url::parse(&normalized)
        .wrap_err_with(|| format!("Failed to parse proxy URL `{trimmed}` for sdkmanager"))?;
    let host = url
        .host_str()
        .ok_or_else(|| eyre::eyre!("Proxy URL `{trimmed}` is missing a host"))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| eyre::eyre!("Proxy URL `{trimmed}` is missing a port"))?;
    let proxy_type = match url.scheme() {
        "http" | "https" => SdkManagerProxyType::Http,
        "socks" | "socks5" | "socks5h" => SdkManagerProxyType::Socks,
        scheme => {
            return Err(eyre::eyre!(
                "Unsupported proxy scheme `{scheme}` for sdkmanager"
            ));
        }
    };
    Ok(SdkManagerProxyConfig {
        proxy_type,
        host,
        port,
    })
}

fn sdkmanager_proxy_args() -> eyre::Result<Vec<OsString>> {
    let Some(proxy) = proxy_env_value() else {
        return Ok(Vec::new());
    };
    let proxy = parse_sdkmanager_proxy_config(&proxy)?;
    Ok(vec![
        OsString::from(format!("--proxy={}", proxy.proxy_type.as_flag())),
        OsString::from(format!("--proxy_host={}", proxy.host)),
        OsString::from(format!("--proxy_port={}", proxy.port)),
    ])
}

pub(super) fn java_proxy_properties_from_env() -> eyre::Result<Vec<String>> {
    let Some(proxy) = proxy_env_value() else {
        return Ok(Vec::new());
    };
    let proxy = parse_sdkmanager_proxy_config(&proxy)?;
    Ok(match proxy.proxy_type {
        SdkManagerProxyType::Http => vec![
            format!("-Dhttp.proxyHost={}", proxy.host),
            format!("-Dhttp.proxyPort={}", proxy.port),
            format!("-Dhttps.proxyHost={}", proxy.host),
            format!("-Dhttps.proxyPort={}", proxy.port),
        ],
        SdkManagerProxyType::Socks => vec![
            format!("-DsocksProxyHost={}", proxy.host),
            format!("-DsocksProxyPort={}", proxy.port),
        ],
    })
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
    full_args.extend(sdkmanager_proxy_args()?);
    full_args.extend(args);

    let mut cmd = smol::process::Command::new(&sdkmanager_path);
    cmd.args(full_args)
        .env("ANDROID_SDK_ROOT", &sdk_root)
        .env("ANDROID_HOME", &sdk_root)
        .env("JAVA_HOME", &java_home)
        .env("PATH", path_env)
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy");

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
    let license_input = sdkmanager_confirmation_input();
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
    let confirmation_input = sdkmanager_confirmation_input();
    let mut output =
        run_sdkmanager_output_with_java(install_args.clone(), Some(&confirmation_input)).await?;
    if sdkmanager_requires_license_acceptance(&output) {
        accept_sdkmanager_licenses().await?;
        output = run_sdkmanager_output_with_java(install_args, Some(&confirmation_input)).await?;
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

fn find_file_in_current_workspace(relative_path: &Path) -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;

    let direct = cwd.join(relative_path);
    if direct.exists() {
        return Some(direct);
    }

    let mut current = cwd.as_path();
    loop {
        let candidate = current.join(relative_path);
        if candidate.exists() {
            return Some(candidate);
        }

        let parent = current.parent()?;
        current = parent;
    }
}

fn parse_android_ndk_version_from_runtime_build_gradle(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.split("//").next()?.trim();
        let remainder = line.strip_prefix("ndkVersion")?;
        let (_, value) = remainder.split_once('=')?;
        let version = value.trim().trim_matches('"');
        if version.is_empty() {
            None
        } else {
            Some(version.to_string())
        }
    })
}

fn required_ndk_version_in_current_workspace() -> Option<String> {
    let runtime_build_gradle =
        find_file_in_current_workspace(Path::new(ANDROID_RUNTIME_BUILD_GRADLE_RELATIVE_PATH))?;
    let contents = std::fs::read_to_string(runtime_build_gradle).ok()?;
    parse_android_ndk_version_from_runtime_build_gradle(&contents)
}

fn select_installed_ndk_path(ndk_dir: &Path, required_version: Option<&str>) -> Option<PathBuf> {
    if !ndk_dir.exists() {
        return None;
    }

    if let Some(required_version) = required_version {
        let required_path = ndk_dir.join(required_version);
        if required_path.is_dir() {
            return Some(required_path);
        }
    }

    let mut versions: Vec<PathBuf> = std::fs::read_dir(ndk_dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    versions.pop()
}

fn ndk_version_from_package_id(package_id: &str) -> eyre::Result<&str> {
    package_id
        .strip_prefix("ndk;")
        .ok_or_else(|| eyre::eyre!("Invalid Android NDK package id `{package_id}`"))
}

fn ndk_path_for_package_id(sdk_root: &Path, package_id: &str) -> eyre::Result<PathBuf> {
    Ok(sdk_root
        .join("ndk")
        .join(ndk_version_from_package_id(package_id)?))
}

fn ndk_layout_is_complete(ndk_path: &Path) -> bool {
    ndk_path.join("toolchains/llvm/prebuilt").exists()
}

async fn remove_directory_if_exists(path: &Path) -> eyre::Result<()> {
    let path = path.to_path_buf();
    smol::unblock(move || {
        if path.exists() {
            remove_dir_all::remove_dir_all(&path)?;
        }
        Ok::<(), eyre::Report>(())
    })
    .await?;
    Ok(())
}

const fn kotlinc_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "kotlinc.bat"
    } else {
        "kotlinc"
    }
}

fn kotlin_executable_from_home(home: &Path) -> Option<PathBuf> {
    let executable = home.join("bin").join(kotlinc_binary_name());
    executable.exists().then_some(executable)
}

fn managed_kotlin_home(version: &str) -> eyre::Result<PathBuf> {
    Ok(water_home_dir()?.join("toolchains/kotlin").join(version))
}

fn kotlin_compiler_release_url(version: &str) -> String {
    format!(
        "https://github.com/JetBrains/kotlin/releases/download/v{version}/kotlin-compiler-{version}.zip"
    )
}

fn find_kotlin_home_dir(root: &Path) -> eyre::Result<PathBuf> {
    let executable_name = kotlinc_binary_name();
    for entry in WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let is_kotlinc = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(executable_name));
        if !is_kotlinc {
            continue;
        }

        let bin_dir = path.parent().ok_or_else(|| {
            eyre::eyre!("Invalid Kotlin compiler archive layout (missing bin directory)")
        })?;
        let kotlin_home = bin_dir.parent().ok_or_else(|| {
            eyre::eyre!("Invalid Kotlin compiler archive layout (missing compiler root)")
        })?;
        return Ok(kotlin_home.to_path_buf());
    }

    Err(eyre::eyre!(
        "Kotlin compiler archive does not contain {}",
        executable_name
    ))
}

fn parse_kotlinc_version_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut tokens = line
            .split_whitespace()
            .map(|token| token.trim_matches(|ch: char| ch == ':' || ch == '(' || ch == ')'));
        while let Some(token) = tokens.next() {
            if token.starts_with("kotlinc") {
                return tokens
                    .find(|candidate| {
                        candidate
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_digit())
                    })
                    .map(ToOwned::to_owned);
            }
        }
        None
    })
}

fn kotlin_version_is_compatible(installed: &str, required: &str) -> bool {
    let installed_segments = installed
        .split('.')
        .map(parse_numeric_prefix)
        .collect::<Vec<_>>();
    let required_segments = required
        .split('.')
        .map(parse_numeric_prefix)
        .collect::<Vec<_>>();
    compare_version_segments(&installed_segments, &required_segments) != Ordering::Less
}

async fn kotlin_compiler_version(kotlinc_path: &Path) -> eyre::Result<String> {
    let output = run_command_output_os(kotlinc_path, ["-version"]).await?;
    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_kotlinc_version_output(&combined).ok_or_else(|| {
        eyre::eyre!(
            "Failed to parse Kotlin compiler version from `{}` output: {}",
            kotlinc_path.display(),
            combined.trim()
        )
    })
}

async fn install_managed_kotlin_compiler(version: &str) -> eyre::Result<PathBuf> {
    let install_home = managed_kotlin_home(version)?;
    if let Some(kotlinc_path) = kotlin_executable_from_home(&install_home)
        && let Ok(installed_version) = kotlin_compiler_version(&kotlinc_path).await
        && kotlin_version_is_compatible(&installed_version, version)
    {
        return Ok(kotlinc_path);
    }

    let install_parent = install_home
        .parent()
        .ok_or_else(|| eyre::eyre!("Managed Kotlin install path has no parent"))?
        .to_path_buf();
    {
        let install_parent = install_parent.clone();
        smol::unblock(move || std::fs::create_dir_all(&install_parent))
            .await
            .map_err(eyre::Report::from)?;
    }

    let temp_dir = {
        let install_parent = install_parent.clone();
        smol::unblock(move || {
            tempfile::Builder::new()
                .prefix(".water-kotlin-")
                .tempdir_in(&install_parent)
                .map_err(eyre::Report::from)
        })
        .await?
    };
    let extract_dir = temp_dir.path().join("extract");
    let archive_path = temp_dir
        .path()
        .join(format!("kotlin-compiler-{version}.zip"));
    {
        let extract_dir = extract_dir.clone();
        smol::unblock(move || {
            std::fs::create_dir_all(&extract_dir)?;
            Ok::<_, eyre::Report>(())
        })
        .await?;
    }

    download_file_with_redirect(&kotlin_compiler_release_url(version), &archive_path).await?;
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

    let extracted_home = {
        let extract_dir = extract_dir.clone();
        smol::unblock(move || find_kotlin_home_dir(&extract_dir)).await?
    };
    remove_directory_if_exists(&install_home).await?;
    {
        let extracted_home = extracted_home.clone();
        let install_home = install_home.clone();
        smol::unblock(move || std::fs::rename(extracted_home, install_home))
            .await
            .map_err(eyre::Report::from)?;
    }

    let kotlinc_path = kotlin_executable_from_home(&install_home).ok_or_else(|| {
        eyre::eyre!(
            "Kotlin compiler `{version}` was extracted but `{}` is still missing",
            kotlinc_binary_name()
        )
    })?;
    let installed_version = kotlin_compiler_version(&kotlinc_path).await?;
    if kotlin_version_is_compatible(&installed_version, version) {
        Ok(kotlinc_path)
    } else {
        Err(eyre::eyre!(
            "Installed Kotlin compiler version `{installed_version}` does not satisfy required version `{version}`"
        ))
    }
}

const fn required_kotlin_version() -> &'static str {
    build_info::ANDROID_KOTLIN_VERSION
}

async fn required_ndk_package_id() -> eyre::Result<String> {
    let runtime_build_gradle =
        find_file_in_current_workspace(Path::new(ANDROID_RUNTIME_BUILD_GRADLE_RELATIVE_PATH))
            .ok_or_else(|| {
                eyre::eyre!(
                    "Failed to locate `{ANDROID_RUNTIME_BUILD_GRADLE_RELATIVE_PATH}` while resolving the required Android NDK version"
                )
            })?;
    let contents = smol::fs::read_to_string(&runtime_build_gradle)
        .await
        .wrap_err_with(|| {
            format!(
                "Failed to read Android runtime Gradle config at `{}`",
                runtime_build_gradle.display()
            )
        })?;
    let version =
        parse_android_ndk_version_from_runtime_build_gradle(&contents).ok_or_else(|| {
            eyre::eyre!(
                "Failed to parse `ndkVersion` from `{}`",
                runtime_build_gradle.display()
            )
        })?;
    let package_id = format!("ndk;{version}");
    let available_packages = list_sdk_package_ids().await?;
    if available_packages
        .iter()
        .any(|candidate| candidate == &package_id)
    {
        Ok(package_id)
    } else {
        Err(eyre::eyre!(
            "Required Android NDK package `{package_id}` from `{}` is not available via `sdkmanager --list`",
            runtime_build_gradle.display()
        ))
    }
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

fn required_android_rust_targets_for_abis(abis: &[AndroidAbi]) -> Vec<String> {
    let mut targets = abis
        .iter()
        .map(|abi| rust_target_for_android_abi(*abi).to_owned())
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

fn missing_android_rust_targets(
    installed_targets: &[String],
    required_targets: &[String],
) -> Vec<String> {
    required_targets
        .iter()
        .filter(|target| {
            !installed_targets
                .iter()
                .any(|installed| installed == *target)
        })
        .cloned()
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
        "Automatic Android SDK command-line tools installation is unsupported on this host. Set up Android SDK manually and set `ANDROID_SDK_ROOT`."
    )]
    UnsupportedPlatform,
}

impl Toolchain for AndroidSdk {
    type Installation = AndroidSdkInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if Self::detect_path().is_some() {
            if Self::sdkmanager_path().await.is_some() {
                Ok(())
            } else {
                Err(ToolchainError::fixable(AndroidSdkInstallation))
            }
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
        } else if cfg!(target_os = "linux") {
            if configured_android_sdk_path().is_some() {
                Err(ToolchainError::fixable(AndroidSdkInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Android SDK root cannot be determined",
                    format!(
                        "Set `ANDROID_SDK_ROOT` to your Android SDK path, then retry `water doctor --fix`. {}",
                        android_cmdline_tools_suggestion()
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
        } else if cfg!(target_os = "linux") {
            // Linux CI/headless containers only need command-line tools in the SDK root.
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

        if AndroidSdk::sdkmanager_path().await.is_some() {
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
#[derive(Debug, Clone, Copy, Default)]
pub enum AndroidPlatformToolsInstallation {
    /// Install the `platform-tools` SDK package with `sdkmanager`.
    #[default]
    SdkPackage,
    /// Install `x86_64` userspace libraries needed by Google's Linux host tools on ARM Linux.
    LinuxX86_64HostToolsCompat,
}

/// Errors that can occur when installing Android Platform-Tools.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidPlatformTools {
    #[error("Android SDK command-line tools (`sdkmanager`) not found.")]
    SdkManagerNotFound,
    #[error("Failed to install Android Platform-Tools via sdkmanager: {0}")]
    InstallFailed(eyre::Report),
    #[error("Failed to install Android x86_64 host-tools compatibility packages: {0}")]
    HostToolsCompatFailed(eyre::Report),
    #[error("Android Platform-Tools (`adb`) is still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidPlatformTools {
    type Installation = AndroidPlatformToolsInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if let Some(adb_path) = AndroidSdk::adb_path() {
            return verify_android_platform_tools_executable(&adb_path).await;
        }

        if AndroidSdk::sdkmanager_path().await.is_some() {
            Err(ToolchainError::fixable(
                AndroidPlatformToolsInstallation::SdkPackage,
            ))
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
        if matches!(self, Self::LinuxX86_64HostToolsCompat) {
            return install_android_linux_x86_64_host_tools_compat()
                .await
                .map_err(FailToInstallAndroidPlatformTools::HostToolsCompatFailed);
        }

        if AndroidSdk::sdkmanager_path().await.is_none() {
            return Err(FailToInstallAndroidPlatformTools::SdkManagerNotFound);
        }

        install_android_sdk_package("platform-tools")
            .await
            .map_err(FailToInstallAndroidPlatformTools::InstallFailed)?;

        verify_android_platform_tools_after_install().await
    }
}

async fn verify_android_platform_tools_after_install()
-> Result<(), FailToInstallAndroidPlatformTools> {
    let adb_path = AndroidSdk::adb_path().ok_or(FailToInstallAndroidPlatformTools::StillMissing)?;
    match verify_android_platform_tools_executable(&adb_path).await {
        Ok(()) => Ok(()),
        Err(ToolchainError::Fixable(
            AndroidPlatformToolsInstallation::LinuxX86_64HostToolsCompat,
        )) => {
            install_android_linux_x86_64_host_tools_compat()
                .await
                .map_err(FailToInstallAndroidPlatformTools::HostToolsCompatFailed)?;
            verify_android_platform_tools_executable(&adb_path)
                .await
                .map_err(|error| FailToInstallAndroidPlatformTools::InstallFailed(error.into()))
        }
        Err(error) => Err(FailToInstallAndroidPlatformTools::InstallFailed(
            error.into(),
        )),
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

        let missing_targets =
            missing_android_rust_targets(&installed_targets, &self.required_targets);
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
        let still_missing = missing_android_rust_targets(&installed_targets, &self.missing_targets);
        if still_missing.is_empty() {
            Ok(())
        } else {
            Err(FailToInstallAndroidRustTargets::StillMissing {
                missing_targets: still_missing.join(", "),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_android_rust_targets_are_deduplicated() {
        let required = required_android_rust_targets_for_abis(&[
            AndroidAbi::Arm64V8a,
            AndroidAbi::Arm64V8a,
            AndroidAbi::X86_64,
        ]);
        assert_eq!(
            required,
            vec![
                "aarch64-linux-android".to_string(),
                "x86_64-linux-android".to_string()
            ]
        );
    }

    #[test]
    fn missing_android_targets_only_consider_requested_abis() {
        let required = required_android_rust_targets_for_abis(&[AndroidAbi::Arm64V8a]);
        let installed = vec![
            "aarch64-linux-android".to_string(),
            "armv7-linux-androideabi".to_string(),
            "x86_64-linux-android".to_string(),
        ];
        assert!(missing_android_rust_targets(&installed, &required).is_empty());
    }

    #[test]
    fn missing_android_targets_report_only_requested_missing_entries() {
        let required =
            required_android_rust_targets_for_abis(&[AndroidAbi::Arm64V8a, AndroidAbi::X86]);
        let installed = vec!["aarch64-linux-android".to_string()];
        assert_eq!(
            missing_android_rust_targets(&installed, &required),
            vec!["i686-linux-android".to_string()]
        );
    }

    #[test]
    fn parse_android_ndk_version_from_runtime_build_gradle_extracts_declared_version() {
        let contents = r#"
android {
    compileSdk = 37
    ndkVersion = "29.0.14206865"
}
"#;
        assert_eq!(
            parse_android_ndk_version_from_runtime_build_gradle(contents),
            Some("29.0.14206865".to_string())
        );
    }

    #[test]
    fn select_installed_ndk_path_prefers_required_version_over_latest_directory() {
        let tempdir = tempfile::tempdir().unwrap();
        let ndk_dir = tempdir.path().join("ndk");
        std::fs::create_dir_all(ndk_dir.join("29.0.14206865")).unwrap();
        std::fs::create_dir_all(ndk_dir.join("30.0.14904198")).unwrap();

        assert_eq!(
            select_installed_ndk_path(&ndk_dir, Some("29.0.14206865")),
            Some(ndk_dir.join("29.0.14206865"))
        );
    }

    #[test]
    fn ndk_path_for_package_id_rejects_non_ndk_package_ids() {
        let error =
            ndk_path_for_package_id(Path::new("/tmp/android-sdk"), "platform-tools").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid Android NDK package id `platform-tools`")
        );
    }

    #[test]
    fn parse_kotlinc_version_output_extracts_version_token() {
        assert_eq!(
            parse_kotlinc_version_output("info: kotlinc-jvm 1.3-SNAPSHOT (JRE 21.0.10+7)"),
            Some("1.3-SNAPSHOT".to_string())
        );
    }

    #[test]
    fn parse_kotlinc_version_output_ignores_jdk_warning_prefix() {
        let output = "OpenJDK 64-Bit Server VM warning: Options -Xverify:none and -noverify were deprecated in JDK 13 and will likely be removed in a future release.\ninfo: kotlinc-jvm 1.3-SNAPSHOT (JRE 21.0.10+7)";
        assert_eq!(
            parse_kotlinc_version_output(output),
            Some("1.3-SNAPSHOT".to_string())
        );
    }

    #[test]
    fn kotlin_version_compatibility_uses_backend_minimum() {
        assert!(kotlin_version_is_compatible("2.0.21", "2.0.21"));
        assert!(kotlin_version_is_compatible("2.1.0", "2.0.21"));
        assert!(!kotlin_version_is_compatible("1.9.24", "2.0.21"));
    }

    #[test]
    fn parse_sdkmanager_proxy_config_maps_http_proxy() {
        assert_eq!(
            parse_sdkmanager_proxy_config("http://host.docker.internal:7891").unwrap(),
            SdkManagerProxyConfig {
                proxy_type: SdkManagerProxyType::Http,
                host: "host.docker.internal".to_string(),
                port: 7891,
            }
        );
    }

    #[test]
    fn parse_sdkmanager_proxy_config_maps_socks5h_proxy() {
        assert_eq!(
            parse_sdkmanager_proxy_config("socks5h://host.docker.internal:7890").unwrap(),
            SdkManagerProxyConfig {
                proxy_type: SdkManagerProxyType::Socks,
                host: "host.docker.internal".to_string(),
                port: 7890,
            }
        );
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

async fn verify_android_platform_tools_executable(
    adb_path: &Path,
) -> Result<(), ToolchainError<AndroidPlatformToolsInstallation>> {
    let output = run_command_output_os(adb_path, ["version"]).await.map_err(|error| {
        ToolchainError::unfixable(
            format!(
                "Android Platform-Tools (`adb`) exists but failed to spawn on this host: {error}"
            ),
            format!(
                "Ensure the Android Platform-Tools binary at `{}` can start on this host, then retry `water doctor`.",
                adb_path.display()
            ),
        )
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim().to_owned()
    } else if !stdout.trim().is_empty() {
        stdout.trim().to_owned()
    } else {
        format!("exit status {}", output.status)
    };
    if needs_linux_x86_64_host_tools_compat(&detail) {
        return Err(ToolchainError::fixable(
            AndroidPlatformToolsInstallation::LinuxX86_64HostToolsCompat,
        ));
    }

    let suggestion = if detail.contains("ld-linux-x86-64.so.2") {
        format!(
            "Install x86_64 userspace compatibility libraries for this Linux host, then retry `water doctor --fix`. Required packages on Debian/Ubuntu: {}.",
            ANDROID_LINUX_X86_64_HOST_TOOLS_COMPAT_PACKAGES.join(" ")
        )
    } else {
        format!(
            "Ensure the Android Platform-Tools binary at `{}` can execute on this host, then retry `water doctor`.",
            adb_path.display()
        )
    };
    Err(ToolchainError::unfixable(
        format!(
            "Android Platform-Tools (`adb`) exists but failed to execute on this host: {detail}"
        ),
        suggestion,
    ))
}

fn ndk_host_clang_path(ndk_path: &Path) -> Option<PathBuf> {
    use super::ANDROID_MIN_API_LEVEL;

    let prebuilt_dir = ndk_path.join("toolchains/llvm/prebuilt");
    let entries = std::fs::read_dir(&prebuilt_dir).ok()?;
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    candidates.sort();

    for candidate in candidates {
        let executable = format!(
            "aarch64-linux-android{ANDROID_MIN_API_LEVEL}-clang{}",
            if cfg!(target_os = "windows") {
                ".cmd"
            } else {
                ""
            }
        );
        let clang = candidate.join("bin").join(executable);
        if clang.exists() {
            return Some(clang);
        }
    }

    None
}

async fn verify_ndk_host_toolchain_executable(
    ndk_path: &Path,
) -> Result<(), ToolchainError<AndroidNdkInstallation>> {
    let clang_path = ndk_host_clang_path(ndk_path).ok_or_else(|| {
        ToolchainError::unfixable(
            "Android NDK toolchain is incomplete (`clang` was not found under toolchains/llvm/prebuilt).",
            android_ndk_install_suggestion(),
        )
    })?;

    let probe_source = std::env::temp_dir().join("waterui_android_ndk_probe.c");
    {
        let probe_source_for_write = probe_source.clone();
        smol::unblock(move || {
            std::fs::write(
                &probe_source_for_write,
                b"int main(void) { return 0; }
",
            )
        })
        .await
        .map_err(|error| {
            ToolchainError::unfixable(
                format!(
                    "Failed to create Android NDK probe source at {}: {error}",
                    probe_source.display()
                ),
                "Ensure the temporary directory is writable, then retry `water doctor`.",
            )
        })?;
    }

    let probe_output = if cfg!(target_os = "windows") {
        PathBuf::from("NUL")
    } else {
        PathBuf::from("/dev/null")
    };
    let result = run_command_output_os(
        &clang_path,
        [
            OsString::from("-x"),
            OsString::from("c"),
            OsString::from("-c"),
            probe_source.clone().into_os_string(),
            OsString::from("-o"),
            probe_output.into_os_string(),
        ],
    )
    .await;
    {
        let probe_source = probe_source.clone();
        let _ = smol::unblock(move || std::fs::remove_file(&probe_source)).await;
    }
    let output = result.map_err(|error| {
        ToolchainError::unfixable(
            format!(
                "Android NDK toolchain exists but failed to spawn on this host: {error}"
            ),
            format!(
                "Ensure the Android NDK toolchain binary `{}` can start on this host, then retry packaging.",
                clang_path.display()
            ),
        )
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim().to_owned()
    } else if !stdout.trim().is_empty() {
        stdout.trim().to_owned()
    } else {
        format!("exit status {}", output.status)
    };
    if needs_linux_x86_64_host_tools_compat(&detail) {
        return Err(ToolchainError::fixable(
            AndroidNdkInstallation::LinuxX86_64HostToolsCompat,
        ));
    }

    let suggestion = if detail.contains("ld-linux-x86-64.so.2") {
        format!(
            "Install x86_64 userspace compatibility libraries for this Linux host, then retry `water doctor --fix`. Required packages on Debian/Ubuntu: {}.",
            ANDROID_LINUX_X86_64_HOST_TOOLS_COMPAT_PACKAGES.join(" ")
        )
    } else {
        format!(
            "Ensure the Android NDK toolchain binaries under `{}` can execute on this host, then retry packaging.",
            clang_path.display()
        )
    };
    Err(ToolchainError::unfixable(
        format!("Android NDK toolchain exists but failed to execute on this host: {detail}"),
        suggestion,
    ))
}

impl Java {
    /// Detect the path to the Java installation for Android development.
    ///
    /// Priority order:
    /// 1. Android Studio's bundled JBR (guaranteed compatible with AGP)
    /// 2. `JAVA_HOME` environment variable (may be incompatible)
    /// 3. Java from `PATH`
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

        if cfg!(target_os = "linux")
            && let Ok(home) = env::var("HOME")
        {
            let paths = [
                format!("{home}/.local/share/JetBrains/Toolbox/apps/android-studio/jbr/bin/java"),
                format!("{home}/android-studio/jbr/bin/java"),
            ];
            for path in paths {
                let java_path = PathBuf::from(&path);
                if java_path.exists() {
                    return Some(java_path);
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

    /// Get the `JAVA_HOME` directory (parent of `bin/`).
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
        let required_version = required_kotlin_version();
        let mut candidates = Vec::new();

        if let Ok(home) = env::var("KOTLIN_HOME")
            && let Some(kotlinc_path) = kotlin_executable_from_home(&PathBuf::from(&home))
        {
            candidates.push(kotlinc_path);
        }

        if cfg!(target_os = "macos") {
            const ANDROID_STUDIO_KOTLINS: &[&str] = &[
                "/Applications/Android Studio.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
                "/Applications/Android Studio Preview.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
            ];
            for path in ANDROID_STUDIO_KOTLINS {
                let kotlinc_path = PathBuf::from(path);
                if kotlinc_path.exists() {
                    candidates.push(kotlinc_path);
                }
            }
        }

        if cfg!(target_os = "linux")
            && let Ok(home) = env::var("HOME")
        {
            let paths = [
                format!(
                    "{home}/.local/share/JetBrains/Toolbox/apps/android-studio/plugins/Kotlin/kotlinc/bin/kotlinc"
                ),
                format!("{home}/android-studio/plugins/Kotlin/kotlinc/bin/kotlinc"),
            ];
            for path in paths {
                let kotlinc_path = PathBuf::from(&path);
                if kotlinc_path.exists() {
                    candidates.push(kotlinc_path);
                }
            }
        }

        if cfg!(target_os = "windows")
            && let Ok(program_files) = env::var("ProgramFiles")
        {
            let kotlinc_path = PathBuf::from(&program_files)
                .join("Android/Android Studio/plugins/Kotlin/kotlinc/bin/kotlinc.bat");
            if kotlinc_path.exists() {
                candidates.push(kotlinc_path);
            }
        }

        if let Ok(managed_home) = managed_kotlin_home(required_version)
            && let Some(kotlinc_path) = kotlin_executable_from_home(&managed_home)
        {
            candidates.push(kotlinc_path);
        }

        if let Ok(path) = which("kotlinc").await {
            candidates.push(path);
        }

        candidates.dedup();
        for candidate in candidates {
            let Ok(installed_version) = kotlin_compiler_version(&candidate).await else {
                continue;
            };
            if kotlin_version_is_compatible(&installed_version, required_version) {
                return Some(candidate);
            }
        }

        None
    }
}

/// Kotlin installation handler.
#[derive(Debug)]
pub struct KotlinInstallation;

/// Errors that can occur when installing Kotlin.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallKotlin {
    #[error("Failed to install Kotlin compiler: {0}")]
    InstallFailed(eyre::Report),
    #[error("Kotlin compiler is still missing after installation.")]
    StillMissing,
}

impl Toolchain for Kotlin {
    type Installation = KotlinInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        let kotlinc_path = Self::detect_path()
            .await
            .ok_or_else(|| ToolchainError::fixable(KotlinInstallation))?;
        // Only unix carries an execute bit, so elsewhere finding the file is
        // the whole check. Both arms are tail expressions rather than an early
        // `return` under one `cfg`, which would leave the other arm as dead
        // code on the platform that does compile it.
        #[cfg(unix)]
        {
            Self::reject_non_executable(kotlinc_path).await
        }
        #[cfg(not(unix))]
        {
            drop(kotlinc_path);
            Ok(())
        }
    }
}

impl Kotlin {
    /// Rejects a `kotlinc` the current user cannot run.
    ///
    /// Only unix carries an execute bit, so elsewhere finding the file is the
    /// whole check — hence the two bodies rather than one with the permission
    /// half wrapped in `cfg`, which left the path unread on every other
    /// platform and tripped an unused-variable lint nobody was running.
    #[cfg(unix)]
    async fn reject_non_executable(
        kotlinc_path: PathBuf,
    ) -> Result<(), ToolchainError<KotlinInstallation>> {
        use std::os::unix::fs::PermissionsExt as _;

        let Ok(metadata) = smol::unblock({
            let kotlinc_path = kotlinc_path.clone();
            move || std::fs::metadata(&kotlinc_path)
        })
        .await
        else {
            return Ok(());
        };
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ToolchainError::unfixable(
                "Kotlin compiler (kotlinc) is not executable",
                format!(
                    "The kotlinc script at '{}' does not have execute permission. Fix it with: sudo chmod +x '{}'",
                    kotlinc_path.display(),
                    kotlinc_path.display()
                ),
            ));
        }
        Ok(())
    }
}

impl Installation for KotlinInstallation {
    type Error = FailToInstallKotlin;

    async fn install(&self) -> Result<(), Self::Error> {
        let required_version = required_kotlin_version();
        install_managed_kotlin_compiler(required_version)
            .await
            .map_err(FailToInstallKotlin::InstallFailed)?;
        if Kotlin::detect_path().await.is_some() {
            Ok(())
        } else {
            Err(FailToInstallKotlin::StillMissing)
        }
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
        select_installed_ndk_path(
            &ndk_dir,
            required_ndk_version_in_current_workspace().as_deref(),
        )
    }
}

/// Android NDK installation handler.
#[derive(Debug, Clone, Copy, Default)]
pub enum AndroidNdkInstallation {
    /// Install the runtime-declared NDK package with `sdkmanager`.
    #[default]
    SdkPackage,
    /// Install `x86_64` userspace libraries needed by Google's Linux host tools on ARM Linux.
    LinuxX86_64HostToolsCompat,
}

/// Errors that can occur when installing the Android NDK.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidNdk {
    #[error("Android SDK command-line tools (`sdkmanager`) not found.")]
    SdkManagerNotFound,
    #[error("Failed to install Android NDK via sdkmanager: {0}")]
    InstallFailed(eyre::Report),
    #[error("Failed to install Android x86_64 host-tools compatibility packages: {0}")]
    HostToolsCompatFailed(eyre::Report),
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
                return verify_ndk_host_toolchain_executable(&ndk_path).await;
            }

            if AndroidSdk::sdkmanager_path().await.is_some() {
                return Err(ToolchainError::fixable(AndroidNdkInstallation::SdkPackage));
            }

            return Err(ToolchainError::unfixable(
                "Android NDK is installed but incomplete",
                android_ndk_install_suggestion(),
            ));
        }

        if AndroidSdk::sdkmanager_path().await.is_some() {
            Err(ToolchainError::fixable(AndroidNdkInstallation::SdkPackage))
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
        if matches!(self, Self::LinuxX86_64HostToolsCompat) {
            return install_android_linux_x86_64_host_tools_compat()
                .await
                .map_err(FailToInstallAndroidNdk::HostToolsCompatFailed);
        }

        if AndroidSdk::sdkmanager_path().await.is_none() {
            return Err(FailToInstallAndroidNdk::SdkManagerNotFound);
        }

        let (_, sdk_root) = resolve_sdkmanager_and_root()
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        let ndk_package = required_ndk_package_id()
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        let ndk_path = ndk_path_for_package_id(&sdk_root, &ndk_package)
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        if ndk_path.exists() && !ndk_layout_is_complete(&ndk_path) {
            remove_directory_if_exists(&ndk_path)
                .await
                .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        }
        install_android_sdk_package(&ndk_package)
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;

        if !ndk_path.exists() {
            return Err(FailToInstallAndroidNdk::StillMissing);
        }

        if !ndk_layout_is_complete(&ndk_path) {
            return Err(FailToInstallAndroidNdk::Incomplete);
        }

        verify_android_ndk_after_install(&ndk_path).await
    }
}

async fn verify_android_ndk_after_install(ndk_path: &Path) -> Result<(), FailToInstallAndroidNdk> {
    match verify_ndk_host_toolchain_executable(ndk_path).await {
        Ok(()) => Ok(()),
        Err(ToolchainError::Fixable(AndroidNdkInstallation::LinuxX86_64HostToolsCompat)) => {
            install_android_linux_x86_64_host_tools_compat()
                .await
                .map_err(FailToInstallAndroidNdk::HostToolsCompatFailed)?;
            verify_ndk_host_toolchain_executable(ndk_path)
                .await
                .map_err(|error| FailToInstallAndroidNdk::InstallFailed(error.into()))
        }
        Err(error) => Err(FailToInstallAndroidNdk::InstallFailed(error.into())),
    }
}
