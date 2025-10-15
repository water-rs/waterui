use std::{
    collections::HashSet,
    env, fmt, fs,
    io::{ErrorKind, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
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

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use dialoguer::{Select, theme::ColorfulTheme};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, info, warn};
use which::which;

use crate::{
    android,
    apple::{
        derived_data_dir, disable_code_signing, ensure_macos_host, prepare_derived_data_dir,
        resolve_xcode_project, xcodebuild_base,
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

#[derive(Debug)]
enum RunnableTarget {
    Web,
    Device(DeviceInfo),
}

impl fmt::Display for RunnableTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunnableTarget::Web => write!(f, "Web Browser"),
            RunnableTarget::Device(device) => {
                let kind = match device.kind {
                    DeviceKind::Device => "device",
                    DeviceKind::Simulator => "simulator",
                    DeviceKind::Emulator => "emulator",
                };
                write!(f, "{} ({}, {})", device.name, device.platform, kind)
            }
        }
    }
}

pub fn run(args: RunArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;

    info!("Running WaterUI app '{}'", config.package.display_name);

    if let Some(platform) = args.platform {
        run_platform(
            platform,
            args.device.clone(),
            &project_dir,
            &config,
            args.release,
            args.no_watch,
        )?;
    } else {
        let targets = discover_runnable_targets(&config)?;
        if targets.is_empty() {
            bail!(
                "No runnable targets found. Please connect a device, start a simulator, or enable a backend in your Water.toml."
            );
        }

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a target to run")
            .items(&targets)
            .default(0)
            .interact()?;

        match &targets[selection] {
            RunnableTarget::Web => {
                run_platform(
                    Platform::Web,
                    None,
                    &project_dir,
                    &config,
                    args.release,
                    args.no_watch,
                )?;
            }
            RunnableTarget::Device(device) => {
                let platform = platform_from_device(device)?;
                run_platform(
                    platform,
                    Some(device.identifier.clone()),
                    &project_dir,
                    &config,
                    args.release,
                    args.no_watch,
                )?;
            }
        }
    }

    Ok(())
}

