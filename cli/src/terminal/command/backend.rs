use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Args, Subcommand};
use color_eyre::eyre::{Context, Result, bail, eyre};
use semver::Version;
use serde::Serialize;
use tracing::warn;

use super::{
    add_backend,
    create::{
        BackendChoice, SWIFT_BACKEND_GIT_URL, SWIFT_TAG_PREFIX,
        android::{
            self, configure_android_local_properties, copy_android_backend,
            ensure_android_backend_release, ensure_dev_android_backend_checkout,
        },
    },
};
use crate::ui;
use waterui_cli::project::Config;

#[derive(Subcommand, Debug)]
pub enum BackendCommands {
    Add(add_backend::AddBackendArgs),
    Update(BackendUpdateArgs),
}

#[derive(Args, Debug, Clone)]
pub struct BackendUpdateArgs {
    #[arg(value_enum)]
    pub backend: BackendChoice,
    #[arg(long)]
    pub project: Option<PathBuf>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendUpdateStatus {
    Updated,
    UpToDate,
    Incompatible,
}

#[derive(Debug, Serialize)]
pub struct BackendUpdateReport {
    pub backend: String,
    pub status: BackendUpdateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl BackendUpdateReport {
    fn new(choice: BackendChoice, status: BackendUpdateStatus) -> Self {
        Self {
            backend: choice.label().to_string(),
            status,
            from_version: None,
            to_version: None,
            message: None,
        }
    }
}

pub fn update(args: BackendUpdateArgs) -> Result<BackendUpdateReport> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let mut config = Config::load(&project_dir)?;

