//! The Mac Catalyst destination — a `.app` staged on the host and
//! launched through `LaunchServices`.
//!
//! A Catalyst `UIApplication` only connects its window scene under a
//! real application launch: `UIApplicationMain` hands the process to
//! `-[NSApplication run]`, the `AppKit` event loop `UIKit`'s Mac-helper
//! machinery dispatches through. A bare exec never launches the
//! application — no scene connects and nothing activates
//! (water-rs/waterui#3358). A marked binary is therefore staged as a
//! macOS-layout `<binary>.app`, ad-hoc signed, and `open -n -W`'d; its
//! stdout/stderr reach the runner through the files `open`'s
//! `--stdout`/`--stderr` name, and the status comes from the file
//! `WATERUI_NATIVE_TEST_STATUS` names. A `--list` invocation or an
//! unmarked binary is exec'd directly — neither needs the application.

use std::io::Write;
use std::path::Path;

use eyre::{Context as _, bail, eyre};
use futures_util::future::Either;

use crate::apple::app_bundle::codesign_path;
use crate::toolchain::Host;

use super::lock::DestinationLock;
use super::status::{StatusFile, TestStatus, unique_name};
use super::{
    HarnessBundle, Invocation, MachOPlatform, STATUS_PATH_VARIABLE, TestDestination,
    pass_through_exit,
};

/// The Catalyst runner: the host Mac is the destination, so nothing is
/// resolved — a GUI session the launch lands in is the caller's
/// responsibility.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct MacCatalyst;

impl MacCatalyst {
    /// The runner for the host Mac.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl TestDestination for MacCatalyst {
    fn platform(&self) -> MachOPlatform {
        MachOPlatform::MACCATALYST
    }

    async fn run(
        &self,
        host: &Host,
        invocation: &Invocation,
        interrupts: smol::channel::Receiver<()>,
        stdout: &mut (impl Write + Send + ?Sized),
        stderr: &mut (impl Write + Send + ?Sized),
    ) -> eyre::Result<TestStatus> {
        let Invocation {
            binary,
            arguments,
            forwarded_env,
        } = invocation;
        binary.require_platform(self.platform())?;

        // The bare route: `--list`, or a binary with no application
        // manifest, is exec'd with its exit status passed through.
        if arguments.is_listing() || binary.manifest().is_none() {
            let mut command = host
                .command(binary.path())
                .wrap_err("failed to spawn the test binary")?;
            command.args(arguments.as_slice());
            for (key, value) in forwarded_env.pairs() {
                command.env(key, value);
            }
            let status = command
                .status()
                .await
                .wrap_err("failed to run the test binary")?;
            return Ok(pass_through_exit(status));
        }

        let manifest = binary
            .manifest()
            .expect("the app route only runs a marked binary");
        // The lock covers staging through the launch's end:
        // `HarnessBundle::stage` rebuilds `<binary>.app` in place, and
        // `open -W` holds the wait — a second run of the same binary
        // would re-stage and re-launch mid-flight.
        let home = host
            .home_dir()
            .ok_or_else(|| eyre!("cannot locate the user's home directory for the launch lock"))?;
        let _lock = DestinationLock::acquire(
            &home
                .join("Library/Caches/dev.waterui.test-runner")
                .join("catalyst.lock"),
        )
        .await?;

        let bundle = HarnessBundle::stage(binary, manifest, "macosx").await?;
        codesign_path(host, bundle.layout().app_path(), "-", None, None).await?;

        let state_dir = binary.state_dir()?;
        let status = StatusFile::host_absolute(&state_dir);
        // `open` relays nothing of the launched app's streams itself:
        // they go to the files `--stdout`/`--stderr` name and are
        // written back here once `open -W` returns.
        let out_path = state_dir.join(format!("{}.stdout", unique_name()));
        let err_path = state_dir.join(format!("{}.stderr", unique_name()));

        let mut command = host.command("open").wrap_err("failed to spawn open")?;
        command.args(["-n", "-W"]);
        command
            .arg("--env")
            .arg(format!("{STATUS_PATH_VARIABLE}={}", status.env_value()));
        for (key, value) in forwarded_env.pairs() {
            command.arg("--env").arg(format!("{key}={value}"));
        }
        command
            .arg("--stdout")
            .arg(&out_path)
            .arg("--stderr")
            .arg(&err_path)
            .arg(bundle.layout().app_path())
            .arg("--args")
            .args(arguments.as_slice());

        // `open -W` waits for the application to exit. An interrupt
        // cannot reach the app through LaunchServices — there is no
        // handle to it — so it ends only `open`'s wait; the harness
        // application always exits itself once its trials conclude.
        let mut child = command.spawn().wrap_err("failed to launch open")?;
        let open_status = {
            let wait = std::pin::pin!(child.status());
            let interrupt = std::pin::pin!(interrupts.recv());
            match futures_util::future::select(wait, interrupt).await {
                Either::Left((status, _)) => Some(status.wrap_err("failed to wait for `open`")?),
                Either::Right(_) => {
                    child
                        .kill()
                        .wrap_err("failed to stop `open` after an interrupt")?;
                    child
                        .status()
                        .await
                        .wrap_err("failed to reap `open` after an interrupt")?;
                    None
                }
            }
        };

        let Some(open_status) = open_status else {
            bail!(
                "the `open` wait for {} was interrupted before the harness reported a status",
                bundle.layout().app_path().display()
            );
        };
        // `open` creates both files before it launches the app, so a
        // missing one means the launch itself never happened.
        relay(&out_path, stdout, "stdout")?;
        relay(&err_path, stderr, "stderr")?;

        let path = status.local_path(&state_dir);
        let content = std::fs::read(&path).ok();
        let _ = std::fs::remove_file(&path);
        if content.is_none() && !open_status.success() {
            bail!(
                "`open -n -W` failed to launch {}: {open_status}",
                bundle.layout().app_path().display()
            );
        }
        StatusFile::conclude(content.as_deref(), bundle.bundle_id())
    }
}

