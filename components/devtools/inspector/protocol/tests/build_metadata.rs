//! Isolated integration tests for the protocol build script's identity
//! resolution — each fixture builds a real Cargo package under an ephemeral
//! Git repository (or a release marker) and asserts the embedded identity.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Probe {
    root: tempfile::TempDir,
    package: PathBuf,
}

fn successful(command: &mut Command) -> Output {
    let output = command.output().expect("run metadata fixture command");
    assert!(
        output.status.success(),
        "{command:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn failing(command: &mut Command) -> Output {
    let output = command.output().expect("run metadata fixture command");
    assert!(
        !output.status.success(),
        "{command:?} unexpectedly succeeded\nstdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

fn git(path: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    command
}

impl Probe {
    fn with_build_script(build_script: &str) -> Self {
        let root = tempfile::tempdir().expect("isolated metadata fixture");
        let package = root.path().join("package");
        std::fs::create_dir_all(package.join("src")).expect("fixture source directory");
        std::fs::write(
            package.join("Cargo.toml"),
            include_str!("fixtures/metadata/Cargo.toml"),
        )
        .expect("fixture manifest");
        std::fs::write(
            package.join("src/lib.rs"),
            include_str!("fixtures/metadata/lib.rs"),
        )
        .expect("fixture library");
        std::fs::write(package.join("build.rs"), build_script)
            .expect("actual protocol build script");
        std::fs::write(package.join(".gitignore"), "/target/\n").expect("fixture target exclusion");
        let probe = Self { root, package };
        successful(probe.cargo().args(["generate-lockfile", "--offline"]));
        probe
    }

    fn new() -> Self {
        Self::with_build_script(include_str!("../build.rs"))
    }

    /// A fixture whose build script resolves HEAD but emits no Git-path
    /// `rerun-if-changed` watch — the only honest way to prove the oracle
    /// discriminates a stale build rather than always agreeing.
    fn new_without_rerun_watches() -> Self {
        let source = include_str!("../build.rs");
        let stripped = source.replace(
            "println!(\"cargo:rerun-if-changed={}\", path.display());",
            "",
        );
        assert_ne!(stripped, source, "the watch emission was not found");
        Self::with_build_script(&stripped)
    }

    fn cargo(&self) -> Command {
        let mut command = Command::new(env!("CARGO"));
        command
            .current_dir(&self.package)
            .env("CARGO_TARGET_DIR", self.package.join("target"))
            .env_remove("CARGO_BUILD_TARGET")
            .env_remove("WATERUI_INSPECTOR_PROTOCOL_COMMIT")
            .env_remove("WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE")
            .env_remove("WATERUI_CLI_COMMIT")
            .env_remove("EXPECTED_PROTOCOL_COMMIT")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");
        command
    }

    fn initialize_git(&self) {
        successful(git(&self.package).args(["init", "--template=", "-b", "metadata"]));
        let key = self.root.path().join("signing-key");
        successful(
            Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(&key),
        );
        for (name, value) in [
            ("user.name", "Metadata Fixture"),
            ("user.email", "metadata-fixture@invalid.test"),
            ("gpg.format", "ssh"),
            ("gpg.ssh.program", "ssh-keygen"),
            ("commit.gpgsign", "true"),
        ] {
            successful(git(&self.package).args(["config", name, value]));
        }
        let hooks = self.root.path().join("hooks");
        std::fs::create_dir(&hooks).expect("isolated fixture hooks");
        successful(
            git(&self.package)
                .args(["config", "core.hooksPath"])
                .arg(&hooks),
        );
        successful(
            git(&self.package)
                .args(["config", "user.signingkey"])
                .arg(&key),
        );
        successful(git(&self.package).args(["add", "."]));
        self.advance_head();
    }

    fn advance_head(&self) {
        successful(git(&self.package).args([
            "commit",
            "--allow-empty",
            "-S",
            "-m",
            "Metadata fixture",
        ]));
    }

    fn head(&self) -> String {
        let output = successful(git(&self.package).args(["rev-parse", "HEAD"]));
        String::from_utf8(output.stdout)
            .expect("fixture commit")
            .trim()
            .to_owned()
    }

    /// A successful build asserts the expected checkout commit — `None` marks
    /// the release expectation, where the variable must stay unset.
    fn check(&self, expected: Option<&str>) {
        let mut command = self.cargo();
        command.args(["test", "--locked", "--offline"]);
        if let Some(commit) = expected {
            command.env("EXPECTED_PROTOCOL_COMMIT", commit);
        }
        successful(&mut command);
    }

    /// Runs `cargo test` expecting a build failure.
    fn check_fails(&self) -> Command {
        let mut command = self.cargo();
        command.args(["test", "--locked", "--offline"]);
        command
    }

    /// Marks the fixture as a published package the way `cargo package` does.
    fn mark_release(&self) {
        std::fs::write(
            self.package.join(".cargo_vcs_info.json"),
            "{\"sha1\": null}\n",
        )
        .expect("release metadata marker");
    }
}

#[test]
fn incremental_head_changes_refresh_protocol_identity() {
    let probe = Probe::new();
    probe.initialize_git();
    probe.check(Some(&probe.head()));
    probe.advance_head();
    probe.check(Some(&probe.head()));
}

#[test]
fn linked_worktree_head_changes_refresh_protocol_identity() {
    let mut probe = Probe::new();
    probe.initialize_git();
    let linked = probe.root.path().join("linked");
    successful(
        git(&probe.package)
            .args(["worktree", "add", "-b", "linked"])
            .arg(&linked),
    );
    probe.package = linked;
    probe.check(Some(&probe.head()));
    probe.advance_head();
    probe.check(Some(&probe.head()));
}

#[test]
fn packed_ref_changes_refresh_protocol_identity() {
    let probe = Probe::new();
    probe.initialize_git();
    successful(git(&probe.package).args(["pack-refs", "--all", "--prune"]));
    probe.check(Some(&probe.head()));
    probe.advance_head();
    successful(git(&probe.package).args(["pack-refs", "--all", "--prune"]));
    probe.check(Some(&probe.head()));
    probe.advance_head();
    probe.check(Some(&probe.head()));
}

#[test]
fn detached_head_changes_refresh_protocol_identity() {
    let probe = Probe::new();
    probe.initialize_git();
    successful(git(&probe.package).args(["checkout", "--detach", "HEAD"]));
    probe.check(Some(&probe.head()));
    probe.advance_head();
    probe.check(Some(&probe.head()));
}

#[test]
fn release_marker_builds_without_a_commit() {
    let probe = Probe::new();
    probe.mark_release();
    probe.check(None);
}

#[test]
fn explicit_release_mode_builds_without_a_commit() {
    let probe = Probe::new();
    probe.initialize_git();
    successful(
        probe
            .cargo()
            .args(["test", "--locked", "--offline"])
            .env("WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE", "release"),
    );
}

#[test]
fn an_invalid_build_mode_fails() {
    let probe = Probe::new();
    let output = failing(
        probe
            .check_fails()
            .env("WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE", "bogus"),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release or checkout"), "{stderr}");
}

#[test]
fn an_invalid_commit_override_fails() {
    let probe = Probe::new();
    probe.initialize_git();
    for invalid in [
        "unknown",
        "",
        &probe.head()[..12],
        "not-a-real-commit-string",
    ] {
        let output = failing(
            probe
                .check_fails()
                .env("WATERUI_INSPECTOR_PROTOCOL_COMMIT", invalid),
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("full Git commit"), "{invalid:?}: {stderr}");
    }
}

#[test]
fn a_release_build_rejects_a_commit_override() {
    let probe = Probe::new();
    probe.mark_release();
    let output = failing(
        probe
            .check_fails()
            .env("WATERUI_INSPECTOR_PROTOCOL_COMMIT", "0".repeat(40)),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("must not override"), "{stderr}");
}

#[test]
fn a_checkout_without_git_fails() {
    let probe = Probe::new();
    let output = failing(&mut probe.check_fails());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires a Git checkout"), "{stderr}");
}

#[test]
fn a_valid_commit_override_is_accepted() {
    let probe = Probe::new();
    let probe_commit = "a".repeat(40);
    successful(
        probe
            .cargo()
            .args(["test", "--locked", "--offline"])
            .env("WATERUI_INSPECTOR_PROTOCOL_COMMIT", &probe_commit)
            .env("EXPECTED_PROTOCOL_COMMIT", &probe_commit),
    );
}

#[test]
fn release_then_checkout_transitions_restamp_the_identity() {
    // One package, one target directory: the stamp must track the build mode
    // each time instead of caching the first resolution.
    let probe = Probe::new();
    probe.initialize_git();
    let head = probe.head();
    probe.check(Some(&head));
    successful(
        probe
            .cargo()
            .args(["test", "--locked", "--offline"])
            .env("WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE", "release"),
    );
    probe.check(Some(&head));
}

#[test]
fn without_git_rerun_watches_the_oracle_detects_a_stale_build() {
    // Deliberately broken fixture: with the Git watch stripped, advancing HEAD
    // keeps the stamp stale and the second check must FAIL — proving the
    // incremental cases above discriminate rather than always agreeing.
    let probe = Probe::new_without_rerun_watches();
    probe.initialize_git();
    probe.check(Some(&probe.head()));
    probe.advance_head();
    failing(
        probe
            .cargo()
            .args(["test", "--locked", "--offline"])
            .env("EXPECTED_PROTOCOL_COMMIT", probe.head()),
    );
}

#[test]
fn cli_identity_override_is_ignored() {
    let probe = Probe::new();
    probe.initialize_git();
    successful(
        probe
            .cargo()
            .args(["test", "--locked", "--offline"])
            .env("WATERUI_CLI_COMMIT", "cli-override")
            .env("EXPECTED_PROTOCOL_COMMIT", probe.head()),
    );
}
