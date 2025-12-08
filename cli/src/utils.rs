//! Utility functions for the CLI.

use std::{
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
};

use color_eyre::eyre;
use smol::{process::Command, stream::Stream, unblock};

/// Locate an executable in the system's PATH.
///
/// Return the path to the executable if found.
///
/// # Errors
/// - If the executable is not found in the PATH.
pub(crate) async fn which(name: &'static str) -> Result<PathBuf, which::Error> {
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
pub(crate) async fn run_command(
    name: &str,
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

pub(crate) fn run_command_streamly<'a>(
    name: &'static str,
    args: impl IntoIterator<Item = &'a str>,
) -> impl Stream<Item = eyre::Result<String>> + 'a {
    let mut command = Command::new(name);
    command.args(args).kill_on_drop(true);

    let stdout = if STD_OUTPUT.load(Ordering::SeqCst) {
        Stdio::inherit()
    } else {
        Stdio::piped()
    };

    let stderr = if STD_OUTPUT.load(Ordering::SeqCst) {
        Stdio::inherit()
    } else {
        Stdio::piped()
    };

    command.stdout(stdout).stderr(stderr);

    smol::stream::unfold(command, move |mut cmd| async move {
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Some((
                    Err(eyre::eyre!("Failed to spawn command {}: {}", name, e)),
                    cmd,
                ));
            }
        };

        let output = match child.output().await {
            Ok(output) => output,
            Err(e) => {
                return Some((
                    Err(eyre::eyre!("Failed to run command {}: {}", name, e)),
                    cmd,
                ));
            }
        };

        if output.status.success() {
            Some((Ok(String::from_utf8_lossy(&output.stdout).to_string()), cmd))
        } else {
            Some((
                Err(eyre::eyre!(
                    "Command {} failed with status {}",
                    name,
                    output.status
                )),
                cmd,
            ))
        }
    })
}
