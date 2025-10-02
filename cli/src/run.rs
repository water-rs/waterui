use std::{
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

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{config::Config, util};

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Target platform to run
    #[arg(long, default_value = "macos", value_enum)]
    pub platform: Platform,

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
}

pub fn run(args: RunArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;

    util::info(format!(
        "Running WaterUI app '{}' (Xcode scheme: {})",
        config.package.display_name, config.swift.scheme
    ));

    run_cargo_build(&project_dir, &config.package.name, args.release)?;

    let watcher = if args.no_watch {
        util::info("CLI hot reload watcher disabled (--no-watch)");
        None
    } else {
        Some(RebuildWatcher::new(
            &project_dir,
            &config.package.name,
            args.release,
            &config.hot_reload.watch,
        )?)
    };

    match args.platform {
        Platform::Macos => run_macos(&project_dir, &config, args.release)?,
        Platform::Ios => run_ios(
            &project_dir,
            &config,
            args.release,
            Platform::Ios,
            args.device.clone(),
        )?,
        Platform::Ipados => run_ios(
            &project_dir,
            &config,
            args.release,
            Platform::Ipados,
            args.device.clone(),
        )?,
        Platform::Watchos | Platform::Tvos | Platform::Visionos => {
            bail!("Platform {:?} is not implemented yet", args.platform)
        }
    }

    drop(watcher);
    Ok(())
}

fn run_cargo_build(project_dir: &Path, package: &str, release: bool) -> Result<()> {
    util::info("Compiling Rust dynamic library...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--package").arg(package);
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(project_dir);
    util::debug(format!("Running command: {:?}", cmd));
    let status = cmd
        .status()
        .with_context(|| format!("failed to run cargo build in {}", project_dir.display()))?;
    if !status.success() {
        bail!("cargo build failed");
    }
    Ok(())
}

fn run_macos(project_dir: &Path, config: &Config, release: bool) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("SwiftUI backend is currently only supported on macOS");
    }

    #[cfg(target_os = "macos")]
    {
        for tool in ["xcodebuild"] {
            if which::which(tool).is_err() {
                bail!(
                    "{tool} not found. Install Xcode command line tools (xcode-select --install)"
                );
            }
        }
    }

    let apple_root = project_dir.join(&config.swift.project_path);
    if !apple_root.exists() {
        bail!(
            "Xcode project directory not found at {}. Did you run 'water create'?",
            apple_root.display()
        );
    }

    let scheme = &config.swift.scheme;
    let project_path = apple_root.join(format!("{scheme}.xcodeproj"));
    if !project_path.exists() {
        bail!("Missing Xcode project: {}", project_path.display());
    }

    let derived_root = project_dir.join(".waterui/DerivedData");
    util::ensure_directory(&derived_root)?;

    let configuration = if release { "Release" } else { "Debug" };

    let mut build_cmd = Command::new("xcodebuild");
    build_cmd
        .arg("-project")
        .arg(&project_path)
        .arg("-scheme")
        .arg(scheme)
        .arg("-configuration")
        .arg(configuration)
        .arg("-derivedDataPath")
        .arg(&derived_root)
        .arg("-destination")
        .arg("platform=macOS");

    util::info("Building macOS app with xcodebuild…");
    util::debug(format!("Executing command: {:?}", build_cmd));
    let status = build_cmd.status().context("failed to invoke xcodebuild")?;
    if !status.success() {
        bail!("xcodebuild failed with status {status}");
    }

    let products_dir = derived_root.join(format!("Build/Products/{configuration}"));
    let app_bundle = products_dir.join(format!("{scheme}.app"));
    if !app_bundle.exists() {
        bail!("Expected app bundle at {}", app_bundle.display());
    }

    util::info("Launching app…");
    let status = Command::new("open")
        .arg(&app_bundle)
        .status()
        .context("failed to open app bundle")?;
    if !status.success() {
        bail!("Failed to launch app");
    }

    util::info("App launched. Press Ctrl+C to stop the watcher.");
    wait_for_interrupt()?;
    Ok(())
}

