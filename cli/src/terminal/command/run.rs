use crate::{ui, util};
use atty::{self, Stream};
use clap::{Args, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use console::style;
use dialoguer::{Select, theme::ColorfulTheme};
use fs2::FileExt;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use skyzen::{
    CreateRouteNode, Route, StaticDir,
    runtime::native as skyzen_runtime,
    websocket::{WebSocket, WebSocketMessage},
};
use std::{
    fs::{self, File},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::fs as tokio_fs;
use waterui_cli::{
    WATERUI_TRACING_PREFIX,
    backend::{
        self,
        android::{configure_rust_android_linker_env, prepare_cmake_env},
    },
    device::{
        self, AndroidDevice, AndroidSelection, AppleSimulatorDevice, Device, DeviceInfo,
        DeviceKind, DevicePlatformFilter, MacosDevice,
    },
    doctor::{
        AnyToolchainIssue,
        toolchain::{self, CheckMode, CheckTarget},
    },
    output,
    platform::{PlatformKind, android::AndroidPlatform, apple::AppleSimulatorKind},
    project::{Config, FailToRun, HotReloadOptions, Project, RunOptions, RunReport, RunStage},
    util as cli_util,
};
type Platform = PlatformKind;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use which::which;

#[derive(ValueEnum, Copy, Clone, Debug)]
enum PlatformArg {
    Web,
    #[value(alias = "mac")]
    Macos,
    #[value(alias = "iphone")]
    Ios,
    #[value(alias = "ipad")]
    Ipados,
    #[value(alias = "watch")]
    Watchos,
    #[value(alias = "tv")]
    Tvos,
    #[value(alias = "vision")]
    Visionos,
    Android,
}

impl From<PlatformArg> for PlatformKind {
    fn from(arg: PlatformArg) -> Self {
        match arg {
            PlatformArg::Web => Self::Web,
            PlatformArg::Macos => Self::Macos,
            PlatformArg::Ios => Self::Ios,
            PlatformArg::Ipados => Self::Ipados,
            PlatformArg::Watchos => Self::Watchos,
            PlatformArg::Tvos => Self::Tvos,
            PlatformArg::Visionos => Self::Visionos,
            PlatformArg::Android => Self::Android,
        }
    }
}

impl From<PlatformKind> for PlatformArg {
    fn from(kind: PlatformKind) -> Self {
        match kind {
            PlatformKind::Web => Self::Web,
            PlatformKind::Macos => Self::Macos,
            PlatformKind::Ios => Self::Ios,
            PlatformKind::Ipados => Self::Ipados,
            PlatformKind::Watchos => Self::Watchos,
            PlatformKind::Tvos => Self::Tvos,
            PlatformKind::Visionos => Self::Visionos,
            PlatformKind::Android => Self::Android,
        }
    }
}

#[derive(Clone, Debug)]
enum RunTarget {
    Platform(PlatformArg),
    Again,
}

fn parse_run_target(value: &str) -> Result<RunTarget, String> {
    if value.eq_ignore_ascii_case("again") {
        return Ok(RunTarget::Again);
    }

    PlatformArg::from_str(value, true)
        .map(RunTarget::Platform)
        .map_err(|_| format!("Invalid platform '{value}'. Use a platform name or 'again'."))
}

#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// Platform to run on (or `again` to repeat the previous `water run`)
    #[arg(value_name = "PLATFORM|again", value_parser = parse_run_target)]
    platform: Option<RunTarget>,

    /// Log filter (RUST_LOG syntax) for logs forwarded to the CLI
    #[arg(long, value_name = "RUST_LOG")]
    log_filter: Option<String>,

    /// Device to run on (e.g. a simulator name or device UDID)
    #[arg(long)]
    device: Option<String>,

    /// Project directory (defaults to current working directory)
    #[arg(long)]
    project: Option<PathBuf>,

    /// Build in release mode
    #[arg(long)]
    release: bool,

    /// Disable sccache
    #[arg(long)]
    no_sccache: bool,

    /// Enable experimental mold linker integration (Linux hosts only)
    #[arg(long)]
    mold: bool,

    /// Disable hot reload (exit after app launch)
    #[arg(long)]
    no_hot_reload: bool,
}

/// Build and launch a `WaterUI` project for the selected platform.
///
/// # Errors
/// Returns an error if toolchain checks fail, builds fail, or launching the target
/// application fails.
///
/// # Panics
/// Panics if the current working directory cannot be determined when `--project` is not set.
#[allow(clippy::needless_pass_by_value)]
pub fn run(args: RunArgs) -> Result<()> {
    if matches!(args.platform, Some(RunTarget::Again)) {
        run_again(args)
    } else {
        run_fresh(args)
    }
}

fn run_fresh(mut args: RunArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let project = Project::open(&project_dir)?;
    let is_playground = project.is_playground();
    let mut config = project.config().clone();
    let is_json = output::global_output_format().is_json();

    if !is_json {
        if is_playground {
            ui::section(format!("Running playground: {}", project.name()));
        } else {
            ui::section(format!("Running: {}", project.name()));
        }
    }

    let had_explicit_platform = matches!(args.platform, Some(RunTarget::Platform(_)));
    let mut platform = match args.platform.take() {
        Some(RunTarget::Platform(arg)) => Some(PlatformKind::from(arg)),
        Some(RunTarget::Again) => unreachable!("run_again handled earlier"),
        None => None,
    };
    let device = args.device.clone();
    let release = args.release;
    let enable_sccache = !args.no_sccache;
    let mold_requested = args.mold;
    let log_filter = args.log_filter.clone();
    let no_hot_reload = args.no_hot_reload;

    if is_json && !had_explicit_platform && device.is_none() {
        bail!(
            "JSON output requires specifying --platform or --device to avoid interactive prompts."
        );
    }

    if platform.is_none() {
        if let Some(device_name) = &device {
            let available_devices = device::list_devices()?;
            let selected_device =
                find_device(&available_devices, device_name).ok_or_else(|| {
                    eyre!(
                        "Device '{}' not found. Run `water devices` to list available targets.",
                        device_name
                    )
                })?;
            platform = Some(platform_from_device(selected_device)?);
        } else if is_playground {
            let backend_choice = prompt_for_playground_backend()?;
            platform = Some(resolve_playground_backend_choice(backend_choice)?);
        } else {
            let backend_choice = prompt_for_backend(&config)?;
            platform = Some(resolve_backend_choice(backend_choice, &config)?);
        }
    }

    // For playground projects, ensure the backend exists in the cache
    if is_playground {
        let platform_kind = platform.ok_or_else(|| eyre!("No platform selected"))?;
        let backend = match platform_kind {
            Platform::Web => super::create::BackendChoice::Web,
            Platform::Android => super::create::BackendChoice::Android,
            Platform::Macos | Platform::Ios | Platform::Ipados | Platform::Tvos | Platform::Visionos => {
                super::create::BackendChoice::Apple
            }
            _ => bail!("Platform {:?} is not supported in playground mode", platform_kind),
        };
        config = super::playground::ensure_playground_backend(
            &project_dir,
            &config,
            project.crate_name(),
            backend,
        )?;
        platform = Some(platform_kind);
    }

    if matches!(platform, Some(Platform::Watchos)) {
        warn!("watchOS support is not available yet.");
        bail!("watchOS backend is not supported yet.");
    }

    let platform = platform.ok_or_else(|| eyre!("No platform selected"))?;

    run_platform(
        platform,
        device,
        &project,
        &config,
        release,
        enable_sccache,
        mold_requested,
        log_filter,
        no_hot_reload,
    )?;

    Ok(())
}

fn run_again(args: RunArgs) -> Result<()> {
    if args.device.is_some() || args.release || args.no_sccache || args.mold || args.no_hot_reload {
        bail!(
            "`water run again` does not accept additional options. Re-run the original command without extra flags."
        );
    }

    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let project = Project::open(&project_dir)?;
    let snapshot = load_last_run(&project)?;
    let replay_args = RunArgs {
        platform: Some(RunTarget::Platform(snapshot.platform.into())),
        log_filter: None,
        device: snapshot.device.clone(),
        project: args.project,
        release: snapshot.release,
        no_sccache: !snapshot.enable_sccache,
        mold: snapshot.mold,
        no_hot_reload: false, // Replay always uses hot reload
    };

    run_fresh(replay_args)
}

#[derive(Clone, Copy)]
enum BackendChoice {
    Platform(Platform),
    AppleAggregate,
}

fn prompt_for_backend(config: &Config) -> Result<BackendChoice> {
    let mut options: Vec<(BackendChoice, String)> = Vec::new();

    if config.backends.web.is_some() {
        options.push((
            BackendChoice::Platform(Platform::Web),
            "Web Browser".to_string(),
        ));
    }

    if config.backends.swift.is_some() {
        options.push((
            BackendChoice::AppleAggregate,
            "Apple (macOS, iOS, iPadOS, tvOS, visionOS)".to_string(),
        ));
    }

    if config.backends.android.is_some() {
        options.push((
            BackendChoice::Platform(Platform::Android),
            "Android".to_string(),
        ));
    }

    if options.is_empty() {
        bail!(
            "No runnable targets found. Please connect a device, start a simulator, or enable a backend in your Water.toml."
        );
    }

    let default_index = default_backend_index(&options);
    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();
    let selection = if is_interactive_terminal() {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a backend to run")
            .items(&labels)
            .default(default_index)
            .interact()?
    } else {
        if !output::global_output_format().is_json() {
            ui::info(format!(
                "Non-interactive terminal detected; using {}.",
                options[default_index].1
            ));
        }
        default_index
    };

    Ok(options[selection].0)
}

fn resolve_backend_choice(choice: BackendChoice, config: &Config) -> Result<Platform> {
    match choice {
        BackendChoice::Platform(platform) => Ok(platform),
        BackendChoice::AppleAggregate => prompt_for_apple_platform(config),
    }
}

/// Prompt for backend selection in playground mode (no pre-configured backends).
fn prompt_for_playground_backend() -> Result<BackendChoice> {
    let mut options: Vec<(BackendChoice, String)> = Vec::new();

    // Web is always available
    options.push((
        BackendChoice::Platform(Platform::Web),
        "Web Browser".to_string(),
    ));

    // Apple is only available on macOS
    #[cfg(target_os = "macos")]
    options.push((
        BackendChoice::AppleAggregate,
        "Apple (macOS, iOS, iPadOS, tvOS, visionOS)".to_string(),
    ));

    // Android is available if Android SDK might be installed
    options.push((
        BackendChoice::Platform(Platform::Android),
        "Android".to_string(),
    ));

    let default_index = default_backend_index(&options);
    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();
    let selection = if is_interactive_terminal() {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a platform to run")
            .items(&labels)
            .default(default_index)
            .interact()?
    } else {
        if !output::global_output_format().is_json() {
            ui::info(format!(
                "Non-interactive terminal detected; using {}.",
                options[default_index].1
            ));
        }
        default_index
    };

    Ok(options[selection].0)
}

/// Resolve playground backend choice to a platform.
fn resolve_playground_backend_choice(choice: BackendChoice) -> Result<Platform> {
    match choice {
        BackendChoice::Platform(platform) => Ok(platform),
        BackendChoice::AppleAggregate => prompt_for_playground_apple_platform(),
    }
}

/// Prompt for Apple platform in playground mode.
fn prompt_for_playground_apple_platform() -> Result<Platform> {
    let mut spinner = ui::spinner("Scanning Apple devices...");
    let scan_started = Instant::now();
    debug!("Starting Apple device scan for playground");
    let devices = match device::list_devices_filtered(DevicePlatformFilter::Apple) {
        Ok(list) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Apple device scan for playground completed in {:?} with {} device(s)",
                scan_started.elapsed(),
                list.len()
            );
            list
        }
        Err(err) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Apple device scan for playground failed after {:?}: {err:?}",
                scan_started.elapsed()
            );
            return Err(err);
        }
    };

    let mut has_ios = false;
    let mut has_ipados = false;
    let mut has_tvos = false;
    let mut has_visionos = false;

    for device in &devices {
        if let Ok(platform) = platform_from_device(device) {
            match platform {
                Platform::Ios => has_ios = true,
                Platform::Ipados => has_ipados = true,
                Platform::Tvos => has_tvos = true,
                Platform::Visionos => has_visionos = true,
                _ => {}
            }
        }
    }

    let mut options: Vec<(Platform, String)> = vec![(Platform::Macos, "Apple: macOS".to_string())];
    if has_ios {
        options.push((Platform::Ios, "Apple: iOS".to_string()));
    }
    if has_ipados {
        options.push((Platform::Ipados, "Apple: iPadOS".to_string()));
    }
    if has_tvos {
        options.push((Platform::Tvos, "Apple: tvOS".to_string()));
    }
    if has_visionos {
        options.push((Platform::Visionos, "Apple: visionOS".to_string()));
    }

    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();
    let selection = if is_interactive_terminal() {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select an Apple platform")
            .items(&labels)
            .default(0)
            .interact()?
    } else {
        if !output::global_output_format().is_json() {
            ui::info(format!(
                "Non-interactive terminal detected; using {}.",
                options[0].1
            ));
        }
        0
    };

    Ok(options[selection].0)
}

