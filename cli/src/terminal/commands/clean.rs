//! `water clean` command implementation.

use std::{
    collections::BTreeSet,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{self, Result, bail};
use dialoguer::{Confirm, theme::ColorfulTheme};
use futures::{StreamExt, stream};
use ignore::{DirEntry, WalkBuilder};
use indicatif::{ProgressBar, ProgressStyle};

use crate::shell;
use crate::{header, note, success, warn};
use waterui_cli::{
    android::platform::clean_android,
    apple::platform::clean_apple,
    gtk4::platform::clean_gtk4,
    hydrolysis::platform::clean_hydrolysis,
    project::{Manifest, PackageType, Project},
    water_dir,
};

/// Target backend for cleaning.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum TargetBackend {
    /// Apple backend (iOS/macOS).
    Apple,
    /// Android backend.
    Android,
    /// GTK4 backend (Linux).
    Gtk4,
    /// Hydrolysis backend (self-drawn renderer).
    Hydrolysis,
    /// All backends.
    All,
}

/// Arguments for the clean command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target backend to clean (defaults to all).
    #[arg(short, long, value_enum, default_value = "all")]
    backend: TargetBackend,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Recursively find all valid WaterUI projects under `--path` and clean each project's `.water` and `target` directories.
    #[arg(short = 'r', long)]
    recursive: bool,

    /// Clean the global playground project cache under `~/.water/project_cache`.
    #[arg(long, visible_alias = "project-cache")]
    global_cache: bool,

    /// Skip confirmation prompt in recursive mode.
    #[arg(short = 'y', long)]
    yes: bool,
}

/// Run the clean command.
pub async fn run(args: Args) -> Result<()> {
    if args.global_cache {
        if args.recursive {
            bail!("`water clean --global-cache` cannot be combined with --recursive");
        }
        if args.backend != TargetBackend::All {
            warn!(
                "Ignoring `--backend {:?}` when cleaning the global playground cache",
                args.backend
            );
        }
        if args.path != PathBuf::from(".") {
            warn!(
                "Ignoring `--path {}` when cleaning the global playground cache",
                args.path.display()
            );
        }
        return clean_global_playground_cache(args.yes).await;
    }

    let root_path = crate::project_path::canonicalize(&args.path)?;

    if args.recursive {
        ensure_recursive_root_is_directory(&root_path)?;
        if args.backend != TargetBackend::All {
            warn!(
                "Ignoring `--backend {:?}` in recursive mode; cleaning project cache directories only",
                args.backend
            );
        }
        return clean_recursive(&root_path, args.yes).await;
    }

    let project = Project::open(&root_path).await?;

    header!("Cleaning build artifacts...");

    match args.backend {
        TargetBackend::All => {
            let spinner = shell::spinner("Cleaning all build artifacts...");
            project.clean_all().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned all build artifacts");
        }
        TargetBackend::Apple => {
            let spinner = shell::spinner("Cleaning Apple build artifacts...");
            clean_apple(&project).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned Apple build artifacts");
        }
        TargetBackend::Android => {
            let spinner = shell::spinner("Cleaning Android build artifacts...");
            clean_android(&project).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned Android build artifacts");
        }
        TargetBackend::Gtk4 => {
            let spinner = shell::spinner("Cleaning GTK4 build artifacts...");
            clean_gtk4(&project).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned GTK4 build artifacts");
        }
        TargetBackend::Hydrolysis => {
            let spinner = shell::spinner("Cleaning hydrolysis build artifacts...");
            clean_hydrolysis(&project).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned hydrolysis build artifacts");
        }
    }

    Ok(())
}

async fn clean_global_playground_cache(yes: bool) -> Result<()> {
    let cache_root = water_dir::playground_cache_root()?;
    clean_global_playground_cache_root(cache_root, yes).await
}

