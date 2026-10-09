//! Preview app launcher and session management.
//!
//! Handles launching the preview app on the target platform and
//! establishing TCP connection.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant, UNIX_EPOCH};

use cargo_toml::Manifest as CargoManifest;
use eyre::{Context, Result, bail};
use futures_util::{FutureExt as _, pin_mut, select};
#[cfg(feature = "preview")]
use notify::{RecursiveMode, Watcher as _};
use sha2::Digest as _;
use smol::stream::StreamExt;
use tracing::{error, info};

use super::app_client::{PreviewAppClient, PreviewProbe};
use super::inputs::{ProjectInputsFingerprint, project_inputs_fingerprint};
use super::protocol::DylibId;
use super::protocol::PreviewPlatform;
use super::protocol::PreviewRuntimePlatform;
use super::protocol::PreviewTcpConfig;
use crate::build::BuildProgress;

use crate::apple::dynamic_runtime;
use crate::build::{BuiltTarget, RustBuild, RustLinkage};
use crate::device::{
    Crash, Device, DeviceEvent, Local, LogLevel, RunOptions, Running, StopRequest,
};
use crate::framework::ResolvedFramework;
use crate::platform::TargetPlatform;
use crate::project::{ManagedBackends, Project};
use crate::runtime_compat::{PREVIEW_RUNTIME_ENV_VARS, runtime_profile_tag};
use crate::runtime_fingerprint::{compute_runtime_fingerprint, runtime_package_identity};
use crate::support_app;

const PREVIEW_TEMPLATE_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
const PREVIEW_METADATA_FILE: &str = ".waterui-preview-signature";
/// Bumped whenever `scaffold_preview_app` changes what it generates beyond the
/// templated files (manifest edits, permissions), which the template fingerprint
/// does not cover.
const PREVIEW_SCAFFOLD_GENERATION: u32 = 1;
const PREVIEW_DYLIB_METADATA_SUFFIX: &str = ".waterui-preview-dylib-signature";

#[derive(Debug, Clone)]
struct PreviewRequirements {
    waterui_path: Option<PathBuf>,
    /// The framework selection the previewed project resolved — its patch
    /// table is what the support app's manifest inherits; the support app's
    /// own scaffold records no framework, so taking it from there would drop
    /// every `[patch]` entry the runtime's graph needs (#197).
    framework: ResolvedFramework,
    /// The previewed project's `Water.lock` bytes, when it records a
    /// channel-managed framework — the support project's lock gate reads the
    /// same lock the app's graph was resolved against.
    framework_lock: Option<Vec<u8>>,
    runtime_fingerprint: String,
    /// The commit the support app's `waterui-preview-protocol` build reports
    /// in its handshake — read from the framework revision the project
    /// resolved, never from the revision this CLI binary was pinned at, so a
    /// stale pair rejects at connect instead of dropping mid-session (#197).
    expected_protocol_commit: String,
    runtime_features: Vec<String>,
    app_crate_name: crate::project_types::CrateName,
    app_path: PathBuf,
    /// The previewed project's own packages — the app crate plus its path
    /// dependencies outside the framework checkout — the `[profile.dev
    /// .package.<name>]` overrides the support manifests write, so the
    /// module's rebuild keeps the app unoptimized with line tables.
    project_packages: BTreeSet<String>,
}

#[derive(Debug)]
struct ResolvedPreviewMetadata {
    metadata: cargo_metadata::Metadata,
    framework: ResolvedFramework,
    app_crate_name: crate::project_types::CrateName,
    app_path: PathBuf,
    project_packages: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreviewLinkMode {
    crate_type_override: Option<&'static str>,
    prefer_dynamic: bool,
    abi_feature: &'static str,
    /// Invalidates the module cache when the link strategy for this mode
    /// changes — a module built under an older scheme resolves its symbols
    /// against a runtime the support app no longer ships.
    signature_tag: &'static str,
}

impl PreviewLinkMode {
    const MACOS_DYNAMIC: Self = Self {
        crate_type_override: None,
        prefer_dynamic: true,
        abi_feature: crate::templates::preview_ffi::APPLE_ABI_FEATURE,
        signature_tag: "preview-dylib+shared-waterui-dylib+prefer-dynamic",
    };
    const PORTABLE_DYNAMIC: Self = Self {
        crate_type_override: Some("cdylib"),
        prefer_dynamic: true,
        abi_feature: crate::templates::preview_ffi::APPLE_ABI_FEATURE,
        signature_tag: "preview-cdylib+shared-waterui-dylib+prefer-dynamic",
    };
    const fn for_platform(platform: PreviewPlatform) -> Self {
        match platform {
            PreviewPlatform::Macos => Self::MACOS_DYNAMIC,
            PreviewPlatform::Ios | PreviewPlatform::IosSimulator => Self::PORTABLE_DYNAMIC,
        }
    }

    const fn signature_tag(self) -> &'static str {
        self.signature_tag
    }

    fn configure_build(self, build: RustBuild) -> RustBuild {
        let build = match self.crate_type_override {
            Some(crate_type) => build.with_crate_type_override(crate_type),
            None => build,
        };
        build.with_feature(self.abi_feature)
    }
}

/// A preview session that manages the preview app and TCP connection.
#[derive(Debug)]
pub struct PreviewSession {
    /// TCP client to the preview app.
    pub client: PreviewAppClient,
    /// Current platform.
    pub platform: PreviewPlatform,
    /// Path to the built dylib (if any).
    dylib_path: Option<PathBuf>,
    /// Running instance for apps launched by this session.
    running: Option<Pin<Box<Running>>>,
    /// Whether this session owns the app lifecycle.
    owns_app: bool,
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<PathBuf>,
    /// Runtime fingerprint used for ABI-safe dylib invalidation.
    runtime_fingerprint: String,
    /// Host the session's builds and app launches run under.
    host: crate::toolchain::Host,
}

#[derive(Debug, Clone)]
/// A built dylib payload (stable id + on-disk path).
pub struct BuiltDylib {
    /// Stable preview cache id for the dylib payload.
    pub id: DylibId,
    /// Path to dylib on disk.
    pub path: PathBuf,
}

impl PreviewSession {
    /// Build the user's project as a dylib.
    ///
    /// # Errors
    /// Returns an error if the project cannot be opened, rebuilt, or fingerprinted.
    pub async fn build_dylib(&mut self, project_path: &std::path::Path) -> Result<BuiltDylib> {
        // The build's state machine spans the opened project, the configured
        // module build and every path they produce, and on Windows it crosses
        // clippy's `large_futures` threshold (16 KiB). Pinning it on the heap
        // keeps that off the stack of whoever awaits a preview build.
        Box::pin(build_preview_dylib(
            &self.host,
            project_path,
            self.platform,
            self.sccache_path.as_ref(),
            &self.runtime_fingerprint,
            &mut self.dylib_path,
        ))
        .await
    }

