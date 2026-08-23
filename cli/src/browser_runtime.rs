use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, Context as _, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol::fs;
use walkdir::WalkDir;
use zenwave::{Client as _, Method, redirect::FollowRedirect};

use crate::platform::TargetPlatform;
use crate::project::{BrowserRuntimePlan, ResolvedWebViewBackend};

const SOURCE_CONFIG: &str = include_str!("browser_runtime.toml");
const RUNTIME_ROOT: &str = "waterui-browser";
const CEF_FRAMEWORK_NAME: &str = "Chromium Embedded Framework.framework";

struct RuntimeLayout {
    executable_directory: PathBuf,
    runtime_root: PathBuf,
    macos_frameworks_directory: Option<PathBuf>,
}

impl RuntimeLayout {
    fn adjacent(executable_directory: &Path) -> Self {
        let runtime_root = executable_directory.join(RUNTIME_ROOT);
        Self {
            executable_directory: executable_directory.to_path_buf(),
            #[cfg(target_os = "macos")]
            macos_frameworks_directory: Some(runtime_root.join(RuntimeEngine::Cef.as_str())),
            #[cfg(not(target_os = "macos"))]
            macos_frameworks_directory: None,
            runtime_root,
        }
    }

    #[cfg(target_os = "macos")]
    fn macos_app(contents_directory: &Path) -> Self {
        Self {
            executable_directory: contents_directory.join("MacOS"),
            runtime_root: contents_directory.join("Resources").join(RUNTIME_ROOT),
            macos_frameworks_directory: Some(contents_directory.join("Frameworks")),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SourceConfiguration {
    release_base_url: String,
    wpe_version: String,
    cef_version: String,
    cef_sbom_created: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactManifest {
    schema_version: u32,
    artifacts: Vec<RuntimeArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeArtifact {
    engine: RuntimeEngine,
    version: String,
    platform: String,
    architecture: String,
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct CefArchiveMetadata {
    name: String,
    sha1: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum RuntimeEngine {
    Wpe,
    Cef,
}

impl RuntimeEngine {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Wpe => "wpe",
            Self::Cef => "cef",
        }
    }
}

#[derive(Debug, Serialize)]
struct RuntimeIdentity<'a> {
    schema_version: u32,
    engine: &'a str,
    version: &'a str,
    platform: &'a str,
    architecture: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpdxDocument<'a> {
    spdx_version: &'a str,
    data_license: &'a str,
    spdx_id: &'a str,
    name: String,
    document_namespace: String,
    creation_info: SpdxCreationInfo<'a>,
    packages: [SpdxPackage<'a>; 1],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpdxCreationInfo<'a> {
    created: &'a str,
    creators: [&'a str; 1],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpdxPackage<'a> {
    name: &'a str,
    spdx_id: &'a str,
    version_info: &'a str,
    download_location: &'a str,
    files_analyzed: bool,
    license_concluded: &'a str,
    license_declared: &'a str,
    copyright_text: &'a str,
}

/// Stages exactly the browser runtimes selected by the resolved application graph.
///
/// `profile_directory` is the Cargo profile directory containing the executable.
/// `executable_directory` is the final directory from which the application runs.
pub async fn stage(
    plan: BrowserRuntimePlan,
    platform: TargetPlatform,
    profile_directory: &Path,
    executable_directory: &Path,
) -> eyre::Result<()> {
    stage_into(
        plan,
        platform,
        profile_directory,
        RuntimeLayout::adjacent(executable_directory),
    )
    .await
}

/// Stages browser runtimes into their canonical macOS application-bundle locations.
///
/// Runtime metadata is stored in `Contents/Resources`, while the CEF framework
/// is stored in `Contents/Frameworks` so the bundle can be signed without
/// treating data files as nested code.
///
/// # Errors
///
/// Returns an error when a selected runtime cannot be resolved, validated, or
/// copied into the application bundle.
#[cfg(target_os = "macos")]
pub async fn stage_macos_app(
    plan: BrowserRuntimePlan,
    profile_directory: &Path,
    contents_directory: &Path,
) -> eyre::Result<()> {
    stage_into(
        plan,
        TargetPlatform::MacOS,
        profile_directory,
        RuntimeLayout::macos_app(contents_directory),
    )
    .await
}

/// Removes browser runtime files previously staged into a macOS application.
///
/// Xcode owns the application bundle while it builds and validates it. The CLI
/// therefore removes its post-build runtime additions before an incremental
/// Xcode build and stages the selected runtime again after that build succeeds.
///
/// # Errors
///
/// Returns an error when previously staged runtime files cannot be removed.
#[cfg(target_os = "macos")]
pub async fn remove_macos_app(contents_directory: &Path) -> eyre::Result<()> {
    remove_staged(&RuntimeLayout::macos_app(contents_directory)).await
}

async fn stage_into(
    plan: BrowserRuntimePlan,
    platform: TargetPlatform,
    profile_directory: &Path,
    layout: RuntimeLayout,
) -> eyre::Result<()> {
    remove_staged(&layout).await?;
    let engines = engines(plan);
    if engines.is_empty() {
        return Ok(());
    }

    let configuration: SourceConfiguration =
        toml::from_str(SOURCE_CONFIG).wrap_err("invalid browser runtime source configuration")?;
    fs::create_dir_all(&layout.runtime_root).await?;

    if engines.contains(&RuntimeEngine::Wpe) {
        stage_wpe(&configuration, platform, &layout.runtime_root).await?;
    }
    if engines.contains(&RuntimeEngine::Cef) {
        stage_cef(&configuration, platform, profile_directory, &layout).await?;
    }
    Ok(())
}

fn engines(plan: BrowserRuntimePlan) -> BTreeSet<RuntimeEngine> {
    let mut engines = BTreeSet::new();
    match plan.webview {
        Some(ResolvedWebViewBackend::Wpe) => {
            engines.insert(RuntimeEngine::Wpe);
        }
        Some(ResolvedWebViewBackend::Cef) => {
            engines.insert(RuntimeEngine::Cef);
        }
        Some(ResolvedWebViewBackend::System) | None => {}
    }
    if plan.chromium {
        engines.insert(RuntimeEngine::Cef);
    }
    engines
}

async fn stage_wpe(
    configuration: &SourceConfiguration,
    platform: TargetPlatform,
    runtime_root: &Path,
) -> eyre::Result<()> {
    if platform != TargetPlatform::Linux {
        bail!("WPE runtime staging is Linux-only");
    }
    let architecture = target_architecture()?;
    let manifest_url = runtime_manifest_url(configuration);
    let manifest = download_manifest(&manifest_url).await?;
    let artifact = select_artifact(
        &manifest,
        RuntimeEngine::Wpe,
        &configuration.wpe_version,
        platform,
        architecture,
    )?;
    let archive = cache_artifact(artifact).await?;
    let destination = runtime_root.join(RuntimeEngine::Wpe.as_str());
    extract_verified_zip(&archive, &artifact.sha256, &destination).await?;
    validate_wpe_runtime(&destination)?;
    Ok(())
}

async fn stage_cef(
    configuration: &SourceConfiguration,
    platform: TargetPlatform,
    profile_directory: &Path,
    layout: &RuntimeLayout,
) -> eyre::Result<()> {
    let architecture = target_architecture()?;
    let source = find_cef_distribution(
        profile_directory,
        platform,
        architecture,
        &configuration.cef_version,
    )?;
    let metadata_root = layout.runtime_root.join(RuntimeEngine::Cef.as_str());
    fs::create_dir_all(metadata_root.join("licenses")).await?;
    let staged_files = if platform == TargetPlatform::MacOS {
        let framework = source.join(CEF_FRAMEWORK_NAME);
        if !framework.is_dir() {
            bail!("CEF distribution is missing {}", framework.display());
        }
        let frameworks_directory = layout
            .macos_frameworks_directory
            .as_deref()
            .ok_or_else(|| eyre::eyre!("macOS CEF staging requires a Frameworks directory"))?;
        fs::create_dir_all(frameworks_directory).await?;
        copy_directory(&framework, &frameworks_directory.join(CEF_FRAMEWORK_NAME)).await?;
        Vec::new()
    } else {
        copy_cef_flat_runtime(&source, &layout.executable_directory).await?
    };

    let credits = source.join("CREDITS.html");
    if !credits.is_file() {
        bail!("CEF distribution is missing {}", credits.display());
    }
    fs::copy(&credits, metadata_root.join("licenses/CREDITS.html")).await?;
    write_json(
        &metadata_root.join("runtime.json"),
        &RuntimeIdentity {
            schema_version: 1,
            engine: "cef",
            version: &configuration.cef_version,
            platform: platform_name(platform)?,
            architecture,
        },
    )
    .await?;
    write_cef_sbom(
        &metadata_root.join("sbom.spdx.json"),
        &configuration.cef_version,
        &configuration.cef_sbom_created,
    )
    .await?;
    write_json(&metadata_root.join("runtime-files.json"), &staged_files).await?;
    validate_cef_runtime(
        platform,
        &layout.executable_directory,
        layout.macos_frameworks_directory.as_deref(),
    )?;
    Ok(())
}

async fn remove_staged(layout: &RuntimeLayout) -> eyre::Result<()> {
    let inventory = layout.runtime_root.join("cef/runtime-files.json");
    if inventory.is_file() {
        let bytes = fs::read(&inventory).await?;
        let paths: Vec<PathBuf> =
            serde_json::from_slice(&bytes).wrap_err("invalid staged CEF runtime inventory")?;
        for relative in paths {
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| component == std::path::Component::ParentDir)
            {
                bail!(
                    "staged CEF inventory contains unsafe path {}",
                    relative.display()
                );
            }
            let path = layout.executable_directory.join(relative);
            if path.is_dir() {
                fs::remove_dir_all(path).await?;
            } else if path.exists() {
                fs::remove_file(path).await?;
            }
        }
    }
    if layout.runtime_root.exists() {
        fs::remove_dir_all(&layout.runtime_root).await?;
    }
    if let Some(frameworks_directory) = &layout.macos_frameworks_directory {
        let framework = frameworks_directory.join(CEF_FRAMEWORK_NAME);
        if framework.exists() {
            fs::remove_dir_all(framework).await?;
        }
    }
    Ok(())
}

async fn download_manifest(url: &str) -> eyre::Result<ArtifactManifest> {
    let mut client = FollowRedirect::new(zenwave::raw_client());
    let response = client.method(Method::GET, url)?.await?;
    if !response.status().is_success() {
        bail!(
            "browser runtime manifest returned HTTP {} from {url}",
            response.status()
        );
    }
    let bytes = response.into_body().into_bytes().await?;
    let manifest: ArtifactManifest =
        serde_json::from_slice(&bytes).wrap_err("invalid browser runtime artifact manifest")?;
    if manifest.schema_version != 1 {
        bail!(
            "unsupported browser runtime manifest schema {}",
            manifest.schema_version
        );
    }
    Ok(manifest)
}

fn select_artifact<'a>(
    manifest: &'a ArtifactManifest,
    engine: RuntimeEngine,
    version: &str,
    platform: TargetPlatform,
    architecture: &str,
) -> eyre::Result<&'a RuntimeArtifact> {
    let platform = platform_name(platform)?;
    let matches = manifest
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.engine == engine
                && artifact.version == version
                && artifact.platform == platform
                && artifact.architecture == architecture
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [artifact] => Ok(*artifact),
        [] => Err(eyre::eyre!(
            "browser runtime manifest has no {} {version} artifact for {platform}/{architecture}",
            engine.as_str()
        )),
        _ => Err(eyre::eyre!(
            "browser runtime manifest contains duplicate {} {version} artifacts for {platform}/{architecture}",
            engine.as_str()
        )),
    }
}

async fn cache_artifact(artifact: &RuntimeArtifact) -> eyre::Result<PathBuf> {
    let cache_root = dirs::cache_dir()
        .ok_or_else(|| eyre::eyre!("platform does not expose a cache directory"))?
        .join("waterui/browser-runtimes")
        .join(artifact.engine.as_str())
        .join(&artifact.version)
        .join(&artifact.platform)
        .join(&artifact.architecture);
    fs::create_dir_all(&cache_root).await?;
    let archive = cache_root.join(format!("{}.zip", artifact.sha256));
    if archive.is_file()
        && file_sha256(&archive).await? == artifact.sha256
        && fs::metadata(&archive).await?.len() == artifact.size
    {
        return Ok(archive);
    }
    if archive.exists() {
        fs::remove_file(&archive).await?;
    }

    let partial = archive.with_extension("zip.partial");
    let mut client = FollowRedirect::new(zenwave::raw_client());
    client
        .method(Method::GET, &artifact.url)?
        .download_to_path(&partial)
        .await
        .wrap_err_with(|| format!("failed to download browser runtime {}", artifact.url))?;
    let size = fs::metadata(&partial).await?.len();
    if size != artifact.size {
        fs::remove_file(&partial).await?;
        bail!(
            "browser runtime size mismatch for {}: expected {}, received {size}",
            artifact.url,
            artifact.size
        );
    }
    let checksum = file_sha256(&partial).await?;
    if checksum != artifact.sha256 {
        fs::remove_file(&partial).await?;
        bail!(
            "browser runtime SHA-256 mismatch for {}: expected {}, received {checksum}",
            artifact.url,
            artifact.sha256
        );
    }
    fs::rename(partial, &archive).await?;
    Ok(archive)
}

async fn file_sha256(path: &Path) -> eyre::Result<String> {
    let path = path.to_path_buf();
    smol::unblock(move || {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok::<_, std::io::Error>(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(Into::into)
}

async fn extract_verified_zip(
    archive: &Path,
    expected_sha256: &str,
    destination: &Path,
) -> eyre::Result<()> {
    let actual_sha256 = file_sha256(archive).await?;
    if actual_sha256 != expected_sha256 {
        bail!(
            "cached browser runtime SHA-256 mismatch: expected {expected_sha256}, received {actual_sha256}"
        );
    }
    if destination.exists() {
        fs::remove_dir_all(destination).await?;
    }
    let archive = archive.to_path_buf();
    let destination = destination.to_path_buf();
    smol::unblock(move || extract_zip(&archive, &destination)).await
}

fn extract_zip(archive: &Path, destination: &Path) -> eyre::Result<()> {
    std::fs::create_dir_all(destination)?;
    let file = File::open(archive)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| eyre::eyre!("browser runtime archive contains an unsafe path"))?;
        let output = destination.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        let parent = output
            .parent()
            .expect("browser runtime archive path must have a parent");
        std::fs::create_dir_all(parent)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            let mut target = String::new();
            entry.read_to_string(&mut target)?;
            let target = PathBuf::from(target);
            if target.is_absolute()
                || target
                    .components()
                    .any(|component| component == std::path::Component::ParentDir)
            {
                bail!(
                    "browser runtime archive contains an unsafe symbolic link {} -> {}",
                    relative.display(),
                    target.display()
                );
            }
            // Each arm owns its own control flow: the `bail!` below diverges, so
            // a shared `continue` after it is dead code on that platform.
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(target, output)?;
                continue;
            }
            #[cfg(not(unix))]
            bail!("browser runtime archive contains a symbolic link on an unsupported host");
        }
        let mut file = File::create(&output)?;
        std::io::copy(&mut entry, &mut file)?;
        file.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

fn find_cef_distribution(
    profile_directory: &Path,
    platform: TargetPlatform,
    architecture: &str,
    version: &str,
) -> eyre::Result<PathBuf> {
    let build_root = profile_directory.join("build");
    let expected_directory = cef_distribution_directory(platform, architecture)?;
    let mut matches = Vec::new();
    for entry in WalkDir::new(&build_root).max_depth(5) {
        let entry = entry?;
        if !entry.file_type().is_file() || entry.file_name() != "archive.json" {
            continue;
        }
        let Some(parent) = entry.path().parent() else {
            continue;
        };
        if parent.file_name().and_then(|name| name.to_str()) != Some(expected_directory) {
            continue;
        }
        let metadata: CefArchiveMetadata = serde_json::from_slice(&std::fs::read(entry.path())?)
            .wrap_err_with(|| format!("invalid CEF archive metadata {}", entry.path().display()))?;
        if metadata.name.contains(version) {
            matches.push((metadata, parent.to_path_buf()));
        }
    }
    matches.sort();
    matches.dedup();
    let identities = matches
        .iter()
        .map(|(metadata, _)| metadata)
        .collect::<BTreeSet<_>>();
    match (identities.len(), matches.first()) {
        (1, Some((_, path))) => Ok(path.clone()),
        (0, None) => Err(eyre::eyre!(
            "CEF {version} build distribution `{expected_directory}` was not found under {}",
            build_root.display()
        )),
        _ => Err(eyre::eyre!(
            "conflicting CEF {version} distributions `{expected_directory}` were found under {}",
            build_root.display()
        )),
    }
}

fn cef_distribution_directory(
    platform: TargetPlatform,
    architecture: &str,
) -> eyre::Result<&'static str> {
    match (platform, architecture) {
        (TargetPlatform::MacOS, "aarch64") => Ok("cef_macos_aarch64"),
        (TargetPlatform::MacOS, "x86_64") => Ok("cef_macos_x86_64"),
        (TargetPlatform::Linux, "aarch64") => Ok("cef_linux_aarch64"),
        (TargetPlatform::Linux, "x86_64") => Ok("cef_linux_x86_64"),
        (TargetPlatform::Windows, "aarch64") => Ok("cef_windows_aarch64"),
        (TargetPlatform::Windows, "x86_64") => Ok("cef_windows_x86_64"),
        _ => Err(eyre::eyre!(
            "CEF does not publish a runtime for {platform:?}/{architecture}"
        )),
    }
}

async fn copy_cef_flat_runtime(source: &Path, destination: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut staged = Vec::new();
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if path.is_dir() {
            if matches!(name_text.as_ref(), "locales" | "swiftshader") {
                copy_directory(&path, &destination.join(&name)).await?;
                staged.push(PathBuf::from(name));
            }
            continue;
        }
        if matches!(
            name_text.as_ref(),
            "archive.json" | "CMakeLists.txt" | "CREDITS.html"
        ) {
            continue;
        }
        fs::copy(&path, destination.join(&name)).await?;
        staged.push(PathBuf::from(name));
    }
    if staged.is_empty() {
        bail!(
            "CEF distribution at {} has no runtime files",
            source.display()
        );
    }
    Ok(staged)
}

async fn copy_directory(source: &Path, destination: &Path) -> eyre::Result<()> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    smol::unblock(move || {
        if destination.exists() {
            std::fs::remove_dir_all(&destination)?;
        }
        std::fs::create_dir_all(&destination)?;
        let options = fs_extra::dir::CopyOptions::new().content_only(true);
        fs_extra::dir::copy(source, destination, &options)?;
        Ok::<_, eyre::Report>(())
    })
    .await
}

fn validate_wpe_runtime(root: &Path) -> eyre::Result<()> {
    for required in ["lib/libwaterui_wpe.so", "runtime.json", "sbom.spdx.json"] {
        let path = root.join(required);
        if !path.is_file() {
            bail!("WPE artifact is missing {}", path.display());
        }
    }
    if !root.join("licenses").is_dir() {
        bail!("WPE artifact is missing its licenses directory");
    }
    Ok(())
}

fn validate_cef_runtime(
    platform: TargetPlatform,
    executable_directory: &Path,
    macos_frameworks_directory: Option<&Path>,
) -> eyre::Result<()> {
    let library = match platform {
        TargetPlatform::MacOS => macos_frameworks_directory
            .ok_or_else(|| eyre::eyre!("macOS CEF validation requires a Frameworks directory"))?
            .join(CEF_FRAMEWORK_NAME)
            .join("Chromium Embedded Framework"),
        TargetPlatform::Linux => executable_directory.join("libcef.so"),
        TargetPlatform::Windows => executable_directory.join("libcef.dll"),
        _ => return Err(eyre::eyre!("CEF runtime is unsupported for {platform:?}")),
    };
    if !library.is_file() {
        bail!(
            "staged CEF runtime library is missing at {}",
            library.display()
        );
    }
    let resources = if platform == TargetPlatform::MacOS {
        macos_frameworks_directory
            .expect("macOS Frameworks directory validated above")
            .join(CEF_FRAMEWORK_NAME)
            .join("Resources")
    } else {
        executable_directory.to_path_buf()
    };
    let icu = resources.join("icudtl.dat");
    if !icu.is_file() {
        bail!("staged CEF ICU data is missing at {}", icu.display());
    }
    if platform == TargetPlatform::MacOS {
        let locale = resources.join("en.lproj/locale.pak");
        if !locale.is_file() {
            bail!(
                "staged CEF macOS locale pack is missing at {}",
                locale.display()
            );
        }
    } else {
        let locales = resources.join("locales");
        if !locales.is_dir() {
            bail!(
                "staged CEF locales directory is missing at {}",
                locales.display()
            );
        }
    }
    Ok(())
}

async fn write_cef_sbom(path: &Path, version: &str, created: &str) -> eyre::Result<()> {
    write_json(
        path,
        &SpdxDocument {
            spdx_version: "SPDX-2.3",
            data_license: "CC0-1.0",
            spdx_id: "SPDXRef-DOCUMENT",
            name: format!("WaterUI CEF runtime {version}"),
            document_namespace: format!("https://waterui.dev/sbom/cef/{version}"),
            creation_info: SpdxCreationInfo {
                created,
                creators: ["Tool: waterui-cli"],
            },
            packages: [SpdxPackage {
                name: "Chromium Embedded Framework",
                spdx_id: "SPDXRef-Package-CEF",
                version_info: version,
                download_location: "https://cef-builds.spotifycdn.com/",
                files_analyzed: false,
                license_concluded: "NOASSERTION",
                license_declared: "BSD-3-Clause",
                copyright_text: "NOASSERTION",
            }],
        },
    )
    .await
}

fn runtime_manifest_url(configuration: &SourceConfiguration) -> String {
    format!(
        "{}/cli-v{}/browser-runtime-manifest.json",
        configuration.release_base_url,
        env!("CARGO_PKG_VERSION")
    )
}

async fn write_json(path: &Path, value: &(impl Serialize + Sync)) -> eyre::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes).await?;
    Ok(())
}

