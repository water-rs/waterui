//! Preview app launcher and session management.
//!
//! Handles launching the preview app on the target platform and
//! establishing TCP connection.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant, UNIX_EPOCH};

use cargo_toml::Manifest as CargoManifest;
use color_eyre::eyre::{Context, Result, bail};
use futures::{FutureExt as _, pin_mut, select};
use notify::{RecursiveMode, Watcher as _};
use sha2::Digest as _;
use smol::stream::StreamExt;
use tracing::{error, info};

use super::app_client::PreviewAppClient;
use super::inputs::{ProjectInputsFingerprint, project_inputs_fingerprint};
use super::protocol::DylibId;
use super::protocol::PreviewPlatform;
use super::protocol::PreviewRuntimePlatform;
use super::protocol::PreviewTcpConfig;

use crate::apple::dynamic_runtime;
use crate::build::{RustBuild, RustLinkage, rust_target_libdir};
use crate::device::{Device, DeviceEvent, Local, LogLevel, RunOptions, Running};
use crate::platform::TargetPlatform;
use crate::project::Project;
use crate::runtime_compat::{PREVIEW_RUNTIME_ENV_VARS, runtime_profile_tag};
use crate::runtime_fingerprint::{compute_runtime_fingerprint, runtime_package_identity};
use crate::support_app;
use waterui_preview_protocol::registry::preview_instance_registry_dir;

const PREVIEW_TEMPLATE_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
const PREVIEW_METADATA_FILE: &str = ".waterui-preview-signature";
const PREVIEW_DYLIB_METADATA_SUFFIX: &str = ".waterui-preview-dylib-signature";

#[derive(Debug, Clone)]
struct PreviewRequirements {
    waterui_path: Option<PathBuf>,
    runtime_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreviewLinkMode {
    crate_type_override: Option<&'static str>,
    prefer_dynamic: bool,
}

impl PreviewLinkMode {
    const MACOS_DYNAMIC: Self = Self {
        crate_type_override: None,
        prefer_dynamic: true,
    };
    const PORTABLE_STATIC: Self = Self {
        crate_type_override: Some("cdylib"),
        prefer_dynamic: false,
    };

    const fn for_platform(platform: PreviewPlatform) -> Self {
        match platform {
            PreviewPlatform::Macos => Self::MACOS_DYNAMIC,
            PreviewPlatform::Ios | PreviewPlatform::IosSimulator | PreviewPlatform::Android => {
                Self::PORTABLE_STATIC
            }
        }
    }

    const fn signature_tag(self) -> &'static str {
        if self.prefer_dynamic {
            "preview-dylib+shared-waterui-dylib+prefer-dynamic"
        } else {
            "preview-cdylib+static-waterui"
        }
    }

    fn configure_build(self, build: RustBuild) -> RustBuild {
        match self.crate_type_override {
            Some(crate_type) => build.with_crate_type_override(crate_type),
            None => build,
        }
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
        build_preview_dylib(
            project_path,
            self.platform,
            self.sccache_path.as_ref(),
            &self.runtime_fingerprint,
            &mut self.dylib_path,
        )
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
            .map_err(|e| color_eyre::eyre::eyre!("Preview app error: {e}"))
    }

