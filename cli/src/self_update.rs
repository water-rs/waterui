//! Self-update for the `water` binary itself.
//!
//! `water` reaches machines through four channels — the dist-generated shell
//! and PowerShell installers, the Homebrew tap, `cargo install` /
//! `cargo binstall`, and everything else — and only the first owns an update
//! path this binary may take itself: a dist install receipt says the
//! release's own installer put the binary where it is, so re-running the
//! newest release's installer (which `axoupdater` drives) is safe. Every
//! other channel belongs to a package manager whose files must not be
//! rewritten underneath it, so [`InstallSource`] resolves which channel owns
//! the running executable before anything is downloaded, and [`update`] /
//! [`check`] act on the answer.
//!
//! The passive check is a separate surface: [`passive_update_notice`] runs at
//! most once per `PASSIVE_CHECK_INTERVAL`, records the attempt in the CLI's
//! own state directory (`~/.water/config.toml`), and is silent on failure.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axoupdater::{AxoUpdater, ReleaseSource, ReleaseSourceType};
use eyre::{Result, WrapErr, bail};
use semver::Version;
use serde::Deserialize;

use crate::toolchain::Host;
use crate::water_dir;

/// The name install receipts and release installers carry: the cargo-dist
/// "app" is the package, so receipts live under `waterui-cli` and the
/// installer assets are `waterui-cli-installer.{sh,ps1}`, not `water`.
const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// The repository the GitHub release source queries when no install receipt
/// supplies one (`--check` and the passive check on non-receipt installs).
const RELEASE_OWNER: &str = "water-rs";
const RELEASE_REPO: &str = "waterui";

/// The refusal `water update` gives for a layout no channel claims — an
/// unrecognized layout is an error with a clear message, not a guess.
const UNKNOWN_INSTALL_MESSAGE: &str = "cannot determine how this `water` \
     binary was installed: no dist install receipt matches it, it resolves \
     under no Homebrew prefix, and it sits outside CARGO_HOME/bin; refusing \
     to update it";

/// The smallest gap between passive version checks.
const PASSIVE_CHECK_INTERVAL: Duration = Duration::from_hours(24);

/// The install channel the running `water` binary came through — the four
/// rows the update path distinguishes before touching anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    /// A cargo-dist install receipt covers this executable; the release's own
    /// installer may rewrite it in place (`water update`).
    Dist,
    /// The executable resolves under the Homebrew prefix; `brew` owns it.
    Homebrew,
    /// The executable sits in `CARGO_HOME/bin` with no receipt; `cargo`
    /// (`install` / `binstall`) owns it.
    Cargo,
    /// No channel's evidence matched; nothing may touch the binary.
    Unknown,
}

impl InstallSource {
    /// The human-readable name of this install channel.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dist => "release installer",
            Self::Homebrew => "Homebrew",
            Self::Cargo => "cargo",
            Self::Unknown => "unknown",
        }
    }

    /// Classify the running executable on `host` — the real-machine entry
    /// point.
    ///
    /// # Errors
    /// Returns an error when the executable's path cannot be determined or a
    /// receipt exists but cannot be read — a corrupt receipt is not evidence
    /// of a different channel.
    pub fn detect(host: &Host) -> Result<Self> {
        let executable =
            Host::current_exe().wrap_err("the running executable's path cannot be determined")?;
        Self::detect_exe(host, &executable)
    }

    /// Classify `executable` against the evidence `host` declares: a matching
    /// install receipt first, then the Homebrew prefix, then
    /// `CARGO_HOME/bin`. Anything left over is [`InstallSource::Unknown`].
    ///
    /// # Errors
    /// Returns an error when a receipt exists but cannot be read.
    fn detect_exe(host: &Host, executable: &Path) -> Result<Self> {
        let executable = canonicalize_or_self(executable);
        if let Some(prefix) = receipt_install_prefix(host)?
            && same_install_root(&executable, &canonicalize_or_self(&prefix))
        {
            return Ok(Self::Dist);
        }
        for prefix in homebrew_prefixes(host) {
            if executable.starts_with(canonicalize_or_self(&prefix)) {
                return Ok(Self::Homebrew);
            }
        }
        if let Some(cargo_bin) = cargo_bin_dir(host)
            && executable.parent() == Some(canonicalize_or_self(&cargo_bin).as_path())
        {
            return Ok(Self::Cargo);
        }
        Ok(Self::Unknown)
    }

    /// The command that updates an install from this channel — `water
    /// update` for receipt installs, the owning package manager's command
    /// otherwise. [`InstallSource::Unknown`] has no channel to name.
    #[must_use]
    pub const fn update_command(self) -> Option<&'static str> {
        match self {
            Self::Dist => Some("water update"),
            Self::Homebrew => Some("brew upgrade water"),
            Self::Cargo => Some("cargo binstall waterui-cli"),
            Self::Unknown => None,
        }
    }
}

