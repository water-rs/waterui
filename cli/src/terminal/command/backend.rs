#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Args, Subcommand};
use color_eyre::eyre::{Context, Result, bail, eyre};
use dialoguer::{Confirm, theme::ColorfulTheme};
use semver::Version;
use serde::Serialize;
use tracing::warn;

use super::{
    add_backend,
    create::{
        self, BackendChoice, SWIFT_BACKEND_GIT_URL, SWIFT_TAG_PREFIX,
        android::{
            self, clear_android_dev_commit, configure_android_local_properties,
            copy_android_backend, ensure_android_backend_release,
            ensure_dev_android_backend_checkout, git_head_commit, read_android_dev_commit,
            write_android_dev_commit,
        },
    },
};
use crate::{ui, util};
use toml::{Value, value::Table};
use waterui_cli::{WATERUI_VERSION, output, project::Config};

#[derive(Subcommand, Debug)]
pub enum BackendCommands {
    Add(add_backend::AddBackendArgs),
    Update(BackendUpdateArgs),
    Upgrade(BackendUpdateArgs),
    List(BackendListArgs),
}

#[derive(Args, Debug, Clone)]
pub struct BackendUpdateArgs {
    #[arg(value_enum)]
    pub backend: BackendChoice,
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// Automatically confirm prompts (useful for JSON/non-interactive modes)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct BackendListArgs {
    /// Project directory (defaults to current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendUpdateStatus {
    Updated,
    UpToDate,
    Incompatible,
    Skipped,
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

#[derive(Debug, Serialize)]
pub struct BackendListEntry {
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub dev: bool,
    pub targets: Vec<String>,
    pub project_path: String,
}

#[derive(Debug, Serialize)]
pub struct BackendListReport {
    pub entries: Vec<BackendListEntry>,
}

pub fn update(args: BackendUpdateArgs) -> Result<BackendUpdateReport> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;

    // Block backend update for local dev mode
    if config.waterui_path.is_some() {
        bail!(
            "Backend update is not available for projects using local dev mode.\n\n\
             Your project is configured with a local WaterUI repository path at:\n  {}\n\n\
             In local dev mode, backends are referenced directly from the WaterUI repository.\n\
             To update backends, simply pull the latest changes in your WaterUI repository:\n\n\
             cd {}\n\
             git pull\n\
             git submodule update --recursive",
            config.waterui_path.as_ref().unwrap(),
            config.waterui_path.as_ref().unwrap()
        );
    }

    let mut config = config;
    match args.backend {
        BackendChoice::Android => update_android_backend(&project_dir, &mut config),
        BackendChoice::Apple => update_swift_backend(&project_dir, &mut config),
        BackendChoice::Web => {
            bail!(
                "Web backend update is not supported yet. Please update the CLI when a new template is available."
            )
        }
    }
}

pub fn upgrade(args: BackendUpdateArgs) -> Result<BackendUpdateReport> {
    let project_dir = args
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;

    // Block backend upgrade for local dev mode
    if config.waterui_path.is_some() {
        bail!(
            "Backend upgrade is not available for projects using local dev mode.\n\n\
             Your project is configured with a local WaterUI repository path at:\n  {}\n\n\
             In local dev mode, backends are referenced directly from the WaterUI repository.\n\
             To update backends, simply pull the latest changes in your WaterUI repository:\n\n\
             cd {}\n\
             git pull\n\
             git submodule update --recursive",
            config.waterui_path.as_ref().unwrap(),
            config.waterui_path.as_ref().unwrap()
        );
    }

    let mut config = config;
    match args.backend {
        BackendChoice::Android => upgrade_android_backend(&project_dir, &mut config, &args),
        BackendChoice::Apple => upgrade_swift_backend(&project_dir, &mut config, &args),
        BackendChoice::Web => bail!("Web backend upgrade is not supported yet."),
    }
}

