//! Preview app launcher and session management.
//!
//! Handles launching the preview app on the target platform and
//! establishing TCP connection.

use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use std::time::SystemTime;

use color_eyre::eyre::{Context, Result, bail};
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

const PREVIEW_APP_VERSION: &str = env!("WATERUI_CLI_COMMIT");
const PREVIEW_METADATA_FILE: &str = ".waterui-preview-version";

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
}

#[derive(Debug, Clone)]
/// A built dylib payload (bytes + stable id).
pub struct BuiltDylib {
    /// SHA-256 id of `bytes`.
    pub id: DylibId,
    /// Raw dylib bytes.
    pub bytes: Vec<u8>,
}

impl PreviewSession {
    /// Build the user's project as a dylib.
    pub async fn build_dylib(&mut self, project_path: &std::path::Path) -> Result<BuiltDylib> {
        let stamp = self.watcher.stamp(project_path).await?;
        let project = Project::open(project_path).await?;
        let target = match self.platform {
            PreviewPlatform::Macos => TargetPlatform::MacOS,
            PreviewPlatform::IosSimulator => TargetPlatform::IOSSimulator,
            PreviewPlatform::Ios => TargetPlatform::IOS,
            PreviewPlatform::Android => TargetPlatform::Android,
        };

        let mut rust_build = RustBuild::new(project.root(), target.triple());
        if let Some(sccache) = &self.sccache_path {
            rust_build = rust_build.with_sccache(sccache.clone());
        }
        let expected_path = rust_build.dylib_path(project.crate_name(), false).await?;
        let candidate_path = self
            .dylib_path
            .clone()
            .unwrap_or_else(|| expected_path.clone());

        let dylib_path = if dylib_is_up_to_date(&candidate_path, stamp.mtime).await? {
            candidate_path
        } else {
            info!("Building dylib...");
            let dylib_path = rust_build
                .build_dylib(project.crate_name(), false)
                .await
                .wrap_err("Failed to build dylib")?;

            info!("Dylib built: {}", dylib_path.display());
            dylib_path
        };

        self.dylib_path = Some(dylib_path.clone());

        let data = smol::fs::read(&dylib_path).await?;
        let id = DylibId::from_payload(&data);
        Ok(BuiltDylib { id, bytes: data })
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
            .render(dylib.id, &dylib.bytes, symbol, width, height)
            .await
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

async fn dylib_is_up_to_date(path: &std::path::Path, source_mtime: SystemTime) -> Result<bool> {
    let metadata = match smol::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    let dylib_mtime = metadata.modified()?;
    Ok(dylib_mtime >= source_mtime)
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
    platform: PreviewPlatform,
    sccache_path: Option<PathBuf>,
) -> Result<PreviewSession> {
    let tcp_config = PreviewTcpConfig::from_env()
        .map_err(|e| color_eyre::eyre::eyre!(e))
        .wrap_err("Invalid preview TCP config")?;

    // First, try to connect to an already-running preview app
    if let Ok(client) = PreviewAppClient::connect(tcp_config).await {
        info!("Connected to existing preview app");
        return Ok(PreviewSession {
            client,
            watcher: ProjectWatcher::new(),
            platform,
            dylib_path: None,
            running: None,
            owns_app: false,
            sccache_path,
        });
    }

    info!("No preview app running, launching...");

    // Ensure the preview support app exists and is up to date
    let preview_app_path = preview_support_path();
    ensure_preview_support_app(&preview_app_path).await?;

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
            let run_options = RunOptions::new();
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
            let run_options = RunOptions::new();
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
                let run_options = RunOptions::new();
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
                let run_options = RunOptions::new();
                project
                    .run_android_with_options(backend, emulator, run_options)
                    .await
                    .map_err(|e| color_eyre::eyre::eyre!("Failed to run preview app: {e}"))?
            }
        }
    };

    info!("Preview app launched, waiting for TCP connection...");

    // Wait for TCP connection while monitoring for crashes
    let result = wait_for_connection_or_crash(running, platform, tcp_config).await;

    match result {
        ConnectionResult::Connected { client, running } => Ok(PreviewSession {
            client,
            watcher: ProjectWatcher::new(),
            platform,
            dylib_path: None,
            running: Some(running),
            owns_app: true,
            sccache_path,
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
        if let Ok(client) = PreviewAppClient::connect(tcp_config).await {
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
fn preview_support_path() -> PathBuf {
    dirs::home_dir()
        .expect("home directory should exist")
        .join(".water")
        .join("preview_support")
}

/// Ensure the preview support app exists and matches the current CLI version.
async fn ensure_preview_support_app(path: &PathBuf) -> Result<()> {
    let metadata_path = path.join(PREVIEW_METADATA_FILE);
    let cargo_path = path.join("Cargo.toml");

    let mut needs_scaffold = !cargo_path.exists();
    if !needs_scaffold {
        let stored_version = smol::fs::read_to_string(&metadata_path)
            .await
            .unwrap_or_default();
        if stored_version.trim() != PREVIEW_APP_VERSION {
            needs_scaffold = true;
        }
    }

    if needs_scaffold {
        if path.exists() {
            smol::fs::remove_dir_all(path).await?;
        }
        info!("Scaffolding preview app at {}", path.display());
        scaffold_preview_app(path).await?;
        smol::fs::write(&metadata_path, PREVIEW_APP_VERSION).await?;
    } else if !metadata_path.exists() {
        smol::fs::write(&metadata_path, PREVIEW_APP_VERSION).await?;
    }

    Ok(())
}

/// Scaffold the preview support app as a normal playground project.
async fn scaffold_preview_app(path: &PathBuf) -> Result<()> {
    use crate::project::{CreateOptions, Manifest as WaterManifest};
    use cargo_toml::{Dependency, DependencyDetail, Manifest as CargoManifest};

    let waterui_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");

    let options = CreateOptions {
        name: "WaterUI Preview".to_string(),
        bundle_identifier: "dev.waterui.preview".to_string(),
        playground: true,
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

    // Add waterui-preview dependency to Cargo.toml
    let cargo_path = project.root().join("Cargo.toml");
    let cargo_content = smol::fs::read_to_string(&cargo_path).await?;
    let mut manifest: CargoManifest = toml::from_str(&cargo_content)?;

    let preview_path = waterui_path.join("components/preview");
    manifest.dependencies.insert(
        "waterui-preview".to_string(),
        Dependency::Detailed(Box::new(DependencyDetail {
            path: Some(preview_path.display().to_string()),
            ..Default::default()
        })),
    );

    let updated = toml::to_string_pretty(&manifest)?;
    smol::fs::write(&cargo_path, updated).await?;

    // Overwrite lib.rs with preview template
    let lib_template = include_str!("../templates/preview/src/lib.rs.tpl");
    smol::fs::write(project.root().join("src/lib.rs"), lib_template).await?;

    info!("Preview app scaffolded at {}", path.display());
    Ok(())
}
