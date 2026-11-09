//! Toolchain support for Windows long paths — the machine-wide
//! `LongPathsEnabled` registry setting.
//!
//! `WaterUI` checkouts nest deep: the `build_cache` directory mirrors the
//! project's absolute path under `~/.water/build_cache/drive-<letter>/…`, the managed
//! Android host adds `managed_backends/hydrolysis/android-host/<sha>/` below
//! it, and Gradle's build directories sit below that — well past the
//! 260-character `MAX_PATH` limit. With `LongPathsEnabled = 0` git fails with
//! "Filename too long" and a cmd-builtin copy drops files silently
//! (water-rs/waterui#3125).
//!
//! The value is read and written through `reg`/`powershell` invocations on
//! the [`Host`] seam rather than the Win32 registry API, so a declared test
//! host can answer them like every other tool probe.

use std::process::Output;

use crate::{
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
};

/// The registry key holding `LongPathsEnabled`.
const FILE_SYSTEM_KEY: &str = r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem";

/// The machine setting opting Windows file APIs into paths past `MAX_PATH`.
const VALUE: &str = "LongPathsEnabled";

/// The `reg query` arguments that read the setting.
const QUERY_ARGS: &[&str] = &["query", FILE_SYSTEM_KEY, "/v", VALUE];

/// The elevated write the fix performs — `Start-Process -Verb RunAs` raises
/// the UAC prompt, since `reg add` under `HKLM` needs an administrator token
/// this process does not carry. `-Wait` returns only after the elevated
/// `reg` exits; a declined prompt makes `Start-Process` throw, failing the
/// run.
const ELEVATED_ENABLE: &str = "Start-Process reg.exe -Verb RunAs -Wait -ArgumentList \
     'add','HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem','/v','LongPathsEnabled','/t','REG_DWORD','/d','1','/f'";

/// The `LongPathsEnabled` setting on a Windows host.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsLongPaths;

/// `reg query` for `LongPathsEnabled`.
///
/// `Ok(false)` covers every "not enabled" answer — the value absent (`reg`
/// exits 1), or data other than `0x1`. `Err` is the probe itself failing:
/// a host without `reg.exe` cannot run the elevated write either.
async fn enabled(host: &Host) -> Result<bool, CommandError> {
    let output = host.output("reg", QUERY_ARGS).await?;
    Ok(enabled_in(&output))
}

/// Whether a `reg query` output reports `LongPathsEnabled REG_DWORD 0x1`.
fn enabled_in(output: &Output) -> bool {
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some(VALUE) && fields.last() == Some("0x1")
        })
}

impl Toolchain for WindowsLongPaths {
    type Installation = LongPathsInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        match enabled(host).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(ToolchainError::fixable(LongPathsInstallation)),
            Err(error) => Err(ToolchainError::unfixable(
                format!("could not query `{FILE_SYSTEM_KEY}\\{VALUE}`: {error}"),
                format!(
                    "Set `{VALUE}` (REG_DWORD) to 1 under `{FILE_SYSTEM_KEY}` from an elevated \
                     prompt: reg add \"{FILE_SYSTEM_KEY}\" /v {VALUE} /t REG_DWORD /d 1 /f"
                ),
            )),
        }
    }
}

/// Set `LongPathsEnabled = 1` through an elevated `reg add`.
///
/// A machine-wide write outside `~/.water`, so `doctor --fix` confirms it
/// first (or proceeds on `--yes`), and the UAC prompt is the user's
/// elevation consent. The value is queried again afterwards: a declined
/// prompt already fails the powershell run, and a write that did not land
/// is still an error rather than a reported success.
#[derive(Debug, Clone, Copy)]
pub struct LongPathsInstallation;

/// Errors that can occur while enabling Windows long paths.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FailToEnableLongPaths {
    /// The elevated `reg add` or the verifying query could not run.
    #[error(transparent)]
    Command(#[from] CommandError),

    /// The elevated write completed but the value is still not 1.
    #[error("the elevated `reg add` completed but `{VALUE}` is still not 1")]
    StillDisabled,
}

impl Installation for LongPathsInstallation {
    type Error = FailToEnableLongPaths;

    fn modifies_system(&self) -> bool {
        true
    }

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        host.run("powershell", ["-NoProfile", "-Command", ELEVATED_ENABLE])
            .await?;
        if enabled(host).await? {
            Ok(())
        } else {
            Err(FailToEnableLongPaths::StillDisabled)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LongPathsInstallation, WindowsLongPaths};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Installation, Toolchain, ToolchainError};

    /// A host with the fake `reg` answering queries for `value`, or for the
    /// absent value when `None`.
    fn host_with(machine: &TestMachine, value: Option<&str>) -> crate::toolchain::Host {
        machine.install("reg");
        value.map_or_else(
            || machine.host(Vec::<(&str, &str)>::new()),
            |value| machine.host([("WATERUI_FAKE_LONG_PATHS", value)]),
        )
    }

    #[test]
    fn ok_when_long_paths_enabled() {
        let machine = TestMachine::new();
        let host = host_with(&machine, Some("1"));
        smol::block_on(WindowsLongPaths.check(&host))
            .expect("LongPathsEnabled = 1 must satisfy the check");
    }

    #[test]
    fn disabled_or_absent_value_is_fixable() {
        for value in [Some("0"), None] {
            let machine = TestMachine::new();
            let host = host_with(&machine, value);
            let result = smol::block_on(WindowsLongPaths.check(&host));
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "a disabled or absent LongPathsEnabled must be fixable: {result:?}"
            );
        }
    }

    /// Without `reg` the query cannot run and the elevated write cannot be
    /// issued either — the item is a manual fix.
    #[test]
    fn unfixable_when_reg_cannot_run() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(&str, &str)>::new());
        let result = smol::block_on(WindowsLongPaths.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "no `reg` on PATH must be unfixable: {result:?}"
        );
    }

    /// The fix elevates `reg add` through the fake `powershell`, which lands
    /// the write in the state file the fake `reg` reads — the re-check then
    /// observes the repair, like the real `--fix` loop.
    #[test]
    fn fix_writes_the_value_through_elevation() {
        let machine = TestMachine::new();
        machine.install("powershell");
        let host = host_with(&machine, Some("0"));
        smol::block_on(LongPathsInstallation.install(&host))
            .expect("the elevated write must succeed on the fake machine");
        smol::block_on(WindowsLongPaths.check(&host))
            .expect("the re-check must observe LongPathsEnabled = 1");
    }

    /// Without `powershell` on PATH the elevated write cannot be issued at
    /// all — the install fails instead of claiming a fix it did not make.
    #[test]
    fn install_fails_without_powershell() {
        let machine = TestMachine::new();
        let host = host_with(&machine, Some("0"));
        let result = smol::block_on(LongPathsInstallation.install(&host));
        assert!(
            matches!(result, Err(super::FailToEnableLongPaths::Command(_))),
            "no powershell must fail the install: {result:?}"
        );
    }

    /// A declined elevation leaves the value unset — the post-install
    /// verification fails fast rather than reporting a fix that did not
    /// land, matching the winget post-install contract.
    #[test]
    fn install_fails_when_the_write_does_not_land() {
        let machine = TestMachine::new();
        machine.install("powershell");
        let host = host_with(&machine, Some("0")).with_env("WATERUI_FAKE_POWERSHELL_NO_WRITE", "1");
        let result = smol::block_on(LongPathsInstallation.install(&host));
        assert!(
            matches!(result, Err(super::FailToEnableLongPaths::StillDisabled)),
            "a value still 0 after the write must fail the install: {result:?}"
        );
    }
}