    /// Render a preview and return PNG bytes.
    ///
    /// # Errors
    /// Returns an error if the preview app rejects the render or the transport fails.
    pub async fn render(
        &mut self,
        dylib: &BuiltDylib,
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Vec<u8>> {
        let prefer_local_path = self.platform == PreviewPlatform::Macos;
        self.client
            .render_with_dylib_file(
                dylib.id,
                &dylib.path,
                symbol,
                width,
                height,
                prefer_local_path,
            )
            .await
            .map_err(|e| eyre::eyre!("Preview app error: {e}"))
    }

    /// Shutdown the preview app if this session launched it.
    ///
    /// # Errors
    /// Returns an error if the support app does not acknowledge the shutdown request.
    pub async fn shutdown(&mut self) -> Result<()> {
        if self.owns_app {
            let result = self.client.shutdown().await;
            if let Some(running) = self.running.take() {
                Pin::into_inner(running).shutdown(StopRequest::Kill).await;
            }
            self.owns_app = false;
            result?;
        }
        Ok(())
    }

    /// Detach the preview app so it keeps running after this session is dropped.
    ///
    /// The app continues running and can be reused by future preview sessions.
    pub fn detach(&mut self) {
        if let Some(mut running) = self.running.take() {
            running.as_mut().detach();
            self.owns_app = false;
        }
    }
}

/// Configures the module build to compile exactly as the runtime it will be
/// loaded into was compiled.
///
/// The preview wrapper crate lives in the managed build cache, whose generated
/// sources are regenerated whenever the CLI's scaffold templates move, so its
/// dependency graph must not be compiled into that regenerated tree.
///
/// It goes into the *support app's* shared target directory rather than the
/// previewed project's. A preview module is loaded into the support app and
/// resolves its framework symbols against the runtime that app already has open,
/// so the two have to be the same build of that runtime — not merely the same
/// source at the same version. Two target directories mean two independent
/// compilations, each with its own `-C metadata` and therefore its own hash in
/// every mangled symbol; the module then fails to `dlopen` against a runtime
/// whose symbols no longer match, even though every input to both builds was
/// identical.
///
/// Cargo folds both the deployment target and the unified feature set into that
/// same `-C metadata` hash, so a module that disagrees with its host on either
/// one links against symbols the host does not have.
async fn configure_preview_module_build(
    host: &crate::toolchain::Host,
    preview_crate_path: &Path,
    target: TargetPlatform,
    link_mode: PreviewLinkMode,
) -> Result<RustBuild> {
    let support_project = Project::open(
        host,
        &preview_support_path(host)?,
        ManagedBackends::for_platform(target),
    )
    .await
    .wrap_err("Failed to open the preview support project")?;
    let support_target_dir = support_project
        .water_target_dir(RustLinkage::SharedRuntime)
        .await?;
    let rust_build = link_mode
        .configure_build(RustBuild::for_project(
            &support_project,
            preview_crate_path,
            target.triple(),
        ))
        .with_target_dir(support_target_dir);
    let browser_runtime = support_project
        .browser_runtime_plan(target, crate::platform::TargetBackend::Apple)
        .await?;
    // The deployment-target env the module once carried explicitly now
    // comes from the triple inside `cargo_build_output`, so a module and
    // the support app it loads into cannot drift on it.
    Ok(rust_build.with_features(
        crate::apple::platform::apple_dependency_features(&support_project, browser_runtime)
            .await?,
    ))
}

async fn build_preview_dylib(
    host: &crate::toolchain::Host,
    project_path: &Path,
    platform: PreviewPlatform,
    sccache_path: Option<&PathBuf>,
    runtime_fingerprint: &str,
    dylib_path: &mut Option<PathBuf>,
) -> Result<BuiltDylib> {
    let total_start = Instant::now();
    let fingerprint_start = Instant::now();
    let project_inputs = project_inputs_fingerprint(project_path).await?;
    info!(
        project_path = %project_path.display(),
        fingerprint = %project_inputs,
        elapsed_ms = fingerprint_start.elapsed().as_millis(),
        "Preview fingerprinted project inputs"
    );

    let project_open_start = Instant::now();
    let project = Project::open_for_preview_build(host, project_path).await?;
    info!(
        project_path = %project_path.display(),
        elapsed_ms = project_open_start.elapsed().as_millis(),
        "Preview opened project"
    );
    // Scaffold rather than assume: this build used to derive the module's path
    // and trust that some earlier flow had written it, which held only while a
    // previous preview's module survived in the build cache. The support-app
    // discard that runs when the runtime checkout changes deletes that cache,
    // and the next dylib build then spawned cargo in a directory that did not
    // exist — the "Failed to execute cargo build: No such file or directory"
    // that hit every first preview after switching workspaces.
    let scaffold_start = Instant::now();
    let preview_crate_path = scaffold_preview_module(&project, platform).await?;
    info!(
        path = %preview_crate_path.display(),
        elapsed_ms = scaffold_start.elapsed().as_millis(),
        "Preview module scaffold is up to date"
    );
    let preview_crate_name = project.preview_dylib_crate_name();
    let target = preview_target_platform(platform);
    let target_triple = target.triple().to_string();
    let link_mode = PreviewLinkMode::for_platform(platform);

    ensure_project_dev_feature_for_preview(&project).await?;

    let rust_build =
        configure_preview_module_build(host, &preview_crate_path, target, link_mode).await?;
    let dylib_path_start = Instant::now();
    let expected_path = rust_build
        .dylib_path(preview_crate_name.as_str(), false)
        .await?;
    info!(
        build_crate_path = %preview_crate_path.display(),
        build_crate_name = %preview_crate_name,
        path = %expected_path.display(),
        elapsed_ms = dylib_path_start.elapsed().as_millis(),
        "Preview resolved dylib path"
    );
    let candidate_path = dylib_path.clone().unwrap_or_else(|| expected_path.clone());

    let dylib_signature = dylib_build_signature(
        project_inputs,
        runtime_fingerprint,
        &target_triple,
        preview_crate_name.as_str(),
        link_mode,
    );
    let built_path = if dylib_is_up_to_date(&candidate_path, &dylib_signature).await? {
        candidate_path
    } else {
        build_preview_module_dylib(
            rust_build,
            sccache_path,
            link_mode,
            &dylib_signature,
            &preview_crate_path,
            &preview_crate_name,
        )
        .await?
    };

    *dylib_path = Some(built_path.clone());

    let dylib_id_start = Instant::now();
    let id = compute_dylib_id(&built_path, &dylib_signature).await?;
    info!(
        path = %built_path.display(),
        elapsed_ms = dylib_id_start.elapsed().as_millis(),
        total_elapsed_ms = total_start.elapsed().as_millis(),
        "Preview prepared dylib payload"
    );
    Ok(BuiltDylib {
        id,
        path: built_path,
    })
}

async fn build_preview_module_dylib(
    mut rust_build: RustBuild,
    sccache_path: Option<&PathBuf>,
    link_mode: PreviewLinkMode,
    dylib_signature: &str,
    preview_crate_path: &Path,
    preview_crate_name: &str,
) -> Result<PathBuf> {
    info!("Building dylib...");
    if let Some(sccache) = sccache_path {
        rust_build = rust_build.with_sccache(sccache.clone());
    }
    if link_mode.prefer_dynamic {
        rust_build = rust_build.with_preferred_dynamic_linking();
    }
    let build_start = Instant::now();
    let built = rust_build
        .build_dylib(false)
        .await
        .wrap_err("Failed to build dylib")?;
    prepare_preview_module_linkage(rust_build.host(), &built, link_mode).await?;
    write_dylib_signature(&built.artifact, dylib_signature).await?;
    info!(
        build_crate_path = %preview_crate_path.display(),
        build_crate_name = %preview_crate_name,
        path = %built.artifact.display(),
        elapsed_ms = build_start.elapsed().as_millis(),
        "Preview built dylib"
    );
    Ok(built.artifact)
}

async fn prepare_preview_module_linkage(
    host: &crate::toolchain::Host,
    built: &BuiltTarget,
    link_mode: PreviewLinkMode,
) -> Result<()> {
    if !link_mode.prefer_dynamic {
        return Ok(());
    }
    dynamic_runtime::retarget_module(host, &built.artifact, built.shared_runtime()?).await
}

pub async fn ensure_project_dev_feature_for_preview(project: &Project) -> Result<()> {
    let manifest_path = project.root().join("Cargo.toml");
    let manifest = smol::unblock(move || CargoManifest::from_path(&manifest_path)).await?;
    let Some(dev_features) = manifest.features.get("dev") else {
        bail!(
            "Preview requires `{}/dev` feature. Add `[features] dev = [\"waterui/dynamic_linking\"]` to {}",
            project.crate_name().as_str(),
            project.root().join("Cargo.toml").display()
        );
    };
    if !dev_features
        .iter()
        .any(|feature| feature == "waterui/dynamic_linking")
    {
        bail!(
            "Preview requires `{}/dev` to include `waterui/dynamic_linking`. Update {}",
            project.crate_name().as_str(),
            project.root().join("Cargo.toml").display()
        );
    }
    Ok(())
}

fn dylib_signature_path(path: &Path) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(PREVIEW_DYLIB_METADATA_SUFFIX);
    PathBuf::from(raw)
}

