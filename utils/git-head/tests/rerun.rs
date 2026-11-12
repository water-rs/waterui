//! Builds a real Cargo package whose build script embeds the commit through
//! `waterui-git-head`, inside an ephemeral Git repository, and checks after
//! each Git operation both whether Cargo reran the script and what it
//! embedded.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// The branch every fixture repository starts on.
const BRANCH: &str = "probe";

struct Probe {
    root: tempfile::TempDir,
    /// The worktree the package is built from.
    package: PathBuf,
    /// Build-script runs recorded so far.
    runs: Cell<usize>,
}

fn successful(command: &mut Command) -> Output {
    let output = command.output().expect("run probe command");
    assert!(
        output.status.success(),
        "{command:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(directory: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(directory);
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
    ] {
        command.env_remove(variable);
    }
    command
}

impl Probe {
    /// A probe package in a directory that is not a Git repository.
    fn new() -> Self {
        let root = tempfile::tempdir().expect("isolated probe root");
        let package = root.path().join("package");
        std::fs::create_dir_all(package.join("src")).expect("probe source directory");
        for (path, source) in [
            ("Cargo.toml", include_str!("fixtures/probe/Cargo.toml")),
            ("build.rs", include_str!("fixtures/probe/build.rs")),
            ("src/lib.rs", include_str!("fixtures/probe/lib.rs")),
        ] {
            std::fs::write(package.join(path), source).expect("probe input");
        }
        let probe = Self {
            root,
            package,
            runs: Cell::new(0),
        };
        successful(
            probe
                .cargo()
                .args(["add", "--offline", "--build", "--path"])
                .arg(env!("CARGO_MANIFEST_DIR")),
        );
        probe
    }

    /// A probe package committed to a fresh repository using the files ref
    /// backend.
    fn repository() -> Self {
        let probe = Self::new();
        probe.initialize(&[]);
        probe.commit();
        probe
    }

    fn initialize(&self, init: &[&str]) {
        successful(
            self.git()
                .args(["init", "--template=", "-b", BRANCH])
                .args(init),
        );
        let key = self.root.path().join("signing-key");
        successful(
            Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(&key),
        );
        let hooks = self.root.path().join("hooks");
        std::fs::create_dir(&hooks).expect("isolated probe hooks");
        for (name, value) in [
            ("user.name", "Git Head Probe"),
            ("user.email", "git-head-probe@invalid.test"),
            ("gpg.format", "ssh"),
            ("gpg.ssh.program", "ssh-keygen"),
            ("commit.gpgsign", "true"),
            ("core.hooksPath", path_str(&hooks)),
            ("user.signingkey", path_str(&key)),
        ] {
            successful(self.git().args(["config", name, value]));
        }
        successful(self.git().args(["add", "."]));
    }

    fn cargo(&self) -> Command {
        let mut command = Command::new(env!("CARGO"));
        command
            .current_dir(&self.package)
            .env("CARGO_TARGET_DIR", self.root.path().join("target"))
            .env("GIT_HEAD_PROBE_RUNS", self.root.path().join("runs"))
            .env_remove("CARGO_BUILD_TARGET");
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
        ] {
            command.env_remove(variable);
        }
        command
    }

    fn git(&self) -> Command {
        git(&self.package)
    }

    fn commit(&self) {
        successful(
            self.git()
                .args(["commit", "--allow-empty", "-S", "-m", "Probe commit"]),
        );
    }

    fn head(&self) -> String {
        let output = successful(self.git().args(["rev-parse", "HEAD"]));
        String::from_utf8(output.stdout)
            .expect("probe commit")
            .trim()
            .to_owned()
    }

    /// Packs every ref, deleting the loose files.
    fn pack(&self) {
        successful(self.git().args(["pack-refs", "--all", "--prune"]));
    }

    /// What a fetch does to the ref store: a remote-tracking ref changes and
    /// `packed-refs` is rewritten, while the checked-out branch's loose file,
    /// if any, is left alone.
    fn fetch(&self) {
        successful(
            self.git()
                .args(["update-ref", "refs/remotes/origin/probe"])
                .arg(self.head()),
        );
        successful(self.git().args(["pack-refs", "--all", "--no-prune"]));
    }

    /// Builds the package and returns the commits every build-script run so
    /// far embedded.
    fn build(&self) -> Vec<String> {
        successful(self.cargo().args(["build", "--offline"]));
        std::fs::read_to_string(self.root.path().join("runs"))
            .expect("the probe build script ran at least once")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[track_caller]
    fn expect_rerun(&self, commit: &str) {
        let runs = self.build();
        assert_eq!(
            runs.len(),
            self.runs.get() + 1,
            "the build script did not rerun: {runs:?}"
        );
        assert_eq!(runs.last().map(String::as_str), Some(commit));
        self.runs.set(runs.len());
    }

    #[track_caller]
    fn expect_fresh(&self) {
        let runs = self.build();
        assert_eq!(
            runs.len(),
            self.runs.get(),
            "the build script reran: {runs:?}"
        );
    }
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("UTF-8 temporary path")
}

