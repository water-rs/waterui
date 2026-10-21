//! Exercise the actual CLI build script in isolated warm Cargo/Git packages.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use waterui_cli::toolchain::Host;

struct Fixture {
    root: tempfile::TempDir,
    package: PathBuf,
}

fn run(command: &mut Command) -> Output {
    let output = command.output().expect("run CLI metadata fixture command");
    assert!(
        output.status.success(),
        "{command:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(directory: &Path) -> std::io::Result<Command> {
    let mut command = Host::current().std_command("git")?;
    command.arg("-C").arg(directory);
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
    ] {
        command.env_remove(variable);
    }
    Ok(command)
}

impl Fixture {
    fn new() -> std::io::Result<Self> {
        let root = tempfile::tempdir().expect("isolated CLI metadata fixture");
        let package = root.path().join("package");
        std::fs::create_dir_all(package.join("src")).expect("fixture source directory");
        for (path, source) in [
            (
                "Cargo.toml",
                include_str!("fixtures/cli_metadata/Cargo.toml"),
            ),
            ("Cargo.toml.orig", include_str!("../Cargo.toml")),
            ("src/lib.rs", include_str!("fixtures/cli_metadata/lib.rs")),
            ("build.rs", include_str!("../build.rs")),
            (".gitignore", "/target/\n"),
        ] {
            std::fs::write(package.join(path), source).expect("fixture input");
        }
        let fixture = Self { root, package };
        run(fixture.cargo()?.args(["generate-lockfile", "--offline"]));
        Ok(fixture)
    }

    fn cargo(&self) -> std::io::Result<Command> {
        let mut command = Host::current().std_command(env!("CARGO"))?;
        command
            .current_dir(&self.package)
            .env("CARGO_TARGET_DIR", self.package.join("target"));
        for variable in [
            "CARGO_BUILD_TARGET",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "WATERUI_CLI_COMMIT",
        ] {
            command.env_remove(variable);
        }
        Ok(command)
    }

    fn initialize(&self) -> std::io::Result<()> {
        run(git(&self.package)?.args(["init", "--template=", "-b", "metadata"]));
        let key = self.root.path().join("signing-key");
        run(Host::current()
            .std_command("ssh-keygen")?
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key));
        for (name, value) in [
            ("user.name", "CLI Metadata Fixture"),
            ("user.email", "cli-metadata@invalid.test"),
            ("gpg.format", "ssh"),
            ("gpg.ssh.program", "ssh-keygen"),
            ("commit.gpgsign", "true"),
        ] {
            run(git(&self.package)?.args(["config", name, value]));
        }
        let hooks = self.root.path().join("hooks");
        std::fs::create_dir(&hooks).expect("isolated hooks");
        run(git(&self.package)?.args(["config", "core.hooksPath"])
            .arg(hooks));
        run(git(&self.package)?.args(["config", "user.signingkey"])
            .arg(key));
        run(git(&self.package)?.args(["add", "."]));
        self.commit()
    }

    fn commit(&self) -> std::io::Result<()> {
        run(git(&self.package)?.args([
            "commit",
            "--allow-empty",
            "-S",
            "-m",
            "CLI metadata fixture",
        ]));
        Ok(())
    }

    fn identity(&self) -> std::io::Result<String> {
        let output = run(git(&self.package)?.args(["rev-parse", "HEAD"]));
        Ok(String::from_utf8(output.stdout)
            .expect("Git identity")
            .trim()
            .to_owned())
    }

    fn check(&self, expected: &str) -> std::io::Result<()> {
        run(self
            .cargo()?.args(["test", "--locked", "--offline"])
            .env("EXPECTED_CLI_COMMIT", expected));
        Ok(())
    }
}

#[test]
fn ordinary_branch_updates_refresh_cli_identity() -> std::io::Result<()> {
    let fixture = Fixture::new()?;
    fixture.initialize()?;
    fixture.check(&fixture.identity()?)?;
    fixture.commit()?;
    fixture.check(&fixture.identity()?)?;
    Ok(())
}

#[test]
fn linked_branch_updates_refresh_cli_identity() -> std::io::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.initialize()?;
    let linked = fixture.root.path().join("linked");
    run(git(&fixture.package)?.args(["worktree", "add", "-b", "linked"])
        .arg(&linked));
    fixture.package = linked;
    fixture.check(&fixture.identity()?)?;
    fixture.commit()?;
    fixture.check(&fixture.identity()?)?;
    Ok(())
}

#[test]
fn packed_ref_updates_refresh_cli_identity() -> std::io::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.initialize()?;
    let linked = fixture.root.path().join("linked");
    run(git(&fixture.package)?.args(["worktree", "add", "-b", "linked"])
        .arg(&linked));
    fixture.package = linked;
    run(git(&fixture.package)?.args(["pack-refs", "--all", "--prune"]));
    fixture.check(&fixture.identity()?)?;
    fixture.commit()?;
    run(git(&fixture.package)?.args(["pack-refs", "--all", "--prune"]));
    fixture.check(&fixture.identity()?)?;
    Ok(())
}

#[test]
fn detached_head_updates_refresh_cli_identity() -> std::io::Result<()> {
    let fixture = Fixture::new()?;
    fixture.initialize()?;
    run(git(&fixture.package)?.args(["checkout", "--detach"]));
    fixture.check(&fixture.identity()?)?;
    fixture.commit()?;
    fixture.check(&fixture.identity()?)?;
    Ok(())
}

#[test]
fn non_git_package_keeps_unknown_identity() -> std::io::Result<()> {
    let fixture = Fixture::new()?;
    fixture.check("unknown")?;
    Ok(())
}
