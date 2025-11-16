use crate::{ui, util};
use atty::{self, Stream};
use axum::{
    Router,
    extract::{
        State,
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use clap::{Args, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use console::style;
use dialoguer::{Select, theme::ColorfulTheme};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    time::{SystemTime, UNIX_EPOCH},
};
use waterui_cli::{
    backend::android::configure_rust_android_linker_env,
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
use tokio::sync::{broadcast, oneshot};
use tower_http::services::ServeDir;
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

    /// Disable hot reloading
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
    let config = project.config().clone();
    let is_json = output::global_output_format().is_json();

    if !is_json {
        ui::section(format!("Running: {}", config.package.display_name));
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
    let hot_reload_requested = !args.no_hot_reload;

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
        } else {
            let backend_choice = prompt_for_backend(&config)?;
            platform = Some(resolve_backend_choice(backend_choice, &config)?);
        }
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
        hot_reload_requested,
        enable_sccache,
        mold_requested,
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
        device: snapshot.device.clone(),
        project: args.project,
        release: snapshot.release,
        no_sccache: !snapshot.enable_sccache,
        mold: snapshot.mold,
        no_hot_reload: !snapshot.hot_reload,
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
    let normalized = crate_name.replace('-', "_");
    let filename = if cfg!(target_os = "windows") {
        format!("{normalized}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{normalized}.dylib")
    } else {
        format!("lib{normalized}.so")
    };
    let mut path = project_dir.join("target");
    if let Some(target) = target_triple {
        path = path.join(target);
    }
    path.join(profile).join(filename)
}

#[allow(clippy::too_many_lines)]
fn run_platform(
    platform: Platform,
    mut device: Option<String>,
    project: &Project,
    config: &Config,
    release: bool,
    hot_reload_requested: bool,
    enable_sccache: bool,
    mold_requested: bool,
) -> Result<()> {
    let project_dir = project.root();
    let mut recorded_device = device.clone();
    let hot_reload_target = hot_reload_target(platform);
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
        return run_web(project_dir, config, release, hot_reload_requested);
    }

    let hot_reload_requested =
        hot_reload_requested && platform_supports_native_hot_reload(platform);
    if platform == Platform::Android && hot_reload_requested {
        if let Some(target) = hot_reload_target.as_deref() {
            configure_rust_android_linker_env(&[target])?;
        }
    }

    let mut server = None;
    let mut hot_reload_enabled = false;
    let mut connection_events: Option<NativeConnectionEvents> = None;
    if hot_reload_requested {
        match Server::start(project_dir.to_path_buf()) {
            Ok(instance) => {
                hot_reload_enabled = true;
                connection_events = Some(instance.connection_events());
                server = Some(instance);
            }
            Err(err) => {
                if err
                    .downcast_ref::<io::Error>()
                    .is_some_and(|io_err| io_err.kind() == io::ErrorKind::PermissionDenied)
                {
                    ui::warning(format!(
                        "Hot reload disabled: could not bind local server ({err}). \
Use --no-hot-reload to skip this step."
                    ));
                } else {
                    return Err(err);
                }
            }
        }
    }

    let hot_reload_port = server.as_ref().map(|s| s.address().port());

    if hot_reload_enabled {
        run_cargo_build(
            project_dir,
            &config.package.name,
            release,
            hot_reload_enabled,
            hot_reload_port,
            enable_sccache,
            mold_requested,
            hot_reload_target.as_deref(),
        )?;

        if let Some(server_ref) = &server {
            let library_path = hot_reload_library_path(
                project_dir,
                &config.package.name,
                release,
                hot_reload_target.as_deref(),
            );
            if library_path.exists() {
                server_ref.notify_native_reload(library_path);
            }
        }
    }

    let watcher = if hot_reload_enabled {
        if let Some(server) = server {
            let mut watch_paths = vec![project_dir.join("src")];
            for path in &config.hot_reload.watch {
                watch_paths.push(project_dir.join(path));
            }

            let project_dir_buf = project_dir.to_path_buf();
            let package_name = config.package.name.clone();
            let build_callback: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
                run_cargo_build(
                    &project_dir_buf,
                    &package_name,
                    release,
                    hot_reload_enabled,
                    hot_reload_port,
                    enable_sccache,
                    mold_requested,
                    hot_reload_target.as_deref(),
                )?;
                let library_path = hot_reload_library_path(
                    &project_dir_buf,
                    &package_name,
                    release,
                    hot_reload_target.as_deref(),
                );
                server.notify_native_reload(library_path);
                Ok(())
            });

            Some(RebuildWatcher::new(watch_paths, &build_callback)?)
        } else {
            None
        }
    } else {
        None
    };

    let run_options = RunOptions {
        release,
        hot_reload: HotReloadOptions {
            enabled: hot_reload_enabled,
            port: hot_reload_port,
        },
    };

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
            run_with_package_spinner(project, device_impl, run_options, spinner_msg)?
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
            run_with_package_spinner(project, simulator, run_options, spinner_msg)?
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
            let platform_impl = AndroidPlatform::new(
                android_config,
                false,
                hot_reload_enabled,
                enable_sccache,
                mold_requested,
            );
            let android_device = AndroidDevice::new(platform_impl, selection)?;
            run_on_device(project, android_device, run_options)?
        }
        Platform::Web => unreachable!(),
    };

    if let Err(err) = persist_last_run(
        project,
        LastRunSnapshot {
            platform,
            device: recorded_device,
            release,
            hot_reload: hot_reload_requested,
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

        if hot_reload_enabled {
            ui::info("App launched with hot reload enabled");
            ui::plain("Press Ctrl+C to stop the watcher");
        } else {
            ui::info("App launched successfully");
        }
    }

    if hot_reload_enabled {
        if let Some(events) = &connection_events {
            if !output::global_output_format().is_json() {
                ui::info("Waiting for the app to connect to the hot reload server…");
            }
            wait_for_hot_reload_connection(events)?;
        }
        match wait_for_interrupt(connection_events)? {
            WaitOutcome::Interrupted => {}
            WaitOutcome::ConnectionLost(reason) => {
                bail!(format_connection_loss_message(reason));
            }
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
    spinner_msg: String,
) -> Result<RunReport>
where
    D: Device,
{
    let mut spinner_guard: Option<ui::SpinnerGuard> = None;
    let result = run_on_device_with_observer(project, device, options, |stage| match stage {
        RunStage::Package => {
            if spinner_guard.is_none() {
                spinner_guard = ui::spinner(spinner_msg.clone());
            }
        }
        RunStage::Launch => {
            if let Some(guard) = spinner_guard.take() {
                guard.finish();
            }
        }
        _ => {}
    });

    if let Some(guard) = spinner_guard.take() {
        guard.finish();
    }

    result
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
        cmd
    };

    let mut cmd = make_command();
    let sccache_enabled =
        cli_util::configure_build_speedups(&mut cmd, enable_sccache, mold_requested);
    debug!("Running command: {:?}", cmd);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run cargo build in {}", project_dir.display()))?;
    if status.success() {
        return Ok(());
    }

    if sccache_enabled {
        warn!("cargo build failed when using sccache; retrying without build cache");
        let mut retry_cmd = make_command();
        cli_util::configure_build_speedups(&mut retry_cmd, false, mold_requested);
        debug!("Running command without sccache: {:?}", retry_cmd);
        let retry_status = retry_cmd.status().with_context(|| {
            format!(
                "failed to rerun cargo build without sccache in {}",
                project_dir.display()
            )
        })?;
        if retry_status.success() {
            info!("cargo build succeeded after disabling sccache");
            return Ok(());
        }
    }

    bail!("cargo build failed");
}

fn indent_lines(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_web(project_dir: &Path, config: &Config, release: bool, hot_reload: bool) -> Result<()> {
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
        &config.package.name,
        &web_dir,
        release,
        &wasm_pack,
        false,
    )?;

    let server = Server::start(web_dir.clone())?;
    let address = server.address();
    let url = format!("http://{address}/");

    let watcher = if hot_reload {
        let main_js_path = web_dir.join("main.js");
        let main_js_template = std::fs::read_to_string(&main_js_path)?;
        let main_js = main_js_template.replace("__HOT_RELOAD_PORT__", &address.port().to_string());
        std::fs::write(&main_js_path, main_js)?;

        let mut watch_paths = vec![project_dir.join("src")];
        for path in &config.hot_reload.watch {
            watch_paths.push(project_dir.join(path));
        }

        let project_dir_buf = project_dir.to_path_buf();
        let package_name = config.package.name.clone();
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

        Some(RebuildWatcher::new(watch_paths, &build_callback)?)
    } else {
        None
    };

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

    if hot_reload {
        if let Ok(template_content) =
            std::fs::read_to_string(project_dir.join("cli/src/templates/web/main.js"))
        {
            let _ = std::fs::write(web_dir.join("main.js"), template_content);
        }
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
struct AppState {
    hot_reload_tx: broadcast::Sender<HotReloadMessage>,
    connection_event_tx: mpsc::Sender<NativeConnectionEvent>,
}

#[derive(Clone)]
struct Server {
    address: SocketAddr,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    hot_reload_tx: broadcast::Sender<HotReloadMessage>,
    connection_events: NativeConnectionEvents,
}

impl Server {
    fn start(static_path: PathBuf) -> Result<Self> {
        let (hot_reload_tx, _) = broadcast::channel(16);
        let (connection_event_tx, connection_event_rx) = mpsc::channel();
        let app_state = AppState {
            hot_reload_tx: hot_reload_tx.clone(),
            connection_event_tx: connection_event_tx.clone(),
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let (startup_tx, startup_rx) =
            std::sync::mpsc::channel::<std::result::Result<SocketAddr, io::Error>>();

        let thread = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let app = Router::new()
                    .route("/hot-reload-native", get(native_ws_handler))
                    .route("/hot-reload-web", get(web_ws_handler))
                    .fallback_service(ServeDir::new(static_path))
                    .with_state(app_state);

                let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
                    Ok(listener) => listener,
                    Err(err) => {
                        let _ = startup_tx.send(Err(err));
                        return;
                    }
                };
                let addr = match listener.local_addr() {
                    Ok(addr) => addr,
                    Err(err) => {
                        let _ = startup_tx.send(Err(err));
                        return;
                    }
                };
                let _ = startup_tx.send(Ok(addr));

                if let Err(err) = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        shutdown_rx.await.ok();
                    })
                    .await
                {
                    warn!("hot reload server shutdown unexpectedly: {err:?}");
                }
            });
        });

        let connection_events = NativeConnectionEvents::new(connection_event_rx);
        let startup_result = match startup_rx.recv() {
            Ok(result) => result,
            Err(_) => {
                let _ = thread.join();
                bail!("hot reload server failed to report its status");
            }
        };
        let address = match startup_result {
            Ok(addr) => addr,
            Err(err) => {
                let _ = thread.join();
                return Err(err).context("failed to bind hot reload server socket");
            }
        };

        Ok(Self {
            address,
            shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
            thread: Arc::new(Mutex::new(Some(thread))),
            hot_reload_tx,
            connection_events,
        })
    }

    const fn address(&self) -> SocketAddr {
        self.address
    }

    fn notify_native_reload(&self, path: PathBuf) {
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
        let shutdown_handle = self.shutdown_tx.lock().unwrap().take();
        if let Some(tx) = shutdown_handle {
            let _ = tx.send(());
        }
        let thread_handle = self.thread.lock().unwrap().take();
        if let Some(thread) = thread_handle {
            thread.join().unwrap();
        }
    }
}

