//! Toolchain support for `sccache` - shared compilation cache.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use smol::process::Command;

use crate::{
    brew::Brew,
    toolchain::linux::{
        LinuxPackageManagerError, has_supported_package_manager, install_named_packages,
    },
    toolchain::managed_tool::{self, ManagedTool, ManagedToolError},
    toolchain::winget::{WingetInstallError, ensure_package_installed},
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::{CommandError, sccache_install_hint, sccache_upgrade_hint},
};

/// Route a Cargo invocation's compiles through `sccache`.
///
/// Caching only bites because generated-crate builds also disable incremental
/// compilation — Cargo does not pass `-C incremental` to registry dependencies but does
/// pass it to every *path* dependency, which for a `WaterUI` build is the entire
/// framework, and `sccache` refuses to cache an incremental compile. That setting lives
/// in [`crate::build::configure_generated_crate_compilation`] rather than here, because
/// it must not depend on whether a machine happens to have `sccache` installed: it
/// changes the compiled ABI, and two builds in one flow have to agree on it.
///
/// The server address is namespaced to the invoking user. sccache discovers
/// its server on a host-wide address — TCP `127.0.0.1:4226` unless told
/// otherwise — and every compile job runs inside the server process under
/// the *server owner's* identity. Left at the default, a build running as one
/// user borrows a server another user left alive and its artifacts land in
/// this user's target dir owned by the other uid, ending the build on
/// `Permission denied`. A unix socket under the user's own Water home gives
/// each account its own server with no port to collide over, and sccache
/// ≥ 0.9.0 prefers it when both are set; the port is still set unconditionally
/// because older builds ignore the socket variable entirely and would fall
/// back to the shared default address.
/// Configuring the cache also brings the server up, because Cargo cannot.
/// Cargo starts its rustc jobs and build scripts in parallel and every one of
/// them is an sccache client that starts the server itself when none is
/// listening; racing to create the socket, the losers die with
/// `Server startup failed: File exists (os error 17)` and rustc reports a
/// failed build. One client of our own beforehand is the serialization point:
/// once it returns, the socket is bound and every client Cargo spawns
/// connects to it instead of trying to create it.
///
/// # Errors
/// Returns an error when the socket directory under the user's Water home
/// cannot be created or exists with permissions wider than `0700`, or when
/// the server does not come up — every compile would fail on the same thing,
/// so it fails here where the reason is still legible.
pub async fn configure_compilation_cache(
    command: &mut Command,
    sccache_path: &Path,
) -> eyre::Result<()> {
    let water_home = crate::project_model::water_dir::water_home_dir().ok();
    #[cfg(unix)]
    let env = compilation_cache_env_in(sccache_path, water_home.as_deref())?;
    #[cfg(not(unix))]
    let env = compilation_cache_env_in(sccache_path, water_home.as_deref());
    for (key, value) in &env {
        command.env(key, value);
    }
    start_server(sccache_path, &env).await
}

