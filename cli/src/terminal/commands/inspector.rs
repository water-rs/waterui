//! `water inspector` command implementation.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use eyre::Result;

use crate::shell::Shell;
use crate::{header, note, success};
use waterui_cli::inspector::{InspectorLaunchOptions, InspectorTarget, launch_inspector_session};

/// Where the application being inspected runs.
///
/// This describes the server — the endpoint compiled into the inspected
/// application — and how the client reaches it. The inspector client itself
/// always runs on this machine, whatever the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliInspectorPlatform {
    /// An application running on this machine; its endpoint is discovered
    /// or given with `--target`.
    Local,
    /// A macOS application, dialed directly at `--target`. On a Mac that is
    /// this machine's own apps; elsewhere it is a remote endpoint.
    Macos,
    /// A Linux application, dialed directly at `--target`. On Linux that is
    /// this machine's own apps; elsewhere it is a remote endpoint.
    Linux,
    /// A Windows application, dialed directly at `--target`. On Windows that
    /// is this machine's own apps; elsewhere it is a remote endpoint.
    Windows,
    /// An iOS application, dialed directly at `--target`. A simulator shares
    /// this machine's loopback (`127.0.0.1:<port>`); a device needs its
    /// routable address on the network.
    Ios,
    /// An application on an Android device or emulator; reached through
    /// `adb forward` (`--device` selects the adb target).
    Android,
}

impl CliInspectorPlatform {
    /// The flag spelling of this platform, for error messages.
    const fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }

    /// Whether the platform's server can run on this machine — `local`, or
    /// a desktop OS equal to the one this binary runs on (its own
    /// `cfg!(target_os)` proves it: the `water` binary always runs on the
    /// developer machine). Only those can be discovered or have a token
    /// looked up here; every other platform is a remote endpoint reached
    /// through `--target` and `--token` (or, for Android, `adb forward`).
    const fn can_run_on_this_machine(self) -> bool {
        let host_desktop = match self {
            Self::Macos => cfg!(target_os = "macos"),
            Self::Linux => cfg!(target_os = "linux"),
            Self::Windows => cfg!(target_os = "windows"),
            Self::Local | Self::Ios | Self::Android => false,
        };
        matches!(self, Self::Local) || host_desktop
    }
}

/// Arguments for `water inspector`.
#[derive(ClapArgs)]
pub struct Args {
    /// Runtime app debug endpoint (`host:port`).
    ///
    /// Omit it to attach to a debug build running on this machine, which
    /// publishes where it is listening. Every `--platform` whose server
    /// cannot run here requires it — `ios`, `android`, and a desktop OS
    /// different from this machine's. For iOS it is the endpoint as the
    /// device or simulator sees it (a simulator shares this machine's
    /// loopback, a device needs its routable address); for Android it is
    /// the endpoint the application bound on the device — the port is what
    /// `adb forward` carries.
    #[arg(long)]
    target: Option<String>,

    /// One-time token for runtime endpoint authentication.
    #[arg(long)]
    token: Option<String>,

    /// Where the application being inspected runs. The inspector client
    /// always runs on this machine; the platform only says how the target's
    /// endpoint is reached.
    #[arg(short, long, value_enum, default_value = "local")]
    platform: CliInspectorPlatform,

    /// Android device serial or AVD name (only with `--platform android`).
    /// Without it the remembered last-used device wins, then an unambiguous
    /// single candidate, then a picker.
    #[arg(short, long)]
    device: Option<String>,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

// The token is a credential: the parse never reaches diagnostics.
impl std::fmt::Debug for Args {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Args")
            .field("target", &self.target)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .field("platform", &self.platform)
            .field("device", &self.device)
            .field("path", &self.path)
            .finish()
    }
}

impl Args {
    /// The project directory this command works on.
    pub(crate) fn project_dir(&self) -> &std::path::Path {
        &self.path
    }
}