    /// Shutdown the preview app if this session launched it.
    ///
    /// # Errors
    /// Returns an error if the support app does not acknowledge the shutdown request.
    pub async fn shutdown(&mut self) -> Result<()> {
        if self.owns_app {
            let result = self.client.shutdown().await;
            // Dropping `running` will terminate the app if still alive.
            self.running.take();
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

async fn build_preview_dylib(
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
    let project = Project::open_for_preview_build(project_path).await?;
    info!(
        project_path = %project_path.display(),
        elapsed_ms = project_open_start.elapsed().as_millis(),
        "Preview opened project"
    );
    let preview_crate_path = project.preview_dylib_crate_path();
    let preview_crate_name = project.preview_dylib_crate_name();
    let target = match platform {
        PreviewPlatform::Macos => TargetPlatform::MacOS,
        PreviewPlatform::IosSimulator => TargetPlatform::IOSSimulator,
        PreviewPlatform::Ios => TargetPlatform::IOS,
        PreviewPlatform::Android => TargetPlatform::Android,
    };
    let target_triple = target.triple().to_string();
    let link_mode = PreviewLinkMode::for_platform(platform);

    ensure_project_dev_feature_for_preview(&project).await?;

    let mut rust_build =
        link_mode.configure_build(RustBuild::new(&preview_crate_path, target.triple()));
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
        info!("Building dylib...");
        if let Some(sccache) = sccache_path {
            rust_build = rust_build.with_sccache(sccache.clone());
        }
        rust_build = rust_build.with_rustc_flag("-Cdebuginfo=0");
        if link_mode.prefer_dynamic {
            rust_build = rust_build.with_rustc_flag("-Cprefer-dynamic");
            let rust_target_libdir = rust_target_libdir(&target.triple()).await?;
            rust_build = rust_build.with_rustc_flag(format!(
                "-Clink-arg=-Wl,-rpath,{}",
                rust_target_libdir.display()
            ));
        }
        let build_start = Instant::now();
        let built_path = rust_build
            .build_dylib(preview_crate_name.as_str(), false)
            .await
            .wrap_err("Failed to build dylib")?;
        if link_mode.prefer_dynamic {
            let build_lib_dir = built_path.parent().ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "Preview dylib path has no output directory: {}",
                    built_path.display()
                )
            })?;
            dynamic_runtime::retarget_module(&built_path, build_lib_dir).await?;
        }
        write_dylib_signature(&built_path, &dylib_signature).await?;
        info!(
            build_crate_path = %preview_crate_path.display(),
            build_crate_name = %preview_crate_name,
            path = %built_path.display(),
            elapsed_ms = build_start.elapsed().as_millis(),
            "Preview built dylib"
        );
        built_path
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

async fn ensure_project_dev_feature_for_preview(project: &Project) -> Result<()> {
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

fn preview_run_options() -> RunOptions {
    let mut run_options = RunOptions::new();
    run_options.set_replace_existing_macos_app_instances(false);
    run_options.set_log_level(LogLevel::Info);
    let preview_cache_root = waterui_preview_protocol::registry::preview_cache_root_dir();
    let water_cache_dir = preview_cache_root.parent().unwrap_or_else(|| {
        panic!(
            "preview cache root must have a parent directory: {}",
            preview_cache_root.display()
        )
    });
    run_options.insert_env_var(
        "WATER_CACHE_DIR".to_string(),
        water_cache_dir.display().to_string(),
    );
    for (key, value) in PREVIEW_RUNTIME_ENV_VARS {
        run_options.insert_env_var(key.to_string(), value.to_string());
    }
    if let Some(rust_log) = std::env::var_os("RUST_LOG") {
        run_options.insert_env_var(
            "RUST_LOG".to_string(),
            rust_log.to_string_lossy().into_owned(),
        );
    }
    run_options
}

async fn write_dylib_signature(path: &Path, signature: &str) -> Result<()> {
    let signature_path = dylib_signature_path(path);
    smol::fs::write(signature_path, signature.as_bytes()).await?;
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
    project_path: &Path,
    platform: PreviewPlatform,
    sccache_path: Option<PathBuf>,
) -> Result<PreviewSession> {
    let requirements_start = Instant::now();
    let requirements = resolve_preview_requirements(project_path).await?;
    info!(
        project_path = %project_path.display(),
        elapsed_ms = requirements_start.elapsed().as_millis(),
        "Preview resolved runtime requirements"
    );
    let expected_fingerprint = requirements.runtime_fingerprint.clone();
    let tcp_config = PreviewTcpConfig::from_env()
        .map_err(|e| color_eyre::eyre::eyre!(e))
        .wrap_err("Invalid preview TCP config")?;

    let connect_start = Instant::now();
    if let Some(session) = try_connect_existing_preview_app(
        tcp_config,
        &expected_fingerprint,
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

    let project = open_preview_support_project(&requirements).await?;
    let running = launch_preview_app_for_platform(&project, platform).await?;
    build_preview_session_from_launch(
        running,
        platform,
        tcp_config,
        expected_fingerprint,
        sccache_path,
    )
    .await
}

async fn try_connect_existing_preview_app(
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: &str,
    platform: PreviewPlatform,
    sccache_path: Option<PathBuf>,
) -> Result<Option<PreviewSession>> {
    let client = match platform {
        PreviewPlatform::Macos => connect_existing_macos_preview_app(expected_fingerprint).await?,
        PreviewPlatform::IosSimulator | PreviewPlatform::Ios | PreviewPlatform::Android => {
            PreviewAppClient::connect(
                tcp_config,
                expected_fingerprint,
                preview_runtime_platform(platform),
            )
            .await
            .ok()
        }
    };
    let Some(client) = client else {
        return Ok(None);
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
    }))
}

async fn connect_existing_macos_preview_app(
    expected_fingerprint: &str,
) -> Result<Option<PreviewAppClient>> {
    Ok(
        PreviewAppClient::connect_registered(expected_fingerprint, PreviewRuntimePlatform::Macos)
            .await
            .ok(),
    )
}

const fn preview_runtime_platform(platform: PreviewPlatform) -> PreviewRuntimePlatform {
    match platform {
        PreviewPlatform::Macos => PreviewRuntimePlatform::Macos,
        PreviewPlatform::IosSimulator => PreviewRuntimePlatform::IosSimulator,
        PreviewPlatform::Ios => PreviewRuntimePlatform::Ios,
        PreviewPlatform::Android => PreviewRuntimePlatform::Android,
    }
}

async fn open_preview_support_project(requirements: &PreviewRequirements) -> Result<Project> {
    info!("No preview app running, launching...");
    let preview_app_path = preview_support_path()?;
    let ensure_start = Instant::now();
    ensure_preview_support_app(&preview_app_path, requirements).await?;
    info!(
        path = %preview_app_path.display(),
        elapsed_ms = ensure_start.elapsed().as_millis(),
        "Preview support app scaffold is up to date"
    );
    let open_start = Instant::now();
    let project = Project::open(&preview_app_path)
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
) -> Result<Running> {
    match platform {
        PreviewPlatform::Macos => launch_preview_on_macos(project).await,
        PreviewPlatform::IosSimulator => launch_preview_on_ios_simulator(project).await,
        PreviewPlatform::Ios => {
            bail!("Physical iOS devices are not yet supported for preview");
        }
        PreviewPlatform::Android => launch_preview_on_android(project).await,
    }
}

async fn launch_preview_on_macos(project: &Project) -> Result<Running> {
    let backend = project
        .apple_backend()
        .ok_or_else(|| color_eyre::eyre::eyre!("Apple backend not configured"))?;
    let device = Local;
    device.launch().await?;
    info!("Building and running preview app on macOS...");
    project
        .run_with_linkage(
            backend,
            TargetPlatform::MacOS,
            device,
            RustLinkage::SharedRuntime,
            preview_run_options(),
        )
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))
}

