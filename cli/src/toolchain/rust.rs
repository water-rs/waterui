//! Rust toolchain checks and remediation.

use semver::Version;

use crate::{
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::{CommandError, parse_semver_version},
};

const REQUIRED_RUST_VERSION: &str = env!("CARGO_PKG_RUST_VERSION");

/// Rust toolchain checker.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustToolchain;

/// Installation plan for Rust toolchain fixes.
#[derive(Debug, Clone, Default)]
pub struct RustToolchainInstallation {
    install_stable_toolchain: bool,
    update_toolchain: Option<String>,
    add_host_target: Option<String>,
}

impl RustToolchainInstallation {
    fn require_stable_install(&mut self) {
        self.install_stable_toolchain = true;
        self.update_toolchain = None;
    }

    fn require_toolchain_update(&mut self, toolchain: String) {
        if !self.install_stable_toolchain {
            self.update_toolchain = Some(toolchain);
        }
    }

    fn require_host_target(&mut self, target: String) {
        self.add_host_target = Some(target);
    }

    /// Returns `true` when at least one automatic fix action is planned.
    #[must_use]
    pub const fn has_actions(&self) -> bool {
        self.install_stable_toolchain
            || self.update_toolchain.is_some()
            || self.add_host_target.is_some()
    }

    /// Human-readable summary of automatic fixes this installation will run.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut actions = Vec::new();
        if self.install_stable_toolchain {
            actions.push(String::from("install `rustup toolchain install stable`"));
        }
        if let Some(toolchain) = &self.update_toolchain {
            actions.push(format!("update `{toolchain}` via rustup"));
        }
        if let Some(target) = &self.add_host_target {
            actions.push(format!("add host target via `rustup target add {target}`"));
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
    /// Failed to install stable toolchain.
    #[error("Failed to install Rust stable toolchain: {0}")]
    InstallStableToolchain(#[source] CommandError),
    /// Failed to update the active toolchain.
    #[error("Failed to update active Rust toolchain `{toolchain}`: {source}")]
    UpdateToolchain {
        /// Active rustup toolchain that failed to update.
        toolchain: String,
        /// Underlying command error.
        source: CommandError,
    },
    /// Failed to add host target.
    #[error("Failed to add Rust host target `{target}`: {source}")]
    AddHostTarget {
        /// Target triple that failed to install.
        target: String,
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

        if self.install_stable_toolchain {
            host.run("rustup", ["toolchain", "install", "stable"])
                .await
                .map_err(FailToInstallRustToolchain::InstallStableToolchain)?;
        }

        if let Some(toolchain) = &self.update_toolchain {
            host.run("rustup", ["update", toolchain.as_str()])
                .await
                .map_err(|source| FailToInstallRustToolchain::UpdateToolchain {
                    toolchain: toolchain.clone(),
                    source,
                })?;
        }

        if let Some(target) = &self.add_host_target {
            host.run("rustup", ["target", "add", target.as_str()])
                .await
                .map_err(|source| FailToInstallRustToolchain::AddHostTarget {
                    target: target.clone(),
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
        let mut installation = RustToolchainInstallation::default();
        let active_toolchain =
            check_active_toolchain(host, availability.rustup_available, &mut installation).await?;
        ensure_cargo_available(availability, &mut installation)?;
        let host_target = check_rustc_version_and_host_target(
            host,
            availability,
            active_toolchain.as_deref(),
            &mut installation,
        )
        .await?;
        check_installed_targets(
            host,
            availability.rustup_available,
            &installation,
            host_target,
        )
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

fn ensure_minimum_rust_tools(
    availability: RustToolAvailability,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    if !availability.rustup_available
        && (!availability.cargo_available || !availability.rustc_available)
    {
        return Err(ToolchainError::unfixable(
            "Rust toolchain is incomplete (`cargo` and/or `rustc` is missing from PATH).",
            "Install rustup from https://rustup.rs, then run `rustup toolchain install stable`.",
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

async fn check_active_toolchain(
    host: &Host,
    rustup_available: bool,
    installation: &mut RustToolchainInstallation,
) -> Result<Option<String>, ToolchainError<RustToolchainInstallation>> {
    if !rustup_available {
        return Ok(None);
    }

    match host.run("rustup", ["show", "active-toolchain"]).await {
        Ok(output) => parse_active_toolchain(&output).map(Some).map_err(|error| {
            ToolchainError::unfixable(
                format!("Could not parse the active rustup toolchain: {error}"),
                "Run `rustup show active-toolchain`; repair or reinstall rustup if it does not return a toolchain name.",
            )
        }),
        Err(error) => {
            let error_message = error.to_string();
            if is_no_active_toolchain_error(&error_message) {
                installation.require_stable_install();
                Ok(None)
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

fn ensure_cargo_available(
    availability: RustToolAvailability,
    installation: &mut RustToolchainInstallation,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    if availability.cargo_available {
        return Ok(());
    }
    if availability.rustup_available {
        installation.require_stable_install();
        Ok(())
    } else {
        Err(ToolchainError::unfixable(
            "`cargo` is not available on PATH.",
            "Install rustup from https://rustup.rs, then run `rustup toolchain install stable`.",
        ))
    }
}

async fn check_rustc_version_and_host_target(
    host: &Host,
    availability: RustToolAvailability,
    active_toolchain: Option<&str>,
    installation: &mut RustToolchainInstallation,
) -> Result<Option<String>, ToolchainError<RustToolchainInstallation>> {
    if !availability.rustc_available {
        return handle_missing_rustc(availability.rustup_available, installation);
    }

    let version_output = host.run("rustc", ["--version"]).await.map_err(|error| {
        let error_message = error.to_string();
        rustc_run_error(availability.rustup_available, &error_message)
    })?;
    let installed_version = parse_installed_rustc_version(&version_output)?;
    let required_version = parse_required_rustc_version()?;
    maybe_require_rust_update(
        availability.rustup_available,
        active_toolchain,
        &installed_version,
        &required_version,
        installation,
    )?;

    if !availability.rustup_available || installation.install_stable_toolchain {
        return Ok(None);
    }

    let rustc_verbose = host.run("rustc", ["-vV"]).await.map_err(|error| {
        ToolchainError::unfixable(
            format!("`rustc -vV` failed: {error}"),
            "Run `rustc -vV` manually; if it fails, reinstall rustup from https://rustup.rs.",
        )
    })?;
    parse_host_target_value(&rustc_verbose).map(Some)
}

fn handle_missing_rustc(
    rustup_available: bool,
    installation: &mut RustToolchainInstallation,
) -> Result<Option<String>, ToolchainError<RustToolchainInstallation>> {
    if rustup_available {
        installation.require_stable_install();
        Ok(None)
    } else {
        Err(ToolchainError::unfixable(
            "`rustc` is not available on PATH.",
            "Install rustup from https://rustup.rs, then run `rustup toolchain install stable`.",
        ))
    }
}

fn rustc_run_error(
    rustup_available: bool,
    error_message: &str,
) -> ToolchainError<RustToolchainInstallation> {
    if rustup_available {
        let mut installation = RustToolchainInstallation::default();
        installation.require_stable_install();
        ToolchainError::fixable(installation)
    } else {
        ToolchainError::unfixable(
            format!("`rustc` exists on PATH but failed to run: {error_message}"),
            "Reinstall Rust toolchain via rustup from https://rustup.rs.",
        )
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

fn parse_required_rustc_version() -> Result<Version, ToolchainError<RustToolchainInstallation>> {
    required_rust_version().map_err(|error| {
        ToolchainError::unfixable(
            format!("Invalid required Rust version `{REQUIRED_RUST_VERSION}`: {error}"),
            "Reinstall waterui-cli from source to restore a valid embedded Rust requirement.",
        )
    })
}

fn maybe_require_rust_update(
    rustup_available: bool,
    active_toolchain: Option<&str>,
    installed_version: &Version,
    required_version: &Version,
    installation: &mut RustToolchainInstallation,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    if installed_version >= required_version {
        return Ok(());
    }

    if rustup_available {
        match active_toolchain {
            Some(toolchain) => installation.require_toolchain_update(toolchain.to_owned()),
            None => installation.require_stable_install(),
        }
        Ok(())
    } else {
        Err(ToolchainError::unfixable(
            format!(
                "Detected Rust {installed_version}, but waterui-cli requires at least Rust {required_version}."
            ),
            format!(
                "Install Rust {required_version} or newer. Recommended: install rustup from https://rustup.rs, then run `rustup update stable`."
            ),
        ))
    }
}

fn parse_host_target_value(
    rustc_verbose: &str,
) -> Result<String, ToolchainError<RustToolchainInstallation>> {
    parse_host_target(rustc_verbose).map_err(|error| {
        ToolchainError::unfixable(
            format!("Could not parse host target from `rustc -vV`: {error}"),
            "Ensure `rustc -vV` includes a `host: <target>` line; reinstall rustup if the output is incomplete.",
        )
    })
}

async fn check_installed_targets(
    host: &Host,
    rustup_available: bool,
    installation: &RustToolchainInstallation,
    host_target: Option<String>,
) -> Result<(), ToolchainError<RustToolchainInstallation>> {
    if !rustup_available || installation.install_stable_toolchain {
        return Ok(());
    }

    let Some(host_target) = host_target else {
        return Ok(());
    };

    let installed_targets = installed_rustup_targets(host).await.map_err(|error| {
        ToolchainError::unfixable(
            format!("Failed to list installed Rust targets: {error}"),
            "Run `rustup target list --installed`; if it fails, repair rustup with `rustup self update` or reinstall rustup.",
        )
    })?;

    if installed_targets
        .iter()
        .any(|target| target == &host_target)
    {
        return Ok(());
    }

    let mut installation = installation.clone();
    installation.require_host_target(host_target);
    Err(ToolchainError::fixable(installation))
}

async fn installed_rustup_targets(host: &Host) -> Result<Vec<String>, CommandError> {
    let installed = host
        .run("rustup", ["target", "list", "--installed"])
        .await?;
    Ok(installed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn required_rust_version() -> Result<Version, RustParseError> {
    Ok(parse_semver_version(REQUIRED_RUST_VERSION)?)
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
    normalized.contains("no active toolchain")
        || normalized.contains("default toolchain")
        || normalized.contains("not installed")
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{
        RustToolchainInstallation, parse_active_toolchain, parse_host_target, parse_rustc_version,
    };

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
    fn installation_summary_lists_actions() {
        let mut installation = RustToolchainInstallation::default();
        installation.require_toolchain_update(String::from("nightly-aarch64-apple-darwin"));
        installation.require_host_target(String::from("x86_64-unknown-linux-gnu"));
        let summary = installation.summary();
        assert!(summary.contains("update `nightly-aarch64-apple-darwin` via rustup"));
        assert!(summary.contains("rustup target add x86_64-unknown-linux-gnu"));
    }
}

#[cfg(test)]
mod host_tests {
    use super::{REQUIRED_RUST_VERSION, RustToolchain};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

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
                format!("{REQUIRED_RUST_VERSION}.0"),
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
        let result = smol::block_on(RustToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "bare host must report an unfixable Rust toolchain: {result:?}"
        );
    }

    #[test]
    fn check_ok_on_complete_fake_toolchain() {
        let machine = complete_machine();
        let host = machine.host(complete_vars());
        smol::block_on(RustToolchain.check(&host)).expect("complete fake toolchain must be ok");
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
        let result = smol::block_on(RustToolchain.check(&host));
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
        let result = smol::block_on(RustToolchain.check(&host));
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
        let result = smol::block_on(RustToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "rustup without an active toolchain must plan a stable install: {result:?}"
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
        let result = smol::block_on(RustToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing host target must plan `rustup target add`: {result:?}"
        );
    }
}
