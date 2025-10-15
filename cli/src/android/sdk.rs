use std::{
    env,
    fs::{self, File},
    io::{Write, copy},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use color_eyre::eyre::{Context, Result, bail, eyre};
use zip::ZipArchive;

const CMDLINE_TOOLS_VERSION: &str = "11076708";
const REQUIRED_SDK_PACKAGES: &[&str] = &[
    "platform-tools",
    "platforms;android-34",
    "build-tools;34.0.0",
    "emulator",
    "ndk;26.2.11394342",
];

#[derive(Clone, Debug)]
pub struct AndroidToolchainPaths {
    pub sdk_root: PathBuf,
    pub ndk_root: PathBuf,
}

pub fn detect_android_toolchain() -> Option<AndroidToolchainPaths> {
    let mut candidates = Vec::new();
    if let Ok(path) = env::var("ANDROID_SDK_ROOT") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(path) = env::var("ANDROID_HOME") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend(super::android_sdk_roots());

    for sdk_root in candidates {
        if !sdk_root.exists() {
            continue;
        }
        if let Ok(ndk_root) = find_installed_ndk(&sdk_root) {
            return Some(AndroidToolchainPaths { sdk_root, ndk_root });
        }
    }

    None
}

pub fn install_android_toolchain() -> Result<AndroidToolchainPaths> {
    println!("Preparing Android SDK installation…");

    let home_dir = home::home_dir().ok_or_else(|| eyre!("Unable to determine home directory"))?;
    let android_dir = home_dir.join(".waterui").join("android");
    fs::create_dir_all(&android_dir).context("failed to create WaterUI Android directory")?;

    let sdk_root = android_dir.join("sdk");
    fs::create_dir_all(&sdk_root).context("failed to create Android SDK directory")?;

    let sdkmanager_path = ensure_command_line_tools(&sdk_root)?;
    install_sdk_components(&sdkmanager_path, &sdk_root)?;
    accept_sdk_licenses(&sdkmanager_path, &sdk_root)?;

    let ndk_root = find_installed_ndk(&sdk_root)?;

    println!("Android SDK installed at {}", sdk_root.display());
    println!("Android NDK installed at {}", ndk_root.display());
    println!("Update your shell environment variables to complete setup:");
    println!("  export ANDROID_HOME=\"{}\"", sdk_root.display());
    println!("  export ANDROID_SDK_ROOT=\"{}\"", sdk_root.display());
    println!("  export ANDROID_NDK_HOME=\"{}\"", ndk_root.display());

    Ok(AndroidToolchainPaths { sdk_root, ndk_root })
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

    let response = reqwest::blocking::get(&url)
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
        let mut reader = response;
        copy(&mut reader, &mut file)
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

        let output_path = latest_dir.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&output_path).with_context(|| {
                format!(
                    "failed to create directory from archive entry {}",
                    output_path.display()
                )
            })?;
        } else {
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create parent directory {}", parent.display())
                })?;
            }
            let mut file = File::create(&output_path)
                .with_context(|| format!("failed to extract file {}", output_path.display()))?;
            copy(&mut entry, &mut file).with_context(|| {
                format!("failed to copy archive entry to {}", output_path.display())
            })?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let perms = fs::Permissions::from_mode(mode);
                fs::set_permissions(&output_path, perms).with_context(|| {
                    format!("failed to set permissions on {}", output_path.display())
                })?;
            }
        }
    }

    fs::remove_file(&archive_path).context("failed to clean up downloaded archive")?;
    Ok(())
}

fn install_sdk_components(sdkmanager_path: &Path, sdk_root: &Path) -> Result<()> {
    println!("Installing Android SDK components via sdkmanager…");
    let mut command = Command::new(sdkmanager_path);
    command.arg("--sdk_root").arg(sdk_root);
    command.arg("--install");
    command.args(REQUIRED_SDK_PACKAGES);
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());

    let status = command
        .status()
        .context("failed to execute sdkmanager for component installation")?;
    if !status.success() {
        bail!("sdkmanager failed while installing Android components");
    }

    Ok(())
}

fn accept_sdk_licenses(sdkmanager_path: &Path, sdk_root: &Path) -> Result<()> {
    println!("Accepting Android SDK licenses…");
    let mut child = Command::new(sdkmanager_path)
        .arg("--sdk_root")
        .arg(sdk_root)
        .arg("--licenses")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn sdkmanager for license acceptance")?;

    if let Some(mut stdin) = child.stdin.take() {
        thread::spawn(move || {
            for _ in 0..32 {
                if stdin.write_all(b"y\n").is_err() {
                    break;
                }
            }
        });
    }

    let status = child
        .wait()
        .context("failed to wait for sdkmanager license acceptance")?;
    if !status.success() {
        bail!("sdkmanager failed while accepting Android licenses");
    }

    Ok(())
}

fn find_installed_ndk(sdk_root: &Path) -> Result<PathBuf> {
    if let Ok(ndk_home) = env::var("ANDROID_NDK_HOME") {
        let path = PathBuf::from(ndk_home);
        if path.exists() {
            return Ok(path);
        }
    }

    let ndk_dir = sdk_root.join("ndk");
    if !ndk_dir.exists() {
        bail!(
            "Android NDK directory {} not found after installation",
            ndk_dir.display()
        );
    }

    let mut best: Option<(String, PathBuf)> = None;
    for entry in fs::read_dir(&ndk_dir).context("failed to read Android NDK directory")? {
        let entry = entry.context("failed to read Android NDK entry")?;
        if !entry
            .file_type()
            .context("failed to inspect NDK entry type")?
            .is_dir()
        {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if best
            .as_ref()
            .is_none_or(|(best_name, _)| file_name > *best_name)
        {
            best = Some((file_name, path));
        }
    }

    let (_, ndk_path) =
        best.ok_or_else(|| eyre!("No Android NDK versions found under {}", ndk_dir.display()))?;
    Ok(ndk_path)
}

fn sdkmanager_executable(latest_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        latest_dir.join("bin").join("sdkmanager.bat")
    } else {
        latest_dir.join("bin").join("sdkmanager")
    }
}

fn command_line_tools_url() -> Result<String> {
    let os = if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "windows") {
        "win"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        bail!("Unsupported operating system for Android command-line tools download");
    };

    Ok(format!(
        "https://dl.google.com/android/repository/commandlinetools-{os}-{CMDLINE_TOOLS_VERSION}_latest.zip"
    ))
}
