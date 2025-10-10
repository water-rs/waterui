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

use crate::{
    config::Config,
    util,
};

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
    Android,
}

pub fn run(args: RunArgs) -> Result<()> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;

    util::info(format!(
        "Running WaterUI app '{}'",
        config.package.display_name
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
        Platform::Macos
        | Platform::Ios
        | Platform::Ipados
        | Platform::Watchos
        | Platform::Tvos
        | Platform::Visionos => {
            if let Some(swift_config) = &config.backends.swift {
                util::info(format!("(Xcode scheme: {})", swift_config.scheme));

                match args.platform {
                    Platform::Macos => run_macos(&project_dir, swift_config, args.release)?,
                    Platform::Ios
                    | Platform::Ipados
                    | Platform::Watchos
                    | Platform::Tvos
                    | Platform::Visionos => run_apple_simulator(
                        &project_dir,
                        &config.package,
                        swift_config,
                        args.release,
                        args.platform,
                        args.device.clone(),
                    )?,
                    _ => unreachable!(),
                }
            } else {
                bail!(
                    "Swift backend not configured for this project. Add it to waterui.toml or recreate the project with the SwiftUI backend."
                );
            }
        }
        Platform::Android => {
            if let Some(android_config) = &config.backends.android {
                run_android(
                    &project_dir,
                    &config.package,
                    android_config,
                    args.release,
                    args.device,
                )?;
            } else {
                bail!(
                    "Android backend not configured for this project. Add it to waterui.toml or recreate the project with the Android backend."
                );
            }
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

fn run_macos(project_dir: &Path, swift_config: &crate::config::Swift, release: bool) -> Result<()> {
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

    let apple_root = project_dir.join(&swift_config.project_path);
    if !apple_root.exists() {
        bail!(
            "Xcode project directory not found at {}. Did you run 'water create'?",
            apple_root.display()
        );
    }

    let scheme = &swift_config.scheme;
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

fn run_apple_simulator(
    project_dir: &Path,
    package: &crate::config::Package,
    swift_config: &crate::config::Swift,
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

    let apple_root = project_dir.join(&swift_config.project_path);
    if !apple_root.exists() {
        bail!(
            "Xcode project directory not found at {}. Did you run 'water create'?",
            apple_root.display()
        );
    }

    let scheme = &swift_config.scheme;
    let project_path = apple_root.join(format!("{scheme}.xcodeproj"));
    if !project_path.exists() {
        bail!("Missing Xcode project: {}", project_path.display());
    }

    let device_name = device.unwrap_or_else(|| default_device.to_string());
    util::info(format!("Building for simulator {device_name}…"));

    let derived_root = project_dir.join(".waterui/DerivedData");
    util::ensure_directory(&derived_root)?;

    let configuration = if release { "Release" } else { "Debug" };

    let mut build_cmd = Command::new("xcodebuild");
    build_cmd
        .arg("-project")
        .arg(&project_path)
        .arg("-scheme")
        .arg(scheme)
        .arg("-destination")
        .arg(format!("platform={},name={}", sim_platform, device_name))
        .arg("-configuration")
        .arg(configuration)
        .arg("-derivedDataPath")
        .arg(&derived_root)
        .arg("CODE_SIGNING_ALLOWED=NO")
        .arg("CODE_SIGNING_REQUIRED=NO");

    util::debug(format!("Executing command: {:?}", build_cmd));
    let status = build_cmd.status().context("failed to invoke xcodebuild")?;
    if !status.success() {
        bail!("xcodebuild failed with status {status}");
    }

    let products_dir = derived_root.join(format!(
        "Build/Products/{}-{}",
        configuration, products_path
    ));
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
        &package.bundle_identifier,
    ]);
    let status = launch_cmd.status().context("failed to launch app")?;
    if !status.success() {
        bail!("Failed to launch app on simulator {device_name}");
    }

    util::info("Simulator launch complete. Press Ctrl+C to stop.");
    wait_for_interrupt()?;
    Ok(())
}

fn run_android(
    project_dir: &Path,
    package: &crate::config::Package,
    android_config: &crate::config::Android,
    release: bool,
    device: Option<String>,
) -> Result<()> {
    util::info("Running for Android...");

    for tool in ["adb", "emulator"] {
        if which::which(tool).is_err() {
            bail!(
                "{} not found. Make sure the Android SDK platform-tools and emulator are in your PATH.",
                tool
            );
        }
    }

    let build_rust_script = project_dir.join("build-rust.sh");
    if build_rust_script.exists() {
        util::info("Building Rust library for Android...");
        let mut cmd = Command::new("bash");
        cmd.arg(&build_rust_script);
        cmd.current_dir(project_dir);
        let status = cmd.status().context("failed to run build-rust.sh")?;
        if !status.success() {
            bail!("build-rust.sh failed");
        }
    }

    util::info("Building Android app with Gradle...");
    let android_dir = project_dir.join(&android_config.project_path);

    let gradlew_executable = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "./gradlew"
    };
    let mut cmd = Command::new(gradlew_executable);

    let task = if release {
        "assembleRelease"
    } else {
        "assembleDebug"
    };
    cmd.arg(task);
    cmd.current_dir(&android_dir);
    util::debug(format!("Running command: {:?}", cmd));
    let status = cmd.status().context("failed to run gradlew")?;
    if !status.success() {
        bail!("Gradle build failed");
    }

    let avd_name = if let Some(device_name) = device {
        device_name
    } else {
        let output = Command::new("emulator")
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
                "No Android emulators found. Please create one using Android Studio's AVD Manager."
            );
        }
        avds[0].clone()
    };

    util::info(format!("Using emulator: {}", avd_name));

    util::info("Launching emulator...");
    Command::new("emulator")
        .arg("-avd")
        .arg(&avd_name)
        .spawn()
        .context("failed to launch emulator")?;

    util::info("Waiting for device to be ready...");
    let status = Command::new("adb")
        .arg("wait-for-device")
        .status()
        .context("failed to run adb wait-for-device")?;
    if !status.success() {
        bail!("'adb wait-for-device' failed. Is the emulator running correctly?");
    }

    // Wait for boot to complete
    loop {
        let output = Command::new("adb")
            .args(["shell", "getprop", "sys.boot_completed"])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim() == "1" {
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }

    util::info("Installing APK...");
    let profile = if release { "release" } else { "debug" };
    let apk_name = if release {
        "app-release.apk".to_string()
    } else {
        "app-debug.apk".to_string()
    };
    let apk_path = android_dir.join(format!("app/build/outputs/apk/{}/{}", profile, apk_name));
    if !apk_path.exists() {
        bail!("APK not found at {}", apk_path.display());
    }

    let mut install_cmd = Command::new("adb");
    install_cmd.args(["install", "-r", apk_path.to_str().unwrap()]);
    util::debug(format!("Running command: {:?}", install_cmd));
    let status = install_cmd.status().context("failed to install APK")?;
    if !status.success() {
        bail!("Failed to install APK");
    }

    util::info("Launching app...");
    let activity = format!("{}/.MainActivity", package.bundle_identifier);
    let mut launch_cmd = Command::new("adb");
    launch_cmd.args(["shell", "am", "start", "-n", &activity]);
    util::debug(format!("Running command: {:?}", launch_cmd));
    let status = launch_cmd.status().context("failed to launch app")?;
    if !status.success() {
        bail!("Failed to launch app");
    }

    util::info("App launched. Press Ctrl+C to stop the watcher.");
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