fn default_backend_index(options: &[(BackendChoice, String)]) -> usize {
    if !cfg!(target_os = "macos") {
        return 0;
    }

    options
        .iter()
        .position(|(choice, _)| matches!(choice, BackendChoice::AppleAggregate))
        .unwrap_or(0)
}

fn prompt_for_apple_platform(config: &Config) -> Result<Platform> {
    if config.backends.swift.is_none() {
        bail!("Apple backend is not configured for this project.");
    }

    let mut spinner = ui::spinner("Scanning Apple devices...");
    let scan_started = Instant::now();
    debug!("Starting Apple device scan for prompt_for_apple_platform");
    let devices = match device::list_devices_filtered(DevicePlatformFilter::Apple) {
        Ok(list) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Apple device scan for prompt_for_apple_platform completed in {:?} with {} device(s)",
                scan_started.elapsed(),
                list.len()
            );
            list
        }
        Err(err) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Apple device scan for prompt_for_apple_platform failed after {:?}: {err:?}",
                scan_started.elapsed()
            );
            return Err(err);
        }
    };
    let mut has_ios = false;
    let mut has_ipados = false;
    let mut has_tvos = false;
    let mut has_visionos = false;

    for device in &devices {
        if let Ok(platform) = platform_from_device(device) {
            match platform {
                Platform::Ios => has_ios = true,
                Platform::Ipados => has_ipados = true,
                Platform::Tvos => has_tvos = true,
                Platform::Visionos => has_visionos = true,
                _ => {}
            }
        }
    }

    let mut options: Vec<(Platform, String)> = vec![(Platform::Macos, "Apple: macOS".to_string())];
    if has_ios {
        options.push((Platform::Ios, "Apple: iOS".to_string()));
    }
    if has_ipados {
        options.push((Platform::Ipados, "Apple: iPadOS".to_string()));
    }
    if has_tvos {
        options.push((Platform::Tvos, "Apple: tvOS".to_string()));
    }
    if has_visionos {
        options.push((Platform::Visionos, "Apple: visionOS".to_string()));
    }

    if options.is_empty() {
        bail!("No Apple targets detected. Connect a device or install a simulator.");
    }

    if options.len() == 1 {
        return Ok(options[0].0);
    }

    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();
    let selection = if is_interactive_terminal() {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select an Apple target")
            .items(&labels)
            .default(0)
            .interact()?
    } else {
        if !output::global_output_format().is_json() {
            ui::info(format!(
                "Non-interactive terminal detected; using {}.",
                options[0].1
            ));
        }
        0
    };

    Ok(options[selection].0)
}

fn find_device<'a>(devices: &'a [DeviceInfo], query: &str) -> Option<&'a DeviceInfo> {
    devices
        .iter()
        .find(|device| device.identifier == query || device.name == query)
}

fn platform_from_device(device: &DeviceInfo) -> Result<Platform> {
    match device.platform.as_str() {
        "Web" => Ok(Platform::Web),
        "macOS" => Ok(Platform::Macos),
        p if p.starts_with("iOS") => Ok(Platform::Ios),
        p if p.starts_with("iPadOS") => Ok(Platform::Ipados),
        p if p.starts_with("watchOS") => Ok(Platform::Watchos),
        p if p.starts_with("tvOS") => Ok(Platform::Tvos),
        p if p.starts_with("visionOS") => Ok(Platform::Visionos),
        "Android" => Ok(Platform::Android),
        _ => bail!("Unsupported platform: {}", device.platform),
    }
}

fn toolchain_targets_for_platform(platform: Platform) -> Vec<CheckTarget> {
    let mut targets = vec![CheckTarget::Rust];
    if platform.is_apple_platform() {
        targets.push(CheckTarget::Swift);
    }

    if platform == Platform::Android {
        targets.push(CheckTarget::Android);
    }
    targets
}

const fn platform_supports_native_hot_reload(platform: Platform) -> bool {
    platform.is_apple_platform() || matches!(platform, Platform::Android)
}

fn hot_reload_target(platform: Platform) -> Option<String> {
    match platform {
        Platform::Android => Some("aarch64-linux-android".to_string()),
        _ => None,
    }
}

fn hot_reload_library_path(
    project_dir: &Path,
    crate_name: &str,
    release: bool,
    target_triple: Option<&str>,
) -> PathBuf {
    let profile = if release { "release" } else { "debug" };
    let filename = dylib_filename(crate_name, target_triple);
    let mut path = project_dir.join("target");
    if let Some(target) = target_triple {
        path = path.join(target);
    }
    path.join(profile).join(filename)
}

