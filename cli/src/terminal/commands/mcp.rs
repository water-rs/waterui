//! `water mcp` command implementation.
//!
//! Serves the app to an agent over MCP on stdio. The CLI process fronts the
//! generated Hydrolysis MCP binary for the whole session: `initialize` and
//! `tools/list` answer before a cold build finishes, `tools/call` is
//! forwarded once the child is up, and `restart` rebuilds from the current
//! sources.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use eyre::{Result, bail};
use tracing::info;

use crate::shell::Shell;
use waterui_cli::mcp::{McpSessionRequest, serve_mcp};

/// Arguments for the `mcp` command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Viewport size in logical pixels, `WIDTHxHEIGHT`.
    #[arg(long, default_value = "390x844")]
    viewport: String,

    /// Display scale factor for the headless render (2 on Retina-class
    /// displays).
    #[arg(long, default_value_t = 2.0)]
    scale: f64,
}

impl Args {
    /// Parses `--viewport` into the integer dimensions the run config takes.
    fn viewport(&self) -> Result<(u32, u32)> {
        super::parse_viewport(&self.viewport)
    }

    /// Validates `--scale`.
    fn scale(&self) -> Result<f64> {
        if !self.scale.is_finite() || self.scale <= 0.0 {
            bail!("Invalid scale factor: must be a positive finite number");
        }
        Ok(self.scale)
    }
}

/// Run the `mcp` command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    if shell.is_json() {
        bail!("`water mcp` cannot run with `--json`: stdout carries the MCP protocol");
    }
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let (width, height) = args.viewport()?;
    let scale_factor = args.scale()?;
    let server_name = waterui_cli::project::read_project_crate_name(&project_path).await?;
    let sccache_path = super::detect_sccache_path(shell).await;

    info!(
        path = %project_path.display(),
        viewport = %args.viewport,
        scale = scale_factor,
        "serving an MCP session"
    );

    serve_mcp(McpSessionRequest {
        project_path,
        width,
        height,
        scale_factor,
        sccache_path,
        server_name,
    })
    .await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct McpCommand {
        #[command(flatten)]
        args: Args,
    }

    fn parse(args: &[&str]) -> Args {
        McpCommand::try_parse_from(args).expect("args parse").args
    }

    #[test]
    fn parses_default_args() {
        let args = parse(&["mcp"]);
        assert_eq!(args.viewport().expect("default viewport"), (390, 844));
        assert!((args.scale().expect("default scale") - 2.0).abs() <= f64::EPSILON);
    }

    #[test]
    fn parses_viewport_and_scale() {
        let args = parse(&[
            "mcp",
            "--path",
            "/tmp/app",
            "--viewport",
            "800x600",
            "--scale",
            "3.0",
        ]);
        assert_eq!(args.path, PathBuf::from("/tmp/app"));
        assert_eq!(args.viewport().expect("viewport"), (800, 600));
        assert!((args.scale().expect("scale") - 3.0).abs() <= f64::EPSILON);
    }

    #[test]
    fn rejects_bad_viewport() {
        for viewport in ["0x100", "390x0", "100x", "axb", "wide", "1.5x2"] {
            assert!(
                parse(&["mcp", "--viewport", viewport]).viewport().is_err(),
                "viewport `{viewport}` should be rejected"
            );
        }
    }

    #[test]
    fn rejects_bad_scale() {
        assert!(parse(&["mcp", "--scale", "0"]).scale().is_err());
        assert!(parse(&["mcp", "--scale", "NaN"]).scale().is_err());
    }
}