fn dylib_build_signature(
    project_inputs: ProjectInputsFingerprint,
    runtime_fingerprint: &str,
    target_triple: &str,
    crate_name: &str,
    link_mode: PreviewLinkMode,
) -> String {
    let link_mode = link_mode.signature_tag();
    format!(
        "inputs={project_inputs}\nruntime={runtime_fingerprint}\ntarget={target_triple}\ncrate={crate_name}\nlink_mode={link_mode}"
    )
}

fn preview_run_options(host: &crate::toolchain::Host) -> RunOptions {
    let mut run_options = RunOptions::new();
    run_options.set_replace_existing_macos_app_instances(false);
    run_options.set_log_level(LogLevel::Info);
    // Point the support app's registry at the cache directory the CLI
    // watches.
    run_options.insert_env_var(
        "WATER_CACHE_DIR".to_string(),
        crate::preview::water_cache_dir(host).display().to_string(),
    );
    for (key, value) in PREVIEW_RUNTIME_ENV_VARS {
        run_options.insert_env_var(key.to_string(), value.to_string());
    }
    if let Some(rust_log) = host.env("RUST_LOG") {
        run_options.insert_env_var(
            "RUST_LOG".to_string(),
            rust_log.to_string_lossy().into_owned(),
        );
    }
    run_options
}

async fn write_dylib_signature(path: &Path, signature: &str) -> Result<()> {
    let signature_path = dylib_signature_path(path);
    crate::templates::write_file_if_changed(&signature_path, signature.as_bytes()).await?;
    Ok(())
}

async fn dylib_is_up_to_date(path: &std::path::Path, expected_signature: &str) -> Result<bool> {
    match smol::fs::metadata(path).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    }

    let signature_path = dylib_signature_path(path);
    let stored_signature = match smol::fs::read_to_string(&signature_path).await {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    Ok(stored_signature.trim() == expected_signature)
}

async fn compute_dylib_id(path: &Path, build_signature: &str) -> Result<DylibId> {
    let path = path.to_path_buf();
    let build_signature = build_signature.to_string();
    smol::unblock(move || {
        let metadata = std::fs::metadata(&path)?;
        let modified = metadata.modified()?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(build_signature.as_bytes());
        hasher.update([0]);
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(metadata.len().to_le_bytes());

        match modified.duration_since(UNIX_EPOCH) {
            Ok(duration) => {
                hasher.update([0]);
                hasher.update(duration.as_secs().to_le_bytes());
                hasher.update(duration.subsec_nanos().to_le_bytes());
            }
            Err(err) => {
                hasher.update([1]);
                hasher.update(err.duration().as_secs().to_le_bytes());
                hasher.update(err.duration().subsec_nanos().to_le_bytes());
            }
        }

        let hash: [u8; 32] = hasher.finalize().into();
        Ok(DylibId::from_bytes(hash))
    })
    .await
}

/// Launch a preview session for the given platform.
///
/// This will:
/// 1. Try to connect to an existing preview app via TCP
/// 2. If not found, scaffold and launch the preview app
/// 3. Wait for TCP connection
///
/// # Arguments
/// * `platform` - Target platform for preview
/// * `sccache_path` - Optional path to sccache for compilation caching
///
/// # Errors
/// Returns an error if the preview app cannot be launched or connected.
pub async fn launch_preview_session(
    host: &crate::toolchain::Host,
    project_path: &Path,
    platform: PreviewPlatform,
    sccache_path: Option<PathBuf>,
    progress: Option<BuildProgress>,
) -> Result<PreviewSession> {
    let requirements_start = Instant::now();
    let requirements = Box::pin(resolve_preview_requirements(host, project_path, platform)).await?;
    info!(
        project_path = %project_path.display(),
        elapsed_ms = requirements_start.elapsed().as_millis(),
        "Preview resolved runtime requirements"
    );
    let expected_fingerprint = requirements.runtime_fingerprint.clone();
    let expected_protocol_commit = requirements.expected_protocol_commit.clone();
    let tcp_config = PreviewTcpConfig::from_env()
        .map_err(|e| eyre::eyre!(e))
        .wrap_err("Invalid preview TCP config")?;

    let connect_start = Instant::now();
    if let Some(session) = try_connect_existing_preview_app(
        host,
        tcp_config,
        &expected_fingerprint,
        &expected_protocol_commit,
        platform,
        sccache_path.clone(),
    )
    .await?
    {
        info!(
            elapsed_ms = connect_start.elapsed().as_millis(),
            "Preview reused existing support app"
        );
        return Ok(session);
    }

    let project = open_preview_support_project(host, &requirements, platform).await?;
    let running = launch_preview_app_for_platform(&project, platform, progress.as_ref()).await?;
    build_preview_session_from_launch(
        host,
        running,
        platform,
        tcp_config,
        expected_fingerprint,
        expected_protocol_commit,
        sccache_path,
    )
    .await
}

async fn try_connect_existing_preview_app(
    host: &crate::toolchain::Host,
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: &str,
    expected_protocol_commit: &str,
    platform: PreviewPlatform,
    sccache_path: Option<PathBuf>,
) -> Result<Option<PreviewSession>> {
    let probe = match platform {
        PreviewPlatform::Macos => {
            PreviewAppClient::probe_registered(
                host,
                expected_fingerprint,
                PreviewRuntimePlatform::Macos,
                expected_protocol_commit,
            )
            .await?
        }
        PreviewPlatform::IosSimulator | PreviewPlatform::Ios => {
            PreviewAppClient::probe_ports(
                host,
                tcp_config,
                expected_fingerprint,
                preview_runtime_platform(platform),
                expected_protocol_commit,
            )
            .await
        }
    };
    let client = match probe {
        PreviewProbe::Connected(client) => *client,
        // Not an error here: a support app from another checkout is exactly the
        // case this function exists to decline, and the caller goes on to launch
        // one that matches. Saying so keeps the launch from looking unexplained.
        PreviewProbe::Rejected(reason) => {
            info!("Not reusing the running preview app: {reason}");
            return Ok(None);
        }
        PreviewProbe::Silent => return Ok(None),
    };

    info!("Connected to existing preview app");
    Ok(Some(PreviewSession {
        client,
        platform,
        dylib_path: None,
        running: None,
        owns_app: false,
        sccache_path,
        runtime_fingerprint: expected_fingerprint.to_string(),
        host: host.clone(),
    }))
}

const fn preview_runtime_platform(platform: PreviewPlatform) -> PreviewRuntimePlatform {
    match platform {
        PreviewPlatform::Macos => PreviewRuntimePlatform::Macos,
        PreviewPlatform::IosSimulator => PreviewRuntimePlatform::IosSimulator,
        PreviewPlatform::Ios => PreviewRuntimePlatform::Ios,
    }
}

/// The commit a `waterui-preview-protocol` build at `manifest_dir` stamps
/// into `PreviewProtocolInfo::build_commit`: the last commit touching that
/// crate's directory, read exactly as the crate's own `build.rs` reads it
/// (`git log -1 --format=%h --abbrev=12 -- .`). A directory that is not a git
/// worktree answers `unknown`, which is also what the build script stamps.
async fn preview_protocol_commit(host: &crate::toolchain::Host, manifest_dir: &Path) -> String {
    use std::ffi::OsStr;
    let commit = host
        .run(
            "git",
            [
                OsStr::new("-C"),
                manifest_dir.as_os_str(),
                OsStr::new("log"),
                OsStr::new("-1"),
                OsStr::new("--format=%h"),
                OsStr::new("--abbrev=12"),
                OsStr::new("--"),
                OsStr::new("."),
            ],
        )
        .await;
    commit.map_or_else(
        |_| "unknown".to_string(),
        |commit| {
            let commit = commit.trim();
            if commit.is_empty() {
                "unknown".to_string()
            } else {
                commit.to_string()
            }
        },
    )
}

/// The directory the `waterui-preview-protocol` manifest lives in for a
/// `waterui` package root — either the checkout itself or the package the
/// app's own metadata resolved.
async fn protocol_commit_from_metadata(
    host: &crate::toolchain::Host,
    metadata: &cargo_metadata::Metadata,
) -> Result<String> {
    let protocol = metadata
        .packages
        .iter()
        .find(|package| package.name == "waterui-preview-protocol")
        .ok_or_else(|| {
            eyre::eyre!("resolved metadata names no waterui-preview-protocol package")
        })?;
    let dir = protocol
        .manifest_path
        .as_std_path()
        .parent()
        .ok_or_else(|| eyre::eyre!("waterui-preview-protocol manifest has no parent directory"))?
        .to_path_buf();
    Ok(preview_protocol_commit(host, &dir).await)
}

