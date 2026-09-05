//! Utility functions for the CLI.

use std::ffi::OsStr;
use std::{
    io,
    path::{Path, PathBuf},
    process::Output,
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

/// Returns a platform-appropriate installation hint for sccache.
#[must_use]
pub const fn sccache_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "brew install sccache"
    } else if cfg!(target_os = "linux") {
        "your distro package manager (e.g. apt/dnf/pacman) or cargo install sccache"
    } else if cfg!(target_os = "windows") {
        "winget install Mozilla.sccache or cargo install sccache"
    } else {
        "cargo install sccache"
    }
}

// Warn: You will lose stdout/stderr piping if you modify this function!
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

/// Run a command and capture its output regardless of exit status.
///
/// Supports non-UTF8 executable paths and arguments.
pub(crate) async fn run_command_output_os<N, A, S>(name: N, args: A) -> eyre::Result<Output>
where
    N: AsRef<OsStr>,
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let name = name.as_ref();
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

    Ok(result)
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
    run_command_os(name, args).await
}

/// Run a command with the specified name and arguments.
///
/// Like `run_command`, but supports non-UTF8 executable paths and arguments.
pub(crate) async fn run_command_os<N, A, S>(name: N, args: A) -> eyre::Result<String>
where
    N: AsRef<OsStr>,
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let name_ref = name.as_ref();
    let result = run_command_output_os(name_ref, args).await?;

    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).to_string())
    } else {
        let name_display = name_ref.to_string_lossy();
        Err(eyre::eyre!(
            "Command {name_display} failed with status {}{}{}",
            result.status,
            format_failure_stream("stderr", &result.stderr),
            format_failure_stream("stdout", &result.stdout),
        ))
    }
}

/// Number of trailing lines reported from each captured stream when a command fails.
const MAX_REPORTED_OUTPUT_LINES: usize = 200;

/// Whether a build tool marked this output line as a diagnostic.
///
/// Covers the compiler form `path:line:col: error: message` (swiftc, clang,
/// rustc, `xcodebuild` relaying any of them) and the bare `error: message` of
/// cargo, `swift build`, and linkers.
fn is_diagnostic_line(line: &str) -> bool {
    line.contains("error: ")
}

/// Render one captured stream for a command-failure report.
///
/// Both streams are always reported: build tools do not agree on which one carries
/// diagnostics, and `xcodebuild` in particular writes compiler and linker errors to
/// stdout while stdout is also where its progress noise goes. The tail is shown in
/// full, and the number of elided lines is stated rather than silently dropped.
///
/// The tail alone is not enough: `xcodebuild` keeps going after a compile error to
/// finish the targets that do not depend on it, and a run-script phase dumps its
/// whole environment on the way, so the diagnostic that explains the failure can sit
/// well over a thousand lines before the end (#345). Every diagnostic line that falls
/// outside the tail is therefore reported ahead of it.
fn format_failure_stream(label: &str, bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = trimmed.lines().collect();
    let elided = lines.len().saturating_sub(MAX_REPORTED_OUTPUT_LINES);
    let body = lines[elided..].join("\n");
    if elided == 0 {
        return format!("\n{label}:\n{body}");
    }

    let mut report = String::new();
    let diagnostics: Vec<&str> = lines[..elided]
        .iter()
        .copied()
        .filter(|line| is_diagnostic_line(line))
        .collect();
    if !diagnostics.is_empty() {
        write!(
            report,
            "\n{label} diagnostics before the reported tail ({} lines):\n{}",
            diagnostics.len(),
            diagnostics.join("\n")
        )
        .expect("writing to a String cannot fail");
    }
    write!(
        report,
        "\n{label} (last {MAX_REPORTED_OUTPUT_LINES} of {} lines):\n{body}",
        lines.len()
    )
    .expect("writing to a String cannot fail");
    report
}

/// Parse whitespace-separated u32 values (e.g., process IDs).
pub(crate) fn parse_whitespace_separated_u32s(input: &str) -> Vec<u32> {
    input
        .split_whitespace()
        .filter_map(|part| part.parse::<u32>().ok())
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{
        MAX_REPORTED_OUTPUT_LINES, format_failure_stream, parse_whitespace_separated_u32s,
    };

    #[test]
    fn failure_report_surfaces_diagnostics_elided_from_the_tail() {
        let diagnostic =
            "Sources/WuiMapView.swift:137:21: error: cannot find 'makeRegionWatcher' in scope";
        let mut lines = vec!["CompileSwift normal arm64", diagnostic];
        let noise = "    export SDKROOT=/Applications/Xcode.app";
        lines.extend(std::iter::repeat_n(noise, MAX_REPORTED_OUTPUT_LINES * 3));
        lines.push("** BUILD FAILED **");
        let report = format_failure_stream("stdout", lines.join("\n").as_bytes());

        assert!(
            report.contains(diagnostic),
            "the elided compiler error must be reported: {report}"
        );
        assert!(report.contains("stdout diagnostics before the reported tail (1 lines):"));
        assert!(report.contains(&format!(
            "stdout (last {MAX_REPORTED_OUTPUT_LINES} of {} lines):",
            lines.len()
        )));
        assert!(report.ends_with("** BUILD FAILED **"));
        assert_eq!(
            report.matches(diagnostic).count(),
            1,
            "a diagnostic outside the tail is reported once"
        );
    }

    #[test]
    fn failure_report_shows_short_output_whole() {
        let report = format_failure_stream("stderr", b"error: linking failed\n");
        assert_eq!(report, "\nstderr:\nerror: linking failed");
    }

    #[test]
    fn parses_pidof_output_with_multiple_pids() {
        let parsed = parse_whitespace_separated_u32s("123 456\n");
        assert_eq!(parsed, vec![123, 456]);
    }

    #[test]
    fn ignores_non_numeric_tokens() {
        let parsed = parse_whitespace_separated_u32s("foo 42 bar\n");
        assert_eq!(parsed, vec![42]);
    }
}
