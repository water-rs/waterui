//! Rust toolchain checks and remediation.

use std::path::{Path, PathBuf};

use semver::Version;

use crate::{
    toolchain::{Host, Installation, Toolchain, ToolchainError, UnfixableToolchain},
    utils::{CommandError, parse_semver_version},
};

/// The CLI's own `rust-version` — always part of the version floor the doctor
/// enforces, because the crates it generates are compiled with this release's
/// language features and edition.
pub const CLI_MINIMUM_RUST_VERSION: &str = env!("CARGO_PKG_RUST_VERSION");

/// Rust toolchain checker.
///
/// `minimum_version` is the rustc floor the build path must satisfy — the
/// caller (the doctor) derives it from the CLI's `rust-version` raised by
/// whatever the project's manifest and selected framework declare. The
/// `rust-toolchain.toml`/`rust-toolchain` pin is read from the host's working
/// directory at check time, matching how rustup resolves it for the commands
/// the build path spawns there.
#[derive(Debug, Clone)]
pub struct RustToolchain {
    minimum_version: String,
}

impl RustToolchain {
    /// A checker enforcing `minimum_version` as the rustc floor.
    #[must_use]
    pub fn new(minimum_version: &Version) -> Self {
        Self {
            minimum_version: minimum_version.to_string(),
        }
    }
}

impl Default for RustToolchain {
    fn default() -> Self {
        Self {
            minimum_version: String::from(CLI_MINIMUM_RUST_VERSION),
        }
    }
}

/// Installation plan for Rust toolchain fixes.
#[derive(Debug, Clone, Default)]
pub struct RustToolchainInstallation {
    /// `rustup default <channel>` — installs the channel and selects it; the
    /// repair for a rustup with no active toolchain and for a default pinned
    /// below the version floor.
    set_default: Option<String>,
    /// `rustup toolchain install <channel>` — installs the toolchain a
    /// `rust-toolchain.toml` pin names without touching the default.
    install_toolchain: Option<String>,
    /// `rustup update <toolchain>` — updates an installed moving-channel
    /// toolchain that fell behind the version floor.
    update_toolchain: Option<String>,
    /// `rustup target add --toolchain <toolchain> <target>` pairs.
    add_targets: Vec<(String, String)>,
    /// `rustup component add --toolchain <toolchain> <component>` pairs.
    add_components: Vec<(String, String)>,
}

impl RustToolchainInstallation {
    fn require_default_install(&mut self, channel: impl Into<String>) {
        self.set_default = Some(channel.into());
        self.install_toolchain = None;
        self.update_toolchain = None;
    }

    fn require_toolchain_install(&mut self, channel: impl Into<String>) {
        if self.set_default.is_none() {
            self.install_toolchain = Some(channel.into());
            self.update_toolchain = None;
        }
    }

    fn require_toolchain_update(&mut self, toolchain: String) {
        if self.set_default.is_none() && self.install_toolchain.is_none() {
            self.update_toolchain = Some(toolchain);
        }
    }

    fn require_target(&mut self, toolchain: &str, target: String) {
        self.add_targets.push((toolchain.to_owned(), target));
    }

    fn require_component(&mut self, toolchain: &str, component: String) {
        self.add_components.push((toolchain.to_owned(), component));
    }

    /// Returns `true` when at least one automatic fix action is planned.
    #[must_use]
    pub const fn has_actions(&self) -> bool {
        self.set_default.is_some()
            || self.install_toolchain.is_some()
            || self.update_toolchain.is_some()
            || !self.add_targets.is_empty()
            || !self.add_components.is_empty()
    }

    /// Human-readable summary of automatic fixes this installation will run.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut actions = Vec::new();
        if let Some(channel) = &self.set_default {
            actions.push(format!("install and select `rustup default {channel}`"));
        }
        if let Some(channel) = &self.install_toolchain {
            actions.push(format!("install `rustup toolchain install {channel}`"));
        }
        if let Some(toolchain) = &self.update_toolchain {
            actions.push(format!(
                "update `{toolchain}` via `rustup update {toolchain}`"
            ));
        }
        for (toolchain, target) in &self.add_targets {
            actions.push(format!(
                "add target via `rustup target add --toolchain {toolchain} {target}`"
            ));
        }
        for (toolchain, component) in &self.add_components {
            actions.push(format!(
                "add component via `rustup component add --toolchain {toolchain} {component}`"
            ));
        }

        if actions.is_empty() {
            String::from("no automatic actions required")
        } else {
            actions.join(", ")
        }
    }
}

/// Errors that can occur during Rust toolchain installation.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallRustToolchain {
    /// rustup was not found.
    #[error("rustup is required for automatic Rust toolchain fixes but is not on PATH.")]
    RustupNotFound,
    /// Failed to install and select the default toolchain.
    #[error("Failed to install default Rust toolchain `{toolchain}`: {source}")]
    SetDefault {
        /// Channel that failed to install.
        toolchain: String,
        /// Underlying command error.
        source: CommandError,
    },
    /// Failed to install a pinned toolchain.
    #[error("Failed to install Rust toolchain `{toolchain}`: {source}")]
    InstallToolchain {
        /// Channel that failed to install.
        toolchain: String,
        /// Underlying command error.
        source: CommandError,
    },
    /// Failed to update the active toolchain.
    #[error("Failed to update Rust toolchain `{toolchain}`: {source}")]
    UpdateToolchain {
        /// Active rustup toolchain that failed to update.
        toolchain: String,
        /// Underlying command error.
        source: CommandError,
    },
    /// Failed to add a compilation target.
    #[error("Failed to add Rust target `{target}` to toolchain `{toolchain}`: {source}")]
    AddTarget {
        /// Toolchain the target was added to.
        toolchain: String,
        /// Target triple that failed to install.
        target: String,
        /// Underlying command error.
        source: CommandError,
    },
    /// Failed to add a component.
    #[error("Failed to add Rust component `{component}` to toolchain `{toolchain}`: {source}")]
    AddComponent {
        /// Toolchain the component was added to.
        toolchain: String,
        /// Component that failed to install.
        component: String,
        /// Underlying command error.
        source: CommandError,
    },
}

/// Parsing rustup/rustc output or a version string failed.
#[derive(Debug, thiserror::Error)]
enum RustParseError {
    /// `rustup show active-toolchain` printed no toolchain token.
    #[error("expected `<toolchain> (<reason>)` output")]
    ActiveToolchain,
    /// `rustc --version` printed no version token.
    #[error("expected `rustc <version>` output")]
    RustcVersion,
    /// `rustc -vV` printed no `host:` line.
    #[error("missing `host:` line")]
    HostLine,
    /// The version string could not be normalized to semver.
    #[error(transparent)]
    Version(#[from] crate::utils::VersionParseError),
}

impl Installation for RustToolchainInstallation {
    type Error = FailToInstallRustToolchain;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if !self.has_actions() {
            return Ok(());
        }

        if host.which("rustup").await.is_err() {
            return Err(FailToInstallRustToolchain::RustupNotFound);
        }

        if let Some(channel) = &self.set_default {
            host.run("rustup", ["default", channel.as_str()])
                .await
                .map_err(|source| FailToInstallRustToolchain::SetDefault {
                    toolchain: channel.clone(),
                    source,
                })?;
        }