/// The build target a preview on `platform` links its module for.
const fn preview_target_platform(platform: PreviewPlatform) -> TargetPlatform {
    match platform {
        PreviewPlatform::Macos => TargetPlatform::MacOS,
        PreviewPlatform::IosSimulator => TargetPlatform::IOSSimulator,
        PreviewPlatform::Ios => TargetPlatform::IOS,
    }
}

async fn open_preview_support_project(
    host: &crate::toolchain::Host,
    requirements: &PreviewRequirements,
    platform: PreviewPlatform,
) -> Result<Project> {
    info!("No preview app running, launching...");
    let preview_app_path = preview_support_path(host)?;
    let ensure_start = Instant::now();
    ensure_preview_support_app(host, &preview_app_path, requirements).await?;
    info!(
        path = %preview_app_path.display(),
        elapsed_ms = ensure_start.elapsed().as_millis(),
        "Preview support app scaffold is up to date"
    );
    let open_start = Instant::now();
    let project = Project::open(
        host,
        &preview_app_path,
        ManagedBackends::for_platform(preview_target_platform(platform)),
    )
    .await
    .wrap_err("Failed to open preview app project")?;
    info!(
        path = %preview_app_path.display(),
        elapsed_ms = open_start.elapsed().as_millis(),
        "Preview support project opened"
    );
    Ok(project)
}

async fn launch_preview_app_for_platform(
    project: &Project,
    platform: PreviewPlatform,
    progress: Option<&BuildProgress>,
) -> Result<Running> {
    match platform {
        PreviewPlatform::Macos => launch_preview_on_macos(project, progress).await,
        PreviewPlatform::IosSimulator => launch_preview_on_ios_simulator(project, progress).await,
        PreviewPlatform::Ios => {
            bail!("Physical iOS devices are not yet supported for preview");
        }
    }
}

async fn launch_preview_on_macos(
    project: &Project,
    progress: Option<&BuildProgress>,
) -> Result<Running> {
    let host = project.host();
    let backend = project
        .apple_backend()
        .ok_or_else(|| eyre::eyre!("Apple backend not configured"))?;
    let device = Local;
    device.launch(host).await?;
    let mut run_options = preview_run_options(host);
    // The support app detaches and outlives this command; its stdout/stderr go
    // to a log file the next `water preview` reopens and appends, never a pipe
    // whose reader is gone (water-rs/cli#197).
    run_options.set_app_log_file(preview_support_log_path(host)?);
    info!("Building and running preview app on macOS...");
    project
        .run_with_options(
            backend,
            TargetPlatform::MacOS,
            device,
            run_options,
            progress.cloned(),
        )
        .await
        .map_err(|e| eyre::eyre!("Failed to run preview app: {e}"))
}

async fn launch_preview_on_ios_simulator(
    project: &Project,
    progress: Option<&BuildProgress>,
) -> Result<Running> {
    let host = project.host();
    let backend = project
        .apple_backend()
        .ok_or_else(|| eyre::eyre!("Apple backend not configured"))?;
    let simulator = crate::apple::device::AppleSimulator::select_ios(project, None).await?;
    simulator.launch(host).await?;
    info!("Building and running preview app on iOS Simulator...");
    project
        .run_with_options(
            backend,
            TargetPlatform::IOSSimulator,
            simulator,
            preview_run_options(host),
            progress.cloned(),
        )
        .await
        .map_err(|e| eyre::eyre!("Failed to run preview app: {e}"))
}

async fn build_preview_session_from_launch(
    host: &crate::toolchain::Host,
    running: Running,
    platform: PreviewPlatform,
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: String,
    expected_protocol_commit: String,
    sccache_path: Option<PathBuf>,
) -> Result<PreviewSession> {
    info!("Preview app launched, waiting for TCP connection...");
    let mut running = Box::pin(running);
    let failure = match wait_for_connection_or_crash(
        host,
        &mut running,
        platform,
        tcp_config,
        &expected_fingerprint,
        &expected_protocol_commit,
    )
    .await
    {
        ConnectionWaitResult::Ready(client) => {
            return Ok(PreviewSession {
                client: *client,
                platform,
                dylib_path: None,
                running: Some(running),
                owns_app: true,
                sccache_path,
                runtime_fingerprint: expected_fingerprint,
                host: host.clone(),
            });
        }
        ConnectionWaitResult::Crashed(crash) => {
            eyre::eyre!(
                "Preview app crashed:
{crash}"
            )
        }
        ConnectionWaitResult::Exited => {
            eyre::eyre!(
                "Preview app exited unexpectedly.
Check the app logs for more information."
            )
        }
        ConnectionWaitResult::Rejected(rejection) => {
            eyre::eyre!(
                "The preview app this run just launched rejected the protocol handshake:
{rejection}"
            )
        }
        // An app that answered and was turned away is not a connection problem,
        // and listing connection problems in front of it is how this timeout
        // once sent two debugging sessions at the network.
        ConnectionWaitResult::Timeout(Some(rejection)) => {
            eyre::eyre!(
                "Preview app started but no compatible app ever answered within {} seconds.
{rejection}",
                STARTUP_DEADLINE.as_secs()
            )
        }
        ConnectionWaitResult::Timeout(None) => {
            eyre::eyre!(
                "Preview app is still running after {} seconds but never accepted a connection.
Possible causes:
- The TCP server failed to start
- Port range {}..={} may be blocked
- The app is stuck during initialization

Try running with WATERUI_CRASH_DEBUG=1 for more details.",
                STARTUP_DEADLINE.as_secs(),
                tcp_config.port_start,
                tcp_config.ports().end()
            )
        }
    };
    Pin::into_inner(running).shutdown(StopRequest::Kill).await;
    Err(failure)
}

/// Result of waiting for preview-app readiness.
enum ConnectionWaitResult {
    /// Preview app accepted a connection and completed the protocol handshake.
    Ready(Box<PreviewAppClient>),
    /// App crashed.
    Crashed(Crash),
    /// App exited without crash.
    Exited,
    /// The app stayed alive but never became reachable before the hang backstop.
    ///
    /// Carries the explanation of an app that answered and was turned away, when
    /// one did: that is a different failure from silence and has to be reported
    /// as itself.
    Timeout(Option<String>),
    /// The app this launch started answered its own announced address and was
    /// rejected by the handshake — it can never become compatible, so waiting
    /// out the deadline would only hang.
    Rejected(String),
}

/// How long a launched preview app may stay alive without ever becoming reachable.
///
/// This is a backstop against a wedged process, not a judgement about how fast a
/// preview app "should" start. Readiness is decided by real signals — the registry
/// entry the app publishes, its listening-address log line, and its crash/exit
/// events — so a slow but healthy launch is waited out rather than failed. An
/// earlier 10s budget sat right on top of the ~10.2s cold start of a debug support
/// app and lost the race by milliseconds, killing an app that was about to work.
const STARTUP_DEADLINE: Duration = Duration::from_mins(3);

/// Wait for TCP connection while monitoring for app crashes.
///
/// macOS support apps publish a registry entry once the TCP server is ready, so wait on that
/// concrete readiness signal instead of sleeping between blind connection retries.
async fn wait_for_connection_or_crash(
    host: &crate::toolchain::Host,
    running: &mut Pin<Box<Running>>,
    platform: PreviewPlatform,
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: &str,
    expected_protocol_commit: &str,
) -> ConnectionWaitResult {
    const NON_MACOS_POLL_INTERVAL: Duration = Duration::from_millis(100);

    let start = Instant::now();

    let ready = match platform {
        PreviewPlatform::Macos => {
            wait_for_registered_preview_ready(
                host,
                running,
                expected_fingerprint,
                expected_protocol_commit,
                start,
                STARTUP_DEADLINE,
            )
            .await
        }
        PreviewPlatform::IosSimulator | PreviewPlatform::Ios => {
            wait_for_polled_preview_ready(
                host,
                running,
                tcp_config,
                PolledPreviewExpectation {
                    fingerprint: expected_fingerprint,
                    protocol_commit: expected_protocol_commit,
                    platform: preview_runtime_platform(platform),
                },
                start,
                STARTUP_DEADLINE,
                NON_MACOS_POLL_INTERVAL,
            )
            .await
        }
    };

    match ready {
        ConnectionWaitResult::Timeout(rejection) => {
            drain_terminal_preview_event(running, rejection).await
        }
        other => other,
    }
}