/// Connect to the per-user server, starting it when it is not listening.
///
/// `--show-stats` is sccache's connect-or-start path, and it runs under the
/// same address environment the build is about to use — a server reached on
/// any other address is not the one Cargo would find.
///
/// # Errors
/// Returns an error when the client cannot be spawned, or when it reports
/// that the server is not available.
async fn start_server(sccache_path: &Path, env: &[(&'static str, OsString)]) -> eyre::Result<()> {
    use eyre::WrapErr as _;

    let mut client = Command::new(sccache_path);
    client
        .arg("--show-stats")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    for (key, value) in env {
        client.env(key, value);
    }
    let output = client
        .output()
        .await
        .wrap_err_with(|| format!("Failed to run {}", sccache_path.display()))?;
    eyre::ensure!(
        output.status.success(),
        "the sccache server did not come up, so every compile of this build would \
         fail the same way: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// The environment a compile command needs for per-user sccache routing, as
/// `(key, value)` pairs so the whole contract is observable without spawning
/// a process. The Water home is a parameter so tests can inject a scratch
/// directory instead of touching the real `~/.water` or depending on the
/// machine's home-path length. Only unix is fallible: it is the one host
/// that adds a socket under that home.
#[cfg(unix)]
fn compilation_cache_env_in(
    sccache_path: &Path,
    water_home: Option<&Path>,
) -> eyre::Result<Vec<(&'static str, OsString)>> {
    let mut env = base_compilation_cache_env(sccache_path);
    if let Some(socket) = water_home.map(server_socket_path_in).transpose()?.flatten() {
        env.push(("SCCACHE_SERVER_UDS", socket.into_os_string()));
    }
    Ok(env)
}

/// `compilation_cache_env_in` for hosts with no per-user socket: the
/// contract is the fixed pair list, so nothing here can fail.
#[cfg(not(unix))]
fn compilation_cache_env_in(
    sccache_path: &Path,
    _water_home: Option<&Path>,
) -> Vec<(&'static str, OsString)> {
    base_compilation_cache_env(sccache_path)
}

/// The pairs every host sets: `RUSTC_WRAPPER` routes each compile through
/// sccache and `SCCACHE_SERVER_PORT` namespaces its server to the user.
fn base_compilation_cache_env(sccache_path: &Path) -> Vec<(&'static str, OsString)> {
    vec![
        ("RUSTC_WRAPPER", sccache_path.as_os_str().to_os_string()),
        (
            "SCCACHE_SERVER_PORT",
            per_user_server_port().to_string().into(),
        ),
    ]
}

/// `sun_path` is 108 bytes on Linux and 104 on macOS/BSD, including the
/// terminator — 103 keeps a socket path bindable on every unix host.
#[cfg(unix)]
const MAX_SUN_PATH_BYTES: usize = 103;

/// The unix socket a per-user sccache server listens on, under a dedicated
/// `0700` directory in the invoking user's Water home so no other account can
/// reach — or be reached by — it. `Ok(None)` when the path would not fit
/// `sun_path`: a socket that cannot bind is no fallback at all, so only the
/// per-user port is offered then.
///
/// # Errors
/// Returns an error when the socket directory cannot be created, or exists
/// with permissions wider than `0700` — sccache's server runs compile jobs
/// under its owner's identity with no authentication, so a socket another
/// account could traverse to is not an isolation mechanism and the build must
/// not silently fall back to the shared-address exposure.
#[cfg(unix)]
fn server_socket_path_in(water_home: &Path) -> eyre::Result<Option<PathBuf>> {
    let socket_dir = water_home.join("sccache");
    ensure_private_socket_dir(&socket_dir)?;
    let socket = socket_dir.join("server.sock");
    Ok((socket.as_os_str().len() <= MAX_SUN_PATH_BYTES).then_some(socket))
}

/// Create `dir` mode `0700`, or verify an existing one is that private. A
/// wider directory fails loudly: the socket inside is how one account would
/// submit compile jobs to another user's server, so narrowing the check to a
/// warning would leave the door it exists to close.
#[cfg(unix)]
fn ensure_private_socket_dir(dir: &Path) -> eyre::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    use eyre::WrapErr as _;

    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(dir)
        .wrap_err_with(|| format!("Failed to create sccache socket dir {}", dir.display()))?;
    let mode = std::fs::metadata(dir)
        .wrap_err_with(|| format!("Failed to stat sccache socket dir {}", dir.display()))?
        .mode()
        & 0o777;
    eyre::ensure!(
        mode.trailing_zeros() >= 6,
        "sccache socket dir {} has mode {mode:o}, wider than 0700 — other local \
         accounts could submit compile jobs to this user's sccache server. \
         Tighten it with `chmod 700 {}`.",
        dir.display(),
        dir.display()
    );
    Ok(())
}

/// A deterministic per-user TCP port for the sccache server, in the
/// 22000–31150 block below every supported host's ephemeral floor (Linux
/// 32768, Windows and macOS 49152) so a transient connection never occupies
/// it. A collision with an unrelated registered service is still possible;
/// that fails the server bind loudly instead of quietly joining another
/// user's server.
fn per_user_server_port() -> u16 {
    port_for_identity(&user_identity())
}

/// Spread a machine-unique user identity over the port block. FNV-1a needs
/// no state and no coordination between accounts.
fn port_for_identity(identity: &str) -> u16 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in identity.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
    }
    22_000 + (hash % 9_151) as u16
}

/// The machine-unique identity of the invoking user. Hashing the user *name*
/// instead would let two accounts share a port — `ayy` and `cad` both
/// produced 46119 — and a name that cannot be read would pin every such
/// machine to one port. The uid is always present and distinct per account;
/// the FNV-1a reduction into 9151 slots can still map two uids to one port —
/// rare, and it re-shares a server rather than failing, so it is worth
/// keeping the identity as distinct as the OS makes possible.
#[cfg(unix)]
fn user_identity() -> String {
    nix::unistd::getuid().to_string()
}

/// The machine-unique identity of the invoking user: the account's SID string
/// (`S-1-5-21-…`), which is unique per machine and always present for a
/// running process.
#[cfg(windows)]
fn user_identity() -> String {
    use std::io;

    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    // SAFETY: every call queries the current process's own token; the token
    // buffer is sized by the API before the second `GetTokenInformation`
    // writes it, the handle is closed on every path past `OpenProcessToken`,
    // and the string the SID conversion allocates is freed with `LocalFree`.
    unsafe {
        let mut token = std::mem::zeroed();
        assert!(
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) != 0,
            "OpenProcessToken failed: {}",
            io::Error::last_os_error()
        );
        let mut size = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &raw mut size);
        // The buffer is read back as a TOKEN_USER, so it needs that struct's
        // alignment — u64 elements guarantee it on every Windows target.
        let mut buffer = vec![0u64; (size as usize).div_ceil(std::mem::size_of::<u64>())];
        let queried = size > 0
            && GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &raw mut size,
            ) != 0;
        CloseHandle(token);
        assert!(
            queried,
            "GetTokenInformation(TokenUser) failed: {}",
            io::Error::last_os_error()
        );
        let sid = (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let mut text = std::ptr::null_mut::<u16>();
        assert!(
            ConvertSidToStringSidW(sid, &raw mut text) != 0,
            "ConvertSidToStringSidW failed: {}",
            io::Error::last_os_error()
        );
        let mut length = 0usize;
        while *text.add(length) != 0 {
            length += 1;
        }
        let identity = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        identity
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!(
    "per-user sccache ports need a user-identity source; supported hosts are unix and Windows"
);

/// Toolchain for `sccache` - a shared compilation cache for Rust.
///
/// sccache is optional but significantly improves build times by caching
/// compiled artifacts across builds and projects.
#[derive(Debug, Clone, Default)]
pub struct Sccache;

impl Sccache {
    /// Get the path to the `sccache` executable if available.
    ///
    /// The managed install under `~/.water/tools` first — a pinned copy can
    /// never be the stale distribution package — then `PATH`.
    ///
    /// # Errors
    /// Returns an error if `sccache` is not found in the managed tools or the
    /// system PATH.
    pub async fn path(&self, host: &Host) -> Result<PathBuf, which::Error> {
        if let Some(path) = managed_tool::sccache().and_then(|tool| tool.binary_path(host)) {
            return Ok(path);
        }
        host.which("sccache").await
    }

    /// Check if sccache is available on `host` without returning an error.
    pub async fn is_available(&self, host: &Host) -> bool {
        self.path(host).await.is_ok()
    }
}

/// The sccache release that understands `SCCACHE_SERVER_UDS` — the mechanism
/// `configure_compilation_cache` uses to keep each user's compile server
/// private on unix hosts.
const MINIMUM_SCCACHE_VERSION: &str = "0.9.0";

/// `sccache` is on PATH; it still has to be new enough to honor the per-user
/// server address the compile path hands it, which only 0.9.0 does. An older
/// build gets the port fallback and keeps working, but a check that cannot
/// name the installed version — or finds one below the floor — reports it
/// instead of letting a quietly-shared host-wide server resurface.
///
/// A managed install (`~/.water/tools`) is a pinned release — its version is
/// known by construction, so only `PATH` copies need this probe.
async fn check_sccache_version(
    host: &Host,
    sccache_path: PathBuf,
) -> Result<(), ToolchainError<SccacheInstallation>> {
    let Ok(output) = host.output(&sccache_path, ["--version"]).await else {
        return Err(ToolchainError::unfixable(
            "sccache is installed but `sccache --version` could not run",
            format!(
                "Reinstall sccache ({}) so it executes correctly, then re-run `water doctor`.",
                sccache_install_hint()
            ),
        ));
    };
    if !output.status.success() {
        return Err(ToolchainError::unfixable(
            "`sccache --version` exited with a failure",
            format!(
                "Reinstall sccache ({}) so `sccache --version` succeeds, then re-run `water doctor`.",
                sccache_install_hint()
            ),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let installed = text
        .split_whitespace()
        .nth(1)
        .and_then(|token| semver::Version::parse(token).ok());
    let Some(installed) = installed else {
        return Err(ToolchainError::unfixable(
            format!(
                "`sccache --version` printed an unreadable version: {}",
                text.trim()
            ),
            format!(
                "Install a released sccache build ({}), then re-run `water doctor`.",
                sccache_install_hint()
            ),
        ));
    };
    let minimum =
        semver::Version::parse(MINIMUM_SCCACHE_VERSION).expect("the version floor is valid semver");
    if installed.cmp_precedence(&minimum).is_lt() {
        return Err(ToolchainError::unfixable(
            format!(
                "sccache {installed} is too old: per-user build-cache isolation needs sccache {MINIMUM_SCCACHE_VERSION} or newer"
            ),
            format!(
                "Upgrade sccache — {} — then re-run `water doctor`.",
                sccache_upgrade_hint()
            ),
        ));
    }
    Ok(())
}

/// What a missing `sccache` on Windows resolves to: `winget` when present,
/// otherwise a pinned release archive unpacked under `~/.water/tools` — no
/// package manager required.
async fn missing_sccache_on_windows(host: &Host) -> ToolchainError<SccacheInstallation> {
    if host.which("winget").await.is_ok() {
        ToolchainError::fixable(SccacheInstallation::Winget)
    } else if let Some(tool) = managed_tool::sccache() {
        ToolchainError::fixable(SccacheInstallation::Managed(tool))
    } else {
        ToolchainError::unfixable(
            "sccache is missing and this host has no usable installer",
            format!(
                "Install sccache manually with {} and ensure `sccache` is available in PATH.",
                sccache_install_hint()
            ),
        )
    }
}

impl Toolchain for Sccache {
    type Installation = SccacheInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        let managed = managed_tool::sccache();
        if managed
            .as_ref()
            .and_then(|tool| tool.binary_path(host))
            .is_some()
        {
            // A managed copy is a pinned, checksum-verified release — its
            // version is known by construction, and `path` resolves to it
            // ahead of any stale PATH copy.
            Ok(())
        } else if let Ok(sccache_path) = host.which("sccache").await {
            match check_sccache_version(host, sccache_path).await {
                Ok(()) => Ok(()),
                // A package-manager install cannot express the upgrade a
                // stale or broken PATH copy needs; the pinned release
                // artifact lands a known-good build alongside it.
                Err(error) => managed.map_or(Err(error), |tool| {
                    Err(ToolchainError::fixable(SccacheInstallation::Managed(tool)))
                }),
            }
        } else if cfg!(target_os = "windows") {
            Err(missing_sccache_on_windows(host).await)
        } else if cfg!(target_os = "macos") {
            if host.which("brew").await.is_ok() {
                Err(ToolchainError::fixable(SccacheInstallation::Brew))
            } else if let Some(tool) = managed {
                Err(ToolchainError::fixable(SccacheInstallation::Managed(tool)))
            } else {
                Err(ToolchainError::unfixable(
                    "sccache not found and Homebrew is unavailable",
                    format!(
                        "Install Homebrew to enable automatic fixes, or install manually with {}.",
                        sccache_install_hint()
                    ),
                ))
            }
        } else if cfg!(target_os = "linux") {
            // Distribution packages lag the 0.9.0 floor (apt carries 0.7.x,
            // Fedora none at all), so the pinned release artifact is the
            // repair; the package manager only remains for architectures
            // upstream does not ship.
            if let Some(tool) = managed {
                Err(ToolchainError::fixable(SccacheInstallation::Managed(tool)))
            } else if has_supported_package_manager(host).await {
                Err(ToolchainError::fixable(SccacheInstallation::PackageManager))
            } else {
                Err(ToolchainError::unfixable(
                    "sccache is missing and no supported package manager was found",
                    format!("Install manually with {}", sccache_install_hint()),
                ))
            }
        } else if let Some(tool) = managed {
            Err(ToolchainError::fixable(SccacheInstallation::Managed(tool)))
        } else {
            Err(ToolchainError::unfixable(
                "sccache not found",
                format!(
                    "Install sccache manually ({}) and ensure `sccache` is available in PATH.",
                    sccache_install_hint()
                ),
            ))
        }
    }
}

/// Installation plan for `sccache` — the strategy `check` selected for this
/// host.
#[derive(Debug, Clone)]
pub enum SccacheInstallation {
    /// `brew install sccache`.
    Brew,
    /// `winget install Mozilla.sccache`.
    Winget,
    /// The host's Linux package manager.
    PackageManager,
    /// A pinned, checksum-verified release archive unpacked under
    /// `~/.water/tools` — no package manager required.
    Managed(ManagedTool),
}

/// Errors that can occur during `sccache` installation.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallSccache {
    /// Homebrew not found error.
    #[error("Homebrew not found. Please install Homebrew to proceed.")]
    BrewNotFound,

    /// An installation command failed.
    #[error("Failed to install sccache: {0}")]
    Command(#[from] CommandError),

    /// winget is required for Windows automatic installation.
    #[error(
        "winget is required for automatic sccache installation on Windows. Install App Installer and retry."
    )]
    WingetNotFound,

    /// Windows installation via winget failed.
    #[error("Failed to install sccache via winget: {0}")]
    WingetInstallFailed(String),

    /// Linux package manager is required for automatic installation.
    #[error(
        "No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk). Install sccache manually."
    )]
    UnsupportedPackageManager,

    /// The managed archive install failed.
    #[error(transparent)]
    Managed(#[from] ManagedToolError),
}

