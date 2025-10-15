use std::{
    env,
    fs::{self, File},
    io::{Write, copy},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use color_eyre::eyre::{Context, Result, bail, eyre};
use tracing::{debug, info};
use which::which;

use zip::ZipArchive;

pub fn find_android_tool(tool: &str) -> Option<PathBuf> {
    if let Ok(path) = which(tool) {
        return Some(path);
    }

    let suffixes: &[&str] = match tool {
        "adb" => &["platform-tools/adb", "platform-tools/adb.exe"],
        "emulator" => &["emulator/emulator", "emulator/emulator.exe"],
        _ => &[],
    };

    for root in android_sdk_roots() {
        for suffix in suffixes {
            let candidate = root.join(suffix);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

pub fn android_sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(path) = env::var("ANDROID_HOME") {
        roots.push(PathBuf::from(path));
    }
    if let Ok(path) = env::var("ANDROID_SDK_ROOT") {
        roots.push(PathBuf::from(path));
    }
    if let Ok(home) = env::var("HOME") {
        let home_path = PathBuf::from(home);
        roots.push(home_path.join("Library/Android/sdk"));
        roots.push(home_path.join("Android/Sdk"));
    }
    roots.into_iter().filter(|p| p.exists()).collect()
}

const CMDLINE_TOOLS_VERSION: &str = "11076708";
const REQUIRED_SDK_COMPONENTS: &[&str] = &[
    "platform-tools",
    "platforms;android-34",
    "build-tools;34.0.0",
    "ndk;26.2.11394342",
];

pub fn install_android_toolchain() -> Result<()> {
    println!("Preparing Android SDK installation…");

    let home_dir = home::home_dir().ok_or_else(|| eyre!("Unable to determine home directory"))?;
    let android_dir = home_dir.join(".waterui").join("android");
    fs::create_dir_all(&android_dir).context("failed to create WaterUI Android directory")?;

    let sdk_root = android_dir.join("sdk");
    fs::create_dir_all(&sdk_root).context("failed to create Android SDK directory")?;

    let sdkmanager_path = ensure_command_line_tools(&sdk_root)?;
    install_sdk_components(&sdkmanager_path, &sdk_root)?;
    accept_sdk_licenses(&sdkmanager_path, &sdk_root)?;

    let ndk_path = find_installed_ndk(&sdk_root)?;

    println!("Android SDK installed at {}", sdk_root.display());
    println!("Android NDK installed at {}", ndk_path.display());
    println!("Update your environment variables to complete setup:");
    println!("  export ANDROID_HOME=\"{}\"", sdk_root.display());
    println!("  export ANDROID_SDK_ROOT=\"{}\"", sdk_root.display());
    println!("  export ANDROID_NDK_HOME=\"{}\"", ndk_path.display());

    Ok(())
}

fn ensure_command_line_tools(sdk_root: &Path) -> Result<PathBuf> {
    let cmdline_root = sdk_root.join("cmdline-tools");
    let latest_dir = cmdline_root.join("latest");
    let sdkmanager_path = sdkmanager_executable(&latest_dir);

    if sdkmanager_path.exists() {
        return Ok(sdkmanager_path);
    }

    if latest_dir.exists() {
        bail!(
            "Android command-line tools directory {} exists without sdkmanager executable. Remove it before reinstalling.",
            latest_dir.display()
        );
    }

    download_command_line_tools(&cmdline_root, &latest_dir)?;
    Ok(sdkmanager_path)
}

fn download_command_line_tools(cmdline_root: &Path, latest_dir: &Path) -> Result<()> {
    fs::create_dir_all(cmdline_root).context("failed to create cmdline-tools directory")?;

    let url = command_line_tools_url()?;
    println!("Downloading Android command-line tools from {url}…");

    let mut response = reqwest::blocking::get(&url)
        .wrap_err_with(|| format!("failed to download Android command-line tools from {url}"))?;
    if !response.status().is_success() {
        bail!(
            "Android command-line tools download failed with status {}",
            response.status()
        );
    }

    let archive_path = cmdline_root.join("commandlinetools.zip");
    {
        let mut file = File::create(&archive_path)
            .context("failed to create temporary command-line tools archive")?;
        copy(&mut response, &mut file)
            .context("failed to write Android command-line tools archive")?;
    }

    let file = File::open(&archive_path).context("failed to reopen downloaded archive")?;
    let mut archive =
        ZipArchive::new(file).context("invalid Android command-line tools archive")?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .context("failed to read archive entry")?;
        let entry_path = entry
            .enclosed_name()
            .ok_or_else(|| eyre!("archive entry contains invalid path"))?;

        let mut components = entry_path.components();
        let root = components
            .next()
            .ok_or_else(|| eyre!("archive entry missing root directory"))?;
        if root.as_os_str() != "cmdline-tools" {
            bail!(
                "unexpected entry '{}' in Android command-line tools archive",
                entry_path.display()
            );
        }

        let mut relative = PathBuf::new();
        for component in components {
            relative.push(component);
        }

        let out_path = latest_dir.join(relative);

        if entry.is_dir() {
            fs::create_dir_all(&out_path).with_context(|| {
                format!(
                    "failed to create directory {} in command-line tools",
                    out_path.display()
                )
            })?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directory {}", parent.display())
            })?;
        }

        let mut outfile = File::create(&out_path).with_context(|| {
            format!(
                "failed to create file {} in command-line tools",
                out_path.display()
            )
        })?;
        copy(&mut entry, &mut outfile)
            .with_context(|| format!("failed to extract {}", out_path.display()))?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(mode);
            fs::set_permissions(&out_path, permissions)
                .with_context(|| format!("failed to set permissions on {}", out_path.display()))?;
        }
    }

    fs::remove_file(&archive_path).context("failed to remove temporary archive")?;
    println!(
        "Installed Android command-line tools to {}",
        latest_dir.display()
    );
    Ok(())
}

