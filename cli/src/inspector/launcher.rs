//! Inspector app launcher and session management.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use color_eyre::eyre::{Context as _, Result, bail};
use tracing::info;

use crate::device::{Device, Local, RunOptions, Running};
use crate::platform::TargetPlatform;
use crate::project::Project;
use crate::runtime_compat::runtime_profile_tag;
use crate::runtime_fingerprint::compute_runtime_fingerprint;
use crate::templates::TemplateContext;

const INSPECTOR_TEMPLATE_COMMIT: &str = env!("WATERUI_CLI_COMMIT");
const INSPECTOR_METADATA_FILE: &str = ".waterui-inspector-signature";

#[derive(Debug, Clone)]
struct InspectorRequirements {
    waterui_root: PathBuf,
    runtime_fingerprint: String,
}

/// Target platform for launching Inspector app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorPlatform {
    /// iOS Simulator.
    IosSimulator,
    /// macOS.
    Macos,
    /// Android (device or emulator).
    Android,
}

/// Launch options for Inspector app.
#[derive(Debug, Clone)]
pub struct InspectorLaunchOptions {
    /// Runtime app endpoint address (`host:port`).
    pub target_addr: String,
    /// One-time token used by runtime endpoint and inspector app.
    pub token: String,
}

/// Inspector session state.
#[derive(Debug)]
pub struct InspectorSession {
    /// Current platform.
    pub platform: InspectorPlatform,
    /// Running handle for apps launched by this session.
    running: Option<Pin<Box<Running>>>,
    /// Whether this session owns the app lifecycle.
    owns_app: bool,
}

impl InspectorSession {
    /// Shutdown the inspector app if this session launched it.
    pub async fn shutdown(&mut self) -> Result<()> {
        if self.owns_app {
            self.running.take();
        }
        Ok(())
    }

    /// Detach inspector app so it keeps running after session drop.
    pub fn detach(&mut self) {
        if let Some(running) = self.running.take() {
            std::mem::forget(running);
            self.owns_app = false;
        }
    }
}

/// Launch (or relaunch) an inspector support app.
pub async fn launch_inspector_session(
    project_path: &Path,
    platform: InspectorPlatform,
    options: InspectorLaunchOptions,
) -> Result<InspectorSession> {
    let requirements = resolve_inspector_requirements(project_path).await?;

    let inspector_app_path = inspector_support_path()?;
    ensure_inspector_support_app(&inspector_app_path, &requirements).await?;

    let project = Project::open(&inspector_app_path)
        .await
        .wrap_err("Failed to open inspector support app project")?;

    let mut run_options = RunOptions::new();
    run_options.insert_env_var(
        "WATERUI_INSPECTOR_TARGET_ADDR".to_string(),
        options.target_addr.clone(),
    );
    run_options.insert_env_var("WATERUI_INSPECTOR_TOKEN".to_string(), options.token.clone());

    let running = match platform {
        InspectorPlatform::Macos => {
            let backend = project
                .apple_backend()
                .ok_or_else(|| color_eyre::eyre::eyre!("Apple backend not configured"))?;
            let device = Local;
            device.launch().await?;
            info!("Building and running inspector app on macOS...");
            project
                .run_with_options(backend, TargetPlatform::MacOS, device, run_options)
                .await
                .map_err(|e| color_eyre::eyre::eyre!("Failed to run inspector app: {e}"))?
        }
        InspectorPlatform::IosSimulator => {
            let backend = project
                .apple_backend()
                .ok_or_else(|| color_eyre::eyre::eyre!("Apple backend not configured"))?;
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
            info!("Building and running inspector app on iOS Simulator...");
            project
                .run_with_options(
                    backend,
                    TargetPlatform::IOSSimulator,
                    simulator,
                    run_options,
                )
                .await
                .map_err(|e| color_eyre::eyre::eyre!("Failed to run inspector app: {e}"))?
        }
        InspectorPlatform::Android => {
            let backend = project
                .android_backend()
                .ok_or_else(|| color_eyre::eyre::eyre!("Android backend not configured"))?;

            let devices = crate::android::device::AndroidDevice::scan().await?;
            if let Some(device) = devices.into_iter().next() {
                device.launch().await?;
                info!("Building and running inspector app on Android device...");
                project
                    .run_android_with_options(backend, device, run_options)
                    .await
                    .map_err(|e| color_eyre::eyre::eyre!("Failed to run inspector app: {e}"))?
            } else {
                let avds = crate::android::platform::AndroidPlatform::list_avds().await?;
                let avd_name = avds.into_iter().next().ok_or_else(|| {
                    color_eyre::eyre::eyre!("No Android devices or emulators available.")
                })?;
                let emulator = crate::android::device::AndroidEmulator::open(avd_name).await?;
                emulator.launch().await?;
                info!("Building and running inspector app on Android emulator...");
                project
                    .run_android_with_options(backend, emulator, run_options)
                    .await
                    .map_err(|e| color_eyre::eyre::eyre!("Failed to run inspector app: {e}"))?
            }
        }
    };

    Ok(InspectorSession {
        platform,
        running: Some(Box::pin(running)),
        owns_app: true,
    })
}

