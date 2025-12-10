//! Shell output abstraction for the CLI.
//!
//! This module provides a global `Shell` for CLI output,
//! handling terminal detection, colors, verbosity, and JSON output mode.

use std::fmt::Display;
use std::io::{self, IsTerminal, Write};
use std::sync::OnceLock;

use anstyle::{AnsiColor, Style};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde::Serialize;
use waterui_cli::utils::set_std_output;

/// Global shell instance.
static SHELL: OnceLock<Shell> = OnceLock::new();

/// ANSI styles for output.
mod styles {
    use super::{AnsiColor, Style};

    pub const HEADER: Style = Style::new()
        .bold()
        .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Green)));
    pub const ERROR: Style = Style::new()
        .bold()
        .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Red)));
    pub const WARN: Style = Style::new()
        .bold()
        .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Yellow)));
    pub const NOTE: Style = Style::new()
        .bold()
        .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Cyan)));
}

/// Initialize the global shell.
///
/// Must be called once at program start.
pub fn init(json: bool) {
    let shell = if json { Shell::json() } else { Shell::new() };
    let _ = SHELL.set(shell);
}

/// Get a reference to the global shell.
///
/// # Panics
///
/// Panics if `init()` was not called.
pub fn get() -> &'static Shell {
    SHELL
        .get()
        .expect("shell not initialized, call shell::init() first")
}

/// Shell output abstraction.
pub struct Shell {
    output: ShellOut,
    multi_progress: MultiProgress,
}

enum ShellOut {
    Human,
    Json,
}

impl Shell {
    fn new() -> Self {
        Self {
            output: ShellOut::Human,
            multi_progress: MultiProgress::new(),
        }
    }

    fn json() -> Self {
        Self {
            output: ShellOut::Json,
            multi_progress: MultiProgress::new(),
        }
    }

    /// Check if output is in JSON mode.
    #[must_use]
    pub const fn is_json(&self) -> bool {
        matches!(self.output, ShellOut::Json)
    }

    /// Check if stderr is a terminal.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        match &self.output {
            ShellOut::Human => io::stderr().is_terminal(),
            ShellOut::Json => false,
        }
    }

    /// Print a status message with a green header.
    pub fn status(&self, status: impl Display, message: impl Display) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => {
                let mut stderr = anstream::stderr().lock();
                writeln!(
                    stderr,
                    "{}{}{} {message}",
                    styles::HEADER,
                    status,
                    styles::HEADER.render_reset()
                )?;
                stderr.flush()
            }
            ShellOut::Json => {
                #[derive(Serialize)]
                struct Status<'a> {
                    status: &'a str,
                    message: &'a str,
                }
                let json = serde_json::to_string(&Status {
                    status: &status.to_string(),
                    message: &message.to_string(),
                })?;
                writeln!(io::stdout(), "{json}")?;
                io::stdout().flush()
            }
        }
    }

    /// Print an error message.
    pub fn error(&self, message: impl Display) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => {
                let mut stderr = anstream::stderr().lock();
                write!(
                    stderr,
                    "{}error{}: ",
                    styles::ERROR,
                    styles::ERROR.render_reset()
                )?;
                writeln!(stderr, "{message}")?;
                stderr.flush()
            }
            ShellOut::Json => {
                #[derive(Serialize)]
                struct Error<'a> {
                    level: &'static str,
                    message: &'a str,
                }
                let json = serde_json::to_string(&Error {
                    level: "error",
                    message: &message.to_string(),
                })?;
                writeln!(io::stdout(), "{json}")?;
                io::stdout().flush()
            }
        }
    }

    /// Print a warning message.
    pub fn warn(&self, message: impl Display) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => {
                let mut stderr = anstream::stderr().lock();
                write!(
                    stderr,
                    "{}warning{}: ",
                    styles::WARN,
                    styles::WARN.render_reset()
                )?;
                writeln!(stderr, "{message}")?;
                stderr.flush()
            }
            ShellOut::Json => {
                #[derive(Serialize)]
                struct Warning<'a> {
                    level: &'static str,
                    message: &'a str,
                }
                let json = serde_json::to_string(&Warning {
                    level: "warning",
                    message: &message.to_string(),
                })?;
                writeln!(io::stdout(), "{json}")?;
                io::stdout().flush()
            }
        }
    }

    /// Print an informational note.
    pub fn note(&self, message: impl Display) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => {
                let mut stderr = anstream::stderr().lock();
                write!(
                    stderr,
                    "{}note{}: ",
                    styles::NOTE,
                    styles::NOTE.render_reset()
                )?;
                writeln!(stderr, "{message}")?;
                stderr.flush()
            }
            ShellOut::Json => Ok(()),
        }
    }

    /// Print a plain line.
    pub fn println(&self, message: impl Display) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => {
                writeln!(anstream::stderr().lock(), "{message}")?;
                Ok(())
            }
            ShellOut::Json => Ok(()),
        }
    }

    /// Print a header/title.
    pub fn header(&self, message: impl Display) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => {
                writeln!(
                    anstream::stderr().lock(),
                    "{}▶ {}{}",
                    styles::HEADER,
                    message,
                    styles::HEADER.render_reset()
                )?;
                Ok(())
            }
            ShellOut::Json => Ok(()),
        }
    }

    /// Create a progress spinner.
    ///
    /// Returns `None` in JSON mode or non-terminal.
    #[must_use]
    pub fn spinner(&self, message: impl Into<String>) -> Option<ProgressBar> {
        if !self.is_terminal() || self.is_json() {
            return None;
        }

        let pb = self.multi_progress.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .expect("valid template"),
        );
        pb.set_message(message.into());
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(pb)
    }
}

