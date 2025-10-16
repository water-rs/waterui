use std::{
    collections::HashSet,
    convert::Infallible,
    env,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::toolchain::{self, CheckMode, CheckTarget};
use clap::{Args, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use dialoguer::{Select, theme::ColorfulTheme};
use hyper::{
    Request, Response,
    body::Incoming,
    header::{CACHE_CONTROL, EXPIRES, HeaderValue, PRAGMA},
    http::StatusCode,
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{runtime::Builder, sync::oneshot, time::sleep};
use tracing::{debug, info, warn};
use which::which;

use crate::{
    android,
    apple::{
        derived_data_dir, disable_code_signing, ensure_macos_host, prepare_derived_data_dir,
        resolve_xcode_project, run_xcodebuild_with_progress, xcodebuild_base,
    },
    config::Config,
    devices::{self, DeviceInfo, DeviceKind},
    util,
};

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Target platform to run
    #[arg(long, value_enum)]
    pub platform: Option<Platform>,

    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Override simulator/device name (for iOS, iPadOS, watchOS, visionOS)
    #[arg(long)]
    pub device: Option<String>,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Disable CLI file watcher hot reload
    #[arg(long)]
    pub no_watch: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum Platform {
    Web,
    #[clap(alias = "mac")]
    Macos,
    #[clap(alias = "iphone")]
    Ios,
    #[clap(alias = "ipad")]
    Ipados,
    #[clap(alias = "watch")]
    Watchos,
    #[clap(alias = "tv")]
    Tvos,
    #[clap(alias = "vision")]
    Visionos,
    Android,
}

pub fn run(args: RunArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;

    info!("Running WaterUI app '{}'", config.package.display_name);

    let mut platform = args.platform;
    let device = args.device.clone();

    if platform.is_none() {
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

    let platform = platform.ok_or_else(|| eyre!("No platform selected"))?;

    run_platform(
        platform,
        device,
        &project_dir,
        &config,
        args.release,
        args.no_watch,
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
    if matches!(
        platform,
        Platform::Macos
            | Platform::Ios
            | Platform::Ipados
            | Platform::Watchos
            | Platform::Tvos
            | Platform::Visionos
    ) {
        targets.push(CheckTarget::Swift);
    }
    if platform == Platform::Android {
        targets.push(CheckTarget::Android);
    }
    targets
}

fn run_platform(
    platform: Platform,
    device: Option<String>,
    project_dir: &Path,
    config: &Config,
    release: bool,
    no_watch: bool,
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
        run_web(project_dir, config, release, no_watch)?;
        return Ok(());
    }

    run_cargo_build(project_dir, &config.package.name, release)?;

    let mut watch_paths = vec![project_dir.join("src")];
    for path in &config.hot_reload.watch {
        watch_paths.push(project_dir.join(path));
    }

    let build_callback = {
        let project_dir = project_dir.to_path_buf();
        let package = config.package.name.clone();
        Arc::new(move || run_cargo_build(&project_dir, &package, release))
    };

    let watcher = if no_watch {
        info!("CLI hot reload watcher disabled (--no-watch)");
        None
    } else {
        Some(RebuildWatcher::new(watch_paths, build_callback)?)
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
                    Platform::Macos => run_macos(project_dir, swift_config, release)?,
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
                )?;
            } else {
                bail!(
                    "Android backend not configured for this project. Add it to Water.toml or recreate the project with the Android backend."
                );
            }
        }
        Platform::Web => unreachable!(),
    }

    drop(watcher);
    Ok(())
}

fn run_cargo_build(project_dir: &Path, package: &str, release: bool) -> Result<()> {
    info!("Compiling Rust library...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--package").arg(package);
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(project_dir);
    apply_build_speedups(&mut cmd);
    debug!("Running command: {:?}", cmd);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run cargo build in {}", project_dir.display()))?;
    if !status.success() {
        bail!("cargo build failed");
    }
    Ok(())
}

fn indent_lines(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_web(project_dir: &Path, config: &Config, release: bool, no_watch: bool) -> Result<()> {
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

    info!("Compiling WebAssembly bundle...");
    build_web_app(
        project_dir,
        &config.package.name,
        &web_dir,
        release,
        &wasm_pack,
    )?;

    let mut watch_paths = vec![project_dir.join("src"), web_dir.clone()];
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
        )
    });

    let watcher = if no_watch {
        info!("CLI hot reload watcher disabled (--no-watch)");
        None
    } else {
        Some(RebuildWatcher::new(watch_paths, build_callback.clone())?)
    };

    let server = WebDevServer::start(web_dir.clone())?;
    let address = server.address();
    let url = format!("http://{}/", address);
    info!("Serving web app at {}", url);
    match webbrowser::open(&url) {
        Ok(_) => info!("Opened default browser"),
        Err(err) => warn!("Failed to open browser automatically: {}", err),
    }
    info!("Press Ctrl+C to stop the server.");

    wait_for_interrupt()?;

    drop(watcher);
    drop(server);

    Ok(())
}