    match args.backend {
        BackendChoice::Android => update_android_backend(&project_dir, &mut config),
        BackendChoice::Swiftui => update_swift_backend(&project_dir, &mut config),
        BackendChoice::Web => {
            bail!(
                "Web backend update is not supported yet. Please update the CLI when a new template is available."
            )
        }
    }
}

fn update_android_backend(project_dir: &Path, config: &mut Config) -> Result<BackendUpdateReport> {
    let android_cfg = config
        .backends
        .android
        .as_mut()
        .ok_or_else(|| eyre!("Android backend is not configured for this project"))?;

    let mut report =
        BackendUpdateReport::new(BackendChoice::Android, BackendUpdateStatus::UpToDate);
    let use_dev_backend = android_cfg.dev || config.dev_dependencies;

    if use_dev_backend {
        ui::warning("Updating Android dev backend. This may include breaking changes.");
        let source = ensure_dev_android_backend_checkout()?;
        copy_android_backend(project_dir, &source)?;
        configure_android_local_properties(project_dir)?;
        android_cfg.dev = true;
        android_cfg.version = None;
        config.save(project_dir)?;
        report.status = BackendUpdateStatus::Updated;
        report.message =
            Some("Synchronized dev backend from the latest upstream commit.".to_string());
        return Ok(report);
    }

    let current_version = android_cfg
        .version
        .clone()
        .ok_or_else(|| eyre!("Android backend version is unknown. Re-create or add the backend again to refresh metadata."))?;
    let current_semver = Version::parse(&current_version)
        .with_context(|| format!("failed to parse Android backend version {current_version}"))?;

    let available = fetch_repo_versions(
        android::ANDROID_BACKEND_REPO,
        android::ANDROID_BACKEND_TAG_PREFIX,
    )?;
    let (candidate, incompatible) = select_compatible_version(&current_semver, &available);

    if let Some(new_version) = candidate {
        ui::step(format!(
            "Updating Android backend from v{} to v{}",
            current_semver, new_version
        ));
        let source = ensure_android_backend_release(&new_version.to_string())?;
        copy_android_backend(project_dir, &source)?;
        configure_android_local_properties(project_dir)?;
        android_cfg.version = Some(new_version.to_string());
        android_cfg.dev = false;
        config.save(project_dir)?;
        report.status = BackendUpdateStatus::Updated;
        report.from_version = Some(current_version);
        report.to_version = Some(new_version.to_string());
        ui::success("Android backend updated");
        return Ok(report);
    }

    if let Some(newer) = incompatible {
        let message = format!(
            "A newer, potentially breaking Android backend (v{}) is available. Update the CLI to evaluate migrating.",
            newer
        );
        warn!("{message}");
        report.status = BackendUpdateStatus::Incompatible;
        report.message = Some(message);
    } else {
        ui::info("Android backend is already up to date");
    }

    Ok(report)
}

fn update_swift_backend(project_dir: &Path, config: &mut Config) -> Result<BackendUpdateReport> {
    let swift_cfg = config
        .backends
        .swift
        .as_mut()
        .ok_or_else(|| eyre!("SwiftUI backend is not configured for this project"))?;

    let mut report =
        BackendUpdateReport::new(BackendChoice::Swiftui, BackendUpdateStatus::UpToDate);
    let project_path = project_dir.join(&swift_cfg.project_path);
    let project_file = swift_cfg
        .project_file
        .as_deref()
        .ok_or_else(|| eyre!("Swift project file is not recorded in Water.toml"))?;
    let pbxproj = project_path.join(project_file).join("project.pbxproj");

    let use_dev_backend = swift_cfg.dev || config.dev_dependencies;

    if use_dev_backend {
        ui::warning("Updating Swift dev backend. This may include breaking API changes.");
        rewrite_swift_requirement(
            &pbxproj,
            SwiftRequirement::Branch(
                swift_cfg
                    .branch
                    .clone()
                    .unwrap_or_else(|| "dev".to_string()),
            ),
        )?;
        config.save(project_dir)?;
        report.status = BackendUpdateStatus::Updated;
        report.message = Some("Swift package requirement reset to the dev branch. Re-open Xcode to fetch the latest commit.".to_string());
        return Ok(report);
    }

    let current_version = swift_cfg.version.clone().ok_or_else(|| {
        eyre!("Swift backend version is unknown. Re-create the backend to refresh metadata.")
    })?;
    let current_semver = Version::parse(&current_version)
        .with_context(|| format!("failed to parse Swift backend version {current_version}"))?;

    let available = fetch_repo_versions(SWIFT_BACKEND_GIT_URL, SWIFT_TAG_PREFIX)?;
    let (candidate, incompatible) = select_compatible_version(&current_semver, &available);

    if let Some(new_version) = candidate {
        ui::step(format!(
            "Updating Swift backend from v{} to v{}",
            current_semver, new_version
        ));
        rewrite_swift_requirement(&pbxproj, SwiftRequirement::Release(new_version.to_string()))?;
        swift_cfg.version = Some(new_version.to_string());
        swift_cfg.dev = false;
        swift_cfg.branch = None;
        config.save(project_dir)?;
        report.status = BackendUpdateStatus::Updated;
        report.from_version = Some(current_version);
        report.to_version = Some(new_version.to_string());
        ui::success(
            "Swift backend updated. Open Xcode and resolve package dependencies to download the new version.",
        );
        return Ok(report);
    }

    if let Some(newer) = incompatible {
        let message = format!(
            "A newer Swift backend major version (v{}) is available but may be incompatible. Update the CLI or review release notes before upgrading.",
            newer
        );
        warn!("{message}");
        report.status = BackendUpdateStatus::Incompatible;
        report.message = Some(message);
    } else {
        ui::info("Swift backend is already up to date");
    }

    Ok(report)
}

fn fetch_repo_versions(repo_url: &str, prefix: &str) -> Result<Vec<Version>> {
    let output = Command::new("git")
        .args(["ls-remote", "--tags", repo_url])
        .output()
        .with_context(|| format!("failed to query tags from {repo_url}"))?;
    if !output.status.success() {
        bail!(
            "git ls-remote for {repo_url} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut seen = HashSet::new();
    let mut versions = Vec::new();

    for line in stdout.lines() {
        let mut parts = line.split('\t');
        let _sha = parts.next();
        let reference = match parts.next() {
            Some(reference) => reference,
            None => continue,
        };
        if !reference.starts_with("refs/tags/") {
            continue;
        }
        let mut tag_name = &reference["refs/tags/".len()..];
        if tag_name.ends_with("^{}") {
            continue;
        }
        if !tag_name.starts_with(prefix) {
            continue;
        }
        tag_name = &tag_name[prefix.len()..];
        if !seen.insert(tag_name.to_string()) {
            continue;
        }
        if let Ok(version) = Version::parse(tag_name) {
            versions.push(version);
        }
    }

    versions.sort();
    Ok(versions)
}

fn select_compatible_version(
    current: &Version,
    available: &[Version],
) -> (Option<Version>, Option<Version>) {
    let mut compatible = None;
    let mut incompatible = None;

    for candidate in available {
        if candidate <= current {
            continue;
        }
        if candidate.major == current.major {
            if compatible
                .as_ref()
                .map_or(true, |best: &Version| candidate > best)
            {
                compatible = Some(candidate.clone());
            }
        } else if incompatible
            .as_ref()
            .map_or(true, |best: &Version| candidate > best)
        {
            incompatible = Some(candidate.clone());
        }
    }

    (compatible, incompatible)
}

enum SwiftRequirement {
    Branch(String),
    Release(String),
}

fn rewrite_swift_requirement(pbxproj: &Path, requirement: SwiftRequirement) -> Result<()> {
    let contents = fs::read_to_string(pbxproj)
        .with_context(|| format!("failed to read {}", pbxproj.display()))?;
    let repo_marker = "repositoryURL";
    let repo_idx = contents
        .find(repo_marker)
        .ok_or_else(|| eyre!("Swift package reference not found in {}", pbxproj.display()))?;
    let req_relative = contents[repo_idx..]
        .find("requirement = {")
        .ok_or_else(|| {
            eyre!(
                "Swift package requirement block not found in {}",
                pbxproj.display()
            )
        })?;
    let req_idx = repo_idx + req_relative;
    let block_relative = contents[req_idx..]
        .find("};")
        .ok_or_else(|| eyre!("Malformed Swift package requirement block"))?;
    let block_end = req_idx + block_relative + 2;
    let indent_start = contents[..req_idx]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let indent = &contents[indent_start..req_idx];
    let replacement = match requirement {
        SwiftRequirement::Branch(branch) => format!(
            "{indent}requirement = {{\n{indent}\tkind = branch;\n{indent}\tbranch = \"{branch}\";\n{indent}}};\n"
        ),
        SwiftRequirement::Release(version) => format!(
            "{indent}requirement = {{\n{indent}\tkind = upToNextMajorVersion;\n{indent}\tminimumVersion = \"{version}\";\n{indent}}};\n"
        ),
    };

    let mut updated = String::with_capacity(contents.len());
    updated.push_str(&contents[..req_idx]);
    updated.push_str(&replacement);
    updated.push_str(&contents[block_end..]);
    fs::write(pbxproj, updated)
        .with_context(|| format!("failed to update {}", pbxproj.display()))?;
    Ok(())
}