async fn clean_global_playground_cache_root(cache_root: PathBuf, yes: bool) -> Result<()> {
    header!("Cleaning global playground cache...");

    if !path_exists(&cache_root).await {
        note!("No global playground cache found at {}", cache_root.display());
        return Ok(());
    }

    ensure_recursive_confirmation_mode(yes, shell::is_interactive())?;
    if !yes {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Delete the global playground cache at {}?",
                cache_root.display()
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            warn!("Cancelled global playground cache clean");
            return Ok(());
        }
    }

    remove_global_playground_cache_root(cache_root.clone()).await?;
    success!("Removed global playground cache at {}", cache_root.display());
    Ok(())
}

async fn remove_global_playground_cache_root(cache_root: PathBuf) -> Result<bool> {
    if !path_exists(&cache_root).await {
        return Ok(false);
    }
    smol::unblock(move || remove_dir_all::remove_dir_all(&cache_root)).await?;
    Ok(true)
}

async fn clean_recursive(root: &Path, yes: bool) -> Result<()> {
    header!("Recursively cleaning `.water` and `target` directories...");

    let spinner = shell::spinner("Scanning for WaterUI projects...");
    let cache_plan = CachePlan::discover(root).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    if cache_plan.project_count == 0 {
        warn!("No valid WaterUI projects found under {}", root.display());
        return Ok(());
    }

    if cache_plan.cache_dirs.is_empty() {
        note!("Found projects, but no cache directories needed cleaning");
        return Ok(());
    }

    let total_dirs_to_remove = cache_plan.cache_dirs.len();

    ensure_recursive_confirmation_mode(yes, shell::is_interactive())?;

    if !yes {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Delete {total_dirs_to_remove} cache directories across {} project(s) under {}?",
                cache_plan.project_count,
                root.display()
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            warn!("Cancelled recursive clean");
            return Ok(());
        }
    }

    let progress = make_progress_bar(total_dirs_to_remove as u64);

    let mut removed_dirs = 0usize;

    let mut clean_results = stream::iter(cache_plan.cache_dirs.into_iter().map(|cache_dir| {
        let progress = progress.clone();
        async move { clean_cache_dir(cache_dir, progress).await }
    }))
    .buffer_unordered(removal_parallelism());

    while let Some(result) = clean_results.next().await {
        if let Some(cache_dir) = result? {
            removed_dirs += 1;
            success!("Removed {}", cache_dir.display());
        }
    }
    drop(clean_results);

    if let Some(pb) = progress {
        pb.finish_and_clear();
    }

    success!(
        "Recursive clean complete: scanned {} project(s), removed {} directory(s)",
        cache_plan.project_count,
        removed_dirs
    );

    Ok(())
}

fn ensure_recursive_confirmation_mode(yes: bool, interactive: bool) -> Result<()> {
    if !yes && !interactive {
        bail!("`water clean --recursive` requires --yes in non-interactive environments");
    }
    Ok(())
}

fn ensure_recursive_root_is_directory(root: &Path) -> Result<()> {
    if !root.is_dir() {
        bail!(
            "`water clean --recursive --path` requires a directory path, got {}",
            root.display()
        );
    }
    Ok(())
}

async fn discover_projects(root: &Path) -> Result<Vec<PathBuf>> {
    let root = root.to_path_buf();
    smol::unblock(move || discover_projects_blocking(&root)).await
}

#[derive(Debug)]
struct CachePlan {
    project_count: usize,
    cache_dirs: Vec<PathBuf>,
}

impl CachePlan {
    async fn discover(root: &Path) -> Result<Self> {
        let project_roots = discover_projects(root).await?;
        let project_count = project_roots.len();
        let cache_dirs = collect_existing_cache_dirs(project_roots).await?;
        Ok(Self {
            project_count,
            cache_dirs,
        })
    }
}

