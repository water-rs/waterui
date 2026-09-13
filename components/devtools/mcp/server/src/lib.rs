//! `waterui-mcp`: an MCP server that lets an agent drive a headless `WaterUI`
//! app.
//!
//! The server mounts a [`waterui_testing::OffscreenApp`] and exposes its
//! accessibility tree, input dispatch, and screenshots as MCP tools —
//! `snapshot`, `find`, `act`, `pointer`, `key`, `type_text`, `wait`,
//! `screenshot`, and `restart`.
//!
//! # Threading
//!
//! `OffscreenApp` is `!Send` and `WaterUI` mounts on the calling thread, so
//! [`serve`] mounts the app on the thread that calls it and moves the
//! `aither-mcp` server onto a spawned thread. Tool calls travel back to the
//! session thread over a channel, execute one at a time, and answer over a
//! per-command reply channel. When the client disconnects, the server thread
//! exits, its tools drop the last command sender, the session loop ends, and
//! `serve` returns.

mod png;
mod session;
mod tools;
mod tree;

use aither_core::llm::tool::Tools;
use aither_mcp::transport::{BidirectionalTransport, StdioTransport};
use aither_mcp::{McpError, McpServer};
use waterui_testing::OffscreenApp;

pub use aither_mcp::protocol::ServerInfo;
use session::{Session, SessionHandle};

/// Server instructions handed to the client during `initialize`.
const INSTRUCTIONS: &str = include_str!("instructions.md");

/// Serves a session over standard input and output.
///
/// Equivalent to `serve(StdioTransport::new(), mount, info)`.
///
/// # Errors
///
/// Returns an error when the server thread cannot be spawned or the server
/// itself fails with a fatal transport error.
pub fn serve_stdio(mount: impl FnMut() -> OffscreenApp, info: ServerInfo) -> Result<(), McpError> {
    serve(StdioTransport::new(), mount, info)
}

/// Serves a session over `transport`, blocking until the client disconnects.
///
/// `mount` is invoked once on the calling thread to create the app, and again
/// there whenever the `restart` tool runs — the calling thread owns the
/// `!Send` app for the whole session. The MCP server itself runs on a spawned
/// thread; see the crate-level docs for the channel protocol between them.
///
/// # Errors
///
/// Returns an error when the server thread cannot be spawned or the server
/// itself fails with a fatal transport error.
///
/// # Panics
///
/// Panics when `mount` does, or when the server thread panics — the panic
/// propagates on this thread when the session ends.
pub fn serve<T>(
    transport: T,
    mount: impl FnMut() -> OffscreenApp,
    info: ServerInfo,
) -> Result<(), McpError>
where
    T: BidirectionalTransport + Sync + Send + 'static,
{
    let mut session = Session::new(mount);

    let (tx, rx) = async_channel::unbounded();
    let mut tools = Tools::new();
    // Scope the handle: it holds a command sender, and the session loop ends
    // only once the last sender drops — which must be the copies inside
    // `tools`, released when the server thread exits.
    {
        let handle = SessionHandle::new(tx);
        tools::register_all(&mut tools, &handle);
    }

    let ServerInfo { name, version } = info;
    let server = std::thread::Builder::new()
        .name("waterui-mcp-server".to_owned())
        .spawn(move || {
            futures_lite::future::block_on(
                McpServer::new(transport, tools, name, version.unwrap_or_default())
                    .with_instructions(INSTRUCTIONS)
                    .run(),
            )
        })
        .map_err(|error| {
            McpError::Transport(format!(
                "failed to spawn waterui-mcp server thread: {error}"
            ))
        })?;

    while let Ok(command) = futures_lite::future::block_on(rx.recv()) {
        session.execute(command);
    }

    match server.join() {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
