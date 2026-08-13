//! `water inspector` command implementation.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::Result;
use sha2::Digest as _;

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
    let (target, token) = match args.target {
        Some(target) => (
            target.parse::<SocketAddr>()?,
            args.token.unwrap_or_else(generate_session_token),
        ),
        None => {
            let found = discover(shell)?;
            note!(shell, "Attaching to {} (pid {})", found.app_name, found.pid);
            (found.addr, args.token.unwrap_or(found.token))
        }
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
    use color_eyre::eyre::{bail, eyre};

    let mut found = waterui_inspector_protocol::discovery::list()
        .map_err(|error| eyre!("could not read advertised inspector endpoints: {error}"))?;

    match found.len() {
        0 => bail!(
            "no running WaterUI debug build was found.\n\
             Start one, or pass --target host:port for an application on another \
             machine or device."
        ),
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
            bail!("several debug builds are running; pass --target host:port to choose one")
        }
    }
}

fn generate_session_token() -> String {
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let mut hasher = sha2::Sha256::new();
    hasher.update(now_ns.to_le_bytes());
    hasher.update(pid.to_le_bytes());
    hasher.update(format!("{:?}", std::thread::current().id()).as_bytes());
    let hash = hasher.finalize();
    let mut token = String::with_capacity(32);
    for b in &hash[..16] {
        use std::fmt::Write as _;
        let _ = write!(&mut token, "{b:02x}");
    }
    token
}
