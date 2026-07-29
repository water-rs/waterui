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
    #[arg(long)]
    target: String,

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
    let target: SocketAddr = args.target.parse()?;
    let token = args.token.unwrap_or_else(generate_session_token);

    header!(shell, "Inspector");
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
