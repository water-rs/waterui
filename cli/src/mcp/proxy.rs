//! The child-process side of `water mcp`: builds the managed Hydrolysis
//! backend in `waterui-mcp-mode`, spawns it, and forwards `tools/call`
//! requests to it over the child's own MCP stdio link.
//!
//! The proxy is the [`ToolDispatch`] the fronting `McpServer` registers its
//! nine tools against. It answers `initialize`/`tools/list` through the
//! static protocol crate immediately — while the child is still building —
//! and parks `tools/call` on a readiness cell until the child is up.

use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use aither_core::llm::tool::ToolResult;
use aither_mcp::protocol::{
    CallToolParams, CallToolResult, Content, InitializeParams, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse,
};
use aither_mcp::transport::{StreamTransport, Transport};
use async_channel::{Receiver, Sender};
use async_lock::{Mutex, OnceCell};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use eyre::{Context as _, Result, bail, eyre};
use futures_lite::io::BufReader;
use serde::Serialize;
use smol::Task;
use smol::process::Child;
use tracing::{debug, error, info};
use waterui_mcp_protocol::{
    ActArgs, AdvanceArgs, FindArgs, KeyArgs, PointerArgs, RestartArgs, ScreenshotArgs,
    SnapshotArgs, ToolDispatch, TypeTextArgs, WaitArgs,
};
use waterui_preview_protocol::hydrolysis::{MCP_RUN_CONFIG_ENV, McpRunConfig};

use crate::build::{BuildOptions, RustLinkage};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::hydrolysis::platform::{
    build_hydrolysis_with_envs_and_features, built_hydrolysis_binary_path,
    stage_hydrolysis_shared_runtime,
};
use crate::mcp::{host_platform, write_run_config};
use crate::preview::hydrolysis::{
    HydrolysisPreviewTheme, ensure_hydrolysis_backend_ready, stage_hydrolysis_resources,
};

/// The Cargo feature that builds the generated backend as an MCP child.
const HYDROLYSIS_MCP_FEATURE: &str = "waterui-mcp-mode";

/// Everything one build of the child needs, cloned into the build task.
#[derive(Debug, Clone)]
struct ChildConfig {
    /// `WaterUI` project directory.
    project_path: PathBuf,
    /// Viewport width in logical pixels.
    width: u32,
    /// Viewport height in logical pixels.
    height: u32,
    /// Display scale factor.
    scale_factor: f64,
    /// `sccache` binary for compilation caching, when available.
    sccache_path: Option<PathBuf>,
}

/// One `tools/call` handed to the child driver task.
#[derive(Debug)]
struct ChildCall {
    /// Tool name as the client called it.
    name: &'static str,
    /// JSON-serialized tool arguments.
    arguments: serde_json::Value,
    /// Where the mapped [`ToolResult`] travels back to the tool handler.
    reply: Sender<ToolResult>,
}

/// The child's MCP link: line-delimited JSON-RPC over its stdin/stdout.
type ChildTransport =
    StreamTransport<BufReader<smol::process::ChildStdout>, smol::process::ChildStdin>;

/// Mutable session state, only ever held across short sections.
#[derive(Debug)]
struct ProxyInner {
    /// Resolves to the live child's call channel, or the build error.
    /// Replaced with a fresh cell at the start of every rebuild.
    ready: Arc<OnceCell<Result<Sender<ChildCall>, String>>>,
    /// The task that builds the child and then drives its call channel.
    /// Cancelling it drops the [`Child`] (kill-on-drop) mid-build or mid-call.
    task: Option<Task<()>>,
}

/// The `water mcp` front: owns the child lifecycle and forwards tool calls.
#[derive(Debug)]
pub struct ChildProxy {
    config: ChildConfig,
    inner: Mutex<ProxyInner>,
    /// Serializes rebuilds; `restart` takes it so two restarts cannot race.
    rebuild_lock: Mutex<()>,
}

impl ChildProxy {
    /// A proxy for the project at `project_path`; the child is not built until
    /// [`Self::rebuild`] runs.
    #[must_use]
    pub fn new(
        project_path: PathBuf,
        width: u32,
        height: u32,
        scale_factor: f64,
        sccache_path: Option<PathBuf>,
    ) -> Self {
        Self {
            config: ChildConfig {
                project_path,
                width,
                height,
                scale_factor,
                sccache_path,
            },
            inner: Mutex::new(ProxyInner {
                ready: Arc::new(OnceCell::new()),
                task: None,
            }),
            rebuild_lock: Mutex::new(()),
        }
    }