fn command_line_tools_url() -> Result<String> {
    let platform = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "mac",
        "windows" => "win",
        other => bail!("Android SDK installation is not supported on {other}"),
    };

    Ok(format!(
        "https://dl.google.com/android/repository/commandlinetools-{platform}-{version}_latest.zip",
        platform = platform,
        version = CMDLINE_TOOLS_VERSION
    ))
}

fn install_sdk_components(sdkmanager_path: &Path, sdk_root: &Path) -> Result<()> {
    println!("Installing Android SDK components…");

    let mut command = Command::new(sdkmanager_path);
    command.arg(format!("--sdk_root={}", sdk_root.display()));
    command.args(REQUIRED_SDK_COMPONENTS);
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());

    let status = command
        .status()
        .context("failed to execute sdkmanager to install SDK components")?;

    if !status.success() {
        bail!("sdkmanager exited with status {status}");
    }

    Ok(())
}

fn accept_sdk_licenses(sdkmanager_path: &Path, sdk_root: &Path) -> Result<()> {
    println!("Accepting Android SDK licenses…");

    let mut child = Command::new(sdkmanager_path)
        .arg(format!("--sdk_root={}", sdk_root.display()))
        .arg("--licenses")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to execute sdkmanager --licenses")?;

    if let Some(mut stdin) = child.stdin.take() {
        thread::spawn(move || {
            for _ in 0..64 {
                if stdin.write_all(b"y\n").is_err() {
                    break;
                }
            }
        });
    }

    let status = child
        .wait()
        .context("sdkmanager --licenses command failed to complete")?;

    if !status.success() {
        bail!("sdkmanager --licenses exited with status {status}");
    }

    Ok(())
}

fn find_installed_ndk(sdk_root: &Path) -> Result<PathBuf> {
    let ndk_root = sdk_root.join("ndk");
    let entries = fs::read_dir(&ndk_root)
        .with_context(|| format!("Android NDK directory {} missing", ndk_root.display()))?;

    for entry in entries {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            return Ok(entry.path());
        }
    }

    bail!(
        "Android NDK installation incomplete: no directories found in {}",
        ndk_root.display()
    );
}

