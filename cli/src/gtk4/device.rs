//! GTK4 device implementation for running on the local machine.

use color_eyre::eyre::{self, eyre};
use smol::{
    io::{AsyncBufReadExt, BufReader},
    process::{Command, Stdio},
    spawn,
    stream::StreamExt,
};
use tracing::info;

use crate::{
    device::{Artifact, Device, DeviceEvent, FailToRun, Running, RunOptions},
    utils::command,
};

/// GTK4 device representing the local machine.
///
/// GTK4 apps run directly on the host machine without emulation or simulation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gtk4Device;

impl Device for Gtk4Device {
    fn name(&self) -> &str {
        "Local Machine (GTK4)"
    }

    async fn launch(&self) -> eyre::Result<()> {
        // No-op - local machine is always "launched"
        Ok(())
    }

    async fn run(
        &self,
        artifact: Artifact,
        options: RunOptions,
    ) -> Result<Running, FailToRun> {
        let binary_path = artifact.path();

        // Verify the binary exists and is executable
        if !binary_path.exists() {
            return Err(FailToRun::InvalidArtifact);
        }

        info!("Launching GTK4 app: {}", binary_path.display());

        // Build the command to run the GTK4 binary
        let mut cmd = Command::new(binary_path);
        command(&mut cmd);

        // Set environment variables
        for (key, value) in options.env_vars() {
            cmd.env(key, value);
        }

        // Capture stdout/stderr for logging
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        // Spawn the GTK app process
        let mut child = cmd
            .spawn()
            .map_err(|e| FailToRun::Launch(eyre!("Failed to launch GTK app: {e}")))?;

        let _pid = child.id();

        // Create Running instance
        let (running, sender) = Running::new(move || {
            // Process will be killed on drop due to kill_on_drop(true)
        });

        // Capture stdout
        if let Some(stdout) = child.stdout.take() {
            let stdout_sender = sender.clone();
            spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Some(result) = lines.next().await {
                    let Ok(line) = result else {
                        break;
                    };
                    // Parse log level from the line if possible
                    let level = parse_log_level(&line);
                    if stdout_sender
                        .try_send(DeviceEvent::Log {
                            level,
                            message: line,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        // Capture stderr
        if let Some(stderr) = child.stderr.take() {
            let stderr_sender = sender.clone();
            spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Some(result) = lines.next().await {
                    let Ok(line) = result else {
                        break;
                    };
                    if stderr_sender
                        .try_send(DeviceEvent::Stderr { message: line })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        // Monitor the process for exit
        let exit_sender = sender;
        spawn(async move {
            let status = child.status().await;

            match status {
                Ok(exit_status) => {
                    if exit_status.success() {
                        let _ = exit_sender.try_send(DeviceEvent::Exited);
                    } else {
                        // Check for common crash signals
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            if let Some(signal) = exit_status.signal() {
                                let crash_msg = match signal {
                                    6 => "Process aborted (SIGABRT)".to_string(),
                                    11 => "Segmentation fault (SIGSEGV)".to_string(),
                                    _ => format!("Terminated by signal {signal}"),
                                };
                                let _ = exit_sender.try_send(DeviceEvent::Crashed(crash_msg));
                                return;
                            }
                        }

                        let code = exit_status.code().unwrap_or(-1);
                        let _ = exit_sender
                            .try_send(DeviceEvent::Crashed(format!("Exit code: {code}")));
                    }
                }
                Err(e) => {
                    let _ = exit_sender.try_send(DeviceEvent::Crashed(format!("Process error: {e}")));
                }
            }
        })
        .detach();

        Ok(running)
    }

    async fn scan() -> eyre::Result<Vec<Self>> {
        // GTK4 device is the local machine - always available
        Ok(vec![Self])
    }
}

/// Parse log level from a line of output.
///
/// Attempts to detect tracing-style log levels in the output.
fn parse_log_level(line: &str) -> tracing::Level {
    let line_lower = line.to_lowercase();

    if line_lower.contains("error") || line_lower.contains("fatal") || line_lower.contains("panic")
    {
        tracing::Level::ERROR
    } else if line_lower.contains("warn") {
        tracing::Level::WARN
    } else if line_lower.contains("debug") {
        tracing::Level::DEBUG
    } else if line_lower.contains("trace") {
        tracing::Level::TRACE
    } else {
        tracing::Level::INFO
    }
}