fn dylib_filename(crate_name: &str, target_triple: Option<&str>) -> String {
    let normalized = crate_name.replace('-', "_");
    let target = target_triple.unwrap_or_else(|| std::env::consts::OS);

    let target_lower = target.to_ascii_lowercase();
    let (prefix, suffix) = if target_lower.contains("windows") {
        ("", "dll")
    } else if target_lower.contains("darwin")
        || target_lower.contains("apple")
        || target_lower.contains("ios")
        || target_lower.contains("macos")
    {
        ("lib", "dylib")
    } else {
        ("lib", "so")
    };

    format!("{prefix}{normalized}.{suffix}")
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn run_platform(
    platform: Platform,
    mut device: Option<String>,
    project: &Project,
    config: &Config,
    release: bool,
    enable_sccache: bool,
    mold_requested: bool,
    log_filter: Option<String>,
    no_hot_reload: bool,
) -> Result<()> {
    let project_dir = project.root();
    let mut recorded_device = device.clone();
    let hot_reload_target_triple = hot_reload_target(platform);
    if let Err(report) =
        toolchain::ensure_ready(CheckMode::Quick, &toolchain_targets_for_platform(platform))
    {
        let details = report.to_string();
        let mut message = format!("Toolchain not ready for {platform:?}");
        let trimmed = details.trim();
        if trimmed.is_empty() {
            message.push('.');
        } else {
            message.push_str(":\n\n");
            message.push_str(&indent_lines(trimmed, "  "));
        }

        bail!(message);
    }

    if platform == Platform::Web {
        return run_web(project_dir, config, project.crate_name(), release, log_filter);
    }

    // Hot reload is enabled unless --no-hot-reload is passed
    let hot_reload_enabled = !no_hot_reload && platform_supports_native_hot_reload(platform);
    if platform == Platform::Android && hot_reload_enabled {
        if let Some(target) = hot_reload_target_triple.as_deref() {
            configure_rust_android_linker_env(&[target])?;
        }
    }

    // Acquire file lock on the library path to prevent concurrent hot reload runs.
    // We create a lock file in the .water directory rather than locking the library itself,
    // since the library may not exist yet and we need to hold the lock during compilation.
    let lock_file_path = project_dir.join(".water").join("hot-reload.lock");
    let _lock_guard = if hot_reload_enabled {
        Some(acquire_hot_reload_lock(&lock_file_path)?)
    } else {
        None
    };

    // For Android, we need to start the hot reload server BEFORE launching the app,
    // because the ADB reverse tunnel and app launch require the port. For Apple platforms,
    // the app connects to the server after launch, so we can start it later.
    let early_server = if hot_reload_enabled && platform == Platform::Android {
        let server = Server::start(project_dir.to_path_buf(), log_filter.clone())
            .context("Failed to start hot reload server")?;
        Some(server)
    } else {
        None
    };

    let hot_reload_port = early_server.as_ref().map(|s| s.address().port());

    // Build run options
    let mut run_options = RunOptions {
        release,
        hot_reload: HotReloadOptions {
            enabled: hot_reload_enabled,
            port: hot_reload_port,
        },
        log_filter: log_filter.clone(),
    };

    // First, package the app (this triggers the Rust build via Xcode/Gradle)
    let run_report = match platform {
        Platform::Macos => {
            let swift_config = config.backends.swift.clone().ok_or_else(|| {
                eyre!(
                    "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the Apple backend."
                )
            })?;
            if !output::global_output_format().is_json() {
                ui::info(format!("Xcode scheme: {}", swift_config.scheme));
            }
            let scheme_name = swift_config.scheme.clone();
            let spinner_msg = apple_build_progress_message(Platform::Macos, &scheme_name, None);
            let device_impl = MacosDevice::new(swift_config);
            run_with_package_spinner(project, device_impl, run_options.clone(), spinner_msg)?
        }
        Platform::Ios
        | Platform::Ipados
        | Platform::Watchos
        | Platform::Tvos
        | Platform::Visionos => {
            let swift_config = config.backends.swift.clone().ok_or_else(|| {
                eyre!(
                    "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the Apple backend."
                )
            })?;
            if !output::global_output_format().is_json() {
                ui::info(format!("Xcode scheme: {}", swift_config.scheme));
            }
            let simulator_kind = apple_simulator_kind(platform);
            let device_name = match device.take() {
                Some(name) => name,
                None => prompt_for_apple_device(platform)?,
            };
            recorded_device = Some(device_name.clone());
            let scheme_name = swift_config.scheme.clone();
            let target_label = format!(
                "{} ({})",
                device_name,
                apple_platform_display_name(platform)
            );
            let spinner_msg =
                apple_build_progress_message(platform, &scheme_name, Some(target_label.as_str()));
            let simulator = AppleSimulatorDevice::new(swift_config, simulator_kind, device_name);
            run_with_package_spinner(project, simulator, run_options.clone(), spinner_msg)?
        }
        Platform::Android => {
            let android_config = config.backends.android.clone().ok_or_else(|| {
                eyre!(
                    "Android backend not configured for this project. Add it to Water.toml or recreate the project with the Android backend."
                )
            })?;
            let selection = match device.take() {
                Some(name) => resolve_android_device(&name)?,
                None => prompt_for_android_device()?,
            };
            let stored_id = selection
                .identifier
                .clone()
                .unwrap_or(selection.name.clone());
            recorded_device = Some(stored_id);
            let platform_impl = AndroidPlatform::new(android_config);
            let android_device = AndroidDevice::new(platform_impl, selection)?;
            run_on_device(project, android_device, run_options.clone())?
        }
        Platform::Web => unreachable!(),
    };

    // Persist last run configuration
    if let Err(err) = persist_last_run(
        project,
        LastRunSnapshot {
            platform,
            device: recorded_device,
            release,
            enable_sccache,
            mold: mold_requested,
            timestamp: current_timestamp(),
        },
    ) {
        warn!("Failed to record last run configuration: {err:?}");
    }

    if !output::global_output_format().is_json() {
        ui::success(format!(
            "Application built: {}",
            run_report.artifact.display()
        ));
    }

    // If hot reload is disabled, we're done after launching the app
    if !hot_reload_enabled {
        if !output::global_output_format().is_json() {
            ui::info("App launched (hot reload disabled)");
        }
        if output::global_output_format().is_json() {
            output::emit_json(&run_report)?;
        }
        return Ok(());
    }

    // Start hot reload server now that packaging succeeded (or reuse the early server for Android)
    let server = match early_server {
        Some(s) => s,
        None => Server::start(project_dir.to_path_buf(), log_filter.clone())
            .context("Failed to start hot reload server")?,
    };
    let connection_events = server.connection_events();
    let hot_reload_port = Some(server.address().port());

    // Update run_options with the actual port for the rebuild callback (for non-Android)
    run_options.hot_reload.port = hot_reload_port;

    // Notify the hot reload server if a library already exists from the build
    let library_path = hot_reload_library_path(
        project_dir,
        project.crate_name(),
        release,
        hot_reload_target_triple.as_deref(),
    );
    if library_path.exists() {
        server.notify_native_reload(library_path);
    }

    // Set up file watcher for hot reload
    let mut watch_paths = vec![project_dir.join("src")];
    for path in &config.hot_reload.watch {
        watch_paths.push(project_dir.join(path));
    }

    let project_dir_buf = project_dir.to_path_buf();
    let package_name = project.crate_name().to_string();
    let hot_reload_target_clone = hot_reload_target_triple.clone();
    let build_callback: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
        run_cargo_build(
            &project_dir_buf,
            &package_name,
            release,
            true, // hot_reload_enabled
            hot_reload_port,
            enable_sccache,
            mold_requested,
            hot_reload_target_clone.as_deref(),
        )?;
        let library_path = hot_reload_library_path(
            &project_dir_buf,
            &package_name,
            release,
            hot_reload_target_clone.as_deref(),
        );
        server.notify_native_reload(library_path);
        Ok(())
    });

    let watcher = RebuildWatcher::new(watch_paths, &build_callback)?;

    if !output::global_output_format().is_json() {
        ui::info("App launched with hot reload enabled");
        ui::plain("Press Ctrl+C to stop the watcher");
    }

    // Wait for app to connect to hot reload server
    if !output::global_output_format().is_json() {
        ui::info("Waiting for the app to connect to the hot reload server…");
    }
    wait_for_hot_reload_connection(&connection_events)?;

    // Wait for user interrupt or connection loss
    match wait_for_interrupt(Some(connection_events))? {
        WaitOutcome::Interrupted => {}
        WaitOutcome::ConnectionLost(reason) => {
            bail!(format_connection_loss_message(reason));
        }
    }

    drop(watcher);
    if let Some(crash) = &run_report.crash_report {
        if !output::global_output_format().is_json() {
            ui::warning(format!(
                "App crashed — logs saved to {}",
                crash.log_path.display()
            ));
            if let Some(summary) = &crash.summary {
                ui::plain(format!("Crash summary: {summary}"));
            }
        }
    }

    if output::global_output_format().is_json() {
        output::emit_json(&run_report)?;
    }

    Ok(())
}

