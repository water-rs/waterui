use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use crate::util;
use color_eyre::eyre::{Context, Result, bail, eyre};
use tracing::{debug, info, warn};
use which::which;

struct AndroidTargetConfig {
    triple: &'static str,
    bin_prefix: &'static str,
    min_api: u32,
}

const ANDROID_TARGETS: &[AndroidTargetConfig] = &[
    AndroidTargetConfig {
        triple: "aarch64-linux-android",
        bin_prefix: "aarch64-linux-android",
        min_api: 21,
    },
    AndroidTargetConfig {
        triple: "x86_64-linux-android",
        bin_prefix: "x86_64-linux-android",
        min_api: 21,
    },
    AndroidTargetConfig {
        triple: "armv7-linux-androideabi",
        bin_prefix: "armv7a-linux-androideabi",
        min_api: 19,
    },
    AndroidTargetConfig {
        triple: "i686-linux-android",
        bin_prefix: "i686-linux-android",
        min_api: 19,
    },
];

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

fn installed_rust_targets() -> Option<HashSet<String>> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let list = String::from_utf8_lossy(&output.stdout);
    let targets = list
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect::<HashSet<_>>();
    Some(targets)
}

fn target_by_triple(triple: &str) -> Option<&'static AndroidTargetConfig> {
    ANDROID_TARGETS
        .iter()
        .find(|target| target.triple == triple)
}

fn target_for_abi(abi: &str) -> Option<&'static AndroidTargetConfig> {
    match abi {
        "arm64-v8a" | "arm64" => target_by_triple("aarch64-linux-android"),
        "armeabi-v7a" | "armeabi" => target_by_triple("armv7-linux-androideabi"),
        "x86_64" => target_by_triple("x86_64-linux-android"),
        "x86" => target_by_triple("i686-linux-android"),
        _ => None,
    }
}

pub fn device_preferred_targets(
    adb_path: &Path,
    identifier: Option<&str>,
) -> Result<Vec<&'static str>> {
    let abis = query_device_abis(adb_path, identifier)?;
    if abis.is_empty() {
        bail!("Unable to determine Android device ABI via adb.");
    }

    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for abi in &abis {
        if let Some(target) = target_for_abi(abi) {
            if seen.insert(target.triple) {
                targets.push(target.triple);
            }
        }
    }

    if targets.is_empty() {
        bail!(
            "No supported Rust targets match the device ABI list: {}",
            abis.join(", ")
        );
    }

    Ok(targets)
}

fn query_device_abis(adb_path: &Path, identifier: Option<&str>) -> Result<Vec<String>> {
    let mut abis = Vec::new();
    let candidates = [
        "ro.product.cpu.abilist",
        "ro.product.cpu.abi",
        "ro.product.cpu.abi2",
    ];
    for prop in candidates {
        let output = adb_command(adb_path, identifier)
            .args(["shell", "getprop", prop])
            .output()
            .with_context(|| format!("failed to query device property {prop}"))?;
        if !output.status.success() {
            continue;
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            continue;
        }
        abis.extend(
            value
                .split(',')
                .map(|abi| abi.trim().to_string())
                .filter(|abi| !abi.is_empty()),
        );
        if !abis.is_empty() {
            break;
        }
    }
    Ok(abis)
}

pub fn sanitize_package_name(input: &str) -> String {
    let mut parts = Vec::new();
    for raw in input.split('.') {
        if raw.is_empty() {
            continue;
        }

        let mut segment = String::new();
        for ch in raw.chars() {
            match ch {
                'a'..='z' | '0'..='9' => segment.push(ch),
                'A'..='Z' => segment.push(ch.to_ascii_lowercase()),
                '_' => segment.push('_'),
                '-' => segment.push('_'),
                _ => {
                    if ch.is_alphanumeric() {
                        segment.push(ch.to_ascii_lowercase());
                    } else {
                        segment.push('_');
                    }
                }
            }
        }

        if segment.is_empty() {
            segment.push('a');
        } else {
            let first = segment.chars().next().unwrap();
            if !first.is_ascii_alphabetic() && first != '_' {
                segment.insert(0, 'a');
            }
        }

        parts.push(segment);
    }

    if parts.is_empty() {
        "com.waterui.app".to_string()
    } else {
        parts.join(".")
    }
}

