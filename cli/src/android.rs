use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use color_eyre::eyre::{Context, Result, bail, eyre};
use tracing::{debug, info, warn};
use which::which;

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
    let mut push_root = |path: PathBuf| {
        if path.exists() && !roots.contains(&path) {
            roots.push(path);
        }
    };

    if let Ok(path) = env::var("ANDROID_HOME") {
        push_root(PathBuf::from(path));
    }
    if let Ok(path) = env::var("ANDROID_SDK_ROOT") {
        push_root(PathBuf::from(path));
    }
    if let Ok(home) = env::var("HOME") {
        let home_path = PathBuf::from(home);
        push_root(home_path.join("Library/Android/sdk"));
        push_root(home_path.join("Android/Sdk"));
    }
    roots
}

pub fn resolve_android_sdk_path() -> Option<PathBuf> {
    android_sdk_roots().into_iter().next()
}

pub fn resolve_android_ndk_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("ANDROID_NDK_HOME") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    for sdk_root in android_sdk_roots() {
        let ndk_bundle = sdk_root.join("ndk-bundle");
        if ndk_bundle.exists() {
            return Some(ndk_bundle);
        }

        let ndk_dir = sdk_root.join("ndk");
        if let Ok(entries) = fs::read_dir(&ndk_dir) {
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect();
            candidates.sort_by(|a, b| b.cmp(a));
            if let Some(candidate) = candidates.into_iter().next() {
                return Some(candidate);
            }
        }
    }

    None
}

fn ndk_toolchain_bin(ndk_root: &Path) -> Option<PathBuf> {
    let host_tags: &[&str] = if cfg!(target_os = "macos") {
        &["darwin-arm64", "darwin-aarch64", "darwin-x86_64"]
    } else if cfg!(target_os = "linux") {
        &["linux-aarch64", "linux-x86_64"]
    } else if cfg!(target_os = "windows") {
        &["windows-x86_64"]
    } else {
        &[]
    };

    for tag in host_tags {
        let candidate = ndk_root
            .join("toolchains/llvm/prebuilt")
            .join(tag)
            .join("bin");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let prebuilt_dir = ndk_root.join("toolchains/llvm/prebuilt");
    if let Ok(entries) = fs::read_dir(&prebuilt_dir) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("bin");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
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
            let ndk_env = env::var("ANDROID_NDK_HOME")
                .ok()
                .map(PathBuf::from)
                .filter(|path| path.exists());
            let ndk_path = ndk_env.clone().or_else(resolve_android_ndk_path);
            if let Some(ndk_path) = ndk_path {
                if ndk_env.is_none() {
                    info!(
                        "ANDROID_NDK_HOME not set; using auto-detected NDK at {}",
                        ndk_path.display()
                    );
                }
                cmd.env("ANDROID_NDK_HOME", &ndk_path);
                if let Some(toolchain_bin) = ndk_toolchain_bin(&ndk_path) {
                    let mut new_path = env::split_paths(&env::var_os("PATH").unwrap_or_default())
                        .collect::<Vec<_>>();
                    new_path.insert(0, toolchain_bin);
                    let merged = env::join_paths(new_path).expect("failed to join PATH entries");
                    cmd.env("PATH", merged);
                } else {
                    warn!(
                        "Unable to locate NDK toolchain binaries under {}. \
                         Ensure the llvm toolchain is installed.",
                        ndk_path.display()
                    );
                }
            } else {
                warn!(
                    "ANDROID_NDK_HOME not set and NDK could not be auto-detected. \
                     build-rust.sh may fail if it requires the NDK."
                );
            }
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
        let sdk_path = resolve_android_sdk_path().ok_or_else(|| {
            eyre!(
                "Android SDK not found. Install it via Android Studio, place it under ~/Library/Android/sdk (macOS) or ~/Android/Sdk, or set ANDROID_HOME/ANDROID_SDK_ROOT; alternatively create {} manually.",
                local_properties.display()
            )
        })?;

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