async fn wait_for_registered_preview_ready(
    host: &crate::toolchain::Host,
    running: &mut Pin<Box<Running>>,
    expected_fingerprint: &str,
    expected_protocol_commit: &str,
    start: Instant,
    timeout: Duration,
) -> ConnectionWaitResult {
    const POLL_INTERVAL: Duration = Duration::from_millis(100);

    // The one app that answered and was turned away outlives every silent poll:
    // on timeout it is the only thing here that explains anything.
    let mut rejection = None;

    match probe_registered_preview(
        host,
        expected_fingerprint,
        expected_protocol_commit,
        PreviewRuntimePlatform::Macos,
        start,
    )
    .await
    {
        PreviewProbe::Connected(client) => return ConnectionWaitResult::Ready(client),
        PreviewProbe::Rejected(reason) => rejection = Some(reason),
        PreviewProbe::Silent => {}
    }

    let registry_dir = crate::preview::preview_instance_registry_dir(host);
    if let Err(error) = smol::fs::create_dir_all(&registry_dir).await {
        error!(path = %registry_dir.display(), "Failed to create preview registry dir: {error}");
        return ConnectionWaitResult::Timeout(rejection);
    }

    #[cfg(feature = "preview")]
    let (event_rx, _watcher) = {
        let (event_tx, event_rx) = async_channel::unbounded();
        let mut watcher = match notify::recommended_watcher(move |result| {
            let _ = event_tx.try_send(result);
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                error!(path = %registry_dir.display(), "Failed to create preview registry watcher: {error}");
                return ConnectionWaitResult::Timeout(rejection);
            }
        };
        if let Err(error) = watcher.watch(&registry_dir, RecursiveMode::NonRecursive) {
            error!(path = %registry_dir.display(), "Failed to watch preview registry dir: {error}");
            return ConnectionWaitResult::Timeout(rejection);
        }
        (event_rx, watcher)
    };

    loop {
        match probe_registered_preview(
            host,
            expected_fingerprint,
            expected_protocol_commit,
            PreviewRuntimePlatform::Macos,
            start,
        )
        .await
        {
            PreviewProbe::Connected(client) => return ConnectionWaitResult::Ready(client),
            PreviewProbe::Rejected(reason) => rejection = Some(reason),
            PreviewProbe::Silent => {}
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return ConnectionWaitResult::Timeout(rejection);
        }

        let sleep = futures_util::FutureExt::fuse(smol::Timer::after(POLL_INTERVAL.min(remaining)));
        let running_event = running.next().fuse();
        #[cfg(feature = "preview")]
        let registry_event = futures_util::FutureExt::fuse(event_rx.recv());
        #[cfg(not(feature = "preview"))]
        let registry_event = futures_util::FutureExt::fuse(futures_util::future::pending::<()>());
        pin_mut!(sleep);
        pin_mut!(running_event);
        pin_mut!(registry_event);

        select! {
            event = running_event => {
                if let Some(result) = preview_connection_result_from_device_event(
                    host,                    event,
                    expected_fingerprint,
                    expected_protocol_commit,
                    PreviewRuntimePlatform::Macos,
                    start,
                )
                .await
                {
                    return result;
                }
            },
            event = registry_event => {
                #[cfg(feature = "preview")]
                match event {
                    Ok(Ok(_notification)) => {}
                    Ok(Err(error)) => {
                        error!(path = %registry_dir.display(), "Preview registry watcher error: {error}");
                    }
                    Err(_) => return ConnectionWaitResult::Timeout(rejection),
                }
                #[cfg(not(feature = "preview"))]
                let () = event;
            },
            _ = sleep => {}
        }
    }
}

/// The identity a polled preview app must advertise to count as this
/// session's app: the runtime fingerprint, protocol commit and platform the
/// handshake checks it for.
#[derive(Clone, Copy)]
struct PolledPreviewExpectation<'a> {
    fingerprint: &'a str,
    protocol_commit: &'a str,
    platform: PreviewRuntimePlatform,
}

async fn wait_for_polled_preview_ready(
    host: &crate::toolchain::Host,
    running: &mut Pin<Box<Running>>,
    tcp_config: PreviewTcpConfig,
    expectation: PolledPreviewExpectation<'_>,
    start: Instant,
    timeout: Duration,
    poll_interval: Duration,
) -> ConnectionWaitResult {
    let mut rejection = None;

    loop {
        match probe_polled_preview(host, tcp_config, expectation, start).await {
            PreviewProbe::Connected(client) => return ConnectionWaitResult::Ready(client),
            PreviewProbe::Rejected(reason) => rejection = Some(reason),
            PreviewProbe::Silent => {}
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return ConnectionWaitResult::Timeout(rejection);
        }

        let sleep = futures_util::FutureExt::fuse(smol::Timer::after(poll_interval.min(remaining)));
        let running_event = running.next().fuse();
        pin_mut!(sleep);
        pin_mut!(running_event);

        select! {
            event = running_event => {
                if let Some(result) = preview_connection_result_from_device_event(
                    host,                    event,
                    expectation.fingerprint,
                    expectation.protocol_commit,
                    expectation.platform,
                    start,
                )
                .await
                {
                    return result;
                }
            },
            _ = sleep => {}
        }
    }
}

/// Probe the registry for a ready preview app, keeping the connection it establishes.
///
/// The probe completes a full protocol handshake, so discarding the client and
/// reconnecting afterwards would pay for that handshake twice and reopen the window
/// for the app to go away in between.
async fn probe_registered_preview(
    host: &crate::toolchain::Host,
    expected_fingerprint: &str,
    expected_protocol_commit: &str,
    expected_platform: PreviewRuntimePlatform,
    start: Instant,
) -> PreviewProbe {
    match PreviewAppClient::probe_registered(
        host,
        expected_fingerprint,
        expected_platform,
        expected_protocol_commit,
    )
    .await
    {
        Ok(PreviewProbe::Connected(client)) => {
            info!(
                "Connected to preview app after {}ms",
                start.elapsed().as_millis()
            );
            PreviewProbe::Connected(client)
        }
        Ok(other) => other,
        Err(error) => {
            error!("Failed to read the preview instance registry: {error}");
            PreviewProbe::Silent
        }
    }
}

/// Probe the configured port range for a ready preview app, keeping the connection.
async fn probe_polled_preview(
    host: &crate::toolchain::Host,
    tcp_config: PreviewTcpConfig,
    expectation: PolledPreviewExpectation<'_>,
    start: Instant,
) -> PreviewProbe {
    let probe = PreviewAppClient::probe_ports(
        host,
        tcp_config,
        expectation.fingerprint,
        expectation.platform,
        expectation.protocol_commit,
    )
    .await;
    if matches!(probe, PreviewProbe::Connected(_)) {
        info!(
            "Connected to preview app after {}ms",
            start.elapsed().as_millis()
        );
    }
    probe
}