/// Run the inspector command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    // The client never runs where the inspected application is: a phone or
    // simulator cannot show the inspector and keep the inspected application
    // answering — Android freezes whichever of the two is in the background
    // (water-rs/waterui#2230). A target's platform only chooses how its
    // endpoint is reached.
    if args.device.is_some() && args.platform != CliInspectorPlatform::Android {
        eyre::bail!("`--device` is supported only with `--platform android`.");
    }
    require_device_endpoint(args.platform, args.target.as_deref(), args.token.as_deref())?;

    let project_path = crate::project_path::canonicalize(&args.path)?;

    header!(shell, "Inspector");

    // An address given explicitly wins; otherwise the running application is
    // asked where it is, which is what makes attaching a single command.
    let (target, token) = if let Some(target) = args.target {
        let addr = target.parse::<SocketAddr>()?;
        let token = match args.token {
            Some(token) => token,
            None => token_for(addr)?,
        };
        (addr, token)
    } else {
        let found = discover(shell)?;
        note!(shell, "Attaching to {} (pid {})", found.app_name, found.pid);
        (found.addr, args.token.unwrap_or(found.token))
    };

    note!(shell, "Target runtime endpoint: {}", target);

    let host = waterui_cli::toolchain::Host::current();
    let inspector_target = match args.platform {
        CliInspectorPlatform::Android => {
            let spinner = shell.spinner("Scanning for devices...");
            let device = super::select_android_target(
                shell,
                &host,
                waterui_cli::water_dir::device_memory_key(
                    waterui_cli::platform::TargetBackend::Hydrolysis,
                    waterui_cli::platform::TargetPlatform::Android,
                ),
                args.device.as_deref(),
                spinner.as_ref(),
            )
            .await;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            // An emulator has no serial before it boots, and adb cannot
            // forward to a device that is not up.
            let serial = device?.launch(&host).await?.identifier().to_owned();
            note!(
                shell,
                "Forwarding a local port to {} on {serial}",
                target.port()
            );
            InspectorTarget::AndroidDevice {
                serial,
                port: target.port(),
            }
        }
        CliInspectorPlatform::Local
        | CliInspectorPlatform::Macos
        | CliInspectorPlatform::Linux
        | CliInspectorPlatform::Windows
        | CliInspectorPlatform::Ios => InspectorTarget::Direct(target),
    };

    let mut session = launch_inspector_session(
        &host,
        &project_path,
        InspectorLaunchOptions {
            target: inspector_target,
            token,
        },
        Some(shell.build_progress()),
    )
    .await?;

    // Keep the inspector app alive after the CLI exits. The `adb forward` it
    // dials through stays registered with the adb server for the same
    // reason — it belongs to the detached app.
    session.detach();
    success!(shell, "Inspector app launched");
    Ok(())
}

/// Inspecting an application off this machine needs the endpoint and token
/// spelled out.
///
/// Discovery lists the endpoints advertised on this computer; an
/// application on a device, a simulator, or another machine's OS
/// advertises nothing here, so there is nothing to discover for it.
fn require_device_endpoint(
    platform: CliInspectorPlatform,
    target: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
    if !platform.can_run_on_this_machine() && (target.is_none() || token.is_none()) {
        eyre::bail!(
            "`--platform {}` needs --target <addr:port> and --token <token>: the endpoint and \
             one-time token the application advertises",
            platform.name()
        );
    }
    Ok(())
}

/// The debug build to attach to, when the caller did not name one.
///
/// Ambiguity is reported rather than guessed at: attaching to the wrong
/// application is more confusing than being asked which one.
fn discover(shell: &Shell) -> Result<waterui_inspector_protocol::discovery::Advertisement> {
    use eyre::eyre;

    let mut found = waterui_inspector_protocol::discovery::list()
        .map_err(|error| eyre!("could not read advertised inspector endpoints: {error}"))?;

    match found.len() {
        // This match produces the result, so each arm yields one rather than
        // returning early: `bail!` expands to a `return ...;`, and a macro
        // ending in a semicolon is not an expression.
        0 => Err(eyre!(
            "no running WaterUI debug build was found.\n\
             Start one, or pass --target host:port for an application on another \
             machine or device."
        )),
        1 => Ok(found.remove(0)),
        _ => {
            for advertisement in &found {
                note!(
                    shell,
                    "  {} (pid {}) at {}",
                    advertisement.app_name,
                    advertisement.pid,
                    advertisement.addr
                );
            }
            Err(eyre!(
                "several debug builds are running; pass --target host:port to choose one"
            ))
        }
    }
}

