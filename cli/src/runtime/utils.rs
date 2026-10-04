//! Utility functions for the CLI.

use std::ffi::OsStr;
use std::{
    io,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

use semver::Version;
use smol::{process::Command, unblock};
use thiserror::Error;

use crate::toolchain::Host;

/// An external command could not be executed or exited unsuccessfully.
#[derive(Debug, Error)]
pub enum CommandError {
    /// The command could not be spawned.
    #[error("failed to spawn `{program}`: {source}")]
    Spawn {
        /// The program that was invoked.
        program: String,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The command exited with a non-zero status.
    #[error("command `{program}` failed with status {status}{report}")]
    Failed {
        /// The program that was invoked.
        program: String,
        /// The process exit status.
        status: ExitStatus,
        /// Formatted diagnostic tail of the captured output streams.
        report: String,
    },
}

/// Locate an executable in the real host's PATH.
///
/// Return the path to the executable if found.
///
/// # Errors
/// - If the executable is not found in the PATH.
pub(crate) async fn which(name: &'static str) -> Result<PathBuf, which::Error> {
    Host::current().which(name).await
}

/// Enable or disable standard output for command executions.
///
/// By default, standard output is disabled.
static STD_OUTPUT: AtomicBool = AtomicBool::new(false);

/// Enable or disable standard output for command executions.
pub fn set_std_output(enabled: bool) {
    STD_OUTPUT.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

/// Whether captured command output is also echoed to the terminal.
pub(crate) fn std_output_enabled() -> bool {
    STD_OUTPUT.load(Ordering::SeqCst)
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

/// Returns a platform-appropriate upgrade hint for an already-installed
/// sccache that is too old — `install` is a no-op on an existing package.
#[must_use]
pub const fn sccache_upgrade_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "brew upgrade sccache"
    } else if cfg!(target_os = "linux") {
        "your distro package manager or cargo install sccache --force"
    } else if cfg!(target_os = "windows") {
        "winget upgrade Mozilla.sccache or cargo install sccache --force"
    } else {
        "cargo install sccache --force"
    }
}

// Warn: You will lose stdout/stderr piping if you modify this function!
pub(crate) fn command(command: &mut Command) -> &mut Command {
    command
        .kill_on_drop(true)
        .stdout(if std_output_enabled() {
            Stdio::inherit()
        } else {
            Stdio::piped()
        })
        .stderr(if std_output_enabled() {
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
/// - [`CommandError::Spawn`] if the command cannot be spawned.
/// - [`CommandError::Failed`] if the command exits with a non-zero status.
pub(crate) async fn run_command(
    name: &str,
    args: impl IntoIterator<Item = &str>,
) -> Result<String, CommandError> {
    run_command_os(name, args).await
}

/// Run a command with the specified name and arguments.
///
/// Like `run_command`, but supports non-UTF8 executable paths and arguments.
///
/// # Errors
/// - [`CommandError::Spawn`] if the command cannot be spawned.
/// - [`CommandError::Failed`] if the command exits with a non-zero status.
pub(crate) async fn run_command_os<N, A, S>(name: N, args: A) -> Result<String, CommandError>
where
    N: AsRef<OsStr>,
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Host::current().run(name, args).await
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
pub(crate) fn format_failure_stream(label: &str, bytes: &[u8]) -> String {
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

/// Parse a version that may omit the minor and/or patch components.
///
/// `semver::Version` requires all three components, but version reporters
/// commonly provide only major.minor — `rustc` accepts `1.88`, and
/// `simctl`/`IPHONEOS_DEPLOYMENT_TARGET` use `26.0`-style iOS versions. Missing
/// trailing components are padded with zeros. A leading `v` and a
/// `-prerelease` suffix are also accepted.
///
/// # Errors
/// - If the input is empty, has more than three numeric components, or is not
///   valid semver after normalization.
pub fn parse_semver_version(input: &str) -> Result<Version, VersionParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(VersionParseError::EmptyVersion);
    }

    let normalized_input = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let mut split = normalized_input.splitn(2, '-');
    let core = split.next().ok_or(VersionParseError::MissingCoreVersion)?;
    let prerelease = split.next();

    let mut components: Vec<&str> = core.split('.').collect();
    match components.len() {
        1 => {
            components.push("0");
            components.push("0");
        }
        2 => {
            components.push("0");
        }
        3 => {}
        count => {
            return Err(VersionParseError::InvalidComponentCount {
                count,
                input: input.to_owned(),
            });
        }
    }

    let mut normalized = components.join(".");
    if let Some(prerelease) = prerelease {
        normalized.push('-');
        normalized.push_str(prerelease);
    }

    Version::parse(&normalized).map_err(|source| VersionParseError::InvalidVersion {
        input: input.to_owned(),
        normalized,
        source,
    })
}

/// A version string `parse_semver_version` could not normalize.
#[derive(Debug, Error)]
pub enum VersionParseError {
    /// The version string was empty.
    #[error("version is empty")]
    EmptyVersion,
    /// The version string had no numeric core.
    #[error("missing numeric core version")]
    MissingCoreVersion,
    /// The version had an unsupported component count.
    #[error("expected 1-3 numeric components, found {count} in `{input}`")]
    InvalidComponentCount {
        /// The number of dotted components found.
        count: usize,
        /// The offending input.
        input: String,
    },
    /// The normalized version failed semver parsing.
    #[error("failed to parse version `{input}` as `{normalized}`: {source}")]
    InvalidVersion {
        /// The offending input.
        input: String,
        /// The normalized form that was attempted.
        normalized: String,
        /// The semver parse error.
        #[source]
        source: semver::Error,
    },
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
/// An existing destination is replaced, matching `fs::copy` semantics:
/// callers stage build outputs into directories that persist across runs,
/// like the `DerivedData` products directory `CACHE_PATHS` keeps between
/// packages.
///
/// # Errors
/// - If the copy operation fails.
pub async fn copy_file(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let from = from.as_ref().to_path_buf();
    let to = to.as_ref().to_path_buf();
    unblock(move || {
        // `reflink_or_copy` refuses to overwrite; every caller expects the
        // staged file at `to` to carry `from`'s contents afterwards.
        match std::fs::remove_file(&to) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        reflink_copy::reflink_or_copy(from, to).map(|_| ())
    })
    .await
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{
        MAX_REPORTED_OUTPUT_LINES, format_failure_stream, parse_semver_version,
        parse_whitespace_separated_u32s,
    };

    #[test]
    fn parse_semver_version_accepts_major_minor() {
        let parsed = parse_semver_version("1.88").expect("version should parse");
        assert_eq!(parsed, Version::new(1, 88, 0));
    }

    #[test]
    fn parse_semver_version_pads_deployment_target_style() {
        let parsed = parse_semver_version("26.0").expect("version should parse");
        assert_eq!(parsed, Version::new(26, 0, 0));
    }

    #[test]
    fn parse_semver_version_orders_release_lines() {
        let ios_18 = parse_semver_version("18.5").expect("version should parse");
        let ios_26 = parse_semver_version("26.5").expect("version should parse");
        assert!(ios_26 > ios_18);
    }

    #[test]
    fn parse_semver_version_rejects_extra_components() {
        assert!(parse_semver_version("1.2.3.4").is_err());
    }

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

    #[test]
    fn copy_file_replaces_an_existing_destination() {
        // Restaging over the products a preserved `DerivedData` still holds
        // is what a second `water package` does; the copy must overwrite.
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("temp dir");
            let source = dir.path().join("lib.a");
            let dest = dir.path().join("staged/lib.a");
            std::fs::create_dir_all(dest.parent().expect("the dest has a parent"))
                .expect("create the staging dir");
            std::fs::write(&source, "built from revision 1").expect("write source");
            super::copy_file(&source, &dest).await.expect("first stage");

            std::fs::write(&source, "built from revision 2").expect("rewrite source");
            super::copy_file(&source, &dest)
                .await
                .expect("restaging must not fail on the previous build's file");
            assert_eq!(
                std::fs::read_to_string(&dest).expect("read staged file"),
                "built from revision 2",
                "the staged file must carry the fresh build's contents"
            );
        });
    }
}
