//! User-friendly terminal UI utilities for human-readable output.
//!
//! This module provides utilities for printing colorful, formatted messages
//! to the terminal when in human output mode. It respects the global output
//! format and suppresses output when in JSON mode.

use console::style;
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