fn detect_java_major(home: &Path) -> Option<u32> {
    let java_bin = home.join("bin/java");
    if !java_bin.exists() {
        return None;
    }
    let output = Command::new(java_bin).arg("-version").output().ok()?;
    let payload = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    let version_text = String::from_utf8_lossy(payload);
    for line in version_text.lines() {
        if let Some(start) = line.find('"') {
            let remainder = &line[start + 1..];
            if let Some(end) = remainder.find('"') {
                let version = &remainder[..end];
                let major_candidate = if let Some(rest) = version.strip_prefix("1.") {
                    rest.split(|c: char| !c.is_ascii_digit()).next()
                } else {
                    version.split(|c: char| !c.is_ascii_digit()).next()
                }?;
                if let Ok(value) = major_candidate.parse::<u32>() {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn prefer_java_home() -> Option<PathBuf> {
    const MAX_SUPPORTED: u32 = 21;

    let check_candidate = |value: Option<PathBuf>| -> Option<PathBuf> {
        let path = value?;
        match detect_java_major(&path) {
            Some(version) if version <= MAX_SUPPORTED => Some(path),
            _ => None,
        }
    };

    if let Some(home) = check_candidate(env::var_os("JAVA_HOME").map(PathBuf::from)) {
        return Some(home);
    }
    if let Some(home) = check_candidate(env::var_os("ANDROID_JAVA_HOME").map(PathBuf::from)) {
        return Some(home);
    }

    #[cfg(target_os = "macos")]
    {
        const CANDIDATES: &[&str] = &["17", "21", "20", "19", "18"];
        for version in CANDIDATES {
            let output = Command::new("/usr/libexec/java_home")
                .args(["-v", version])
                .output()
                .ok()?;
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if path.is_empty() {
                    continue;
                }
                let candidate = PathBuf::from(path);
                if detect_java_major(&candidate).is_some() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn move_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            move_dir_contents(&src_path, &dst_path)?;
            if src_path != dst_path {
                fs::remove_dir_all(&src_path).ok();
            }
        } else if let Err(err) = fs::rename(&src_path, &dst_path) {
            const CROSS_DEVICE_ERR: i32 = 18; // EXDEV on Unix platforms
            let cross_device = err.raw_os_error() == Some(CROSS_DEVICE_ERR);
            if err.kind() == std::io::ErrorKind::Unsupported || cross_device {
                fs::copy(&src_path, &dst_path)?;
                fs::remove_file(&src_path)?;
            } else {
                return Err(err)
                    .context("failed to move file while sanitizing Android package structure");
            }
        }
    }
    Ok(())
}

fn prepare_android_package(project_dir: &Path, bundle_identifier: &str) -> Result<String> {
    let sanitized = sanitize_package_name(bundle_identifier);
    if sanitized == bundle_identifier {
        return Ok(sanitized);
    }

    let android_dir = project_dir.join("android");
    let app_dir = android_dir.join("app");
    let gradle_file = app_dir.join("build.gradle.kts");
    if gradle_file.exists() {
        let contents = fs::read_to_string(&gradle_file)?;
        if contents.contains(bundle_identifier) {
            let updated = contents.replace(bundle_identifier, &sanitized);
            if updated != contents {
                fs::write(&gradle_file, updated)?;
            }
        }
    }

    let java_dir = app_dir.join("src/main/java");
    let original_path = java_dir.join(bundle_identifier.replace('.', "/"));
    let sanitized_path = java_dir.join(sanitized.replace('.', "/"));
    if original_path.exists() && original_path != sanitized_path {
        move_dir_contents(&original_path, &sanitized_path)?;
        fs::remove_dir_all(&original_path).ok();
    }

    let activity_path = if sanitized_path.exists() {
        sanitized_path.join("MainActivity.kt")
    } else {
        original_path.join("MainActivity.kt")
    };
    if activity_path.exists() {
        let contents = fs::read_to_string(&activity_path)?;
        let updated = contents.replace(bundle_identifier, &sanitized);
        if updated != contents {
            fs::write(&activity_path, updated)?;
        }
    }

    info!(
        "Adjusted Android bundle identifier from '{}' to '{}'",
        bundle_identifier, sanitized
    );

    Ok(sanitized)
}

pub fn configure_rust_android_linker_env(desired_triples: &[&str]) -> Result<()> {
    let ndk_env = env::var("ANDROID_NDK_HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.exists());
    let ndk_path = ndk_env
        .clone()
        .or_else(resolve_android_ndk_path)
        .ok_or_else(|| eyre!("Android NDK not found. Install it and set ANDROID_NDK_HOME."))?;

    if ndk_env.is_none() {
        // SAFETY: environment mutation happens on the main CLI thread before other threads run.
        unsafe {
            env::set_var("ANDROID_NDK_HOME", &ndk_path);
        }
        info!(
            "ANDROID_NDK_HOME not set; using auto-detected NDK at {}",
            ndk_path.display()
        );
    }

    let toolchain_bin = ndk_toolchain_bin(&ndk_path)
        .ok_or_else(|| eyre!("Unable to locate NDK LLVM toolchain binaries."))?;

    let mut path_entries =
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    if !path_entries.iter().any(|entry| entry == &toolchain_bin) {
        path_entries.insert(0, toolchain_bin.clone());
        let joined = env::join_paths(path_entries)
            .context("failed to join PATH entries for NDK toolchain")?;
        // SAFETY: see comment above for environment mutation safety.
        unsafe {
            env::set_var("PATH", joined);
        }
    }

    let installed_targets = installed_rust_targets();
    let mut configured_triples = Vec::new();
    for &triple in desired_triples {
        let target = target_by_triple(triple)
            .ok_or_else(|| eyre!("Unsupported Android target `{triple}`"))?;
        if let Some(installed) = &installed_targets {
            if !installed.contains(target.triple) {
                bail!(
                    "Rust target `{}` is not installed. Run `rustup target add {}` and retry.",
                    target.triple,
                    target.triple
                );
            }
        }

        configure_target_toolchain_env(target, &toolchain_bin)?;
        configured_triples.push(target.triple.to_string());
    }

    if configured_triples.is_empty() {
        bail!("No Android Rust targets were configured.");
    }

    info!(
        "Configured Android Rust targets: {}",
        configured_triples.join(", ")
    );
    // SAFETY: single-threaded mutation of process environment in CLI.
    unsafe {
        env::set_var("ANDROID_BUILD_TARGETS", configured_triples.join(","));
    }

    Ok(())
}

fn configure_target_toolchain_env(
    target: &AndroidTargetConfig,
    toolchain_bin: &Path,
) -> Result<()> {
    let api_levels = api_level_candidates(target.min_api);
    let clang = find_tool(toolchain_bin, target, &api_levels, "clang").ok_or_else(|| {
        eyre!(
            "Unable to find clang for target {} in the NDK toolchain",
            target.triple
        )
    })?;
    let fallback_pp = clang
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| clang.with_file_name(format!("{name}++")));
    let clang_pp = find_tool(toolchain_bin, target, &api_levels, "clang++")
        .or_else(|| fallback_pp.filter(|candidate| candidate.exists()))
        .ok_or_else(|| {
            eyre!(
                "Unable to find clang++ for target {} in the NDK toolchain",
                target.triple
            )
        })?;

    let ar = toolchain_bin.join("llvm-ar");
    let ranlib = toolchain_bin.join("llvm-ranlib");

    let target_env_prefix = target.triple.replace('-', "_");
    // SAFETY: the CLI configures environment variables on the main thread before any
    // parallel work begins, so mutating the process environment is safe here.
    unsafe {
        env::set_var(format!("CC_{target_env_prefix}"), &clang);
        env::set_var(format!("CXX_{target_env_prefix}"), &clang_pp);
        if ar.exists() {
            env::set_var(format!("AR_{target_env_prefix}"), &ar);
            env::set_var(
                format!("CARGO_TARGET_{}_AR", target_env_prefix.to_uppercase()),
                &ar,
            );
        }
        if ranlib.exists() {
            env::set_var(format!("RANLIB_{target_env_prefix}"), &ranlib);
        }
        env::set_var(
            format!("CARGO_TARGET_{}_LINKER", target_env_prefix.to_uppercase()),
            &clang,
        );
    }

    Ok(())
}

fn find_tool(
    toolchain_bin: &Path,
    target: &AndroidTargetConfig,
    api_levels: &[u32],
    tool: &str,
) -> Option<PathBuf> {
    for level in api_levels {
        let candidate = toolchain_bin.join(format!("{}{}-{tool}", target.bin_prefix, level));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let fallback = toolchain_bin.join(format!("{}-{tool}", target.bin_prefix));
    fallback.exists().then_some(fallback)
}

fn api_level_candidates(min_api: u32) -> Vec<u32> {
    const MAX_API: u32 = 35;
    (min_api..=MAX_API).rev().collect()
}

pub fn build_android_apk(
    project_dir: &Path,
    android_config: &crate::config::Android,
    release: bool,
    skip_native: bool,
    hot_reload_enabled: bool,
    bundle_identifier: &str,
) -> Result<PathBuf> {
    let _sanitized = prepare_android_package(project_dir, bundle_identifier)?;
    let build_rust_script = project_dir.join("build-rust.sh");
    if build_rust_script.exists() {
        if skip_native {
            info!("Skipping Android native build (requested via --skip-native)");
        } else {
            info!("Building Rust library for Android...");
            let mut cmd = Command::new("bash");
            cmd.arg(&build_rust_script);
            if release {
                cmd.arg("release");
            } else {
                cmd.arg("debug");
            }
            util::configure_hot_reload_env(&mut cmd, hot_reload_enabled, None);
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
    util::configure_hot_reload_env(&mut cmd, hot_reload_enabled, None);

    let ipv4_flag = "-Djava.net.preferIPv4Stack=true";
    let gradle_opts = ensure_jvm_flag(env::var("GRADLE_OPTS").ok(), ipv4_flag);
    cmd.env("GRADLE_OPTS", &gradle_opts);
    let java_tool_options = ensure_jvm_flag(env::var("JAVA_TOOL_OPTIONS").ok(), ipv4_flag);
    cmd.env("JAVA_TOOL_OPTIONS", &java_tool_options);

    if let Some(java_home) = prefer_java_home() {
        info!("Using Java from {}", java_home.display());
        cmd.env("JAVA_HOME", &java_home);
        let java_bin = java_home.join("bin");
        if java_bin.exists() {
            let mut path_entries =
                env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
            if !path_entries.iter().any(|entry| entry == &java_bin) {
                path_entries.insert(0, java_bin);
                if let Ok(joined) = env::join_paths(path_entries) {
                    cmd.env("PATH", joined);
                }
            }
        }
    }

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
