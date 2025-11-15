use crate::{ui, util};
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
use waterui_cli::{
    device::{
        self, AndroidDevice, AndroidSelection, AppleSimulatorDevice, Device, DeviceInfo,
        DeviceKind, MacosDevice,
    },
    doctor::{
        AnyToolchainIssue,
        toolchain::{self, CheckMode, CheckTarget},
    },
    output,
    platform::{PlatformKind, android::AndroidPlatform, apple::AppleSimulatorKind},
    project::{Config, FailToRun, HotReloadOptions, Project, RunOptions},
    util as cli_util,
};
type Platform = PlatformKind;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
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

#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// Platform to run on
    #[arg(value_enum)]
    platform: Option<PlatformArg>,

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

    let mut platform = args.platform.map(PlatformKind::from);
    let device = args.device.clone();

    if is_json && args.platform.is_none() && device.is_none() {
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

    let platform = platform.ok_or_else(|| eyre!("No platform selected"))?;

    run_platform(
        platform,
        device,
        &project,
        &config,
        args.release,
        !args.no_hot_reload,
    )?;

    Ok(())
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
            "Apple (macOS, iOS, watchOS)".to_string(),
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

    let labels: Vec<_> = options.iter().map(|(_, label)| label.as_str()).collect();
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a backend to run")
        .items(&labels)
        .default(default_backend_index(&options))
        .interact()?;

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
    let devices = match device::list_devices() {
        Ok(list) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            list
        }
        Err(err) => {
            if let Some(spinner_guard) = spinner.take() {
                spinner_guard.finish();
            }
            return Err(err);
        }
    };
    let mut has_ios = false;
    let mut has_ipados = false;
    let mut has_watchos = false;
    let mut has_tvos = false;
    let mut has_visionos = false;

    for device in &devices {
        if let Ok(platform) = platform_from_device(device) {
            match platform {
                Platform::Ios => has_ios = true,
                Platform::Ipados => has_ipados = true,
                Platform::Watchos => has_watchos = true,
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
    if has_watchos {
        options.push((Platform::Watchos, "Apple: watchOS".to_string()));
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
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select an Apple target")
        .items(&labels)
        .default(0)
        .interact()?;

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
    platform.is_apple_platform()
}

fn hot_reload_library_path(project_dir: &Path, crate_name: &str, release: bool) -> PathBuf {
    let profile = if release { "release" } else { "debug" };
    let normalized = crate_name.replace('-', "_");
    let filename = if cfg!(target_os = "windows") {
        format!("{normalized}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{normalized}.dylib")
    } else {
        format!("lib{normalized}.so")
    };
    project_dir.join("target").join(profile).join(filename)
}

#[allow(clippy::too_many_lines)]
fn run_platform(
    platform: Platform,
    device: Option<String>,
    project: &Project,
    config: &Config,
    release: bool,
    hot_reload_requested: bool,
) -> Result<()> {
    let project_dir = project.root();
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

    let hot_reload = hot_reload_requested && platform_supports_native_hot_reload(platform);

    let server = if hot_reload {
        Some(Server::start(project_dir.to_path_buf())?)
    } else {
        None
    };

    let hot_reload_port = server.as_ref().map(|s| s.address().port());

    if hot_reload {
        run_cargo_build(
            project_dir,
            &config.package.name,
            release,
            hot_reload,
            hot_reload_port,
        )?;

        if let Some(server_ref) = &server {
            let library_path = hot_reload_library_path(project_dir, &config.package.name, release);
            if library_path.exists() {
                server_ref.notify_native_reload(library_path);
            }
        }
    }

    let watcher = if hot_reload {
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
                    hot_reload,
                    hot_reload_port,
                )?;
                let library_path =
                    hot_reload_library_path(&project_dir_buf, &package_name, release);
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
            enabled: hot_reload,
            port: hot_reload_port,
        },
    };

    let artifact = match platform {
        Platform::Macos => {
            let swift_config = config.backends.swift.clone().ok_or_else(|| {
                eyre!(
                    "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the SwiftUI backend."
                )
            })?;
            if !output::global_output_format().is_json() {
                ui::info(format!("Xcode scheme: {}", swift_config.scheme));
            }
            let device_impl = MacosDevice::new(swift_config);
            run_on_device(project, device_impl, run_options)?
        }
        Platform::Ios
        | Platform::Ipados
        | Platform::Watchos
        | Platform::Tvos
        | Platform::Visionos => {
            let swift_config = config.backends.swift.clone().ok_or_else(|| {
                eyre!(
                    "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the SwiftUI backend."
                )
            })?;
            if !output::global_output_format().is_json() {
                ui::info(format!("Xcode scheme: {}", swift_config.scheme));
            }
            let simulator_kind = apple_simulator_kind(platform);
            let device_name = match device {
                Some(name) => name,
                None => prompt_for_apple_device(platform)?,
            };
            let simulator = AppleSimulatorDevice::new(swift_config, simulator_kind, device_name);
            run_on_device(project, simulator, run_options)?
        }
        Platform::Android => {
            let android_config = config.backends.android.clone().ok_or_else(|| {
                eyre!(
                    "Android backend not configured for this project. Add it to Water.toml or recreate the project with the Android backend."
                )
            })?;
            let selection = match device {
                Some(name) => resolve_android_device(&name)?,
                None => prompt_for_android_device()?,
            };
            let platform_impl = AndroidPlatform::new(android_config.clone(), false, hot_reload);
            let android_device = AndroidDevice::new(platform_impl, selection)?;
            run_on_device(project, android_device, run_options)?
        }
        Platform::Web => unreachable!(),
    };

    if !output::global_output_format().is_json() {
        ui::success(format!("Application built: {}", artifact.display()));

        if hot_reload {
            ui::info("App launched with hot reload enabled");
            ui::plain("Press Ctrl+C to stop the watcher");
        } else {
            ui::info("App launched successfully");
        }
    }

    if hot_reload {
        wait_for_interrupt()?;
    }

    drop(watcher);
    Ok(())
}

fn run_on_device<D>(project: &Project, device: D, options: RunOptions) -> Result<PathBuf>
where
    D: Device,
{
    project
        .run(&device, options)
        .map(|report| report.artifact)
        .map_err(convert_run_error)
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
) -> Result<()> {
    if !output::global_output_format().is_json() {
        ui::step("Compiling Rust library...");
    }
    let make_command = || {
        let mut cmd = Command::new("cargo");
        cmd.arg("build").arg("--package").arg(package);
        if release {
            cmd.arg("--release");
        }
        cmd.current_dir(project_dir);
        cli_util::configure_hot_reload_env(&mut cmd, hot_reload_enabled, hot_reload_port);
        cmd
    };

    let mut cmd = make_command();
    let sccache_enabled = configure_build_speedups(&mut cmd, true);
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
        configure_build_speedups(&mut retry_cmd, false);
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

    wait_for_interrupt()?;

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

fn configure_build_speedups(cmd: &mut Command, enable_sccache: bool) -> bool {
    let sccache_enabled = if enable_sccache {
        configure_sccache(cmd)
    } else {
        false
    };
    #[cfg(target_os = "linux")]
    configure_mold(cmd);
    sccache_enabled
}

fn configure_sccache(cmd: &mut Command) -> bool {
    if env::var_os("RUSTC_WRAPPER").is_some() {
        debug!("RUSTC_WRAPPER already set; not overriding with sccache");
        return false;
    }

    which("sccache").map_or_else(
        |_| {
            warn!("`sccache` not found on PATH; proceeding without build cache");
            false
        },
        |path| {
            debug!("Enabling sccache for cargo builds");
            cmd.env("RUSTC_WRAPPER", path);
            true
        },
    )
}

#[cfg(target_os = "linux")]
fn configure_mold(cmd: &mut Command) {
    const MOLD_FLAG: &str = "-C link-arg=-fuse-ld=mold";

    if env::var("RUSTFLAGS")
        .map(|flags| flags.split_whitespace().any(|flag| flag == MOLD_FLAG))
        .unwrap_or(false)
    {
        debug!("mold linker already enabled via RUSTFLAGS");
        return;
    }

    match which("mold") {
        Ok(_) => {
            let mut flags = env::var("RUSTFLAGS").unwrap_or_default();
            if !flags.trim().is_empty() {
                flags.push(' ');
            }
            flags.push_str(MOLD_FLAG);
            debug!("Using mold linker for faster linking");
            cmd.env("RUSTFLAGS", flags);
        }
        Err(_) => {
            warn!("`mold` linker not found; using system default linker");
        }
    }
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

fn wait_for_interrupt() -> Result<()> {
    let (tx, rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .context("failed to install Ctrl+C handler")?;

    // Block until interrupt signal received
    let _ = rx.recv();
    Ok(())
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

fn apple_simulator_candidates(platform: Platform) -> Result<Vec<DeviceInfo>> {
    let raw_platform = apple_simulator_platform_id(platform);
    if raw_platform.is_empty() {
        return Ok(Vec::new());
    }

    let mut candidates: Vec<DeviceInfo> = device::list_devices()?
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
    let devices = device::list_devices()?;
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

    Ok(AndroidSelection {
        name: candidates[selection].name.clone(),
        identifier: match candidates[selection].kind {
            DeviceKind::Device => Some(candidates[selection].identifier.clone()),
            _ => None,
        },
        kind: candidates[selection].kind.clone(),
    })
}

fn resolve_android_device(name: &str) -> Result<AndroidSelection> {
    let devices = device::list_devices()?;
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
}

#[derive(Clone)]
struct Server {
    address: SocketAddr,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    hot_reload_tx: broadcast::Sender<HotReloadMessage>,
}

impl Server {
    fn start(static_path: PathBuf) -> Result<Self> {
        let (hot_reload_tx, _) = broadcast::channel(16);
        let app_state = AppState {
            hot_reload_tx: hot_reload_tx.clone(),
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let (startup_tx, startup_rx) = std::sync::mpsc::channel();

        let thread = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let app = Router::new()
                    .route("/hot-reload-native", get(native_ws_handler))
                    .route("/hot-reload-web", get(web_ws_handler))
                    .fallback_service(ServeDir::new(static_path))
                    .with_state(app_state);

                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                startup_tx.send(addr).unwrap();

                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        shutdown_rx.await.ok();
                    })
                    .await
                    .unwrap();
            });
        });

        let address = startup_rx.recv()?;

        Ok(Self {
            address,
            shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
            thread: Arc::new(Mutex::new(Some(thread))),
            hot_reload_tx,
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
    while let Ok(msg) = rx.recv().await {
        if let HotReloadMessage::Native(path) = msg {
            if let Ok(data) = std::fs::read(path) {
                if socket.send(AxumMessage::Binary(data)).await.is_err() {
                    break;
                }
            }
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
