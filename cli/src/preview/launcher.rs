//! Preview app launcher and session management.
//!
//! Handles launching the preview app on the target platform and
//! establishing TCP connection.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use color_eyre::eyre::{Context, Result, bail};
use sha2::Digest as _;
use smol::stream::StreamExt;
use tracing::{error, info};

use super::app_client::PreviewAppClient;
use super::protocol::DylibId;
use super::protocol::PreviewPlatform;
use super::protocol::PreviewTcpConfig;
use super::watcher::ProjectWatcher;

use crate::build::RustBuild;
use crate::device::{Device, DeviceEvent, Local, RunOptions, Running};
use crate::platform::TargetPlatform;
use crate::project::Project;
use crate::runtime_compat::{PREVIEW_RUNTIME_ENV_VARS, runtime_profile_tag};
use crate::runtime_fingerprint::compute_runtime_fingerprint;
use crate::support_app;

const PREVIEW_TEMPLATE_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
const PREVIEW_METADATA_FILE: &str = ".waterui-preview-signature";
const PREVIEW_DYLIB_METADATA_SUFFIX: &str = ".waterui-preview-dylib-signature";

#[derive(Debug, Clone)]
struct PreviewRequirements {
    waterui_root: PathBuf,
    runtime_fingerprint: String,
}

/// A preview session that manages the preview app and TCP connection.
#[derive(Debug)]
pub struct PreviewSession {
    /// TCP client to the preview app.
    pub client: PreviewAppClient,
    /// Watcher for detecting file changes.
    pub watcher: ProjectWatcher,
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
    /// SHA-256 id of dylib content.
    pub id: DylibId,
    /// Path to dylib on disk.
    pub path: PathBuf,
}

impl PreviewSession {
    /// Build the user's project as a dylib.
    pub async fn build_dylib(&mut self, project_path: &std::path::Path) -> Result<BuiltDylib> {
        build_preview_dylib(
            project_path,
            self.platform,
            &mut self.watcher,
            self.sccache_path.as_ref(),
            &self.runtime_fingerprint,
            &mut self.dylib_path,
        )
        .await
    }

    /// Render a preview and return PNG bytes.
    pub async fn render(
        &mut self,
        dylib: &BuiltDylib,
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Vec<u8>> {
        self.client
            .render_with_dylib_file(dylib.id, &dylib.path, symbol, width, height)
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Preview app error: {e}"))
    }

    /// Shutdown the preview app if this session launched it.
    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.client.shutdown().await;
        if self.owns_app {
            // Dropping `running` will terminate the app if still alive.
            self.running.take();
        }
        Ok(())
    }

    /// Detach the preview app so it keeps running after this session is dropped.
    ///
    /// This "forgets" the Running instance so its Drop handler won't kill the app.
    /// The app will continue running and can be reused by future preview sessions.
    pub fn detach(&mut self) {
        if let Some(running) = self.running.take() {
            // Leak the Running to prevent Drop from killing the app
            std::mem::forget(running);
            self.owns_app = false;
        }
    }
}

pub(crate) async fn build_preview_dylib(
    project_path: &Path,
    platform: PreviewPlatform,
    watcher: &mut ProjectWatcher,
    sccache_path: Option<&PathBuf>,
    runtime_fingerprint: &str,
    dylib_path: &mut Option<PathBuf>,
) -> Result<BuiltDylib> {
    let stamp = watcher.stamp(project_path).await?;
    let project = Project::open(project_path).await?;
    let target = match platform {
        PreviewPlatform::Macos => TargetPlatform::MacOS,
        PreviewPlatform::IosSimulator => TargetPlatform::IOSSimulator,
        PreviewPlatform::Ios => TargetPlatform::IOS,
        PreviewPlatform::Android => TargetPlatform::Android,
    };

    let mut rust_build = RustBuild::new(project.root(), target.triple());
    if let Some(sccache) = sccache_path {
        rust_build = rust_build.with_sccache(sccache.clone());
    }
    let expected_path = rust_build.dylib_path(project.crate_name(), false).await?;
    let candidate_path = dylib_path.clone().unwrap_or_else(|| expected_path.clone());

    let target_triple = target.triple().to_string();
    let dylib_signature =
        dylib_build_signature(runtime_fingerprint, &target_triple, project.crate_name());
    let built_path = if dylib_is_up_to_date(&candidate_path, stamp.mtime, &dylib_signature).await? {
        candidate_path
    } else {
        info!("Building dylib...");
        let built_path = rust_build
            .build_dylib(project.crate_name(), false)
            .await
            .wrap_err("Failed to build dylib")?;
        write_dylib_signature(&built_path, &dylib_signature).await?;
        info!("Dylib built: {}", built_path.display());
        built_path
    };

    *dylib_path = Some(built_path.clone());

    let id = compute_dylib_id(&built_path).await?;
    Ok(BuiltDylib {
        id,
        path: built_path,
    })
}