pub fn list(args: BackendListArgs) -> Result<BackendListReport> {
    let project_dir = args
        .project
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));
    let config = Config::load(&project_dir)?;
    let mut entries = Vec::new();

    if let Some(swift) = config.backends.swift.as_ref() {
        entries.push(BackendListEntry {
            backend: BackendChoice::Apple.label().to_string(),
            version: if swift.dev {
                swift
                    .revision
                    .clone()
                    .or_else(|| swift.branch.clone())
                    .or_else(|| Some("dev".to_string()))
            } else {
                swift.version.clone()
            },
            dev: swift.dev || config.dev_dependencies,
            targets: backend_targets(BackendChoice::Apple)
                .iter()
                .map(|t| (*t).to_string())
                .collect(),
            project_path: swift.project_path.clone(),
        });
    }

    if let Some(android) = config.backends.android.as_ref() {
        entries.push(BackendListEntry {
            backend: BackendChoice::Android.label().to_string(),
            version: if android.dev {
                Some("dev".to_string())
            } else {
                android.version.clone()
            },
            dev: android.dev || config.dev_dependencies,
            targets: backend_targets(BackendChoice::Android)
                .iter()
                .map(|t| (*t).to_string())
                .collect(),
            project_path: android.project_path.clone(),
        });
    }

    if let Some(web) = config.backends.web.as_ref() {
        entries.push(BackendListEntry {
            backend: BackendChoice::Web.label().to_string(),
            version: web.version.clone(),
            dev: web.dev || config.dev_dependencies,
            targets: backend_targets(BackendChoice::Web)
                .iter()
                .map(|t| (*t).to_string())
                .collect(),
            project_path: web.project_path.clone(),
        });
    }

    if entries.is_empty() && !output::global_output_format().is_json() {
        ui::warning("No backends are configured for this project.");
    } else if !output::global_output_format().is_json() {
        ui::section("Configured Backends");
        for entry in &entries {
            ui::kv("Backend", &entry.backend);
            ui::kv(
                "Version",
                entry
                    .version
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            ui::kv("Dev install", entry.dev.to_string());
            ui::kv("Targets", entry.targets.join(", "));
            ui::kv("Path", &entry.project_path);
            ui::newline();
        }
    }

    Ok(BackendListReport { entries })
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
        let previous_commit = read_android_dev_commit(project_dir)?;
        let source = ensure_dev_android_backend_checkout()?;
        let new_commit = git_head_commit(&source);
        copy_android_backend(project_dir, &source)?;
        configure_android_local_properties(project_dir)?;
        if let Some(commit) = &new_commit {
            write_android_dev_commit(project_dir, commit)?;
        } else {
            clear_android_dev_commit(project_dir)?;
        }
        android_cfg.dev = true;
        android_cfg.version = None;
        config.save(project_dir)?;
        report.from_version = previous_commit.clone();
        report.to_version = new_commit.clone();
        let (message, changed) = match (&previous_commit, &new_commit) {
            (Some(prev), Some(next)) if prev == next => (
                format!(
                    "Android dev backend already at commit {}. No changes applied.",
                    short_commit(next)
                ),
                false,
            ),
            (Some(prev), Some(next)) => (
                format!(
                    "Android dev backend updated ({} → {}).",
                    short_commit(prev),
                    short_commit(next)
                ),
                true,
            ),
            (None, Some(next)) => (
                format!(
                    "Android dev backend pinned to commit {}.",
                    short_commit(next)
                ),
                true,
            ),
            _ => (
                "Synchronized dev backend from the latest upstream commit.".to_string(),
                true,
            ),
        };
        if changed {
            report.status = BackendUpdateStatus::Updated;
            ui::success(&message);
        } else {
            report.status = BackendUpdateStatus::UpToDate;
            ui::warning(&message);
        }
        report.message = Some(message);
        sync_android_build_script(project_dir)?;
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
            "Updating Android backend from v{current_semver} to v{new_version}"
        ));
        let source = ensure_android_backend_release(&new_version.to_string())?;
        copy_android_backend(project_dir, &source)?;
        configure_android_local_properties(project_dir)?;
        clear_android_dev_commit(project_dir)?;
        android_cfg.version = Some(new_version.to_string());
        android_cfg.dev = false;
        config.save(project_dir)?;
        report.status = BackendUpdateStatus::Updated;
        report.from_version = Some(current_version);
        report.to_version = Some(new_version.to_string());
        ui::success("Android backend updated");
        sync_android_build_script(project_dir)?;
        return Ok(report);
    }

    if let Some(newer) = incompatible {
        let message = format!(
            "A newer, potentially breaking Android backend (v{newer}) is available. Update the CLI to evaluate migrating."
        );
        warn!("{message}");
        report.status = BackendUpdateStatus::Incompatible;
        report.message = Some(message);
    } else {
        ui::info("Android backend is already up to date");
    }

    sync_android_build_script(project_dir)?;
    Ok(report)
}

