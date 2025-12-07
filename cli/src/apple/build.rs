use std::{
    collections::VecDeque,
    env,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use color_eyre::{
    Section,
    eyre::{self, Context, Result as EyreResult, bail},
};
use heck::ToUpperCamelCase;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::info;
use which::which;

use crate::{
    backend::Backend,
    device::{DeviceInfo, DeviceKind},
    impl_display,
    project::Project,
    toolchain::ToolchainError,
    util,
};

impl Backend for Apple {
    fn init(&self, _project: &Project, _dev: bool) -> eyre::Result<()> {
        Ok(())
    }

    fn is_existing(&self, project: &Project) -> bool {
        project.root().join("apple").exists()
    }

    fn clean(&self, project: &Project) -> eyre::Result<()> {
        clean_project(project)
    }

    fn check_requirements(&self, _: &Project) -> Result<(), Vec<ToolchainError>> {
        let mut issues = Vec::new();

        if cfg!(target_os = "macos") {
            if which("xcodebuild").is_err() {
                issues.push(
                    ToolchainError::unfixable("Xcode is not installed")
                        .with_suggestion("Install Xcode from the Mac App Store"),
                );
            }

            if which("xcode-select").is_err() {
                issues.push(
                    ToolchainError::unfixable("Xcode Command Line Tools are not installed")
                        .with_suggestion("Run: xcode-select --install"),
                );
            }
        } else {
            issues.push(
                ToolchainError::unfixable("Apple development requires macOS")
                    .with_suggestion("Use a Mac for iOS/macOS development"),
            );
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    async fn scan_devices(&self, _cancel: CancellationToken) -> eyre::Result<Vec<DeviceInfo>> {
        scan_apple_devices().await
    }
}

fn apple_platform_friendly_name(identifier: &str) -> String {
    match identifier {
        "com.apple.platform.iphoneos" => "iOS device",
        "com.apple.platform.ipados" => "iPadOS device",
        "com.apple.platform.watchos" => "watchOS device",
        "com.apple.platform.appletvos" => "tvOS device",
        "com.apple.platform.iphonesimulator" => "iOS simulator",
        "com.apple.platform.appletvsimulator" => "tvOS simulator",
        "com.apple.platform.watchsimulator" => "watchOS simulator",
        "com.apple.platform.visionos" => "visionOS device",
        "com.apple.platform.visionossimulator" => "visionOS simulator",
        "com.apple.platform.macosx" => "macOS",
        other => other,
    }
    .to_string()
}

#[derive(Debug)]
pub struct XcodeProject<'a> {
    pub scheme: &'a str,
    pub project_file: PathBuf,
}

/// Ensure the current host is macOS before running a feature.
///
/// # Errors
/// Returns an error when invoked on non-macOS hosts.
pub fn ensure_macos_host(feature: &str) -> EyreResult<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        bail!("{feature} requires macOS")
    }
}

/// Locate the Xcode project described by the Swift configuration.
///
/// # Errors
/// Returns an error if the expected project directory or file is missing.
pub fn resolve_xcode_project<'a>(
    project_dir: &Path,
    swift_config: &'a Swift,
) -> EyreResult<XcodeProject<'a>> {
    let project_root = project_dir.join(&swift_config.project_path);
    if !project_root.exists() {
        bail!(
            "Xcode project directory not found at {}. Did you run 'water create'?",
            project_root.display()
        );
    }

    let project_file = swift_config.project_file.as_ref().map_or_else(
        || project_root.join(format!("{}.xcodeproj", swift_config.scheme)),
        |custom| project_root.join(custom),
    );

    if !project_file.exists() {
        bail!("Missing Xcode project: {}", project_file.display());
    }

    Ok(XcodeProject {
        scheme: &swift_config.scheme,
        project_file,
    })
}

#[must_use]
pub fn derived_data_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".water/DerivedData")
}

/// Ensure the derived data directory exists for Xcode builds.
///
/// # Errors
/// Returns an error if the directory cannot be created.
pub fn prepare_derived_data_dir(dir: &Path) -> EyreResult<()> {
    util::ensure_directory(dir)
}

#[must_use]
pub fn xcodebuild_base(
    project: &XcodeProject<'_>,
    configuration: &str,
    derived_root: &Path,
) -> Command {
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("-project")
        .arg(&project.project_file)
        .arg("-scheme")
        .arg(project.scheme)
        .arg("-configuration")
        .arg(configuration)
        .arg("-derivedDataPath")
        .arg(derived_root)
        .arg("-allowProvisioningUpdates")
        .arg("-allowProvisioningDeviceRegistration");
    cmd
}

