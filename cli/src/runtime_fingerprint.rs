//! Runtime fingerprint computation shared by preview and inspector launchers.

use std::collections::HashSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{Context as _, Result};
use sha2::Digest as _;

use crate::runtime_compat::{is_preview_build_input_file, should_skip_scan_dir};

#[derive(Debug, Clone)]
struct RuntimeFingerprintFile {
    relative_path: PathBuf,
    absolute_path: PathBuf,
}

#[allow(clippy::redundant_pub_crate)]
pub(crate) async fn compute_runtime_fingerprint(
    waterui_root: &Path,
    waterui_core_id: &str,
) -> Result<String> {
    let waterui_root = waterui_root.to_path_buf();
    let waterui_core_id = waterui_core_id.to_string();
    smol::unblock(move || compute_runtime_fingerprint_sync(&waterui_root, &waterui_core_id)).await
}

fn compute_runtime_fingerprint_sync(waterui_root: &Path, waterui_core_id: &str) -> Result<String> {
    if let Some(git_fingerprint) = compute_git_clean_fingerprint(waterui_root, waterui_core_id)? {
        return Ok(git_fingerprint);
    }

    let mut files = Vec::new();
    if !collect_runtime_fingerprint_files_from_git(waterui_root, &mut files) {
        collect_runtime_fingerprint_files_from_fs(waterui_root, waterui_root, &mut files)?;
    }
    files.sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut hasher = sha2::Sha256::new();
    hasher.update(waterui_core_id.as_bytes());
    hasher.update([0]);

    for file in files {
        hasher.update(file.relative_path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hash_file_contents(&mut hasher, &file.absolute_path)?;
    }

    Ok(format!("{waterui_core_id}:{:x}", hasher.finalize()))
}

fn compute_git_clean_fingerprint(root: &Path, waterui_core_id: &str) -> Result<Option<String>> {
    if !is_git_work_tree(root) {
        return Ok(None);
    }

    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=normal")
        .output();
    let Ok(status) = status else {
        return Ok(None);
    };
    if !status.status.success() || !status.stdout.is_empty() {
        return Ok(None);
    }

    let head = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .output();
    let Ok(head) = head else {
        return Ok(None);
    };
    if !head.status.success() {
        return Ok(None);
    }

    let commit = String::from_utf8(head.stdout)
        .wrap_err("`git rev-parse HEAD` returned non-UTF8 output")?
        .trim()
        .to_string();
    if commit.is_empty() {
        return Ok(None);
    }

    Ok(Some(format!("{waterui_core_id}:git:{commit}")))
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

fn collect_runtime_fingerprint_files_from_git(
    root: &Path,
    files: &mut Vec<RuntimeFingerprintFile>,
) -> bool {
    if !is_git_work_tree(root) {
        return false;
    }

    let mut seen = HashSet::new();

    // Tracked files.
    if !collect_git_paths(
        root,
        &["ls-files", "--recurse-submodules", "-z"],
        files,
        &mut seen,
    ) {
        return false;
    }

    // Untracked files that are not ignored.
    if !collect_git_paths(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        files,
        &mut seen,
    ) {
        return false;
    }

    true
}

fn collect_git_paths(
    root: &Path,
    args: &[&str],
    files: &mut Vec<RuntimeFingerprintFile>,
    seen: &mut HashSet<PathBuf>,
) -> bool {
    let command = Command::new("git").arg("-C").arg(root).args(args).output();
    let Ok(output) = command else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    for entry in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let relative = PathBuf::from(String::from_utf8_lossy(entry).into_owned());
        if !seen.insert(relative.clone()) {
            continue;
        }
        let absolute = root.join(&relative);
        if !absolute.is_file() || !is_preview_build_input_file(&absolute) {
            continue;
        }
        files.push(RuntimeFingerprintFile {
            relative_path: relative,
            absolute_path: absolute,
        });
    }
    true
}

fn collect_runtime_fingerprint_files_from_fs(
    root: &Path,
    dir: &Path,
    files: &mut Vec<RuntimeFingerprintFile>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let file_name = entry.file_name();
            if should_skip_scan_dir(&file_name) {
                continue;
            }
            collect_runtime_fingerprint_files_from_fs(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() || !is_preview_build_input_file(&path) {
            continue;
        }

        let relative_path = path
            .strip_prefix(root)
            .wrap_err_with(|| {
                format!(
                    "Runtime fingerprint file was not under root: {}",
                    path.display()
                )
            })?
            .to_path_buf();

        files.push(RuntimeFingerprintFile {
            relative_path,
            absolute_path: path,
        });
    }
    Ok(())
}

fn hash_file_contents(hasher: &mut sha2::Sha256, path: &Path) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
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
        fs::create_dir_all(dir.path().join("src")).expect("create src");
        fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}").expect("write source");
        commit_all(dir.path(), "init");

        let fingerprint = compute_runtime_fingerprint_sync(dir.path(), "waterui-core 0.0.1")
            .expect("fingerprint");
        assert!(fingerprint.contains(":git:"));
    }

    #[test]
    fn falls_back_to_content_hash_when_tree_is_dirty() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        fs::create_dir_all(dir.path().join("src")).expect("create src");
        fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}").expect("write source");
        commit_all(dir.path(), "init");

        fs::write(dir.path().join("src/lib.rs"), "pub fn a() { let _x = 1; }")
            .expect("modify source");
        let fingerprint = compute_runtime_fingerprint_sync(dir.path(), "waterui-core 0.0.1")
            .expect("fingerprint");
        assert!(!fingerprint.contains(":git:"));
    }

    #[test]
    fn untracked_runtime_inputs_disable_git_fast_path() {
        let dir = tempdir().expect("temp dir");
        init_git_repo(dir.path());
        fs::create_dir_all(dir.path().join("src")).expect("create src");
        fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}").expect("write source");
        commit_all(dir.path(), "init");

        fs::write(dir.path().join("src/new.rs"), "pub fn b() {}").expect("write untracked source");
        let fingerprint = compute_runtime_fingerprint_sync(dir.path(), "waterui-core 0.0.1")
            .expect("fingerprint");
        assert!(!fingerprint.contains(":git:"));
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