fn update_swift_backend(project_dir: &Path, config: &mut Config) -> Result<BackendUpdateReport> {
    let swift_cfg = config
        .backends
        .swift
        .as_mut()
        .ok_or_else(|| eyre!("Apple backend is not configured for this project"))?;

    let mut report = BackendUpdateReport::new(BackendChoice::Apple, BackendUpdateStatus::UpToDate);
    let project_path = project_dir.join(&swift_cfg.project_path);
    let project_file = swift_cfg
        .project_file
        .as_deref()
        .ok_or_else(|| eyre!("Swift project file is not recorded in Water.toml"))?;
    let pbxproj = project_path.join(project_file).join("project.pbxproj");

    let use_dev_backend = swift_cfg.dev || config.dev_dependencies;

    if use_dev_backend {
        ui::warning("Updating Apple dev backend. This may include breaking API changes.");
        let branch = swift_cfg
            .branch
            .clone()
            .unwrap_or_else(|| "dev".to_string());
        let previous_revision = swift_cfg.revision.clone();
        let revision = create::fetch_swift_branch_head(&branch).with_context(|| {
            format!("failed to resolve latest Apple backend commit for branch '{branch}'")
        })?;
        rewrite_swift_requirement(&pbxproj, SwiftRequirement::Revision(revision.clone()))?;
        swift_cfg.version = None;
        swift_cfg.revision = Some(revision.clone());
        swift_cfg.dev = true;
        if swift_cfg.branch.is_none() {
            swift_cfg.branch = Some(branch);
        }
        config.save(project_dir)?;
        report.from_version = previous_revision.clone();
        report.to_version = Some(revision.clone());
        let (message, changed) = match previous_revision {
            Some(prev) if prev == revision => (
                format!(
                    "Apple dev backend already pinned to commit {}. No changes applied.",
                    short_commit(&revision)
                ),
                false,
            ),
            Some(prev) => (
                format!(
                    "Apple dev backend updated ({} → {}).",
                    short_commit(&prev),
                    short_commit(&revision)
                ),
                true,
            ),
            None => (
                format!(
                    "Apple dev backend pinned to commit {}.",
                    short_commit(&revision)
                ),
                true,
            ),
        };
        report.status = if changed {
            BackendUpdateStatus::Updated
        } else {
            BackendUpdateStatus::UpToDate
        };
        report.message = Some(message.clone());
        if changed {
            ui::success(&message);
        } else {
            ui::warning(&message);
        }
        sync_swift_build_script(project_dir)?;
        return Ok(report);
    }

    let current_version = swift_cfg.version.clone().ok_or_else(|| {
        eyre!("Apple backend version is unknown. Re-create the backend to refresh metadata.")
    })?;
    let current_semver = Version::parse(&current_version)
        .with_context(|| format!("failed to parse Apple backend version {current_version}"))?;

    let available = fetch_repo_versions(SWIFT_BACKEND_GIT_URL, SWIFT_TAG_PREFIX)?;
    let (candidate, incompatible) = select_compatible_version(&current_semver, &available);

    if let Some(new_version) = candidate {
        ui::step(format!(
            "Updating Apple backend from v{current_semver} to v{new_version}"
        ));
        rewrite_swift_requirement(&pbxproj, SwiftRequirement::Release(new_version.to_string()))?;
        swift_cfg.version = Some(new_version.to_string());
        swift_cfg.dev = false;
        swift_cfg.branch = None;
        swift_cfg.revision = None;
        config.save(project_dir)?;
        report.status = BackendUpdateStatus::Updated;
        report.from_version = Some(current_version);
        report.to_version = Some(new_version.to_string());
        ui::success(
            "Apple backend updated. Open Xcode and resolve package dependencies to download the new version.",
        );
        sync_swift_build_script(project_dir)?;
        return Ok(report);
    }

    if let Some(newer) = incompatible {
        let message = format!(
            "A newer Apple backend major version (v{newer}) is available but may be incompatible. Update the CLI or review release notes before upgrading."
        );
        warn!("{message}");
        report.status = BackendUpdateStatus::Incompatible;
        report.message = Some(message);
    } else {
        ui::info("Apple backend is already up to date");
    }

    sync_swift_build_script(project_dir)?;
    Ok(report)
}