impl Installation for SccacheInstallation {
    type Error = FailToInstallSccache;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        match self {
            Self::Brew => {
                let brew = Brew::default();
                brew.check(host)
                    .await
                    .map_err(|_| FailToInstallSccache::BrewNotFound)?;
                brew.install(host, "sccache").await?;
                Ok(())
            }
            Self::Winget => ensure_package_installed(host, "Mozilla.sccache")
                .await
                .map_err(map_winget_error_for_sccache),
            Self::PackageManager => install_named_packages(host, &["sccache"])
                .await
                .map_err(map_linux_error_for_sccache),
            Self::Managed(tool) => {
                tool.install(host).await?;
                Ok(())
            }
        }
    }
}

fn map_linux_error_for_sccache(error: LinuxPackageManagerError) -> FailToInstallSccache {
    match error {
        LinuxPackageManagerError::UnsupportedPackageManager => {
            FailToInstallSccache::UnsupportedPackageManager
        }
        LinuxPackageManagerError::Command(source) => FailToInstallSccache::Command(source),
    }
}

fn map_winget_error_for_sccache(error: WingetInstallError) -> FailToInstallSccache {
    match error {
        WingetInstallError::WingetNotFound => FailToInstallSccache::WingetNotFound,
        WingetInstallError::CommandFailed(err) => {
            FailToInstallSccache::WingetInstallFailed(err.to_string())
        }
        WingetInstallError::NotInstalled { package_id } => {
            FailToInstallSccache::WingetInstallFailed(format!(
                "Package `{package_id}` is still missing after winget install; verify winget sources and retry."
            ))
        }
    }
}

