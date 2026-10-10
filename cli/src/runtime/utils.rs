//! Utility functions for the CLI.

use std::{
    io,
    path::Path,
    process::{ExitStatus, Stdio},
};

use semver::Version;
use smol::{process::Command, unblock};
use thiserror::Error;

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
pub(crate) fn command(command: &mut Command, std_output: bool) -> &mut Command {
    command
        .kill_on_drop(true)
        .stdout(if std_output {
            Stdio::inherit()
        } else {
            Stdio::piped()
        })
        .stderr(if std_output {
            Stdio::inherit()
        } else {
            Stdio::piped()
        })
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
    unblock(move || copy_file_overwriting(&from, &to)).await
}

/// The lowercase hex SHA-256 of the file at `path`, read in chunks off the
/// executor thread.
///
/// # Errors
/// - If the file cannot be read.
pub async fn file_sha256(path: &Path) -> io::Result<String> {
    use sha2::{Digest as _, Sha256};
    use std::io::Read as _;
    let path = path.to_path_buf();
    unblock(move || {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
}

/// Copy `from` onto `to` only when its bytes differ.
///
/// The write-on-change path for file-to-file copies — the counterpart of
/// `crate::templates::write_file_if_changed` for byte buffers. A per-build
/// staged copy that already carries the right bytes is left untouched, so
/// its mtime never marks a managed output as fresh input to the next build.
///
/// The comparison is the size first and then the bytes themselves — every
/// file under a Cargo registry source carries the same deterministic
/// mtime, so a metadata check would read two different files as unchanged.
/// A copy that does get written keeps whatever mtime the copy mechanism
/// leaves — `reflink_or_copy` is `clonefile` on macOS, which carries the
/// source's mtime over, while a plain byte write stamps its own — so
/// nothing downstream may read the copy's metadata as a source state; the
/// byte compare is the only guarantee.
///
/// An existing `to` that cannot be read is an error — only `NotFound`
/// counts as absent, matching `write_file_if_changed`.
///
/// # Errors
/// - If `from` or an existing `to` cannot be read, or the copy fails.
pub async fn copy_file_if_changed(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let from = from.as_ref().to_path_buf();
    let to = to.as_ref().to_path_buf();
    unblock(move || copy_file_if_changed_sync(&from, &to)).await
}

/// The blocking form of [`copy_file_if_changed`], for call sites already
/// inside `smol::unblock` or synchronous contexts.
///
/// # Errors
/// - If `from` or an existing `to` cannot be read, or the copy fails.
pub fn copy_file_if_changed_sync(from: &Path, to: &Path) -> io::Result<()> {
    if files_same_contents(from, to)? {
        return Ok(());
    }
    replace_with_copy(from, to)
}

/// `replace_with_copy` plus the source's mtime stamped on the copy: a
/// verbatim staged copy whose metadata mirrors the file it carries — the
/// property [`cargo_output_unmodified`]'s size-and-mtime check relies on
/// where it applies. The copy keeps the source's mode, which may be
/// read-only, so the time is set by path — no write handle is needed.
fn copy_file_overwriting(from: &Path, to: &Path) -> io::Result<()> {
    let from_modified = std::fs::metadata(from)?.modified()?;
    replace_with_copy(from, to)?;
    filetime::set_file_mtime(to, filetime::FileTime::from_system_time(from_modified))
}

/// Remove `to` and copy `from`'s bytes in its place — `reflink_or_copy`
/// refuses to overwrite, and every caller expects `to` to carry `from`'s
/// contents afterwards. The copy's modification time is whatever the copy
/// mechanism leaves: `clonefile` on macOS preserves the source's mtime
/// while a plain write stamps the write's own. [`copy_file_if_changed_sync`]'s
/// byte-compare reads the copy's contents, never its mtime, so the
/// difference is inert; a caller needing a named mtime stamps it
/// explicitly the way [`copy_file_overwriting`] does.
fn replace_with_copy(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::remove_file(to) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    reflink_copy::reflink_or_copy(from, to).map(|_| ())
}

/// Whether `to` currently carries `from`'s bytes: the sizes first —
/// different lengths cannot match — then a streamed compare through
/// `BufReader`'s own buffers, never a whole-file read. `false` when `to`
/// does not exist, an error for any other stat or read failure on either
/// side.
fn files_same_contents(from: &Path, to: &Path) -> io::Result<bool> {
    use std::io::BufRead as _;

    let from_meta = std::fs::metadata(from)?;
    let to_meta = match std::fs::metadata(to) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if to_meta.len() != from_meta.len() {
        return Ok(false);
    }
    let mut from = io::BufReader::new(std::fs::File::open(from)?);
    let mut to = io::BufReader::new(std::fs::File::open(to)?);
    loop {
        let from_chunk = from.fill_buf()?;
        let to_chunk = to.fill_buf()?;
        // Both empty means equal at the end; one empty means different
        // trailing bytes — the size check already kept the lengths equal.
        if from_chunk.is_empty() || to_chunk.is_empty() {
            return Ok(from_chunk.is_empty() && to_chunk.is_empty());
        }
        let common = from_chunk.len().min(to_chunk.len());
        if from_chunk[..common] != to_chunk[..common] {
            return Ok(false);
        }
        from.consume(common);
        to.consume(common);
    }
}

/// Whether `to` still names the same Cargo output `from` reported: the
/// same size and the same modification time. Sound only where the caller's
/// contract holds — a Cargo build output's mtime changes on every write,
/// so a matching mtime means the artifact was not rewritten. Anything else
/// must compare bytes — registry sources share one deterministic mtime —
/// which is what [`copy_file_if_changed`] does instead. `false` when `to`
/// does not exist, an error for any other stat failure on either side.
pub(crate) fn cargo_output_unmodified(from: &Path, to: &Path) -> io::Result<bool> {
    let from_meta = std::fs::metadata(from)?;
    let to_meta = match std::fs::metadata(to) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(to_meta.len() == from_meta.len() && to_meta.modified()? == from_meta.modified()?)
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{
        MAX_REPORTED_OUTPUT_LINES, format_failure_stream, parse_semver_version,
        parse_whitespace_separated_u32s,
    };

    /// A read-only source copies fine: `reflink_or_copy` keeps the 0444
    /// mode, so stamping the mtime must not need a write handle on the
    /// copy.
    #[test]
    fn copy_file_stamps_the_mtime_of_a_read_only_copy() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let source = temporary.path().join("source.bin");
        let staged = temporary.path().join("staged.bin");
        let staged_again = temporary.path().join("staged-again.bin");
        std::fs::write(&source, b"read only").expect("write source");
        let mut permissions = std::fs::metadata(&source)
            .expect("source metadata")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&source, permissions).expect("mark the source read-only");

        smol::block_on(super::copy_file(&source, &staged))
            .expect("copying a read-only source should succeed");
        smol::block_on(super::copy_file_if_changed(&source, &staged_again))
            .expect("copying a read-only source should succeed");
        for copy in [&staged, &staged_again] {
            assert_eq!(std::fs::read(copy).expect("staged copy"), b"read only");
        }
        assert!(
            super::cargo_output_unmodified(&source, &staged).expect("compare the copy"),
            "a verbatim copy carries the source's mtime stamp"
        );
    }

    /// `copy_file_if_changed` compares bytes, not the registry mtimes: two
    /// sources with the same size and the same mtime but different bytes
    /// still copy. A metadata compare would read them as unchanged and
    /// leave the stale destination in place.
    #[test]
    fn copy_file_if_changed_compares_bytes_not_metadata() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let source = temporary.path().join("source.bin");
        let staged = temporary.path().join("staged.bin");
        std::fs::write(&source, b"new bytes").expect("write source");
        std::fs::write(&staged, b"old bytes").expect("write stale copy");
        // Same size, and the registry's deterministic mtime stamped on
        // both: only the bytes distinguish them.
        let shared = filetime::FileTime::from_unix_time(1_700_000_000, 0);
        filetime::set_file_mtime(&source, shared).expect("stamp source mtime");
        filetime::set_file_mtime(&staged, shared).expect("stamp staged mtime");

        smol::block_on(super::copy_file_if_changed(&source, &staged))
            .expect("a different-bytes same-metadata copy must still run");
        assert_eq!(std::fs::read(&staged).expect("staged copy"), b"new bytes");
    }

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
