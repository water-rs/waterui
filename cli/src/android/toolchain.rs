use std::{
    cmp::Ordering,
    env,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::Output,
};

use url::Url;
use walkdir::WalkDir;
use waterui_assets_core::{AssetError, download_remote_bytes, write_bytes_atomically};

use crate::{
    android::platform::{ALL_ABIS, AndroidAbi},
    brew::Brew,
    build_info,
    toolchain::{
        Host, Installation, Toolchain, ToolchainError,
        linux::{
            LinuxPackageManagerError, has_supported_package_manager, install_java_jdk,
            install_named_packages,
        },
        winget::{WingetInstallError, ensure_package_installed},
    },
    utils::{CommandError, command},
    water_dir::{HomeDirError, water_home_dir_in},
};

/// Errors from Android SDK/NDK inspection and installation pipelines.
#[derive(Debug, thiserror::Error)]
pub enum AndroidToolchainError {
    /// `sdkmanager` could not be located.
    #[error("Android SDK command-line tools (`sdkmanager`) not found")]
    SdkManagerNotFound,
    /// The Android SDK root could not be derived from the environment or `sdkmanager` path.
    #[error("Android SDK root could not be determined from environment or sdkmanager path")]
    SdkRootUndetermined,
    /// The Android SDK root cannot be determined on this host.
    #[error("Android SDK root cannot be determined on this host")]
    SdkRootUnavailable,
    /// No Java runtime is available for `sdkmanager`.
    #[error("Java runtime not found while invoking sdkmanager")]
    JavaNotFound,
    /// The Water home directory could not be resolved.
    #[error(transparent)]
    HomeDir(#[from] HomeDirError),
    /// The SDK repository metadata request failed.
    #[error("Failed to query Android SDK repository metadata: {0}")]
    RepositoryQuery(#[source] zenwave::Error),
    /// The SDK repository metadata request returned an unsuccessful status.
    #[error("Failed to query Android SDK repository metadata: HTTP {0}")]
    RepositoryStatus(zenwave::StatusCode),
    /// The SDK repository metadata body could not be read.
    #[error("Failed to read Android SDK repository metadata: {0}")]
    RepositoryBody(#[from] zenwave::BodyError),
    /// The command-line tools archive is absent from the repository metadata.
    #[error("Could not locate Android command-line tools archive")]
    CmdlineToolsArchiveNotFound,
    /// A remote archive could not be downloaded.
    #[error("Failed to download {url}: {source}")]
    Download {
        /// The URL that failed to download.
        url: String,
        /// The underlying asset error.
        #[source]
        source: AssetError,
    },
    /// A downloaded archive could not be written to disk.
    #[error("Failed to write downloaded archive to {}: {source}", path.display())]
    ArchiveWrite {
        /// The destination path.
        path: PathBuf,
        /// The underlying asset error.
        #[source]
        source: AssetError,
    },
    /// The command-line tools archive has no `bin` directory.
    #[error("Invalid Android command-line tools archive layout (missing bin directory)")]
    CmdlineToolsMissingBinDir,
    /// The command-line tools archive has no `cmdline-tools` root.
    #[error("Invalid Android command-line tools archive layout (missing cmdline-tools root)")]
    CmdlineToolsMissingRoot,
    /// The command-line tools archive does not contain `sdkmanager`.
    #[error("Android command-line tools archive does not contain sdkmanager")]
    CmdlineToolsMissingSdkManager,
    /// `sdkmanager` is still absent after extraction.
    #[error("Android command-line tools were extracted but sdkmanager is still missing")]
    CmdlineToolsStillMissingSdkManager,
    /// The Kotlin compiler archive has no `bin` directory.
    #[error("Invalid Kotlin compiler archive layout (missing bin directory)")]
    KotlinMissingBinDir,
    /// The Kotlin compiler archive has no compiler root.
    #[error("Invalid Kotlin compiler archive layout (missing compiler root)")]
    KotlinMissingRoot,
    /// The Kotlin compiler archive does not contain the `kotlinc` executable.
    #[error("Kotlin compiler archive does not contain {0}")]
    KotlinMissingCompiler(String),
    /// The managed Kotlin install path has no parent directory.
    #[error("Managed Kotlin install path has no parent")]
    KotlinInstallPathNoParent,
    /// `kotlinc` is still absent after extraction.
    #[error("Kotlin compiler `{version}` was extracted but `{executable}` is still missing")]
    KotlinCompilerStillMissing {
        /// The requested Kotlin version.
        version: String,
        /// The executable that is missing.
        executable: &'static str,
    },
    /// The installed Kotlin compiler does not satisfy the required version.
    #[error(
        "Installed Kotlin compiler version `{installed}` does not satisfy required version `{required}`"
    )]
    KotlinVersionMismatch {
        /// The version reported by the installed compiler.
        installed: String,
        /// The required Kotlin version.
        required: String,
    },
    /// The Kotlin compiler version output could not be parsed.
    #[error(
        "Failed to parse Kotlin compiler version from `{}` output: {output}",
        path.display()
    )]
    KotlinVersionParse {
        /// The `kotlinc` path that was probed.
        path: PathBuf,
        /// The combined compiler output.
        output: String,
    },
    /// The proxy environment value is not a valid URL.
    #[error("Failed to parse proxy URL `{url}` for sdkmanager: {source}")]
    ProxyParse {
        /// The offending proxy value.
        url: String,
        /// The URL parse error.
        #[source]
        source: url::ParseError,
    },
    /// The proxy URL has no host.
    #[error("Proxy URL `{0}` is missing a host")]
    ProxyMissingHost(String),
    /// The proxy URL has no port.
    #[error("Proxy URL `{0}` is missing a port")]
    ProxyMissingPort(String),
    /// The proxy URL scheme is not supported by `sdkmanager`.
    #[error("Unsupported proxy scheme `{0}` for sdkmanager")]
    ProxyUnsupportedScheme(String),
    /// A PATH entry could not be joined into `PATH`.
    #[error("Failed to construct PATH with required entry '{}': {source}", entry.display())]
    PathJoin {
        /// The entry that could not be joined.
        entry: PathBuf,
        /// The path-join error.
        #[source]
        source: env::JoinPathsError,
    },
    /// `sdkmanager --licenses` did not succeed.
    #[error("Failed to accept Android SDK licenses. {0}")]
    LicenseAcceptance(String),
    /// `sdkmanager --install` did not succeed.
    #[error("Failed to install package `{package_id}` via sdkmanager. {output}")]
    PackageInstall {
        /// The SDK package that failed to install.
        package_id: String,
        /// The combined `sdkmanager` output.
        output: String,
    },
    /// `sdkmanager --list` did not succeed.
    #[error("Failed to list Android SDK packages via sdkmanager. {0}")]
    PackageList(String),
    /// An `ndk;` package id is malformed.
    #[error("Invalid Android NDK package id `{0}`")]
    InvalidNdkPackageId(String),
    /// The Android runtime Gradle config could not be located in the workspace.
    #[error("Failed to locate `{path}` while resolving the required Android NDK version")]
    RuntimeGradleMissing {
        /// The relative path that was searched for.
        path: &'static str,
    },
    /// The Android runtime Gradle config could not be read.
    #[error("Failed to read Android runtime Gradle config at `{}`: {source}", path.display())]
    RuntimeGradleRead {
        /// The Gradle config path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// `ndkVersion` could not be parsed from the runtime Gradle config.
    #[error("Failed to parse `ndkVersion` from `{}`", path.display())]
    NdkVersionUnparseable {
        /// The Gradle config path.
        path: PathBuf,
    },
    /// The required NDK package is not offered by `sdkmanager`.
    #[error(
        "Required Android NDK package `{package_id}` from `{}` is not available via `sdkmanager --list`",
        gradle_path.display()
    )]
    NdkPackageUnavailable {
        /// The required NDK package id.
        package_id: String,
        /// The Gradle config that declared the requirement.
        gradle_path: PathBuf,
    },
    /// No Android platform package is offered by `sdkmanager`.
    #[error("No installable Android platform package found via `sdkmanager --list`")]
    NoPlatformPackage,
    /// No Android build-tools package is offered by `sdkmanager`.
    #[error("No installable Android build-tools package found via `sdkmanager --list`")]
    NoBuildToolsPackage,
    /// An external command failed.
    #[error(transparent)]
    Command(#[from] CommandError),
    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A ZIP archive operation failed.
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    /// A directory-tree walk failed.
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
}

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

