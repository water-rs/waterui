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
    pub const DEBUG: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Magenta)));
    pub const TRACE: Style =
        Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::BrightBlack)));
    pub const TAG: Style = Style::new().bold();
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

    /// Print a raw JSON line to stdout (JSON mode only).
    pub fn json_raw(&self, json: &str) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => Ok(()),
            ShellOut::Json => {
                let mut stdout = io::stdout().lock();
                writeln!(stdout, "{json}")?;
                stdout.flush()
            }
        }
    }

    /// Print a device log with level-appropriate styling.
    ///
    /// The message should be in format "[TAG] message" for best display.
    /// Platform is used as a prefix (e.g., "Android", "Apple").
    pub fn device_log(
        &self,
        platform: &str,
        level: tracing::Level,
        message: impl Display,
    ) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => {
                let mut stderr = anstream::stderr().lock();
                let msg = message.to_string();

                // Get level style and short name
                let (level_style, level_char) = match level {
                    tracing::Level::ERROR => (styles::ERROR, 'E'),
                    tracing::Level::WARN => (styles::WARN, 'W'),
                    tracing::Level::INFO => (styles::NOTE, 'I'),
                    tracing::Level::DEBUG => (styles::DEBUG, 'D'),
                    tracing::Level::TRACE => (styles::TRACE, 'V'),
                };
                let reset = Style::new().render_reset();

                // Try to extract [TAG] from message for styled output
                if let Some((tag, rest)) = parse_log_tag(&msg) {
                    writeln!(
                        stderr,
                        "{level_style}{platform}/{level_char}{reset} {tag_style}[{tag}]{reset} {rest}",
                        tag_style = styles::TAG,
                    )?;
                } else {
                    writeln!(stderr, "{level_style}{platform}/{level_char}{reset} {msg}",)?;
                }
                stderr.flush()
            }
            ShellOut::Json => {
                #[derive(Serialize)]
                struct Log<'a> {
                    #[serde(rename = "type")]
                    ty: &'static str,
                    platform: &'a str,
                    level: &'a str,
                    message: &'a str,
                }
                let level_str = match level {
                    tracing::Level::ERROR => "error",
                    tracing::Level::WARN => "warn",
                    tracing::Level::INFO => "info",
                    tracing::Level::DEBUG => "debug",
                    tracing::Level::TRACE => "trace",
                };
                let json = serde_json::to_string(&Log {
                    ty: "log",
                    platform,
                    level: level_str,
                    message: &message.to_string(),
                })?;
                writeln!(io::stdout(), "{json}")?;
                io::stdout().flush()
            }
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

/// Parse a log message to extract [TAG] prefix.
/// Returns (tag, `rest_of_message`) if found.
fn parse_log_tag(msg: &str) -> Option<(&str, &str)> {
    let msg = msg.trim();
    if !msg.starts_with('[') {
        return None;
    }
    let end = msg.find(']')?;
    let tag = &msg[1..end];
    let rest = msg[end + 1..].trim_start();
    Some((tag, rest))
}

/// Find a file by walking up from cwd to find workspace root.
///
/// Tries to find the file relative to directories containing Cargo.toml.
fn find_file_in_workspace(relative_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;

    // First try relative to cwd
    let direct = cwd.join(relative_path);
    if direct.exists() {
        return Some(direct);
    }

    // Walk up the directory tree looking for Cargo.toml (workspace root indicators)
    let mut current = cwd.as_path();
    while let Some(parent) = current.parent() {
        let candidate = parent.join(relative_path);
        if candidate.exists() {
            return Some(candidate);
        }

        // Stop at filesystem root or if we've gone too far up
        if parent.join("Cargo.toml").exists() || parent.components().count() <= 2 {
            // Keep going but check this level too
        }

        current = parent;
    }

    None
}