async fn launch_preview_on_ios_simulator(project: &Project) -> Result<Running> {
    let backend = project
        .apple_backend()
        .ok_or_else(|| color_eyre::eyre::eyre!("Apple backend not configured"))?;
    let simulator = select_preview_ios_simulator().await?;
    simulator.launch().await?;
    info!("Building and running preview app on iOS Simulator...");
    project
        .run_with_options(
            backend,
            TargetPlatform::IOSSimulator,
            simulator,
            preview_run_options(),
        )
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))
}

async fn select_preview_ios_simulator() -> Result<crate::apple::device::AppleSimulator> {
    let simulators = crate::apple::device::AppleSimulator::scan_ios().await?;
    simulators
        .iter()
        .find(|simulator| simulator.state == "Booted")
        .cloned()
        .or_else(|| simulators.into_iter().next())
        .ok_or_else(|| {
            color_eyre::eyre::eyre!("No iOS simulator available. Please create one in Xcode.")
        })
}

async fn launch_preview_on_android(project: &Project) -> Result<Running> {
    let backend = project
        .android_backend()
        .ok_or_else(|| color_eyre::eyre::eyre!("Android backend not configured"))?;

    if let Some(device) = crate::android::device::AndroidDevice::scan()
        .await?
        .into_iter()
        .next()
    {
        device.launch().await?;
        info!("Building and running preview app on Android device...");
        return project
            .run_android_with_options(backend, device, preview_run_options())
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"));
    }

    let avd_name = crate::android::platform::AndroidPlatform::list_avds()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| color_eyre::eyre::eyre!("No Android devices or emulators available."))?;
    let emulator = crate::android::device::AndroidEmulator::open(avd_name).await?;
    emulator.launch().await?;
    info!("Building and running preview app on Android emulator...");
    project
        .run_android_with_options(backend, emulator, preview_run_options())
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))
}

