use std::{
    cmp::Ordering,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
};

use color_eyre::eyre;
use futures::StreamExt;
use futures_lite::io::AsyncWriteExt;
use roxmltree::Document;
use smol::{fs, unblock};

use crate::{
    brew::Brew,
    toolchain::{
        Installation, Toolchain, ToolchainError,
        cmake::Cmake,
        linux::{has_supported_package_manager, install_java_jdk},
        winget::{WingetInstallError, ensure_package_installed},
    },
    utils::{run_command_os, run_command_output_os, which},
};

/// Complete Android toolchain including SDK, NDK, platform-tools, Java, and CMake.
pub type AndroidToolchain = (
    AndroidSdk,
    AndroidPlatformSdk,
    AndroidNdk,
    AndroidPlatformTools,
    Java,
    Cmake,
);

/// Android SDK toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidSdk;

/// Android platform SDK component required for `android.jar`.
#[derive(Debug, Clone, Default)]
pub struct AndroidPlatformSdk;

/// Android Platform-Tools (`adb`) toolchain component.
#[derive(Debug, Clone, Default)]
pub struct AndroidPlatformTools;

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

const ANDROID_REPOSITORY_XML_URL: &str =
    "https://dl.google.com/android/repository/repository2-1.xml";
const ANDROID_REPOSITORY_BASE_URL: &str = "https://dl.google.com/android/repository/";
const ANDROID_CMDLINE_TOOLS_PACKAGE_PATH: &str = "cmdline-tools;latest";
const ANDROID_FIX_MAX_LICENSE_RESPONSES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AndroidCmdlineToolsArchive {
    download_url: String,
    size_bytes: u64,
}

const ANDROID_APP_BUILD_GRADLE_TEMPLATE: &str =
    include_str!("../templates/android/app/build.gradle.kts.tpl");

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

fn looks_like_android_sdk_root(path: &Path) -> bool {
    path.join("cmdline-tools").exists()
        || path.join("platform-tools").exists()
        || path.join("platforms").exists()
        || path.join("ndk").exists()
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

fn parse_cmdline_tools_archive(
    repository_xml: &str,
    host_os: &str,
) -> eyre::Result<AndroidCmdlineToolsArchive> {
    let document = Document::parse(repository_xml)
        .map_err(|error| eyre::eyre!("Failed to parse Android repository XML: {error}"))?;

    let package = document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "remotePackage"
                && node.attribute("path") == Some(ANDROID_CMDLINE_TOOLS_PACKAGE_PATH)
        })
        .ok_or_else(|| {
            eyre::eyre!(
                "Android repository XML does not contain `{ANDROID_CMDLINE_TOOLS_PACKAGE_PATH}`"
            )
        })?;

    for archive in package
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "archive")
    {
        let host_matches = archive
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "host-os")
            .and_then(|node| node.text())
            .is_some_and(|value| value == host_os);
        if !host_matches {
            continue;
        }

        let complete = archive
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "complete")
            .ok_or_else(|| eyre::eyre!("Android repository archive missing `<complete>`"))?;
        let relative_url = complete
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "url")
            .and_then(|node| node.text())
            .ok_or_else(|| eyre::eyre!("Android repository archive missing download url"))?;
        let size_bytes = complete
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "size")
            .and_then(|node| node.text())
            .ok_or_else(|| eyre::eyre!("Android repository archive missing download size"))?
            .parse::<u64>()
            .map_err(|error| eyre::eyre!("Invalid Android repository archive size: {error}"))?;
        return Ok(AndroidCmdlineToolsArchive {
            download_url: format!("{ANDROID_REPOSITORY_BASE_URL}{relative_url}"),
            size_bytes,
        });
    }

    Err(eyre::eyre!(
        "Android repository XML does not contain a `{host_os}` archive for `{ANDROID_CMDLINE_TOOLS_PACKAGE_PATH}`"
    ))
}