/// The token the application listening at `addr` is asking for.
///
/// An endpoint accepts exactly one token, chosen by the application, so there is
/// nothing for this side to invent: a made-up token is refused at the handshake
/// and looks to the developer like the inspector simply never attached. A local
/// application publishes its token, so it can be looked up; one on a device
/// cannot be, and that is said plainly instead.
fn token_for(addr: SocketAddr) -> Result<String> {
    use eyre::eyre;

    let found = waterui_inspector_protocol::discovery::list()
        .map_err(|error| eyre!("could not read advertised inspector endpoints: {error}"))?;

    // An endpoint reachable from another machine advertises the port it bound on
    // every interface, so the port is what identifies it; the host names the way
    // in, which differs between the application and whoever dials it.
    found
        .into_iter()
        .find(|advertisement| advertisement.addr.port() == addr.port())
        .map(|advertisement| advertisement.token)
        .map_or_else(
            || {
                Err(eyre!(
                    "no application on this machine advertises {addr}.\n\
                     Pass --token with the token it printed at startup, which is how \
                     to attach to an application on another machine or device."
                ))
            },
            Ok,
        )
}

#[cfg(test)]
mod tests {
    use super::{Args, CliInspectorPlatform, require_device_endpoint};

    /// `--token` is a credential: `Debug` redacts a set value and keeps
    /// `None` and the rest of the parse intact.
    #[test]
    fn the_args_debug_never_shows_the_token() {
        let args = Args {
            target: Some(String::from("127.0.0.1:23106")),
            token: Some(String::from("s3cret-cli-token")),
            platform: super::CliInspectorPlatform::Local,
            device: Some(String::from("pixel-9-pro")),
            path: std::path::PathBuf::from("."),
        };
        let rendered = format!("{args:?}");
        assert!(!rendered.contains("s3cret-cli-token"), "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");
        assert!(rendered.contains("23106"), "{rendered}");
        assert!(rendered.contains("pixel-9-pro"), "{rendered}");

        let without = Args {
            token: None,
            ..args
        };
        let rendered = format!("{without:?}");
        assert!(rendered.contains("token: None"), "{rendered}");
    }

    fn expected_error(platform: CliInspectorPlatform) -> String {
        format!(
            "`--platform {}` needs --target <addr:port> and --token <token>: the endpoint and \
             one-time token the application advertises",
            platform.name()
        )
    }

    /// The desktop-OS platform matching the machine these tests run on.
    const fn host_desktop() -> CliInspectorPlatform {
        if cfg!(target_os = "macos") {
            CliInspectorPlatform::Macos
        } else if cfg!(target_os = "windows") {
            CliInspectorPlatform::Windows
        } else {
            CliInspectorPlatform::Linux
        }
    }

    /// A platform whose server can never be on this machine.
    const REMOTE_PLATFORMS: &[CliInspectorPlatform] = if cfg!(target_os = "macos") {
        &[
            CliInspectorPlatform::Ios,
            CliInspectorPlatform::Android,
            CliInspectorPlatform::Linux,
            CliInspectorPlatform::Windows,
        ]
    } else if cfg!(target_os = "windows") {
        &[
            CliInspectorPlatform::Ios,
            CliInspectorPlatform::Android,
            CliInspectorPlatform::Macos,
            CliInspectorPlatform::Linux,
        ]
    } else {
        &[
            CliInspectorPlatform::Ios,
            CliInspectorPlatform::Android,
            CliInspectorPlatform::Macos,
            CliInspectorPlatform::Windows,
        ]
    };

    #[test]
    fn platforms_off_this_machine_require_the_target_and_the_token() {
        for (target, token) in [
            (None, None),
            (Some("127.0.0.1:47123"), None),
            (None, Some("token")),
        ] {
            for &platform in REMOTE_PLATFORMS {
                let error = require_device_endpoint(platform, target, token)
                    .expect_err("an application off this machine advertises nothing on it");
                assert_eq!(error.to_string(), expected_error(platform));
            }
        }
    }

    #[test]
    fn a_remote_platform_accepts_both_flags() {
        for &platform in REMOTE_PLATFORMS {
            require_device_endpoint(platform, Some("10.0.0.2:47123"), Some("token"))
                .expect("both flags given");
        }
    }

    #[test]
    fn platforms_on_this_machine_may_be_discovered() {
        for platform in [CliInspectorPlatform::Local, host_desktop()] {
            require_device_endpoint(platform, None, None).expect("discovery is available");
        }
    }

    #[test]
    fn the_host_desktop_is_the_only_desktop_platform_on_this_machine() {
        for platform in [
            CliInspectorPlatform::Macos,
            CliInspectorPlatform::Linux,
            CliInspectorPlatform::Windows,
        ] {
            assert_eq!(
                platform.can_run_on_this_machine(),
                platform == host_desktop(),
                "{} should match the host iff it is the host's OS",
                platform.name()
            );
        }
    }
}