async fn install_android_linux_x86_64_host_tools_compat(
    host: &Host,
) -> Result<(), LinuxPackageManagerError> {
    install_named_packages(host, ANDROID_LINUX_X86_64_HOST_TOOLS_COMPAT_PACKAGES).await
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

fn default_android_sdk_path(host: &Host) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let localappdata = host.env_string("LOCALAPPDATA")?;
        return Some(PathBuf::from(localappdata).join("Android/Sdk"));
    }

    let home = host.home_dir()?;
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Android/sdk"));
    }

    if cfg!(target_os = "linux") {
        return Some(home.join("Android/Sdk"));
    }

    None
}

fn configured_android_sdk_path(host: &Host) -> Option<PathBuf> {
    for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Some(raw) = host.env_string(key) {
            return Some(PathBuf::from(raw));
        }
    }
    default_android_sdk_path(host)
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

async fn latest_cmdline_tools_archive_url() -> Result<String, AndroidToolchainError> {
    use zenwave::{Client, Method};

    const REPOSITORY_URL: &str = "https://dl.google.com/android/repository/repository2-3.xml";
    const REPOSITORY_PREFIX: &str = "https://dl.google.com/android/repository/";

    let mut client = zenwave::client();
    let response = client
        .method(Method::GET, REPOSITORY_URL)
        .map_err(AndroidToolchainError::RepositoryQuery)?
        .await
        .map_err(AndroidToolchainError::RepositoryQuery)?;
    if !response.status().is_success() {
        return Err(AndroidToolchainError::RepositoryStatus(response.status()));
    }

    let bytes = response.into_body().into_bytes().await?;
    let repository_xml = String::from_utf8_lossy(&bytes).into_owned();
    let archive_name = parse_latest_cmdline_tools_archive(&repository_xml)
        .ok_or(AndroidToolchainError::CmdlineToolsArchiveNotFound)?;
    Ok(format!("{REPOSITORY_PREFIX}{archive_name}"))
}