#[cfg(test)]
mod host_tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::{
        Sccache, SccacheInstallation, compilation_cache_env_in, per_user_server_port,
        port_for_identity,
    };
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    fn check(machine: &TestMachine) -> Result<(), ToolchainError<SccacheInstallation>> {
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Sccache.check(&host))
    }

    #[test]
    fn ok_when_sccache_on_path() {
        let machine = TestMachine::new();
        machine.install("sccache");
        check(&machine).expect("sccache on PATH must be ok");
    }

    #[test]
    fn sccache_below_the_uds_floor_is_rejected() {
        let machine = TestMachine::new();
        machine.install("sccache");
        let host = machine.host([("WATERUI_FAKE_SCCACHE_VERSION", "0.8.2")]);
        let result = smol::block_on(Sccache.check(&host));
        // The pinned artifact repairs a stale PATH copy the package manager
        // cannot upgrade, and every host this CLI supports has one — see
        // `every_supported_host_has_a_pinned_artifact`.
        assert!(
            matches!(
                result,
                Err(ToolchainError::Fixable(SccacheInstallation::Managed(_)))
            ),
            "an sccache below the UDS floor resolves to the managed artifact: {result:?}"
        );
    }

    #[test]
    fn sccache_with_unreadable_version_is_rejected() {
        let machine = TestMachine::new();
        machine.install("sccache");
        let host = machine.host([("WATERUI_FAKE_SCCACHE_VERSION", "unknown")]);
        let result = smol::block_on(Sccache.check(&host));
        assert!(
            matches!(
                result,
                Err(ToolchainError::Fixable(SccacheInstallation::Managed(_)))
            ),
            "an sccache whose version cannot be read resolves to the managed artifact: {result:?}"
        );
    }

    #[test]
    fn port_is_deterministic_and_inside_the_reserved_block() {
        let port = per_user_server_port();
        assert_eq!(port, per_user_server_port());
        assert!(
            (22_000..=31_150).contains(&port),
            "the port stays below every host's ephemeral floor: {port}"
        );
    }

    #[test]
    fn distinct_identities_land_on_distinct_ports() {
        // 0 and 1 are the two uids that exist on every unix host; the names
        // that used to feed this hash (`ayy`/`cad`) collided.
        assert_ne!(port_for_identity("0"), port_for_identity("1"));
    }

    /// The environment contract: `RUSTC_WRAPPER` routes compiles through
    /// sccache, the port is always set — sccache < 0.9.0 knows nothing else —
    /// and unix additionally gets the socket that newer builds prefer. The
    /// Water home is injected so the test never touches the real `~/.water`
    /// or depends on this machine's home-path length.
    #[test]
    fn compilation_cache_env_sets_wrapper_port_and_unix_socket() {
        let water_home = tempfile::tempdir().expect("water home");
        #[cfg(unix)]
        let env =
            compilation_cache_env_in(Path::new("/toolchain/bin/sccache"), Some(water_home.path()))
                .expect("a scratch Water home yields the env");
        #[cfg(not(unix))]
        let env =
            compilation_cache_env_in(Path::new("/toolchain/bin/sccache"), Some(water_home.path()));

        assert!(
            env.contains(&("RUSTC_WRAPPER", OsString::from("/toolchain/bin/sccache"))),
            "RUSTC_WRAPPER routes rustc through sccache: {env:?}"
        );
        let port = env
            .iter()
            .find(|(key, _)| *key == "SCCACHE_SERVER_PORT")
            .map(|(_, value)| {
                value
                    .to_str()
                    .expect("port is text")
                    .parse::<u16>()
                    .expect("port parses")
            })
            .expect("SCCACHE_SERVER_PORT is always set");
        assert!((22_000..=31_150).contains(&port));

        #[cfg(unix)]
        {
            let socket = env
                .iter()
                .find(|(key, _)| *key == "SCCACHE_SERVER_UDS")
                .map(|(_, value)| value.to_string_lossy().into_owned())
                .expect("unix builds get the per-user socket");
            assert!(
                socket.ends_with("sccache/server.sock"),
                "the socket lives in a private dir under the Water home: {socket}"
            );
            assert!(
                socket.starts_with(&water_home.path().display().to_string()),
                "the socket lives under the injected Water home: {socket}"
            );
        }
        #[cfg(not(unix))]
        assert!(
            !env.iter().any(|(key, _)| *key == "SCCACHE_SERVER_UDS"),
            "non-unix builds only get the port"
        );
    }

    /// A socket path that cannot fit `sun_path` must not produce a socket
    /// that fails to bind — the port then carries the whole contract.
    #[cfg(unix)]
    #[test]
    fn oversized_home_path_falls_back_to_port_only() {
        let long_home = tempfile::tempdir()
            .expect("water home")
            .path()
            .join("a".repeat(200));
        assert!(
            super::server_socket_path_in(&long_home)
                .expect("creatable but overlong home")
                .is_none()
        );

        let home = tempfile::tempdir().expect("water home");
        let socket = super::server_socket_path_in(&home.path().join(".water"))
            .expect("a normal Water home gets a socket")
            .expect("a normal Water home gets a socket");
        assert!(socket.ends_with("sccache/server.sock"));
        assert!(
            socket
                .parent()
                .and_then(Path::parent)
                .is_some_and(|dir| dir.ends_with(".water")),
            "the socket's parent dir sits directly under the Water home: {}",
            socket.display()
        );
    }

    /// A socket dir another account can traverse is the exact exposure the
    /// mechanism exists to close — an existing `sccache/` wider than `0700`
    /// must fail rather than quietly offer the socket.
    #[cfg(unix)]
    #[test]
    fn a_socket_dir_wider_than_private_is_rejected() {
        use std::os::unix::fs::PermissionsExt as _;

        let home = tempfile::tempdir().expect("water home");
        let socket_dir = home.path().join("sccache");
        std::fs::create_dir(&socket_dir).expect("socket dir");
        std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o755))
            .expect("chmod socket dir");

        let error = super::server_socket_path_in(home.path())
            .expect_err("a world-traversable socket dir must be rejected");
        assert!(
            error.to_string().contains("0755") || error.to_string().contains("755"),
            "the error names the offending mode: {error}"
        );

        std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700))
            .expect("tighten socket dir");
        super::server_socket_path_in(home.path())
            .expect("a 0700 socket dir is accepted")
            .expect("a 0700 socket dir yields a socket");
    }

    #[test]
    fn missing_without_installer_uses_the_release_artifact() {
        let machine = TestMachine::new();
        let result = check(&machine);
        // The pinned build is the repair wherever upstream publishes an
        // artifact, because a distribution package can lag the floor.
        assert!(
            matches!(
                result,
                Err(ToolchainError::Fixable(SccacheInstallation::Managed(_)))
            ),
            "missing sccache resolves to the managed release artifact: {result:?}"
        );

        assert!(
            crate::toolchain::managed_tool::sccache().is_some(),
            "the assertions above hold because this host has a pinned artifact; \
             a host without one falls back to the package manager or stays manual"
        );
    }

    /// A Windows host without `winget` gets the managed archive — fixable,
    /// never a pointer at another prerequisite installer.
    #[test]
    fn windows_host_without_winget_is_fixable_managed() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(super::missing_sccache_on_windows(&host));
        match crate::toolchain::managed_tool::sccache() {
            Some(_) => assert!(
                matches!(
                    result,
                    ToolchainError::Fixable(SccacheInstallation::Managed(_))
                ),
                "no winget must fall back to the managed archive: {result:?}"
            ),
            None => assert!(
                matches!(result, ToolchainError::Unfixable(_)),
                "no managed build for this architecture must be unfixable: {result:?}"
            ),
        }
    }

    #[test]
    fn windows_host_with_winget_prefers_winget() {
        let machine = TestMachine::new();
        machine.install("winget");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(super::missing_sccache_on_windows(&host));
        assert!(
            matches!(result, ToolchainError::Fixable(SccacheInstallation::Winget)),
            "winget stays preferred when present: {result:?}"
        );
    }

    /// A pinned sccache unpacked under `~/.water/tools` satisfies the check
    /// — its version is known by construction, so no `--version` run is
    /// needed — even though nothing named `sccache` is on `PATH`.
    /// The three checks above assert the managed artifact unconditionally, which
    /// is only right because `managed_tool::sccache()` covers every triple this
    /// CLI is built for. It is a `cfg!` cascade, so a new host target silently
    /// returns `None` and would turn those assertions into a different test —
    /// this one fails instead, and names the missing artifact.
    #[test]
    fn every_supported_host_has_a_pinned_artifact() {
        assert!(
            crate::toolchain::managed_tool::sccache().is_some(),
            "no pinned sccache artifact for this host triple: add it to \
             `managed_tool::sccache()` or the doctor cannot repair sccache here"
        );
    }

    #[test]
    fn ok_when_sccache_is_managed() {
        let machine = TestMachine::new();
        let Some(tool) = crate::toolchain::managed_tool::sccache() else {
            return; // this architecture has no managed build
        };
        let host = machine.host(Vec::<(String, String)>::new());
        let install_dir = tool.install_dir(&host).unwrap();
        machine.file(
            install_dir
                .join(&tool.binary)
                .strip_prefix(machine.root())
                .unwrap(),
            "",
        );
        let result = smol::block_on(Sccache.check(&host));
        assert!(
            result.is_ok(),
            "a managed sccache must satisfy the check: {result:?}"
        );
    }

    /// A managed install also satisfies the check when the PATH copy is
    /// below the floor: `Sccache::path` resolves the managed binary first,
    /// so the stale copy never serves a build.
    #[test]
    fn ok_when_managed_shadows_a_stale_path_copy() {
        let machine = TestMachine::new();
        let Some(tool) = crate::toolchain::managed_tool::sccache() else {
            return; // this architecture has no managed build
        };
        machine.install("sccache");
        let host = machine.host([("WATERUI_FAKE_SCCACHE_VERSION", "0.8.2")]);
        let install_dir = tool.install_dir(&host).unwrap();
        machine.file(
            install_dir
                .join(&tool.binary)
                .strip_prefix(machine.root())
                .unwrap(),
            "",
        );
        let result = smol::block_on(Sccache.check(&host));
        assert!(
            result.is_ok(),
            "the managed copy must shadow a stale PATH sccache: {result:?}"
        );
    }

    #[test]
    fn missing_with_installer_is_fixable() {
        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        #[cfg(target_os = "windows")]
        machine.install("winget");
        let result = check(&machine);
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing sccache with a package manager must be fixable: {result:?}"
        );
    }
}