/// Acquires an exclusive file lock on the hot reload lock file.
/// Returns a guard that releases the lock when dropped.
fn acquire_hot_reload_lock(lock_path: &Path) -> Result<HotReloadLockGuard> {
    use std::io::{Read, Seek, Write};

    // Ensure the parent directory exists
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .with_context(|| format!("Failed to open lock file: {}", lock_path.display()))?;

    // Try to acquire an exclusive lock without blocking
    match file.try_lock_exclusive() {
        Ok(()) => {
            // Write our PID to the lock file
            file.set_len(0)?;
            file.seek(std::io::SeekFrom::Start(0))?;
            write!(file, "{}", std::process::id())?;
            file.flush()?;
            debug!("Acquired hot reload lock: {}", lock_path.display());
            Ok(HotReloadLockGuard {
                file,
                lock_path: lock_path.to_path_buf(),
            })
        }
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            // Check if the process holding the lock is still alive
            let mut contents = String::new();
            if file.read_to_string(&mut contents).is_ok() {
                if let Ok(pid) = contents.trim().parse::<u32>() {
                    if !is_process_running(pid) {
                        // The process is dead, the lock is stale
                        // Try to remove the lock file and retry
                        drop(file);
                        if fs::remove_file(lock_path).is_ok() {
                            info!("Removed stale hot reload lock (PID {pid} no longer running)");
                            // Retry acquiring the lock
                            return acquire_hot_reload_lock(lock_path);
                        }
                    }
                }
            }
            bail!(
                "Another `water run` process is already running with hot reload enabled.\n\
                 Only one hot reload session is allowed at a time to prevent conflicts.\n\
                 Stop the other process or use `--no-hot-reload` to run without hot reload."
            );
        }
        Err(err) => {
            Err(err).with_context(|| format!("Failed to acquire lock: {}", lock_path.display()))
        }
    }
}

/// Check if a process with the given PID is running.
fn is_process_running(pid: u32) -> bool {
    // Use `kill -0` to check if a process exists (works on macOS/Linux)
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, assume the process is running to be safe
        true
    }
}

/// Guard that holds an exclusive file lock and releases it when dropped.
/// The lock file uses flock (file system lock) which is automatically released
/// when the file descriptor is closed (process exit, including crashes).
struct HotReloadLockGuard {
    file: File,
    #[allow(dead_code)]
    lock_path: std::path::PathBuf,
}

impl Drop for HotReloadLockGuard {
    fn drop(&mut self) {
        // Explicitly unlock the file. The lock is also released when the file
        // descriptor is closed, but this makes it more explicit.
        if let Err(err) = self.file.unlock() {
            warn!("Failed to release hot reload lock: {err}");
        }
        // Note: We intentionally do NOT remove the lock file. The file itself
        // is just a vessel for the flock. Removing it could cause race conditions
        // if another process is trying to acquire the lock at the same time.
    }
}

fn run_on_device<D>(project: &Project, device: D, options: RunOptions) -> Result<RunReport>
where
    D: Device,
{
    run_on_device_with_observer(project, device, options, |_| {})
}

fn run_on_device_with_observer<D, O>(
    project: &Project,
    device: D,
    options: RunOptions,
    observer: O,
) -> Result<RunReport>
where
    D: Device,
    O: FnMut(RunStage),
{
    project
        .run_with_observer(&device, options, observer)
        .map_err(convert_run_error)
}

fn run_with_package_spinner<D>(
    project: &Project,
    device: D,
    options: RunOptions,
    _spinner_msg: String,
) -> Result<RunReport>
where
    D: Device,
{
    // Note: We don't show a spinner during packaging because both Cargo
    // and xcodebuild/Gradle have their own progress output that would
    // conflict with the spinner animation.
    run_on_device_with_observer(project, device, options, |_stage| {})
}

fn convert_run_error(err: FailToRun) -> color_eyre::eyre::Report {
    match err {
        FailToRun::BuildError(message) => eyre!(message),
        FailToRun::RequirementNotMet(issues) => {
            emit_toolchain_error("toolchain", &issues);
            eyre!(
                "toolchain requirements are not satisfied. Run `water doctor` for automatic fixes."
            )
        }
        FailToRun::Other(report) => report,
    }
}

fn emit_toolchain_error(label: &str, issues: &[AnyToolchainIssue]) {
    if output::global_output_format().is_json() {
        // JSON consumers rely on stderr being quiet, so let the structured error bubble up.
        return;
    }

    eprintln!(
        "{} {}",
        style("Error:").red().bold(),
        style(format!("{label} requirements are not satisfied"))
            .red()
            .bold()
    );

    for issue in issues {
        eprintln!("  {} {}", style("•").red(), style(issue).bold());
        let suggestion_text = issue.suggestion();
        let suggestion = suggestion_text.trim();
        if !suggestion.is_empty() {
            eprintln!(
                "    {} {}",
                style("suggestion:").dim(),
                style(suggestion).dim()
            );
        }
    }

    eprintln!(
        "  {} {}",
        style("→").yellow(),
        style("Run `water doctor` for automatic fixes.").yellow()
    );
}

fn run_cargo_build(
    project_dir: &Path,
    package: &str,
    release: bool,
    hot_reload_enabled: bool,
    hot_reload_port: Option<u16>,
    enable_sccache: bool,
    mold_requested: bool,
    hot_reload_target: Option<&str>,
) -> Result<()> {
    if let Some(target) = hot_reload_target {
        if target.contains("android") {
            configure_rust_android_linker_env(&[target])?;
            prepare_cmake_env(&[target])?;
        }
    }

    if !output::global_output_format().is_json() {
        ui::step("Compiling Rust library...");
    }
    let make_command = || {
        let mut cmd = Command::new("cargo");
        cmd.arg("build").arg("--package").arg(package);
        if let Some(target) = hot_reload_target {
            cmd.arg("--target").arg(target);
        }
        if release {
            cmd.arg("--release");
        }
        cmd.current_dir(project_dir);
        cli_util::configure_hot_reload_env(&mut cmd, hot_reload_enabled, hot_reload_port);
        configure_hot_reload_build_profile(&mut cmd, release);
        cmd
    };

    let mut cmd = make_command();
    let sccache_enabled =
        cli_util::configure_build_speedups(&mut cmd, enable_sccache, mold_requested);
    debug!("Running command: {:?}", cmd);
    let status = cli_util::run_command_interruptible(cmd)
        .with_context(|| format!("failed to run cargo build in {}", project_dir.display()))?;
    if status.success() {
        if hot_reload_enabled && hot_reload_target.is_some() {
            ensure_cdylib(project_dir, package, release, hot_reload_target)?;
        }
        // For Android, copy the .so to jniLibs so Gradle can package it
        if let Some(target) = hot_reload_target {
            if target.contains("android") {
                copy_android_library_to_jnilibs(project_dir, package, release, target)?;
            }
        }
        return Ok(());
    }

    if sccache_enabled {
        warn!("cargo build failed when using sccache; retrying without build cache");
        let mut retry_cmd = make_command();
        cli_util::configure_build_speedups(&mut retry_cmd, false, mold_requested);
        debug!("Running command without sccache: {:?}", retry_cmd);
        let retry_status = cli_util::run_command_interruptible(retry_cmd).with_context(|| {
            format!(
                "failed to rerun cargo build without sccache in {}",
                project_dir.display()
            )
        })?;
        if retry_status.success() {
            info!("cargo build succeeded after disabling sccache");
            if hot_reload_enabled && hot_reload_target.is_some() {
                ensure_cdylib(project_dir, package, release, hot_reload_target)?;
            }
            return Ok(());
        }
    }

    bail!("cargo build failed");
}

fn ensure_cdylib(
    project_dir: &Path,
    package: &str,
    release: bool,
    hot_reload_target: Option<&str>,
) -> Result<()> {
    let expected = hot_reload_library_path(project_dir, package, release, hot_reload_target);
    if expected.exists() {
        strip_artifact_if_needed(&expected, hot_reload_target);
        return Ok(());
    }

    debug!(
        "Hot reload library missing at {}; forcing cargo rustc --crate-type cdylib",
        expected.display()
    );

    let mut cmd = Command::new("cargo");
    cmd.arg("rustc").arg("--package").arg(package).arg("--lib");
    if let Some(target) = hot_reload_target {
        cmd.arg("--target").arg(target);
    }
    if release {
        cmd.arg("--release");
    }
    cmd.arg("--");
    cmd.arg("--crate-type").arg("cdylib");
    cmd.current_dir(project_dir);
    cli_util::configure_hot_reload_env(&mut cmd, true, None);
    configure_hot_reload_build_profile(&mut cmd, release);
    debug!("Running command: {:?}", cmd);
    let status = cmd.status().with_context(|| {
        format!(
            "failed to run cargo rustc to force cdylib in {}",
            project_dir.display()
        )
    })?;
    if !status.success() {
        bail!("cargo rustc --crate-type cdylib failed");
    }

    if expected.exists() {
        strip_artifact_if_needed(&expected, hot_reload_target);
        return Ok(());
    }

    let profile = if release { "release" } else { "debug" };
    let mut deps_dir = project_dir.join("target");
    if let Some(target) = hot_reload_target {
        deps_dir.push(target);
    }
    deps_dir.push(profile);
    deps_dir.push("deps");

    let filename = dylib_filename(package, hot_reload_target);
    let suffix = filename
        .rsplit('.')
        .next()
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    let prefix = if filename.starts_with("lib") {
        "lib"
    } else {
        ""
    };
    let normalized = package.replace('-', "_");

    if let Ok(entries) = fs::read_dir(&deps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let has_prefix = if prefix.is_empty() {
                    name.starts_with(&normalized)
                } else {
                    name.starts_with(&format!("{prefix}{normalized}"))
                };
                if has_prefix && name.ends_with(&suffix) {
                    if let Some(parent) = expected.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::copy(&path, &expected)?;
                    debug!(
                        "Copied cdylib from {} to {}",
                        path.display(),
                        expected.display()
                    );
                    strip_artifact_if_needed(&expected, hot_reload_target);
                    return Ok(());
                }
            }
        }
    }

    bail!(
        "Failed to locate generated cdylib after forcing build; expected {}",
        expected.display()
    );
}