async fn resolve_cmdline_tools_archive() -> eyre::Result<AndroidCmdlineToolsArchive> {
    use zenwave::{Client, Method, redirect::FollowRedirect};

    let host_os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macosx"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        return Err(eyre::eyre!(
            "Automatic Android SDK installation is not supported on this host"
        ));
    };

    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, ANDROID_REPOSITORY_XML_URL)
        .await
        .map_err(|error| eyre::eyre!("Failed to fetch Android repository metadata: {error}"))?;
    if !response.status().is_success() {
        return Err(eyre::eyre!(
            "Failed to fetch Android repository metadata: HTTP {}",
            response.status()
        ));
    }
    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .map_err(|error| eyre::eyre!("Failed to read Android repository metadata: {error}"))?;
    let repository_xml = String::from_utf8(bytes.to_vec())
        .map_err(|error| eyre::eyre!("Android repository metadata is not valid UTF-8: {error}"))?;
    parse_cmdline_tools_archive(&repository_xml, host_os)
}

async fn download_cmdline_tools_archive(
    archive: &AndroidCmdlineToolsArchive,
    destination: &Path,
) -> eyre::Result<()> {
    use zenwave::{Client, Method, redirect::FollowRedirect};

    let parent = destination.parent().ok_or_else(|| {
        eyre::eyre!(
            "Android cmdline-tools download destination has no parent: {}",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent).await?;

    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, &archive.download_url)
        .await
        .map_err(|error| eyre::eyre!("Failed to download Android cmdline-tools: {error}"))?;
    if !response.status().is_success() {
        return Err(eyre::eyre!(
            "Failed to download Android cmdline-tools: HTTP {}",
            response.status()
        ));
    }

    let expected_length = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let mut body = response.into_body();
    let mut file = fs::File::create(destination).await?;
    let mut bytes_written = 0_u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk
            .map_err(|error| eyre::eyre!("Failed while downloading Android cmdline-tools: {error}"))?;
        file.write_all(&chunk).await?;
        bytes_written = bytes_written.saturating_add(
            u64::try_from(chunk.len()).expect("download chunk length must fit into u64"),
        );
    }
    file.flush().await?;

    if let Some(expected_length) = expected_length
        && expected_length != bytes_written
    {
        return Err(eyre::eyre!(
            "Android cmdline-tools download size mismatch: expected {expected_length} bytes, got {bytes_written}"
        ));
    }
    if bytes_written != archive.size_bytes {
        return Err(eyre::eyre!(
            "Android cmdline-tools archive size mismatch against repository metadata: expected {} bytes, got {bytes_written}",
            archive.size_bytes,
        ));
    }

    Ok(())
}