async fn build_preview_session_from_launch(
    running: Running,
    platform: PreviewPlatform,
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: String,
    sccache_path: Option<PathBuf>,
) -> Result<PreviewSession> {
    info!("Preview app launched, waiting for TCP connection...");
    let mut running = Box::pin(running);
    match wait_for_connection_or_crash(&mut running, platform, tcp_config, &expected_fingerprint)
        .await
    {
        ConnectionWaitResult::Ready(client) => {
            let client = match client {
                Some(client) => client,
                None => match platform {
                    PreviewPlatform::Macos => PreviewAppClient::connect_registered(
                        &expected_fingerprint,
                        PreviewRuntimePlatform::Macos,
                    )
                    .await
                    .wrap_err("Preview app became ready but registry connection still failed")?,
                    PreviewPlatform::IosSimulator
                    | PreviewPlatform::Ios
                    | PreviewPlatform::Android => PreviewAppClient::connect(
                        tcp_config,
                        &expected_fingerprint,
                        preview_runtime_platform(platform),
                    )
                    .await
                    .wrap_err("Preview app became ready but TCP connection still failed")?,
                },
            };
            Ok(PreviewSession {
                client,
                platform,
                dylib_path: None,
                running: Some(running),
                owns_app: true,
                sccache_path,
                runtime_fingerprint: expected_fingerprint,
            })
        }
        ConnectionWaitResult::Crashed(message) => bail!(
            "Preview app crashed:
{message}"
        ),
        ConnectionWaitResult::Exited => {
            bail!(
                "Preview app exited unexpectedly.
Check the app logs for more information."
            )
        }
        ConnectionWaitResult::Timeout => {
            bail!(
                "Preview app started but failed to connect via TCP after 10 seconds.
Possible causes:
- The app may have crashed during initialization
- The TCP server failed to start
- Port range {}..={} may be blocked

Try running with WATERUI_CRASH_DEBUG=1 for more details.",
                tcp_config.port_start,
                tcp_config.ports().end()
            )
        }
    }
}

/// Result of waiting for preview-app readiness.
enum ConnectionWaitResult {
    /// Preview app reported readiness and should now accept a connection.
    Ready(Option<PreviewAppClient>),
    /// App crashed with error message.
    Crashed(String),
    /// App exited without crash.
    Exited,
    /// Connection timed out.
    Timeout,
}

/// Wait for TCP connection while monitoring for app crashes.
///
/// macOS support apps publish a registry entry once the TCP server is ready, so wait on that
/// concrete readiness signal instead of sleeping between blind connection retries.
async fn wait_for_connection_or_crash(
    running: &mut Pin<Box<Running>>,
    platform: PreviewPlatform,
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: &str,
) -> ConnectionWaitResult {
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
    const NON_MACOS_POLL_INTERVAL: Duration = Duration::from_millis(100);

    let start = Instant::now();

    let ready = match platform {
        PreviewPlatform::Macos => {
            wait_for_registered_preview_ready(running, expected_fingerprint, start, STARTUP_TIMEOUT)
                .await
        }
        PreviewPlatform::IosSimulator | PreviewPlatform::Ios | PreviewPlatform::Android => {
            wait_for_polled_preview_ready(
                running,
                tcp_config,
                expected_fingerprint,
                preview_runtime_platform(platform),
                start,
                STARTUP_TIMEOUT,
                NON_MACOS_POLL_INTERVAL,
            )
            .await
        }
    };

    match ready {
        ConnectionWaitResult::Timeout => drain_terminal_preview_event(running).await,
        other => other,
    }
}