fn sdkmanager_executable(latest_dir: &Path) -> PathBuf {
    let bin_dir = latest_dir.join("bin");
    if cfg!(windows) {
        bin_dir.join("sdkmanager.bat")
    } else {
        bin_dir.join("sdkmanager")
    }
}

pub fn build_android_apk(
    project_dir: &Path,
    android_config: &crate::config::Android,
    release: bool,
    skip_native: bool,
) -> Result<PathBuf> {
    let build_rust_script = project_dir.join("build-rust.sh");
    if build_rust_script.exists() {
        if skip_native {
            info!("Skipping Android native build (requested via --skip-native)");
        } else {
            info!("Building Rust library for Android...");
            let mut cmd = Command::new("bash");
            cmd.arg(&build_rust_script);
            cmd.current_dir(project_dir);
            let status = cmd.status().context("failed to run build-rust.sh")?;
            if !status.success() {
                bail!("build-rust.sh failed");
            }
        }
    } else if !skip_native {
        info!("No build-rust.sh script found. Skipping native build.");
    }

    info!("Building Android app with Gradle...");
    let android_dir = project_dir.join(&android_config.project_path);

    let local_properties = android_dir.join("local.properties");
    if !local_properties.exists() {
        let sdk_path = env::var("ANDROID_SDK_ROOT")
            .or_else(|_| env::var("ANDROID_HOME"))
            .map(PathBuf::from)
            .map_err(|_| {
                eyre!(
                    "Android SDK not found. Set ANDROID_HOME or ANDROID_SDK_ROOT, or create {}",
                    local_properties.display()
                )
            })?;

        if !sdk_path.exists() {
            bail!(
                "Android SDK directory '{}' does not exist. Update ANDROID_HOME/ANDROID_SDK_ROOT or create {} manually with a valid sdk.dir entry.",
                sdk_path.display(),
                local_properties.display()
            );
        }

        let contents = format!("sdk.dir={}\n", sdk_path.to_string_lossy());
        fs::write(&local_properties, contents).context("failed to write local.properties")?;
        info!(
            "Wrote Android SDK location {} to {}",
            sdk_path.display(),
            local_properties.display()
        );
    }

    let gradlew_executable = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "./gradlew"
    };
    let mut cmd = Command::new(gradlew_executable);

    let ipv4_flag = "-Djava.net.preferIPv4Stack=true";
    let gradle_opts = ensure_jvm_flag(env::var("GRADLE_OPTS").ok(), ipv4_flag);
    cmd.env("GRADLE_OPTS", &gradle_opts);
    let java_tool_options = ensure_jvm_flag(env::var("JAVA_TOOL_OPTIONS").ok(), ipv4_flag);
    cmd.env("JAVA_TOOL_OPTIONS", &java_tool_options);

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

    let profile = if release { "release" } else { "debug" };
    let apk_name = if release {
        "app-release.apk"
    } else {
        "app-debug.apk"
    };
    let apk_path = android_dir.join(format!("app/build/outputs/apk/{}/{}", profile, apk_name));
    if !apk_path.exists() {
        bail!("APK not found at {}", apk_path.display());
    }

    info!("Generated {} APK at {}", profile, apk_path.display());
    Ok(apk_path)
}

pub fn wait_for_android_device(adb_path: &Path, identifier: Option<&str>) -> Result<()> {
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

pub fn adb_command(adb_path: &Path, identifier: Option<&str>) -> Command {
    let mut cmd = Command::new(adb_path);
    if let Some(id) = identifier {
        cmd.arg("-s").arg(id);
    }
    cmd
}

fn ensure_jvm_flag(existing: Option<String>, flag: &str) -> String {
    if let Some(current) = existing {
        let trimmed = current.trim();
        if trimmed.split_whitespace().any(|token| token == flag) {
            trimmed.to_string()
        } else if trimmed.is_empty() {
            flag.to_string()
        } else {
            format!("{trimmed} {flag}")
        }
    } else {
        flag.to_string()
    }
}
