use crate::{
    apple::{
        derived_data_dir, disable_code_signing, ensure_macos_host, prepare_derived_data_dir,
        resolve_xcode_project, run_xcodebuild_with_progress, xcodebuild_base,
    },
    config::Config,
    platform::Platform,
    util,
};
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
use dialoguer::{Select, theme::ColorfulTheme};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    env,
    io::Write,
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
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};
use tower_http::services::ServeDir;
use tracing::{debug, info, warn};

use crate::{
    android, apple,
    devices::{self, DeviceInfo, DeviceKind},
    output,
    toolchain::{self, CheckMode, CheckTarget},
};

use which::which;

#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// Platform to run on
    #[arg(value_enum)]
    pub platform: Option<Platform>,

    /// Device to run on (e.g. a simulator name or device UDID)
    #[arg(long)]
    pub device: Option<String>,

    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Disable sccache
    #[arg(long)]
    pub no_sccache: bool,

    /// Disable hot reloading
    #[arg(long)]
    pub no_hot_reload: bool,
}

pub fn run(args: RunArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;
    let is_json = output::global_output_format().is_json();

    info!("Running WaterUI app '{}'", config.package.display_name);

    let mut platform = args.platform;
    let device = args.device.clone();

    if is_json && args.platform.is_none() && device.is_none() {
        bail!(
            "JSON output requires specifying --platform or --device to avoid interactive prompts."
        );
    }

    if args.platform.is_none() {
        let available_devices = devices::list_devices()?;
        if let Some(device_name) = &device {
            let selected_device =
                find_device(&available_devices, device_name).ok_or_else(|| {
                    eyre!(
                        "Device '{}' not found. Run `water devices` to list available targets.",
                        device_name
                    )
                })?;
            platform = Some(platform_from_device(selected_device)?);
        } else {
            platform = Some(prompt_for_platform(&config, &available_devices)?);
        }
    }

    let platform = args.platform.ok_or_else(|| eyre!("No platform selected"))?;

    run_platform(
        platform,
        device,
        &project_dir,
        &config,
        args.release,
        !args.no_hot_reload,
    )?;

    Ok(())
}