async fn wait_for_registered_preview_ready(
    running: &mut Pin<Box<Running>>,
    expected_fingerprint: &str,
    start: Instant,
    timeout: Duration,
) -> ConnectionWaitResult {
    const POLL_INTERVAL: Duration = Duration::from_millis(100);

    if try_connect_registered_preview(expected_fingerprint, PreviewRuntimePlatform::Macos, start)
        .await
    {
        return ConnectionWaitResult::Ready(None);
    }

    let registry_dir = preview_instance_registry_dir();
    if let Err(error) = smol::fs::create_dir_all(&registry_dir).await {
        error!(path = %registry_dir.display(), "Failed to create preview registry dir: {error}");
        return ConnectionWaitResult::Timeout;
    }

    let (event_tx, event_rx) = async_channel::unbounded();
    let callback_tx = event_tx.clone();
    let mut watcher = match notify::recommended_watcher(move |result| {
        let _ = callback_tx.try_send(result);
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            error!(path = %registry_dir.display(), "Failed to create preview registry watcher: {error}");
            return ConnectionWaitResult::Timeout;
        }
    };
    if let Err(error) = watcher.watch(&registry_dir, RecursiveMode::NonRecursive) {
        error!(path = %registry_dir.display(), "Failed to watch preview registry dir: {error}");
        return ConnectionWaitResult::Timeout;
    }

    loop {
        if try_connect_registered_preview(
            expected_fingerprint,
            PreviewRuntimePlatform::Macos,
            start,
        )
        .await
        {
            return ConnectionWaitResult::Ready(None);
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return ConnectionWaitResult::Timeout;
        }

        let sleep = futures::FutureExt::fuse(smol::Timer::after(POLL_INTERVAL.min(remaining)));
        let running_event = running.next().fuse();
        let registry_event = futures::FutureExt::fuse(event_rx.recv());
        pin_mut!(sleep);
        pin_mut!(running_event);
        pin_mut!(registry_event);

        select! {
            event = running_event => {
                if let Some(result) = preview_connection_result_from_device_event(
                    event,
                    expected_fingerprint,
                    PreviewRuntimePlatform::Macos,
                    start,
                )
                .await
                {
                    return result;
                }
            },
            event = registry_event => {
                match event {
                    Ok(Ok(_notification)) => {}
                    Ok(Err(error)) => {
                        error!(path = %registry_dir.display(), "Preview registry watcher error: {error}");
                    }
                    Err(_) => return ConnectionWaitResult::Timeout,
                }
            },
            _ = sleep => {}
        }
    }
}