fn run_ios(
    project_dir: &Path,
    config: &Config,
    release: bool,
    platform: Platform,
    device: Option<String>,
) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("Running Apple simulators requires macOS");
    }

    #[cfg(target_os = "macos")]
    {
        for tool in ["xcrun", "xcodebuild"] {
            if which::which(tool).is_err() {
                bail!(
                    "{} not found. Install Xcode command line tools (xcode-select --install)",
                    tool
                );
            }
        }
    }

    let apple_root = project_dir.join(&config.swift.project_path);
    if !apple_root.exists() {
        bail!(
            "Xcode project directory not found at {}. Did you run 'water create'?",
            apple_root.display()
        );
    }

    let scheme = &config.swift.scheme;
    let project_path = apple_root.join(format!("{scheme}.xcodeproj"));
    if !project_path.exists() {
        bail!("Missing Xcode project: {}", project_path.display());
    }

    let default_device = match platform {
        Platform::Ipados => "iPad Pro (11-inch) (4th generation)",
        _ => "iPhone 15",
    };
    let device_name = device.unwrap_or_else(|| default_device.to_string());
    util::info(format!("Building for simulator {device_name}…"));

    let derived_root = project_dir.join(".waterui/DerivedData");
    util::ensure_directory(&derived_root)?;

    let mut build_cmd = Command::new("xcodebuild");
    build_cmd
        .arg("-project")
        .arg(&project_path)
        .arg("-scheme")
        .arg(scheme)
        .arg("-destination")
        .arg(format!("platform=iOS Simulator,name={device_name}"))
        .arg("-configuration")
        .arg(if release { "Release" } else { "Debug" })
        .arg("-derivedDataPath")
        .arg(&derived_root)
        .arg("CODE_SIGNING_ALLOWED=NO")
        .arg("CODE_SIGNING_REQUIRED=NO");

    util::debug(format!("Executing command: {:?}", build_cmd));
    let status = build_cmd.status().context("failed to invoke xcodebuild")?;
    if !status.success() {
        bail!("xcodebuild failed with status {status}");
    }

    let products_dir = if release {
        derived_root.join("Build/Products/Release-iphonesimulator")
    } else {
        derived_root.join("Build/Products/Debug-iphonesimulator")
    };
    let app_bundle = products_dir.join(format!("{scheme}.app"));
    if !app_bundle.exists() {
        bail!(
            "Expected app bundle at {}, but it was not created",
            app_bundle.display()
        );
    }

    util::info("Booting simulator…");
    let mut boot_cmd = Command::new("xcrun");
    boot_cmd.args(["simctl", "boot", &device_name]);
    let _ = boot_cmd.status(); // Ignore errors if already booted

    util::info("Installing app on simulator…");
    let mut install_cmd = Command::new("xcrun");
    install_cmd.args([
        "simctl",
        "install",
        &device_name,
        app_bundle.to_str().unwrap(),
    ]);
    let status = install_cmd
        .status()
        .context("failed to install app on simulator")?;
    if !status.success() {
        bail!("Failed to install app on simulator {device_name}");
    }

    util::info("Launching app…");
    let mut launch_cmd = Command::new("xcrun");
    launch_cmd.args([
        "simctl",
        "launch",
        "--terminate-running-process",
        &device_name,
        &config.package.bundle_identifier,
    ]);
    let status = launch_cmd.status().context("failed to launch app")?;
    if !status.success() {
        bail!("Failed to launch app on simulator {device_name}");
    }

    util::info("Simulator launch complete. Press Ctrl+C to stop.");
    wait_for_interrupt()?;
    Ok(())
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

struct RebuildWatcher {
    _watcher: RecommendedWatcher,
    signal: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RebuildWatcher {
    fn new(
        project_dir: &Path,
        package: &str,
        release: bool,
        extra_paths: &[String],
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

        watcher.watch(&project_dir.join("src"), RecursiveMode::Recursive)?;
        for path in extra_paths {
            let watch_path = project_dir.join(path);
            if watch_path.exists() {
                watcher.watch(&watch_path, RecursiveMode::Recursive)?;
            }
        }

        let project_dir = project_dir.to_path_buf();
        let package = package.to_string();
        let signal = Arc::new(AtomicBool::new(false));
        let shutdown_flag = signal.clone();

        let handle = thread::spawn(move || {
            util::info("Hot reload watcher started (CLI)");
            let mut last_run = Instant::now();
            while !shutdown_flag.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(_) => {
                        if last_run.elapsed() < Duration::from_millis(250) {
                            continue;
                        }
                        if let Err(err) = run_cargo_build(&project_dir, &package, release) {
                            util::warn(format!("Rebuild failed: {err}"));
                        }
                        last_run = Instant::now();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            util::debug("Hot reload watcher stopped");
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
