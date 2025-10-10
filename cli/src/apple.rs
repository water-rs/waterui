use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Result, bail};

use crate::{config::Swift, util};

pub struct XcodeProject<'a> {
    pub scheme: &'a str,
    pub project_file: PathBuf,
}

pub fn ensure_macos_host(feature: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        bail!("{feature} requires macOS");
    }
}

pub fn require_tool(tool: &str, hint: &str) -> Result<()> {
    if which::which(tool).is_ok() {
        Ok(())
    } else {
        bail!("{tool} not found. {hint}")
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
        .arg(derived_root);
    cmd
}

pub fn disable_code_signing(cmd: &mut Command) {
    cmd.arg("CODE_SIGNING_ALLOWED=NO")
        .arg("CODE_SIGNING_REQUIRED=NO")
        .arg("CODE_SIGN_IDENTITY=-");
}