fn upgrade_android_backend(
    project_dir: &Path,
    config: &mut Config,
    args: &BackendUpdateArgs,
) -> Result<BackendUpdateReport> {
    let android_cfg = config
        .backends
        .android
        .as_mut()
        .ok_or_else(|| eyre!("Android backend is not configured for this project"))?;

    if android_cfg.dev || config.dev_dependencies {
        return update_android_backend(project_dir, config);
    }

    let available = fetch_repo_versions(
        android::ANDROID_BACKEND_REPO,
        android::ANDROID_BACKEND_TAG_PREFIX,
    )?;
    let latest = available
        .last()
        .cloned()
        .ok_or_else(|| eyre!("No Android backend releases available"))?;
    let release_path = ensure_android_backend_release(&latest.to_string())?;
    let required_ffi = default_ffi_version()?;

    let compatibility =
        ensure_backend_ffi_compat(project_dir, &required_ffi, args, BackendChoice::Android)?;
    if let DependencyUpgradeOutcome::Cancelled(message) = compatibility {
        let mut report =
            BackendUpdateReport::new(BackendChoice::Android, BackendUpdateStatus::Skipped);
        report.message = Some(message);
        return Ok(report);
    }

    let previous_version = android_cfg.version.clone();
    copy_android_backend(project_dir, &release_path)?;
    configure_android_local_properties(project_dir)?;
    clear_android_dev_commit(project_dir)?;
    android_cfg.version = Some(latest.to_string());
    android_cfg.dev = false;
    config.save(project_dir)?;

    let mut report = BackendUpdateReport::new(BackendChoice::Android, BackendUpdateStatus::Updated);
    report.from_version = previous_version;
    report.to_version = Some(latest.to_string());
    if let DependencyUpgradeOutcome::Upgraded(message) = compatibility {
        report.message = Some(message);
    }
    ui::success("Android backend upgraded to the latest release");
    Ok(report)
}

fn upgrade_swift_backend(
    project_dir: &Path,
    config: &mut Config,
    args: &BackendUpdateArgs,
) -> Result<BackendUpdateReport> {
    let swift_cfg = config
        .backends
        .swift
        .as_mut()
        .ok_or_else(|| eyre!("Apple backend is not configured for this project"))?;

    if swift_cfg.dev || config.dev_dependencies {
        return update_swift_backend(project_dir, config);
    }

    let available = fetch_repo_versions(SWIFT_BACKEND_GIT_URL, SWIFT_TAG_PREFIX)?;
    let latest = available
        .last()
        .cloned()
        .ok_or_else(|| eyre!("No Apple backend releases available"))?;

    let required_ffi = default_ffi_version()?;
    let compatibility =
        ensure_backend_ffi_compat(project_dir, &required_ffi, args, BackendChoice::Apple)?;
    if let DependencyUpgradeOutcome::Cancelled(message) = compatibility {
        let mut report =
            BackendUpdateReport::new(BackendChoice::Apple, BackendUpdateStatus::Skipped);
        report.message = Some(message);
        return Ok(report);
    }

    let project_path = project_dir.join(&swift_cfg.project_path);
    let project_file = swift_cfg
        .project_file
        .as_deref()
        .ok_or_else(|| eyre!("Swift project file is not recorded in Water.toml"))?;
    let pbxproj = project_path.join(project_file).join("project.pbxproj");

    let previous_version = swift_cfg.version.clone();
    rewrite_swift_requirement(&pbxproj, SwiftRequirement::Release(latest.to_string()))?;
    swift_cfg.version = Some(latest.to_string());
    swift_cfg.dev = false;
    swift_cfg.branch = None;
    swift_cfg.revision = None;
    config.save(project_dir)?;

    let mut report = BackendUpdateReport::new(BackendChoice::Apple, BackendUpdateStatus::Updated);
    report.from_version = previous_version;
    report.to_version = Some(latest.to_string());
    if let DependencyUpgradeOutcome::Upgraded(message) = compatibility {
        report.message = Some(message);
    }
    ui::success("Apple backend requirement updated to the latest release");
    Ok(report)
}