/// Copy the compiled Android .so library to jniLibs so Gradle can package it.
/// This ensures the APK contains the native library even if Gradle's buildRustLibraries
/// task is skipped or doesn't run.
fn copy_android_library_to_jnilibs(
    project_dir: &Path,
    package: &str,
    release: bool,
    target: &str,
) -> Result<()> {
    let profile = if release { "release" } else { "debug" };
    let crate_file = package.replace('-', "_");
    let source = project_dir
        .join("target")
        .join(target)
        .join(profile)
        .join(format!("lib{crate_file}.so"));

    if !source.exists() {
        debug!(
            "Android library not found at {}; skipping jniLibs copy",
            source.display()
        );
        return Ok(());
    }

    // Map target triple to Android ABI
    let abi = match target {
        "aarch64-linux-android" => "arm64-v8a",
        "armv7-linux-androideabi" => "armeabi-v7a",
        "x86_64-linux-android" => "x86_64",
        "i686-linux-android" => "x86",
        _ => {
            debug!("Unknown Android target {target}; skipping jniLibs copy");
            return Ok(());
        }
    };

    let jni_dir = project_dir.join("android/app/src/main/jniLibs").join(abi);
    fs::create_dir_all(&jni_dir)
        .with_context(|| format!("failed to create jniLibs directory {}", jni_dir.display()))?;

    // Copy the main library with standardized name (libwaterui_app.so)
    // This convention allows the Android backend to always load "waterui_app"
    // regardless of the actual crate name
    let dest = jni_dir.join("libwaterui_app.so");
    fs::copy(&source, &dest)
        .with_context(|| format!("failed to copy library to {}", dest.display()))?;
    debug!(
        "Copied Android library {} -> {}",
        source.display(),
        dest.display()
    );

    // Also copy libc++_shared.so from NDK - required for C++ runtime
    if let Some(ndk_path) = backend::android::resolve_ndk_path() {
        // Find the prebuilt directory - NDK uses darwin-x86_64 even on Apple Silicon
        let prebuilt_base = ndk_path.join("toolchains/llvm/prebuilt");
        let host_tags = [
            "darwin-x86_64",  // macOS (including Apple Silicon via Rosetta)
            "darwin-arm64",   // macOS native arm64 (some newer NDKs)
            "linux-x86_64",   // Linux
            "windows-x86_64", // Windows
        ];

        let libcxx_src = host_tags
            .iter()
            .map(|tag| {
                prebuilt_base
                    .join(tag)
                    .join("sysroot/usr/lib")
                    .join(target)
                    .join("libc++_shared.so")
            })
            .find(|p| p.exists());

        if let Some(libcxx_src) = libcxx_src {
            let libcxx_dst = jni_dir.join("libc++_shared.so");
            fs::copy(&libcxx_src, &libcxx_dst).with_context(|| {
                format!(
                    "failed to copy libc++_shared.so to {}",
                    libcxx_dst.display()
                )
            })?;
            debug!("Copied libc++_shared.so to {}", libcxx_dst.display());
        } else {
            warn!("libc++_shared.so not found in NDK for target {}", target);
        }
    }

    Ok(())
}

fn strip_artifact_if_needed(path: &Path, target_triple: Option<&str>) {
    // Only try to strip Android artifacts to shrink websocket payloads.
    if !target_triple.is_some_and(|t| t.contains("android")) {
        return;
    }

    if !path.exists() {
        return;
    }

    let before = path.metadata().ok().map(|m| m.len()).unwrap_or(0);
    if let Some(strip) = find_llvm_strip() {
        // Use --strip-debug to only remove debug symbols while preserving the
        // dynamic symbol table needed for FFI exports (dlopen/dlsym)
        let status = Command::new(strip)
            .arg("--strip-debug")
            .arg(path)
            .status()
            .map_err(|e| {
                debug!("Failed to run llvm-strip: {e}");
                e
            });
        if let Ok(status) = status {
            if status.success() {
                let after = path.metadata().ok().map(|m| m.len()).unwrap_or(before);
                info!(
                    "Stripped hot reload artifact ({} -> {} bytes)",
                    before, after
                );
                return;
            }
        }
    }

    debug!(
        "llvm-strip not found or failed; skipping strip for {}",
        path.display()
    );
}

fn find_llvm_strip() -> Option<PathBuf> {
    if let Ok(path) = which("llvm-strip") {
        return Some(path);
    }

    if let Some(ndk_root) = backend::android::resolve_ndk_path() {
        let host_tag = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "darwin-arm64"
        } else if cfg!(target_os = "macos") {
            "darwin-x86_64"
        } else if cfg!(target_os = "linux") {
            "linux-x86_64"
        } else if cfg!(target_os = "windows") {
            "windows-x86_64"
        } else {
            "unknown"
        };

        let candidate = ndk_root
            .join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .join(host_tag)
            .join("bin")
            .join("llvm-strip");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn configure_hot_reload_build_profile(cmd: &mut Command, release: bool) {
    if release {
        return;
    }

    // Keep dev artifacts small enough for the 16MB websocket frame limit during hot reload.
    cmd.env("CARGO_PROFILE_DEV_DEBUG", "1");
    cmd.env("CARGO_PROFILE_DEV_SPLIT_DEBUGINFO", "packed");
}

fn indent_lines(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_web(
    project_dir: &Path,
    config: &Config,
    crate_name: &str,
    release: bool,
    log_filter: Option<String>,
) -> Result<()> {
    let web_config = config.backends.web.as_ref().ok_or_else(|| {
        eyre!("Web backend not configured for this project. Add it to Water.toml or recreate the project with the web backend.")
    })?;

    util::require_tool(
        "wasm-pack",
        "Install it from https://rustwasm.github.io/wasm-pack/ and try again.",
    )?;
    let wasm_pack = which("wasm-pack").context("failed to locate wasm-pack on PATH")?;

    let web_dir = project_dir.join(&web_config.project_path);
    if !web_dir.exists() {
        bail!(
            "Web assets directory '{}' does not exist. Ensure the project was created with the web backend.",
            web_dir.display()
        );
    }

    build_web_app(
        project_dir,
        crate_name,
        &web_dir,
        release,
        &wasm_pack,
        false,
    )?;

    let server = Server::start(web_dir.clone(), log_filter)?;
    let address = server.address();
    let url = format!("http://{address}/");

    // Hot reload is always enabled - set up file watcher
    let main_js_path = web_dir.join("main.js");
    let main_js_template = std::fs::read_to_string(&main_js_path)?;
    let main_js = main_js_template.replace("__HOT_RELOAD_PORT__", &address.port().to_string());
    std::fs::write(&main_js_path, main_js)?;

    let mut watch_paths = vec![project_dir.join("src")];
    for path in &config.hot_reload.watch {
        watch_paths.push(project_dir.join(path));
    }

    let project_dir_buf = project_dir.to_path_buf();
    let package_name = crate_name.to_string();
    let web_dir_buf = web_dir.clone();
    let wasm_pack_path = wasm_pack;
    let build_callback: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
        build_web_app(
            project_dir_buf.as_path(),
            &package_name,
            web_dir_buf.as_path(),
            release,
            wasm_pack_path.as_path(),
            false,
        )?;
        server.notify_web_reload();
        Ok(())
    });

    let watcher = RebuildWatcher::new(watch_paths, &build_callback)?;

    if output::global_output_format().is_json() {
        let _ = webbrowser::open(&url);
    } else {
        ui::success(format!("Web server started at {url}"));
        match webbrowser::open(&url) {
            Ok(()) => ui::info("Opened in default browser"),
            Err(err) => ui::warning(format!("Could not open browser: {err}")),
        }
        ui::plain("Press Ctrl+C to stop the server");
    }

    match wait_for_interrupt(None)? {
        WaitOutcome::Interrupted => {}
        WaitOutcome::ConnectionLost(reason) => {
            bail!(format_connection_loss_message(reason));
        }
    }

    // Restore original main.js template
    if let Ok(template_content) =
        std::fs::read_to_string(project_dir.join("cli/src/templates/web/main.js"))
    {
        let _ = std::fs::write(web_dir.join("main.js"), template_content);
    }

    drop(watcher);

    Ok(())
}

fn build_web_app(
    project_dir: &Path,
    package: &str,
    web_dir: &Path,
    release: bool,
    wasm_pack: &Path,
    hot_reload_enabled: bool,
) -> Result<()> {
    let mut cmd = Command::new(wasm_pack);
    cmd.arg("build")
        .arg("--target")
        .arg("web")
        .arg("--out-dir")
        .arg(web_dir.join("pkg"))
        .arg("--out-name")
        .arg("app");
    if release {
        cmd.arg("--release");
    } else {
        cmd.arg("--dev");
    }
    cmd.current_dir(project_dir);
    cli_util::configure_hot_reload_env(&mut cmd, hot_reload_enabled, None);

    debug!("Running command: {:?}", cmd);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run wasm-pack build for {package}"))?;
    if !status.success() {
        bail!("wasm-pack build failed with status {}", status);
    }

    Ok(())
}