/// What a completed `water update` leaves behind.
#[derive(Debug)]
pub enum UpdateOutcome {
    /// The release installer moved the binary between versions.
    Updated {
        /// The version the install receipt recorded before the update.
        previous: Option<Version>,
        /// The version now installed.
        installed: Version,
    },
    /// The release source lists nothing newer.
    UpToDate {
        /// The running version.
        current: Version,
    },
    /// A package manager owns the binary; `command` is its update path and
    /// nothing was changed.
    ExternallyManaged {
        /// The owning package manager's update command.
        command: &'static str,
    },
}

/// The result of a `water update` operation.
#[derive(Debug)]
pub struct UpdateReport {
    /// The channel that owns the running executable.
    pub source: InstallSource,
    /// The directory containing the running executable.
    pub install_dir: PathBuf,
    /// The update operation's outcome.
    pub outcome: UpdateOutcome,
}

/// What `water update --check` reports.
#[derive(Debug)]
pub enum CheckOutcome {
    /// The running version is the newest the release source lists.
    UpToDate {
        /// The running version.
        current: Version,
    },
    /// The release source lists a newer version.
    Available {
        /// The running version.
        current: Version,
        /// The newest version the release source lists.
        latest: Version,
        /// The command that installs it for this install channel.
        command: &'static str,
    },
}

/// `water update`: self-update in place when a dist receipt owns the binary;
/// name the owning package manager's command for every other channel and
/// change nothing.
///
/// # Errors
/// Returns an error when the install source cannot be determined (no receipt,
/// no Homebrew prefix, no `CARGO_HOME/bin`) or when the updater fails — a
/// missing receipt, a failed release query, or an installer that exits badly.
pub async fn update(host: &Host) -> Result<UpdateReport> {
    let source = InstallSource::detect(host)?;
    let install_dir = Host::current_exe()
        .wrap_err("the running executable's path cannot be determined")?
        .canonicalize()
        .wrap_err("the running executable's path cannot be canonicalized")?
        .parent()
        .ok_or_else(|| eyre::eyre!("the running executable's path has no parent directory"))?
        .to_path_buf();
    let outcome = match source {
        InstallSource::Dist => run_dist_update(host).await?,
        source => {
            let Some(command) = source.update_command() else {
                bail!("{UNKNOWN_INSTALL_MESSAGE}");
            };
            UpdateOutcome::ExternallyManaged { command }
        }
    };
    Ok(UpdateReport {
        source,
        install_dir,
        outcome,
    })
}

/// `water update --check`: report the newest release without installing it.
///
/// The release source comes from the install receipt when one covers this
/// binary and from this repository's GitHub releases otherwise.
///
/// # Errors
/// Returns an error when the install source cannot be determined or the
/// release query fails.
pub async fn check(host: &Host) -> Result<CheckOutcome> {
    let source = InstallSource::detect(host)?;
    let Some(command) = source.update_command() else {
        bail!("{UNKNOWN_INSTALL_MESSAGE}");
    };
    let current = current_version();
    let latest = query_latest(host, source).await?;
    if current < latest {
        Ok(CheckOutcome::Available {
            current,
            latest,
            command,
        })
    } else {
        Ok(CheckOutcome::UpToDate { current })
    }
}

/// The update command the `minimum-cli-version` rejection names.
///
/// `water update` when a dist receipt owns this binary, `brew upgrade water`
/// under the Homebrew prefix, and `fallback` — the channel-appropriate cargo
/// invocation the caller already selected — everywhere else.
#[must_use]
pub fn cli_update_command(host: &Host, fallback: &str) -> String {
    match InstallSource::detect(host) {
        Ok(InstallSource::Dist) => "water update".to_owned(),
        Ok(InstallSource::Homebrew) => "brew upgrade water".to_owned(),
        Ok(InstallSource::Cargo | InstallSource::Unknown) | Err(_) => fallback.to_owned(),
    }
}

