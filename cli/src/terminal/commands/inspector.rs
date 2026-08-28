//! `water inspector` command implementation.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::Result;

use crate::shell::Shell;
use crate::{header, note, success};
use waterui_cli::inspector::{InspectorLaunchOptions, InspectorPlatform, launch_inspector_session};

/// Target platform for inspector app.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliInspectorPlatform {
    /// iOS Simulator.
    Ios,
    /// macOS.
    Macos,
    /// Android Emulator / Device.
    Android,
}

impl From<CliInspectorPlatform> for InspectorPlatform {
    fn from(platform: CliInspectorPlatform) -> Self {
        match platform {
            CliInspectorPlatform::Ios => Self::IosSimulator,
            CliInspectorPlatform::Macos => Self::Macos,
            CliInspectorPlatform::Android => Self::Android,
        }
    }
}

/// Arguments for `water inspector`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Runtime app debug endpoint (`host:port`).
    ///
    /// Omit it to attach to a debug build running on this machine, which
    /// publishes where it is listening.
    #[arg(long)]
    target: Option<String>,

    /// One-time token for runtime endpoint authentication.
    #[arg(long)]
    token: Option<String>,

    /// Target platform for the inspector app.
    #[arg(short, long, value_enum, default_value = "macos")]
    platform: CliInspectorPlatform,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

/// Run the inspector command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
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
    note!(shell, "Session token: {}", token);

    let mut session = launch_inspector_session(
        &project_path,
        args.platform.into(),
        InspectorLaunchOptions {
            target_addr: target.to_string(),
            token: token.clone(),
        },
    )
    .await?;

    // Keep the inspector app alive after CLI exits.
    session.detach();
    success!(shell, "Inspector app launched");
    note!(
        shell,
        "Ensure the target app runs with WATERUI_INSPECTOR_TOKEN={token} and matching inspector endpoint."
    );
    Ok(())
}

/// The debug build to attach to, when the caller did not name one.
///
/// Ambiguity is reported rather than guessed at: attaching to the wrong
/// application is more confusing than being asked which one.
fn discover(shell: &Shell) -> Result<waterui_inspector_protocol::discovery::Advertisement> {
    use color_eyre::eyre::eyre;

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
    use color_eyre::eyre::eyre;

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