async fn native_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_native_socket(socket, state))
}

async fn handle_native_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.hot_reload_tx.subscribe();
    let _ = state
        .connection_event_tx
        .send(NativeConnectionEvent::Connected);
    loop {
        tokio::select! {
            Some(msg) = socket.recv() => {
                match msg {
                    Ok(AxumMessage::Text(payload)) => {
                        handle_native_client_message(&payload);
                    }
                    Ok(AxumMessage::Close(frame)) => {
                        let reason = NativeDisconnectReason::Graceful(frame.map(|f| f.code.into()));
                        let _ = state.connection_event_tx.send(NativeConnectionEvent::Disconnected(reason));
                        break;
                    }
                    Ok(AxumMessage::Ping(payload)) => {
                        let _ = socket.send(AxumMessage::Pong(payload)).await;
                    }
                    Ok(_) => {}
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
                        match std::fs::read(path) {
                            Ok(data) => {
                                if let Err(err) = socket.send(AxumMessage::Binary(data)).await {
                                    let reason = NativeDisconnectReason::Abnormal(err.to_string());
                                    let _ = state.connection_event_tx.send(NativeConnectionEvent::Disconnected(reason));
                                    break;
                                }
                            }
                            Err(err) => {
                                warn!("Failed to read hot reload artifact: {err:?}");
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

async fn web_ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_web_socket(socket, state))
}

async fn handle_web_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.hot_reload_tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if matches!(msg, HotReloadMessage::Web)
            && socket
                .send(AxumMessage::Text("reload".to_string()))
                .await
                .is_err()
        {
            break;
        }
    }
}

const LAST_RUN_FILE: &str = "last-run.json";

#[derive(Debug, Clone)]
struct LastRunSnapshot {
    platform: Platform,
    device: Option<String>,
    release: bool,
    hot_reload: bool,
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
    project.root().join(".waterui").join(LAST_RUN_FILE)
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
    hot_reload: bool,
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
            hot_reload: value.hot_reload,
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
            hot_reload: value.hot_reload,
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
}

#[derive(Debug, Deserialize)]
struct NativePanicReport {
    message: String,
    location: Option<NativePanicLocation>,
    thread: Option<String>,
    backtrace: Option<String>,
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

    ui::warning("App panic reported via hot reload");
    ui::kv("Message", &report.message);
    if let Some(thread) = report.thread {
        ui::kv("Thread", thread);
    }
    if let Some(location) = report.location {
        let formatted = format!("{}:{}:{}", location.file, location.line, location.column);
        ui::kv("Location", &formatted);
    }
    if let Some(backtrace) = report.backtrace {
        ui::kv("Backtrace", "");
        for line in backtrace.lines() {
            ui::plain(format!("    {line}"));
        }
    }

    ui::plain("  Hint: fix the panic above, save, and WaterUI will rebuild automatically.");
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
        let now = Instant::now();
        if now >= deadline {
            bail!(
                "App failed to establish a hot reload WebSocket connection. Confirm it launched and can reach the CLI."
            );
        }
        let remaining = deadline.saturating_duration_since(now);
        match events.recv_timeout(remaining) {
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
    let (tx, rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .context("failed to install Ctrl+C handler")?;

    loop {
        if let Some(events) = &connection_events {
            match events.try_recv() {
                Ok(NativeConnectionEvent::Connected) => {}
                Ok(NativeConnectionEvent::Disconnected(reason)) => {
                    return Ok(WaitOutcome::ConnectionLost(reason));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    return Ok(WaitOutcome::ConnectionLost(
                        NativeDisconnectReason::Abnormal(
                            "Hot reload server stopped unexpectedly".to_string(),
                        ),
                    ));
                }
            }
        }

        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(()) => return Ok(WaitOutcome::Interrupted),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(WaitOutcome::Interrupted),
        }
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