/// The passive version check behind every non-hot-path command.
///
/// At most one release query per `PASSIVE_CHECK_INTERVAL`, recorded in the
/// CLI's state directory, silent on any failure. Returns the notice to print
/// when a newer release exists, `None` otherwise.
#[must_use]
pub async fn passive_update_notice(host: &Host) -> Option<String> {
    let water_home = water_dir::water_home_dir(host).ok()?;
    let mut config = water_dir::ensure_global_config_in(&water_home).await.ok()?;
    if !passive_check_due(config.last_update_check_unix_seconds, unix_now()) {
        return None;
    }
    let notice = passive_notice_inner(host).await;
    config.last_update_check_unix_seconds = Some(unix_now());
    if let Err(error) = water_dir::write_global_config_in(&water_home, &config).await {
        tracing::debug!("update check: failed to record the check timestamp: {error}");
    }
    notice
}

/// The query half of the passive check; failures degrade to `None` because
/// the notice must stay silent when the network does.
async fn passive_notice_inner(host: &Host) -> Option<String> {
    let source = match InstallSource::detect(host) {
        Ok(source) => source,
        Err(error) => {
            tracing::debug!("update check: install source detection failed: {error}");
            return None;
        }
    };
    if source == InstallSource::Unknown {
        return None;
    }
    let latest = match query_latest(host, source).await {
        Ok(latest) => latest,
        Err(error) => {
            tracing::debug!("update check: release query failed: {error}");
            return None;
        }
    };
    let current = current_version();
    if latest > current {
        Some(format!(
            "water {latest} is available (installed: {current}); update with `{}`",
            source.update_command()?,
        ))
    } else {
        None
    }
}

/// Whether the passive check may query again — at most once per interval,
/// and always when no check has been recorded or the recorded timestamp is
/// in the future (a clock that moved backward makes it untrustworthy).
fn passive_check_due(last_unix_seconds: Option<u64>, now_unix_seconds: u64) -> bool {
    last_unix_seconds.is_none_or(|last| {
        last > now_unix_seconds || now_unix_seconds - last >= PASSIVE_CHECK_INTERVAL.as_secs()
    })
}

/// Re-run the newest release's installer over the receipt-installed binary.
async fn run_dist_update(host: &Host) -> Result<UpdateOutcome> {
    let mut updater = configured_updater(host);
    let result = unblock_axoupdater(move || async move {
        updater.load_receipt()?;
        updater.run().await
    })
    .await
    .map_err(eyre::Report::new)?;
    match result {
        Some(result) => Ok(UpdateOutcome::Updated {
            previous: result.old_version,
            installed: result.new_version,
        }),
        None => Ok(UpdateOutcome::UpToDate {
            current: current_version(),
        }),
    }
}

/// The newest version the release source for `source` lists.
async fn query_latest(host: &Host, source: InstallSource) -> Result<Version> {
    let mut updater = configured_updater(host);
    let latest = unblock_axoupdater(move || async move {
        match source {
            InstallSource::Dist => {
                updater.load_receipt()?;
            }
            _ => {
                updater.set_release_source(github_release_source());
            }
        }
        updater
            .query_new_version()
            .await
            .map(Option::<&Version>::cloned)
    })
    .await
    .map_err(eyre::Report::new)?;
    latest.ok_or_else(|| eyre::eyre!("the release source lists no releases"))
}

/// An [`AxoUpdater`] for this app, carrying a GitHub token when the host
/// declares one — axoupdater's own recommendation for CI rate limits.
fn configured_updater(host: &Host) -> AxoUpdater {
    let mut updater = AxoUpdater::new_for(APP_NAME);
    if let Some(token) = host.env_string("WATERUI_GITHUB_TOKEN") {
        updater.set_github_token(&token);
    }
    updater
}

/// The release source a receipt would name, constructed explicitly for
/// installs no receipt covers.
fn github_release_source() -> ReleaseSource {
    ReleaseSource {
        release_type: ReleaseSourceType::GitHub,
        owner: RELEASE_OWNER.to_owned(),
        name: RELEASE_REPO.to_owned(),
        app_name: APP_NAME.to_owned(),
    }
}