    /// Kills the current child if any, then spawns the task that rebuilds and
    /// re-drives it. Returns as soon as the build task is scheduled — callers
    /// gate on the readiness cell, not on this function.
    pub async fn rebuild(&self) {
        let _serialized = self.rebuild_lock.lock().await;
        let (cell, calls, rx) = self.swap_ready_cell().await;
        let task = smol::spawn(build_and_drive(self.config.clone(), cell, calls, rx));
        self.inner.lock().await.task = Some(task);
    }

    /// Cancels the current build/child task and swaps in a fresh readiness
    /// cell, returning it with the channel the new task drives.
    ///
    /// The outgoing cell is resolved with a restart error when it is still
    /// unset: a caller parked on it would otherwise wait forever, because the
    /// cancelled task can no longer publish a result.
    async fn swap_ready_cell(
        &self,
    ) -> (
        Arc<OnceCell<Result<Sender<ChildCall>, String>>>,
        Sender<ChildCall>,
        Receiver<ChildCall>,
    ) {
        let mut inner = self.inner.lock().await;
        if let Some(task) = inner.task.take() {
            task.cancel().await;
        }
        let _ = inner
            .ready
            .set(Err(
                "water mcp: the app was restarted while it was still building; retry the call"
                    .to_owned(),
            ))
            .await;
        let cell = Arc::new(OnceCell::new());
        inner.ready = cell.clone();
        drop(inner);
        let (calls, rx) = async_channel::unbounded();
        (cell, calls, rx)
    }

    /// Waits for the current readiness cell and clones the live call channel.
    async fn child_calls(&self) -> Result<Sender<ChildCall>, ToolResult> {
        let cell = self.inner.lock().await.ready.clone();
        match cell.wait().await {
            Ok(calls) => Ok(calls.clone()),
            Err(message) => Err(ToolResult::error(message.clone())),
        }
    }

    /// Forwards one tool call to the child.
    async fn forward_call(&self, name: &'static str, args: impl Serialize) -> ToolResult {
        let arguments = match serde_json::to_value(args) {
            Ok(arguments) => arguments,
            Err(error) => {
                return ToolResult::error(format!(
                    "water mcp: failed to serialize `{name}` arguments: {error}"
                ));
            }
        };
        let calls = match self.child_calls().await {
            Ok(calls) => calls,
            Err(result) => return result,
        };
        let (reply, replies) = async_channel::bounded(1);
        if calls
            .send(ChildCall {
                name,
                arguments,
                reply,
            })
            .await
            .is_err()
        {
            return ToolResult::error("water mcp: the app process exited before the call");
        }
        replies
            .recv()
            .await
            .unwrap_or_else(|_| ToolResult::error("water mcp: the app process dropped the call"))
    }

    /// Kills the child and any in-flight build. Called when the MCP session
    /// ends or the process is shutting down.
    pub async fn shutdown(&self) {
        let _serialized = self.rebuild_lock.lock().await;
        let task = self.inner.lock().await.task.take();
        if let Some(task) = task {
            task.cancel().await;
        }
    }
}

impl ToolDispatch for ChildProxy {
    fn snapshot(&self, args: SnapshotArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("snapshot", args)
    }

    fn find(&self, args: FindArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("find", args)
    }

    fn act(&self, args: ActArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("act", args)
    }

    fn pointer(&self, args: PointerArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("pointer", args)
    }

    fn key(&self, args: KeyArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("key", args)
    }

    fn type_text(&self, args: TypeTextArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("type_text", args)
    }

    fn wait(&self, args: WaitArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("wait", args)
    }

    fn screenshot(&self, args: ScreenshotArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("screenshot", args)
    }

    /// `restart` is intercepted rather than forwarded: the child is killed,
    /// rebuilt from the current sources — picking up edits — respawned, and
    /// the fresh tree comes back through the new child's `snapshot`.
    async fn restart(&self, _args: RestartArgs) -> ToolResult {
        self.rebuild().await;
        self.forward_call("snapshot", SnapshotArgs::default()).await
    }

    fn advance(&self, args: AdvanceArgs) -> impl Future<Output = ToolResult> + Send {
        self.forward_call("advance", args)
    }
}

/// The spawned task's whole lifetime: build, spawn, handshake, publish the
/// call channel, then drive calls until every sender is gone (session
/// shutdown) or the task is cancelled (restart / teardown).
async fn build_and_drive(
    config: ChildConfig,
    cell: Arc<OnceCell<Result<Sender<ChildCall>, String>>>,
    calls: Sender<ChildCall>,
    rx: Receiver<ChildCall>,
) {
    match build_and_spawn(&config).await {
        Ok((transport, child)) => {
            info!("water mcp: app is up, forwarding tool calls");
            cell.set(Ok(calls))
                .await
                .expect("a fresh readiness cell is unset");
            drive_child(transport, child, rx).await;
        }
        Err(build_error) => {
            error!(%build_error, "water mcp: failed to launch the app");
            cell.set(Err(format!("{build_error:#}")))
                .await
                .expect("a fresh readiness cell is unset");
        }
    }
}