/// Parsed panic information for display.
pub struct PanicReport<'a> {
    /// The panic message (e.g., "Test panic: something failed")
    pub message: &'a str,
    /// Source file path
    pub file: Option<&'a str>,
    /// Line number (1-indexed)
    pub line: Option<usize>,
    /// Column number (1-indexed)
    pub column: Option<usize>,
    /// Additional crash info (exception, signal, etc.)
    pub extra: Option<&'a str>,
    /// Path to crash report file
    pub crash_report_path: Option<&'a str>,
}

impl<'a> PanicReport<'a> {
    /// Parse a crash message into a structured panic report.
    ///
    /// Expected format:
    /// ```text
    /// Panic: message
    ///   at file.rs:123:45
    ///
    /// Exception: EXC_CRASH, Signal: SIGABRT, Reason: ...
    ///
    /// Crash report: /path/to/crash.ips
    /// ```
    pub fn parse(crash_msg: &'a str) -> Self {
        let mut message = crash_msg;
        let mut file = None;
        let mut line = None;
        let mut column = None;
        let mut extra = None;
        let mut crash_report_path = None;

        // Split into lines for parsing
        let lines: Vec<&str> = crash_msg.lines().collect();

        for (i, ln) in lines.iter().enumerate() {
            let ln = ln.trim();

            // Parse "Panic: message"
            if ln.starts_with("Panic:") {
                message = ln.strip_prefix("Panic:").unwrap_or(ln).trim();
            }
            // Parse "  at file:line:col"
            else if ln.starts_with("at ") {
                if let Some(loc) = ln.strip_prefix("at ") {
                    let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
                    match parts.as_slice() {
                        [col, ln_num, path] => {
                            file = Some(*path);
                            line = ln_num.parse().ok();
                            column = col.parse().ok();
                        }
                        [ln_num, path] => {
                            file = Some(*path);
                            line = ln_num.parse().ok();
                        }
                        _ => {}
                    }
                }
            }
            // Parse "Crash report: path"
            else if ln.starts_with("Crash report:") {
                crash_report_path = ln.strip_prefix("Crash report:").map(str::trim);
            }
            // Capture exception/signal info
            else if ln.starts_with("Exception:") || ln.starts_with("Signal:") {
                // Find the range of extra info (from this line to before "Crash report:")
                let extra_end = lines[i..]
                    .iter()
                    .position(|l| l.starts_with("Crash report:"))
                    .map_or(lines.len(), |pos| i + pos);
                if extra_end > i
                    && lines[i..extra_end]
                        .iter()
                        .map(|line| line.trim())
                        .any(|line| !line.is_empty())
                {
                    // We'll store the first line as extra
                    extra = Some(lines[i].trim());
                }
            }
        }

        Self {
            message,
            file,
            line,
            column,
            extra,
            crash_report_path,
        }
    }
}

impl Shell {
    /// Display a panic report with colored output and code context.
    pub fn panic_report(&self, report: &PanicReport<'_>) -> io::Result<()> {
        match &self.output {
            ShellOut::Human => Self::panic_report_human(report),
            ShellOut::Json => Self::panic_report_json(report),
        }
    }

