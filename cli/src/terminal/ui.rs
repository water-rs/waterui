//! User-friendly terminal UI utilities for human-readable output.
//!
//! This module provides utilities for printing colorful, formatted messages
//! to the terminal when in human output mode. It respects the global output
//! format and suppresses output when in JSON mode.

use console::style;
use std::{
    io::{self, Write},
    sync::mpsc,
    thread,
    time::Duration,
};
use waterui_cli::output;

/// Print a success message with a green checkmark.
///
/// Suppressed in JSON output mode.
pub fn success(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{} {}", style("✓").green().bold(), message.as_ref());
}

/// Print an informational message with a blue bullet.
///
/// Suppressed in JSON output mode.
pub fn info(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{} {}", style("•").blue(), message.as_ref());
}

/// Print a warning message with a yellow warning symbol.
///
/// Suppressed in JSON output mode.
pub fn warning(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{} {}", style("⚠").yellow().bold(), message.as_ref());
}

/// Print a step or action message with dimmed styling.
///
/// Suppressed in JSON output mode.
pub fn step(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{} {}", style("→").cyan(), style(message.as_ref()).dim());
}

/// Print a key-value pair with consistent formatting.
///
/// Suppressed in JSON output mode.
pub fn kv(key: impl AsRef<str>, value: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!(
        "  {} {}",
        style(format!("{}:", key.as_ref())).bold(),
        value.as_ref()
    );
}

/// Print a section header with emphasis.
///
/// Suppressed in JSON output mode.
pub fn section(title: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("\n{}", style(title.as_ref()).bold().underlined());
}

/// Print a plain message without any styling.
///
/// Suppressed in JSON output mode.
pub fn plain(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{}", message.as_ref());
}

/// Print an empty line for spacing.
///
/// Suppressed in JSON output mode.
pub fn newline() {
    if output::global_output_format().is_json() {
        return;
    }
    println!();
}

/// Print an error/panic message with red styling.
///
/// Suppressed in JSON output mode.
pub fn error(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{} {}", style("✗").red().bold(), style(message.as_ref()).red().bold());
}

/// Print a dimmed/muted line (for less important info like frame numbers).
///
/// Suppressed in JSON output mode.
pub fn dimmed(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{}", style(message.as_ref()).dim());
}

/// Print a hint message with cyan styling.
///
/// Suppressed in JSON output mode.
pub fn hint(message: impl AsRef<str>) {
    if output::global_output_format().is_json() {
        return;
    }
    println!("{} {}", style("💡").cyan(), style(message.as_ref()).cyan());
}

const SPINNER_FRAMES: &[&str] = &["-", "\\", "|", "/"];

/// Handle for a background spinner animating in the terminal.
pub struct SpinnerGuard {
    stop: Option<mpsc::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SpinnerGuard {
    fn start(message: String) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut stdout = io::stdout();
            let mut index = 0;
            loop {
                if rx.try_recv().is_ok() {
                    let _ = write!(stdout, "\r\x1b[K");
                    let _ = stdout.flush();
                    break;
                }
                let frame = SPINNER_FRAMES[index % SPINNER_FRAMES.len()];
                let _ = write!(stdout, "\r{frame} {message}");
                let _ = stdout.flush();
                index = (index + 1) % SPINNER_FRAMES.len();
                thread::sleep(Duration::from_millis(120));
            }
        });
        Self {
            stop: Some(tx),
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    /// Finish the spinner animation and move to the next line.
    pub fn finish(mut self) {
        self.stop();
        println!();
    }
}

impl Drop for SpinnerGuard {
    fn drop(&mut self) {
        if self.stop.is_some() {
            self.stop();
        }
    }
}

/// Start a spinner animation with the provided message.
///
/// Returns `None` when JSON output is enabled so automated consumers stay quiet.
pub fn spinner(message: impl Into<String>) -> Option<SpinnerGuard> {
    if output::global_output_format().is_json() {
        return None;
    }
    Some(SpinnerGuard::start(message.into()))
}