        if let Some(channel) = &self.install_toolchain {
            host.run("rustup", ["toolchain", "install", channel.as_str()])
                .await
                .map_err(|source| FailToInstallRustToolchain::InstallToolchain {
                    toolchain: channel.clone(),
                    source,
                })?;
        }

        if let Some(toolchain) = &self.update_toolchain {
            host.run("rustup", ["update", toolchain.as_str()])
                .await
                .map_err(|source| FailToInstallRustToolchain::UpdateToolchain {
                    toolchain: toolchain.clone(),
                    source,
                })?;
        }

        for (toolchain, target) in &self.add_targets {
            host.run(
                "rustup",
                [
                    "target",
                    "add",
                    "--toolchain",
                    toolchain.as_str(),
                    target.as_str(),
                ],
            )
            .await
            .map_err(|source| FailToInstallRustToolchain::AddTarget {
                toolchain: toolchain.clone(),
                target: target.clone(),
                source,
            })?;
        }

        for (toolchain, component) in &self.add_components {
            host.run(
                "rustup",
                [
                    "component",
                    "add",
                    "--toolchain",
                    toolchain.as_str(),
                    component.as_str(),
                ],
            )
            .await
            .map_err(|source| FailToInstallRustToolchain::AddComponent {
                toolchain: toolchain.clone(),
                component: component.clone(),
                source,
            })?;
        }

        Ok(())
    }
}

impl Toolchain for RustToolchain {
    type Installation = RustToolchainInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        let availability = detect_rust_tool_availability(host).await;
        ensure_minimum_rust_tools(availability)?;
        check_rustup_proxies(host, availability)?;

        let pin = read_toolchain_pin(host.cwd()).map_err(|malformed| {
            ToolchainError::unfixable(
                format!(
                    "{} is malformed: {}",
                    malformed.path.display(),
                    malformed.reason
                ),
                "rustup expects a `[toolchain]` table with a `channel` string and `components`/`targets` string arrays — fix the file or delete it.",
            )
        })?;

        let mut installation = RustToolchainInstallation::default();
        let selected =
            select_toolchain(host, availability, pin.as_ref(), &mut installation).await?;
        check_rustc_version(
            host,
            &self.minimum_version,
            pin.as_ref(),
            &selected,
            &mut installation,
        )
        .await?;
        check_required_targets_and_components(host, pin.as_ref(), &selected, &mut installation)
            .await?;

        installation
            .has_actions()
            .then_some(ToolchainError::fixable(installation))
            .map_or(Ok(()), Err)
    }
}

#[derive(Debug, Clone, Copy)]
struct RustToolAvailability {
    rustup_available: bool,
    cargo_available: bool,
    rustc_available: bool,
}

/// The toolchain the build path resolves for the host's working directory.
#[derive(Debug)]
enum SelectedToolchain {
    /// rustup resolved a toolchain (the project pin or the rustup default).
    Rustup(String),
    /// No rustup on PATH; `rustc`/`cargo` are standalone binaries.
    Standalone,
}

fn ensure_minimum_rust_tools(
    availability: RustToolAvailability,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    if !availability.rustup_available
        && (!availability.cargo_available || !availability.rustc_available)
    {
        return Err(ToolchainError::unfixable(
            "Rust toolchain is incomplete (`cargo` and/or `rustc` is missing from PATH).",
            "Install rustup from https://rustup.rs, then run `rustup default stable`.",
        ));
    }
    Ok(())
}

async fn detect_rust_tool_availability(host: &Host) -> RustToolAvailability {
    RustToolAvailability {
        rustup_available: host.which("rustup").await.is_ok(),
        cargo_available: host.which("cargo").await.is_ok(),
        rustc_available: host.which("rustc").await.is_ok(),
    }
}

/// The directory rustup writes its `cargo`/`rustc` proxies into on this host:
/// `$CARGO_HOME/bin`, falling back to `~/.cargo/bin`.
fn cargo_bin_dir(host: &Host) -> Option<PathBuf> {
    if let Some(cargo_home) = host.env_string("CARGO_HOME") {
        return Some(PathBuf::from(cargo_home).join("bin"));
    }
    host.home_dir().map(|home| home.join(".cargo/bin"))
}

/// The directory rustup unpacks toolchains into on this host:
/// `$RUSTUP_HOME/toolchains`, falling back to `~/.rustup/toolchains`.
pub(crate) fn rustup_toolchains_dir(host: &Host) -> Option<PathBuf> {
    host.env_string("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| host.home_dir().map(|home| home.join(".rustup")))
        .map(|root| root.join("toolchains"))
}

/// When `rustup` is reachable but `cargo`/`rustc` are not, the rustup proxies
/// either sit in a directory missing from PATH or were never created. Both
/// are manual repairs: doctor does not edit shell profiles or reinstall
/// rustup.
fn check_rustup_proxies(
    host: &Host,
    availability: RustToolAvailability,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    if (availability.cargo_available && availability.rustc_available)
        || !availability.rustup_available
    {
        return Ok(());
    }

    let missing: Vec<&str> = [
        ("cargo", availability.cargo_available),
        ("rustc", availability.rustc_available),
    ]
    .into_iter()
    .filter_map(|(name, present)| (!present).then_some(name))
    .collect();

    let Some(bin_dir) = cargo_bin_dir(host) else {
        return Err(ToolchainError::unfixable(
            format!(
                "rustup is on PATH but its {} proxies are not, and no cargo home could be located.",
                missing.join("`/`")
            ),
            "Reinstall rustup from https://rustup.rs so its proxies are installed, then ensure the directory is on PATH.",
        ));
    };

    let absent: Vec<&str> = missing
        .iter()
        .copied()
        .filter(|name| !bin_dir.join(tool_binary_name(name)).is_file())
        .collect();
    if absent.is_empty() {
        return Err(ToolchainError::unfixable(
            format!(
                "rustup proxies exist in {} but that directory is not on PATH, so `{}` {} unreachable.",
                bin_dir.display(),
                missing.join("`, `"),
                if missing.len() == 1 { "is" } else { "are" },
            ),
            format!(
                "Add `{}` to PATH (e.g. `export PATH=\"{}:$PATH\"` in your shell profile).",
                bin_dir.display(),
                bin_dir.display()
            ),
        ));
    }

    Err(ToolchainError::unfixable(
        format!(
            "rustup is on PATH but the `{}` {} do not exist under {}.",
            absent.join("`, `"),
            if absent.len() == 1 {
                "proxy"
            } else {
                "proxies"
            },
            bin_dir.display()
        ),
        "Re-run the rustup installer from https://rustup.rs (or `rustup-init`) so the proxies are created, then ensure the directory is on PATH.",
    ))
}

fn tool_binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The `rust-toolchain.toml`/`rust-toolchain` override applying to a
/// directory — the same file rustup reads when the build path spawns
/// `cargo`/`rustc` there.
#[derive(Debug)]
struct ToolchainPin {
    /// `channel = "…"`, when the file declares one.
    channel: Option<String>,
    /// `targets = […]` the toolchain must carry.
    targets: Vec<String>,
    /// `components = […]` the toolchain must carry.
    components: Vec<String>,
}

/// A `rust-toolchain` file that exists but cannot be interpreted.
#[derive(Debug)]
struct MalformedToolchainPin {
    path: PathBuf,
    reason: String,
}

/// Read the toolchain pin applying to `dir`, walking ancestors the way rustup
/// does: the nearest `rust-toolchain.toml` or `rust-toolchain` wins.
fn read_toolchain_pin(dir: &Path) -> Result<Option<ToolchainPin>, MalformedToolchainPin> {
    for ancestor in dir.ancestors() {
        for name in ["rust-toolchain.toml", "rust-toolchain"] {
            let path = ancestor.join(name);
            if !path.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|error| MalformedToolchainPin {
                path: path.clone(),
                reason: format!("cannot be read: {error}"),
            })?;
            return parse_toolchain_pin(&text, name == "rust-toolchain.toml", &path).map(Some);
        }
    }
    Ok(None)
}