#[test]
fn loose_branch_reruns_only_for_its_own_commits() {
    let probe = Probe::repository();
    probe.expect_rerun(&probe.head());
    probe.expect_fresh();
    probe.fetch();
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
    probe.expect_fresh();
}

#[test]
fn packed_branch_stays_fresh_and_reruns_for_its_commits() {
    let probe = Probe::repository();
    probe.pack();
    probe.expect_rerun(&probe.head());
    probe.expect_fresh();
    probe.fetch();
    probe.expect_fresh();
    // The commit writes the branch loose again.
    probe.commit();
    probe.expect_rerun(&probe.head());
    probe.expect_fresh();
    // Packing deletes the watched loose file: one rerun, same commit.
    probe.pack();
    probe.expect_rerun(&probe.head());
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
}

#[test]
fn packed_branch_without_reflog_reruns_for_its_commits() {
    let probe = Probe::new();
    probe.initialize(&[]);
    successful(
        probe
            .git()
            .args(["config", "core.logAllRefUpdates", "false"]),
    );
    probe.commit();
    probe.pack();
    let reflog = probe.package.join(".git/logs/refs/heads").join(BRANCH);
    assert!(
        !reflog.exists(),
        "core.logAllRefUpdates=false still wrote {}",
        reflog.display()
    );
    probe.expect_rerun(&probe.head());
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
    probe.expect_fresh();
}

#[test]
fn checkouts_rerun_with_the_checked_out_commit() {
    let probe = Probe::repository();
    let first = probe.head();
    probe.commit();
    let second = probe.head();
    probe.expect_rerun(&second);
    successful(probe.git().args(["checkout", "--detach", &first]));
    probe.expect_rerun(&first);
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
    successful(probe.git().args(["switch", BRANCH]));
    probe.expect_rerun(&second);
    successful(probe.git().args(["switch", "-c", "other", &first]));
    probe.expect_rerun(&first);
    probe.expect_fresh();
}

#[test]
fn linked_worktree_ignores_other_worktrees() {
    let mut probe = Probe::repository();
    let main = probe.package.clone();
    let linked = probe.root.path().join("linked");
    successful(
        probe
            .git()
            .args(["worktree", "add", "-b", "linked"])
            .arg(&linked),
    );
    probe.package = linked;
    probe.expect_rerun(&probe.head());
    successful(git(&main).args(["commit", "--allow-empty", "-S", "-m", "Main commit"]));
    probe.fetch();
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
    probe.pack();
    probe.expect_rerun(&probe.head());
    successful(git(&main).args(["commit", "--allow-empty", "-S", "-m", "Main commit"]));
    probe.fetch();
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
}

#[test]
fn reftable_reruns_for_commits_and_checkouts() {
    let probe = Probe::new();
    probe.initialize(&["--ref-format=reftable"]);
    probe.commit();
    let first = probe.head();
    probe.expect_rerun(&first);
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
    successful(probe.git().args(["checkout", "--detach", &first]));
    probe.expect_rerun(&first);
}

#[test]
fn unborn_branch_reruns_for_its_first_commit() {
    let probe = Probe::new();
    probe.initialize(&[]);
    probe.expect_rerun("unknown");
    probe.expect_fresh();
    probe.commit();
    probe.expect_rerun(&probe.head());
}

#[test]
fn package_outside_git_embeds_nothing() {
    let probe = Probe::new();
    probe.expect_rerun("unknown");
    probe.expect_fresh();
}