/// Run an axoupdater call to completion on the blocking thread
/// [`smol::unblock`] provides.
///
/// axoupdater's futures are reqwest-based and need a tokio reactor the
/// smol-based CLI does not run, so each call builds a scratch current-thread
/// runtime here; the `unblock` hop keeps the calling executor free to observe
/// cancellation while the updater works.
async fn unblock_axoupdater<Fut, T>(f: impl FnOnce() -> Fut + Send + 'static) -> T
where
    Fut: std::future::Future<Output = T>,
    T: Send + 'static,
{
    smol::unblock(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio current-thread runtime for axoupdater")
            .block_on(f())
    })
    .await
}

/// The version this binary was built as.
fn current_version() -> Version {
    env!("CARGO_PKG_VERSION")
        .parse()
        .expect("package version is semver")
}

/// The `install_prefix` the first found install receipt records, or `None`
/// when no receipt exists. A receipt that exists but cannot be read is an
/// error — a corrupt receipt is not evidence of a different channel.
fn receipt_install_prefix(host: &Host) -> Result<Option<PathBuf>> {
    for dir in receipt_dirs(host) {
        let path = dir.join(format!("{APP_NAME}-receipt.json"));
        if !path.is_file() {
            continue;
        }
        let contents = std::fs::read_to_string(&path).wrap_err_with(|| {
            format!("the install receipt at {} cannot be read", path.display())
        })?;
        let receipt: ReceiptPrefix = serde_json::from_str(&contents)
            .wrap_err_with(|| format!("the install receipt at {} is invalid", path.display()))?;
        return Ok(Some(PathBuf::from(receipt.install_prefix)));
    }
    Ok(None)
}

/// The `install_prefix` a dist receipt records — the only field detection
/// needs; axoupdater re-parses the whole receipt when it runs the update.
#[derive(Deserialize)]
struct ReceiptPrefix {
    install_prefix: String,
}

/// The directories that may hold `<app>-receipt.json`, in axoupdater's own
/// search order: the `AXOUPDATER_*` overrides first, then
/// `$XDG_CONFIG_HOME` (existing dirs only) ahead of the platform default —
/// `~/.config` on Unix, `%LOCALAPPDATA%` on Windows.
fn receipt_dirs(host: &Host) -> Vec<PathBuf> {
    if host.env("AXOUPDATER_CONFIG_WORKING_DIR").is_some() {
        return vec![host.cwd().to_owned()];
    }
    if let Some(path) = host.env_string("AXOUPDATER_CONFIG_PATH") {
        return vec![PathBuf::from(path)];
    }
    let mut dirs = Vec::new();
    if cfg!(windows) {
        if let Some(local) = host.env_string("LOCALAPPDATA") {
            dirs.push(Path::new(&local).join(APP_NAME));
        }
    } else {
        if let Some(xdg) = host.env_string("XDG_CONFIG_HOME") {
            let dir = Path::new(&xdg).join(APP_NAME);
            if dir.is_dir() {
                dirs.push(dir);
            }
        }
        if let Some(home) = host.home_dir() {
            dirs.push(home.join(".config").join(APP_NAME));
        }
    }
    dirs
}

/// The Homebrew prefixes that could own a binary: `$HOMEBREW_PREFIX` (set by
/// `brew shellenv`, so custom-prefix installs are covered) plus the prefix a
/// `brew` on this host's `PATH` resolves to — `brew` always lives at
/// `<prefix>/bin/brew`, so its parent's parent is the prefix and no
/// well-known locations need guessing. Windows has no Homebrew.
fn homebrew_prefixes(host: &Host) -> Vec<PathBuf> {
    let mut prefixes = Vec::new();
    if let Some(prefix) = host.env_string("HOMEBREW_PREFIX") {
        prefixes.push(PathBuf::from(prefix));
    }
    let paths = host.path_entries();
    if !paths.is_empty()
        && let Ok(path) = std::env::join_paths(&paths)
        && let Ok(brew) = which::which_in("brew", Some(path), host.cwd())
        && let Some(prefix) = canonicalize_or_self(&brew).parent().and_then(Path::parent)
    {
        prefixes.push(prefix.to_path_buf());
    }
    prefixes
}