pub fn disable_code_signing(cmd: &mut Command) {
    cmd.arg("CODE_SIGNING_ALLOWED=NO")
        .arg("CODE_SIGNING_REQUIRED=NO")
        .arg("CODE_SIGN_IDENTITY=-");
}

#[derive(Debug)]
pub struct AppleRustBuildOptions<'a> {
    pub project_dir: &'a Path,
    pub crate_name: &'a str,
    pub release: bool,
    pub platform_name: Option<String>,
    pub arch: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub enable_sccache: bool,
    pub enable_mold: bool,
}

#[derive(Debug)]
pub struct AppleRustBuildReport {
    pub target: String,
    pub profile: String,
    pub output_library: PathBuf,
}

pub fn build_apple_static_library(
    opts: AppleRustBuildOptions<'_>,
) -> EyreResult<AppleRustBuildReport> {
    ensure_cmake_available_for_apple()?;

    let profile = if opts.release { "release" } else { "debug" };
    let env_platform = opts
        .platform_name
        .clone()
        .or_else(|| env::var("PLATFORM_NAME").ok());
    let env_arch = opts.arch.clone().or_else(|| env::var("ARCHS").ok());
    let arch_token = env_arch
        .as_deref()
        .and_then(|value| value.split_whitespace().next())
        .map_or_else(
            || host_default_arch().to_string(),
            std::string::ToString::to_string,
        );
    let platform_token = env_platform.as_deref().map(str::to_ascii_lowercase);
    let platform_ref = platform_token.as_deref();
    let target = resolve_apple_rust_target(platform_ref, arch_token.as_str());
    info!(
        "Building Rust crate '{}' for Apple target {} ({profile})",
        opts.crate_name, target
    );

    let make_command = || {
        let mut cmd = Command::new("cargo");
        cmd.arg("build")
            .arg("--package")
            .arg(opts.crate_name)
            .arg("--target")
            .arg(target);
        if opts.release {
            cmd.arg("--release");
        }
        cmd.current_dir(opts.project_dir);
        cmd
    };

    let mut cmd = make_command();
    let sccache_enabled =
        util::configure_build_speedups(&mut cmd, opts.enable_sccache, opts.enable_mold);
    let status = cmd.status().with_context(|| {
        format!(
            "failed to compile Rust crate {} for target {}",
            opts.crate_name, target
        )
    })?;
    if !status.success() {
        if sccache_enabled {
            let mut retry_cmd = make_command();
            util::configure_build_speedups(&mut retry_cmd, false, opts.enable_mold);
            let retry_status = retry_cmd.status().with_context(|| {
                format!("failed to re-run cargo build for target {target} without sccache")
            })?;
            if !retry_status.success() {
                bail!(
                    "cargo build failed for target {} (retry without sccache also failed)",
                    target
                );
            }
        } else {
            bail!(
                "cargo build failed for target {} with status {}",
                target,
                status
            );
        }
    }

    let lib_name = opts.crate_name.replace('-', "_");
    let rust_lib = opts
        .project_dir
        .join("target")
        .join(target)
        .join(profile)
        .join(format!("lib{lib_name}.a"));
    if !rust_lib.exists() {
        bail!(
            "Rust static library not found at {}. Did the build succeed?",
            rust_lib.display()
        );
    }

    let output_dir = if let Some(custom) = &opts.output_dir {
        custom.clone()
    } else if let Ok(env_dir) = env::var("BUILT_PRODUCTS_DIR") {
        PathBuf::from(env_dir)
    } else {
        opts.project_dir.join("apple/build")
    };
    util::ensure_directory(&output_dir)?;

    // Copy with standardized name (libwaterui_app.a) so the Xcode project
    // can always link against "waterui_app" regardless of the actual crate name
    let output_library = output_dir.join("libwaterui_app.a");
    fs::copy(&rust_lib, &output_library).with_context(|| {
        format!(
            "failed to copy {} to {}",
            rust_lib.display(),
            output_library.display()
        )
    })?;

    let xcconfig = opts.project_dir.join("apple/rust_build_info.xcconfig");
    let xcconfig_contents = format!("RUST_LIBRARY_PATH={}\n", rust_lib.display());
    if let Some(parent) = xcconfig.parent() {
        util::ensure_directory(parent)?;
    }
    fs::write(&xcconfig, xcconfig_contents)?;

    Ok(AppleRustBuildReport {
        target: target.to_string(),
        profile: profile.to_string(),
        output_library,
    })
}