async fn wait_for_polled_preview_ready(
    running: &mut Pin<Box<Running>>,
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: &str,
    expected_platform: PreviewRuntimePlatform,
    start: Instant,
    timeout: Duration,
    poll_interval: Duration,
) -> ConnectionWaitResult {
    loop {
        if try_connect_polled_preview(tcp_config, expected_fingerprint, expected_platform, start)
            .await
        {
            return ConnectionWaitResult::Ready(None);
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return ConnectionWaitResult::Timeout;
        }

        let sleep = futures::FutureExt::fuse(smol::Timer::after(poll_interval.min(remaining)));
        let running_event = running.next().fuse();
        pin_mut!(sleep);
        pin_mut!(running_event);

        select! {
            event = running_event => {
                if let Some(result) = preview_connection_result_from_device_event(
                    event,
                    expected_fingerprint,
                    expected_platform,
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

async fn try_connect_registered_preview(
    expected_fingerprint: &str,
    expected_platform: PreviewRuntimePlatform,
    start: Instant,
) -> bool {
    match PreviewAppClient::connect_registered(expected_fingerprint, expected_platform).await {
        Ok(_) => {
            info!(
                "Connected to preview app after {}ms",
                start.elapsed().as_millis()
            );
            true
        }
        Err(_) => false,
    }
}

async fn try_connect_polled_preview(
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: &str,
    expected_platform: PreviewRuntimePlatform,
    start: Instant,
) -> bool {
    match PreviewAppClient::connect(tcp_config, expected_fingerprint, expected_platform).await {
        Ok(_) => {
            info!(
                "Connected to preview app after {}ms",
                start.elapsed().as_millis()
            );
            true
        }
        Err(_) => false,
    }
}

async fn preview_connection_result_from_device_event(
    event: Option<DeviceEvent>,
    expected_fingerprint: &str,
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
        DeviceEvent::Log { level, message } => {
            info!("Preview app log event: {message}");
            if level == tracing::Level::ERROR {
                error!("{message}");
            }
            if let Some(addr) = parse_preview_listening_addr(&message)
                && let Ok(client) =
                    PreviewAppClient::connect_addr(addr, expected_fingerprint, expected_platform)
                        .await
            {
                info!(
                    "Connected to preview app after {}ms",
                    start.elapsed().as_millis()
                );
                return Some(ConnectionWaitResult::Ready(Some(client)));
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

async fn drain_terminal_preview_event(running: &mut Pin<Box<Running>>) -> ConnectionWaitResult {
    while let Some(event) = futures_lite::future::poll_once(running.as_mut().next())
        .await
        .flatten()
    {
        match event {
            DeviceEvent::Crashed(message) => return ConnectionWaitResult::Crashed(message),
            DeviceEvent::Exited(_) => return ConnectionWaitResult::Exited,
            _ => {}
        }
    }

    ConnectionWaitResult::Timeout
}

/// Get the path to the preview support app.
fn preview_support_path() -> Result<PathBuf> {
    support_app::support_app_path("preview_support")
}

/// Ensure the preview support app exists and matches the current project requirements.
async fn ensure_preview_support_app(path: &Path, requirements: &PreviewRequirements) -> Result<()> {
    let desired_signature = preview_signature(requirements);
    let scaffold_path = path.to_path_buf();
    let scaffold_requirements = requirements.clone();
    support_app::ensure_support_app(
        path,
        PREVIEW_METADATA_FILE,
        &desired_signature,
        "preview support",
        move || async move { scaffold_preview_app(&scaffold_path, &scaffold_requirements).await },
    )
    .await
}

/// Scaffold the preview support app as a normal playground project.
async fn scaffold_preview_app(path: &Path, requirements: &PreviewRequirements) -> Result<()> {
    use crate::project::{CreateOptions, Manifest as WaterManifest, PackageType};
    use crate::templates::TemplateContext;

    let waterui_path = requirements.waterui_path.clone();

    let options = CreateOptions {
        name: "WaterUI Preview".to_string(),
        bundle_identifier: crate::project_types::BundleIdentifier::try_from("dev.waterui.preview")
            .expect("preview support bundle identifier must be valid"),
        package_type: PackageType::Playground,
        waterui_path: waterui_path.clone(),
        author: String::new(),
    };

    // Create as normal playground project
    let project = Project::create(path, options)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to create preview app: {e}"))?;

    // Mark the preview app as accessory/headless.
    let mut manifest = WaterManifest::open(project.root().join("Water.toml")).await?;
    manifest.package.accessory = true;
    manifest.save(project.root()).await?;

    let ctx = TemplateContext::for_support_playground(
        "WaterUI Preview",
        "WaterUIPreview",
        project.crate_name().clone(),
        crate::project_types::BundleIdentifier::try_from("dev.waterui.preview")
            .expect("preview support bundle identifier must be valid"),
        waterui_path,
        true,
        Some(requirements.runtime_fingerprint.clone()),
    );

    crate::templates::preview::scaffold(project.root(), &ctx)
        .await
        .wrap_err("Failed to scaffold embedded preview app template")?;

    info!("Preview app scaffolded at {}", path.display());
    Ok(())
}

fn preview_signature(requirements: &PreviewRequirements) -> String {
    format!(
        "template_commit={PREVIEW_TEMPLATE_COMMIT}\nwaterui_dependency={}\nruntime_fingerprint={}\ntemplate_fingerprint={}",
        requirements.waterui_path.as_ref().map_or_else(
            || String::from("registry"),
            |path| path.display().to_string()
        ),
        requirements.runtime_fingerprint,
        crate::templates::preview::template_fingerprint(),
    )
}

async fn resolve_preview_requirements(project_path: &Path) -> Result<PreviewRequirements> {
    if let Some(requirements) = resolve_preview_requirements_from_manifest(project_path).await? {
        return Ok(requirements);
    }

    let current_dir = project_path.to_path_buf();
    let metadata_start = Instant::now();
    let metadata = smol::unblock(move || {
        cargo_metadata::MetadataCommand::new()
            .current_dir(current_dir)
            .exec()
    })
    .await
    .wrap_err("Failed to resolve user project Cargo metadata for preview compatibility")?;
    info!(
        project_path = %project_path.display(),
        elapsed_ms = metadata_start.elapsed().as_millis(),
        "Preview resolved user project cargo metadata"
    );

    let waterui = select_unique_package(&metadata, "waterui")?;
    let waterui_core = select_unique_package(&metadata, "waterui-core")?;
    let runtime_identity = runtime_package_identity(waterui_core);

    let runtime_fingerprint_start = Instant::now();
    let runtime_fingerprint_base = if waterui.source.is_none() {
        let waterui_root = waterui
            .manifest_path
            .as_std_path()
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| color_eyre::eyre::eyre!("Failed to derive waterui package root path"))?;
        let fingerprint = compute_runtime_fingerprint(&waterui_root, &runtime_identity).await?;
        info!(
            waterui_root = %waterui_root.display(),
            elapsed_ms = runtime_fingerprint_start.elapsed().as_millis(),
            "Preview computed dev-mode runtime fingerprint"
        );
        return Ok(PreviewRequirements {
            waterui_path: Some(waterui_root),
            runtime_fingerprint: format!("{fingerprint}|profile={}", runtime_profile_tag()),
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
        runtime_fingerprint: format!(
            "{runtime_fingerprint_base}|profile={}",
            runtime_profile_tag()
        ),
    })
}

async fn resolve_preview_requirements_from_manifest(
    project_path: &Path,
) -> Result<Option<PreviewRequirements>> {
    let manifest_open_start = Instant::now();
    let manifest = crate::project::Manifest::open(project_path.join("Water.toml"))
        .await
        .map_err(|error| {
            color_eyre::eyre::eyre!(
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
    let runtime_fingerprint = format!(
        "{}|profile={}",
        compute_runtime_fingerprint(&waterui_root, &runtime_identity).await?,
        runtime_profile_tag()
    );
    info!(
        project_path = %project_path.display(),
        waterui_root = %waterui_root.display(),
        elapsed_ms = runtime_fingerprint_start.elapsed().as_millis(),
        "Preview resolved runtime requirements from Water.toml"
    );

    Ok(Some(PreviewRequirements {
        waterui_path: Some(waterui_root),
        runtime_fingerprint,
    }))
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
            color_eyre::eyre::eyre!(
                "Invalid waterui-core manifest at {}: missing package section",
                core_manifest_path.display()
            )
        })?;
    let package_name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            color_eyre::eyre::eyre!(
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
            color_eyre::eyre::eyre!(
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
    let first = matches.next().ok_or_else(|| {
        color_eyre::eyre::eyre!("Could not resolve package `{name}` from metadata")
    })?;
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
            link_mode.signature_tag(),
            "preview-dylib+shared-waterui-dylib+prefer-dynamic"
        );
    }

    #[test]
    fn remote_preview_platforms_use_self_contained_cdylibs() {
        for platform in [
            PreviewPlatform::Ios,
            PreviewPlatform::IosSimulator,
            PreviewPlatform::Android,
        ] {
            let link_mode = PreviewLinkMode::for_platform(platform);

            assert_eq!(link_mode, PreviewLinkMode::PORTABLE_STATIC);
            assert_eq!(link_mode.crate_type_override, Some("cdylib"));
            assert!(!link_mode.prefer_dynamic);
            assert_eq!(link_mode.signature_tag(), "preview-cdylib+static-waterui");
        }
    }
}
