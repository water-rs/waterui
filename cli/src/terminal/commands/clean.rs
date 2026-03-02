//! `water clean` command implementation.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use dialoguer::{Confirm, theme::ColorfulTheme};
use futures::{StreamExt, stream};
use indicatif::{ProgressBar, ProgressStyle};

use crate::shell;
use crate::{header, note, success, warn};
use waterui_cli::{
    android::platform::clean_android,
    apple::platform::clean_apple,
    gtk4::platform::clean_gtk4,
    hydrolysis::platform::clean_hydrolysis,
    project::{Manifest, Project},
};

/// Target backend for cleaning.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum TargetBackend {
    /// Apple backend (iOS/macOS).
    Apple,
    /// Android backend.
    Android,
    /// GTK4 backend (Linux/macOS/Windows).
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

    /// Skip confirmation prompt in recursive mode.
    #[arg(short = 'y', long)]
    yes: bool,
}

/// Run the clean command.
pub async fn run(args: Args) -> Result<()> {
    let root_path = crate::project_path::canonicalize(&args.path)?;

    if args.recursive {
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

async fn clean_recursive(root: &Path, yes: bool) -> Result<()> {
    header!("Recursively cleaning `.water` and `target` directories...");

    let spinner = shell::spinner("Scanning for WaterUI projects...");
    let project_roots = discover_projects(root).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    if project_roots.is_empty() {
        warn!("No valid WaterUI projects found under {}", root.display());
        return Ok(());
    }

    let mut project_cache_dirs = Vec::with_capacity(project_roots.len());
    let mut discovered = stream::iter(
        project_roots
            .into_iter()
            .map(|project_root| async move { ProjectCacheDirs::discover(project_root).await }),
    )
    .buffer_unordered(clean_parallelism());
    while let Some(cache_dirs) = discovered.next().await {
        project_cache_dirs.push(cache_dirs);
    }

    let unique_cache_dirs = unique_cache_dirs(&project_cache_dirs);
    let total_dirs_to_remove = removable_cache_dir_count(&unique_cache_dirs).await;

    if total_dirs_to_remove == 0 {
        note!("Found projects, but no cache directories needed cleaning");
        return Ok(());
    }

    ensure_recursive_confirmation_mode(yes, shell::is_interactive())?;

    if !yes {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Delete {total_dirs_to_remove} cache directories across {} project(s) under {}?",
                project_cache_dirs.len(),
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

    let project_count = project_cache_dirs.len();
    let mut removed_dirs = 0usize;

    let mut clean_results = stream::iter(unique_cache_dirs.into_iter().map(|cache_dir| {
        let progress = progress.clone();
        async move { clean_cache_dir(cache_dir, progress).await }
    }))
    .buffer_unordered(clean_parallelism());

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
        project_count,
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

async fn discover_projects(root: &Path) -> Result<Vec<PathBuf>> {
    let mut stack = vec![root.to_path_buf()];
    let mut project_roots = Vec::new();

    while let Some(dir) = stack.pop() {
        let manifest_path = dir.join("Water.toml");
        if path_exists(&manifest_path).await && Manifest::open(&manifest_path).await.is_ok() {
            project_roots.push(dir.clone());
        }

        let mut entries = match smol::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        while let Some(entry) = entries.next().await {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();

            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            if should_skip_dir(&name) {
                continue;
            }

            stack.push(path);
        }
    }

    project_roots.sort();
    project_roots.dedup();
    Ok(project_roots)
}

#[derive(Debug)]
struct ProjectCacheDirs {
    cache_dirs: Vec<PathBuf>,
}

impl ProjectCacheDirs {
    async fn discover(project_root: PathBuf) -> Self {
        let mut cache_dirs = vec![project_root.join(".water")];
        match resolve_target_dir(project_root.clone()).await {
            Ok(target_dir) => cache_dirs.push(target_dir),
            Err(error) => warn!(
                "Skipping target cache discovery for {}: {}",
                project_root.display(),
                error
            ),
        }
        cache_dirs.sort();
        cache_dirs.dedup();
        Self { cache_dirs }
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

fn unique_cache_dirs(project_cache_dirs: &[ProjectCacheDirs]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for project_cache_dirs in project_cache_dirs {
        for dir in &project_cache_dirs.cache_dirs {
            if seen.insert(dir.clone()) {
                unique.push(dir.clone());
            }
        }
    }

    unique
}

async fn removable_cache_dir_count(cache_dirs: &[PathBuf]) -> usize {
    let mut count = 0usize;

    for dir in cache_dirs {
        if path_exists(dir).await {
            count += 1;
        }
    }

    count
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
    smol::fs::remove_dir_all(&cache_dir).await?;
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

async fn path_exists(path: &Path) -> bool {
    smol::fs::metadata(path).await.is_ok()
}

fn should_skip_dir(name: &str) -> bool {
    matches!(name, ".git" | "node_modules" | ".water" | "target")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use smol::block_on;

    use super::{ensure_recursive_confirmation_mode, removable_cache_dir_count, should_skip_dir};

    #[test]
    fn skip_dir_filters_heavy_dirs() {
        assert!(should_skip_dir(".git"));
        assert!(should_skip_dir("node_modules"));
        assert!(should_skip_dir(".water"));
        assert!(should_skip_dir("target"));
        assert!(!should_skip_dir("src"));
    }

    #[test]
    fn removable_cache_count_is_zero_when_missing() {
        let missing = Path::new("/definitely/not/exist/waterui-clean-test");
        let cache_dirs = vec![missing.join(".water"), missing.join("target")];
        assert_eq!(block_on(removable_cache_dir_count(&cache_dirs)), 0);
    }

    #[test]
    fn recursive_requires_yes_when_non_interactive() {
        assert!(ensure_recursive_confirmation_mode(false, false).is_err());
        assert!(ensure_recursive_confirmation_mode(true, false).is_ok());
        assert!(ensure_recursive_confirmation_mode(false, true).is_ok());
    }
}
