//! Produce the inspector's release archive from a `WaterUI` checkout.
//!
//! `package_inspector_client <workspace> <output-dir>` builds
//! `apps/inspector` from `<workspace>` in release mode, verifies the
//! binary's `--identity` answer against the workspace's resolved
//! `waterui`/protocol versions, and writes
//! `waterui-inspector-<host triple>.zip` under `<output-dir>` — refusing to
//! overwrite an existing archive. Release pipeline wiring is not connected
//! yet; this is the producer it will call.

use std::path::PathBuf;

use clap::Parser;
use eyre::{Context as _, Result};
use waterui_cli::inspector::package_release_client;
use waterui_cli::toolchain::Host;

/// Produce the inspector's release archive from a `WaterUI` checkout.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The `WaterUI` checkout whose `apps/inspector` builds the client.
    workspace: PathBuf,
    /// The directory `waterui-inspector-<host triple>.zip` lands in —
    /// created when missing, never overwritten inside.
    output_dir: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let host = Host::current();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init()
        .expect("the package tool's logger installs once");

    smol::block_on(async {
        let archive = package_release_client(&host, &args.workspace, &args.output_dir)
            .await
            .wrap_err("failed to package the inspector client")?;
        tracing::info!(archive = %archive.display(), "release archive written");
        Ok(())
    })
}