fn prompt_for_platform(config: &Config, devices: &[DeviceInfo]) -> Result<Platform> {
    let mut options: Vec<(Platform, String)> = Vec::new();

    if config.backends.web.is_some() {
        options.push((Platform::Web, "Web Browser".to_string()));
    }

    if config.backends.swift.is_some() {
        options.push((Platform::Macos, "Apple: macOS".to_string()));

        let mut has_ios = false;
        let mut has_ipados = false;
        let mut has_watchos = false;
        let mut has_tvos = false;
        let mut has_visionos = false;

        for device in devices {
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
    }

    if config.backends.android.is_some() {
        let has_android = devices
            .iter()
            .any(|device| matches!(platform_from_device(device), Ok(Platform::Android)));
        if has_android {
            options.push((Platform::Android, "Android".to_string()));
        }
    }

    if options.is_empty() {
        bail!(
            "No runnable targets found. Please connect a device, start a simulator, or enable a backend in your Water.toml."
        );
    }

    let labels: Vec<String> = options.iter().map(|(_, label)| label.clone()).collect();
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a backend to run")
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

fn run_platform(
    platform: Platform,
    device: Option<String>,
    project_dir: &Path,
    config: &Config,
    release: bool,
    hot_reload: bool,
) -> Result<()> {
    if let Err(report) =
        toolchain::ensure_ready(CheckMode::Quick, &toolchain_targets_for_platform(platform))
    {
        let details = report.to_string();
        let mut message = format!("Toolchain not ready for {:?}", platform);
        let trimmed = details.trim();
        if !trimmed.is_empty() {
            message.push_str(":\n\n");
            message.push_str(&indent_lines(trimmed, "  "));
        } else {
            message.push('.');
        }
        return Err(eyre!(message));
    }

    if platform == Platform::Web {
        return run_web(project_dir, config, release, hot_reload);
    }

    let server = if hot_reload {
        Some(Server::start(project_dir.to_path_buf())?)
    } else {
        None
    };

    let hot_reload_port = server.as_ref().map(|s| s.address().port());

    run_cargo_build(
        project_dir,
        &config.package.name,
        release,
        hot_reload,
        hot_reload_port,
    )?;

    if let Some(server) = &server {
        let library_path = hot_reload_library_path(project_dir, &config.package.name, release);
        if library_path.exists() {
            server.notify_native_reload(library_path);
        }
    }

    let watcher = if let Some(server) = server {
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
            let library_path = hot_reload_library_path(&project_dir_buf, &package_name, release);
            server.notify_native_reload(library_path);
            Ok(())
        });

        Some(RebuildWatcher::new(watch_paths, build_callback)?)
    } else {
        None
    };

    match platform {
        Platform::Macos
        | Platform::Ios
        | Platform::Ipados
        | Platform::Watchos
        | Platform::Tvos
        | Platform::Visionos => {
            if let Some(swift_config) = &config.backends.swift {
                info!("(Xcode scheme: {})", swift_config.scheme);

                match platform {
                    Platform::Macos => {
                        run_macos(project_dir, swift_config, release, hot_reload_port)?
                    }
                    Platform::Ios
                    | Platform::Ipados
                    | Platform::Watchos
                    | Platform::Tvos
                    | Platform::Visionos => {
                        if platform == Platform::Watchos {
                            ensure_rust_target_installed("aarch64-apple-watchos-sim")?;
                        }
                        let device_name = match device {
                            Some(name) => name,
                            None => prompt_for_apple_device(platform)?,
                        };
                        run_apple_simulator(
                            project_dir,
                            &config.package,
                            swift_config,
                            release,
                            platform,
                            Some(device_name),
                            hot_reload_port,
                        )?
                    }
                    _ => unreachable!(),
                }
            } else {
                bail!(
                    "Apple backend not configured for this project. Add it to Water.toml or recreate the project with the SwiftUI backend."
                );
            }
        }
        Platform::Android => {
            if let Some(android_config) = &config.backends.android {
                let selection = match device {
                    Some(name) => Some(resolve_android_device(&name)?),
                    None => Some(prompt_for_android_device()?),
                };
                run_android(
                    project_dir,
                    &config.package,
                    android_config,
                    release,
                    selection,
                    hot_reload,
                )?;
            } else {
                bail!(
                    "Android backend not configured for this project. Add it to Water.toml or recreate the project with the Android backend."
                );
            }
        }
        Platform::Web => unreachable!(),
    }

    if hot_reload {
        info!("App launched. Press Ctrl+C to stop the watcher.");
        wait_for_interrupt()?;
    } else {
        info!("App launched.");
    }

    drop(watcher);
    Ok(())
}

