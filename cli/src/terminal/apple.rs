use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};

use crate::{config::Swift, output, util};

pub struct XcodeProject<'a> {
    pub scheme: &'a str,
    pub project_file: PathBuf,
}

pub fn ensure_macos_host(feature: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        bail!("{feature} requires macOS")
    }
}

pub fn resolve_xcode_project<'a>(
    project_dir: &Path,
    swift_config: &'a Swift,
) -> Result<XcodeProject<'a>> {
    let project_root = project_dir.join(&swift_config.project_path);
    if !project_root.exists() {
        bail!(
            "Xcode project directory not found at {}. Did you run 'water create'?",
            project_root.display()
        );
    }

    let project_file = if let Some(custom) = &swift_config.project_file {
        project_root.join(custom)
    } else {
        project_root.join(format!("{}.xcodeproj", swift_config.scheme))
    };

    if !project_file.exists() {
        bail!("Missing Xcode project: {}", project_file.display());
    }

    Ok(XcodeProject {
        scheme: &swift_config.scheme,
        project_file,
    })
}

pub fn derived_data_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".waterui/DerivedData")
}

pub fn prepare_derived_data_dir(dir: &Path) -> Result<()> {
    util::ensure_directory(dir)
}

pub fn xcodebuild_base<'a>(
    project: &XcodeProject<'a>,
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

pub fn run_xcodebuild_with_progress(
    mut cmd: Command,
    description: &str,
    log_dir: &Path,
) -> Result<PathBuf> {
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

    let spinner = if output::global_output_format().is_json() {
        None
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner} {msg}")
                .expect("spinner template should be valid")
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.set_message(description.to_string());
        pb.enable_steady_tick(Duration::from_millis(120));
        Some(pb)
    };

    let mut child = cmd.spawn().context("failed to invoke xcodebuild")?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        sleep(Duration::from_millis(150));
    };

    if let Some(pb) = spinner {
        if status.success() {
            pb.finish_with_message(format!("{description} complete"));
        } else {
            pb.finish_and_clear();
        }
    }

    if status.success() {
        Ok(log_path)
    } else {
        if !output::global_output_format().is_json() {
            if let Ok(lines) = last_lines(&log_path, 80) {
                if !lines.is_empty() {
                    eprintln!(
                        "xcodebuild failed — showing last {} lines from {}:",
                        lines.len(),
                        log_path.display()
                    );
                    for line in lines {
                        eprintln!("{line}");
                    }
                }
            }
        }
        bail!(
            "xcodebuild failed with status {}. See log at {}",
            status,
            log_path.display()
        );
    }
}

fn last_lines(path: &Path, max_lines: usize) -> Result<Vec<String>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to open log file {}", path.display()))?;
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
