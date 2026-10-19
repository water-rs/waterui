//! Isolated integration tests for the protocol build script's commit
//! metadata resolution — each fixture builds a real Cargo package under an
//! ephemeral Git repository and asserts the embedded identity.

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
    fn new() -> Self {
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
        std::fs::write(package.join("build.rs"), include_str!("../build.rs"))
            .expect("actual protocol build script");
        std::fs::write(package.join(".gitignore"), "/target/\n").expect("fixture target exclusion");
        let probe = Self { root, package };
        successful(probe.cargo().args(["generate-lockfile", "--offline"]));
        probe
    }

    fn cargo(&self) -> Command {
        let mut command = Command::new(env!("CARGO"));
        command
            .current_dir(&self.package)
            .env("CARGO_TARGET_DIR", self.package.join("target"))
            .env_remove("CARGO_BUILD_TARGET")
            .env_remove("WATERUI_INSPECTOR_PROTOCOL_COMMIT")
            .env_remove("WATERUI_CLI_COMMIT")
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
        let output = successful(git(&self.package).args(["rev-parse", "--short=12", "HEAD"]));
        String::from_utf8(output.stdout)
            .expect("fixture commit")
            .trim()
            .to_owned()
    }

    fn check(&self, expected: &str) {
        successful(
            self.cargo()
                .args(["test", "--locked", "--offline"])
                .env("EXPECTED_PROTOCOL_COMMIT", expected),
        );
    }
}

#[test]
fn incremental_head_changes_refresh_protocol_identity() {
    let probe = Probe::new();
    probe.initialize_git();
    probe.check(&probe.head());
    probe.advance_head();
    probe.check(&probe.head());
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
    probe.check(&probe.head());
    probe.advance_head();
    probe.check(&probe.head());
}

#[test]
fn packed_ref_changes_refresh_protocol_identity() {
    let probe = Probe::new();
    probe.initialize_git();
    successful(git(&probe.package).args(["pack-refs", "--all", "--prune"]));
    probe.check(&probe.head());
    probe.advance_head();
    successful(git(&probe.package).args(["pack-refs", "--all", "--prune"]));
    probe.check(&probe.head());
    probe.advance_head();
    probe.check(&probe.head());
}

#[test]
fn detached_head_changes_refresh_protocol_identity() {
    let probe = Probe::new();
    probe.initialize_git();
    successful(git(&probe.package).args(["checkout", "--detach", "HEAD"]));
    probe.check(&probe.head());
    probe.advance_head();
    probe.check(&probe.head());
}

#[test]
fn explicit_protocol_identity_overrides_cli_identity() {
    let probe = Probe::new();
    successful(
        probe
            .cargo()
            .args(["test", "--locked", "--offline"])
            .env("WATERUI_INSPECTOR_PROTOCOL_COMMIT", "protocol-override")
            .env("WATERUI_CLI_COMMIT", "cli-override")
            .env("EXPECTED_PROTOCOL_COMMIT", "protocol-override"),
    );
}

#[test]
fn cli_identity_override_is_preserved() {
    let probe = Probe::new();
    successful(
        probe
            .cargo()
            .args(["test", "--locked", "--offline"])
            .env("WATERUI_CLI_COMMIT", "cli-override")
            .env("EXPECTED_PROTOCOL_COMMIT", "cli-override"),
    );
}

#[test]
fn non_git_package_preserves_unknown_identity() {
    Probe::new().check("unknown");
}