async fn collect_existing_cache_dirs(project_roots: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut cache_dirs = BTreeSet::new();
    let mut discovered = stream::iter(
        project_roots
            .into_iter()
            .map(|project_root| async move { discover_project_cache_dirs(project_root).await }),
    )
    .buffer_unordered(discovery_parallelism());
    while let Some(project_cache_dirs) = discovered.next().await.transpose()? {
        cache_dirs.extend(project_cache_dirs);
    }

    let mut existing_cache_dirs = Vec::new();
    for cache_dir in cache_dirs {
        if path_exists(&cache_dir).await {
            existing_cache_dirs.push(cache_dir);
        }
    }

    Ok(collapse_nested_cache_dirs(existing_cache_dirs))
}

async fn discover_project_cache_dirs(project_root: PathBuf) -> Result<BTreeSet<PathBuf>> {
    let mut cache_dirs = BTreeSet::from([managed_project_cache_dir(&project_root).await?]);
    match resolve_target_dir(project_root.clone()).await {
        Ok(target_dir) => {
            cache_dirs.insert(target_dir);
        }
        Err(error) => warn!(
            "Skipping target cache discovery for {}: {}",
            project_root.display(),
            error
        ),
    }
    Ok(cache_dirs)
}

async fn managed_project_cache_dir(project_root: &Path) -> Result<PathBuf> {
    let manifest = Manifest::open(project_root.join("Water.toml"))
        .await
        .map_err(eyre::Report::from)?;
    match manifest.package.package_type {
        PackageType::Playground => water_dir::playground_cache_dir(project_root),
        PackageType::App => Ok(project_root.join(".water")),
    }
}

async fn resolve_target_dir(
    project_root: PathBuf,
) -> std::result::Result<PathBuf, cargo_metadata::Error> {
    smol::unblock(move || {
        let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
        metadata_cmd.current_dir(&project_root);
        metadata_cmd.no_deps();
        let metadata = metadata_cmd.exec()?;
        Ok(metadata.target_directory.as_std_path().to_path_buf())
    })
    .await
}

fn discover_projects_blocking(root: &Path) -> Result<Vec<PathBuf>> {
    let mut project_roots = BTreeSet::new();
    let mut builder = WalkBuilder::new(root);
    builder.hidden(false);
    builder.parents(false);
    builder.ignore(false);
    builder.git_ignore(false);
    builder.git_global(false);
    builder.git_exclude(false);
    builder.require_git(false);
    builder.filter_entry(|entry| should_descend(entry));

    for entry in builder.build() {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_name() != OsStr::new("Water.toml") {
            continue;
        }
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        if !manifest_is_valid(entry.path()) {
            continue;
        }
        let project_root = entry
            .path()
            .parent()
            .expect("Water.toml entries should always have a parent directory");
        project_roots.insert(project_root.to_path_buf());
    }

    Ok(project_roots.into_iter().collect())
}

fn should_descend(entry: &DirEntry) -> bool {
    entry.depth() == 0 || !should_skip_dir(entry.file_name())
}

fn manifest_is_valid(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    toml::from_str::<Manifest>(&contents).is_ok()
}

fn collapse_nested_cache_dirs(mut cache_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    cache_dirs.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut collapsed = Vec::new();
    for cache_dir in cache_dirs {
        if collapsed
            .iter()
            .any(|existing: &PathBuf| cache_dir.starts_with(existing))
        {
            continue;
        }
        collapsed.push(cache_dir);
    }
    collapsed
}

async fn clean_cache_dir(
    cache_dir: PathBuf,
    progress: Option<ProgressBar>,
) -> Result<Option<PathBuf>> {
    if !path_exists(&cache_dir).await {
        return Ok(None);
    }

    if let Some(pb) = progress.as_ref() {
        pb.set_message(format!("Removing {}", cache_dir.display()));
    }
    let cache_dir_for_remove = cache_dir.clone();
    smol::unblock(move || remove_dir_all::remove_dir_all(&cache_dir_for_remove)).await?;
    if let Some(pb) = progress.as_ref() {
        pb.inc(1);
    }

    Ok(Some(cache_dir))
}