async fn preview_connection_result_from_device_event(
    host: &crate::toolchain::Host,
    event: Option<DeviceEvent>,
    expected_fingerprint: &str,
    expected_protocol_commit: &str,
    expected_platform: PreviewRuntimePlatform,
    start: Instant,
) -> Option<ConnectionWaitResult> {
    match event? {
        DeviceEvent::Crashed(message) => {
            info!("App crashed after {}ms", start.elapsed().as_millis());
            Some(ConnectionWaitResult::Crashed(message))
        }
        DeviceEvent::Exited(_) => {
            info!("App exited after {}ms", start.elapsed().as_millis());
            Some(ConnectionWaitResult::Exited)
        }
        DeviceEvent::MonitorError { message } => {
            error!("{message}");
            None
        }
        DeviceEvent::Log { level, message } => {
            info!("Preview app log event: {message}");
            if level == tracing::Level::ERROR {
                error!("{message}");
            }
            if let Some(addr) = parse_preview_listening_addr(&message) {
                match PreviewAppClient::probe_addr(
                    host,
                    addr,
                    expected_fingerprint,
                    expected_platform,
                    expected_protocol_commit,
                )
                .await
                {
                    PreviewProbe::Connected(client) => {
                        info!(
                            "Connected to preview app after {}ms",
                            start.elapsed().as_millis()
                        );
                        return Some(ConnectionWaitResult::Ready(client));
                    }
                    // The app this launch just started announced its own port
                    // and is the wrong build: its protocol is fixed at build
                    // time, so it can never become compatible — report it now
                    // rather than letting the deadline stand in (#197).
                    PreviewProbe::Rejected(reason) => {
                        return Some(ConnectionWaitResult::Rejected(reason));
                    }
                    PreviewProbe::Silent => {}
                }
            }
            None
        }
        _ => None,
    }
}

fn parse_preview_listening_addr(message: &str) -> Option<SocketAddr> {
    const PREFIX: &str = "Preview support app listening on ";
    let suffix = message.split(PREFIX).nth(1)?;
    let port = suffix.rsplit(':').next()?.trim().parse::<u16>().ok()?;
    Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
}

async fn drain_terminal_preview_event(
    running: &mut Pin<Box<Running>>,
    rejection: Option<String>,
) -> ConnectionWaitResult {
    while let Some(event) = futures_lite::future::poll_once(running.as_mut().next())
        .await
        .flatten()
    {
        match event {
            DeviceEvent::Crashed(message) => return ConnectionWaitResult::Crashed(message),
            DeviceEvent::Exited(_) => return ConnectionWaitResult::Exited,
            DeviceEvent::MonitorError { message } => error!("{message}"),
            _ => {}
        }
    }

    ConnectionWaitResult::Timeout(rejection)
}

/// Get the path to the preview support app.
fn preview_support_path(host: &crate::toolchain::Host) -> Result<PathBuf> {
    support_app::support_app_path(host, "preview_support")
}

/// The file the macOS preview support app's stdout/stderr append to, under the
/// CLI's `~/.water` state dir beside `preview_support/` — stable across the
/// pooled instances a later `water preview` reuses.
fn preview_support_log_path(host: &crate::toolchain::Host) -> Result<PathBuf> {
    Ok(crate::water_dir::water_home_dir(host)?
        .join("logs")
        .join("preview-support.log"))
}

/// Root of the workspace a preview module joins.
///
/// This is the support runtime's generated FFI crate. The path is derived rather
/// than read from an opened [`Project`] because the module has to exist before the
/// support application is scaffolded: resolving the runtime's requirements reads
/// the module's own Cargo metadata.
async fn preview_support_ffi_crate_path(host: &crate::toolchain::Host) -> Result<PathBuf> {
    // The support application's root has to exist before its build-cache path can
    // be derived, because deriving it canonicalizes the root. On the very first
    // preview nothing has scaffolded it yet, and an empty directory is exactly
    // what the scaffolder expects to find.
    let support_path = preview_support_path(host)?;
    smol::fs::create_dir_all(&support_path)
        .await
        .wrap_err("Failed to create the preview support application directory")?;
    // The cache is brought into shape here, where the path into it is first
    // handed out, and not left to whoever opens the support project later. A
    // managed cache built by a different CLI is emptied when its shape is
    // checked, and the check used to land *after* the preview module had been
    // written into it: the module was deleted out from under the `cargo
    // metadata` that reads it, and the first preview after any change to the
    // CLI failed with a manifest path that does not exist.
    Ok(
        crate::water_dir::ensure_project_build_cache(host, &support_path)
            .await?
            .join("ffi"),
    )
}

/// Write the project's preview module into the support runtime's workspace.
///
/// Only one module lives there at a time. A module left behind by a previously
/// previewed project would still be a workspace member, and Cargo resolves every
/// member of a workspace, so a stale one whose project has since moved or been
/// deleted breaks the build of an unrelated preview.
async fn scaffold_preview_module(project: &Project, platform: PreviewPlatform) -> Result<PathBuf> {
    let support_path = preview_support_path(project.host())?;
    // Before anything reads the support runtime's workspace: one left over from
    // a different `WaterUI` checkout points its manifests at a path that may no
    // longer exist, and reading it fails before the scaffolder gets a chance to
    // notice and rebuild.
    // The project's recorded runtime path is written relative to the project,
    // so it is resolved against the project rather than against wherever the
    // CLI happens to have been invoked from.
    let runtime_path = project
        .manifest()
        .waterui_path
        .as_deref()
        .map(|path| project.root().join(path));
    support_app::discard_support_app_for_other_runtime(
        project.host(),
        &support_path,
        runtime_path.as_deref(),
    )
    .await?;
    let workspace_root = preview_support_ffi_crate_path(project.host()).await?;
    let modules_root = workspace_root.join(crate::templates::PREVIEW_MODULES_DIR);
    let crate_path = project.preview_dylib_crate_path(&workspace_root);
    if let Ok(mut entries) = smol::fs::read_dir(&modules_root).await {
        use smol::stream::StreamExt as _;
        while let Some(entry) = entries.next().await {
            let entry = entry.wrap_err("Failed to read preview modules directory")?;
            if entry.path() != crate_path {
                smol::fs::remove_dir_all(entry.path())
                    .await
                    .wrap_err("Failed to remove a stale preview module")?;
            }
        }
    }
    let crate_path = project
        .scaffold_preview_ffi_companion(&workspace_root)
        .await
        .wrap_err("Failed to scaffold the preview module")?;

    // Then refresh the support runtime, so the manifest that roots this workspace
    // is rewritten with the module now on disk. A module under a root that does
    // not declare it is rejected outright by Cargo, and the root lists whichever
    // modules it finds — so it has to be written after, never before.
    if support_path.join("Water.toml").is_file() {
        Project::open(
            project.host(),
            &support_path,
            ManagedBackends::for_platform(preview_target_platform(platform)),
        )
        .await
        .wrap_err("Failed to open the preview support project")?;
    } else {
        // First run: no support project exists yet, so nothing generates the
        // workspace root the module manifest resolves under. Write a virtual
        // root now — the managed manifest replaces it once the support
        // project scaffolds — or `cargo metadata` on the module resolves
        // without any `[patch]` and picks registry `waterui-*` copies (#197).
        let framework = project.resolved_framework().await?;
        let patches = match runtime_path.as_deref() {
            Some(root) => {
                let root = root.to_path_buf();
                smol::unblock(move || {
                    crate::project_model::templates::collect_framework_checkout_patches(&root)
                })
                .await?
            }
            None => framework.patches(),
        };
        crate::project_model::templates::ffi::write_workspace_root_manifest(
            &workspace_root,
            patches,
            Some(project.root()),
            Some(&project.project_packages(&framework).await?),
        )
        .await?;
    }
    Ok(crate_path)
}

/// Ensure the preview support app exists and matches the current project requirements.
async fn ensure_preview_support_app(
    host: &crate::toolchain::Host,
    path: &Path,
    requirements: &PreviewRequirements,
) -> Result<()> {
    let desired_signature = preview_signature(requirements);
    let scaffold_path = path.to_path_buf();
    let scaffold_requirements = requirements.clone();
    support_app::ensure_support_app(
        path,
        PREVIEW_METADATA_FILE,
        &desired_signature,
        "preview support",
        move || async move { scaffold_preview_app(host, &scaffold_path, &scaffold_requirements).await },
    )
    .await
}