fn dylib_signature_path(path: &Path) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(PREVIEW_DYLIB_METADATA_SUFFIX);
    PathBuf::from(raw)
}

fn dylib_build_signature(
    runtime_fingerprint: &str,
    target_triple: &str,
    crate_name: &str,
) -> String {
    format!("runtime={runtime_fingerprint}\ntarget={target_triple}\ncrate={crate_name}")
}

fn preview_run_options() -> RunOptions {
    let mut run_options = RunOptions::new();
    for (key, value) in PREVIEW_RUNTIME_ENV_VARS {
        run_options.insert_env_var(key.to_string(), value.to_string());
    }
    run_options
}

async fn write_dylib_signature(path: &Path, signature: &str) -> Result<()> {
    let signature_path = dylib_signature_path(path);
    smol::fs::write(signature_path, signature.as_bytes()).await?;
    Ok(())
}

async fn dylib_is_up_to_date(
    path: &std::path::Path,
    source_mtime: SystemTime,
    expected_signature: &str,
) -> Result<bool> {
    let metadata = match smol::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    let dylib_mtime = metadata.modified()?;
    if dylib_mtime < source_mtime {
        return Ok(false);
    }

    let signature_path = dylib_signature_path(path);
    let stored_signature = match smol::fs::read_to_string(&signature_path).await {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    Ok(stored_signature.trim() == expected_signature)
}

async fn compute_dylib_id(path: &Path) -> Result<DylibId> {
    let path = path.to_path_buf();
    smol::unblock(move || {
        use std::io::Read as _;

        let mut file = std::fs::File::open(&path)?;
        let mut hasher = sha2::Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
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
    let requirements = resolve_preview_requirements(project_path).await?;
    let expected_fingerprint = requirements.runtime_fingerprint.clone();

    let tcp_config = PreviewTcpConfig::from_env()
        .map_err(|e| color_eyre::eyre::eyre!(e))
        .wrap_err("Invalid preview TCP config")?;

    // First, try to connect to an already-running preview app
    if let Ok(client) = PreviewAppClient::connect(tcp_config, &expected_fingerprint).await {
        info!("Connected to existing preview app");
        return Ok(PreviewSession {
            client,
            watcher: ProjectWatcher::new(),
            platform,
            dylib_path: None,
            running: None,
            owns_app: false,
            sccache_path,
            runtime_fingerprint: expected_fingerprint.clone(),
        });
    }

    info!("No preview app running, launching...");

    // Ensure the preview support app exists and is up to date
    let preview_app_path = preview_support_path()?;
    ensure_preview_support_app(&preview_app_path, &requirements).await?;

    // Open the preview app project
    let project = Project::open(&preview_app_path)
        .await
        .wrap_err("Failed to open preview app project")?;

    // Launch based on platform
    let running = match platform {
        PreviewPlatform::Macos => {
            let backend = project
                .apple_backend()
                .ok_or_else(|| color_eyre::eyre::eyre!("Apple backend not configured"))?;
            let device = Local;
            device.launch().await?;
            info!("Building and running preview app on macOS...");
            let run_options = preview_run_options();
            project
                .run_with_options(backend, TargetPlatform::MacOS, device, run_options)
                .await
                .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))?
        }
        PreviewPlatform::IosSimulator => {
            let backend = project
                .apple_backend()
                .ok_or_else(|| color_eyre::eyre::eyre!("Apple backend not configured"))?;

            // Find an iOS simulator
            let simulators = crate::apple::device::AppleSimulator::scan_ios().await?;
            let simulator = simulators
                .iter()
                .find(|s| s.state == "Booted")
                .cloned()
                .or_else(|| simulators.into_iter().next())
                .ok_or_else(|| {
                    color_eyre::eyre::eyre!(
                        "No iOS simulator available. Please create one in Xcode."
                    )
                })?;

            simulator.launch().await?;
            info!("Building and running preview app on iOS Simulator...");
            let run_options = preview_run_options();
            project
                .run_with_options(
                    backend,
                    TargetPlatform::IOSSimulator,
                    simulator,
                    run_options,
                )
                .await
                .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))?
        }
        PreviewPlatform::Ios => {
            bail!("Physical iOS devices are not yet supported for preview");
        }
        PreviewPlatform::Android => {
            let backend = project
                .android_backend()
                .ok_or_else(|| color_eyre::eyre::eyre!("Android backend not configured"))?;

            // Find an Android device or emulator
            let devices = crate::android::device::AndroidDevice::scan().await?;

            if let Some(device) = devices.into_iter().next() {
                device.launch().await?;
                info!("Building and running preview app on Android device...");
                let run_options = preview_run_options();
                project
                    .run_android_with_options(backend, device, run_options)
                    .await
                    .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))?
            } else {
                // Try emulator
                let avds = crate::android::platform::AndroidPlatform::list_avds().await?;
                let avd_name = avds.into_iter().next().ok_or_else(|| {
                    color_eyre::eyre::eyre!("No Android devices or emulators available.")
                })?;
                let emulator = crate::android::device::AndroidEmulator::open(avd_name).await?;
                emulator.launch().await?;
                info!("Building and running preview app on Android emulator...");
                let run_options = preview_run_options();
                project
                    .run_android_with_options(backend, emulator, run_options)
                    .await
                    .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))?
            }
        }
    };

    info!("Preview app launched, waiting for TCP connection...");

    // Wait for TCP connection while monitoring for crashes
    let result =
        wait_for_connection_or_crash(running, platform, tcp_config, &expected_fingerprint).await;

    match result {
        ConnectionResult::Connected { client, running } => Ok(PreviewSession {
            client,
            watcher: ProjectWatcher::new(),
            platform,
            dylib_path: None,
            running: Some(running),
            owns_app: true,
            sccache_path,
            runtime_fingerprint: expected_fingerprint.clone(),
        }),
        ConnectionResult::Crashed(message) => {
            bail!("Preview app crashed:\n{message}")
        }
        ConnectionResult::Exited => {
            bail!("Preview app exited unexpectedly.\nCheck the app logs for more information.")
        }
        ConnectionResult::Timeout => {
            bail!(
                "Preview app started but failed to connect via TCP after 10 seconds.\nPossible causes:\n- The app may have crashed during initialization\n- The TCP server failed to start\n- Port range {}..={} may be blocked\n\nTry running with WATERUI_CRASH_DEBUG=1 for more details.",
                tcp_config.port_start,
                tcp_config.ports().end()
            )
        }
    }
}