fn make_progress_bar(total: u64) -> Option<ProgressBar> {
    if !shell::is_interactive() {
        return None;
    }

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} {msg}")
            .expect("valid template")
            .progress_chars("=>-"),
    );
    Some(pb)
}

fn clean_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(4)
        .clamp(2, 16)
}

fn discovery_parallelism() -> usize {
    clean_parallelism()
}

fn removal_parallelism() -> usize {
    if cfg!(target_os = "macos") {
        return 1;
    }
    clean_parallelism()
}

async fn path_exists(path: &Path) -> bool {
    smol::fs::metadata(path).await.is_ok()
}

fn should_skip_dir(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "node_modules" | ".water" | "target")
    )
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, fs, path::Path};

    use smol::block_on;
    use tempfile::tempdir;

    use super::{
        collapse_nested_cache_dirs, discover_projects, ensure_recursive_confirmation_mode,
        ensure_recursive_root_is_directory, remove_global_playground_cache_root, should_skip_dir,
    };
    use waterui_cli::project::{Manifest, Package, PackageType};

    #[test]
    fn skip_dir_filters_heavy_dirs() {
        assert!(should_skip_dir(OsStr::new(".git")));
        assert!(should_skip_dir(OsStr::new("node_modules")));
        assert!(should_skip_dir(OsStr::new(".water")));
        assert!(should_skip_dir(OsStr::new("target")));
        assert!(!should_skip_dir(OsStr::new("src")));
    }

    #[test]
    fn collapse_nested_cache_dirs_skips_redundant_children() {
        let root = Path::new("/tmp/waterui-clean-test");
        let collapsed = collapse_nested_cache_dirs(vec![
            root.join("target/debug"),
            root.join(".water"),
            root.join("target"),
        ]);
        assert_eq!(collapsed, vec![root.join(".water"), root.join("target")]);
    }

    #[test]
    fn recursive_requires_yes_when_non_interactive() {
        assert!(ensure_recursive_confirmation_mode(false, false).is_err());
        assert!(ensure_recursive_confirmation_mode(true, false).is_ok());
        assert!(ensure_recursive_confirmation_mode(false, true).is_ok());
    }

    #[test]
    fn recursive_requires_directory_root() {
        let temp = tempdir().expect("tempdir");
        let manifest = temp.path().join("Water.toml");
        fs::write(&manifest, "").expect("write manifest placeholder");

        assert!(ensure_recursive_root_is_directory(temp.path()).is_ok());
        assert!(ensure_recursive_root_is_directory(&manifest).is_err());
    }

    #[test]
    fn discover_projects_includes_gitignored_worktrees() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join(".gitignore"), ".worktrees/\n").expect("write gitignore");

        let visible_project = temp.path().join("examples/visible");
        write_manifest(&visible_project, "visible");

        let ignored_project = temp
            .path()
            .join(".worktrees/stale/examples/ignored-project");
        write_manifest(&ignored_project, "ignored-project");

        let discovered = block_on(discover_projects(temp.path())).expect("discover projects");

        assert_eq!(discovered, vec![ignored_project, visible_project]);
    }

    #[test]
    fn clean_global_playground_cache_removes_cache_root() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        write_manifest(&project, "demo");
        let cache_root = temp.path().join("project_cache");
        let project_cache_dir = cache_root.join("demo");
        fs::create_dir_all(&project_cache_dir).expect("create project cache");
        fs::write(project_cache_dir.join("marker"), b"cache").expect("write marker");

        assert!(
            block_on(remove_global_playground_cache_root(cache_root.clone()))
                .expect("remove global cache")
        );

        assert!(!cache_root.exists());
    }

    fn write_manifest(project_root: &Path, name: &str) {
        fs::create_dir_all(project_root).expect("create project root");
        let manifest = Manifest::new(Package {
            package_type: PackageType::Playground,
            name: name.to_owned(),
            bundle_identifier: format!("dev.waterui.{name}"),
            assets_path: String::from("assets"),
            accessory: false,
        });
        block_on(manifest.save(project_root)).expect("save manifest");
    }
}