fn build_web_app(
    project_dir: &Path,
    package: &str,
    web_dir: &Path,
    release: bool,
    wasm_pack: &Path,
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

    debug!("Running command: {:?}", cmd);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run wasm-pack build for {}", package))?;
    if !status.success() {
        bail!("wasm-pack build failed with status {}", status);
    }

    Ok(())
}

struct WebDevServer {
    thread: Option<thread::JoinHandle<()>>,
    shutdown: Option<oneshot::Sender<()>>,
    address: SocketAddr,
}

impl WebDevServer {
    fn start(root: PathBuf) -> Result<Self> {
        use hyper_staticfile::{Body as StaticBody, Static};

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .context("failed to bind local development server")?;
        listener
            .set_nonblocking(true)
            .context("failed to configure web server socket")?;
        let address = listener
            .local_addr()
            .context("failed to read web server socket address")?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let (startup_tx, startup_rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to construct tokio runtime for web dev server");

            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("failed to convert listener to tokio listener");
                let static_files = Static::new(root);
                let mut shutdown_rx = shutdown_rx;

                if startup_tx.send(()).is_err() {
                    warn!("web dev server startup receiver dropped");
                    return;
                }

                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => {
                            break;
                        }
                        accept_result = listener.accept() => {
                            match accept_result {
                                Ok((stream, _)) => {
                                    let handler = static_files.clone();
                                    tokio::spawn(async move {
                                        let service = service_fn(move |request: Request<Incoming>| {
                                            let handler = handler.clone();
                                            async move {
                                                let result = handler.serve(request).await;
                                                let mut response = match result {
                                                    Ok(response) => response,
                                                    Err(error) => {
                                                        warn!("web dev server static file error: {}", error);
                                                        Response::builder()
                                                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                                                            .body(StaticBody::Empty)
                                                            .unwrap()
                                                    }
                                                };
                                                apply_dev_cache_headers(&mut response);
                                                Ok::<Response<StaticBody>, Infallible>(response)
                                            }
                                        });

                                        if let Err(err) = http1::Builder::new()
                                            .serve_connection(TokioIo::new(stream), service)
                                            .await
                                        {
                                            warn!("web dev server connection error: {}", err);
                                        }
                                    });
                                }
                                Err(err) => {
                                    warn!("web dev server accept error: {}", err);
                                    sleep(Duration::from_millis(200)).await;
                                }
                            }
                        }
                    }
                }
            });
        });

        startup_rx
            .recv()
            .context("failed to receive web dev server startup confirmation")?;

        Ok(Self {
            thread: Some(thread),
            shutdown: Some(shutdown_tx),
            address,
        })
    }

    fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for WebDevServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.thread.take() {
            if let Err(err) = handle.join() {
                warn!("web dev server thread panicked: {:?}", err);
            }
        }
    }
}

fn apply_dev_cache_headers(response: &mut Response<hyper_staticfile::Body>) {
    let headers = response.headers_mut();
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(EXPIRES, HeaderValue::from_static("0"));
}

fn apply_build_speedups(cmd: &mut Command) {
    configure_sccache(cmd);
    #[cfg(target_os = "linux")]
    configure_mold(cmd);
}

fn configure_sccache(cmd: &mut Command) {
    if env::var_os("RUSTC_WRAPPER").is_some() {
        debug!("RUSTC_WRAPPER already set; not overriding with sccache");
        return;
    }

    match which("sccache") {
        Ok(path) => {
            debug!("Enabling sccache for cargo builds");
            cmd.env("RUSTC_WRAPPER", path);
        }
        Err(_) => {
            warn!("`sccache` not found on PATH; proceeding without build cache");
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

fn run_macos(project_dir: &Path, swift_config: &crate::config::Swift, release: bool) -> Result<()> {
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
    let status = Command::new("open")
        .arg(&app_bundle)
        .status()
        .context("failed to open app bundle")?;
    if !status.success() {
        bail!("Failed to launch app");
    }

    info!("App launched. Press Ctrl+C to stop the watcher.");
    wait_for_interrupt()?;
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
    launch_cmd.args([
        "simctl",
        "launch",
        "--terminate-running-process",
        &device_name,
        &package.bundle_identifier,
    ]);
    let status = launch_cmd.status().context("failed to launch app")?;
    if !status.success() {
        bail!("Failed to launch app on simulator {}", device_name);
    }

    info!("Simulator launch complete. Press Ctrl+C to stop.");
    wait_for_interrupt()?;
    Ok(())
}

fn run_android(
    project_dir: &Path,
    package: &crate::config::Package,
    android_config: &crate::config::Android,
    release: bool,
    selection: Option<AndroidSelection>,
) -> Result<()> {
    info!("Running for Android...");

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

    info!("App launched. Press Ctrl+C to stop the watcher.");
    wait_for_interrupt()?;

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
