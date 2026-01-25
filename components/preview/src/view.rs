//! Preview view component.
//!
//! The main `Preview` view that handles dylib loading and view rendering.
//!
//! Connections are processed sequentially to prevent concurrency issues with
//! the ViewRenderer and dispatch queue integration on macOS.

use std::sync::OnceLock;
use std::time::Duration;

use async_io::Async;
use executor_core::spawn_local;
use futures_lite::io::{AsyncReadExt, AsyncWriteExt};
use waterui_core::view_renderer::{RenderSize, ViewRenderer};
use waterui_core::{Environment, View};

use crate::library::PreviewLibrary;
use crate::protocol::{
    DylibSource, PreviewError, PreviewOutput, PreviewRequest, PreviewResponse, Size,
};
use crate::renderer::RenderResultExt;

/// Starting port for preview daemon.
const PORT_START: u16 = 2106;
/// Number of ports to try before giving up.
const PORT_RANGE: u16 = 50;

/// Wrapper for ViewRenderer pointer that implements Send+Sync.
struct ViewRendererPtr(*const ViewRenderer);

// SAFETY: ViewRenderer is only accessed from the main thread via spawn_local.
// All spawn_local tasks run on the same thread where the pointer was captured.
unsafe impl Send for ViewRendererPtr {}
unsafe impl Sync for ViewRendererPtr {}

/// Global storage for the ViewRenderer pointer.
static VIEW_RENDERER: OnceLock<ViewRendererPtr> = OnceLock::new();

/// Flag to track if the server is already started.
static SERVER_STARTED: OnceLock<()> = OnceLock::new();

/// Get the global ViewRenderer, if installed.
fn get_view_renderer() -> Option<&'static ViewRenderer> {
    VIEW_RENDERER.get().map(|wrapper| unsafe { &*wrapper.0 })
}

/// The main preview view - handles dylib loading and view rendering.
#[derive(Debug, Default)]
pub struct Preview;

impl Preview {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl View for Preview {
    fn body(self, env: &Environment) -> impl View {
        use waterui_layout::spacer::Spacer;

        tracing::info!("Preview::body called");

        // Install ViewRenderer globally (only once)
        if VIEW_RENDERER.get().is_none() {
            if let Some(renderer) = env.get::<ViewRenderer>() {
                let _ = VIEW_RENDERER.set(ViewRendererPtr(renderer as *const ViewRenderer));
                tracing::info!("ViewRenderer installed for preview");
            } else {
                tracing::warn!("No ViewRenderer found in environment");
            }
        }

        // Start TCP server (only once) using spawn_local
        if SERVER_STARTED.set(()).is_ok() {
            spawn_local(async {
                if let Err(e) = run_tcp_server().await {
                    tracing::error!("TCP server error: {e}");
                }
            })
            .detach();
        }

        Spacer::new(0.0)
    }
}

/// Shared state for preview handling across all connections.
struct ServerState {
    library: Option<PreviewLibrary>,
}

impl ServerState {
    const fn new() -> Self {
        Self { library: None }
    }
}

/// Run the TCP server accepting connections.
///
/// Connections are processed sequentially (one at a time) to avoid concurrency
/// issues with the ViewRenderer on macOS, where the DispatchQueue.main integration
/// requires serialized access.
async fn run_tcp_server() -> std::io::Result<()> {
    // Find an available port
    let listener = (PORT_START..(PORT_START + PORT_RANGE))
        .find_map(|port| {
            std::net::TcpListener::bind(("127.0.0.1", port))
                .ok()
                .map(|l| {
                    tracing::info!("Preview daemon listening on port {port}");
                    l
                })
        })
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "Failed to bind to any port in range",
            )
        })?;

    // Set a short accept timeout so we can process connections without blocking forever
    listener.set_nonblocking(true)?;
    let listener = Async::new(listener)?;

    // Shared state across all connections (library is cached)
    let mut state = ServerState::new();

    let mut connection_count = 0;
    loop {
        tracing::info!("Waiting to accept connection {connection_count}...");

        // Accept the next connection
        let (stream, addr) = listener.accept().await?;
        tracing::info!("Accepted connection {connection_count} from {addr}");

        // Handle this connection completely before accepting the next one
        // This serializes all requests to avoid concurrency issues
        if let Err(e) = handle_connection(&mut state, stream).await {
            tracing::error!("Connection {connection_count} error from {addr}: {e}");
        }

        tracing::info!("Finished handling connection {connection_count}");
        connection_count += 1;
    }
}

