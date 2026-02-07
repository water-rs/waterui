//! `water clean` command implementation.

use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::Result;
use dialoguer::{Confirm, theme::ColorfulTheme};
use futures::{StreamExt, stream};
use indicatif::{ProgressBar, ProgressStyle};

use crate::shell;
use crate::{header, note, success, warn};
use waterui_cli::{
    android::platform::clean_android,
    apple::platform::clean_apple,
    gtk4::platform::clean_gtk4,
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
                "Ignoring `--backend {:?}` in recursive mode; cleaning `.water` and `target` only",
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

    let mut total_dirs_to_remove = 0usize;
    for project_root in &project_roots {
        total_dirs_to_remove += removable_cache_dir_count(project_root).await;
    }

    if total_dirs_to_remove == 0 {
        note!("Found projects, but no `.water` or `target` directories needed cleaning");
        return Ok(());
    }

    if !yes {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Delete {total_dirs_to_remove} cache directories across {} project(s) under {}?",
                project_roots.len(),
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

    let mut cleaned_projects = 0usize;
    let mut removed_dirs = 0usize;

    let mut clean_results = stream::iter(project_roots.into_iter().map(|project_root| {
        let progress = progress.clone();
        async move {
            let removed = clean_project_caches(&project_root, progress).await?;
            Ok::<_, color_eyre::Report>((project_root, removed))
        }
    }))
    .buffer_unordered(clean_parallelism());

    while let Some(result) = clean_results.next().await {
        let (project_root, removed) = result?;
        if removed > 0 {
            cleaned_projects += 1;
            removed_dirs += removed;
            success!(
                "Cleaned {} ({})",
                project_root.display(),
                if removed == 2 {
                    ".water + target"
                } else {
                    "partial cache"
                }
            );
        }
    }
    drop(clean_results);

    if let Some(pb) = progress {
        pb.finish_and_clear();
    }

    success!(
        "Recursive clean complete: cleaned {} project(s), removed {} directory(s)",
        cleaned_projects,
        removed_dirs
    );

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

async fn removable_cache_dir_count(project_root: &Path) -> usize {
    let mut count = 0usize;

    for dir_name in [".water", "target"] {
        if path_exists(&project_root.join(dir_name)).await {
            count += 1;
        }
    }

    count
}

async fn clean_project_caches(project_root: &Path, progress: Option<ProgressBar>) -> Result<usize> {
    let mut removed = 0usize;

    for dir_name in [".water", "target"] {
        let dir = project_root.join(dir_name);
        if path_exists(&dir).await {
            if let Some(pb) = progress.as_ref() {
                pb.set_message(format!("Removing {}", dir.display()));
            }
            smol::fs::remove_dir_all(&dir).await?;
            removed += 1;
            if let Some(pb) = progress.as_ref() {
                pb.inc(1);
            }
        }
    }

    Ok(removed)
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

    use super::{removable_cache_dir_count, should_skip_dir};

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
        assert_eq!(block_on(removable_cache_dir_count(missing)), 0);
    }
}