fn resolve_apple_rust_target(platform: Option<&str>, arch: &str) -> &'static str {
    match platform {
        Some("iphonesimulator") => {
            if arch == "x86_64" {
                "x86_64-apple-ios"
            } else {
                "aarch64-apple-ios-sim"
            }
        }
        Some("iphoneos") => "aarch64-apple-ios",
        Some("macosx") => {
            if arch == "x86_64" {
                "x86_64-apple-darwin"
            } else {
                "aarch64-apple-darwin"
            }
        }
        Some("xrsimulator") => "aarch64-apple-ios-sim",
        Some("xros") => "aarch64-apple-ios",
        Some("watchsimulator") => "aarch64-apple-watchos-sim",
        Some("watchos") => "aarch64-apple-watchos",
        Some("appletvsimulator") => "aarch64-apple-tvos",
        Some("appletvos") => "aarch64-apple-tvos",
        _ => default_host_target(),
    }
}

const fn default_host_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "aarch64-apple-darwin"
    }
}

const fn host_default_arch() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "arm64"
    }
}

/// Run `xcodebuild` while streaming progress to both the terminal and a log file.
///
/// Output is displayed in real-time so users can see build progress and errors.
///
/// # Errors
/// Returns an error if the process fails or if the log cannot be written.
pub fn run_xcodebuild_with_progress(
    mut cmd: Command,
    description: &str,
    log_dir: &Path,
) -> EyreResult<PathBuf> {
    util::ensure_directory(log_dir)?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let log_path = log_dir.join(format!("xcodebuild-{timestamp}.log"));

    let log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .with_context(|| format!("failed to create {}", log_path.display()))?;

    // Pipe stdout/stderr so we can stream to both terminal and log file
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().context("failed to invoke xcodebuild")?;

    // Stream stdout to both terminal and log file
    let stdout = child.stdout.take();
    let log_file_stdout = log_file
        .try_clone()
        .with_context(|| format!("failed to clone handle for {}", log_path.display()))?;
    let stdout_handle = thread::spawn(move || {
        if let Some(stdout) = stdout {
            stream_output(stdout, std::io::stdout(), log_file_stdout);
        }
    });

    // Stream stderr to both terminal and log file
    let stderr = child.stderr.take();
    let log_file_stderr = log_file;
    let stderr_handle = thread::spawn(move || {
        if let Some(stderr) = stderr {
            stream_output(stderr, std::io::stderr(), log_file_stderr);
        }
    });

    // Wait for output threads to complete
    let _ = stdout_handle.join();
    let _ = stderr_handle.join();

    // Wait for process to complete
    let status = child.wait().context("failed to wait for xcodebuild")?;

    if status.success() {
        Ok(log_path)
    } else {
        let mut err = eyre::eyre!(format!(
            "xcodebuild failed with status {}. See full log at {}",
            status,
            log_path.display()
        ));
        if let Ok(lines) = last_lines(&log_path, 80) {
            if !lines.is_empty() {
                let snippet = lines.join("\n");
                err = err.with_section(move || {
                    format!("{description} (last {} lines)\n{snippet}", lines.len())
                });
            }
        }
        Err(err)
    }
}

/// Stream output from a reader to both a terminal writer and a log file.
fn stream_output<R, W>(reader: R, mut terminal: W, mut log_file: File)
where
    R: std::io::Read,
    W: Write,
{
    let buf_reader = BufReader::new(reader);
    for line in buf_reader.lines() {
        match line {
            Ok(line) => {
                // Write to terminal
                let _ = writeln!(terminal, "{line}");
                // Write to log file
                let _ = writeln!(log_file, "{line}");
            }
            Err(_) => break,
        }
    }
}

fn last_lines(path: &Path, max_lines: usize) -> EyreResult<Vec<String>> {
    let file =
        File::open(path).with_context(|| format!("failed to open log file {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut buffer = VecDeque::with_capacity(max_lines);
    for line in reader.lines() {
        let line = line?;
        if buffer.len() == max_lines {
            buffer.pop_front();
        }
        buffer.push_back(line);
    }
    Ok(buffer.into_iter().collect())
}
