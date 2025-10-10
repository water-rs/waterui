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
use dialoguer::{Select, theme::ColorfulTheme};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, info, warn};

use crate::{
    apple::{
        derived_data_dir, disable_code_signing, ensure_macos_host, prepare_derived_data_dir,
        require_tool, resolve_xcode_project, xcodebuild_base,
    },
    config::Config,
    devices::{self, DeviceInfo, DeviceKind},
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

    info!("Running WaterUI app '{}'", config.package.display_name);

    run_cargo_build(&project_dir, &config.package.name, args.release)?;

    let watcher = if args.no_watch {
        info!("CLI hot reload watcher disabled (--no-watch)");
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
                info!("(Xcode scheme: {})", swift_config.scheme);

                match args.platform {
                    Platform::Macos => run_macos(&project_dir, swift_config, args.release)?,
                    Platform::Ios
                    | Platform::Ipados
                    | Platform::Watchos
                    | Platform::Tvos
                    | Platform::Visionos => {
                        let device_name = match args.device.clone() {
                            Some(name) => name,
                            None => prompt_for_apple_device(args.platform)?,
                        };
                        run_apple_simulator(
                            &project_dir,
                            &config.package,
                            swift_config,
                            args.release,
                            args.platform,
                            Some(device_name),
                        )?
                    }
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
                let selection = match args.device.clone() {
                    Some(name) => Some(resolve_android_device(&name)?),
                    None => Some(prompt_for_android_device()?),
                };
                run_android(
                    &project_dir,
                    &config.package,
                    android_config,
                    args.release,
                    selection,
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
    info!("Compiling Rust dynamic library...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--package").arg(package);
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(project_dir);
    debug!("Running command: {:?}", cmd);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run cargo build in {}", project_dir.display()))?;
    if !status.success() {
        bail!("cargo build failed");
    }
    Ok(())
}

fn run_macos(project_dir: &Path, swift_config: &crate::config::Swift, release: bool) -> Result<()> {
    ensure_macos_host("SwiftUI backend support")?;
    require_tool(
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
        bail!("xcodebuild failed with status {status}");
    }

    let products_dir = derived_root.join(format!("Build/Products/{configuration}"));
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
        require_tool(
            tool,
            "Install Xcode and command line tools (xcode-select --install)",
        )?;
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
        bail!("xcodebuild failed with status {status}");
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
        app_bundle.to_str().unwrap(),
    ]);
    let status = install_cmd
        .status()
        .context("failed to install app on simulator")?;
    if !status.success() {
        bail!("Failed to install app on simulator {device_name}");
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
        bail!("Failed to launch app on simulator {device_name}");
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

    let adb_path = devices::find_android_tool("adb").ok_or_else(|| {
        anyhow::anyhow!(
            "`adb` not found. Install the Android SDK platform-tools and ensure they are on your PATH or ANDROID_HOME."
        )
    })?;
    let emulator_path = devices::find_android_tool("emulator");

    let build_rust_script = project_dir.join("build-rust.sh");
    if build_rust_script.exists() {
        info!("Building Rust library for Android...");
        let mut cmd = Command::new("bash");
        cmd.arg(&build_rust_script);
        cmd.current_dir(project_dir);
        let status = cmd.status().context("failed to run build-rust.sh")?;
        if !status.success() {
            bail!("build-rust.sh failed");
        }
    }

    info!("Building Android app with Gradle...");
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
    debug!("Running command: {:?}", cmd);
    let status = cmd.status().context("failed to run gradlew")?;
    if !status.success() {
        bail!("Gradle build failed");
    }

    let selection = if let Some(selection) = selection {
        selection
    } else {
        let emulator = emulator_path.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "No Android emulator available. Install the Android SDK emulator tools or specify a connected device."
            )
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
            anyhow::anyhow!(
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
    wait_for_android_device(&adb_path, target_identifier.as_deref())?;

    info!("Installing APK...");
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

    let mut install_cmd = adb_command(&adb_path, target_identifier.as_deref());
    install_cmd.args(["install", "-r", apk_path.to_str().unwrap()]);
    debug!("Running command: {:?}", install_cmd);
    let status = install_cmd.status().context("failed to install APK")?;
    if !status.success() {
        bail!("Failed to install APK");
    }

    info!("Launching app...");
    let activity = format!("{}/.MainActivity", package.bundle_identifier);
    let mut launch_cmd = adb_command(&adb_path, target_identifier.as_deref());
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
        "Android device or emulator '{name}' not found. Run `water devices` to list available targets."
    );
}

fn apple_simulator_platform_id(platform: Platform) -> &'static str {
    match platform {
        Platform::Ios | Platform::Ipados => "com.apple.platform.iphonesimulator",
        Platform::Watchos => "com.apple.platform.watchsimulator",
        Platform::Tvos => "com.apple.platform.appletvsimulator",
        Platform::Visionos => "com.apple.platform.visionossimulator",
        Platform::Macos | Platform::Android => "",
    }
}

fn wait_for_android_device(adb_path: &Path, identifier: Option<&str>) -> Result<()> {
    let mut wait_cmd = adb_command(adb_path, identifier);
    wait_cmd.arg("wait-for-device");
    let status = wait_cmd
        .status()
        .context("failed to run adb wait-for-device")?;
    if !status.success() {
        bail!("'adb wait-for-device' failed. Is the device/emulator running correctly?");
    }

    // Wait for Android to finish booting (best effort)
    loop {
        let output = adb_command(adb_path, identifier)
            .args(["shell", "getprop", "sys.boot_completed"])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim() == "1" {
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn adb_command(adb_path: &Path, identifier: Option<&str>) -> Command {
    let mut cmd = Command::new(adb_path);
    if let Some(id) = identifier {
        cmd.arg("-s").arg(id);
    }
    cmd
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
            info!("Hot reload watcher started (CLI)");
            let mut last_run = Instant::now();
            while !shutdown_flag.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(_) => {
                        if last_run.elapsed() < Duration::from_millis(250) {
                            continue;
                        }
                        if let Err(err) = run_cargo_build(&project_dir, &package, release) {
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