fn find_extracted_cmdline_tools_dir(root: &Path) -> eyre::Result<PathBuf> {
    let sdkmanager_name = if cfg!(target_os = "windows") {
        "sdkmanager.bat"
    } else {
        "sdkmanager"
    };
    if root.join("bin").join(sdkmanager_name).exists() {
        return Ok(root.to_path_buf());
    }

    for entry in std::fs::read_dir(root)
        .map_err(|error| eyre::eyre!("Failed to scan extracted Android cmdline-tools: {error}"))?
    {
        let entry = entry
            .map_err(|error| eyre::eyre!("Failed to read extracted Android cmdline-tools entry: {error}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Ok(found) = find_extracted_cmdline_tools_dir(&path) {
            return Ok(found);
        }
    }

    Err(eyre::eyre!(
        "Extracted Android cmdline-tools archive does not contain `sdkmanager`"
    ))
}

#[cfg(unix)]
fn ensure_android_cmdline_tools_bin_executable(bin_dir: &Path) -> eyre::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if !bin_dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(bin_dir)
        .map_err(|error| eyre::eyre!("Failed to read Android cmdline-tools bin directory: {error}"))?
    {
        let entry = entry
            .map_err(|error| eyre::eyre!("Failed to inspect Android cmdline-tools bin entry: {error}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let mut permissions = std::fs::metadata(&path)
            .map_err(|error| eyre::eyre!("Failed to read permissions for {}: {error}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).map_err(|error| {
            eyre::eyre!(
                "Failed to mark Android cmdline-tools executable at {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_android_cmdline_tools_bin_executable(_bin_dir: &Path) -> eyre::Result<()> {
    Ok(())
}

async fn install_android_cmdline_tools(sdk_root: &Path) -> eyre::Result<()> {
    let archive = resolve_cmdline_tools_archive().await?;
    fs::create_dir_all(sdk_root).await?;
    let download_dir = sdk_root.join(".water").join("android-sdk");
    fs::create_dir_all(&download_dir).await?;
    let archive_path = download_dir.join("cmdline-tools-latest.zip");
    download_cmdline_tools_archive(&archive, &archive_path).await?;

    let sdk_root = sdk_root.to_path_buf();
    let archive_path_for_extract = archive_path.clone();
    unblock(move || -> eyre::Result<()> {
        let extract_root = sdk_root.join(".water").join("android-sdk").join("extract");
        if extract_root.exists() {
            std::fs::remove_dir_all(&extract_root).map_err(|error| {
                eyre::eyre!(
                    "Failed to reset temporary Android cmdline-tools extraction dir {}: {error}",
                    extract_root.display()
                )
            })?;
        }
        std::fs::create_dir_all(&extract_root).map_err(|error| {
            eyre::eyre!(
                "Failed to create temporary Android cmdline-tools extraction dir {}: {error}",
                extract_root.display()
            )
        })?;

        let archive_file = std::fs::File::open(&archive_path_for_extract).map_err(|error| {
            eyre::eyre!(
                "Failed to open downloaded Android cmdline-tools archive {}: {error}",
                archive_path_for_extract.display()
            )
        })?;
        let mut zip_archive = zip::ZipArchive::new(archive_file)
            .map_err(|error| eyre::eyre!("Failed to open Android cmdline-tools zip archive: {error}"))?;
        zip_archive.extract(&extract_root).map_err(|error| {
            eyre::eyre!(
                "Failed to extract Android cmdline-tools archive into {}: {error}",
                extract_root.display()
            )
        })?;

        let extracted_dir = find_extracted_cmdline_tools_dir(&extract_root)?;
        let cmdline_tools_dir = sdk_root.join("cmdline-tools");
        let latest_dir = cmdline_tools_dir.join("latest");
        if latest_dir.exists() {
            std::fs::remove_dir_all(&latest_dir).map_err(|error| {
                eyre::eyre!(
                    "Failed to replace existing Android cmdline-tools dir {}: {error}",
                    latest_dir.display()
                )
            })?;
        }
        std::fs::create_dir_all(&cmdline_tools_dir).map_err(|error| {
            eyre::eyre!(
                "Failed to create Android cmdline-tools parent dir {}: {error}",
                cmdline_tools_dir.display()
            )
        })?;
        std::fs::rename(&extracted_dir, &latest_dir).map_err(|error| {
            eyre::eyre!(
                "Failed to install Android cmdline-tools into {}: {error}",
                latest_dir.display()
            )
        })?;
        ensure_android_cmdline_tools_bin_executable(&latest_dir.join("bin"))?;
        if extract_root.exists() {
            let _ = std::fs::remove_dir_all(&extract_root);
        }
        Ok(())
    })
    .await?;

    let _ = fs::remove_file(&archive_path).await;
    Ok(())
}

async fn run_command_with_stdin_os<N, A, S>(
    name: N,
    args: A,
    stdin_payload: &[u8],
) -> eyre::Result<String>
where
    N: AsRef<std::ffi::OsStr>,
    A: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let name_ref = name.as_ref();
    let mut command = smol::process::Command::new(name_ref);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(stdin_payload).await?;
        stdin.flush().await?;
    }
    let output = child.output().await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let output_body = if !stderr.is_empty() {
            format!("
stderr:
{stderr}")
        } else if !stdout.is_empty() {
            format!("
stdout:
{stdout}")
        } else {
            String::new()
        };
        let name_display = name_ref.to_string_lossy();
        Err(eyre::eyre!(
            "Command {name_display} failed with status {}{output_body}",
            output.status,
        ))
    }
}

async fn accept_android_sdk_licenses() -> eyre::Result<()> {
    let (sdkmanager_path, sdk_root) = resolve_sdkmanager_and_root().await?;
    let mut responses = Vec::with_capacity(ANDROID_FIX_MAX_LICENSE_RESPONSES * 2);
    for _ in 0..ANDROID_FIX_MAX_LICENSE_RESPONSES {
        responses.extend_from_slice(b"y
");
    }
    let args = vec![
        OsString::from(format!("--sdk_root={}", sdk_root.display())),
        OsString::from("--licenses"),
    ];
    let _ = run_command_with_stdin_os(&sdkmanager_path, args, &responses).await?;
    Ok(())
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

async fn install_android_sdk_package(package_id: &str) -> eyre::Result<()> {
    let (sdkmanager_path, sdk_root) = resolve_sdkmanager_and_root().await?;
    accept_android_sdk_licenses().await?;

    let mut responses = Vec::with_capacity(ANDROID_FIX_MAX_LICENSE_RESPONSES * 2);
    for _ in 0..ANDROID_FIX_MAX_LICENSE_RESPONSES {
        responses.extend_from_slice(b"y
");
    }
    let args = vec![
        OsString::from(format!("--sdk_root={}", sdk_root.display())),
        OsString::from(package_id),
    ];
    run_command_with_stdin_os(&sdkmanager_path, args, &responses).await?;
    Ok(())
}

async fn latest_ndk_package_id() -> eyre::Result<String> {
    let (sdkmanager_path, sdk_root) = resolve_sdkmanager_and_root().await?;
    let args = vec![
        OsString::from(format!("--sdk_root={}", sdk_root.display())),
        OsString::from("--list"),
    ];
    let output = run_command_output_os(&sdkmanager_path, args).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(eyre::eyre!(
            "Failed to list Android SDK packages via sdkmanager. stdout: {} stderr: {}",
            stdout.trim(),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut ndk_packages = stdout
        .lines()
        .filter_map(parse_sdkmanager_package_id)
        .filter(|package| package.starts_with("ndk;"))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    ndk_packages.sort_by(|left, right| compare_sdk_package_ids(left, right));
    ndk_packages.dedup();

    ndk_packages.pop().ok_or_else(|| {
        eyre::eyre!("No installable Android NDK package found via `sdkmanager --list`")
    })
}

fn android_sdk_default_root() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return Some(PathBuf::from(env::var("HOME").ok()?).join("Library/Android/sdk"));
    }
    if cfg!(target_os = "linux") {
        return Some(PathBuf::from(env::var("HOME").ok()?).join("Android/Sdk"));
    }
    if cfg!(target_os = "windows") {
        let localappdata = env::var("LOCALAPPDATA").ok()?;
        return Some(PathBuf::from(localappdata).join("Android/Sdk"));
    }
    None
}

fn android_sdk_install_root() -> Option<PathBuf> {
    for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Ok(raw) = env::var(key) {
            let sdk_path = PathBuf::from(raw);
            if !sdk_path.as_os_str().is_empty() {
                return Some(sdk_path);
            }
        }
    }
    android_sdk_default_root()
}

fn android_compile_sdk_level() -> eyre::Result<u32> {
    ANDROID_APP_BUILD_GRADLE_TEMPLATE
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("compileSdk = "))
        .ok_or_else(|| eyre::eyre!("Android app template is missing `compileSdk = ...`"))?
        .parse::<u32>()
        .map_err(|error| eyre::eyre!("Invalid Android compileSdk value in template: {error}"))
}

fn android_platform_package_id() -> eyre::Result<String> {
    Ok(format!("platforms;android-{}", android_compile_sdk_level()?))
}

impl AndroidPlatformSdk {
    /// Detect the installed Android platform SDK jar required by the app template.
    #[must_use]
    pub fn detect_android_jar() -> Option<PathBuf> {
        let sdk_root = AndroidSdk::detect_path()?;
        let api_level = android_compile_sdk_level().ok()?;
        let android_jar = sdk_root
            .join("platforms")
            .join(format!("android-{api_level}"))
            .join("android.jar");
        if android_jar.exists() {
            Some(android_jar)
        } else {
            None
        }
    }
}

impl AndroidSdk {
    /// Detect the path to the Android SDK installation.
    #[must_use]
    pub fn detect_path() -> Option<PathBuf> {
        for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
            if let Ok(raw) = env::var(key) {
                let sdk_path = PathBuf::from(raw);
                if sdk_path.exists() && looks_like_android_sdk_root(&sdk_path) {
                    return Some(sdk_path);
                }
            }
        }

        if let Some(default_root) = android_sdk_default_root()
            && default_root.exists()
            && looks_like_android_sdk_root(&default_root)
        {
            return Some(default_root);
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
        "Android Studio was installed, but Android SDK was not provisioned yet. Open Android Studio once to complete setup, or install command-line tools and set `ANDROID_SDK_ROOT`."
    )]
    PostInstallSetupRequired,
    #[error(
        "Automatic Android SDK installation is only supported on macOS, Windows, and Linux command-line tools hosts."
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
        } else if cfg!(target_os = "linux") {
            Err(ToolchainError::fixable(AndroidSdkInstallation))
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
            let sdk_root = android_sdk_install_root().ok_or_else(|| {
                FailToInstallAndroidSdk::InstallFailed(eyre::eyre!(
                    "Unable to determine Android SDK installation root. {}",
                    android_sdk_path_suggestion()
                ))
            })?;
            install_android_cmdline_tools(&sdk_root)
                .await
                .map_err(FailToInstallAndroidSdk::InstallFailed)?;
        } else {
            return Err(FailToInstallAndroidSdk::UnsupportedPlatform);
        }

        if cfg!(target_os = "linux") {
            if AndroidSdk::sdkmanager_path().await.is_some() {
                Ok(())
            } else {
                Err(FailToInstallAndroidSdk::InstallFailed(eyre::eyre!(
                    "Android SDK command-line tools were installed, but `sdkmanager` was not detected. {}",
                    android_sdk_path_suggestion()
                )))
            }
        } else if AndroidSdk::detect_path().is_some() {
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

/// Installation procedure for the Android platform SDK required by the app template.
#[derive(Debug, Clone, Default)]
pub struct AndroidPlatformSdkInstallation;

/// Errors that can occur when installing the Android platform SDK.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallAndroidPlatformSdk {
    #[error("Android SDK command-line tools (`sdkmanager`) not found.")]
    SdkManagerNotFound,
    #[error("Failed to determine required Android platform package: {0}")]
    MissingPackageId(eyre::Report),
    #[error("Failed to install Android platform SDK via sdkmanager: {0}")]
    InstallFailed(eyre::Report),
    #[error("Android platform SDK is still missing after installation.")]
    StillMissing,
}

impl Toolchain for AndroidPlatformSdk {
    type Installation = AndroidPlatformSdkInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if Self::detect_android_jar().is_some() {
            return Ok(());
        }

        if AndroidSdk::sdkmanager_path().await.is_some() {
            Err(ToolchainError::fixable(AndroidPlatformSdkInstallation))
        } else {
            Err(ToolchainError::unfixable(
                "Android platform SDK not found",
                format!(
                    "Install `{}` with sdkmanager, or run `water doctor --fix` after Android SDK command-line tools are ready.",
                    android_platform_package_id().unwrap_or_else(|_| String::from("platforms;android-<api>"))
                ),
            ))
        }
    }
}

impl Installation for AndroidPlatformSdkInstallation {
    type Error = FailToInstallAndroidPlatformSdk;

    async fn install(&self) -> Result<(), Self::Error> {
        if AndroidSdk::sdkmanager_path().await.is_none() {
            return Err(FailToInstallAndroidPlatformSdk::SdkManagerNotFound);
        }
        let package_id = android_platform_package_id()
            .map_err(FailToInstallAndroidPlatformSdk::MissingPackageId)?;
        install_android_sdk_package(&package_id)
            .await
            .map_err(FailToInstallAndroidPlatformSdk::InstallFailed)?;
        AndroidPlatformSdk::detect_android_jar()
            .ok_or(FailToInstallAndroidPlatformSdk::StillMissing)?;
        Ok(())
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
        }

        if let Ok(home) = env::var("JAVA_HOME") {
            let java_path = PathBuf::from(home).join("bin/java");
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

async fn ndk_host_clang_path(ndk_path: &Path) -> Option<PathBuf> {
    let prebuilt_dir = ndk_path.join("toolchains/llvm/prebuilt");
    let entries = std::fs::read_dir(&prebuilt_dir).ok()?;
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    candidates.sort();

    for candidate in candidates {
        let clang = candidate.join("bin").join(if cfg!(target_os = "windows") {
            "aarch64-linux-android24-clang.cmd"
        } else {
            "aarch64-linux-android24-clang"
        });
        if clang.exists() {
            return Some(clang);
        }
    }

    None
}

async fn verify_ndk_host_toolchain_executable(
    ndk_path: &Path,
) -> Result<(), ToolchainError<AndroidNdkInstallation>> {
    let clang_path = ndk_host_clang_path(ndk_path).await.ok_or_else(|| {
        ToolchainError::unfixable(
            "Android NDK toolchain is incomplete (`clang` was not found under toolchains/llvm/prebuilt).",
            android_ndk_install_suggestion(),
        )
    })?;

    let probe_source = std::env::temp_dir().join("waterui_android_ndk_probe.c");
    fs::write(&probe_source, b"int main(void) { return 0; }
")
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
    let _ = fs::remove_file(&probe_source).await;
    result.map_err(|error| {
        let detail = error.to_string();
        let suggestion = if detail.contains("ld-linux-x86-64.so.2") {
            String::from(
                "This host cannot execute the NDK's x86_64 Linux toolchain binaries. Use an x86_64 Linux machine/VM, or install x86_64 userland emulation/runtime support so the Android clang executable can run.",
            )
        } else {
            format!(
                "Ensure the Android NDK toolchain binaries under `{}` can execute on this host, then retry packaging.",
                clang_path.display()
            )
        };
        ToolchainError::unfixable(
            format!(
                "Android NDK toolchain exists but failed to execute on this host: {detail}"
            ),
            suggestion,
        )
    })?;

    Ok(())
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
                return verify_ndk_host_toolchain_executable(&ndk_path).await;
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


#[cfg(test)]
mod tests {
    use super::{
        ANDROID_APP_BUILD_GRADLE_TEMPLATE, ANDROID_CMDLINE_TOOLS_PACKAGE_PATH,
        AndroidCmdlineToolsArchive, android_compile_sdk_level, parse_cmdline_tools_archive,
    };

    #[test]
    fn parses_latest_linux_cmdline_tools_archive_from_repository_xml() {
        let xml = r#"
            <sdk:sdk-repository xmlns:sdk="http://schemas.android.com/sdk/android/repo/repository2/01">
              <remotePackage path="cmdline-tools;latest">
                <archives>
                  <archive>
                    <host-os>linux</host-os>
                    <complete>
                      <size>42</size>
                      <url>commandlinetools-linux-latest.zip</url>
                    </complete>
                  </archive>
                </archives>
              </remotePackage>
            </sdk:sdk-repository>
        "#;

        let archive = parse_cmdline_tools_archive(xml, "linux")
            .expect("linux cmdline-tools archive must parse");
        assert_eq!(
            archive,
            AndroidCmdlineToolsArchive {
                download_url: String::from(
                    "https://dl.google.com/android/repository/commandlinetools-linux-latest.zip"
                ),
                size_bytes: 42,
            }
        );
    }

    #[test]
    fn parse_cmdline_tools_archive_rejects_missing_latest_package() {
        let xml = r#"
            <sdk:sdk-repository xmlns:sdk="http://schemas.android.com/sdk/android/repo/repository2/01">
              <remotePackage path="cmdline-tools;19.0" />
            </sdk:sdk-repository>
        "#;

        let error = parse_cmdline_tools_archive(xml, "linux")
            .expect_err("missing latest package must fail");
        assert!(error.to_string().contains(ANDROID_CMDLINE_TOOLS_PACKAGE_PATH));
    }

    #[test]
    fn parses_compile_sdk_level_from_android_template() {
        assert!(ANDROID_APP_BUILD_GRADLE_TEMPLATE.contains("compileSdk = 35"));
        assert_eq!(
            android_compile_sdk_level().expect("compileSdk must parse from template"),
            35
        );
    }
}