/// Scaffold the preview support app as a normal project.
async fn scaffold_preview_app(
    host: &crate::toolchain::Host,
    path: &Path,
    requirements: &PreviewRequirements,
) -> Result<()> {
    use crate::project::{CreateOptions, Manifest as WaterManifest};
    use crate::templates::TemplateContext;

    let waterui_path = requirements.waterui_path.clone();

    let options = CreateOptions {
        name: "WaterUI Preview".to_string(),
        bundle_identifier: crate::project_types::BundleIdentifier::try_from("dev.waterui.preview")
            .expect("preview support bundle identifier must be valid"),
        waterui_path: waterui_path.clone(),
        channel: None,
        framework_manifest: None,
        // The support app inherits the previewed project's framework
        // selection exactly: its scaffold's lockfile and `[patch]` table are
        // generated against the same revision the app resolves, so a `dev`
        // project never meets a `stable` support graph (cli#197).
        framework: Some(requirements.framework.clone()),
        framework_lock: requirements.framework_lock.clone(),
        author: String::new(),
        web: None,
    };

    let project = Project::create(host, path, options)
        .await
        .map_err(|e| eyre::eyre!("Failed to create preview app: {e}"))?;

    // Mark the preview app as accessory/headless.
    let mut manifest = WaterManifest::open(project.root().join("Water.toml")).await?;
    manifest.package.accessory = true;
    // The support app hosts the preview TCP server on-device; binding a socket
    // requires INTERNET in its manifest regardless of what the previewed app
    // declares.
    manifest.permissions.insert(
        crate::project_types::PermissionKey::Internet,
        crate::project::PermissionEntry::enabled(
            "Hosts the preview TCP server that the CLI connects to",
        ),
    );
    manifest.save(project.root()).await?;

    let ctx = TemplateContext::for_support_app(
        host,
        crate::templates::SupportAppIdentity {
            display_name: "WaterUI Preview".to_string(),
            crate_name: project.crate_name().clone(),
            bundle_identifier: crate::project_types::BundleIdentifier::try_from(
                "dev.waterui.preview",
            )
            .expect("preview support bundle identifier must be valid"),
        },
        waterui_path,
        &requirements.framework,
        true,
        Some(requirements.runtime_fingerprint.clone()),
        project.local_sources(),
    )
    .with_preview_runtime_features(requirements.runtime_features.clone())
    .with_preview_app_dependency(
        requirements.app_crate_name.clone(),
        requirements.app_path.clone(),
    )
    .with_project_packages(requirements.project_packages.clone());

    crate::templates::preview::scaffold(host, project.root(), &ctx)
        .await
        .wrap_err("Failed to scaffold embedded preview app template")?;

    // The manifest the preview template writes supersedes the one the create
    // pass resolved, so the lock must resolve once more before the locked
    // project open that follows reads it. Seeding from the previewed app's
    // lock keeps every version the project's graph already pins.
    let canonical = requirements
        .framework
        .canonical_lock(project.root())
        .await
        .wrap_err("Failed to read the channel's canonical lock")?;
    crate::templates::seed_lockfile(
        project.root(),
        &requirements.app_path.join("Cargo.lock"),
        canonical.as_ref(),
    )
    .await
    .wrap_err("Failed to seed the preview support app's Cargo.lock")?;
    // `cargo metadata` refreshes the lock in place, keeping the seeded
    // versions and adding only the entries the support manifest's own
    // packages need — `generate-lockfile` would re-resolve every crate at
    // its newest and drift the support app off the project's lock.
    let mut command = cargo_metadata::MetadataCommand::new();
    command.manifest_path(project.root().join("Cargo.toml"));
    host.cargo_metadata(&command)
        .await
        .wrap_err("Failed to refresh the preview support app's Cargo.lock")?;

    info!("Preview app scaffolded at {}", path.display());
    Ok(())
}

fn preview_signature(requirements: &PreviewRequirements) -> String {
    format!(
        "template_commit={PREVIEW_TEMPLATE_COMMIT}\nscaffold_generation={PREVIEW_SCAFFOLD_GENERATION}\nwaterui_dependency={}\nruntime_fingerprint={}\ntemplate_fingerprint={}",
        requirements.waterui_path.as_ref().map_or_else(
            || String::from("registry"),
            |path| path.display().to_string()
        ),
        requirements.runtime_fingerprint,
        crate::templates::preview::template_fingerprint(&requirements.project_packages),
    )
}

async fn resolve_preview_requirements(
    host: &crate::toolchain::Host,
    project_path: &Path,
    platform: PreviewPlatform,
) -> Result<PreviewRequirements> {
    let resolved = resolve_preview_metadata(host, project_path, platform).await?;
    let metadata = &resolved.metadata;
    let waterui = select_unique_package(metadata, "waterui")?;
    let runtime_features = resolved_package_features(metadata, waterui)?;
    let graph_fingerprint = resolved_graph_fingerprint(metadata)?;

    if let Some(requirements) = resolve_preview_requirements_from_manifest(
        host,
        project_path,
        &runtime_features,
        &graph_fingerprint,
        &resolved,
    )
    .await?
    {
        return Ok(requirements);
    }
    let waterui_core = select_unique_package(metadata, "waterui-core")?;
    let runtime_identity = runtime_package_identity(waterui_core);

    let runtime_fingerprint_start = Instant::now();
    let runtime_fingerprint_base = if waterui.source.is_none() {
        let waterui_root = waterui
            .manifest_path
            .as_std_path()
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| eyre::eyre!("Failed to derive waterui package root path"))?;
        let fingerprint =
            compute_runtime_fingerprint(host, &waterui_root, &runtime_identity).await?;
        info!(
            waterui_root = %waterui_root.display(),
            elapsed_ms = runtime_fingerprint_start.elapsed().as_millis(),
            "Preview computed dev-mode runtime fingerprint"
        );
        let protocol_dir = waterui_root.join("components/devtools/preview/protocol");
        let expected_protocol_commit = preview_protocol_commit(host, &protocol_dir).await;
        return Ok(PreviewRequirements {
            waterui_path: Some(waterui_root),
            framework: resolved.framework,
            framework_lock: None,
            expected_protocol_commit,
            runtime_fingerprint: runtime_fingerprint(
                &fingerprint,
                &runtime_features,
                &graph_fingerprint,
            ),
            runtime_features,
            app_crate_name: resolved.app_crate_name,
            app_path: resolved.app_path,
            project_packages: resolved.project_packages,
        });
    } else {
        let source = waterui
            .source
            .as_ref()
            .map(ToString::to_string)
            .expect("registry dependency must have a source");
        info!(
            package = %runtime_identity,
            source = %source,
            elapsed_ms = runtime_fingerprint_start.elapsed().as_millis(),
            "Preview resolved release-mode runtime fingerprint"
        );
        format!("{runtime_identity}:source:{source}")
    };

    Ok(PreviewRequirements {
        waterui_path: None,
        framework_lock: Some(
            smol::fs::read(resolved.app_path.join("Water.lock"))
                .await
                .wrap_err("the project's Water.lock could not be read")?,
        ),
        framework: resolved.framework,
        expected_protocol_commit: protocol_commit_from_metadata(host, metadata).await?,
        runtime_fingerprint: runtime_fingerprint(
            &runtime_fingerprint_base,
            &runtime_features,
            &graph_fingerprint,
        ),
        runtime_features,
        app_crate_name: resolved.app_crate_name,
        app_path: resolved.app_path,
        project_packages: resolved.project_packages,
    })
}