fn short_commit(hash: &str) -> String {
    hash.chars().take(7).collect()
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
                .is_none_or(|best: &Version| candidate > best)
            {
                compatible = Some(candidate.clone());
            }
        } else if incompatible
            .as_ref()
            .is_none_or(|best: &Version| candidate > best)
        {
            incompatible = Some(candidate.clone());
        }
    }

    (compatible, incompatible)
}

enum SwiftRequirement {
    Branch(String),
    Release(String),
    Revision(String),
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
    let indent_start = contents[..req_idx].rfind('\n').map_or(0, |idx| idx + 1);
    let indent = &contents[indent_start..req_idx];
    let replacement = match requirement {
        SwiftRequirement::Branch(branch) => format!(
            "{indent}requirement = {{\n{indent}\tkind = branch;\n{indent}\tbranch = \"{branch}\";\n{indent}}};\n"
        ),
        SwiftRequirement::Release(version) => format!(
            "{indent}requirement = {{\n{indent}\tkind = upToNextMajorVersion;\n{indent}\tminimumVersion = \"{version}\";\n{indent}}};\n"
        ),
        SwiftRequirement::Revision(revision) => format!(
            "{indent}requirement = {{\n{indent}\tkind = revision;\n{indent}\trevision = \"{revision}\";\n{indent}}};\n"
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

enum DependencyUpgradeOutcome {
    Compatible,
    Upgraded(String),
    Cancelled(String),
}

struct CargoManifest {
    path: PathBuf,
    doc: Value,
}

impl CargoManifest {
    fn load(project_dir: &Path) -> Result<Self> {
        let path = project_dir.join("Cargo.toml");
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let doc: Value = contents
            .parse()
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(Self { path, doc })
    }

    fn dependency_spec(&self, name: &str) -> DependencySpec {
        let table = match self.doc.get("dependencies").and_then(Value::as_table) {
            Some(table) => table,
            None => return DependencySpec::Missing,
        };
        dependency_spec_from_value(table.get(name))
    }

    fn set_dependency_version(&mut self, name: &str, version: &str) -> Result<()> {
        let deps = self.dependencies_table_mut()?;
        deps.insert(name.to_string(), Value::String(version.to_string()));
        Ok(())
    }

    fn dependencies_table_mut(&mut self) -> Result<&mut Table> {
        let root = self
            .doc
            .as_table_mut()
            .ok_or_else(|| eyre!("Cargo manifest root is not a table"))?;
        if !root.contains_key("dependencies") {
            root.insert("dependencies".to_string(), Value::Table(Table::new()));
        }
        root.get_mut("dependencies")
            .and_then(Value::as_table_mut)
            .ok_or_else(|| eyre!("dependencies section is not a table"))
    }

    fn save(&self) -> Result<()> {
        let contents =
            toml::to_string_pretty(&self.doc).context("failed to serialize Cargo manifest")?;
        fs::write(&self.path, contents)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

#[derive(Debug)]
enum DependencySpec {
    Version(Version),
    Git,
    Other,
    Missing,
}

fn dependency_spec_from_value(value: Option<&Value>) -> DependencySpec {
    match value {
        None => DependencySpec::Missing,
        Some(Value::String(version)) => Version::parse(version)
            .map(DependencySpec::Version)
            .unwrap_or(DependencySpec::Other),
        Some(Value::Table(table)) => {
            if table.contains_key("git") {
                DependencySpec::Git
            } else if let Some(Value::String(version)) = table.get("version") {
                Version::parse(version)
                    .map(DependencySpec::Version)
                    .unwrap_or(DependencySpec::Other)
            } else {
                DependencySpec::Other
            }
        }
        _ => DependencySpec::Other,
    }
}

/// Default FFI version for backend upgrades. This is a constant that represents
/// the minimum FFI version required for latest backends.
const DEFAULT_FFI_VERSION: &str = "0.1.0";

fn default_ffi_version() -> Result<Version> {
    Version::parse(DEFAULT_FFI_VERSION)
        .with_context(|| format!("invalid default waterui-ffi version '{DEFAULT_FFI_VERSION}'"))
}

fn ensure_backend_ffi_compat(
    project_dir: &Path,
    required: &Version,
    args: &BackendUpdateArgs,
    backend: BackendChoice,
) -> Result<DependencyUpgradeOutcome> {
    let mut manifest = CargoManifest::load(project_dir)?;
    let current = manifest.dependency_spec("waterui-ffi");
    match current {
        DependencySpec::Version(ref version) => {
            if semver_compatible(version, required) {
                return Ok(DependencyUpgradeOutcome::Compatible);
            }
            if output::global_output_format().is_json() && !args.yes {
                bail!(
                    "{} backend upgrade requires updating Cargo dependencies. Re-run with --yes to confirm.",
                    backend.label()
                );
            }
            let latest = latest_cli_waterui_version()?;
            let proceed = if args.yes {
                true
            } else {
                let prompt = format!(
                    "{} backend requires waterui-ffi v{}. Update waterui and waterui-ffi to v{}?",
                    backend.label(),
                    required,
                    latest
                );
                Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(prompt)
                    .default(false)
                    .interact()?
            };
            if !proceed {
                return Ok(DependencyUpgradeOutcome::Cancelled(
                    "Backend upgrade cancelled because waterui-ffi was not updated.".to_string(),
                ));
            }
            manifest.set_dependency_version("waterui", &latest.to_string())?;
            manifest.set_dependency_version("waterui-ffi", &latest.to_string())?;
            manifest.save()?;
            Ok(DependencyUpgradeOutcome::Upgraded(format!(
                "Updated waterui dependencies to v{latest}."
            )))
        }
        DependencySpec::Git => Ok(DependencyUpgradeOutcome::Compatible),
        DependencySpec::Other => {
            warn!(
                "Unable to determine waterui-ffi version in Cargo.toml; skipping compatibility check."
            );
            Ok(DependencyUpgradeOutcome::Compatible)
        }
        DependencySpec::Missing => {
            warn!("waterui-ffi dependency not found in Cargo.toml; skipping compatibility check.");
            Ok(DependencyUpgradeOutcome::Compatible)
        }
    }
}

fn semver_compatible(current: &Version, required: &Version) -> bool {
    if required.major == 0 {
        current.major == 0 && current.minor == required.minor && current >= required
    } else {
        current.major == required.major && current >= required
    }
}

fn latest_cli_waterui_version() -> Result<Version> {
    if WATERUI_VERSION.is_empty() {
        bail!("WATERUI_VERSION is not set. Upgrade requires a released CLI build.");
    }
    Version::parse(WATERUI_VERSION)
        .with_context(|| format!("invalid WATERUI_VERSION value '{WATERUI_VERSION}'"))
}

const fn backend_targets(choice: BackendChoice) -> &'static [&'static str] {
    match choice {
        BackendChoice::Android => &["Android"],
        BackendChoice::Apple => &["macOS", "iOS", "iPadOS", "watchOS", "tvOS", "visionOS"],
        BackendChoice::Web => &["Web"],
    }
}

fn sync_android_build_script(project_dir: &Path) -> Result<()> {
    write_template_file(
        "android/build-rust.sh.tpl",
        &project_dir.join("build-rust.sh"),
    )
}

fn sync_swift_build_script(project_dir: &Path) -> Result<()> {
    write_template_file(
        "apple/build-rust.sh.tpl",
        &project_dir.join("apple/build-rust.sh"),
    )
}

fn write_template_file(relative_path: &str, destination: &Path) -> Result<()> {
    let template = create::template::TEMPLATES_DIR
        .get_file(relative_path)
        .ok_or_else(|| eyre!("Missing template asset {relative_path}"))?;
    if let Some(parent) = destination.parent() {
        util::ensure_directory(parent)?;
    }
    fs::write(destination, template.contents())?;
    mark_executable(destination)?;
    Ok(())
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)?;
    let mut perms = metadata.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> Result<()> {
    Ok(())
}