// Convenience functions that use the global shell

/// Print a status message.
pub fn status(status: impl Display, message: impl Display) {
    let _ = get().status(status, message);
}

/// Print an error message (use `error!` macro instead).
#[doc(hidden)]
pub fn error_fn(message: impl Display) {
    let _ = get().error(message);
}

/// Print a warning message (use `warn!` macro instead).
#[doc(hidden)]
pub fn warn_fn(message: impl Display) {
    let _ = get().warn(message);
}

/// Print a note message (use `note!` macro instead).
#[doc(hidden)]
pub fn note_fn(message: impl Display) {
    let _ = get().note(message);
}

/// Print a plain line (use `line!` macro instead).
#[doc(hidden)]
pub fn println(message: impl Display) {
    let _ = get().println(message);
}

/// Print a header (use `header!` macro instead).
#[doc(hidden)]
pub fn header_fn(message: impl Display) {
    let _ = get().header(message);
}

pub async fn display_output<Fut: Future>(fut: Fut) -> Fut::Output {
    if is_interactive() {
        set_std_output(true);
        let result = fut.await;
        set_std_output(false);
        result
    } else {
        fut.await
    }
}

/// Create a spinner.
pub fn spinner(message: impl Into<String>) -> Option<ProgressBar> {
    get().spinner(message)
}

/// Check if running in an interactive terminal.
pub fn is_interactive() -> bool {
    get().is_terminal() && !get().is_json()
}

// ============================================================================
// Convenience macros
// ============================================================================

/// Print a success message with a checkmark.
///
/// # Example
/// ```ignore
/// success!("Project created");
/// success!("Built {} files", count);
/// ```
#[macro_export]
macro_rules! success {
    ($($arg:tt)*) => {
        $crate::shell::status("✓", format!($($arg)*))
    };
}

/// Print a plain line (like println but through shell).
///
/// # Example
/// ```ignore
/// line!("Next steps:");
/// line!("  cd {}", path);
/// line!();  // empty line
/// ```
#[macro_export]
macro_rules! line {
    () => {
        $crate::shell::println("")
    };
    ($($arg:tt)*) => {
        $crate::shell::println(format!($($arg)*))
    };
}

/// Print a warning message.
///
/// # Example
/// ```ignore
/// warn!("File not found");
/// warn!("Missing {} dependencies", count);
/// ```
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::shell::warn_fn(format!($($arg)*))
    };
}

/// Print an error message.
///
/// # Example
/// ```ignore
/// error!("Build failed");
/// error!("Cannot find {}", path);
/// ```
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::shell::error_fn(format!($($arg)*))
    };
}

/// Print a note/info message.
///
/// # Example
/// ```ignore
/// note!("Press Ctrl+C to stop");
/// note!("Using {} as default", value);
/// ```
#[macro_export]
macro_rules! note {
    ($($arg:tt)*) => {
        $crate::shell::note_fn(format!($($arg)*))
    };
}

/// Print a header/title.
///
/// # Example
/// ```ignore
/// header!("Building project");
/// header!("Running on {}", device);
/// ```
#[macro_export]
macro_rules! header {
    ($($arg:tt)*) => {
        $crate::shell::header_fn(format!($($arg)*))
    };
}
