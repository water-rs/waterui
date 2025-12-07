use std::{
    path::PathBuf,
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
pub async fn which(name: &'static str) -> Result<PathBuf, which::Error> {
    unblock(move || which::which(name)).await
}

static STD_OUTPUT: AtomicBool = AtomicBool::new(true);

/// Enable or disable standard output for command executions.
pub fn set_std_output(enabled: bool) {
    STD_OUTPUT.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

/// Run a command with the specified name and arguments.
///
///
/// Return the standard output as a `String` if successful.
/// # Errors
/// - If the command fails to execute or returns a non-zero exit status.
pub async fn run_command(
    name: &'static str,
    args: impl IntoIterator<Item = &str>,
) -> eyre::Result<String> {
    let result = Command::new(name)
        .args(args)
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
        .output()
        .await?;

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