/// Builds the generated backend in MCP mode, spawns the binary with its run
/// config, and performs the MCP handshake.
async fn build_and_spawn(config: &ChildConfig) -> Result<(ChildTransport, Child)> {
    let platform = host_platform();
    let project = ensure_hydrolysis_backend_ready(&config.project_path).await?;
    stage_hydrolysis_resources(
        &project,
        HydrolysisPreviewTheme::Material3,
        config.sccache_path.as_deref(),
    )
    .await?;

    let mut build_options = BuildOptions::development(false);
    if let Some(sccache_path) = &config.sccache_path {
        build_options = build_options.with_sccache(sccache_path.clone());
    }
    build_hydrolysis_with_envs_and_features(
        &project,
        platform,
        build_options,
        &[],
        &[HYDROLYSIS_MCP_FEATURE],
    )
    .await?;

    let binary_path =
        built_hydrolysis_binary_path(&project, platform, "debug", RustLinkage::SharedRuntime)
            .await?;
    stage_hydrolysis_shared_runtime(&binary_path, platform).await?;

    let run_config = McpRunConfig {
        width: config.width,
        height: config.height,
        scale_factor: config.scale_factor,
    };
    let config_path = write_run_config(&project, &run_config).await?;
    let backend_path = project.backend_path::<HydrolysisBackend>();

    // stdout is the MCP link — never inherit it; stderr flows straight to the
    // parent's stderr so app logs and build diagnostics stay visible.
    let mut command = smol::process::Command::new(&binary_path);
    command
        .kill_on_drop(true)
        .current_dir(&backend_path)
        .env(MCP_RUN_CONFIG_ENV, &config_path)
        .env("WATERUI_PROJECT_DIR", project.root())
        .env("WATERUI_APP_NAME", &project.manifest().package.name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command.spawn().wrap_err_with(|| {
        format!(
            "failed to spawn the MCP app binary {}",
            binary_path.display()
        )
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| eyre!("app binary spawned without a piped stdout"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| eyre!("app binary spawned without a piped stdin"))?;
    let mut transport = StreamTransport::new(BufReader::new(stdout), stdin);
    handshake(&mut transport).await?;
    Ok((transport, child))
}

/// The child handshake: `initialize` then `notifications/initialized`, the
/// same exchange the front just performed with its own client.
async fn handshake(transport: &mut ChildTransport) -> Result<()> {
    let response = transport
        .request(JsonRpcRequest::with_params(
            0_i64,
            "initialize",
            InitializeParams::default(),
        ))
        .await
        .wrap_err("the app binary did not answer `initialize`")?;
    if response.error.is_some() || response.result.is_none() {
        bail!("the app binary refused `initialize`: {response:?}");
    }
    transport
        .notify(JsonRpcNotification::new("notifications/initialized"))
        .await
        .wrap_err("failed to send `notifications/initialized` to the app")?;
    Ok(())
}

/// Reads `tools/call` requests off the channel, forwards each to the child,
/// and replies with the mapped [`ToolResult`]. When the channel closes — every
/// sender dropped on session shutdown — the child is killed and reaped.
async fn drive_child(mut transport: ChildTransport, mut child: Child, calls: Receiver<ChildCall>) {
    while let Ok(call) = calls.recv().await {
        let request = JsonRpcRequest::with_params(
            0_i64,
            "tools/call",
            CallToolParams {
                name: call.name.to_owned(),
                arguments: call.arguments,
            },
        );
        let result = match transport.request(request).await {
            Ok(response) => map_tool_call_response(response),
            Err(error) => ToolResult::error(format!("water mcp: app transport failed: {error}")),
        };
        debug!(tool = call.name, "forwarded tools/call to the app");
        // A caller cancelled during shutdown leaves no receiver — the result
        // is simply dropped.
        let _ = call.reply.try_send(result);
    }
    // Kill rather than wait on stdin EOF: the app should already be exiting,
    // but a wedged app must not outlive the session.
    let _ = child.kill();
    let _ = child.status().await;
}

/// Maps a child `tools/call` JSON-RPC response to a [`ToolResult`].
fn map_tool_call_response(response: JsonRpcResponse) -> ToolResult {
    if let Some(error) = response.error {
        return ToolResult::error(format!(
            "water mcp: the app reported error {}: {}",
            error.code, error.message
        ));
    }
    let Some(result) = response.result else {
        return ToolResult::error("water mcp: the app returned an empty `tools/call` response");
    };
    match serde_json::from_value::<CallToolResult>(result) {
        Ok(result) => map_call_tool_result(&result),
        Err(error) => {
            ToolResult::error(format!("water mcp: malformed `tools/call` result: {error}"))
        }
    }
}

/// Maps one MCP [`CallToolResult`] onto the in-process [`ToolResult`] the
/// fronting server's tool returns.
fn map_call_tool_result(result: &CallToolResult) -> ToolResult {
    if result.is_error {
        let message = result
            .content
            .iter()
            .filter_map(|content| match content {
                Content::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return ToolResult::error(if message.is_empty() {
            "water mcp: the app reported a tool error".to_owned()
        } else {
            message
        });
    }
    match result.content.as_slice() {
        [Content::Text(text)] => ToolResult::text(text.text.clone()),
        [Content::Image(image)] => match BASE64.decode(&image.data) {
            Ok(bytes) => ToolResult::image(bytes, &image.mime_type),
            Err(error) => ToolResult::error(format!(
                "water mcp: the app returned malformed base64 image data: {error}"
            )),
        },
        content => {
            let kinds = content
                .iter()
                .map(|content| match content {
                    Content::Text(_) => "text",
                    Content::Image(_) => "image",
                    Content::Resource(_) => "resource",
                })
                .collect::<Vec<_>>()
                .join(", ");
            ToolResult::error(format!(
                "water mcp: the app returned unsupported tool content [{kinds}]"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ChildProxy, map_call_tool_result, map_tool_call_response};
    use aither_mcp::protocol::{
        CallToolResult, Content, ImageContent, JsonRpcResponse, RequestId, TextContent,
    };
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    fn text_result(text: &str) -> CallToolResult {
        CallToolResult {
            content: vec![Content::Text(TextContent {
                text: text.to_owned(),
                annotations: None,
            })],
            is_error: false,
        }
    }

    fn json_response(result: CallToolResult) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id: RequestId::Number(0),
            result: Some(serde_json::to_value(result).expect("result serializes")),
            error: None,
        }
    }

    #[test]
    fn text_content_maps_to_text_result() {
        let mapped = map_tool_call_response(json_response(text_result("the tree")));
        assert_eq!(mapped.as_text(), Some("the tree"));
        assert!(mapped.error_message().is_none());
    }

    #[test]
    fn error_flag_maps_to_error_result() {
        let mut result = text_result("no nodes matched");
        result.is_error = true;
        let mapped = map_tool_call_response(json_response(result));
        assert_eq!(mapped.error_message(), Some("no nodes matched"));
    }

    #[test]
    fn image_content_maps_to_image_result() {
        let pixels = [0x89, b'P', b'N', b'G', 1, 2, 3];
        let result = CallToolResult {
            content: vec![Content::Image(ImageContent {
                data: BASE64.encode(pixels),
                mime_type: "image/png".to_owned(),
                annotations: None,
            })],
            is_error: false,
        };
        let mapped = map_tool_call_response(json_response(result));
        assert_eq!(mapped.content(), Some(pixels.as_slice()));
        assert_eq!(
            mapped.mime().map(|mime| mime.essence_str().to_owned()),
            Some("image/png".to_owned())
        );
    }

    #[test]
    fn malformed_base64_image_is_an_error() {
        let result = CallToolResult {
            content: vec![Content::Image(ImageContent {
                data: "not base64!!!".to_owned(),
                mime_type: "image/png".to_owned(),
                annotations: None,
            })],
            is_error: false,
        };
        let mapped = map_call_tool_result(&result);
        assert!(
            mapped
                .error_message()
                .is_some_and(|message| message.contains("base64"))
        );
    }

    #[test]
    fn rebuild_resolves_waiters_on_the_previous_readiness_cell() {
        smol::block_on(async {
            let proxy = ChildProxy::new(
                PathBuf::from("/definitely/not/a/project"),
                390,
                844,
                2.0,
                None,
            );
            // A caller that arrived before the restart is parked on this cell.
            let parked = proxy.inner.lock().await.ready.clone();
            proxy.rebuild().await;
            let result = parked.wait().await;
            assert!(
                matches!(result, Err(message) if message.contains("restarted")),
                "a caller parked during the build should get a restart error, got {result:?}"
            );
            proxy.shutdown().await;
        });
    }

    #[test]
    fn multi_item_content_is_an_error() {
        let result = CallToolResult {
            content: vec![
                Content::Text(TextContent {
                    text: "one".to_owned(),
                    annotations: None,
                }),
                Content::Text(TextContent {
                    text: "two".to_owned(),
                    annotations: None,
                }),
            ],
            is_error: false,
        };
        let mapped = map_call_tool_result(&result);
        assert!(
            mapped
                .error_message()
                .is_some_and(|message| message.contains("text, text"))
        );
    }
}