/// Writes the capture file `open` filled for one of the app's streams to
/// the runner's matching stream, then removes it.
fn relay(path: &Path, sink: &mut (impl Write + Send + ?Sized), name: &str) -> eyre::Result<()> {
    let captured = std::fs::read(path)
        .wrap_err_with(|| format!("failed to read the app's {name} from {}", path.display()))?;
    sink.write_all(&captured)
        .wrap_err_with(|| format!("failed to relay the app's {name}"))?;
    std::fs::remove_file(path).wrap_err_with(|| format!("failed to remove {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::test_runner::binary::tests::{manifest_plist, stage_binary};
    use crate::apple::test_runner::{ForwardedEnv, TestArguments, TestBinary};
    use crate::toolchain::testing::TestMachine;

    #[test]
    fn the_bare_route_passes_the_spawned_exit_through() {
        let machine = TestMachine::new();
        let binary_path = stage_binary(machine.root(), MachOPlatform::MACCATALYST.0, None);
        let binary = TestBinary::read(&binary_path).expect("read fixture");
        // The exec route spawns the binary itself: replace the fixture
        // bytes with an exec'able fake tool at the same path.
        machine.executable("deps/harness-abc123");
        let log = machine.file("fake.log", "");
        let host = machine.host([
            ("WATERUI_FAKE_LOG", log.as_os_str()),
            ("WATERUI_FAKE_EXEC_STATUS", std::ffi::OsStr::new("17")),
        ]);
        let (_tx, interrupts) = smol::channel::unbounded();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = smol::block_on(MacCatalyst::new().run(
            &host,
            &Invocation {
                binary,
                arguments: TestArguments::new(vec!["suite::case".into()]),
                forwarded_env: ForwardedEnv::new([]),
            },
            interrupts,
            &mut stdout,
            &mut stderr,
        ))
        .expect("the exec route returns a status");
        assert_eq!(status.code(), 17);
        let log = std::fs::read_to_string(&log).expect("read log");
        assert!(
            log.contains("harness-abc123 suite::case"),
            "the binary itself ran: {log}"
        );
    }

    #[test]
    fn the_app_route_stages_signs_launches_and_reports_the_status() {
        let machine = TestMachine::new();
        machine.install("codesign");
        machine.install("open");
        let plist = manifest_plist();
        let binary_path = stage_binary(machine.root(), MachOPlatform::MACCATALYST.0, Some(&plist));
        let binary = TestBinary::read(&binary_path).expect("read fixture");
        let log = machine.file("fake.log", "");
        let host = machine.host([
            ("WATERUI_FAKE_LOG", log.as_os_str()),
            ("WATERUI_FAKE_OPEN_STATUS", std::ffi::OsStr::new("7")),
            (
                "WATERUI_FAKE_OPEN_STDOUT",
                std::ffi::OsStr::new("app says hi"),
            ),
        ]);
        let (_tx, interrupts) = smol::channel::unbounded();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = smol::block_on(MacCatalyst::new().run(
            &host,
            &Invocation {
                binary,
                arguments: TestArguments::new(vec!["suite::case".into(), "--exact".into()]),
                forwarded_env: ForwardedEnv::new([]),
            },
            interrupts,
            &mut stdout,
            &mut stderr,
        ))
        .expect("the app route returns the harness status");
        // The status comes from the file the launch wrote, not `open`'s.
        assert_eq!(status.code(), 7);

        // Staged as the macOS Contents layout: Info.plist is the
        // manifest verbatim and the executable is the binary itself
        // under its CFBundleExecutable name.
        let app = binary_path.with_extension("app");
        assert_eq!(
            std::fs::read(app.join("Contents/Info.plist")).expect("staged plist"),
            plist
        );
        assert_eq!(
            std::fs::read(app.join("Contents/MacOS/harness")).expect("staged exe"),
            std::fs::read(&binary_path).expect("fixture bytes")
        );

        let log = std::fs::read_to_string(&log).expect("read log");
        assert!(log.contains("codesign"), "the bundle was signed: {log}");
        assert!(
            log.contains("open -n -W"),
            "the app launched through open -n -W: {log}"
        );
        assert!(
            log.contains("--args suite::case --exact"),
            "the case arguments were forwarded: {log}"
        );
        // The app's stdout reaches the runner's stdout.
        assert_eq!(stdout, b"app says hi\n");
    }

    #[test]
    fn a_launch_without_a_status_is_an_error() {
        let machine = TestMachine::new();
        machine.install("codesign");
        machine.install("open");
        let plist = manifest_plist();
        let binary_path = stage_binary(machine.root(), MachOPlatform::MACCATALYST.0, Some(&plist));
        let binary = TestBinary::read(&binary_path).expect("read fixture");
        let host = machine.host([("WATERUI_FAKE_OPEN_NO_STATUS", std::ffi::OsStr::new("1"))]);
        let (_tx, interrupts) = smol::channel::unbounded();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let error = smol::block_on(MacCatalyst::new().run(
            &host,
            &Invocation {
                binary,
                arguments: TestArguments::new(vec!["suite::case".into()]),
                forwarded_env: ForwardedEnv::new([]),
            },
            interrupts,
            &mut stdout,
            &mut stderr,
        ))
        .expect_err("a missing status file must fail");
        assert!(
            error
                .to_string()
                .contains("died before its trials concluded"),
            "unexpected error: {error}"
        );
    }
}