/// Result of waiting for TCP connection.
enum ConnectionResult {
    /// Successfully connected to preview app.
    Connected {
        client: PreviewAppClient,
        running: Pin<Box<Running>>,
    },
    /// App crashed with error message.
    Crashed(String),
    /// App exited without crash.
    Exited,
    /// Connection timed out.
    Timeout,
}

/// Wait for TCP connection while monitoring for app crashes.
///
/// This function polls both the TCP connection and the Running stream
/// to detect crashes early and provide better error messages.
async fn wait_for_connection_or_crash(
    running: Running,
    _platform: PreviewPlatform,
    tcp_config: PreviewTcpConfig,
    expected_fingerprint: &str,
) -> ConnectionResult {
    const MAX_ATTEMPTS: u32 = 100; // 10 seconds total
    const POLL_INTERVAL_MS: u64 = 100;

    let mut running = Box::pin(running);

    for i in 0..MAX_ATTEMPTS {
        // Check for app events (crash, exit) - non-blocking
        while let Some(event) = futures_lite::future::poll_once(running.as_mut().next())
            .await
            .flatten()
        {
            match event {
                DeviceEvent::Crashed(message) => {
                    info!("App crashed after {}ms", (i + 1) * POLL_INTERVAL_MS as u32);
                    return ConnectionResult::Crashed(message);
                }
                DeviceEvent::Exited => {
                    info!("App exited after {}ms", (i + 1) * POLL_INTERVAL_MS as u32);
                    return ConnectionResult::Exited;
                }
                DeviceEvent::Log { level, message } => {
                    // Print log messages at ERROR level to help diagnose startup issues
                    if level == tracing::Level::ERROR {
                        error!("{message}");
                    }
                }
                _ => {}
            }
        }

        // Try to connect to TCP
        if let Ok(client) = PreviewAppClient::connect(tcp_config, expected_fingerprint).await {
            info!(
                "Connected to preview app after {}ms",
                (i + 1) * POLL_INTERVAL_MS as u32
            );
            return ConnectionResult::Connected { client, running };
        }

        smol::Timer::after(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }

    // One final check for crash events before giving up
    while let Some(event) = futures_lite::future::poll_once(running.as_mut().next())
        .await
        .flatten()
    {
        match event {
            DeviceEvent::Crashed(message) => {
                return ConnectionResult::Crashed(message);
            }
            DeviceEvent::Exited => {
                return ConnectionResult::Exited;
            }
            _ => {}
        }
    }

    ConnectionResult::Timeout
}

/// Get the path to the preview support app.
fn preview_support_path() -> Result<PathBuf> {
    support_app::support_app_path("preview_support")
}

/// Ensure the preview support app exists and matches the current project requirements.
async fn ensure_preview_support_app(
    path: &PathBuf,
    requirements: &PreviewRequirements,
) -> Result<()> {
    let desired_signature = preview_signature(requirements);
    let scaffold_path = path.clone();
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

    let waterui_path = requirements.waterui_root.clone();

    let options = CreateOptions {
        name: "WaterUI Preview".to_string(),
        bundle_identifier: crate::project_types::BundleIdentifier::try_from("dev.waterui.preview")
            .expect("preview support bundle identifier must be valid"),
        package_type: PackageType::Playground,
        waterui_path: Some(waterui_path.clone()),
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
        "template_commit={PREVIEW_TEMPLATE_COMMIT}\nwaterui_root={}\nruntime_fingerprint={}\ntemplate_fingerprint={}",
        requirements.waterui_root.display(),
        requirements.runtime_fingerprint,
        crate::templates::preview::template_fingerprint(),
    )
}

async fn resolve_preview_requirements(project_path: &Path) -> Result<PreviewRequirements> {
    let current_dir = project_path.to_path_buf();
    let metadata = smol::unblock(move || {
        cargo_metadata::MetadataCommand::new()
            .current_dir(current_dir)
            .exec()
    })
    .await
    .wrap_err("Failed to resolve user project Cargo metadata for preview compatibility")?;

    let waterui = select_unique_package(&metadata, "waterui")?;
    if waterui.source.is_some() {
        bail!(
            "Preview requires path-based WaterUI dependencies to guarantee ABI/runtime compatibility. \
Current project resolves `waterui` from a non-path source."
        );
    }
    let waterui_root = waterui
        .manifest_path
        .as_std_path()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| color_eyre::eyre::eyre!("Failed to derive waterui package root path"))?;

    let waterui_core = select_unique_package(&metadata, "waterui-core")?;
    let waterui_core_id = waterui_core.id.to_string();
    let runtime_fingerprint_base =
        compute_runtime_fingerprint(&waterui_root, &waterui_core_id).await?;
    let runtime_fingerprint = format!(
        "{runtime_fingerprint_base}|profile={}",
        runtime_profile_tag()
    );
    Ok(PreviewRequirements {
        waterui_root,
        runtime_fingerprint,
    })
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