fn parse_toolchain_pin(
    text: &str,
    toml_file: bool,
    path: &Path,
) -> Result<ToolchainPin, MalformedToolchainPin> {
    let malformed = |reason: String| MalformedToolchainPin {
        path: path.to_path_buf(),
        reason,
    };

    let trimmed = text.trim();
    if !toml_file && !trimmed.starts_with('[') && !trimmed.contains('=') {
        // The legacy `rust-toolchain` file may hold a bare channel name.
        return Ok(ToolchainPin {
            channel: (!trimmed.is_empty()).then(|| trimmed.to_owned()),
            targets: Vec::new(),
            components: Vec::new(),
        });
    }

    let document: toml::Value =
        toml::from_str(text).map_err(|error| malformed(format!("invalid TOML: {error}")))?;
    let table = match document.get("toolchain") {
        Some(value) => value
            .as_table()
            .ok_or_else(|| malformed("`toolchain` must be a table".to_owned()))?,
        // A pin file may declare `channel`/`targets`/`components` at the top
        // level; rustup treats the whole document as the toolchain table.
        None => document
            .as_table()
            .ok_or_else(|| malformed("the file must be a TOML table".to_owned()))?,
    };

    let mut pin = ToolchainPin {
        channel: None,
        targets: Vec::new(),
        components: Vec::new(),
    };
    if let Some(channel) = table.get("channel") {
        pin.channel = Some(
            channel
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| malformed("`toolchain.channel` must be a string".to_owned()))?,
        );
    }
    if pin.channel.is_none() {
        // rustup requires `channel` in a TOML toolchain file.
        return Err(malformed("`toolchain.channel` is required".to_owned()));
    }
    for (key, slot) in [
        ("targets", &mut pin.targets),
        ("components", &mut pin.components),
    ] {
        let Some(value) = table.get(key) else {
            continue;
        };
        let entries = value
            .as_array()
            .ok_or_else(|| malformed(format!("`toolchain.{key}` must be an array")))?;
        for entry in entries {
            slot.push(
                entry.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                    malformed(format!("`toolchain.{key}` entries must be strings"))
                })?,
            );
        }
    }
    Ok(pin)
}

/// How a `channel` value maps onto rustup's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelKind {
    /// `stable`/`beta`/`nightly` — rustup installs and updates it.
    Moving,
    /// `stable-`/`beta-`/`nightly-YYYY-MM-DD` — installable, never updates.
    Dated,
    /// `1.85`, `1.85.0`, optionally with a host suffix — installable, never
    /// updates past the pinned version.
    Version,
    /// A custom toolchain name (a linked toolchain, `esp`, `stage0`, …) —
    /// rustup cannot install it; its provider does.
    Custom,
}

fn classify_channel(channel: &str) -> ChannelKind {
    match channel {
        "stable" | "beta" | "nightly" => ChannelKind::Moving,
        _ if channel
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit()) =>
        {
            ChannelKind::Version
        }
        _ if is_dated_channel(channel) => ChannelKind::Dated,
        _ => ChannelKind::Custom,
    }
}

fn is_dated_channel(channel: &str) -> bool {
    let Some((name, date)) = channel.split_once('-') else {
        return false;
    };
    matches!(name, "stable" | "beta" | "nightly") && is_iso_date(date)
}

fn is_iso_date(value: &str) -> bool {
    let is_digits = |part: &str, len: usize| {
        part.len() == len && part.bytes().all(|byte| byte.is_ascii_digit())
    };
    matches!(
        value.split('-').collect::<Vec<_>>().as_slice(),
        [year, month, day]
            if is_digits(year, 4) && is_digits(month, 2) && is_digits(day, 2)
    )
}

/// Whether rustup can manage targets and components on a resolved toolchain
/// name — `stable-aarch64-apple-darwin`, `nightly-2024-01-01-…`, `1.85.0-…`
/// are managed; linked/custom names like `esp` or `stage0` are not.
pub(crate) fn toolchain_is_rustup_managed(name: &str) -> bool {
    let channel = name.split('-').next().unwrap_or(name);
    matches!(channel, "stable" | "beta" | "nightly")
        || channel
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
}

/// The toolchain rustup resolves for the host's working directory — the
/// `rust-toolchain.toml` override when one is installed, else the rustup
/// default.
///
/// Shared by every check that must qualify `rustup` commands with
/// `--toolchain <name>`.
pub(crate) async fn selected_rustup_toolchain(host: &Host) -> Result<String, UnfixableToolchain> {
    if host.which("rustup").await.is_err() {
        return Err(UnfixableToolchain::new(
            "rustup is not installed, so no Rust toolchain is selected",
            "Install rustup from https://rustup.rs, then run `rustup default stable`.",
        ));
    }
    let output = host
        .run("rustup", ["show", "active-toolchain"])
        .await
        .map_err(|error| {
            UnfixableToolchain::new(
                format!("rustup cannot resolve a toolchain for this directory: {error}"),
                "Fix the `rust` doctor item first (the pinned or default toolchain is missing).",
            )
        })?;
    parse_active_toolchain(&output).map_err(|error| {
        UnfixableToolchain::new(
            format!("Could not parse the active rustup toolchain: {error}"),
            "Run `rustup show active-toolchain`; repair or reinstall rustup if it does not return a toolchain name.",
        )
    })
}

/// The rustup toolchain a project's own directory selects.
///
/// The generated crates a build compiles live in the build cache, outside the
/// project tree, so rustup would resolve them to its default toolchain rather
/// than the project's `rust-toolchain.toml`; every cargo and rustc invocation
/// made on the project's behalf names this toolchain explicitly instead.
///
/// # Errors
/// Returns an error when rustup is missing or resolves no toolchain for the
/// project directory.
pub(crate) async fn project_rustup_toolchain(
    project_root: &Path,
) -> Result<String, UnfixableToolchain> {
    selected_rustup_toolchain(&Host::current().with_cwd(project_root)).await
}