fn apple_simulator_kind(platform: Platform) -> AppleSimulatorKind {
    match platform {
        Platform::Ios => AppleSimulatorKind::Ios,
        Platform::Ipados => AppleSimulatorKind::Ipados,
        Platform::Watchos => AppleSimulatorKind::Watchos,
        Platform::Tvos => AppleSimulatorKind::Tvos,
        Platform::Visionos => AppleSimulatorKind::Visionos,
        _ => unreachable!("apple_simulator_kind called for unsupported platform {platform:?}"),
    }
}

const fn apple_platform_display_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Ios => "iOS",
        Platform::Ipados => "iPadOS",
        Platform::Watchos => "watchOS",
        Platform::Tvos => "tvOS",
        Platform::Visionos => "visionOS",
        _ => "Apple",
    }
}

fn apple_build_progress_message(
    platform: Platform,
    scheme: &str,
    explicit_target: Option<&str>,
) -> String {
    explicit_target.map_or_else(
        || {
            let platform_name = apple_platform_display_name(platform);
            format!("Building Xcode project \"{scheme}\" for {platform_name}...")
        },
        |target| format!("Building Xcode project \"{scheme}\" for {target}..."),
    )
}

fn apple_simulator_candidates(platform: Platform) -> Result<Vec<DeviceInfo>> {
    let raw_platform = apple_simulator_platform_id(platform);
    if raw_platform.is_empty() {
        return Ok(Vec::new());
    }

    let mut spinner = ui::spinner("Scanning Apple devices...");
    let scan_started = Instant::now();
    debug!("Starting Apple device scan for apple_simulator_candidates ({platform:?})");
    let devices = match device::list_devices_filtered(DevicePlatformFilter::Apple) {
        Ok(list) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Apple device scan for apple_simulator_candidates ({platform:?}) completed in {:?} with {} device(s)",
                scan_started.elapsed(),
                list.len()
            );
            list
        }
        Err(err) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Apple device scan for apple_simulator_candidates ({platform:?}) failed after {:?}: {err:?}",
                scan_started.elapsed()
            );
            return Err(err);
        }
    };

    let mut candidates: Vec<DeviceInfo> = devices
        .into_iter()
        .filter(|d| d.kind == DeviceKind::Simulator)
        .filter(|d| d.raw_platform.as_deref() == Some(raw_platform))
        .collect();

    if candidates.is_empty() {
        bail!(
            "No simulators found for {}. Install one using Xcode's Devices window.",
            apple_platform_display_name(platform)
        );
    }

    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(candidates)
}

fn prompt_for_apple_device(platform: Platform) -> Result<String> {
    let candidates = apple_simulator_candidates(platform)?;
    if candidates.len() == 1 {
        announce_apple_auto_choice(
            &candidates[0],
            platform,
            AutoSelectionReason::SingleCandidate,
        );
        return Ok(candidates[0].name.clone());
    }

    if !is_interactive_terminal() {
        announce_apple_auto_choice(
            &candidates[0],
            platform,
            AutoSelectionReason::NonInteractive,
        );
        return Ok(candidates[0].name.clone());
    }

    let theme = ColorfulTheme::default();
    let options: Vec<String> = candidates
        .iter()
        .map(|d| {
            d.detail.as_ref().map_or_else(
                || d.name.clone(),
                |detail| format!("{} ({})", d.name, detail),
            )
        })
        .collect();

    let selection = Select::with_theme(&theme)
        .with_prompt("Select a simulator")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(candidates[selection].name.clone())
}

fn prompt_for_android_device() -> Result<AndroidSelection> {
    let mut spinner = ui::spinner("Scanning Android devices...");
    let scan_started = Instant::now();
    debug!("Starting Android device scan for prompt_for_android_device");
    let devices = match device::list_devices_filtered(DevicePlatformFilter::Android) {
        Ok(list) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Android device scan for prompt_for_android_device completed in {:?} with {} device(s)",
                scan_started.elapsed(),
                list.len()
            );
            list
        }
        Err(err) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            debug!(
                "Android device scan for prompt_for_android_device failed after {:?}: {err:?}",
                scan_started.elapsed()
            );
            return Err(err);
        }
    };
    let mut candidates: Vec<DeviceInfo> = devices
        .into_iter()
        .filter(|d| {
            d.raw_platform.as_deref() == Some("android-device")
                || d.raw_platform.as_deref() == Some("android-emulator")
        })
        .collect();

    if candidates.is_empty() {
        bail!("No Android devices or emulators detected. Connect a device or create an AVD.");
    }

    candidates.sort_by(|a, b| {
        let kind_order = match a.kind {
            DeviceKind::Device => 0,
            DeviceKind::Emulator => 1,
            DeviceKind::Simulator => 2,
        }
        .cmp(&match b.kind {
            DeviceKind::Device => 0,
            DeviceKind::Emulator => 1,
            DeviceKind::Simulator => 2,
        });
        if kind_order == std::cmp::Ordering::Equal {
            a.name.cmp(&b.name)
        } else {
            kind_order
        }
    });

    if candidates.len() == 1 {
        announce_android_auto_choice(&candidates[0], AutoSelectionReason::SingleCandidate);
        return Ok(selection_from_device_info(&candidates[0]));
    }

    if !is_interactive_terminal() {
        announce_android_auto_choice(&candidates[0], AutoSelectionReason::NonInteractive);
        return Ok(selection_from_device_info(&candidates[0]));
    }

    let theme = ColorfulTheme::default();
    let options: Vec<String> = candidates
        .iter()
        .map(|d| {
            let kind = match d.kind {
                DeviceKind::Device => "device",
                DeviceKind::Emulator => "emulator",
                DeviceKind::Simulator => "simulator",
            };
            d.state.as_ref().map_or_else(
                || format!("{} ({})", d.name, kind),
                |state| format!("{} ({}, {})", d.name, kind, state),
            )
        })
        .collect();

    let selection = Select::with_theme(&theme)
        .with_prompt("Select an Android device/emulator")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(selection_from_device_info(&candidates[selection]))
}

fn announce_android_auto_choice(candidate: &DeviceInfo, reason: AutoSelectionReason) {
    if output::global_output_format().is_json() {
        return;
    }

    let label = describe_android_candidate(candidate);
    ui::info(format!("{} {label}.", reason.message_prefix()));
}

fn describe_android_candidate(info: &DeviceInfo) -> String {
    let kind = match info.kind {
        DeviceKind::Device => "device",
        DeviceKind::Emulator => "emulator",
        DeviceKind::Simulator => "simulator",
    };
    match &info.state {
        Some(state) if !state.is_empty() => {
            format!("{} ({kind}, {state})", info.name)
        }
        _ => format!("{} ({kind})", info.name),
    }
}

fn selection_from_device_info(info: &DeviceInfo) -> AndroidSelection {
    AndroidSelection {
        name: info.name.clone(),
        identifier: if info.kind == DeviceKind::Device {
            Some(info.identifier.clone())
        } else {
            None
        },
        kind: info.kind.clone(),
    }
}

fn announce_apple_auto_choice(
    candidate: &DeviceInfo,
    platform: Platform,
    reason: AutoSelectionReason,
) {
    if output::global_output_format().is_json() {
        return;
    }

    let platform_name = apple_platform_display_name(platform);
    ui::info(format!(
        "{} {} for {}.",
        reason.message_prefix(),
        candidate.name,
        platform_name
    ));
}

#[derive(Copy, Clone)]
enum AutoSelectionReason {
    SingleCandidate,
    NonInteractive,
}

impl AutoSelectionReason {
    const fn message_prefix(self) -> &'static str {
        match self {
            Self::SingleCandidate => "Using",
            Self::NonInteractive => "Non-interactive terminal detected; using",
        }
    }
}

fn is_interactive_terminal() -> bool {
    atty::is(Stream::Stdin) && atty::is(Stream::Stdout)
}

fn resolve_android_device(name: &str) -> Result<AndroidSelection> {
    let devices = device::list_devices_filtered(DevicePlatformFilter::Android)?;
    if let Some(device) = devices
        .iter()
        .find(|d| d.identifier == name || d.name == name)
    {
        return Ok(AndroidSelection {
            name: device.name.clone(),
            identifier: if device.kind == DeviceKind::Device {
                Some(device.identifier.clone())
            } else {
                None
            },
            kind: device.kind.clone(),
        });
    }
    bail!(
        "Android device or emulator '{}' not found. Run `water devices` to list available targets.",
        name
    );
}

const fn apple_simulator_platform_id(platform: Platform) -> &'static str {
    match platform {
        Platform::Ios | Platform::Ipados => "com.apple.platform.iphonesimulator",

        Platform::Watchos => "com.apple.platform.watchsimulator",

        Platform::Tvos => "com.apple.platform.appletvsimulator",

        Platform::Visionos => "com.apple.platform.visionossimulator",

        Platform::Macos | Platform::Android | Platform::Web => "",
    }
}

#[derive(Debug, Clone)]
enum HotReloadMessage {
    Native(PathBuf),
    Web,
}

