//! Runtime fingerprint computation shared by preview and inspector launchers.

use std::path::Path;
use std::process::Command;

use color_eyre::eyre::{Context as _, Result, bail};
use tracing::info;

use crate::runtime_compat::{runtime_fingerprint_root_dirs, runtime_fingerprint_root_files};

#[allow(clippy::redundant_pub_crate)]
pub(crate) fn runtime_package_identity(package: &cargo_metadata::Package) -> String {
    format!("{}@{}", package.name, package.version)
}

#[allow(clippy::redundant_pub_crate)]
pub(crate) async fn compute_runtime_fingerprint(
    waterui_root: &Path,
    runtime_identity: &str,
) -> Result<String> {
    let waterui_root = waterui_root.to_path_buf();
    let runtime_identity = runtime_identity.to_string();
    smol::unblock(move || compute_runtime_fingerprint_sync(&waterui_root, &runtime_identity)).await
}

fn compute_runtime_fingerprint_sync(waterui_root: &Path, runtime_identity: &str) -> Result<String> {
    let git_fingerprint_start = std::time::Instant::now();
    let git_fingerprint = compute_git_clean_fingerprint(waterui_root, runtime_identity)?;
    info!(
        waterui_root = %waterui_root.display(),
        elapsed_ms = git_fingerprint_start.elapsed().as_millis(),
        "Runtime fingerprint used clean git commit"
    );
    Ok(git_fingerprint)
}

fn compute_git_clean_fingerprint(root: &Path, runtime_identity: &str) -> Result<String> {
    if !is_git_work_tree(root) {
        bail!(
            "Preview dev mode requires `waterui_path` to point at a git worktree: {}",
            root.display()
        );
    }

    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=normal")
        .arg("--")
        .args(runtime_fingerprint_root_files())
        .args(runtime_fingerprint_root_dirs())
        .output()
        .wrap_err_with(|| format!("Failed to run `git status` for {}", root.display()))?;
    if !status.status.success() {
        bail!(
            "Failed to inspect WaterUI worktree state at {}",
            root.display()
        );
    }
    if !status.stdout.is_empty() {
        bail!(
            "Preview dev mode requires a clean WaterUI worktree at {}. Commit or stash changes before running preview.",
            root.display()
        );
    }

    let head = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .wrap_err_with(|| format!("Failed to run `git rev-parse HEAD` for {}", root.display()))?;
    if !head.status.success() {
        bail!("Failed to resolve WaterUI commit at {}", root.display());
    }

    let commit = String::from_utf8(head.stdout)
        .wrap_err("`git rev-parse HEAD` returned non-UTF8 output")?
        .trim()
        .to_string();
    if commit.is_empty() {
        bail!("Resolved empty WaterUI commit for {}", root.display());
    }

    Ok(format!("{runtime_identity}:git:{commit}"))
}

fn is_git_work_tree(root: &Path) -> bool {
    let inside_work_tree = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output();
    let Ok(inside_work_tree) = inside_work_tree else {
        return false;
    };
    inside_work_tree.status.success()
        && String::from_utf8_lossy(&inside_work_tree.stdout).trim() == "true"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn uses_git_commit_fingerprint_when_tree_is_clean() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        commit_all(dir.path(), "init");

        let fingerprint = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("fingerprint");
        assert!(fingerprint.contains(":git:"));
    }

    #[test]
    fn rejects_dirty_runtime_inputs() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        commit_all(dir.path(), "init");

        write_runtime_file(
            dir.path(),
            "core/src/lib.rs",
            "pub fn a() { let _x = 1; }\n",
        );
        let error = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect_err("dirty runtime inputs must fail");
        assert!(
            error
                .to_string()
                .contains("requires a clean WaterUI worktree")
        );
    }

    #[test]
    fn rejects_untracked_runtime_inputs() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        commit_all(dir.path(), "init");

        write_runtime_file(dir.path(), "core/src/new.rs", "pub fn b() {}\n");
        let error = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect_err("untracked runtime inputs must fail");
        assert!(
            error
                .to_string()
                .contains("requires a clean WaterUI worktree")
        );
    }

    #[test]
    fn ignores_example_changes_for_git_fast_path() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        write_runtime_file(
            dir.path(),
            "examples/demo/src/lib.rs",
            "pub fn preview() {}\n",
        );
        commit_all(dir.path(), "init");

        write_runtime_file(
            dir.path(),
            "examples/demo/src/lib.rs",
            "pub fn preview() { let _changed = true; }\n",
        );

        let fingerprint = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("fingerprint");
        assert!(fingerprint.contains(":git:"));
    }

    #[test]
    fn clean_git_fingerprint_is_stable_across_clone_paths() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        commit_all(dir.path(), "init");

        let clone_dir = tempdir().expect("clone dir");
        let cloned_repo = clone_dir.path().join("repo");
        let status = Command::new("git")
            .arg("clone")
            .arg(dir.path())
            .arg(&cloned_repo)
            .status()
            .expect("git clone should run");
        assert!(
            status.success(),
            "git clone failed into {}",
            cloned_repo.display()
        );

        let original = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("original fingerprint");
        let cloned = compute_runtime_fingerprint_sync(&cloned_repo, "waterui-core@0.0.1")
            .expect("cloned fingerprint");
        assert_eq!(original, cloned);
    }

    #[test]
    fn rejects_non_git_runtime_root() {
        let dir = tempdir().expect("temp dir");
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");

        let error = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect_err("non-git root must fail");
        assert!(
            error
                .to_string()
                .contains("requires `waterui_path` to point at a git worktree")
        );
    }

    fn write_runtime_file(root: &Path, relative_path: &str, contents: &str) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("runtime file must have parent"))
            .expect("create runtime parent");
        fs::write(path, contents).expect("write runtime file");
    }

    fn init_git_repo(root: &Path) {
        run_git(root, ["init"]);
        run_git(root, ["config", "user.name", "WaterUI Test"]);
        run_git(root, ["config", "user.email", "waterui-test@example.com"]);
    }

    fn commit_all(root: &Path, message: &str) {
        run_git(root, ["add", "."]);
        run_git(root, ["commit", "-m", message]);
    }

    fn run_git<I, S>(root: &Path, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git command should run");
        assert!(status.success(), "git command failed in {}", root.display());
    }
}
