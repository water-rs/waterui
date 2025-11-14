use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use color_eyre::{
    Section,
    eyre::{self, Context, Result as EyreResult, bail},
};
use heck::ToUpperCamelCase;
use thiserror::Error;
use which::which;

use crate::{
    backend::Backend,
    doctor::{AnyToolchainIssue, ToolchainIssue},
    impl_display,
    project::{Project, Swift},
    util,
};

#[derive(Clone, Copy, Debug)]
pub struct Apple;

impl_display!(Apple, "apple");

#[derive(Debug, Clone, Error)]
pub enum AppleToolchainIssue {
    #[error("Xcode is not installed.")]
    XcodeNotInstalled,
    #[error("Xcode Command Line Tools are not installed.")]
    CommandLineToolsNotInstalled,
}

impl ToolchainIssue for AppleToolchainIssue {
    fn suggestion(&self) -> String {
        match self {
            Self::XcodeNotInstalled => "Install Xcode from the App Store.".to_string(),
            Self::CommandLineToolsNotInstalled => {
                "Install Xcode Command Line Tools by running `xcode-select --install`.".to_string()
            }
        }
    }
}

impl Backend for Apple {
    type ToolchainIssue = AnyToolchainIssue;

    fn init(&self, _project: &Project, _dev: bool) -> eyre::Result<()> {
        Ok(())
    }

    fn is_existing(&self, project: &Project) -> bool {
        project.root().join("apple").exists()
    }

    fn clean(&self, project: &Project) -> eyre::Result<()> {
        clean_project(project)
    }

    fn check_requirements(&self, _: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        let mut issues = Vec::new();

        if cfg!(target_os = "macos") {
            if which("xcodebuild").is_err() {
                issues.push(AppleToolchainIssue::XcodeNotInstalled);
            }

            if which("xcode-select").is_err() {
                issues.push(AppleToolchainIssue::CommandLineToolsNotInstalled);
            }
        } else {
            issues.push(AppleToolchainIssue::XcodeNotInstalled);
            issues.push(AppleToolchainIssue::CommandLineToolsNotInstalled);
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues
                .into_iter()
                .map(|issue| Box::new(issue) as AnyToolchainIssue)
                .collect())
        }
    }
}

fn clean_project(project: &Project) -> eyre::Result<()> {
    // run command, clean xcode build artifacts
    let ident = project.identifier().to_upper_camel_case();
    let status = Command::new("xcodebuild")
        .arg("-workspace")
        .arg(format!("apple/{ident}.xcworkspace"))
        .arg("-scheme")
        .arg(ident)
        .arg("clean")
        .current_dir(project.root())
        .status()?;

    if !status.success() {
        return Err(
            eyre::eyre!("Failed to clean Xcode project.").with_section(move || status.to_string())
        );
    }

    Ok(())
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
    project_dir.join(".waterui/DerivedData")
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

/// Run `xcodebuild` while streaming progress to a log file.
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
    let log_clone = log_file
        .try_clone()
        .with_context(|| format!("failed to clone handle for {}", log_path.display()))?;
    cmd.stdout(Stdio::from(log_clone));
    cmd.stderr(Stdio::from(log_file));

    let mut child = cmd.spawn().context("failed to invoke xcodebuild")?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        sleep(Duration::from_millis(150));
    };

    if status.success() {
        Ok(log_path)
    } else {
        let mut err = eyre::eyre!(format!(
            "xcodebuild failed with status {}. See log at {}",
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