#[derive(Clone)]
struct ServerState {
    hot_reload_tx: broadcast::Sender<HotReloadMessage>,
    connection_event_tx: mpsc::Sender<NativeConnectionEvent>,
    log_filter: Option<String>,
    shutdown: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Server {
    address: SocketAddr,
    thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    hot_reload_tx: broadcast::Sender<HotReloadMessage>,
    connection_events: NativeConnectionEvents,
    shutdown: Arc<AtomicBool>,
}

type HotReloadSocket = WebSocket;

impl Server {
    fn start(static_path: PathBuf, log_filter: Option<String>) -> Result<Self> {
        let (hot_reload_tx, _) = broadcast::channel(16);
        let (connection_event_tx, connection_event_rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let app_state = Arc::new(ServerState {
            hot_reload_tx: hot_reload_tx.clone(),
            connection_event_tx: connection_event_tx.clone(),
            log_filter: log_filter.clone(),
            shutdown: shutdown.clone(),
        });

        let (startup_tx, startup_rx) =
            std::sync::mpsc::channel::<std::result::Result<SocketAddr, io::Error>>();
        let thread = thread::spawn(move || {
            skyzen_runtime::init_logging();
            let router = build_hot_reload_router(app_state, static_path);
            let address = reserve_loopback_addr().expect("Failed to reserve loopback address");
            // Safe because the address string is well-formed and under our control.
            unsafe {
                std::env::set_var("SKYZEN_ADDRESS", address.to_string());
            }
            let _ = startup_tx.send(Ok(address));
            skyzen_runtime::launch(move || async { router });
        });

        let connection_events = NativeConnectionEvents::new(connection_event_rx);
        let startup_result = match startup_rx.recv() {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::Other,
                "hot reload server failed to report its status",
            )),
        };
        let address = startup_result.context("failed to bind hot reload server socket")?;

        Ok(Self {
            address,
            thread: Arc::new(Mutex::new(Some(thread))),
            hot_reload_tx,
            connection_events,
            shutdown,
        })
    }

    const fn address(&self) -> SocketAddr {
        self.address
    }

    fn notify_native_reload(&self, path: PathBuf) {
        info!("Hot reload: queueing native artifact {}", path.display());
        let _ = self.hot_reload_tx.send(HotReloadMessage::Native(path));
    }

    fn notify_web_reload(&self) {
        let _ = self.hot_reload_tx.send(HotReloadMessage::Web);
    }