fn discover_runnable_targets(config: &Config) -> Result<Vec<RunnableTarget>> {
    let mut targets = Vec::new();

    if config.backends.web.is_some() {
        targets.push(RunnableTarget::Web);
    }

    let all_devices = devices::list_devices()?;

    if config.backends.swift.is_some() {
        let apple_devices = all_devices.iter().filter(|d| {
            let is_apple = d.platform.starts_with("iOS")
                || d.platform.starts_with("iPadOS")
                || d.platform.starts_with("watchOS")
                || d.platform.starts_with("tvOS")
                || d.platform.starts_with("visionOS")
                || d.platform == "macOS";
            let is_available = d.state.as_deref().unwrap_or("") != "unavailable";
            is_apple && is_available
        });
        targets.extend(apple_devices.map(|d| RunnableTarget::Device(d.clone())));
    }

    if config.backends.android.is_some() {
        let android_devices = all_devices.iter().filter(|d| {
            let is_android = d.platform == "Android";
            let is_available = d.state.as_deref().unwrap_or("") != "offline";
            is_android && is_available
        });
        targets.extend(android_devices.map(|d| RunnableTarget::Device(d.clone())));
    }

    Ok(targets)
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

fn run_platform(
    platform: Platform,
    device: Option<String>,
    project_dir: &Path,
    config: &Config,
    release: bool,
    no_watch: bool,
) -> Result<()> {
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
                    "Swift backend not configured for this project. Add it to Water.toml or recreate the project with the SwiftUI backend."
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

fn run_web(project_dir: &Path, config: &Config, release: bool, no_watch: bool) -> Result<()> {
    let web_config = config.backends.web.as_ref().ok_or_else(|| {
        eyre!("Web backend not configured for this project. Add it to Water.toml or recreate the project with the web backend.")
    })?;

    util::require_tool(
        "wasm-pack",
        "Install it from https://rustwasm.github.io/wasm-pack/ and try again.",
    )?;
    let wasm_pack = which("wasm-pack").unwrap();

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
    _listener: Arc<TcpListener>,
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    address: SocketAddr,
}

impl WebDevServer {
    fn start(root: PathBuf) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .context("failed to bind local development server")?;
        listener
            .set_nonblocking(true)
            .context("failed to configure web server socket")?;
        let address = listener
            .local_addr()
            .context("failed to read web server socket address")?;

        let listener = Arc::new(listener);
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_listener = Arc::clone(&listener);
        let thread_shutdown = Arc::clone(&shutdown);
        let root_dir = Arc::new(root);
        let thread_root = Arc::clone(&root_dir);

        let handle = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match thread_listener.accept() {
                    Ok((mut stream, _)) => {
                        if let Err(err) =
                            handle_http_connection(&mut stream, thread_root.as_ref().as_path())
                        {
                            warn!("Web server connection error: {}", err);
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(err) => {
                        warn!("Web server accept error: {}", err);
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        });

        Ok(Self {
            _listener: listener,
            shutdown,
            thread: Some(handle),
            address,
        })
    }

    fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for WebDevServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Trigger the accept loop to notice the shutdown flag.
        if let Ok(stream) = TcpStream::connect(self.address) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn handle_http_connection(stream: &mut TcpStream, root: &Path) -> std::io::Result<()> {
    let mut buffer = [0u8; 8192];
    let mut request = Vec::new();

    loop {
        let read_bytes = stream.read(&mut buffer)?;
        if read_bytes == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read_bytes]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() > 16 * 1024 {
            break;
        }
    }

    let request_str = String::from_utf8_lossy(&request);
    let mut lines = request_str.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path_part = parts.next().unwrap_or("/");
    let path_only = path_part.split('?').next().unwrap_or("/");

    let (status_line, body, content_type, send_body) = match method {
        "GET" | "HEAD" => {
            if let Some(mut file_path) = resolve_requested_path(root, path_only) {
                if file_path.is_dir() {
                    file_path.push("index.html");
                }
                match fs::read(&file_path) {
                    Ok(bytes) => (
                        "200 OK",
                        bytes,
                        content_type_for_path(&file_path),
                        method == "GET",
                    ),
                    Err(_) => (
                        "404 Not Found",
                        format!("File not found: {}", path_only).into_bytes(),
                        "text/plain; charset=utf-8",
                        method == "GET",
                    ),
                }
            } else {
                (
                    "404 Not Found",
                    format!("Not found: {}", path_only).into_bytes(),
                    "text/plain; charset=utf-8",
                    method == "GET",
                )
            }
        }
        _ => (
            "405 Method Not Allowed",
            b"Method Not Allowed".to_vec(),
            "text/plain; charset=utf-8",
            false,
        ),
    };

    let content_length = body.len();
    write!(
        stream,
        "HTTP/1.1 {}
",
        status_line
    )?;
    write!(
        stream,
        "Content-Length: {}
",
        content_length
    )?;
    write!(
        stream,
        "Content-Type: {}
",
        content_type
    )?;
    write!(
        stream,
        "Cache-Control: no-cache, no-store, must-revalidate\r\n"
    )?;
    write!(stream, "Pragma: no-cache\r\n")?;
    write!(stream, "Expires: 0\r\n")?;
    write!(stream, "Connection: close\r\n\r\n")?;
    if send_body {
        stream.write_all(&body)?;
    }
    stream.flush()?;

    Ok(())
}

fn resolve_requested_path(root: &Path, request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_start_matches('/');
    let mut parts: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();

    if request_path.ends_with('/') || parts.is_empty() {
        parts.push("index.html");
    }

    let mut path = PathBuf::from(root);
    for part in parts {
        if part == ".."
            || part.contains("..")
            || part.contains('\r')
            || part.contains('\n')
            || part.contains('\\')
        {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
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
    let status = build_cmd.status().context("failed to invoke xcodebuild")?;
    if !status.success() {
        bail!("xcodebuild failed with status {}", status);
    }

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

    let (sim_platform, default_device, products_path) = match platform {
        Platform::Ios => ("iOS Simulator", "iPhone 15", "iphonesimulator"),
        Platform::Ipados => (
            "iOS Simulator",
            "iPad Pro (11-inch) (4th generation)",
            "iphonesimulator",
        ),
        Platform::Watchos => (
            "watchOS Simulator",
            "Apple Watch Series 9 (45mm)",
            "watchsimulator",
        ),
        Platform::Tvos => (
            "tvOS Simulator",
            "Apple TV 4K (3rd generation)",
            "appletvsimulator",
        ),
        Platform::Visionos => ("visionOS Simulator", "Apple Vision Pro", "xrsimulator"),
        _ => bail!("Unsupported platform for simulator: {:?}", platform),
    };

    let project = resolve_xcode_project(project_dir, swift_config)?;

    let device_name = device.unwrap_or_else(|| default_device.to_string());
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
    let status = build_cmd.status().context("failed to invoke xcodebuild")?;
    if !status.success() {
        bail!("xcodebuild failed with status {}", status);
    }

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

    let apk_path = android::build_android_apk(project_dir, android_config, release, false)?;

    let adb_path = android::find_android_tool("adb").ok_or_else(|| {
        eyre!("`adb` not found. Install the Android SDK platform-tools and ensure they are on your PATH or ANDROID_HOME.")
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
    let activity = format!("{}/.MainActivity", package.bundle_identifier);
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

fn prompt_for_apple_device(platform: Platform) -> Result<String> {
    let raw_platform = apple_simulator_platform_id(platform);
    let devices = devices::list_devices()?;
    let mut candidates: Vec<DeviceInfo> = devices
        .into_iter()
        .filter(|d| d.kind == DeviceKind::Simulator)
        .filter(|d| d.raw_platform.as_deref() == Some(raw_platform))
        .collect();

    if candidates.is_empty() {
        bail!(
            "No simulators found for {}. Install one using Xcode's Devices window.",
            match platform {
                Platform::Ios => "iOS",
                Platform::Ipados => "iPadOS",
                Platform::Watchos => "watchOS",
                Platform::Tvos => "tvOS",
                Platform::Visionos => "visionOS",
                _ => "Apple",
            }
        );
    }

    candidates.sort_by(|a, b| a.name.cmp(&b.name));
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