async fn download_file_with_redirect(
    url: &str,
    destination: &Path,
) -> Result<(), AndroidToolchainError> {
    let bytes =
        download_remote_bytes(url)
            .await
            .map_err(|source| AndroidToolchainError::Download {
                url: url.to_owned(),
                source,
            })?;
    write_bytes_atomically(destination, &bytes)
        .await
        .map_err(|source| AndroidToolchainError::ArchiveWrite {
            path: destination.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn find_cmdline_tools_dir(root: &Path) -> Result<PathBuf, AndroidToolchainError> {
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

        let bin_dir = path
            .parent()
            .ok_or(AndroidToolchainError::CmdlineToolsMissingBinDir)?;
        let cmdline_tools_dir = bin_dir
            .parent()
            .ok_or(AndroidToolchainError::CmdlineToolsMissingRoot)?;
        return Ok(cmdline_tools_dir.to_path_buf());
    }

    Err(AndroidToolchainError::CmdlineToolsMissingSdkManager)
}

async fn ensure_cmdline_tools_available(sdk_root: &Path) -> Result<(), AndroidToolchainError> {
    let latest_dir = sdk_root.join("cmdline-tools/latest");
    let sdkmanager = latest_dir.join("bin").join(sdkmanager_binary_name());
    if sdkmanager.exists() {
        return Ok(());
    }

    let cmdline_tools_root = sdk_root.join("cmdline-tools");
    let temp_dir = {
        let cmdline_tools_root = cmdline_tools_root.clone();
        smol::unblock(move || -> io::Result<_> {
            std::fs::create_dir_all(&cmdline_tools_root)?;
            tempfile::Builder::new()
                .prefix(".water-cmdline-tools-")
                .tempdir_in(&cmdline_tools_root)
        })
        .await?
    };
    let extract_dir = temp_dir.path().join("extract");
    let archive_path = temp_dir.path().join("commandline-tools.zip");

    {
        let extract_dir = extract_dir.clone();
        smol::unblock(move || std::fs::create_dir_all(&extract_dir)).await?;
    }

    let archive_url = latest_cmdline_tools_archive_url().await?;
    download_file_with_redirect(&archive_url, &archive_path).await?;

    {
        let archive_path = archive_path.clone();
        let extract_dir = extract_dir.clone();
        smol::unblock(move || -> Result<(), AndroidToolchainError> {
            let archive_file = std::fs::File::open(&archive_path)?;
            let mut archive = zip::ZipArchive::new(archive_file)?;
            archive.extract(&extract_dir)?;
            Ok(())
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
        Err(AndroidToolchainError::CmdlineToolsStillMissingSdkManager)
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
            .and_then(parse_android_version_pair)
            .unwrap_or((0, 0));
        let right_api = right
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("android-"))
            .and_then(parse_android_version_pair)
            .unwrap_or((0, 0));
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

fn find_sdkmanager_on_host_path(host: &Host) -> Option<PathBuf> {
    let path_env = host.env("PATH")?;
    for path_dir in env::split_paths(path_env) {
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

/// The `(major, minor)` API-level pair of an Android platform identifier.
///
/// `sdkmanager` lists packages like `platforms;android-37` or, for minor
/// API revisions, `platforms;android-37.0` (#633): `android-36` parses as
/// `(36, 0)`, `android-36.1` as `(36, 1)`, so pair ordering gives
/// `android-36 < android-36.1 < android-37.0`. A non-numeric identifier is
/// not a numbered platform at all.
fn parse_android_version_pair(value: &str) -> Option<(u32, u32)> {
    let mut segments = value.split('.');
    let major = segments.next()?.parse().ok()?;
    let minor = match segments.next() {
        Some(segment) => segment.parse().ok()?,
        None => 0,
    };
    if segments.next().is_some() {
        return None;
    }
    Some((major, minor))
}

fn parse_android_platform_api_level(package_id: &str) -> Option<(u32, u32)> {
    parse_android_version_pair(package_id.strip_prefix("platforms;android-")?)
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

async fn resolve_sdkmanager_and_root(
    host: &Host,
) -> Result<(PathBuf, PathBuf), AndroidToolchainError> {
    let sdkmanager_path = AndroidSdk::sdkmanager_path(host)
        .await
        .ok_or(AndroidToolchainError::SdkManagerNotFound)?;
    let sdk_root = AndroidSdk::detect_path(host)
        .or_else(|| derive_sdk_root_from_sdkmanager_path(&sdkmanager_path))
        .ok_or(AndroidToolchainError::SdkRootUndetermined)?;
    Ok((sdkmanager_path, sdk_root))
}

fn prepend_path_entry(
    entry: &Path,
    existing: Option<OsString>,
) -> Result<OsString, AndroidToolchainError> {
    let mut entries = vec![entry.to_path_buf()];
    if let Some(existing) = existing {
        entries.extend(env::split_paths(&existing));
    }
    env::join_paths(entries).map_err(|source| AndroidToolchainError::PathJoin {
        entry: entry.to_path_buf(),
        source,
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

fn proxy_env_value(host: &Host) -> Option<String> {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .into_iter()
    .find_map(|key| {
        host.env_string(key)
            .filter(|value| !value.trim().is_empty())
    })
}

fn parse_sdkmanager_proxy_config(
    proxy: &str,
) -> Result<SdkManagerProxyConfig, AndroidToolchainError> {
    let trimmed = proxy.trim();
    let normalized = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let url = Url::parse(&normalized).map_err(|source| AndroidToolchainError::ProxyParse {
        url: trimmed.to_owned(),
        source,
    })?;
    let host = url
        .host_str()
        .ok_or_else(|| AndroidToolchainError::ProxyMissingHost(trimmed.to_owned()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| AndroidToolchainError::ProxyMissingPort(trimmed.to_owned()))?;
    let proxy_type = match url.scheme() {
        "http" | "https" => SdkManagerProxyType::Http,
        "socks" | "socks5" | "socks5h" => SdkManagerProxyType::Socks,
        scheme => {
            return Err(AndroidToolchainError::ProxyUnsupportedScheme(
                scheme.to_owned(),
            ));
        }
    };
    Ok(SdkManagerProxyConfig {
        proxy_type,
        host,
        port,
    })
}

fn sdkmanager_proxy_args(host: &Host) -> Result<Vec<OsString>, AndroidToolchainError> {
    let Some(proxy) = proxy_env_value(host) else {
        return Ok(Vec::new());
    };
    let proxy = parse_sdkmanager_proxy_config(&proxy)?;
    Ok(vec![
        OsString::from(format!("--proxy={}", proxy.proxy_type.as_flag())),
        OsString::from(format!("--proxy_host={}", proxy.host)),
        OsString::from(format!("--proxy_port={}", proxy.port)),
    ])
}

pub(super) fn java_proxy_properties_from_env(
    host: &Host,
) -> Result<Vec<String>, AndroidToolchainError> {
    let Some(proxy) = proxy_env_value(host) else {
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
    host: &Host,
    args: Vec<OsString>,
    stdin_payload: Option<&str>,
) -> Result<Output, AndroidToolchainError> {
    let (sdkmanager_path, sdk_root) = resolve_sdkmanager_and_root(host).await?;
    let java_home = Java::detect_home(host)
        .await
        .ok_or(AndroidToolchainError::JavaNotFound)?;
    let java_bin = java_home.join("bin");
    let path_env = prepend_path_entry(&java_bin, host.env("PATH").map(OsStr::to_os_string))?;

    let mut sdk_root_arg = OsString::from("--sdk_root=");
    sdk_root_arg.push(&sdk_root);
    let mut full_args = vec![sdk_root_arg];
    full_args.extend(sdkmanager_proxy_args(host)?);
    full_args.extend(args);

    let mut cmd = host.command(&sdkmanager_path);
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
        let mut child = command(&mut cmd).spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(stdin_payload.as_bytes()).await?;
            stdin.flush().await?;
        }
        child.output().await.map_err(AndroidToolchainError::from)
    } else {
        command(&mut cmd)
            .output()
            .await
            .map_err(AndroidToolchainError::from)
    }
}

async fn accept_sdkmanager_licenses(host: &Host) -> Result<(), AndroidToolchainError> {
    let license_input = sdkmanager_confirmation_input();
    let output = run_sdkmanager_output_with_java(
        host,
        vec![OsString::from("--licenses")],
        Some(&license_input),
    )
    .await?;
    if output.status.success() {
        return Ok(());
    }
    Err(AndroidToolchainError::LicenseAcceptance(
        sdkmanager_combined_output(&output),
    ))
}

async fn install_android_sdk_package(
    host: &Host,
    package_id: &str,
) -> Result<(), AndroidToolchainError> {
    let install_args = vec![OsString::from("--install"), OsString::from(package_id)];
    let confirmation_input = sdkmanager_confirmation_input();
    let mut output =
        run_sdkmanager_output_with_java(host, install_args.clone(), Some(&confirmation_input))
            .await?;
    if sdkmanager_requires_license_acceptance(&output) {
        accept_sdkmanager_licenses(host).await?;
        output =
            run_sdkmanager_output_with_java(host, install_args, Some(&confirmation_input)).await?;
    }
    if output.status.success() {
        return Ok(());
    }

    Err(AndroidToolchainError::PackageInstall {
        package_id: package_id.to_owned(),
        output: sdkmanager_combined_output(&output),
    })
}

async fn list_sdk_package_ids(host: &Host) -> Result<Vec<String>, AndroidToolchainError> {
    let output =
        run_sdkmanager_output_with_java(host, vec![OsString::from("--list")], None).await?;
    if !output.status.success() {
        return Err(AndroidToolchainError::PackageList(
            sdkmanager_combined_output(&output),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(parse_sdkmanager_package_id)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>())
}

fn find_file_in_workspace(cwd: &Path, relative_path: &Path) -> Option<PathBuf> {
    let direct = cwd.join(relative_path);
    if direct.exists() {
        return Some(direct);
    }

    let mut current = cwd;
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

fn required_ndk_version_in_workspace(cwd: &Path) -> Option<String> {
    let runtime_build_gradle =
        find_file_in_workspace(cwd, Path::new(ANDROID_RUNTIME_BUILD_GRADLE_RELATIVE_PATH))?;
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

fn ndk_version_from_package_id(package_id: &str) -> Result<&str, AndroidToolchainError> {
    package_id
        .strip_prefix("ndk;")
        .ok_or_else(|| AndroidToolchainError::InvalidNdkPackageId(package_id.to_owned()))
}

fn ndk_path_for_package_id(
    sdk_root: &Path,
    package_id: &str,
) -> Result<PathBuf, AndroidToolchainError> {
    Ok(sdk_root
        .join("ndk")
        .join(ndk_version_from_package_id(package_id)?))
}

fn ndk_layout_is_complete(ndk_path: &Path) -> bool {
    ndk_path.join("toolchains/llvm/prebuilt").exists()
}

async fn remove_directory_if_exists(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    smol::unblock(move || {
        if path.exists() {
            remove_dir_all::remove_dir_all(&path)?;
        }
        Ok(())
    })
    .await
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

fn managed_kotlin_home(host: &Host, version: &str) -> Result<PathBuf, AndroidToolchainError> {
    Ok(water_home_dir_in(host)?
        .join("toolchains/kotlin")
        .join(version))
}

fn kotlin_compiler_release_url(version: &str) -> String {
    format!(
        "https://github.com/JetBrains/kotlin/releases/download/v{version}/kotlin-compiler-{version}.zip"
    )
}

fn find_kotlin_home_dir(root: &Path) -> Result<PathBuf, AndroidToolchainError> {
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

        let bin_dir = path
            .parent()
            .ok_or(AndroidToolchainError::KotlinMissingBinDir)?;
        let kotlin_home = bin_dir
            .parent()
            .ok_or(AndroidToolchainError::KotlinMissingRoot)?;
        return Ok(kotlin_home.to_path_buf());
    }

    Err(AndroidToolchainError::KotlinMissingCompiler(
        executable_name.to_owned(),
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

async fn kotlin_compiler_version(
    host: &Host,
    kotlinc_path: &Path,
) -> Result<String, AndroidToolchainError> {
    let output = host.output(kotlinc_path, ["-version"]).await?;
    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_kotlinc_version_output(&combined).ok_or_else(|| {
        AndroidToolchainError::KotlinVersionParse {
            path: kotlinc_path.to_path_buf(),
            output: combined.trim().to_owned(),
        }
    })
}

async fn install_managed_kotlin_compiler(
    host: &Host,
    version: &str,
) -> Result<PathBuf, AndroidToolchainError> {
    let install_home = managed_kotlin_home(host, version)?;
    if let Some(kotlinc_path) = kotlin_executable_from_home(&install_home)
        && let Ok(installed_version) = kotlin_compiler_version(host, &kotlinc_path).await
        && kotlin_version_is_compatible(&installed_version, version)
    {
        return Ok(kotlinc_path);
    }

    let install_parent = install_home
        .parent()
        .ok_or(AndroidToolchainError::KotlinInstallPathNoParent)?
        .to_path_buf();
    {
        let install_parent = install_parent.clone();
        smol::unblock(move || std::fs::create_dir_all(&install_parent)).await?;
    }

    let temp_dir = {
        let install_parent = install_parent.clone();
        smol::unblock(move || {
            tempfile::Builder::new()
                .prefix(".water-kotlin-")
                .tempdir_in(&install_parent)
        })
        .await?
    };
    let extract_dir = temp_dir.path().join("extract");
    let archive_path = temp_dir
        .path()
        .join(format!("kotlin-compiler-{version}.zip"));
    {
        let extract_dir = extract_dir.clone();
        smol::unblock(move || std::fs::create_dir_all(&extract_dir)).await?;
    }

    download_file_with_redirect(&kotlin_compiler_release_url(version), &archive_path).await?;
    {
        let archive_path = archive_path.clone();
        let extract_dir = extract_dir.clone();
        smol::unblock(move || -> Result<(), AndroidToolchainError> {
            let archive_file = std::fs::File::open(&archive_path)?;
            let mut archive = zip::ZipArchive::new(archive_file)?;
            archive.extract(&extract_dir)?;
            Ok(())
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
        smol::unblock(move || std::fs::rename(extracted_home, install_home)).await?;
    }

    let kotlinc_path = kotlin_executable_from_home(&install_home).ok_or(
        AndroidToolchainError::KotlinCompilerStillMissing {
            version: version.to_owned(),
            executable: kotlinc_binary_name(),
        },
    )?;
    let installed_version = kotlin_compiler_version(host, &kotlinc_path).await?;
    if kotlin_version_is_compatible(&installed_version, version) {
        Ok(kotlinc_path)
    } else {
        Err(AndroidToolchainError::KotlinVersionMismatch {
            installed: installed_version,
            required: version.to_owned(),
        })
    }
}

const fn required_kotlin_version() -> &'static str {
    build_info::ANDROID_KOTLIN_VERSION
}

async fn required_ndk_package_id(host: &Host) -> Result<String, AndroidToolchainError> {
    let runtime_build_gradle = find_file_in_workspace(
        host.cwd(),
        Path::new(ANDROID_RUNTIME_BUILD_GRADLE_RELATIVE_PATH),
    )
    .ok_or(AndroidToolchainError::RuntimeGradleMissing {
        path: ANDROID_RUNTIME_BUILD_GRADLE_RELATIVE_PATH,
    })?;
    let contents = smol::fs::read_to_string(&runtime_build_gradle)
        .await
        .map_err(|source| AndroidToolchainError::RuntimeGradleRead {
            path: runtime_build_gradle.clone(),
            source,
        })?;
    let version =
        parse_android_ndk_version_from_runtime_build_gradle(&contents).ok_or_else(|| {
            AndroidToolchainError::NdkVersionUnparseable {
                path: runtime_build_gradle.clone(),
            }
        })?;
    let package_id = format!("ndk;{version}");
    let available_packages = list_sdk_package_ids(host).await?;
    if available_packages
        .iter()
        .any(|candidate| candidate == &package_id)
    {
        Ok(package_id)
    } else {
        Err(AndroidToolchainError::NdkPackageUnavailable {
            package_id,
            gradle_path: runtime_build_gradle,
        })
    }
}

async fn latest_android_platform_package_id(host: &Host) -> Result<String, AndroidToolchainError> {
    list_sdk_package_ids(host)
        .await?
        .into_iter()
        .filter_map(|package_id| {
            parse_android_platform_api_level(&package_id).map(|api_level| (api_level, package_id))
        })
        .max_by_key(|(api_level, _)| *api_level)
        .map(|(_, package_id)| package_id)
        .ok_or(AndroidToolchainError::NoPlatformPackage)
}

async fn latest_android_build_tools_package_id(
    host: &Host,
) -> Result<String, AndroidToolchainError> {
    let mut build_tools_packages = list_sdk_package_ids(host)
        .await?
        .into_iter()
        .filter(|package_id| parse_android_build_tools_version(package_id).is_some())
        .collect::<Vec<_>>();
    build_tools_packages.sort_by(|left, right| compare_sdk_package_ids(left, right));
    build_tools_packages.dedup();

    build_tools_packages
        .pop()
        .ok_or(AndroidToolchainError::NoBuildToolsPackage)
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

async fn installed_rustup_targets(host: &Host) -> Result<Vec<String>, CommandError> {
    let installed = host
        .run("rustup", ["target", "list", "--installed"])
        .await?;
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
    /// Detect the path to the Android SDK installation on `host`.
    #[must_use]
    pub fn detect_path(host: &Host) -> Option<PathBuf> {
        if let Some(configured) = configured_android_sdk_path(host)
            && configured.exists()
            && looks_like_android_sdk_root(&configured)
        {
            return Some(configured);
        }

        if let Some(sdkmanager_path) = find_sdkmanager_on_host_path(host)
            && let Some(sdk_root) = derive_sdk_root_from_sdkmanager_path(&sdkmanager_path)
            && sdk_root.exists()
            && looks_like_android_sdk_root(&sdk_root)
        {
            return Some(sdk_root);
        }

        None
    }

    /// Detect the highest available `android.jar` from installed SDK platforms on `host`.
    #[must_use]
    pub fn android_jar_path(host: &Host) -> Option<PathBuf> {
        let sdk_root = Self::detect_path(host)?;
        find_android_jar_in_sdk(&sdk_root)
    }

    /// Detect the highest available `d8.jar` from installed SDK build-tools on `host`.
    #[must_use]
    pub fn d8_jar_path(host: &Host) -> Option<PathBuf> {
        let sdk_root = Self::detect_path(host)?;
        find_d8_jar_in_sdk(&sdk_root)
    }

    /// Detect the sdkmanager executable path on `host`.
    pub async fn sdkmanager_path(host: &Host) -> Option<PathBuf> {
        if let Some(sdk_root) = Self::detect_path(host) {
            for candidate in sdkmanager_candidates_under_sdk_root(&sdk_root) {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }

        for name in sdkmanager_search_names() {
            if let Ok(path) = host.which(name).await {
                return Some(path);
            }
        }

        find_sdkmanager_on_host_path(host)
    }

    /// Get the path to the `adb` executable on `host`.
    #[must_use]
    pub fn adb_path(host: &Host) -> Option<PathBuf> {
        let sdk_path = Self::detect_path(host)?;
        let adb = sdk_path
            .join("platform-tools")
            .join(if cfg!(target_os = "windows") {
                "adb.exe"
            } else {
                "adb"
            });
        if adb.exists() { Some(adb) } else { None }
    }

    /// Get the path to the `emulator` executable on `host`.
    #[must_use]
    pub fn emulator_path(host: &Host) -> Option<PathBuf> {
        let sdk_path = Self::detect_path(host)?;
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
    InstallFailed(#[from] AndroidToolchainError),
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

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if Self::detect_path(host).is_some() {
            if Self::sdkmanager_path(host).await.is_some() {
                Ok(())
            } else {
                Err(ToolchainError::fixable(AndroidSdkInstallation))
            }
        } else if cfg!(target_os = "windows") {
            if host.which("winget").await.is_ok() {
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
            if host.which("brew").await.is_ok() {
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
            if configured_android_sdk_path(host).is_some() {
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if cfg!(target_os = "windows") {
            ensure_package_installed(host, "Google.AndroidStudio")
                .await
                .map_err(map_winget_error_for_android_sdk)?;
        } else if cfg!(target_os = "macos") {
            let brew = Brew::default();
            brew.check(host)
                .await
                .map_err(|_| FailToInstallAndroidSdk::BrewNotFound)?;
            brew.install_cask(host, "android-studio")
                .await
                .map_err(|source| {
                    FailToInstallAndroidSdk::InstallFailed(AndroidToolchainError::from(source))
                })?;
        } else if cfg!(target_os = "linux") {
            // Linux CI/headless containers only need command-line tools in the SDK root.
        } else {
            return Err(FailToInstallAndroidSdk::UnsupportedPlatform);
        }

        let sdk_root = configured_android_sdk_path(host)
            .ok_or(AndroidToolchainError::SdkRootUnavailable)
            .map_err(FailToInstallAndroidSdk::InstallFailed)?;
        {
            let sdk_root = sdk_root.clone();
            smol::unblock(move || std::fs::create_dir_all(&sdk_root))
                .await
                .map_err(AndroidToolchainError::from)
                .map_err(FailToInstallAndroidSdk::InstallFailed)?;
        }
        ensure_cmdline_tools_available(&sdk_root)
            .await
            .map_err(FailToInstallAndroidSdk::InstallFailed)?;

        if AndroidSdk::sdkmanager_path(host).await.is_some() {
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
    InstallFailed(#[from] AndroidToolchainError),
    #[error("Failed to install Android x86_64 host-tools compatibility packages: {0}")]
    HostToolsCompatFailed(#[from] LinuxPackageManagerError),
    /// Post-install `adb` verification reported an unhealthy toolchain state.
    #[error("{0}")]
    VerificationFailed(#[from] ToolchainError<AndroidPlatformToolsInstallation>),
    #[error("Android Platform-Tools (`adb`) is still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidPlatformTools {
    type Installation = AndroidPlatformToolsInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if let Some(adb_path) = AndroidSdk::adb_path(host) {
            return verify_android_platform_tools_executable(host, &adb_path).await;
        }

        if AndroidSdk::sdkmanager_path(host).await.is_some() {
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if matches!(self, Self::LinuxX86_64HostToolsCompat) {
            return install_android_linux_x86_64_host_tools_compat(host)
                .await
                .map_err(FailToInstallAndroidPlatformTools::HostToolsCompatFailed);
        }

        if AndroidSdk::sdkmanager_path(host).await.is_none() {
            return Err(FailToInstallAndroidPlatformTools::SdkManagerNotFound);
        }

        install_android_sdk_package(host, "platform-tools")
            .await
            .map_err(FailToInstallAndroidPlatformTools::InstallFailed)?;

        verify_android_platform_tools_after_install(host).await
    }
}

async fn verify_android_platform_tools_after_install(
    host: &Host,
) -> Result<(), FailToInstallAndroidPlatformTools> {
    let adb_path =
        AndroidSdk::adb_path(host).ok_or(FailToInstallAndroidPlatformTools::StillMissing)?;
    match verify_android_platform_tools_executable(host, &adb_path).await {
        Ok(()) => Ok(()),
        Err(ToolchainError::Fixable(
            AndroidPlatformToolsInstallation::LinuxX86_64HostToolsCompat,
        )) => {
            install_android_linux_x86_64_host_tools_compat(host)
                .await
                .map_err(FailToInstallAndroidPlatformTools::HostToolsCompatFailed)?;
            verify_android_platform_tools_executable(host, &adb_path)
                .await
                .map_err(FailToInstallAndroidPlatformTools::VerificationFailed)
        }
        Err(error) => Err(FailToInstallAndroidPlatformTools::VerificationFailed(error)),
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
    InstallFailed(#[from] AndroidToolchainError),
    #[error("Android SDK platforms are still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidSdkPlatforms {
    type Installation = AndroidSdkPlatformsInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if AndroidSdk::android_jar_path(host).is_some() {
            return Ok(());
        }

        if AndroidSdk::sdkmanager_path(host).await.is_some() {
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if AndroidSdk::sdkmanager_path(host).await.is_none() {
            return Err(FailToInstallAndroidSdkPlatforms::SdkManagerNotFound);
        }

        let platform_package = latest_android_platform_package_id(host)
            .await
            .map_err(FailToInstallAndroidSdkPlatforms::InstallFailed)?;
        install_android_sdk_package(host, &platform_package)
            .await
            .map_err(FailToInstallAndroidSdkPlatforms::InstallFailed)?;

        if AndroidSdk::android_jar_path(host).is_some() {
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
    InstallFailed(#[from] AndroidToolchainError),
    #[error("Android SDK build-tools are still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidBuildTools {
    type Installation = AndroidBuildToolsInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if AndroidSdk::d8_jar_path(host).is_some() {
            return Ok(());
        }

        if AndroidSdk::sdkmanager_path(host).await.is_some() {
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if AndroidSdk::sdkmanager_path(host).await.is_none() {
            return Err(FailToInstallAndroidBuildTools::SdkManagerNotFound);
        }

        let build_tools_package = latest_android_build_tools_package_id(host)
            .await
            .map_err(FailToInstallAndroidBuildTools::InstallFailed)?;
        install_android_sdk_package(host, &build_tools_package)
            .await
            .map_err(FailToInstallAndroidBuildTools::InstallFailed)?;

        if AndroidSdk::d8_jar_path(host).is_some() {
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
        source: CommandError,
    },
    #[error("Failed to list installed Rust targets after installation: {0}")]
    QueryTargets(CommandError),
    #[error("Android Rust targets are still missing after installation: {missing_targets}")]
    StillMissing {
        /// Comma-separated missing targets.
        missing_targets: String,
    },
}

impl Toolchain for AndroidRustTargets {
    type Installation = AndroidRustTargetsInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if host.which("rustup").await.is_err() {
            return Err(ToolchainError::unfixable(
                "rustup is not available, so Android Rust targets cannot be managed automatically",
                "Install rustup from https://rustup.rs, then run `water doctor --fix`.",
            ));
        }

        let installed_targets = installed_rustup_targets(host).await.map_err(|error| {
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if host.which("rustup").await.is_err() {
            return Err(FailToInstallAndroidRustTargets::RustupNotFound);
        }

        for target in &self.missing_targets {
            host.run("rustup", ["target", "add", target.as_str()])
                .await
                .map_err(|source| FailToInstallAndroidRustTargets::AddTarget {
                    target: target.clone(),
                    source,
                })?;
        }

        let installed_targets = installed_rustup_targets(host)
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
        assert_eq!(
            missing_android_rust_targets(&installed, &required),
            [] as [String; 0]
        );
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

fn detect_windows_jdk_java_path(host: &Host) -> Option<PathBuf> {
    let program_files = host.env_string("ProgramFiles")?;
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
    host: &Host,
    adb_path: &Path,
) -> Result<(), ToolchainError<AndroidPlatformToolsInstallation>> {
    let output = host.output(adb_path, ["version"]).await.map_err(|error| {
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

/// An `aarch64-linux-android<api>-clang` wrapper from the first NDK prebuilt
/// host toolchain that ships one (its lowest API level, so the probe is
/// deterministic). Every API-level wrapper execs the same `clang`, so one
/// running proves the toolchain executes on this host. This check has no
/// resolved framework to read a floor from; the wrapper for the floor a build
/// targets is required on the build path in `platform.rs`.
fn ndk_host_clang_path(ndk_path: &Path) -> Option<PathBuf> {
    let wrapper_suffix = if cfg!(target_os = "windows") {
        "-clang.cmd"
    } else {
        "-clang"
    };
    let clang_wrapper = |bin_dir: &Path| {
        std::fs::read_dir(bin_dir)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let api_level = entry
                    .file_name()
                    .to_str()?
                    .strip_prefix("aarch64-linux-android")?
                    .strip_suffix(wrapper_suffix)?
                    .parse::<u32>()
                    .ok()?;
                Some((api_level, entry.path()))
            })
            .min_by_key(|(api_level, _)| *api_level)
            .map(|(_, path)| path)
    };

    let prebuilt_dir = ndk_path.join("toolchains/llvm/prebuilt");
    let entries = std::fs::read_dir(&prebuilt_dir).ok()?;
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    candidates.sort();

    candidates
        .iter()
        .find_map(|candidate| clang_wrapper(&candidate.join("bin")))
}

async fn verify_ndk_host_toolchain_executable(
    host: &Host,
    ndk_path: &Path,
) -> Result<(), ToolchainError<AndroidNdkInstallation>> {
    let clang_path = ndk_host_clang_path(ndk_path).ok_or_else(|| {
        ToolchainError::unfixable(
            "Android NDK toolchain is incomplete (no `aarch64-linux-android*-clang` wrapper was found under toolchains/llvm/prebuilt).",
            android_ndk_install_suggestion(),
        )
    })?;

    // Unique scratch source for the compile probe; the `NamedTempFile`
    // deletes itself on drop, including on the early-error paths below.
    let probe_file = smol::unblock(|| -> std::io::Result<tempfile::NamedTempFile> {
        use std::io::Write as _;
        let mut file = tempfile::Builder::new()
            .prefix("waterui-android-ndk-probe-")
            .suffix(".c")
            .tempfile()?;
        file.write_all(b"int main(void) { return 0; }\n")?;
        file.flush()?;
        Ok(file)
    })
    .await
    .map_err(|error| {
        ToolchainError::unfixable(
            format!("Failed to create the Android NDK probe source: {error}"),
            "Ensure the temporary directory is writable, then retry `water doctor`.",
        )
    })?;
    let probe_source = probe_file.path().to_path_buf();

    let probe_output = if cfg!(target_os = "windows") {
        PathBuf::from("NUL")
    } else {
        PathBuf::from("/dev/null")
    };
    let result = host
        .output(
            &clang_path,
            [
                OsString::from("-x"),
                OsString::from("c"),
                OsString::from("-c"),
                probe_source.into_os_string(),
                OsString::from("-o"),
                probe_output.into_os_string(),
            ],
        )
        .await;
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
    /// 3. Java from the host `PATH`
    pub async fn detect_path(host: &Host) -> Option<PathBuf> {
        if cfg!(target_os = "macos") {
            const ANDROID_STUDIO_JBRS: &[&str] = &[
                "Android Studio.app/Contents/jbr/Contents/Home/bin/java",
                "Android Studio Preview.app/Contents/jbr/Contents/Home/bin/java",
            ];
            for app_dir in host.app_dirs() {
                for relative in ANDROID_STUDIO_JBRS {
                    let java_path = app_dir.join(relative);
                    if java_path.exists() {
                        return Some(java_path);
                    }
                }
            }
        }

        if cfg!(target_os = "linux")
            && let Some(home) = host.home_dir()
        {
            let paths = [
                home.join(".local/share/JetBrains/Toolbox/apps/android-studio/jbr/bin/java"),
                home.join("android-studio/jbr/bin/java"),
            ];
            for java_path in paths {
                if java_path.exists() {
                    return Some(java_path);
                }
            }
        }

        if cfg!(target_os = "windows") {
            if let Some(program_files) = host.env_string("ProgramFiles") {
                let java_path =
                    PathBuf::from(&program_files).join("Android/Android Studio/jbr/bin/java.exe");
                if java_path.exists() {
                    return Some(java_path);
                }
            }

            if let Some(java_path) = detect_windows_jdk_java_path(host) {
                return Some(java_path);
            }
        }

        if let Some(home) = host.env_string("JAVA_HOME") {
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

        host.which("java").await.ok()
    }

    /// Get the `JAVA_HOME` directory (parent of `bin/`) on `host`.
    pub async fn detect_home(host: &Host) -> Option<PathBuf> {
        let java_path = Self::detect_path(host).await?;
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
    InstallFailed(#[from] CommandError),
    #[error(
        "Automatic Java installation is not supported on this host. Install a JDK manually and set `JAVA_HOME`."
    )]
    UnsupportedPlatform,
}

impl Toolchain for Java {
    type Installation = JavaInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if Self::detect_path(host).await.is_some() {
            Ok(())
        } else if cfg!(target_os = "windows") {
            if host.which("winget").await.is_ok() {
                Err(ToolchainError::fixable(JavaInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Java runtime not found and winget is unavailable",
                    "Install Microsoft App Installer to provide winget, or install a JDK manually and set `JAVA_HOME`.",
                ))
            }
        } else if cfg!(target_os = "macos") {
            if host.which("brew").await.is_ok() {
                Err(ToolchainError::fixable(JavaInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "Java runtime not found and Homebrew is unavailable",
                    "Install Homebrew to enable automatic fixes, or install a JDK manually and set `JAVA_HOME`.",
                ))
            }
        } else if cfg!(target_os = "linux") {
            if has_supported_package_manager(host).await {
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if cfg!(target_os = "windows") {
            ensure_package_installed(host, "Microsoft.OpenJDK.21")
                .await
                .map_err(map_winget_error_for_java)
        } else if cfg!(target_os = "macos") {
            let brew = Brew::default();
            brew.check(host)
                .await
                .map_err(|_| FailToInstallJava::BrewNotFound)?;
            brew.install_cask(host, "temurin")
                .await
                .map_err(FailToInstallJava::InstallFailed)
        } else if cfg!(target_os = "linux") {
            install_java_jdk(host)
                .await
                .map_err(map_linux_error_for_java)
        } else {
            Err(FailToInstallJava::UnsupportedPlatform)
        }
    }
}

fn map_linux_error_for_java(error: LinuxPackageManagerError) -> FailToInstallJava {
    match error {
        LinuxPackageManagerError::UnsupportedPackageManager => {
            FailToInstallJava::UnsupportedPackageManager
        }
        LinuxPackageManagerError::Command(source) => FailToInstallJava::InstallFailed(source),
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
    /// Detect the path to the kotlinc compiler on `host`.
    pub async fn detect_path(host: &Host) -> Option<PathBuf> {
        let required_version = required_kotlin_version();
        let mut candidates = Vec::new();

        if let Some(home) = host.env_string("KOTLIN_HOME")
            && let Some(kotlinc_path) = kotlin_executable_from_home(&PathBuf::from(&home))
        {
            candidates.push(kotlinc_path);
        }

        if cfg!(target_os = "macos") {
            const ANDROID_STUDIO_KOTLINS: &[&str] = &[
                "Android Studio.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
                "Android Studio Preview.app/Contents/plugins/Kotlin/kotlinc/bin/kotlinc",
            ];
            for app_dir in host.app_dirs() {
                for relative in ANDROID_STUDIO_KOTLINS {
                    let kotlinc_path = app_dir.join(relative);
                    if kotlinc_path.exists() {
                        candidates.push(kotlinc_path);
                    }
                }
            }
        }

        if cfg!(target_os = "linux")
            && let Some(home) = host.home_dir()
        {
            let paths = [
                home.join(
                    ".local/share/JetBrains/Toolbox/apps/android-studio/plugins/Kotlin/kotlinc/bin/kotlinc",
                ),
                home.join("android-studio/plugins/Kotlin/kotlinc/bin/kotlinc"),
            ];
            for kotlinc_path in paths {
                if kotlinc_path.exists() {
                    candidates.push(kotlinc_path);
                }
            }
        }

        if cfg!(target_os = "windows")
            && let Some(program_files) = host.env_string("ProgramFiles")
        {
            let kotlinc_path = PathBuf::from(&program_files)
                .join("Android/Android Studio/plugins/Kotlin/kotlinc/bin/kotlinc.bat");
            if kotlinc_path.exists() {
                candidates.push(kotlinc_path);
            }
        }

        if let Ok(managed_home) = managed_kotlin_home(host, required_version)
            && let Some(kotlinc_path) = kotlin_executable_from_home(&managed_home)
        {
            candidates.push(kotlinc_path);
        }

        if let Ok(path) = host.which("kotlinc").await {
            candidates.push(path);
        }

        candidates.dedup();
        for candidate in candidates {
            let Ok(installed_version) = kotlin_compiler_version(host, &candidate).await else {
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
    InstallFailed(#[from] AndroidToolchainError),
    #[error("Kotlin compiler is still missing after installation.")]
    StillMissing,
}

impl Toolchain for Kotlin {
    type Installation = KotlinInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        let kotlinc_path = Self::detect_path(host)
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        let required_version = required_kotlin_version();
        install_managed_kotlin_compiler(host, required_version)
            .await
            .map_err(FailToInstallKotlin::InstallFailed)?;
        if Kotlin::detect_path(host).await.is_some() {
            Ok(())
        } else {
            Err(FailToInstallKotlin::StillMissing)
        }
    }
}

impl AndroidNdk {
    /// Detect the Android NDK path from `host` environment variables or standard locations.
    #[must_use]
    pub fn detect_path(host: &Host) -> Option<PathBuf> {
        if let Some(ndk_root) = host.env_string("ANDROID_NDK_ROOT") {
            let ndk_path = PathBuf::from(ndk_root);
            if ndk_path.exists() {
                return Some(ndk_path);
            }
        }

        if let Some(ndk_home) = host.env_string("ANDROID_NDK_HOME") {
            let ndk_path = PathBuf::from(ndk_home);
            if ndk_path.exists() {
                return Some(ndk_path);
            }
        }

        let sdk_path = AndroidSdk::detect_path(host)?;
        let ndk_dir = sdk_path.join("ndk");
        select_installed_ndk_path(
            &ndk_dir,
            required_ndk_version_in_workspace(host.cwd()).as_deref(),
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
    InstallFailed(#[from] AndroidToolchainError),
    #[error("Failed to install Android x86_64 host-tools compatibility packages: {0}")]
    HostToolsCompatFailed(#[from] LinuxPackageManagerError),
    /// Post-install NDK verification reported an unhealthy toolchain state.
    #[error("{0}")]
    VerificationFailed(#[from] ToolchainError<AndroidNdkInstallation>),
    #[error("Android NDK is still missing after installation.")]
    StillMissing,
    #[error("Android NDK is installed but incomplete (`toolchains/llvm/prebuilt` is missing).")]
    Incomplete,
}

impl Toolchain for AndroidNdk {
    type Installation = AndroidNdkInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if let Some(ndk_path) = Self::detect_path(host) {
            let llvm_dir = ndk_path.join("toolchains/llvm/prebuilt");
            if llvm_dir.exists() {
                return verify_ndk_host_toolchain_executable(host, &ndk_path).await;
            }

            if AndroidSdk::sdkmanager_path(host).await.is_some() {
                return Err(ToolchainError::fixable(AndroidNdkInstallation::SdkPackage));
            }

            return Err(ToolchainError::unfixable(
                "Android NDK is installed but incomplete",
                android_ndk_install_suggestion(),
            ));
        }

        if AndroidSdk::sdkmanager_path(host).await.is_some() {
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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if matches!(self, Self::LinuxX86_64HostToolsCompat) {
            return install_android_linux_x86_64_host_tools_compat(host)
                .await
                .map_err(FailToInstallAndroidNdk::HostToolsCompatFailed);
        }

        if AndroidSdk::sdkmanager_path(host).await.is_none() {
            return Err(FailToInstallAndroidNdk::SdkManagerNotFound);
        }

        let (_, sdk_root) = resolve_sdkmanager_and_root(host)
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        let ndk_package = required_ndk_package_id(host)
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        let ndk_path = ndk_path_for_package_id(&sdk_root, &ndk_package)
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        if ndk_path.exists() && !ndk_layout_is_complete(&ndk_path) {
            remove_directory_if_exists(&ndk_path)
                .await
                .map_err(AndroidToolchainError::from)
                .map_err(FailToInstallAndroidNdk::InstallFailed)?;
        }
        install_android_sdk_package(host, &ndk_package)
            .await
            .map_err(FailToInstallAndroidNdk::InstallFailed)?;

        if !ndk_path.exists() {
            return Err(FailToInstallAndroidNdk::StillMissing);
        }

        if !ndk_layout_is_complete(&ndk_path) {
            return Err(FailToInstallAndroidNdk::Incomplete);
        }

        verify_android_ndk_after_install(host, &ndk_path).await
    }
}

async fn verify_android_ndk_after_install(
    host: &Host,
    ndk_path: &Path,
) -> Result<(), FailToInstallAndroidNdk> {
    match verify_ndk_host_toolchain_executable(host, ndk_path).await {
        Ok(()) => Ok(()),
        Err(ToolchainError::Fixable(AndroidNdkInstallation::LinuxX86_64HostToolsCompat)) => {
            install_android_linux_x86_64_host_tools_compat(host)
                .await
                .map_err(FailToInstallAndroidNdk::HostToolsCompatFailed)?;
            verify_ndk_host_toolchain_executable(host, ndk_path)
                .await
                .map_err(FailToInstallAndroidNdk::VerificationFailed)
        }
        Err(error) => Err(FailToInstallAndroidNdk::VerificationFailed(error)),
    }
}

#[cfg(test)]
mod host_tests {
    use std::path::Path;

    use super::{
        AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
        AndroidSdkPlatforms, Java, Kotlin, latest_android_platform_package_id,
        parse_android_platform_api_level, parse_android_version_pair, required_kotlin_version,
    };
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Host, Toolchain, ToolchainError};

    /// Host declaring `ANDROID_SDK_ROOT` at `sdk`.
    fn sdk_host(machine: &TestMachine, sdk: &Path) -> Host {
        machine.host([(
            String::from("ANDROID_SDK_ROOT"),
            sdk.as_os_str().to_os_string(),
        )])
    }

    /// Machine with a staged SDK (`cmdline-tools/latest/bin/sdkmanager`) and a
    /// host declaring `ANDROID_SDK_ROOT` at it.
    fn sdk_machine() -> (TestMachine, Host) {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        let host = sdk_host(&machine, &sdk);
        (machine, host)
    }

    // -- #633: minor-versioned platform package identifiers ----------------

    #[test]
    fn platform_api_level_parser_accepts_minor_versioned_packages() {
        assert_eq!(
            parse_android_platform_api_level("platforms;android-37.0"),
            Some((37, 0)),
            "`platforms;android-37.0` must parse to API 37.0 (#633)"
        );
        assert_eq!(
            parse_android_platform_api_level("platforms;android-36"),
            Some((36, 0))
        );
        assert_eq!(
            parse_android_platform_api_level("platforms;android-Tiramisu"),
            None
        );
        assert_eq!(parse_android_platform_api_level("build-tools;37.0.0"), None);
    }

    #[test]
    fn android_version_pair_orders_minor_within_major() {
        assert_eq!(parse_android_version_pair("37.0"), Some((37, 0)));
        assert_eq!(parse_android_version_pair("37.1"), Some((37, 1)));
        assert_eq!(parse_android_version_pair("36"), Some((36, 0)));
        assert_eq!(parse_android_version_pair("android-37"), None);
        assert_eq!(parse_android_version_pair(""), None);
        assert_eq!(parse_android_version_pair("36.1.2"), None);
        // android-36 < android-36.1 < android-37.0 < android-37.1
        assert!(parse_android_version_pair("36") < parse_android_version_pair("36.1"));
        assert!(parse_android_version_pair("36.1") < parse_android_version_pair("37.0"));
        assert!(parse_android_version_pair("37.0") < parse_android_version_pair("37.1"));
    }

    #[test]
    fn sdkmanager_list_prefers_latest_platform_including_minor_versions() {
        let (machine, host) = sdk_machine();
        machine.install("java");
        machine.respond(
            "SDKMANAGER_LIST",
            include_str!("testdata/sdkmanager_list.txt"),
        );
        let package = smol::block_on(latest_android_platform_package_id(&host))
            .expect("sdkmanager --list transcript must yield a platform package");
        assert_eq!(
            package, "platforms;android-37.1",
            "android-37.1 outranks android-37.0 and android-36.1 (#633)"
        );
    }

    #[test]
    fn android_jar_prefers_minor_versioned_platform_dir() {
        // #633: the platform-directory sort is keyed on the same
        // (major, minor) pair, so android-36.1 outranks android-36 and
        // android-37.1 outranks android-37.0 on disk too.
        let (machine, host) = sdk_machine();
        machine.install_android_platform("android-36");
        machine.install_android_platform("android-36.1");
        machine.install_android_platform("android-37.0");
        machine.install_android_platform("android-37.1");
        let jar = AndroidSdk::android_jar_path(&host).expect("a staged platform jar");
        assert_eq!(
            jar.parent().and_then(|dir| dir.file_name()),
            Some(std::ffi::OsStr::new("android-37.1")),
            "the highest (major, minor) platform dir wins: {jar:?}"
        );

        let (machine36, host36) = sdk_machine();
        machine36.install_android_platform("android-36");
        machine36.install_android_platform("android-36.1");
        let jar = AndroidSdk::android_jar_path(&host36).expect("a staged platform jar");
        assert_eq!(
            jar.parent().and_then(|dir| dir.file_name()),
            Some(std::ffi::OsStr::new("android-36.1")),
            "android-36.1 outranks android-36: {jar:?}"
        );
    }

    // -- AndroidSdk --------------------------------------------------------

    #[test]
    fn sdk_detect_path_reads_declared_env() {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        let host = sdk_host(&machine, &sdk);
        assert_eq!(
            AndroidSdk::detect_path(&host).as_deref(),
            Some(sdk.as_path())
        );
    }

    #[test]
    fn sdk_check_ok_when_sdkmanager_present() {
        let (_machine, host) = sdk_machine();
        smol::block_on(AndroidSdk.check(&host)).expect("a staged SDK with sdkmanager must be ok");
    }

    #[test]
    fn sdk_check_fixable_when_sdkmanager_absent() {
        let machine = TestMachine::new();
        // A root that "looks like" an SDK (platform-tools marker) but has no
        // sdkmanager anywhere.
        let sdk = machine.dir("sdk/platform-tools");
        let host = sdk_host(&machine, sdk.parent().expect("sdk root"));
        let result = smol::block_on(AndroidSdk.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "an SDK root without sdkmanager must be fixable: {result:?}"
        );
    }

    #[test]
    fn sdk_missing_classification_matches_platform_installer() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AndroidSdk.check(&host));
        #[cfg(target_os = "linux")]
        {
            // `~/Android/Sdk` under the scratch home counts as a configured
            // root, so Linux always plans the cmdline-tools install.
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "missing SDK on Linux must plan a cmdline-tools install: {result:?}"
            );
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            assert!(
                matches!(result, Err(ToolchainError::Unfixable(_))),
                "missing SDK without brew/winget must be unfixable: {result:?}"
            );
            #[cfg(target_os = "macos")]
            machine.install("brew");
            #[cfg(target_os = "windows")]
            machine.install("winget");
            let host = machine.host(Vec::<(String, String)>::new());
            let result = smol::block_on(AndroidSdk.check(&host));
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "missing SDK with a platform installer must be fixable: {result:?}"
            );
        }
    }

    // -- AndroidPlatformTools ----------------------------------------------

    #[test]
    fn platform_tools_fixable_when_sdkmanager_can_install_it() {
        let (_machine, host) = sdk_machine();
        let result = smol::block_on(AndroidPlatformTools.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing adb with sdkmanager must be fixable: {result:?}"
        );
    }

    #[test]
    fn platform_tools_unfixable_without_sdk() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AndroidPlatformTools.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "no SDK and no sdkmanager must be unfixable: {result:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn platform_tools_ok_when_adb_runs() {
        let (machine, host) = sdk_machine();
        machine.install_adb();
        smol::block_on(AndroidPlatformTools.check(&host))
            .expect("a runnable adb must satisfy platform-tools");
    }

    #[test]
    #[cfg(windows)]
    fn platform_tools_unfixable_when_adb_cannot_spawn() {
        // The staged adb.exe carries cmd text; CreateProcess cannot run it,
        // so the verify branch reports the executable-broken diagnostic.
        let (machine, host) = sdk_machine();
        machine.install_adb();
        let result = smol::block_on(AndroidPlatformTools.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "a non-spawning adb must be unfixable: {result:?}"
        );
    }

    // -- AndroidSdkPlatforms -------------------------------------------------

    #[test]
    fn sdk_platforms_ok_with_android_jar() {
        let (machine, host) = sdk_machine();
        machine.install_android_platform("android-36");
        smol::block_on(AndroidSdkPlatforms.check(&host))
            .expect("an installed android.jar must satisfy the check");
    }

    #[test]
    fn sdk_platforms_ok_with_minor_versioned_platform_dir() {
        // #633: `platforms/android-37.0` is a real layout on disk.
        let (machine, host) = sdk_machine();
        machine.install_android_platform("android-37.0");
        smol::block_on(AndroidSdkPlatforms.check(&host))
            .expect("android-37.0 platform dir must satisfy the check (#633)");
    }

    #[test]
    fn sdk_platforms_fixable_when_sdkmanager_can_install() {
        let (_machine, host) = sdk_machine();
        let result = smol::block_on(AndroidSdkPlatforms.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing platforms with sdkmanager must be fixable: {result:?}"
        );
    }

    #[test]
    fn sdk_platforms_unfixable_without_sdk() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AndroidSdkPlatforms.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "no SDK and no sdkmanager must be unfixable: {result:?}"
        );
    }

    // -- AndroidBuildTools ---------------------------------------------------

    #[test]
    fn build_tools_ok_with_d8_jar() {
        let (machine, host) = sdk_machine();
        machine.install_android_build_tools("36.0.0");
        smol::block_on(AndroidBuildTools.check(&host))
            .expect("an installed d8.jar must satisfy the check");
    }

    #[test]
    fn build_tools_fixable_when_sdkmanager_can_install() {
        let (_machine, host) = sdk_machine();
        let result = smol::block_on(AndroidBuildTools.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing build-tools with sdkmanager must be fixable: {result:?}"
        );
    }

    // -- AndroidRustTargets --------------------------------------------------

    #[test]
    fn rust_targets_unfixable_without_rustup() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AndroidRustTargets::default().check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "no rustup must be unfixable: {result:?}"
        );
    }

    #[test]
    fn rust_targets_ok_when_all_installed() {
        let machine = TestMachine::new();
        machine.install("rustup");
        // `rustup target list --installed` emits one target per line.
        machine.respond(
            "RUSTUP_INSTALLED_TARGETS",
            &[
                "aarch64-linux-android",
                "armv7-linux-androideabi",
                "i686-linux-android",
                "x86_64-linux-android",
            ]
            .join("\n"),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(AndroidRustTargets::default().check(&host))
            .expect("all four Android targets installed must be ok");
    }

    #[test]
    fn rust_targets_fixable_lists_missing_targets() {
        let machine = TestMachine::new();
        machine.install("rustup");
        let host = machine.host([(
            String::from("WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS"),
            String::from("aarch64-linux-android"),
        )]);
        let result = smol::block_on(AndroidRustTargets::default().check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing Android targets must be fixable: {result:?}"
        );
    }

    // -- Java ----------------------------------------------------------------

    #[test]
    fn java_ok_when_on_path() {
        let machine = TestMachine::new();
        machine.install("java");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Java.check(&host)).expect("java on PATH must be ok");
    }

    #[test]
    fn java_missing_is_unfixable_without_installer() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Java.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "no java and no installer must be unfixable: {result:?}"
        );
    }

    #[test]
    fn java_missing_is_fixable_with_installer() {
        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "windows")]
        machine.install("winget");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Java.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "no java with a platform installer must be fixable: {result:?}"
        );
    }

    // -- Kotlin --------------------------------------------------------------

    #[test]
    fn kotlin_ok_when_compatible_kotlinc_on_path() {
        let machine = TestMachine::new();
        machine.install("kotlinc");
        let host = machine.host([(
            String::from("WATERUI_FAKE_KOTLINC_VERSION"),
            required_kotlin_version().to_string(),
        )]);
        smol::block_on(Kotlin.check(&host)).expect("a compatible kotlinc on PATH must be ok");
    }

    #[test]
    fn kotlin_fixable_when_absent() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Kotlin.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "no kotlinc must be fixable via managed install: {result:?}"
        );
    }

    #[test]
    fn kotlin_fixable_when_too_old() {
        let machine = TestMachine::new();
        machine.install("kotlinc");
        let host = machine.host([(
            String::from("WATERUI_FAKE_KOTLINC_VERSION"),
            String::from("1.0.0"),
        )]);
        let result = smol::block_on(Kotlin.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "an incompatible kotlinc must trigger the managed install fix: {result:?}"
        );
    }

    // -- AndroidNdk ----------------------------------------------------------

    #[test]
    fn ndk_fixable_when_sdkmanager_can_install() {
        let (_machine, host) = sdk_machine();
        let result = smol::block_on(AndroidNdk.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing NDK with sdkmanager must be fixable: {result:?}"
        );
    }

    #[test]
    fn ndk_unfixable_without_sdk() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AndroidNdk.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "no SDK and no sdkmanager must be unfixable: {result:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn ndk_ok_when_host_toolchain_runs() {
        let (machine, host) = sdk_machine();
        machine.install_android_ndk("29.0.14206865");
        smol::block_on(AndroidNdk.check(&host))
            .expect("a staged NDK whose clang runs must satisfy the check");
    }
}