async fn resolve_preview_requirements_from_manifest(
    host: &crate::toolchain::Host,
    project_path: &Path,
    runtime_features: &[String],
    graph_fingerprint: &str,
    resolved: &ResolvedPreviewMetadata,
) -> Result<Option<PreviewRequirements>> {
    let ResolvedPreviewMetadata {
        app_crate_name,
        app_path,
        framework,
        project_packages,
        ..
    } = resolved;
    let manifest_open_start = Instant::now();
    let manifest = crate::project::Manifest::open(project_path.join("Water.toml"))
        .await
        .map_err(|error| {
            eyre::eyre!(
                "Failed to read Water.toml for preview requirements at {}: {error}",
                project_path.display()
            )
        })?;
    info!(
        project_path = %project_path.display(),
        elapsed_ms = manifest_open_start.elapsed().as_millis(),
        "Preview opened Water.toml for runtime requirements"
    );
    let Some(waterui_path) = manifest.waterui_path else {
        return Ok(None);
    };

    let resolve_root_start = Instant::now();
    let waterui_root = resolve_waterui_root_from_manifest(project_path, &waterui_path).await?;
    info!(
        project_path = %project_path.display(),
        waterui_root = %waterui_root.display(),
        elapsed_ms = resolve_root_start.elapsed().as_millis(),
        "Preview resolved waterui root from manifest"
    );

    let runtime_identity_start = Instant::now();
    let runtime_identity = runtime_identity_from_waterui_root(&waterui_root).await?;
    info!(
        waterui_root = %waterui_root.display(),
        elapsed_ms = runtime_identity_start.elapsed().as_millis(),
        "Preview resolved runtime identity"
    );

    let runtime_fingerprint_start = Instant::now();
    let runtime_fingerprint = runtime_fingerprint(
        &compute_runtime_fingerprint(host, &waterui_root, &runtime_identity).await?,
        runtime_features,
        graph_fingerprint,
    );
    info!(
        project_path = %project_path.display(),
        waterui_root = %waterui_root.display(),
        elapsed_ms = runtime_fingerprint_start.elapsed().as_millis(),
        "Preview resolved runtime requirements from Water.toml"
    );

    let protocol_dir = waterui_root.join("components/devtools/preview/protocol");
    let expected_protocol_commit = preview_protocol_commit(host, &protocol_dir).await;
    Ok(Some(PreviewRequirements {
        waterui_path: Some(waterui_root),
        framework: framework.clone(),
        framework_lock: None,
        expected_protocol_commit,
        runtime_fingerprint,
        runtime_features: runtime_features.to_vec(),
        app_crate_name: app_crate_name.clone(),
        app_path: app_path.clone(),
        project_packages: project_packages.clone(),
    }))
}

async fn resolve_preview_metadata(
    host: &crate::toolchain::Host,
    project_path: &Path,
    platform: PreviewPlatform,
) -> Result<ResolvedPreviewMetadata> {
    let project = Project::open_for_preview_build(host, project_path).await?;
    ensure_project_dev_feature_for_preview(&project).await?;
    let framework = project.resolved_framework().await?;
    let manifest_path = scaffold_preview_module(&project, platform)
        .await?
        .join("Cargo.toml");
    let app_crate_name = project.crate_name().clone();
    let app_path = project.root().to_path_buf();
    let project_packages = project.project_packages(&framework).await?;
    let metadata_start = Instant::now();
    let abi_feature = PreviewLinkMode::for_platform(platform)
        .abi_feature
        .to_string();
    let mut command = cargo_metadata::MetadataCommand::new();
    command
        .manifest_path(&manifest_path)
        .features(cargo_metadata::CargoOpt::SomeFeatures(vec![abi_feature]));
    let metadata = host
        .cargo_metadata(&command)
        .await
        .wrap_err("Failed to resolve user project Cargo metadata with its dev feature")?;
    info!(
        project_path = %project_path.display(),
        elapsed_ms = metadata_start.elapsed().as_millis(),
        "Preview resolved user project cargo metadata"
    );
    Ok(ResolvedPreviewMetadata {
        metadata,
        framework,
        app_crate_name,
        app_path,
        project_packages,
    })
}

fn resolved_package_features(
    metadata: &cargo_metadata::Metadata,
    package: &cargo_metadata::Package,
) -> Result<Vec<String>> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| eyre::eyre!("Cargo metadata omitted its dependency resolution graph"))?;
    let node = resolve
        .nodes
        .iter()
        .find(|node| node.id == package.id)
        .ok_or_else(|| {
            eyre::eyre!(
                "Cargo metadata omitted the resolution node for package `{}`",
                package.name
            )
        })?;
    let mut features = node
        .features
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    features.sort_unstable();
    features.dedup();
    if !features.iter().any(|feature| feature == "dynamic_linking") {
        bail!("Preview requires the project dev feature to enable waterui/dynamic_linking");
    }
    Ok(features)
}

fn resolved_graph_fingerprint(metadata: &cargo_metadata::Metadata) -> Result<String> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| eyre::eyre!("Cargo metadata omitted its dependency resolution graph"))?;
    let mut units = resolve
        .nodes
        .iter()
        .map(|node| {
            let mut features = node
                .features
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            features.sort_unstable();
            format!("{}|{}", node.id, features.join(","))
        })
        .collect::<Vec<_>>();
    units.sort_unstable();
    let mut hasher = sha2::Sha256::new();
    for unit in units {
        hasher.update(unit.as_bytes());
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

fn runtime_fingerprint(base: &str, features: &[String], graph_fingerprint: &str) -> String {
    format!(
        "{base}|features={}|graph={}|profile={}",
        features.join(","),
        graph_fingerprint,
        runtime_profile_tag()
    )
}

async fn resolve_waterui_root_from_manifest(
    project_path: &Path,
    waterui_path: &str,
) -> Result<PathBuf> {
    let candidate = PathBuf::from(waterui_path);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        project_path.join(candidate)
    };
    smol::fs::canonicalize(&resolved).await.wrap_err_with(|| {
        format!(
            "Failed to resolve `waterui_path = {waterui_path}` from {}",
            project_path.display()
        )
    })
}

async fn runtime_identity_from_waterui_root(waterui_root: &Path) -> Result<String> {
    let core_manifest_path = waterui_root.join("core").join("Cargo.toml");
    let manifest_text = smol::fs::read_to_string(&core_manifest_path)
        .await
        .wrap_err("Failed to read waterui-core Cargo.toml for preview requirements")?;
    let manifest: toml::Table = manifest_text
        .parse()
        .wrap_err("Failed to parse waterui-core Cargo.toml for preview requirements")?;
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            eyre::eyre!(
                "Invalid waterui-core manifest at {}: missing package section",
                core_manifest_path.display()
            )
        })?;
    let package_name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            eyre::eyre!(
                "Invalid waterui-core manifest at {}: missing package.name",
                core_manifest_path.display()
            )
        })?;
    if package_name != "waterui-core" {
        bail!(
            "Invalid preview runtime root {}: expected core/Cargo.toml package `waterui-core`, found `{}`",
            waterui_root.display(),
            package_name
        );
    }
    let package_version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            eyre::eyre!(
                "Invalid waterui-core manifest at {}: missing package.version",
                core_manifest_path.display()
            )
        })?;

    Ok(format!("{package_name}@{package_version}"))
}

fn select_unique_package<'a>(
    metadata: &'a cargo_metadata::Metadata,
    name: &str,
) -> Result<&'a cargo_metadata::Package> {
    let mut matches = metadata.packages.iter().filter(|p| p.name == name);
    let first = matches
        .next()
        .ok_or_else(|| eyre::eyre!("Could not resolve package `{name}` from metadata"))?;
    if matches.next().is_some() {
        bail!(
            "Multiple `{name}` packages were resolved. Preview requires a single resolved `{name}` package to guarantee compatibility."
        );
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::{PreviewLinkMode, PreviewPlatform};

    #[test]
    fn macos_preview_uses_shared_waterui_runtime() {
        let link_mode = PreviewLinkMode::for_platform(PreviewPlatform::Macos);

        assert_eq!(link_mode, PreviewLinkMode::MACOS_DYNAMIC);
        assert_eq!(link_mode.crate_type_override, None);
        assert!(link_mode.prefer_dynamic);
        assert_eq!(
            link_mode.abi_feature,
            crate::templates::preview_ffi::APPLE_ABI_FEATURE
        );
        assert_eq!(
            link_mode.signature_tag(),
            "preview-dylib+shared-waterui-dylib+prefer-dynamic"
        );
    }

    #[test]
    fn remote_preview_platforms_use_shared_runtime_cdylibs() {
        for platform in [PreviewPlatform::Ios, PreviewPlatform::IosSimulator] {
            let link_mode = PreviewLinkMode::for_platform(platform);

            assert_eq!(link_mode, PreviewLinkMode::PORTABLE_DYNAMIC);
            assert_eq!(link_mode.crate_type_override, Some("cdylib"));
            assert!(link_mode.prefer_dynamic);
            assert_eq!(
                link_mode.abi_feature,
                crate::templates::preview_ffi::APPLE_ABI_FEATURE
            );
            assert_eq!(
                link_mode.signature_tag(),
                "preview-cdylib+shared-waterui-dylib+prefer-dynamic"
            );
        }
    }
}