/// Handle a single TCP connection (supports multiple requests).
async fn handle_connection(
    state: &mut ServerState,
    stream: Async<std::net::TcpStream>,
) -> std::io::Result<()> {
    tracing::info!("Starting to handle connection");

    let mut reader = futures_lite::io::BufReader::new(&stream);
    let mut writer = &stream;

    let mut request_count = 0;
    loop {
        tracing::info!("Waiting for request (count={request_count})...");

        // Read length-prefixed message
        let mut len_buf = [0u8; 4];
        match reader.read_exact(&mut len_buf).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                tracing::info!("Client disconnected after {request_count} requests");
                return Ok(());
            }
            Err(e) => {
                tracing::error!("Read error: {e}");
                return Err(e);
            }
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        tracing::info!("Reading {len} bytes for request {request_count}");

        // Read message body
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await?;
        tracing::debug!("Read complete, parsing request");

        // Deserialize request
        let request: PreviewRequest = serde_json::from_slice(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        tracing::debug!("Received request: {request:?}");

        let should_shutdown = matches!(request, PreviewRequest::Shutdown);

        // Handle the request
        let response = handle_request(state, request).await;
        tracing::debug!("Request handled, sending response");

        // Serialize and send response
        let response_buf = serde_json::to_vec(&response)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        writer
            .write_all(&(response_buf.len() as u32).to_be_bytes())
            .await?;
        writer.write_all(&response_buf).await?;
        writer.flush().await?;

        tracing::info!(
            "Sent response ({} bytes) for request {request_count}",
            response_buf.len()
        );

        request_count += 1;

        if should_shutdown {
            tracing::info!("Shutdown requested, exiting preview app");
            std::process::exit(0);
        }
    }
}

/// Handle a preview request.
async fn handle_request(state: &mut ServerState, request: PreviewRequest) -> PreviewResponse {
    match request {
        PreviewRequest::Render {
            dylib,
            symbol,
            frame,
        } => handle_render(state, dylib, &symbol, frame).await,
        PreviewRequest::Shutdown => {
            tracing::info!("Received shutdown request");
            // Exit after sending response
            spawn_local(async {
                async_io::Timer::after(Duration::from_millis(100)).await;
                std::process::exit(0);
            })
            .detach();
            Ok(PreviewOutput {
                png_data: Vec::new(),
            })
        }
    }
}

/// Handle a render request.
async fn handle_render(
    state: &mut ServerState,
    dylib: DylibSource,
    symbol: &str,
    frame: Size,
) -> PreviewResponse {
    // Load dylib if provided
    let symbol = symbol.to_string();
    match dylib {
        DylibSource::Bytes(data) => {
            tracing::info!("Loading new dylib ({} bytes)", data.len());
            #[cfg(unix)]
            {
                let library = unsafe { PreviewLibrary::load_from_bytes(&data) }
                    .map_err(|e| PreviewError::DylibLoad(e.to_string()))?;
                state.library = Some(library);
            }
            #[cfg(not(unix))]
            {
                return Err(PreviewError::DylibLoad(
                    "Library loading not supported on this platform".to_string(),
                ));
            }
        }
        DylibSource::Cached => {
            if state.library.is_none() {
                return Err(PreviewError::NoCachedDylib);
            }
        }
    }

    // Get view from library
    let library = state.library.as_ref().expect("Library should be loaded");

    if !library.has_symbol(&symbol) {
        return Err(PreviewError::SymbolNotFound(symbol.clone()));
    }

    let view = unsafe { library.load_view(&symbol) }
        .map_err(|e| PreviewError::RenderFailed(e.to_string()))?;

    tracing::info!(
        "Rendering view: {symbol} at {}x{}",
        frame.width,
        frame.height
    );

    // Render the view directly
    let renderer = get_view_renderer()
        .ok_or_else(|| PreviewError::RenderFailed("ViewRenderer not installed".to_string()))?;

    let render_size = RenderSize::new(frame.width, frame.height);
    let result = renderer.render(view, render_size).await;

    let png_data = result.to_png();
    tracing::info!(
        "Rendered {}x{} ({} bytes PNG)",
        result.width,
        result.height,
        png_data.len()
    );

    Ok(PreviewOutput { png_data })
}
