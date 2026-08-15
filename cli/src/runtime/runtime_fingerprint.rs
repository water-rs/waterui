//! Runtime fingerprint computation shared by preview and inspector launchers.

use std::path::Path;
use std::process::Command;

use color_eyre::eyre::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};
use tracing::info;

use crate::runtime_compat::{runtime_fingerprint_root_dirs, runtime_fingerprint_root_files};
use crate::templates::scaffold_template_digest;

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
    let git_fingerprint = compute_git_fingerprint(waterui_root, runtime_identity)?;
    // The generated host crate is a product of *both* the runtime source and the
    // scaffold templates baked into this CLI. Two CLI builds at the same WaterUI
    // commit can emit different code, so a fingerprint over the runtime alone
    // lets a stale generated crate survive a CLI upgrade — it then fails to
    // compile against the API it was regenerated for. Mixing the template digest
    // in makes a template edit invalidate the cache the way a source edit does.
    let fingerprint = format!("{git_fingerprint}-{}", scaffold_template_digest());
    info!(
        waterui_root = %waterui_root.display(),
        elapsed_ms = git_fingerprint_start.elapsed().as_millis(),
        "Runtime fingerprint used clean git commit and scaffold template digest"
    );
    Ok(fingerprint)
}

/// A key that changes whenever the runtime a dev-mode build would compile does.
///
/// The commit alone is not that key: a worktree with uncommitted work builds
/// something the commit does not describe, and reusing an artifact cached under
/// it would run stale code. Uncommitted work is therefore hashed into the key
/// rather than rejected — refusing to build is no answer for tools whose whole
/// purpose is to look at what you are in the middle of changing.
fn compute_git_fingerprint(root: &Path, runtime_identity: &str) -> Result<String> {
    if !is_git_work_tree(root) {
        bail!(
            "Preview dev mode requires `waterui_path` to point at a git worktree: {}",
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

    Ok(uncommitted_digest(root)?.map_or_else(
        || format!("{runtime_identity}:git:{commit}"),
        |digest| format!("{runtime_identity}:git:{commit}:dirty:{digest:016x}"),
    ))
}

/// A digest of everything the runtime paths carry beyond `HEAD`, or `None` when
/// they carry nothing.
///
/// Two sources together cover every way a worktree can differ from its commit:
/// the diff against `HEAD` describes edits, deletions and renames of tracked
/// files, and untracked files are hashed with their paths because nothing else
/// mentions them.
fn uncommitted_digest(root: &Path) -> Result<Option<u64>> {
    let diff = git_output(
        root,
        ["diff", "HEAD", "--"],
        "Failed to diff the WaterUI worktree",
    )?;
    let untracked = git_output(
        root,
        ["ls-files", "--others", "--exclude-standard", "--"],
        "Failed to list untracked WaterUI files",
    )?;

    if diff.is_empty() && untracked.is_empty() {
        return Ok(None);
    }

    let mut hasher = Sha256::new();
    hasher.update(&diff);
    // `ls-files` sorts its output, so the same worktree hashes the same way.
    for relative_path in String::from_utf8_lossy(&untracked).lines() {
        hasher.update(relative_path.as_bytes());
        // A file listed and then removed before it is read contributes its name
        // alone, which is still a difference from the commit.
        if let Ok(contents) = std::fs::read(root.join(relative_path)) {
            hasher.update(&contents);
        }
    }

    // The leading eight bytes are kept: enough that two worktrees never collide
    // in practice, short enough to read in a path.
    let digest = hasher.finalize();
    let head = digest
        .get(..8)
        .and_then(|head| <[u8; 8]>::try_from(head).ok())
        .expect("a SHA-256 digest is longer than eight bytes");
    Ok(Some(u64::from_be_bytes(head)))
}

/// Runs `git` over the paths that make up the runtime and returns its stdout.
fn git_output<'a>(
    root: &Path,
    args: impl IntoIterator<Item = &'a str>,
    what: &str,
) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .args(runtime_fingerprint_root_files())
        .args(runtime_fingerprint_root_dirs())
        .output()
        .wrap_err_with(|| format!("{what} at {}", root.display()))?;
    if !output.status.success() {
        bail!("{what} at {}", root.display());
    }
    Ok(output.stdout)
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
    fn dirty_runtime_inputs_change_the_fingerprint() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        commit_all(dir.path(), "init");

        let clean = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("clean fingerprint");

        write_runtime_file(
            dir.path(),
            "core/src/lib.rs",
            "pub fn a() { let _x = 1; }\n",
        );
        let dirty = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("a worktree with uncommitted work still has a fingerprint");

        assert_ne!(clean, dirty, "an edit must not reuse the commit's key");
        assert!(dirty.contains(":dirty:"), "got {dirty}");

        write_runtime_file(
            dir.path(),
            "core/src/lib.rs",
            "pub fn a() { let _y = 2; }\n",
        );
        let other = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("fingerprint");
        assert_ne!(dirty, other, "two different edits must not share a key");
    }

    #[test]
    fn untracked_runtime_inputs_change_the_fingerprint() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        commit_all(dir.path(), "init");

        let clean = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("clean fingerprint");

        write_runtime_file(dir.path(), "core/src/new.rs", "pub fn b() {}\n");
        let with_untracked = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("an untracked file still has a fingerprint");

        assert_ne!(clean, with_untracked, "a new file must not reuse the key");
        assert!(with_untracked.contains(":dirty:"), "got {with_untracked}");
    }

    #[test]
    fn reverting_an_edit_restores_the_clean_fingerprint() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");
        commit_all(dir.path(), "init");

        let clean = compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
            .expect("clean fingerprint");
        write_runtime_file(
            dir.path(),
            "core/src/lib.rs",
            "pub fn a() { let _x = 1; }\n",
        );
        write_runtime_file(dir.path(), "core/src/lib.rs", "pub fn a() {}\n");

        assert_eq!(
            clean,
            compute_runtime_fingerprint_sync(dir.path(), "waterui-core@0.0.1")
                .expect("fingerprint"),
            "a worktree back at its commit must reuse the commit's build"
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