fn platform_name(platform: TargetPlatform) -> eyre::Result<&'static str> {
    match platform {
        TargetPlatform::MacOS => Ok("macos"),
        TargetPlatform::Linux => Ok("linux"),
        TargetPlatform::Windows => Ok("windows"),
        _ => Err(eyre::eyre!(
            "browser runtimes are unsupported for {platform:?}"
        )),
    }
}

fn target_architecture() -> eyre::Result<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Ok("aarch64"),
        "x86_64" => Ok("x86_64"),
        architecture => Err(eyre::eyre!(
            "browser runtimes do not support host architecture {architecture}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactManifest, CEF_FRAMEWORK_NAME, RuntimeEngine, SourceConfiguration, engines,
        find_cef_distribution, runtime_manifest_url, select_artifact, validate_cef_runtime,
    };
    use crate::platform::TargetPlatform;
    use crate::project::{BrowserRuntimePlan, ResolvedWebViewBackend};
    #[cfg(unix)]
    use std::io::Write as _;

    #[test]
    fn chromium_and_cef_share_one_runtime() {
        let engines = engines(BrowserRuntimePlan {
            webview: Some(ResolvedWebViewBackend::Cef),
            chromium: true,
        });
        assert_eq!(
            engines.into_iter().collect::<Vec<_>>(),
            [RuntimeEngine::Cef]
        );
    }

    #[test]
    fn selects_exact_artifact_and_rejects_missing_runtime() {
        let source = include_str!("browser_runtime_test_manifest.json");
        let manifest: ArtifactManifest = serde_json::from_str(source).unwrap();
        let artifact = select_artifact(
            &manifest,
            RuntimeEngine::Wpe,
            "2.52.5",
            TargetPlatform::Linux,
            "x86_64",
        )
        .unwrap();
        assert_eq!(artifact.size, 42);
        assert!(
            select_artifact(
                &manifest,
                RuntimeEngine::Wpe,
                "2.52.5",
                TargetPlatform::Linux,
                "aarch64"
            )
            .is_err()
        );
    }

    #[test]
    fn runtime_manifest_is_bound_to_the_cli_release() {
        let configuration: SourceConfiguration =
            toml::from_str(super::SOURCE_CONFIG).expect("source configuration must parse");
        assert_eq!(
            runtime_manifest_url(&configuration),
            format!(
                "https://github.com/water-rs/waterui/releases/download/cli-v{}/browser-runtime-manifest.json",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn identical_cef_build_directories_share_one_distribution_identity() {
        let temporary = tempfile::tempdir().expect("temporary CEF build directory");
        let build = temporary.path().join("build");
        let metadata = serde_json::json!({
            "name": "cef_binary_150.0.14+g7c1aa68+chromium-150.0.7871.129_macosarm64_minimal.tar.bz2",
            "sha1": "47baa745213c938861861f0f3f65c6dd03e7254e"
        });
        for hash in ["first", "second"] {
            let distribution = build
                .join(format!("cef-dll-sys-{hash}"))
                .join("out/cef_macos_aarch64");
            std::fs::create_dir_all(&distribution).expect("CEF distribution directory");
            std::fs::write(
                distribution.join("archive.json"),
                serde_json::to_vec(&metadata).expect("CEF metadata must serialize"),
            )
            .expect("CEF metadata must be written");
        }

        let selected = find_cef_distribution(
            temporary.path(),
            TargetPlatform::MacOS,
            "aarch64",
            "150.0.14",
        )
        .expect("identical CEF build outputs must be reusable");
        assert!(selected.ends_with("cef-dll-sys-first/out/cef_macos_aarch64"));
    }

    #[test]
    fn validates_platform_specific_cef_locale_layouts() {
        let temporary = tempfile::tempdir().expect("temporary CEF runtime directory");
        let macos_executable = temporary.path().join("macos/bin");
        let macos_frameworks = temporary.path().join("macos/frameworks");
        let macos_framework = macos_frameworks.join(CEF_FRAMEWORK_NAME);
        let macos_resources = macos_framework.join("Resources");
        std::fs::create_dir_all(macos_resources.join("en.lproj"))
            .expect("macOS CEF locale directory");
        std::fs::write(
            macos_framework.join("Chromium Embedded Framework"),
            b"framework",
        )
        .expect("macOS CEF framework");
        std::fs::write(macos_resources.join("icudtl.dat"), b"icu").expect("macOS CEF ICU data");
        std::fs::write(macos_resources.join("en.lproj/locale.pak"), b"locale")
            .expect("macOS CEF locale pack");
        validate_cef_runtime(
            TargetPlatform::MacOS,
            &macos_executable,
            Some(&macos_frameworks),
        )
        .expect("macOS CEF resources must use .lproj locale packs");

        let linux_executable = temporary.path().join("linux/bin");
        let linux_runtime = linux_executable.join("waterui-browser/cef");
        std::fs::create_dir_all(linux_executable.join("locales"))
            .expect("Linux CEF locales directory");
        std::fs::create_dir_all(&linux_runtime).expect("Linux CEF metadata directory");
        std::fs::write(linux_executable.join("libcef.so"), b"library").expect("Linux CEF library");
        std::fs::write(linux_executable.join("icudtl.dat"), b"icu").expect("Linux CEF ICU data");
        validate_cef_runtime(TargetPlatform::Linux, &linux_executable, None)
            .expect("Linux CEF resources must use the locales directory");
    }

    #[cfg(unix)]
    #[test]
    fn runtime_zip_restores_relative_symbolic_links() {
        let temporary = tempfile::tempdir().expect("temporary runtime archive directory");
        let archive_path = temporary.path().join("runtime.zip");
        let archive_file =
            std::fs::File::create(&archive_path).expect("runtime archive must be created");
        let mut archive = zip::ZipWriter::new(archive_file);
        archive
            .start_file(
                "lib/libengine.so",
                zip::write::SimpleFileOptions::default().unix_permissions(0o100_755),
            )
            .expect("runtime library entry must be created");
        archive
            .write_all(b"engine")
            .expect("runtime library entry must be written");
        archive
            .add_symlink(
                "lib/libengine.so.1",
                "libengine.so",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("runtime symbolic link entry must be created");
        archive.finish().expect("runtime archive must be finished");

        let destination = temporary.path().join("extracted");
        super::extract_zip(&archive_path, &destination)
            .expect("runtime archive must extract successfully");
        let link = destination.join("lib/libengine.so.1");
        assert_eq!(
            std::fs::read_link(link).expect("runtime symbolic link must be restored"),
            std::path::Path::new("libengine.so")
        );
    }
}