fn run_cargo_build(
    project_dir: &Path,
    package: &str,
    release: bool,
    hot_reload_enabled: bool,
    hot_reload_port: Option<u16>,
) -> Result<()> {
    info!("Compiling Rust library...");
    let make_command = || {
        let mut cmd = Command::new("cargo");
        cmd.arg("build").arg("--package").arg(package);
        if release {
            cmd.arg("--release");
        }
        cmd.current_dir(project_dir);
        util::configure_hot_reload_env(&mut cmd, hot_reload_enabled, hot_reload_port);
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
    let url = format!("http://{}/", address);

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
        let wasm_pack_path = wasm_pack.clone();
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

        Some(RebuildWatcher::new(watch_paths, build_callback)?)
    } else {
        None
    };

    info!("Serving web app at {}", url);
    match webbrowser::open(&url) {
        Ok(_) => info!("Opened default browser"),
        Err(err) => warn!("Failed to open browser automatically: {}", err),
    }
    info!("Press Ctrl+C to stop the server.");

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
    util::configure_hot_reload_env(&mut cmd, false, None);

    debug!("Running command: {:?}", cmd);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run wasm-pack build for {}", package))?;
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

    match which("sccache") {
        Ok(path) => {
            debug!("Enabling sccache for cargo builds");
            cmd.env("RUSTC_WRAPPER", path);
            true
        }
        Err(_) => {
            warn!("`sccache` not found on PATH; proceeding without build cache");
            false
        }
    }
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

fn run_macos(
    project_dir: &Path,
    swift_config: &crate::config::Swift,
    release: bool,
    hot_reload_port: Option<u16>,
) -> Result<()> {
    ensure_macos_host("SwiftUI backend support")?;
    util::require_tool(
        "xcodebuild",
        "Install Xcode and command line tools (xcode-select --install)",
    )?;

    let project = resolve_xcode_project(project_dir, swift_config)?;
    let derived_root = derived_data_dir(project_dir);
    prepare_derived_data_dir(&derived_root)?;

    let configuration = if release { "Release" } else { "Debug" };

    let mut build_cmd = xcodebuild_base(&project, configuration, &derived_root);
    build_cmd.arg("-destination").arg("platform=macOS");
    disable_code_signing(&mut build_cmd);

    info!("Building macOS app with xcodebuild…");
    debug!("Executing command: {:?}", build_cmd);
    let log_dir = project_dir.join(".waterui/logs");
    run_xcodebuild_with_progress(
        build_cmd,
        &format!("Building {} ({configuration})", project.scheme),
        &log_dir,
    )?;

    let products_dir = derived_root.join(format!("Build/Products/{}", configuration));
    let app_bundle = products_dir.join(format!("{}.app", project.scheme));
    if !app_bundle.exists() {
        bail!("Expected app bundle at {}", app_bundle.display());
    }

    info!("Launching app…");
    if let Some(port) = hot_reload_port {
        let executable_path = app_bundle.join("Contents/MacOS").join(&project.scheme);
        if !executable_path.exists() {
            bail!("App executable not found at {}", executable_path.display());
        }
        let mut cmd = Command::new(executable_path);
        cmd.env("WATERUI_HOT_RELOAD_PORT", port.to_string());
        cmd.spawn().context("failed to launch app executable")?;
    } else {
        let status = Command::new("open")
            .arg(&app_bundle)
            .status()
            .context("failed to open app bundle")?;
        if !status.success() {
            bail!("Failed to launch app");
        }
    }

    Ok(())
}

fn ensure_rust_target_installed(target: &str) -> Result<()> {
    if which("rustup").is_err() {
        bail!(
            "Rust target `{target}` is required for watchOS builds, but `rustup` was not found. \
            Install Rust from https://rustup.rs and try again."
        );
    }

    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("failed to query installed Rust targets via rustup")?;

    if !output.status.success() {
        bail!(
            "Failed to query installed Rust targets (status {}). \
             Run `rustup target list --installed` manually for more details.",
            output.status
        );
    }

    let installed = String::from_utf8_lossy(&output.stdout);
    let has_target = installed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .any(|line| line == target);

    if has_target {
        Ok(())
    } else {
        bail!(
            "Rust target `{target}` is required for watchOS simulator builds. \
            Install it with `rustup target add {target}` and try again."
        );
    }
}

fn run_apple_simulator(
    project_dir: &Path,
    package: &crate::config::Package,
    swift_config: &crate::config::Swift,
    release: bool,
    platform: Platform,
    device: Option<String>,
    hot_reload_port: Option<u16>,
) -> Result<()> {
    ensure_macos_host("Apple simulators")?;
    for tool in ["xcrun", "xcodebuild"] {
        util::require_tool(
            tool,
            "Install Xcode and command line tools (xcode-select --install)",
        )?;
    }

    info!("Opening Simulator app...");
    Command::new("open")
        .arg("-a")
        .arg("Simulator")
        .status()
        .context("failed to open Simulator app")?;

    let (sim_platform, products_path) = match platform {
        Platform::Ios | Platform::Ipados => ("iOS Simulator", "iphonesimulator"),
        Platform::Watchos => ("watchOS Simulator", "watchsimulator"),
        Platform::Tvos => ("tvOS Simulator", "appletvsimulator"),
        Platform::Visionos => ("visionOS Simulator", "xrsimulator"),
        _ => bail!("Unsupported platform for simulator: {:?}", platform),
    };

    let project = resolve_xcode_project(project_dir, swift_config)?;

    let device_name = match device {
        Some(name) => name,
        None => default_apple_simulator(platform)?,
    };
    info!("Building for simulator {}…", device_name);

    let derived_root = derived_data_dir(project_dir);
    prepare_derived_data_dir(&derived_root)?;

    let configuration = if release { "Release" } else { "Debug" };

    let mut build_cmd = xcodebuild_base(&project, configuration, &derived_root);
    build_cmd
        .arg("-destination")
        .arg(format!("platform={},name={}", sim_platform, device_name))
        .arg("CODE_SIGNING_ALLOWED=NO")
        .arg("CODE_SIGNING_REQUIRED=NO");

    debug!("Executing command: {:?}", build_cmd);
    let log_dir = project_dir.join(".waterui/logs");
    run_xcodebuild_with_progress(
        build_cmd,
        &format!(
            "Building {} for {device_name} ({configuration})",
            project.scheme
        ),
        &log_dir,
    )?;

    let products_dir = derived_root.join(format!(
        "Build/Products/{}-{}",
        configuration, products_path
    ));
    let app_bundle = products_dir.join(format!("{}.app", project.scheme));
    if !app_bundle.exists() {
        bail!(
            "Expected app bundle at {}, but it was not created",
            app_bundle.display()
        );
    }

    info!("Booting simulator…");
    let mut boot_cmd = Command::new("xcrun");
    boot_cmd.args(["simctl", "boot", &device_name]);
    let _ = boot_cmd.status(); // Ignore errors if already booted

    info!("Installing app on simulator…");
    let mut install_cmd = Command::new("xcrun");
    install_cmd.args([
        "simctl",
        "install",
        &device_name,
        app_bundle.to_str().expect("path should be valid UTF-8"),
    ]);
    let status = install_cmd
        .status()
        .context("failed to install app on simulator")?;
    if !status.success() {
        bail!("Failed to install app on simulator {}", device_name);
    }

    info!("Launching app…");
    let mut launch_cmd = Command::new("xcrun");
    launch_cmd.args(["simctl", "launch", "--terminate-running-process"]);
    if let Some(port) = hot_reload_port {
        launch_cmd.arg("--setenv");
        launch_cmd.arg("WATERUI_HOT_RELOAD_PORT");
        launch_cmd.arg(port.to_string());
    }
    launch_cmd.args([&device_name, &package.bundle_identifier]);
    let status = launch_cmd.status().context("failed to launch app")?;
    if !status.success() {
        bail!("Failed to launch app on simulator {}", device_name);
    }

    info!("Simulator launch complete.");
    Ok(())
}

fn run_android(
    project_dir: &Path,
    package: &crate::config::Package,
    android_config: &crate::config::Android,
    release: bool,
    selection: Option<AndroidSelection>,
    no_watch: bool,
) -> Result<()> {
    info!("Running for Android...");

    let hot_reload_enabled = !no_watch;

    let adb_path = android::find_android_tool("adb").ok_or_else(|| {
        eyre!("`adb` not found. Install the Android SDK platform-tools and ensure they are available in your Android SDK directory (e.g. ~/Library/Android/sdk) or on your PATH.")
    })?;
    let emulator_path = android::find_android_tool("emulator");

    let selection = if let Some(selection) = selection {
        selection
    } else {
        let emulator = emulator_path.clone().ok_or_else(|| {
            eyre!("No Android emulator available. Install the Android SDK emulator tools or specify a connected device.")
        })?;
        let output = Command::new(&emulator)
            .arg("-list-avds")
            .output()
            .context("failed to get list of Android emulators")?;
        let avds = String::from_utf8(output.stdout)?
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<String>>();
        if avds.is_empty() {
            bail!(
                "No Android emulators found. Please create one using Android Studio's AVD Manager or connect a device."
            );
        }
        AndroidSelection {
            name: avds[0].clone(),
            identifier: avds[0].clone(),
            kind: DeviceKind::Emulator,
        }
    };

    let target_identifier = if selection.kind == DeviceKind::Device {
        info!("Using Android device: {}", selection.name);
        Some(selection.identifier.clone())
    } else {
        let emulator = emulator_path.ok_or_else(|| {
            eyre!(
                "`emulator` not found. Install the Android SDK emulator tools or add them to PATH."
            )
        })?;
        info!("Using emulator: {}", selection.name);
        info!("Launching emulator...");
        Command::new(emulator)
            .arg("-avd")
            .arg(&selection.name)
            .spawn()
            .context("failed to launch emulator")?;
        None
    };

    info!("Waiting for device to be ready...");
    android::wait_for_android_device(&adb_path, target_identifier.as_deref())?;

    let desired_targets =
        android::device_preferred_targets(&adb_path, target_identifier.as_deref())
            .context("Failed to determine device CPU architecture")?;
    android::configure_rust_android_linker_env(&desired_targets)
        .context("Failed to configure Android NDK toolchain for Rust builds")?;

    let apk_path = android::build_android_apk(
        project_dir,
        android_config,
        release,
        false,
        hot_reload_enabled,
        &package.bundle_identifier,
    )?;

    info!("Installing APK...");
    let mut install_cmd = android::adb_command(&adb_path, target_identifier.as_deref());
    install_cmd.args([
        "install",
        "-r",
        apk_path.to_str().expect("path should be valid UTF-8"),
    ]);
    debug!("Running command: {:?}", install_cmd);
    let status = install_cmd.status().context("failed to install APK")?;
    if !status.success() {
        bail!("Failed to install APK");
    }

    info!("Launching app...");
    let sanitized_package = android::sanitize_package_name(&package.bundle_identifier);
    let activity = format!("{sanitized_package}/.MainActivity");
    let mut launch_cmd = android::adb_command(&adb_path, target_identifier.as_deref());
    launch_cmd.args(["shell", "am", "start", "-n", &activity]);
    debug!("Running command: {:?}", launch_cmd);
    let status = launch_cmd.status().context("failed to launch app")?;
    if !status.success() {
        bail!("Failed to launch app");
    }

    Ok(())
}

struct AndroidSelection {
    name: String,
    identifier: String,
    kind: DeviceKind,
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

fn apple_platform_display_name(platform: Platform) -> &'static str {
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

    let mut candidates: Vec<DeviceInfo> = devices::list_devices()?
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

fn default_apple_simulator(platform: Platform) -> Result<String> {
    let candidates = apple_simulator_candidates(platform)?;
    let preferred = candidates
        .iter()
        .find(|d| d.state.as_deref() != Some("unavailable"))
        .unwrap_or(&candidates[0]);
    Ok(preferred.name.clone())
}

fn prompt_for_apple_device(platform: Platform) -> Result<String> {
    let candidates = apple_simulator_candidates(platform)?;
    let theme = ColorfulTheme::default();
    let options: Vec<String> = candidates
        .iter()
        .map(|d| {
            if let Some(detail) = &d.detail {
                format!("{} ({})", d.name, detail)
            } else {
                d.name.clone()
            }
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
    let devices = devices::list_devices()?;
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
            _ => 2,
        }
        .cmp(&match b.kind {
            DeviceKind::Device => 0,
            DeviceKind::Emulator => 1,
            _ => 2,
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
            if let Some(state) = &d.state {
                format!("{} ({}, {})", d.name, kind, state)
            } else {
                format!("{} ({})", d.name, kind)
            }
        })
        .collect();

    let selection = Select::with_theme(&theme)
        .with_prompt("Select an Android device/emulator")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(AndroidSelection {
        name: candidates[selection].name.clone(),
        identifier: candidates[selection].identifier.clone(),
        kind: candidates[selection].kind.clone(),
    })
}

fn resolve_android_device(name: &str) -> Result<AndroidSelection> {
    let devices = devices::list_devices()?;
    if let Some(device) = devices
        .iter()
        .find(|d| d.identifier == name || d.name == name)
    {
        return Ok(AndroidSelection {
            name: device.name.clone(),
            identifier: device.identifier.clone(),
            kind: device.kind.clone(),
        });
    }
    bail!(
        "Android device or emulator '{}' not found. Run `water devices` to list available targets.",
        name
    );
}

fn apple_simulator_platform_id(platform: Platform) -> &'static str {
    match platform {
        Platform::Ios | Platform::Ipados => "com.apple.platform.iphonesimulator",

        Platform::Watchos => "com.apple.platform.watchsimulator",

        Platform::Tvos => "com.apple.platform.appletvsimulator",

        Platform::Visionos => "com.apple.platform.visionossimulator",

        Platform::Macos | Platform::Android | Platform::Web => "",
    }
}

fn find_free_port() -> Result<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()
        .map(|addr| addr.port())
        .context("Failed to get free port")
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

        let thread_hot_reload_tx = hot_reload_tx.clone();
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

    fn address(&self) -> SocketAddr {
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
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.lock().unwrap().take() {
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
        if let HotReloadMessage::Web = msg {
            if socket
                .send(AxumMessage::Text("reload".to_string()))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

fn accept_loop(
    listener: TcpListener,
    clients: Arc<Mutex<Vec<TcpStream>>>,
    last_message: Arc<Mutex<Option<String>>>,
) {
    for connection in listener.accept().await {
        match connection {
            Ok(mut stream) => {
                if let Err(err) = stream.set_nodelay(true) {
                    warn!("Failed to configure hot reload socket: {err}");
                }

                let latest = {
                    let guard = last_message
                        .lock()
                        .expect("hot reload last message poisoned");
                    guard.clone()
                };

                if let Some(message) = latest {
                    if let Err(err) = send_hot_reload_message(&mut stream, &message) {
                        warn!("Failed to send latest hot reload payload to client: {err}");
                        continue;
                    }
                }

                info!("Hot reload client connected");
                clients
                    .lock()
                    .expect("hot reload clients lock poisoned")
                    .push(stream);
            }
            Err(err) => {
                warn!("Hot reload accept error: {err}");
            }
        }
    }
}

fn broadcast_loop(
    receiver: mpsc::Receiver<PathBuf>,
    clients: Arc<Mutex<Vec<TcpStream>>>,
    last_message: Arc<Mutex<Option<String>>>,
) {
    for path in receiver {
        let message = path.to_string_lossy().into_owned();
        {
            let mut guard = last_message
                .lock()
                .expect("hot reload last message poisoned");
            *guard = Some(message.clone());
        }

        let mut clients_guard = clients.lock().expect("hot reload clients lock poisoned");
        clients_guard.retain_mut(|stream| match send_hot_reload_message(stream, &message) {
            Ok(()) => true,
            Err(err) => {
                warn!("Hot reload client disconnected: {err}");
                false
            }
        });
    }
}

async fn send_hot_reload_message(stream: &mut TcpStream, message: &str) -> std::io::Result<()> {
    stream.write_all(message.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}

struct RebuildWatcher {
    _watcher: RecommendedWatcher,
    signal: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RebuildWatcher {
    fn new(
        watch_paths: Vec<PathBuf>,
        build_callback: Arc<dyn Fn() -> Result<()> + Send + Sync>,
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
        let build = build_callback.clone();

        let handle = thread::spawn(move || {
            info!("Hot reload watcher started (CLI)");
            let mut last_run = Instant::now();
            while !shutdown_flag.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(_) => {
                        if last_run.elapsed() < Duration::from_millis(250) {
                            continue;
                        }
                        if let Err(err) = build() {
                            warn!("Rebuild failed: {}", err);
                        }
                        last_run = Instant::now();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
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