/// The directory `cargo install` and `cargo binstall` write binaries to:
/// `$CARGO_HOME/bin`, or `~/.cargo/bin` when `CARGO_HOME` is unset.
fn cargo_bin_dir(host: &Host) -> Option<PathBuf> {
    if let Some(cargo_home) = host.env_string("CARGO_HOME") {
        return Some(PathBuf::from(cargo_home).join("bin"));
    }
    host.home_dir().map(|home| home.join(".cargo").join("bin"))
}

/// Whether `executable` lives under the receipt's `install_prefix`, matching
/// axoupdater's own normalization: strip the executable's `bin` parent only
/// when the prefix is not itself a `bin` directory, so both the `cargo-home`
/// layout (prefix `~/.cargo`, binary in `bin/`) and the `flat` layout
/// (prefix is the `bin` dir itself) match.
fn same_install_root(executable: &Path, install_prefix: &Path) -> bool {
    let exe_dir = executable.parent().unwrap_or(executable);
    let exe_root = if exe_dir.file_name() == Some(OsStr::new("bin"))
        && install_prefix.file_name() != Some(OsStr::new("bin"))
    {
        exe_dir.parent().unwrap_or(exe_dir)
    } else {
        exe_dir
    };
    exe_root == install_prefix
}

/// Canonicalize when the path exists, keep it verbatim otherwise — the
/// normalization axoupdater applies to receipt paths.
fn canonicalize_or_self(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::testing::TestMachine;

    /// A realistic cargo-dist receipt body; detection reads only
    /// `install_prefix` out of it.
    fn receipt_json(install_prefix: &Path) -> String {
        serde_json::json!({
            "binaries": ["water"],
            "install_layout": "cargo-home",
            "install_prefix": install_prefix,
            "modify_path": true,
            "provider": { "source": "cargo-dist", "version": "0.30.2" },
            "source": {
                "app_name": "waterui-cli",
                "name": "cli",
                "owner": "water-rs",
                "release_type": "github",
            },
            "version": "0.3.2",
        })
        .to_string()
    }

    /// Write a receipt for `install_prefix` where the platform's receipt
    /// search finds it, and return the host vars that make it visible.
    fn stage_receipt(machine: &TestMachine, install_prefix: &Path) -> Vec<(String, String)> {
        let contents = receipt_json(install_prefix);
        if cfg!(windows) {
            let local = machine.dir("localappdata");
            machine.file(
                Path::new("localappdata")
                    .join(APP_NAME)
                    .join(format!("{APP_NAME}-receipt.json")),
                &contents,
            );
            vec![("LOCALAPPDATA".to_owned(), local.display().to_string())]
        } else {
            machine.file(
                Path::new("home/.config")
                    .join(APP_NAME)
                    .join(format!("{APP_NAME}-receipt.json")),
                &contents,
            );
            Vec::new()
        }
    }

    /// A dist install receipt covering the executable — the self-update row.
    #[test]
    fn receipt_covering_the_executable_is_a_dist_install() {
        let machine = TestMachine::new();
        let install = machine.dir("install");
        let exe = machine.file("install/bin/water", "");
        let vars = stage_receipt(&machine, &install);
        let host = machine.host(vars);
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Dist
        );
    }

    /// dist's `cargo-home` layout puts the binary in `CARGO_HOME/bin` too —
    /// the receipt, not the location, distinguishes it from `cargo install`.
    #[test]
    fn a_receipt_wins_over_the_cargo_bin_location() {
        let machine = TestMachine::new();
        let cargo_home = machine.dir("cargo");
        let exe = machine.file("cargo/bin/water", "");
        let mut vars = stage_receipt(&machine, &cargo_home);
        vars.push(("CARGO_HOME".to_owned(), cargo_home.display().to_string()));
        let host = machine.host(vars);
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Dist
        );
    }

    /// An executable resolving under the Homebrew prefix is `brew`-owned —
    /// `water update` must print `brew upgrade water` and change nothing.
    #[test]
    fn executable_under_the_homebrew_prefix_is_homebrew_owned() {
        let machine = TestMachine::new();
        let prefix = machine.dir("homebrew");
        let exe = machine.file("homebrew/bin/water", "");
        let host = machine.host([("HOMEBREW_PREFIX", prefix.display().to_string())]);
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Homebrew
        );
    }

    /// `brew` found on the host's `PATH` names its own prefix — the binary
    /// it lives beside is brew-owned even when `HOMEBREW_PREFIX` is unset.
    #[test]
    fn executable_beside_brew_on_the_path_is_homebrew_owned() {
        let machine = TestMachine::new();
        machine.install("brew");
        let exe = machine.file("bin/water", "");
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Homebrew
        );
    }

    /// A receipt for some *other* install must not shadow the package
    /// manager that owns this binary — the guard that keeps a stale receipt
    /// from authorizing a rewrite of a brew-owned file.
    #[test]
    fn a_receipt_for_another_install_does_not_shadow_the_package_manager() {
        let machine = TestMachine::new();
        let other_install = machine.dir("other-install");
        let prefix = machine.dir("homebrew");
        let exe = machine.file("homebrew/bin/water", "");
        let mut vars = stage_receipt(&machine, &other_install);
        vars.push(("HOMEBREW_PREFIX".to_owned(), prefix.display().to_string()));
        let host = machine.host(vars);
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Homebrew
        );
    }

    /// `CARGO_HOME/bin` without a receipt — the `cargo install` /
    /// `cargo binstall` row, updated with `cargo binstall waterui-cli`.
    #[test]
    fn executable_in_cargo_home_bin_without_a_receipt_is_cargo_owned() {
        let machine = TestMachine::new();
        let cargo_home = machine.dir("cargo");
        let exe = machine.file("cargo/bin/water", "");
        let host = machine.host([("CARGO_HOME", cargo_home.display().to_string())]);
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Cargo
        );
    }

    /// `~/.cargo/bin` without `CARGO_HOME` or a receipt is the same row.
    #[test]
    fn executable_in_default_cargo_bin_is_cargo_owned() {
        let machine = TestMachine::new();
        let exe = machine.file("home/.cargo/bin/water", "");
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Cargo
        );
    }

    /// No receipt, no Homebrew prefix, outside `CARGO_HOME/bin` — the
    /// "say so and stop" row.
    #[test]
    fn no_evidence_is_unknown() {
        let machine = TestMachine::new();
        let exe = machine.file("somewhere/water", "");
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(
            InstallSource::detect_exe(&host, &exe).unwrap(),
            InstallSource::Unknown
        );
    }

    /// A receipt that exists but is not JSON is an error, not evidence for
    /// another channel — the corrupt-receipt case must not silently classify.
    #[test]
    fn a_corrupt_receipt_is_an_error_not_a_guess() {
        let machine = TestMachine::new();
        if cfg!(windows) {
            machine.file(
                Path::new("localappdata")
                    .join(APP_NAME)
                    .join(format!("{APP_NAME}-receipt.json")),
                "not a receipt",
            );
        } else {
            machine.file(
                Path::new("home/.config")
                    .join(APP_NAME)
                    .join(format!("{APP_NAME}-receipt.json")),
                "not a receipt",
            );
        }
        let vars: Vec<(String, String)> = if cfg!(windows) {
            vec![(
                "LOCALAPPDATA".to_owned(),
                machine.root().join("localappdata").display().to_string(),
            )]
        } else {
            Vec::new()
        };
        let host = machine.host(vars);
        let exe = machine.file("home/.cargo/bin/water", "");
        assert!(InstallSource::detect_exe(&host, &exe).is_err());
    }

    #[test]
    fn install_source_labels_are_human_readable() {
        assert_eq!(InstallSource::Dist.label(), "release installer");
        assert_eq!(InstallSource::Homebrew.label(), "Homebrew");
        assert_eq!(InstallSource::Cargo.label(), "cargo");
        assert_eq!(InstallSource::Unknown.label(), "unknown");
    }

    /// The 24-hour gate: first check always runs, then once per interval.
    #[test]
    fn passive_check_is_due_at_most_once_per_interval() {
        let interval = PASSIVE_CHECK_INTERVAL.as_secs();
        assert!(passive_check_due(None, 1_000));
        assert!(!passive_check_due(Some(1_000), 1_000 + interval - 1));
        assert!(passive_check_due(Some(1_000), 1_000 + interval));
        assert!(passive_check_due(Some(1_000 + interval), 1_000));
    }
}
