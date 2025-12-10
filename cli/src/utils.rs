//! Utility functions for the CLI.

use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
};

use color_eyre::eyre;
use smol::{process::Command, unblock};

/// Locate an executable in the system's PATH.
///
/// Return the path to the executable if found.
///
/// # Errors
/// - If the executable is not found in the PATH.
pub(crate) async fn which(name: &'static str) -> Result<PathBuf, which::Error> {
    unblock(move || which::which(name)).await
}

/// Enable or disable standard output for command executions.
///
/// By default, standard output is disabled.
static STD_OUTPUT: AtomicBool = AtomicBool::new(false);

/// Enable or disable standard output for command executions.
pub fn set_std_output(enabled: bool) {
    STD_OUTPUT.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

pub(crate) fn command(command: &mut Command) -> &mut Command {
    command
        .kill_on_drop(true)
        .stdout(if STD_OUTPUT.load(Ordering::SeqCst) {
            Stdio::inherit()
        } else {
            Stdio::piped()
        })
        .stderr(if STD_OUTPUT.load(Ordering::SeqCst) {
            Stdio::inherit()
        } else {
            Stdio::piped()
        })
}

/// Run a command with the specified name and arguments.
///
/// Always captures output. When `STD_OUTPUT` is enabled, also prints to terminal.
///
/// Return the standard output as a `String` if successful.
/// # Errors
/// - If the command fails to execute or returns a non-zero exit status.
pub(crate) async fn run_command(
    name: &str,
    args: impl IntoIterator<Item = &str>,
) -> eyre::Result<String> {
    let result = Command::new(name)
        .args(args)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    // If STD_OUTPUT is enabled, also print to terminal
    if STD_OUTPUT.load(Ordering::SeqCst) {
        use std::io::Write;
        let _ = std::io::stdout().write_all(&result.stdout);
        let _ = std::io::stderr().write_all(&result.stderr);
    }

    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).to_string())
    } else {
        Err(eyre::eyre!(
            "Command {} failed with status {}",
            name,
            result.status
        ))
    }
}

/// Async file copy using reflink when available, falling back to regular copy.
///
/// This is more efficient than regular copy on filesystems that support reflinks (APFS, Btrfs).
///
/// # Errors
/// - If the copy operation fails.
pub async fn copy_file(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let from = from.as_ref().to_path_buf();
    let to = to.as_ref().to_path_buf();
    unblock(move || reflink::reflink_or_copy(from, to).map(|_| ())).await
}
