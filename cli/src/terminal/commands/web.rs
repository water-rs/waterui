//! Shared web-frontend scaffolding for `create --template web` and `init`:
//! running the declared package manager, moving or copying a frontend into
//! `web/`, and the package-manager prompt.

use std::path::Path;
use std::process::Stdio;

use color_eyre::eyre::{Result, bail};
use dialoguer::{Select, theme::ColorfulTheme};
use walkdir::WalkDir;

use crate::shell::Shell;
use crate::success;
use waterui_cli::web::PackageManager;

/// The `create`/`init` package-manager prompt, asked once; `initial` is the
/// pre-selected entry — a lockfile guess or the `bun` default.
pub fn prompt_package_manager(initial: PackageManager) -> Result<PackageManager> {
    const MANAGERS: [PackageManager; 4] = [
        PackageManager::Bun,
        PackageManager::Pnpm,
        PackageManager::Npm,
        PackageManager::Yarn,
    ];
    let items: Vec<&str> = MANAGERS.iter().map(|pm| pm.binary()).collect();
    let default = MANAGERS
        .iter()
        .position(|pm| *pm == initial)
        .unwrap_or_default();
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Package manager for the web frontend")
        .items(&items)
        .default(default)
        .interact()?;
    Ok(MANAGERS[selection])
}

/// Fail before touching disk when the declared manager is not on `PATH`.
pub async fn ensure_installed(package_manager: PackageManager) -> Result<()> {
    if !package_manager.is_installed().await {
        bail!(
            "`{}` is not installed; run `water doctor`",
            package_manager.binary()
        );
    }
    Ok(())
}

/// `<pm> create vite <dir>` in `parent` with stdio inherited, so Vite's own
/// output — and its framework picker when no `--vite-template` was given —
/// reaches the user.
pub async fn create_vite(
    shell: &Shell,
    parent: &Path,
    dir: &str,
    package_manager: PackageManager,
    template: Option<&str>,
) -> Result<()> {
    let spinner = shell.spinner("Scaffolding the Vite frontend...");
    let status = package_manager
        .create_vite(dir, template)
        .current_dir(parent)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    if !status.success() {
        bail!(
            "`{} create vite` failed in {}: {status}",
            package_manager.binary(),
            parent.display()
        );
    }
    success!(shell, "Scaffolded the Vite frontend in {dir}/");
    Ok(())
}

/// `<pm> install` inside `dir` with stdio inherited.
pub async fn install_dependencies(
    shell: &Shell,
    package_manager: PackageManager,
    dir: &Path,
) -> Result<()> {
    let spinner = shell.spinner("Installing frontend dependencies...");
    let status = package_manager
        .install()
        .current_dir(dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    if !status.success() {
        bail!(
            "`{} install` failed in {}: {status}",
            package_manager.binary(),
            dir.display()
        );
    }
    Ok(())
}

/// Move top-level entries of `root` into `root/web`. Entries tracked by git
/// move with `git mv` to preserve history; the rest rename.
pub async fn move_entries_to_web(root: &Path, entries: &[std::path::PathBuf]) -> Result<()> {
    smol::fs::create_dir_all(root.join("web")).await?;
    for entry in entries {
        let source = root.join(entry);
        let dest = root.join("web").join(entry);
        if git_tracked(root, entry).await {
            let status = smol::process::Command::new("git")
                .args(["mv"])
                .arg(entry)
                .arg(Path::new("web").join(entry))
                .current_dir(root)
                .status()
                .await?;
            if !status.success() {
                bail!("`git mv {} web/` failed: {status}", entry.display());
            }
        } else {
            smol::fs::rename(&source, &dest).await?;
        }
    }
    Ok(())
}

/// Whether `entry` is tracked in the git work tree at `root`.
async fn git_tracked(root: &Path, entry: &Path) -> bool {
    smol::process::Command::new("git")
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(entry)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// Copy a frontend project into `dest`, skipping `node_modules` and `.git`.
pub async fn copy_frontend(source: &Path, dest: &Path) -> Result<()> {
    let source = source.to_path_buf();
    let dest = dest.to_path_buf();
    smol::unblock(move || {
        for item in WalkDir::new(&source).into_iter().filter_entry(|entry| {
            !matches!(entry.file_name().to_str(), Some("node_modules" | ".git"))
        }) {
            let item = item?;
            let target = dest.join(item.path().strip_prefix(&source)?);
            if item.file_type().is_dir() {
                std::fs::create_dir_all(&target)?;
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(item.path(), &target)?;
            }
        }
        Ok::<_, color_eyre::eyre::Error>(())
    })
    .await
}
