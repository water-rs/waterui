//! `water mcp`: serves an MCP session that drives the app headless.
//!
//! The CLI process fronts the generated Hydrolysis MCP binary for the whole
//! session rather than `exec`ing it: `initialize` and `tools/list` must
//! answer inside the client's startup timeout even when the first Hydrolysis
//! build is still compiling, so the front serves those from the static
//! contract in `waterui-mcp-protocol` and forwards `tools/call` to the child
//! once it is up.

pub mod preview;
mod proxy;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aither_core::llm::tool::Tools;
use aither_mcp::McpServer;
use aither_mcp::transport::StdioTransport;
use eyre::{Context as _, Result};
use serde::Serialize;
use tracing::info;
use waterui_mcp_protocol::{INSTRUCTIONS, register_session_tools};
use waterui_preview_protocol::hydrolysis::McpRunConfig;

use crate::hydrolysis::backend::HydrolysisBackend;
use crate::platform::TargetPlatform;
use crate::project::Project;

pub use proxy::ChildProxy;

/// One `water mcp` session.
#[derive(Debug)]
pub struct McpSessionRequest {
    /// `WaterUI` project directory.
    pub project_path: PathBuf,
    /// Viewport width in logical pixels.
    pub width: u32,
    /// Viewport height in logical pixels.
    pub height: u32,
    /// Display scale factor.
    pub scale_factor: f64,
    /// `sccache` binary used for compilation caching, when available.
    pub sccache_path: Option<PathBuf>,
    /// Server name reported to the client at `initialize`.
    pub server_name: String,
}

/// Serves an MCP session on stdio until the client disconnects.
///
/// The child build starts immediately but in the background; the front
/// answers `initialize` and `tools/list` itself while it compiles. When the
/// client closes stdio, the child is killed and this returns.
///
/// # Errors
///
/// Returns an error when the stdio transport fails. A child that fails to
/// build does not fail the session — `tools/call` reports the build error to
/// the model until `restart` retries it.
///
/// # Panics
///
/// Panics if a static tool registration fails — a programming error, not a
/// runtime condition.
pub async fn serve_mcp(request: McpSessionRequest) -> Result<()> {
    let McpSessionRequest {
        project_path,
        width,
        height,
        scale_factor,
        sccache_path,
        server_name,
    } = request;

    let proxy = Arc::new(ChildProxy::new(
        project_path.clone(),
        width,
        height,
        scale_factor,
        sccache_path.clone(),
    ));
    let mut tools = Tools::new();
    register_session_tools(&mut tools, Arc::clone(&proxy));
    // `preview` is served by this front — it renders through the preview
    // machinery rather than the running app, and must answer before the
    // child's first build finishes.
    tools
        .register(preview::PreviewTool::new(project_path, sccache_path))
        .expect("static tool registration cannot fail");

    // Schedule the first build before the server starts so it is already
    // compiling while `initialize` is being answered.
    proxy.rebuild().await;

    let result = McpServer::new(
        StdioTransport::new(),
        tools,
        server_name,
        env!("CARGO_PKG_VERSION"),
    )
    .with_instructions(INSTRUCTIONS)
    .run()
    .await;

    // The client closed stdio (or the session is tearing down): the child
    // must not outlive us.
    proxy.shutdown().await;
    info!("water mcp: session ended");

    result.map_err(|error| eyre::eyre!("MCP stdio server failed: {error}"))
}

/// The Hydrolysis platform of this host — `water mcp` always builds the app
/// for the machine the agent is running on.
const fn host_platform() -> TargetPlatform {
    #[cfg(target_os = "macos")]
    {
        TargetPlatform::MacOS
    }
    #[cfg(target_os = "linux")]
    {
        TargetPlatform::Linux
    }
    #[cfg(target_os = "windows")]
    {
        TargetPlatform::Windows
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        panic!("`water mcp` requires macOS, Linux, or Windows");
    }
}

/// The project-level `.mcp.json`: registers `water mcp` for MCP clients that
/// read project config. The command carries no `--path` because the client's
/// working directory is the project root.
#[derive(Serialize)]
struct McpJson {
    #[serde(rename = "mcpServers")]
    mcp_servers: McpJsonServers,
}

/// The `mcpServers` map of the project-level `.mcp.json`.
#[derive(Serialize)]
struct McpJsonServers {
    app: McpJsonServer,
}

/// One server entry of the project-level `.mcp.json`.
#[derive(Serialize)]
struct McpJsonServer {
    command: &'static str,
    args: [&'static str; 1],
}

const MCP_JSON: McpJson = McpJson {
    mcp_servers: McpJsonServers {
        app: McpJsonServer {
            command: "water",
            args: ["mcp"],
        },
    },
};

/// Writes `.mcp.json` at `project_root` when none exists and reports whether
/// it wrote. The file is user-owned — an existing one is never overwritten.
pub(crate) async fn ensure_mcp_json(project_root: &Path) -> io::Result<bool> {
    let path = project_root.join(".mcp.json");
    match smol::fs::metadata(&path).await {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let json = serde_json::to_vec_pretty(&MCP_JSON).map_err(io::Error::other)?;
    smol::fs::write(&path, json).await?;
    Ok(true)
}

/// Writes the [`McpRunConfig`] JSON next to the backend sources and returns
/// its path; the file is rewritten on every build.
pub(crate) async fn write_run_config(project: &Project, config: &McpRunConfig) -> Result<PathBuf> {
    let path = project
        .backend_path::<HydrolysisBackend>()
        .join("mcp-run.json");
    let json = serde_json::to_vec_pretty(config)
        .wrap_err("failed to serialize the hydrolysis MCP run config")?;
    smol::fs::write(&path, json)
        .await
        .wrap_err_with(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}