fn inspector_support_path() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| color_eyre::eyre::eyre!("Home directory is not available"))?;
    Ok(home.join(".water").join("inspector_support"))
}

async fn ensure_inspector_support_app(
    path: &Path,
    requirements: &InspectorRequirements,
) -> Result<()> {
    let metadata_path = path.join(INSPECTOR_METADATA_FILE);
    let cargo_path = path.join("Cargo.toml");
    let desired_signature = inspector_signature(requirements);

    let mut needs_scaffold = !cargo_path.exists();
    if !needs_scaffold {
        let stored_signature = match smol::fs::read_to_string(&metadata_path).await {
            Ok(signature) => signature,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(color_eyre::eyre::eyre!(
                    "Failed to read inspector metadata {}: {err}",
                    metadata_path.display()
                ));
            }
        };
        if stored_signature.trim() != desired_signature {
            needs_scaffold = true;
        }
    }

    if needs_scaffold {
        if path.exists() {
            remove_dir_all_retry(path).await?;
        }
        info!("Scaffolding inspector app at {}", path.display());
        scaffold_inspector_app(path, requirements).await?;
        smol::fs::write(&metadata_path, desired_signature.as_bytes()).await?;
    } else if !metadata_path.exists() {
        smol::fs::write(&metadata_path, desired_signature.as_bytes()).await?;
    }

    Ok(())
}

async fn remove_dir_all_retry(path: &Path) -> Result<()> {
    const ATTEMPTS: usize = 6;
    for attempt in 0..ATTEMPTS {
        match smol::fs::remove_dir_all(path).await {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err)
                if err.kind() == std::io::ErrorKind::DirectoryNotEmpty
                    && attempt + 1 < ATTEMPTS =>
            {
                smol::Timer::after(Duration::from_millis(50 * (attempt as u64 + 1))).await;
            }
            Err(err) => return Err(err.into()),
        }
    }

    bail!("Failed to remove inspector support directory after retries")
}

async fn scaffold_inspector_app(path: &Path, requirements: &InspectorRequirements) -> Result<()> {
    use crate::project::{CreateOptions, Manifest as WaterManifest, PackageType};

    let waterui_path = requirements.waterui_root.clone();

    let options = CreateOptions {
        name: "WaterUI Inspector".to_string(),
        bundle_identifier: "dev.waterui.inspector".to_string(),
        package_type: PackageType::Playground,
        waterui_path: Some(waterui_path.clone()),
        author: String::new(),
    };

    let project = Project::create(path, options)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to create inspector app: {e}"))?;

    let mut manifest = WaterManifest::open(project.root().join("Water.toml")).await?;
    manifest.package.accessory = false;
    manifest.save(project.root()).await?;

    let ctx = TemplateContext::for_support_playground(
        "WaterUI Inspector",
        "WaterUIInspector",
        project.crate_name().to_string(),
        "dev.waterui.inspector",
        waterui_path,
        false,
        None,
    );

    crate::templates::inspector::scaffold(project.root(), &ctx)
        .await
        .wrap_err("Failed to scaffold embedded inspector app template")?;

    info!("Inspector app scaffolded at {}", path.display());
    Ok(())
}

fn inspector_signature(requirements: &InspectorRequirements) -> String {
    format!(
        "template_commit={INSPECTOR_TEMPLATE_COMMIT}\nwaterui_root={}\nruntime_fingerprint={}\ntemplate_fingerprint={}",
        requirements.waterui_root.display(),
        requirements.runtime_fingerprint,
        crate::templates::inspector::template_fingerprint(),
    )
}

async fn resolve_inspector_requirements(project_path: &Path) -> Result<InspectorRequirements> {
    let current_dir = project_path.to_path_buf();
    let metadata = smol::unblock(move || {
        cargo_metadata::MetadataCommand::new()
            .current_dir(current_dir)
            .exec()
    })
    .await
    .wrap_err("Failed to resolve user project Cargo metadata for inspector compatibility")?;

    let waterui = select_unique_package(&metadata, "waterui")?;
    if waterui.source.is_some() {
        bail!(
            "Inspector requires path-based WaterUI dependencies to guarantee runtime compatibility. \
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

    Ok(InspectorRequirements {
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
            "Multiple `{name}` packages were resolved. Inspector requires a single resolved `{name}` package."
        );
    }
    Ok(first)
}