/// Targets installed on `toolchain`, via `rustup target list --installed`.
pub(crate) async fn installed_rustup_targets(
    host: &Host,
    toolchain: &str,
) -> Result<Vec<String>, UnfixableToolchain> {
    let output = host
        .run(
            "rustup",
            ["target", "list", "--installed", "--toolchain", toolchain],
        )
        .await
        .map_err(|error| {
            UnfixableToolchain::new(
                format!("Failed to list installed Rust targets for `{toolchain}`: {error}"),
                "Run `rustup target list --installed`; if it fails, repair rustup with `rustup self update` or reinstall rustup.",
            )
        })?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

/// Components installed on `toolchain`, via `rustup component list
/// --installed`. rustup prints component names with the toolchain's target
/// suffix (`clippy-aarch64-apple-darwin`), so membership tests use
/// [`component_is_installed`].
async fn installed_rustup_components(
    host: &Host,
    toolchain: &str,
) -> Result<Vec<String>, UnfixableToolchain> {
    let output = host
        .run(
            "rustup",
            [
                "component",
                "list",
                "--installed",
                "--toolchain",
                toolchain,
            ],
        )
        .await
        .map_err(|error| {
            UnfixableToolchain::new(
                format!("Failed to list installed Rust components for `{toolchain}`: {error}"),
                "Run `rustup component list --installed`; if it fails, repair rustup with `rustup self update` or reinstall rustup.",
            )
        })?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

/// `rustup component list` prints either a bare component name (`rust-src`)
/// or `<component>-<host>` (`clippy-aarch64-apple-darwin`). A declared
/// component is installed when a line matches one of the two exactly — a
/// prefix match would count `rustfmt-preview` as `rustfmt`.
fn component_is_installed(installed: &str, component: &str, host_target: &str) -> bool {
    installed == component || installed == format!("{component}-{host_target}")
}

/// Installation plan adding rustup targets to a named toolchain.
#[derive(Debug, Clone)]
pub struct RustTargetAdditions {
    toolchain: String,
    targets: Vec<String>,
}

impl RustTargetAdditions {
    /// Plan `rustup target add --toolchain <toolchain>` for each of `targets`.
    #[must_use]
    pub const fn new(toolchain: String, targets: Vec<String>) -> Self {
        Self { toolchain, targets }
    }
}

/// Errors from `rustup target add`.
#[derive(Debug, thiserror::Error)]
pub enum FailToAddRustTargets {
    /// rustup was not found.
    #[error("rustup is required to add Rust targets but is not on PATH.")]
    RustupNotFound,
    /// A target could not be added.
    #[error("Failed to add Rust target `{target}` to toolchain `{toolchain}`: {source}")]
    AddTarget {
        /// Toolchain the target was added to.
        toolchain: String,
        /// Target triple that failed to install.
        target: String,
        /// Underlying command error.
        source: CommandError,
    },
}

impl Installation for RustTargetAdditions {
    type Error = FailToAddRustTargets;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if host.which("rustup").await.is_err() {
            return Err(FailToAddRustTargets::RustupNotFound);
        }
        for target in &self.targets {
            host.run(
                "rustup",
                [
                    "target",
                    "add",
                    "--toolchain",
                    self.toolchain.as_str(),
                    target.as_str(),
                ],
            )
            .await
            .map_err(|source| FailToAddRustTargets::AddTarget {
                toolchain: self.toolchain.clone(),
                target: target.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

/// A required set of rustup targets on the toolchain selected for the host's
/// working directory — how platform checkers verify the compilation targets
/// the build path invokes.
#[derive(Debug, Clone)]
pub struct SelectedToolchainTargets {
    required: Vec<String>,
}

impl SelectedToolchainTargets {
    /// Check that `required` targets are installed on the selected toolchain.
    #[must_use]
    pub const fn new(required: Vec<String>) -> Self {
        Self { required }
    }
}

impl Toolchain for SelectedToolchainTargets {
    type Installation = RustTargetAdditions;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        let toolchain = selected_rustup_toolchain(host).await?;
        if !toolchain_is_rustup_managed(&toolchain) {
            return Err(ToolchainError::unfixable(
                format!(
                    "the selected toolchain `{toolchain}` is not rustup-managed, so targets cannot be verified or added"
                ),
                "The project pins a custom toolchain; install its targets through the toolchain's provider.",
            ));
        }
        let installed = installed_rustup_targets(host, &toolchain).await?;
        let missing: Vec<String> = self
            .required
            .iter()
            .filter(|target| !installed.contains(*target))
            .cloned()
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ToolchainError::fixable(RustTargetAdditions::new(
                toolchain, missing,
            )))
        }
    }
}

async fn select_toolchain(
    host: &Host,
    availability: RustToolAvailability,
    pin: Option<&ToolchainPin>,
    installation: &mut RustToolchainInstallation,
) -> Result<SelectedToolchain, ToolchainError<RustToolchainInstallation>> {
    if !availability.rustup_available {
        return Ok(SelectedToolchain::Standalone);
    }

    match host.run("rustup", ["show", "active-toolchain"]).await {
        Ok(output) => parse_active_toolchain(&output)
            .map(SelectedToolchain::Rustup)
            .map_err(|error| {
                ToolchainError::unfixable(
                    format!("Could not parse the active rustup toolchain: {error}"),
                    "Run `rustup show active-toolchain`; repair or reinstall rustup if it does not return a toolchain name.",
                )
            }),
        Err(error) => {
            let error_message = error.to_string();
            if error_message.contains("not installed") {
                missing_pinned_toolchain(pin, installation, &error_message)
            } else if is_no_active_toolchain_error(&error_message) {
                installation.require_default_install("stable");
                Err(ToolchainError::fixable(std::mem::take(installation)))
            } else {
                Err(ToolchainError::unfixable(
                    format!(
                        "rustup is installed but cannot report an active toolchain: {error_message}"
                    ),
                    "Run `rustup self update` and `rustup toolchain install stable`; if that fails, reinstall rustup from https://rustup.rs.",
                ))
            }
        }
    }
}

/// `rustup show active-toolchain` failed because the pinned (or defaulted)
/// toolchain is not installed. When the pin names a channel rustup cannot
/// install — `esp` being the common case — the repair is manual and names
/// the toolchain's own installer.
fn missing_pinned_toolchain(
    pin: Option<&ToolchainPin>,
    installation: &mut RustToolchainInstallation,
    error_message: &str,
) -> Result<SelectedToolchain, ToolchainError<RustToolchainInstallation>> {
    let Some(channel) = pin.and_then(|pin| pin.channel.clone()) else {
        // No pin, yet the recorded default is not installed: installing
        // `stable` alone leaves no default selected, so install and select it.
        installation.require_default_install("stable");
        return Err(ToolchainError::fixable(std::mem::take(installation)));
    };
    if classify_channel(&channel) == ChannelKind::Custom {
        return Err(ToolchainError::unfixable(
            format!("rust-toolchain pin `{channel}` is not a rustup channel: {error_message}"),
            if channel == "esp" {
                "Install the Espressif Rust toolchain with `espup install` (install `espup` first with `cargo install espup`)."
            } else {
                "Install the toolchain through its provider; rustup only installs stable/beta/nightly and released versions."
            },
        ));
    }
    installation.require_toolchain_install(channel);
    Err(ToolchainError::fixable(std::mem::take(installation)))
}

async fn check_rustc_version(
    host: &Host,
    minimum_version: &str,
    pin: Option<&ToolchainPin>,
    selected: &SelectedToolchain,
    installation: &mut RustToolchainInstallation,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    let version_output = host.run("rustc", ["--version"]).await.map_err(|error| {
        let error_message = error.to_string();
        rustc_run_error(pin, selected, &error_message)
    })?;
    let installed_version = parse_installed_rustc_version(&version_output)?;
    let required_version = parse_required_rustc_version(minimum_version)?;

    if installed_version >= required_version {
        return Ok(());
    }

    let channel = pin.and_then(|pin| pin.channel.as_deref());
    match (selected, channel.map(classify_channel)) {
        (SelectedToolchain::Standalone, _) => Err(ToolchainError::unfixable(
            format!(
                "Detected Rust {installed_version}, but the project requires at least Rust {required_version}."
            ),
            format!(
                "Install Rust {required_version} or newer. Recommended: install rustup from https://rustup.rs, then run `rustup update stable`."
            ),
        )),
        (SelectedToolchain::Rustup(_), Some(ChannelKind::Custom)) => {
            Err(ToolchainError::unfixable(
                format!(
                    "The pinned toolchain `{}` provides Rust {installed_version}, below the required {required_version}.",
                    channel.unwrap_or_default()
                ),
                if channel == Some("esp") {
                    String::from("Update the Espressif Rust toolchain with `espup update`.")
                } else {
                    String::from(
                        "Update the pinned toolchain through its provider, or raise the floor in `rust-toolchain.toml`.",
                    )
                },
            ))
        }
        (SelectedToolchain::Rustup(_), Some(ChannelKind::Version | ChannelKind::Dated)) => {
            Err(ToolchainError::unfixable(
                format!(
                    "The project pins Rust toolchain `{}` (providing {installed_version}), below the required {required_version}.",
                    channel.unwrap_or_default()
                ),
                format!(
                    "Update the `channel` in `rust-toolchain.toml` to a release providing Rust {required_version} or newer."
                ),
            ))
        }
        (SelectedToolchain::Rustup(name), Some(ChannelKind::Moving)) => {
            installation.require_toolchain_update(name.clone());
            Ok(())
        }
        (SelectedToolchain::Rustup(name), None) => {
            if toolchain_is_rustup_managed(name) && !is_version_or_dated_name(name) {
                installation.require_toolchain_update(name.clone());
            } else if toolchain_is_rustup_managed(name) {
                // The rustup default is pinned to a version or date that no
                // update can move past; select `stable` instead.
                installation.require_default_install("stable");
            } else {
                return Err(ToolchainError::unfixable(
                    format!(
                        "The default toolchain `{name}` provides Rust {installed_version}, below the required {required_version}."
                    ),
                    format!(
                        "`{name}` is a custom toolchain; select a rustup channel with `rustup default stable` or update the custom toolchain through its provider."
                    ),
                ));
            }
            Ok(())
        }
    }
}

/// Whether a resolved toolchain name pins a version or a date, so
/// `rustup update` cannot move it past the floor.
fn is_version_or_dated_name(name: &str) -> bool {
    let mut segments = name.split('-');
    let channel = segments.next().unwrap_or(name);
    if channel
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
    {
        return true;
    }
    if !matches!(channel, "stable" | "beta" | "nightly") {
        return false;
    }
    // `<channel>-YYYY-MM-DD[-<host>]` names pin a date.
    let is_digits = |segment: Option<&str>, len: usize| {
        segment.is_some_and(|segment| {
            segment.len() == len && segment.bytes().all(|byte| byte.is_ascii_digit())
        })
    };
    is_digits(segments.next(), 4) && is_digits(segments.next(), 2) && is_digits(segments.next(), 2)
}

fn rustc_run_error(
    pin: Option<&ToolchainPin>,
    selected: &SelectedToolchain,
    error_message: &str,
) -> ToolchainError<RustToolchainInstallation> {
    let detail = format!("`rustc` exists on PATH but failed to run: {error_message}");
    match (pin.and_then(|pin| pin.channel.as_deref()), selected) {
        (Some(channel), SelectedToolchain::Rustup(_)) => ToolchainError::unfixable(
            detail,
            format!(
                "Reinstall the pinned toolchain with `rustup toolchain install {channel} --force`."
            ),
        ),
        (None, SelectedToolchain::Rustup(name)) => ToolchainError::unfixable(
            detail,
            format!("Reinstall the toolchain with `rustup toolchain install {name} --force`."),
        ),
        (_, SelectedToolchain::Standalone) => {
            ToolchainError::unfixable(detail, "Reinstall Rust via rustup from https://rustup.rs.")
        }
    }
}

fn parse_installed_rustc_version(
    version_output: &str,
) -> Result<Version, ToolchainError<RustToolchainInstallation>> {
    parse_rustc_version(version_output).map_err(|error| {
        ToolchainError::unfixable(
            format!(
                "Failed to parse `rustc --version` output `{}`: {error}",
                version_output.trim()
            ),
            "Run `rustc --version` manually. If output is malformed, reinstall rustup from https://rustup.rs.",
        )
    })
}

fn parse_required_rustc_version(
    minimum_version: &str,
) -> Result<Version, ToolchainError<RustToolchainInstallation>> {
    parse_semver_version(minimum_version).map_err(|error| {
        ToolchainError::unfixable(
            format!("Invalid required Rust version `{minimum_version}`: {error}"),
            "Reinstall waterui-cli from source to restore a valid embedded Rust requirement.",
        )
    })
}

async fn check_required_targets_and_components(
    host: &Host,
    pin: Option<&ToolchainPin>,
    selected: &SelectedToolchain,
    installation: &mut RustToolchainInstallation,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    let SelectedToolchain::Rustup(name) = selected else {
        return Ok(());
    };
    if !toolchain_is_rustup_managed(name) {
        // Custom toolchains (esp, linked) carry their own standard library;
        // rustup cannot add targets or components to them.
        return Ok(());
    }

    let host_target = host
        .run("rustc", ["-vV"])
        .await
        .map_err(|error| {
            ToolchainError::unfixable(
                format!("`rustc -vV` failed: {error}"),
                "Run `rustc -vV` manually; if it fails, reinstall rustup from https://rustup.rs.",
            )
        })
        .and_then(|output| {
            parse_host_target(&output).map_err(|error| {
                ToolchainError::unfixable(
                    format!("Could not parse host target from `rustc -vV`: {error}"),
                    "Ensure `rustc -vV` includes a `host: <target>` line; reinstall rustup if the output is incomplete.",
                )
            })
        })?;

    let mut required_targets: Vec<String> = vec![host_target.clone()];
    if let Some(pin) = pin {
        for target in &pin.targets {
            if !required_targets.contains(target) {
                required_targets.push(target.clone());
            }
        }
    }

    let installed_targets = installed_rustup_targets(host, name).await?;
    for target in required_targets {
        if !installed_targets.contains(&target) {
            installation.require_target(name, target);
        }
    }

    if let Some(pin) = pin
        && !pin.components.is_empty()
    {
        let installed_components = installed_rustup_components(host, name).await?;
        for component in &pin.components {
            if !installed_components
                .iter()
                .any(|installed| component_is_installed(installed, component, &host_target))
            {
                installation.require_component(name, component.clone());
            }
        }
    }

    Ok(())
}

fn parse_active_toolchain(output: &str) -> Result<String, RustParseError> {
    output
        .split_whitespace()
        .next()
        .filter(|toolchain| !toolchain.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(RustParseError::ActiveToolchain)
}

fn parse_rustc_version(output: &str) -> Result<Version, RustParseError> {
    let version_token = output
        .split_whitespace()
        .nth(1)
        .ok_or(RustParseError::RustcVersion)?;
    Ok(parse_semver_version(version_token)?)
}

fn parse_host_target(output: &str) -> Result<String, RustParseError> {
    output
        .lines()
        .find_map(|line| {
            line.strip_prefix("host:")
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(ToOwned::to_owned)
        })
        .ok_or(RustParseError::HostLine)
}

fn is_no_active_toolchain_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("no active toolchain") || normalized.contains("no default toolchain")
}

/// A nightly toolchain that can compile the standard library from source.
///
/// `-Zbuild-std` needs a nightly cargo and the `rust-src` component. The
/// Android preview needs both: the only shared `libstd` rustup ships is
/// 4 KB-aligned, and a device with 16 KB pages refuses to map it, so the
/// preview runtime builds `std` itself under the page-size link flag.
///
/// The choice is deterministic — the plain `nightly` channel first, then the
/// newest dated nightly — because the support app and the preview module are
/// separate Cargo invocations that must land on the same `libstd-<hash>.so`.
///
/// # Errors
/// Returns an error when no nightly toolchain is installed, or when the
/// selected toolchain lacks `rust-src` — the error names the exact
/// `rustup component add` command rather than mutating the toolchain
/// silently.
pub async fn nightly_toolchain_with_rust_src(host: &Host) -> eyre::Result<String> {
    let list = host
        .run("rustup", ["toolchain", "list"])
        .await
        .map_err(|error| {
            eyre::eyre!(
                "Android preview needs a nightly Rust toolchain to build `std` from source, \
                 and `rustup toolchain list` failed: {error}"
            )
        })?;
    let host_triple = target_lexicon::Triple::host().to_string();
    let Some(toolchain) = pick_nightly(&list, &host_triple) else {
        eyre::bail!(
            "Android preview needs a nightly Rust toolchain to build `std` from source. \
             Install one with `rustup toolchain install nightly --component rust-src`."
        );
    };

    let components = host
        .run(
            "rustup",
            [
                "component",
                "list",
                "--toolchain",
                &toolchain,
                "--installed",
            ],
        )
        .await
        .map_err(|error| {
            eyre::eyre!("Failed to list components of Rust toolchain `{toolchain}`: {error}")
        })?;
    let has_rust_src = components
        .lines()
        .map(str::trim)
        .any(|line| line == "rust-src" || line.starts_with("rust-src "));
    if !has_rust_src {
        eyre::bail!(
            "Android preview needs the `rust-src` component on `{toolchain}` to build `std` from source. \
             Install it with `rustup component add --toolchain {toolchain} rust-src`."
        );
    }
    Ok(toolchain)
}

/// The `rustc -vV` identity of `toolchain` — what a cached `-Zbuild-std`
/// artifact pins to, because a channel name like `nightly` outlives the
/// compiler it resolves to after `rustup update`.
///
/// # Errors
/// Returns an error when `rustup` cannot run the toolchain's `rustc`.
pub async fn rustc_verbose_version(host: &Host, toolchain: &str) -> eyre::Result<String> {
    host.run("rustup", ["run", toolchain, "rustc", "-vV"])
        .await
        .map_err(|error| {
            eyre::eyre!("Failed to read `rustc -vV` of Rust toolchain `{toolchain}`: {error}")
        })
}

/// Pick the toolchain a `-Zbuild-std` build should use out of `rustup
/// toolchain list` output: `nightly-<host>` first, else the newest dated
/// nightly for the host.
fn pick_nightly(list_output: &str, host_triple: &str) -> Option<String> {
    let default_nightly = format!("nightly-{host_triple}");
    let suffix = format!("-{host_triple}");
    let mut dated = Vec::new();
    for name in list_output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
    {
        if name == default_nightly {
            return Some(name.to_string());
        }
        let Some(date) = name
            .strip_prefix("nightly-")
            .and_then(|rest| rest.strip_suffix(&suffix))
        else {
            continue;
        };
        // Only the dated shape `nightly-YYYY-MM-DD-<host>` is ordered by
        // recency: a custom-linked toolchain named `nightly-anything-<host>`
        // must not sort last and silently outrank every dated nightly.
        let mut fields = date.split('-');
        let is_dated = matches!(
            (fields.next(), fields.next(), fields.next(), fields.next()),
            (Some(year), Some(month), Some(day), None)
                if year.len() == 4 && month.len() == 2 && day.len() == 2
                    && year.bytes().chain(month.bytes()).chain(day.bytes())
                        .all(|byte| byte.is_ascii_digit())
        );
        if is_dated {
            dated.push(name.to_string());
        }
    }
    dated.sort_unstable();
    dated.pop()
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{
        ChannelKind, RustToolchainInstallation, classify_channel, component_is_installed,
        parse_active_toolchain, parse_host_target, parse_rustc_version, pick_nightly,
    };

    #[test]
    fn pick_nightly_prefers_the_plain_channel_then_the_newest_date() {
        let host = "aarch64-apple-darwin";
        let list = "stable-aarch64-apple-darwin (default)\nnightly-aarch64-apple-darwin\nnightly-2026-05-28-aarch64-apple-darwin\n";
        assert_eq!(
            pick_nightly(list, host).as_deref(),
            Some("nightly-aarch64-apple-darwin")
        );

        let dated =
            "nightly-2026-05-28-aarch64-apple-darwin\nnightly-2026-09-09-aarch64-apple-darwin\n";
        assert_eq!(
            pick_nightly(dated, host).as_deref(),
            Some("nightly-2026-09-09-aarch64-apple-darwin")
        );

        assert_eq!(pick_nightly("stable-aarch64-apple-darwin\n", host), None);
    }

    #[test]
    fn pick_nightly_ignores_custom_toolchains_shaped_like_dated_ones() {
        let host = "aarch64-apple-darwin";
        // A linked toolchain named `nightly-zzz-<host>` sorts after every
        // dated nightly — without the date-shape check it would win.
        let list = "nightly-2026-05-28-aarch64-apple-darwin\nnightly-zzz-aarch64-apple-darwin\n";
        assert_eq!(
            pick_nightly(list, host).as_deref(),
            Some("nightly-2026-05-28-aarch64-apple-darwin")
        );
        assert_eq!(
            pick_nightly("nightly-zzz-aarch64-apple-darwin\n", host),
            None
        );
    }

    #[test]
    fn parse_rustc_version_accepts_prerelease() {
        let parsed =
            parse_rustc_version("rustc 1.88.0-nightly (d9a5f4fa4 2026-01-01)").expect("version");
        assert_eq!(
            parsed,
            Version::parse("1.88.0-nightly").expect("expected semver")
        );
    }

    #[test]
    fn parse_active_toolchain_preserves_selected_channel() {
        let toolchain = parse_active_toolchain("nightly-aarch64-apple-darwin (default)")
            .expect("active toolchain");
        assert_eq!(toolchain, "nightly-aarch64-apple-darwin");
    }

    #[test]
    fn parse_host_target_extracts_host_line() {
        let output = "rustc 1.88.0 (aabbcc 2026-01-01)\nbinary: rustc\nhost: x86_64-unknown-linux-gnu\nrelease: 1.88.0\n";
        let host = parse_host_target(output).expect("host target");
        assert_eq!(host, "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn channel_classification() {
        assert_eq!(classify_channel("stable"), ChannelKind::Moving);
        assert_eq!(classify_channel("nightly"), ChannelKind::Moving);
        assert_eq!(classify_channel("nightly-2026-01-15"), ChannelKind::Dated);
        assert_eq!(classify_channel("1.85"), ChannelKind::Version);
        assert_eq!(classify_channel("1.85.0"), ChannelKind::Version);
        assert_eq!(classify_channel("esp"), ChannelKind::Custom);
        assert_eq!(classify_channel("stage0"), ChannelKind::Custom);
    }

    #[test]
    fn component_matching_accepts_target_suffix() {
        let host = "aarch64-apple-darwin";
        assert!(component_is_installed(
            "clippy-aarch64-apple-darwin",
            "clippy",
            host
        ));
        assert!(component_is_installed("rust-src", "rust-src", host));
        assert!(!component_is_installed("rustfmt-preview", "rustfmt", host));
        assert!(!component_is_installed(
            "cargo-aarch64-apple-darwin",
            "clippy",
            host
        ));
        assert!(!component_is_installed(
            "clippy-x86_64-unknown-linux-gnu",
            "clippy",
            host
        ));
    }

    #[test]
    fn installation_summary_lists_actions() {
        let mut installation = RustToolchainInstallation::default();
        installation.require_toolchain_update(String::from("nightly-aarch64-apple-darwin"));
        installation.require_target(
            "stable-aarch64-apple-darwin",
            String::from("x86_64-unknown-linux-gnu"),
        );
        installation.require_component("stable-aarch64-apple-darwin", String::from("clippy"));
        let summary = installation.summary();
        assert!(summary.contains("rustup update nightly-aarch64-apple-darwin"));
        assert!(summary.contains(
            "rustup target add --toolchain stable-aarch64-apple-darwin x86_64-unknown-linux-gnu"
        ));
        assert!(
            summary.contains("rustup component add --toolchain stable-aarch64-apple-darwin clippy")
        );
    }
}

#[cfg(test)]
mod host_tests {
    use super::{
        CLI_MINIMUM_RUST_VERSION, RustToolchain, nightly_toolchain_with_rust_src,
        rustc_verbose_version, tool_binary_name,
    };
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Installation, Toolchain, ToolchainError};

    const FAKE_TARGET: &str = "wasm32test-test-none";

    /// A host with rustup/cargo/rustc fakes reporting a compliant toolchain.
    fn complete_machine() -> TestMachine {
        let machine = TestMachine::new();
        for tool in ["rustup", "cargo", "rustc"] {
            machine.install(tool);
        }
        machine
    }

    /// Declared vars for a complete machine: current rustc, an active
    /// toolchain, and `FAKE_TARGET` both as the rustc host triple and among
    /// the installed rustup targets.
    fn complete_vars() -> Vec<(String, String)> {
        vec![
            (
                String::from("WATERUI_FAKE_RUSTC_VERSION"),
                format!("{CLI_MINIMUM_RUST_VERSION}.0"),
            ),
            (
                String::from("WATERUI_FAKE_RUSTC_HOST"),
                FAKE_TARGET.to_string(),
            ),
            (
                String::from("WATERUI_FAKE_RUSTUP_ACTIVE_TOOLCHAIN"),
                format!("stable-{FAKE_TARGET} (default)"),
            ),
            (
                String::from("WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS"),
                FAKE_TARGET.to_string(),
            ),
        ]
    }

    #[test]
    fn check_unfixable_when_no_rust_tools_exist() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(RustToolchain::default().check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "bare host must report an unfixable Rust toolchain: {result:?}"
        );
    }

    #[test]
    fn check_ok_on_complete_fake_toolchain() {
        let machine = complete_machine();
        let host = machine.host(complete_vars());
        smol::block_on(RustToolchain::default().check(&host))
            .expect("complete fake toolchain must be ok");
    }

    #[test]
    fn check_fixable_when_rustc_too_old() {
        let machine = complete_machine();
        let mut vars = complete_vars();
        vars.retain(|(key, _)| key != "WATERUI_FAKE_RUSTC_VERSION");
        vars.push((
            String::from("WATERUI_FAKE_RUSTC_VERSION"),
            String::from("1.0.0"),
        ));
        let host = machine.host(vars);
        let result = smol::block_on(RustToolchain::default().check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "outdated rustc under rustup must be fixable: {result:?}"
        );
    }

    #[test]
    fn check_unfixable_when_rustc_too_old_without_rustup() {
        let machine = TestMachine::new();
        machine.install("cargo");
        machine.install("rustc");
        let host = machine.host([(
            String::from("WATERUI_FAKE_RUSTC_VERSION"),
            String::from("1.0.0"),
        )]);
        let result = smol::block_on(RustToolchain::default().check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "outdated rustc without rustup cannot be fixed automatically: {result:?}"
        );
    }

    #[test]
    fn check_fixable_when_no_active_toolchain() {
        let machine = complete_machine();
        let mut vars = complete_vars();
        vars.push((
            String::from("WATERUI_FAKE_RUSTUP_NO_ACTIVE_TOOLCHAIN"),
            String::from("1"),
        ));
        let host = machine.host(vars);
        let result = smol::block_on(RustToolchain::default().check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "rustup without an active toolchain must plan a default install: {result:?}"
        );
    }

    #[test]
    fn check_fixable_when_host_target_not_installed() {
        let machine = complete_machine();
        let mut vars = complete_vars();
        vars.retain(|(key, _)| key != "WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS");
        vars.push((
            String::from("WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS"),
            String::from("some-other-target"),
        ));
        let host = machine.host(vars);
        let result = smol::block_on(RustToolchain::default().check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing host target must plan `rustup target add`: {result:?}"
        );
    }

    #[test]
    fn check_fixable_when_pinned_toolchain_missing() {
        let machine = complete_machine();
        machine.file("rust-toolchain.toml", "[toolchain]\nchannel = \"1.90\"\n");
        let mut vars = complete_vars();
        vars.push((
            String::from("WATERUI_FAKE_RUSTUP_TOOLCHAIN_NOT_INSTALLED"),
            String::from("1.90"),
        ));
        let host = machine.host(vars);
        let result = smol::block_on(RustToolchain::default().check(&host));
        match &result {
            Err(ToolchainError::Fixable(installation)) => {
                assert!(
                    installation
                        .summary()
                        .contains("rustup toolchain install 1.90"),
                    "the repair must install the pinned channel: {}",
                    installation.summary()
                );
            }
            other => panic!("missing pinned toolchain must be fixable: {other:?}"),
        }
    }

    #[test]
    fn check_unfixable_when_pinned_custom_toolchain_missing() {
        let machine = complete_machine();
        machine.file("rust-toolchain.toml", "[toolchain]\nchannel = \"esp\"\n");
        let mut vars = complete_vars();
        vars.push((
            String::from("WATERUI_FAKE_RUSTUP_TOOLCHAIN_NOT_INSTALLED"),
            String::from("esp"),
        ));
        let host = machine.host(vars);
        let result = smol::block_on(RustToolchain::default().check(&host));
        match &result {
            Err(ToolchainError::Unfixable(error)) => {
                assert!(
                    error.suggestion().contains("espup install"),
                    "an `esp` pin must name the espup repair: {}",
                    error.suggestion()
                );
            }
            other => panic!("missing custom toolchain must be manual: {other:?}"),
        }
    }

    #[test]
    fn check_unfixable_when_version_pin_below_floor() {
        let machine = complete_machine();
        machine.file("rust-toolchain.toml", "[toolchain]\nchannel = \"1.50\"\n");
        let mut vars = complete_vars();
        vars.retain(|(key, _)| key != "WATERUI_FAKE_RUSTC_VERSION");
        vars.push((
            String::from("WATERUI_FAKE_RUSTC_VERSION"),
            String::from("1.50.0"),
        ));
        let host = machine.host(vars);
        let result = smol::block_on(RustToolchain::default().check(&host));
        match &result {
            Err(ToolchainError::Unfixable(error)) => {
                assert!(
                    error.message().contains("1.50"),
                    "the pin must be named in the diagnostic: {}",
                    error.message()
                );
            }
            other => panic!("a version pin below the floor must be manual: {other:?}"),
        }
    }

    #[test]
    fn nightly_with_rust_src_returns_the_selected_toolchain() {
        let machine = TestMachine::new();
        machine.install("rustup");
        let host_triple = target_lexicon::Triple::host().to_string();
        let nightly = format!("nightly-{host_triple}");
        machine.file(
            "home/.fake-rustup-toolchains",
            &format!("stable-{host_triple}\n{nightly}\n"),
        );
        machine.respond("RUSTUP_INSTALLED_COMPONENTS", "cargo\nrust-src\n");
        let host = machine.host(Vec::<(String, String)>::new());
        let toolchain = smol::block_on(nightly_toolchain_with_rust_src(&host))
            .expect("a nightly carrying rust-src must be selected");
        assert_eq!(toolchain, nightly);
    }

    #[test]
    fn nightly_without_rust_src_fails_with_the_exact_component_command() {
        let machine = TestMachine::new();
        machine.install("rustup");
        let host_triple = target_lexicon::Triple::host().to_string();
        let nightly = format!("nightly-{host_triple}");
        machine.file(
            "home/.fake-rustup-toolchains",
            &format!("stable-{host_triple}\n{nightly}\n"),
        );
        // `component list` prints nothing — `rust-src` is absent, and the
        // check must refuse rather than mutate the toolchain silently.
        let host = machine.host(Vec::<(String, String)>::new());
        let error = smol::block_on(nightly_toolchain_with_rust_src(&host))
            .expect_err("missing rust-src must fail");
        assert!(
            error.to_string().contains(&format!(
                "rustup component add --toolchain {nightly} rust-src"
            )),
            "the error must name the exact install command: {error}"
        );
    }

    #[test]
    fn rustc_verbose_version_proxies_through_rustup_run() {
        let machine = TestMachine::new();
        machine.install("rustup");
        machine.install("rustc");
        let host = machine.host([(
            String::from("WATERUI_FAKE_RUSTC_HOST"),
            String::from("aarch64-apple-darwin"),
        )]);
        let version = smol::block_on(rustc_verbose_version(&host, "nightly-fake"))
            .expect("`rustup run` must dispatch to the sibling rustc");
        assert!(
            version.contains("host: aarch64-apple-darwin"),
            "the toolchain's `rustc -vV` identity must come back: {version}"
        );
    }

    #[test]
    fn check_fixable_when_pinned_component_missing() {
        let machine = complete_machine();
        machine.file(
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"stable\"\ncomponents = [\"clippy\", \"rustfmt\"]\n",
        );
        let host = machine.host(complete_vars());
        let result = smol::block_on(RustToolchain::default().check(&host));
        match &result {
            Err(ToolchainError::Fixable(installation)) => {
                let summary = installation.summary();
                assert!(
                    summary.contains("rustup component add") && summary.contains("clippy"),
                    "missing pin components must plan component add: {summary}"
                );
            }
            other => panic!("missing pin components must be fixable: {other:?}"),
        }
    }

    #[test]
    fn check_ok_when_pinned_components_installed() {
        let machine = complete_machine();
        machine.file(
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"stable\"\ncomponents = [\"clippy\"]\ntargets = [\"wasm32test-test-none\"]\n",
        );
        let mut vars = complete_vars();
        vars.push((
            String::from("WATERUI_FAKE_RUSTUP_INSTALLED_COMPONENTS"),
            String::from("clippy-wasm32test-test-none"),
        ));
        let host = machine.host(vars);
        smol::block_on(RustToolchain::default().check(&host))
            .expect("pin-declared targets and components installed must be ok");
    }

    #[test]
    fn check_unfixable_when_proxies_missing_from_path() {
        let machine = TestMachine::new();
        machine.install("rustup");
        // rustup is on PATH, cargo/rustc are not — but their proxies exist
        // under the fake home's `.cargo/bin`.
        machine.file(
            format!("home/.cargo/bin/{}", tool_binary_name("cargo")),
            "proxy",
        );
        machine.file(
            format!("home/.cargo/bin/{}", tool_binary_name("rustc")),
            "proxy",
        );
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(RustToolchain::default().check(&host));
        match &result {
            Err(ToolchainError::Unfixable(error)) => {
                assert!(
                    error.message().contains("not on PATH"),
                    "the PATH gap must be diagnosed: {}",
                    error.message()
                );
            }
            other => panic!("proxies off PATH must be a manual diagnosis: {other:?}"),
        }
    }

    #[test]
    fn check_unfixable_when_proxies_never_installed() {
        let machine = TestMachine::new();
        machine.install("rustup");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(RustToolchain::default().check(&host));
        match &result {
            Err(ToolchainError::Unfixable(error)) => {
                assert!(
                    error.suggestion().contains("rustup"),
                    "missing proxies must name the reinstall: {}",
                    error.suggestion()
                );
            }
            other => panic!("absent proxies must be a manual diagnosis: {other:?}"),
        }
    }

    #[test]
    fn install_repairs_missing_pinned_toolchain_and_recheck_passes() {
        let machine = complete_machine();
        machine.file("rust-toolchain.toml", "[toolchain]\nchannel = \"1.90\"\n");
        let mut vars = complete_vars();
        vars.retain(|(key, _)| key != "WATERUI_FAKE_RUSTUP_ACTIVE_TOOLCHAIN");
        vars.push((
            String::from("WATERUI_FAKE_RUSTUP_TOOLCHAIN_NOT_INSTALLED"),
            String::from("1.90"),
        ));
        let host = machine.host(vars);

        let Err(ToolchainError::Fixable(installation)) =
            smol::block_on(RustToolchain::default().check(&host))
        else {
            panic!("missing pinned toolchain must be fixable");
        };
        smol::block_on(installation.install(&host)).expect("fake rustup install must succeed");

        smol::block_on(RustToolchain::default().check(&host))
            .expect("after `rustup toolchain install`, the check must pass");
    }
}
