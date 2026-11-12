//! The Git commit a build script embeds, and the files Cargo watches so the
//! script reruns exactly when that commit can move.
//!
//! The embedded commit is a function of two things only: the worktree's
//! `HEAD` and, when `HEAD` names a branch, that branch's value. Every path
//! this crate prints for `cargo:rerun-if-changed` stands for one of them and
//! exists when it is printed — Cargo treats a watched path that is missing as
//! changed on every build, so a missing path would rerun the script, and
//! rebuild everything that depends on its crate, forever.
//!
//! Which file holds the branch value depends on the ref storage:
//!
//! - **Loose** (`refs/heads/<branch>` exists): that file is the value, and
//!   Git replaces it on every update, so it alone is watched. Packing deletes
//!   it; Cargo then reruns the script once, which watches the packed form.
//! - **Packed** (only `packed-refs` holds the value): `packed-refs` is shared
//!   by every ref and rewritten by fetches and packs, so it is not watched.
//!   The update that moves a packed branch writes a fresh loose file and
//!   appends to the branch's reflog (`logs/refs/heads/<branch>`), which Git
//!   appends to on every update once it exists and nothing else touches; the
//!   reflog is watched. With reflogs disabled the nearest existing directory
//!   above the loose path is watched instead: Cargo scans it recursively, so
//!   the loose file appearing inside it reruns the script.
//! - **Reftable** (`<common-dir>/reftable` exists): every ref lives in one
//!   stack per repository plus one per linked worktree, and each update
//!   rewrites the stack's `tables.list`. No file is narrower, so both stacks'
//!   `tables.list` are watched and any ref update in the repository reruns
//!   the script.
//!
//! `HEAD` itself is watched in the files backend: a checkout, a branch
//! switch and a commit on a detached `HEAD` each rewrite it.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Resolves the commit `HEAD` names in the Git checkout containing `dir`,
/// prints the `cargo:rerun-if-changed` lines that rerun the calling build
/// script whenever that commit can change, and returns the commit.
///
/// `abbreviation` selects the form: `None` is the full object name,
/// `Some(n)` the unique abbreviation of at least `n` hex digits
/// (`git rev-parse --short=<n>`).
///
/// Returns `None`, printing nothing, when `dir` is not inside a Git work tree
/// or `git` cannot be run — a packaged crate has no repository to read. On an
/// unborn branch the watches are printed and `None` is returned, so the first
/// commit reruns the script.
///
/// # Panics
///
/// Panics when Git reports a repository it then cannot describe: a failed
/// `git` invocation after the repository was located, or a reftable stack
/// without its `tables.list`.
#[must_use]
pub fn head_commit(dir: &Path, abbreviation: Option<u8>) -> Option<String> {
    let checkout = Checkout::locate(dir)?;
    for path in checkout.head_inputs() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    checkout.commit(abbreviation)
}

/// A Git work tree as `git rev-parse` locates it from a directory inside it.
struct Checkout {
    dir: PathBuf,
    /// This worktree's own Git directory, which holds its `HEAD`.
    git_dir: PathBuf,
    /// The directory every worktree of the repository shares, which holds
    /// branches and their reflogs.
    common_dir: PathBuf,
}

impl Checkout {
    fn locate(dir: &Path) -> Option<Self> {
        let output = git(dir)
            .args([
                "rev-parse",
                "--path-format=absolute",
                "--git-dir",
                "--git-common-dir",
            ])
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        let [git_dir, common_dir] = lines(&output, "rev-parse --git-dir --git-common-dir");
        Some(Self {
            dir: dir.to_path_buf(),
            git_dir: PathBuf::from(git_dir),
            common_dir: PathBuf::from(common_dir),
        })
    }

    /// The existing files whose change can move the commit `HEAD` resolves
    /// to, per the storage rules in the crate documentation.
    fn head_inputs(&self) -> Vec<PathBuf> {
        if self.common_dir.join("reftable").is_dir() {
            let mut stacks = vec![self.common_dir.join("reftable/tables.list")];
            if self.git_dir != self.common_dir {
                stacks.push(self.git_dir.join("reftable/tables.list"));
            }
            for stack in &stacks {
                assert!(
                    stack.is_file(),
                    "reftable repository at {} has no stack list {}",
                    self.common_dir.display(),
                    stack.display()
                );
            }
            return stacks;
        }

        let mut inputs = vec![self.git_dir.join("HEAD")];
        if let Some(branch) = self.head_branch() {
            let reflog = format!("logs/{branch}");
            let output = self.run(&[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                &branch,
                "--git-path",
                &reflog,
            ]);
            let [loose, reflog] = lines(&output, "rev-parse --git-path").map(PathBuf::from);
            inputs.push(if loose.is_file() {
                loose
            } else if reflog.is_file() {
                reflog
            } else {
                loose
                    .ancestors()
                    .skip(1)
                    .find(|directory| directory.is_dir())
                    .unwrap_or_else(|| {
                        panic!("no directory above the loose ref {}", loose.display())
                    })
                    .to_path_buf()
            });
        }
        inputs
    }

    /// The ref `HEAD` points at, or `None` for a detached `HEAD`.
    fn head_branch(&self) -> Option<String> {
        let output = git(&self.dir)
            .args(["symbolic-ref", "--quiet", "HEAD"])
            .output()
            .unwrap_or_else(|error| panic!("failed to run git symbolic-ref: {error}"));
        // `--quiet` makes a detached HEAD exit 1 with no output; any other
        // failure is a repository Git cannot read.
        match output.status.code() {
            Some(0) => {
                let [branch] = lines(&output, "symbolic-ref HEAD");
                Some(branch.to_owned())
            }
            Some(1) => None,
            _ => panic!(
                "git symbolic-ref HEAD failed in {}: {}",
                self.dir.display(),
                String::from_utf8_lossy(&output.stderr)
            ),
        }
    }

    fn commit(&self, abbreviation: Option<u8>) -> Option<String> {
        let short = abbreviation.map(|length| format!("--short={length}"));
        let output = git(&self.dir)
            .args(["rev-parse", "--verify", "--quiet"])
            .args(short.as_deref())
            .arg("HEAD")
            .output()
            .unwrap_or_else(|error| panic!("failed to run git rev-parse HEAD: {error}"));
        // `--verify --quiet` exits non-zero with no output on an unborn
        // branch, the one state in which HEAD names no commit.
        output.status.success().then(|| {
            let [commit] = lines(&output, "rev-parse HEAD");
            commit.to_owned()
        })
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = git(&self.dir)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {}: {error}", args.join(" ")));
        assert!(
            output.status.success(),
            "git {} failed in {}: {}",
            args.join(" "),
            self.dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

/// Exactly `N` lines of a Git command's output.
fn lines<'output, const N: usize>(output: &'output Output, command: &str) -> [&'output str; N] {
    let stdout = std::str::from_utf8(&output.stdout)
        .unwrap_or_else(|error| panic!("git {command} printed non-UTF-8 output: {error}"));
    let lines: Vec<&str> = stdout.lines().collect();
    lines.try_into().unwrap_or_else(|lines: Vec<&str>| {
        panic!(
            "git {command} printed {} lines, expected {N}: {stdout:?}",
            lines.len()
        )
    })
}