    fn connection_events(&self) -> NativeConnectionEvents {
        self.connection_events.clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Signal all handlers to shutdown
        self.shutdown.store(true, Ordering::Relaxed);

        // Take the thread handle
        let thread_handle = self.thread.lock().unwrap().take();

        // Give the server a brief moment to shut down gracefully
        // If skyzen is stuck, we don't block forever - just let the process exit
        if let Some(handle) = thread_handle {
            // Wait a short time for graceful shutdown
            let start = Instant::now();
            let timeout = Duration::from_millis(500);

            loop {
                if handle.is_finished() {
                    let _ = handle.join();
                    break;
                }
                if start.elapsed() > timeout {
                    // Server didn't shutdown in time, just continue
                    // The thread will be terminated when the process exits
                    debug!("Hot reload server shutdown timed out, continuing...");
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn reserve_loopback_addr() -> Result<SocketAddr> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr)
}

fn build_hot_reload_router(
    state: Arc<ServerState>,
    static_path: PathBuf,
) -> skyzen::routing::Router {
    let native_state = state.clone();
    let web_state = state.clone();

    Route::new((
        "/hot-reload-native".ws(move |socket| handle_native_socket(socket, native_state.clone())),
        "/hot-reload-web".ws(move |socket| handle_web_socket(socket, web_state.clone())),
        StaticDir::new("/", static_path),
    ))
    .build()
}

async fn handle_native_socket(mut socket: HotReloadSocket, state: Arc<ServerState>) {
    if let Some(filter) = &state.log_filter {
        let message = json!({
            "type": "log_filter",
            "filter": filter,
        })
        .to_string();
        let _ = socket.send(WebSocketMessage::Text(message.into())).await;
    }

    let mut rx = state.hot_reload_tx.subscribe();
    let _ = state
        .connection_event_tx
        .send(NativeConnectionEvent::Connected);

    // Interval for checking shutdown flag
    let mut shutdown_check = tokio::time::interval(tokio::time::Duration::from_millis(100));

    loop {
        // Check shutdown flag
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        tokio::select! {
            _ = shutdown_check.tick() => {
                // Periodic check for shutdown handled above
            }
            Some(msg) = socket.next() => {
                match msg {
                    Ok(WebSocketMessage::Text(payload)) => {
                        handle_native_client_message(&payload);
                    }
                    Ok(WebSocketMessage::Close(frame)) => {
                        let reason = NativeDisconnectReason::Graceful(frame.map(|f| f.code.into()));
                        let _ = state.connection_event_tx.send(NativeConnectionEvent::Disconnected(reason));
                        break;
                    }
                    Ok(WebSocketMessage::Ping(payload)) => {
                        let _ = socket.send(WebSocketMessage::Pong(payload)).await;
                    }
                    Ok(WebSocketMessage::Binary(_))
                    | Ok(WebSocketMessage::Pong(_))
                    | Ok(WebSocketMessage::Frame(_)) => {}
                    Err(err) => {
                        let reason = NativeDisconnectReason::Abnormal(err.to_string());
                        let _ = state.connection_event_tx.send(NativeConnectionEvent::Disconnected(reason));
                        break;
                    }
                }
            }
            msg = rx.recv() => {
                match msg {
                    Ok(HotReloadMessage::Native(path)) => {
                        match tokio_fs::read(&path).await {
                            Ok(data) => {
                                info!(
                                    "Hot reload: sending {} ({} bytes)",
                                    path.display(),
                                    data.len()
                                );
                                if let Err(err) = socket.send(WebSocketMessage::Binary(data.into())).await {
                                    let reason = NativeDisconnectReason::Abnormal(err.to_string());
                                    let _ = state.connection_event_tx.send(NativeConnectionEvent::Disconnected(reason));
                                    break;
                                }
                            }
                            Err(err) => {
                                let exists = path.exists();
                                warn!(
                                    "Failed to read hot reload artifact at {} (exists: {}): {err:?}",
                                    path.display(),
                                    exists
                                );
                            }
                        }
                    }
                    Ok(HotReloadMessage::Web) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("Missed {skipped} hot reload updates (CLI lagged behind)");
                    }
                }
            }
            else => break,
        }
    }
}

async fn handle_web_socket(mut socket: HotReloadSocket, state: Arc<ServerState>) {
    let mut rx = state.hot_reload_tx.subscribe();
    let mut shutdown_check = tokio::time::interval(tokio::time::Duration::from_millis(100));

    loop {
        // Check shutdown flag
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        tokio::select! {
            _ = shutdown_check.tick() => {
                // Periodic check for shutdown handled above
            }
            msg = rx.recv() => {
                match msg {
                    Ok(HotReloadMessage::Web) => {
                        if socket
                            .send(WebSocketMessage::Text("reload".to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(HotReloadMessage::Native(_)) => {}
                    Err(_) => break,
                }
            }
        }
    }
}

const LAST_RUN_FILE: &str = "last-run.json";

#[derive(Debug, Clone)]
struct LastRunSnapshot {
    platform: Platform,
    device: Option<String>,
    release: bool,
    enable_sccache: bool,
    mold: bool,
    timestamp: u64,
}

fn persist_last_run(project: &Project, snapshot: LastRunSnapshot) -> Result<()> {
    let path = last_run_path(project);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(&path)?;
    let record: SerializableLastRunSnapshot = snapshot.into();
    serde_json::to_writer_pretty(file, &record)?;
    Ok(())
}

fn load_last_run(project: &Project) -> Result<LastRunSnapshot> {
    let path = last_run_path(project);
    let file = File::open(&path).with_context(|| {
        format!(
            "No previous run recorded for {}. Run `water run` first.",
            project.root().display()
        )
    })?;
    let record: SerializableLastRunSnapshot = serde_json::from_reader(file).with_context(|| {
        format!(
            "Failed to parse last run configuration at {}",
            path.display()
        )
    })?;
    Ok(record.into())
}

fn last_run_path(project: &Project) -> PathBuf {
    project.root().join(".water").join(LAST_RUN_FILE)
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableLastRunSnapshot {
    platform: StoredPlatform,
    device: Option<String>,
    release: bool,
    enable_sccache: bool,
    mold: bool,
    timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredPlatform {
    Web,
    Macos,
    Ios,
    Ipados,
    Watchos,
    Tvos,
    Visionos,
    Android,
}

impl From<Platform> for StoredPlatform {
    fn from(platform: Platform) -> Self {
        match platform {
            Platform::Web => StoredPlatform::Web,
            Platform::Macos => StoredPlatform::Macos,
            Platform::Ios => StoredPlatform::Ios,
            Platform::Ipados => StoredPlatform::Ipados,
            Platform::Watchos => StoredPlatform::Watchos,
            Platform::Tvos => StoredPlatform::Tvos,
            Platform::Visionos => StoredPlatform::Visionos,
            Platform::Android => StoredPlatform::Android,
        }
    }
}

impl From<StoredPlatform> for Platform {
    fn from(value: StoredPlatform) -> Self {
        match value {
            StoredPlatform::Web => Platform::Web,
            StoredPlatform::Macos => Platform::Macos,
            StoredPlatform::Ios => Platform::Ios,
            StoredPlatform::Ipados => Platform::Ipados,
            StoredPlatform::Watchos => Platform::Watchos,
            StoredPlatform::Tvos => Platform::Tvos,
            StoredPlatform::Visionos => Platform::Visionos,
            StoredPlatform::Android => Platform::Android,
        }
    }
}

impl From<LastRunSnapshot> for SerializableLastRunSnapshot {
    fn from(value: LastRunSnapshot) -> Self {
        Self {
            platform: StoredPlatform::from(value.platform),
            device: value.device,
            release: value.release,
            enable_sccache: value.enable_sccache,
            mold: value.mold,
            timestamp: value.timestamp,
        }
    }
}

impl From<SerializableLastRunSnapshot> for LastRunSnapshot {
    fn from(value: SerializableLastRunSnapshot) -> Self {
        Self {
            platform: value.platform.into(),
            device: value.device,
            release: value.release,
            enable_sccache: value.enable_sccache,
            mold: value.mold,
            timestamp: value.timestamp,
        }
    }
}

struct RebuildWatcher {
    _watcher: RecommendedWatcher,
    signal: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RebuildWatcher {
    fn new(
        watch_paths: Vec<PathBuf>,
        build_callback: &Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                    ) {
                        let _ = tx.send(());
                    }
                }
            })?;

        let mut seen = HashSet::new();
        for path in watch_paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            if path.exists() {
                watcher.watch(&path, RecursiveMode::Recursive)?;
            } else {
                debug!("Skipping hot reload path (not found): {}", path.display());
            }
        }

        let signal = Arc::new(AtomicBool::new(false));
        let shutdown_flag = signal.clone();
        let build = Arc::clone(build_callback);

        let handle = thread::spawn(move || {
            info!("Hot reload watcher started (CLI)");
            let mut last_run = Instant::now();
            while !shutdown_flag.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(()) => {
                        if last_run.elapsed() < Duration::from_millis(250) {
                            continue;
                        }
                        if let Err(err) = build() {
                            warn!("Rebuild failed: {}", err);
                        }
                        last_run = Instant::now();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            debug!("Hot reload watcher stopped");
        });

        Ok(Self {
            _watcher: watcher,
            signal,
            thread: Some(handle),
        })
    }
}

impl Drop for RebuildWatcher {
    fn drop(&mut self) {
        self.signal.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum NativeClientEvent {
    #[serde(rename = "panic")]
    Panic(NativePanicReport),
    #[serde(rename = "log")]
    Log(NativeLogEvent),
}

#[derive(Debug, Deserialize)]
struct NativePanicReport {
    message: String,
    location: Option<NativePanicLocation>,
    thread: Option<String>,
    backtrace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeLogEvent {
    message: String,
    level: String,
    target: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativePanicLocation {
    file: String,
    line: u32,
    column: u32,
}

fn handle_native_client_message(payload: &str) {
    match serde_json::from_str::<NativeClientEvent>(payload) {
        Ok(NativeClientEvent::Panic(report)) => emit_remote_panic(report),
        Ok(NativeClientEvent::Log(event)) => emit_remote_log(event),
        Err(err) => {
            warn!("Failed to parse native client message ({err}): {payload}");
        }
    }
}

fn emit_remote_panic(report: NativePanicReport) {
    if output::global_output_format().is_json() {
        warn!("App panic: {:?}", report);
        return;
    }

    ui::newline();
    println!(
        "{}",
        style("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
            .red()
            .dim()
    );
    ui::error("PANIC in app");
    println!(
        "{}",
        style("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
            .red()
            .dim()
    );
    ui::newline();

    // Message - the most important part, highlighted
    println!("  {} {}", style("Message:").bold(), style(&report.message).red());

    // Location - formatted as clickable path
    if let Some(location) = &report.location {
        let location_str = format!("{}:{}:{}", location.file, location.line, location.column);
        println!("  {} {}", style("Location:").bold(), style(&location_str).cyan().underlined());
    }

    // Thread info
    if let Some(thread) = &report.thread {
        println!("  {} {}", style("Thread:").bold(), thread);
    }

    // Backtrace - with smart formatting
    if let Some(backtrace) = &report.backtrace {
        let backtrace = backtrace.trim();
        if !backtrace.is_empty() && backtrace != "disabled backtrace" {
            ui::newline();
            println!("  {}", style("Backtrace:").bold());
            for line in backtrace.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                // Highlight lines that look like user code (not std/core/alloc)
                let is_user_frame = !line.contains("std::")
                    && !line.contains("core::")
                    && !line.contains("alloc::")
                    && !line.contains("<unknown>")
                    && !line.contains("rust_begin_unwind");

                if is_user_frame && (line.contains("::") || line.contains(" at ")) {
                    println!("    {}", style(line).yellow());
                } else {
                    ui::dimmed(format!("    {line}"));
                }
            }
        }
    }

    ui::newline();
    println!(
        "{}",
        style("────────────────────────────────────────────────────────────────────────────────")
            .dim()
    );
    ui::hint("Fix the panic above, save, and WaterUI will rebuild automatically.");
    ui::newline();
}

fn emit_remote_log(event: NativeLogEvent) {
    if output::global_output_format().is_json() {
        return;
    }
    let target = event.target.unwrap_or_default();
    let message = event
        .message
        .trim()
        .trim_start_matches(WATERUI_TRACING_PREFIX)
        .trim_start();
    if target.is_empty() {
        println!("{} [{}] {}", WATERUI_TRACING_PREFIX, event.level, message);
    } else {
        println!(
            "{} [{}] {} ({})",
            WATERUI_TRACING_PREFIX, event.level, message, target
        );
    }
}

#[derive(Clone)]
struct NativeConnectionEvents {
    receiver: Arc<Mutex<mpsc::Receiver<NativeConnectionEvent>>>,
}

impl NativeConnectionEvents {
    fn new(receiver: mpsc::Receiver<NativeConnectionEvent>) -> Self {
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }

    fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<NativeConnectionEvent, mpsc::RecvTimeoutError> {
        self.receiver.lock().unwrap().recv_timeout(timeout)
    }

    fn try_recv(&self) -> Result<NativeConnectionEvent, TryRecvError> {
        self.receiver.lock().unwrap().try_recv()
    }
}

#[derive(Clone, Debug)]
enum NativeConnectionEvent {
    Connected,
    Disconnected(NativeDisconnectReason),
}

#[derive(Clone, Debug)]
enum NativeDisconnectReason {
    Graceful(Option<u16>),
    Abnormal(String),
}

enum WaitOutcome {
    Interrupted,
    ConnectionLost(NativeDisconnectReason),
}

const HOT_RELOAD_CONNECTION_TIMEOUT: Duration = Duration::from_secs(20);

fn wait_for_hot_reload_connection(events: &NativeConnectionEvents) -> Result<()> {
    let deadline = Instant::now() + HOT_RELOAD_CONNECTION_TIMEOUT;
    loop {
        // Check for interrupt signal first
        if waterui_cli::util::is_interrupted() {
            bail!("Interrupted while waiting for hot reload connection");
        }

        let now = Instant::now();
        if now >= deadline {
            bail!(
                "App failed to establish a hot reload WebSocket connection. Confirm it launched and can reach the CLI."
            );
        }
        // Use a short timeout to allow interrupt checks
        let poll_timeout = Duration::from_millis(200);
        match events.recv_timeout(poll_timeout) {
            Ok(NativeConnectionEvent::Connected) => return Ok(()),
            Ok(NativeConnectionEvent::Disconnected(reason)) => {
                bail!(format_connection_loss_message(reason));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("Hot reload server shut down before the app connected. Restart `water run`.");
            }
        }
    }
}

fn wait_for_interrupt(connection_events: Option<NativeConnectionEvents>) -> Result<WaitOutcome> {
    // Use the global interrupt flag set by the ctrlc handler in main.rs
    loop {
        // Check global interrupt flag
        if waterui_cli::util::is_interrupted() {
            return Ok(WaitOutcome::Interrupted);
        }

        if let Some(events) = &connection_events {
            match events.try_recv() {
                Ok(NativeConnectionEvent::Connected) => {}
                Ok(NativeConnectionEvent::Disconnected(reason)) => {
                    return Ok(WaitOutcome::ConnectionLost(reason));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    if waterui_cli::util::is_interrupted() {
                        return Ok(WaitOutcome::Interrupted);
                    }
                    return Ok(WaitOutcome::ConnectionLost(
                        NativeDisconnectReason::Abnormal(
                            "Hot reload server stopped unexpectedly".to_string(),
                        ),
                    ));
                }
            }
        }

        // Sleep briefly to avoid busy-waiting
        thread::sleep(Duration::from_millis(100));
    }
}

fn format_connection_loss_message(reason: NativeDisconnectReason) -> String {
    match reason {
        NativeDisconnectReason::Graceful(code) => {
            if let Some(code) = code {
                format!("Hot reload connection closed by the app (close code {code}).")
            } else {
                "Hot reload connection closed by the app.".to_string()
            }
        }
        NativeDisconnectReason::Abnormal(details) => {
            let detail = details.trim();
            if detail.is_empty() {
                "Hot reload connection failed. The app likely crashed.".to_string()
            } else {
                format!("Hot reload connection failed ({detail}). The app likely crashed.")
            }
        }
    }
}