    fn panic_report_human(report: &PanicReport<'_>) -> io::Result<()> {
        use std::fs::File;
        use std::io::BufRead;
        use std::path::Path;

        let mut stderr = anstream::stderr().lock();
        let reset = Style::new().render_reset();

        // Style definitions
        let error_style = styles::ERROR;
        let note_style = styles::NOTE;
        let line_num_style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Blue)));
        let highlight_style = Style::new()
            .bold()
            .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Red)));

        // Print "error: Panic: message"
        writeln!(
            stderr,
            "{error_style}error{reset}: {error_style}Panic{reset}: {}",
            report.message
        )?;

        // Print location if available
        if let (Some(file), Some(line)) = (report.file, report.line) {
            let col = report.column.unwrap_or(1);
            writeln!(stderr, "   {note_style}-->{reset} {file}:{line}:{col}")?;

            // Try to resolve the file path (may be relative to workspace root)
            let file_path = Path::new(file);
            let resolved_path = if file_path.is_absolute() {
                Some(file_path.to_path_buf())
            } else {
                // Try to find the file by walking up from cwd to find workspace root
                find_file_in_workspace(file_path)
            };

            // Try to read and display code context
            if let Some(ref resolved) = resolved_path
                && let Ok(source_file) = File::open(resolved)
            {
                let reader = io::BufReader::new(source_file);
                let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

                let line_idx = line.saturating_sub(1);
                let start = line_idx.saturating_sub(1);
                let end = (line_idx + 2).min(lines.len());

                // Calculate the width needed for line numbers
                let max_line_num = end;
                let line_num_width = max_line_num.to_string().len();

                writeln!(stderr, "    {line_num_style}|{reset}")?;

                for (idx, source_line) in lines[start..end].iter().enumerate() {
                    let current_line = start + idx + 1;
                    let is_panic_line = current_line == line;

                    if is_panic_line {
                        // Highlight the panic line
                        writeln!(
                            stderr,
                            "{error_style}{current_line:>line_num_width$}{reset} {line_num_style}|{reset} {highlight_style}{source_line}{reset}"
                        )?;

                        // Print the column indicator
                        let col_offset = col.saturating_sub(1);
                        let spaces = " ".repeat(col_offset);
                        let carets =
                            "^".repeat(source_line.len().saturating_sub(col_offset).clamp(1, 20));
                        writeln!(
                            stderr,
                            "{:>line_num_width$} {line_num_style}|{reset} {spaces}{error_style}{carets}{reset}",
                            ""
                        )?;
                    } else {
                        writeln!(
                            stderr,
                            "{line_num_style}{current_line:>line_num_width$}{reset} {line_num_style}|{reset} {source_line}"
                        )?;
                    }
                }

                writeln!(stderr, "    {line_num_style}|{reset}")?;
            }
        }

        // Print extra info (exception, signal, etc.)
        if let Some(extra) = report.extra {
            writeln!(stderr)?;
            writeln!(stderr, "{note_style}note{reset}: {extra}")?;
        }

        // Print crash report path
        if let Some(path) = report.crash_report_path {
            writeln!(stderr)?;
            writeln!(stderr, "{note_style}crash report{reset}: {path}")?;
        }

        stderr.flush()
    }

    fn panic_report_json(report: &PanicReport<'_>) -> io::Result<()> {
        #[derive(Serialize)]
        struct JsonPanic<'a> {
            #[serde(rename = "type")]
            ty: &'static str,
            message: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            file: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            line: Option<usize>,
            #[serde(skip_serializing_if = "Option::is_none")]
            column: Option<usize>,
            #[serde(skip_serializing_if = "Option::is_none")]
            extra: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            crash_report: Option<&'a str>,
        }

        let json = serde_json::to_string(&JsonPanic {
            ty: "panic",
            message: report.message,
            file: report.file,
            line: report.line,
            column: report.column,
            extra: report.extra,
            crash_report: report.crash_report_path,
        })?;
        writeln!(io::stdout(), "{json}")?;
        io::stdout().flush()
    }
}

// Convenience functions that use the global shell

/// Print a status message.
pub fn status(status: impl Display, message: impl Display) {
    let _ = get().status(status, message);
}

/// Print a device log with level-appropriate styling.
pub fn device_log(platform: &str, level: tracing::Level, message: impl Display) {
    let _ = get().device_log(platform, level, message);
}

/// Print an error message (use `error!` macro instead).
#[doc(hidden)]
pub fn error_fn(message: impl Display) {
    let _ = get().error(message);
}

/// Display a panic report with colored output and code context.
pub fn panic_report(crash_msg: &str) {
    let report = PanicReport::parse(crash_msg);
    let _ = get().panic_report(&report);
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

/// Print a raw JSON line to stdout (JSON mode only).
#[doc(hidden)]
pub fn json_raw(json: &str) {
    let _ = get().json_raw(json);
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

/// Clear all progress bars and release resources.
///
/// Call this before exiting to ensure progress bars don't block exit.
pub fn clear() {
    get().multi_progress.clear().ok();
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
