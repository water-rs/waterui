//! Preview view component.
//!
//! The main `Preview` view that handles dylib loading and view rendering.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use async_io::Timer;
use futures_lite::future;
use waterui_core::view_renderer::{RenderSize, ViewRenderer};
use waterui_core::{Environment, View};

use crate::library::PreviewLibrary;
use crate::protocol::{DylibSource, PreviewError, PreviewOutput, PreviewRequest, PreviewResponse, Size};
use crate::renderer::RenderResultExt;
use crate::tcp::{RequestHandler, TcpServer};

/// Wrapper for ViewRenderer pointer that implements Send+Sync.
///
/// This is safe because:
/// 1. The ViewRenderer is only read, never mutated
/// 2. There's only one Preview instance per app
/// 3. The Environment (which owns the ViewRenderer) lives for the app lifetime
struct ViewRendererPtr(*const ViewRenderer);

// SAFETY: ViewRenderer is accessed read-only from a single-threaded executor
unsafe impl Send for ViewRendererPtr {}
unsafe impl Sync for ViewRendererPtr {}

/// Global storage for the ViewRenderer.
///
/// This is necessary because the TCP handler runs asynchronously and needs
/// access to the ViewRenderer installed in the Environment. We store it
/// globally since there's only one preview instance per app.
static VIEW_RENDERER: OnceLock<ViewRendererPtr> = OnceLock::new();

/// Global flag to track if TCP server has been started.
static TCP_SERVER_STARTED: OnceLock<u16> = OnceLock::new();

/// Get the global ViewRenderer, if installed.
fn get_view_renderer() -> Option<&'static ViewRenderer> {
    VIEW_RENDERER.get().map(|wrapper| unsafe { &*wrapper.0 })
}

/// State shared between the preview view and the request handler.
struct PreviewState {
    /// Currently loaded library.
    library: Option<PreviewLibrary>,
}

impl PreviewState {
    const fn new() -> Self {
        Self { library: None }
    }
}

/// The main preview view - handles dylib loading and view rendering.
///
/// This view starts a TCP server to receive commands from the CLI,
/// loads dylibs, and renders views.
#[derive(Debug, Default)]
pub struct Preview;

impl Preview {
    /// Create a new preview view.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl View for Preview {
    fn body(self, env: &Environment) -> impl View {
        use waterui_layout::spacer::Spacer;

        tracing::info!("Preview::body called");

        // Install the ViewRenderer globally so the TCP handler can access it.
        // This is safe because:
        // 1. The Environment lives for the lifetime of the app
        // 2. There's only one Preview instance per app
        // 3. The ViewRenderer is only read, never mutated
        if let Some(renderer) = env.get::<ViewRenderer>() {
            let _ = VIEW_RENDERER.set(ViewRendererPtr(renderer as *const ViewRenderer));
            tracing::info!("ViewRenderer installed for preview");
        } else {
            tracing::warn!("No ViewRenderer found in environment - preview rendering will fail");
        }

        // Start TCP server (only once)
        let _ = TCP_SERVER_STARTED.get_or_init(|| {
            tracing::info!("Starting TCP server");
            let state = Rc::new(RefCell::new(PreviewState::new()));

            // Create async request handler that captures state
            let handler: RequestHandler = Rc::new({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    Box::pin(async move { handle_request(&state, request).await })
                }
            });

            let server = TcpServer::bind(handler);
            let port = server.port();

            tracing::info!("Preview daemon started on port {port}");
            port
        });

        // Return a simple spacer view
        Spacer::new(0.0)
    }
}

/// Handle a preview request from the CLI.
async fn handle_request(state: &RefCell<PreviewState>, request: PreviewRequest) -> PreviewResponse {
    match request {
        PreviewRequest::Render { dylib, symbol, frame } => {
            handle_render(state, dylib, &symbol, frame).await
        }
        PreviewRequest::Shutdown => {
            tracing::info!("Received shutdown request");
            // TODO: Actually shut down the app
            Ok(PreviewOutput {
                png_data: Vec::new(),
            })
        }
    }
}

/// Handle a render request.
async fn handle_render(
    state: &RefCell<PreviewState>,
    dylib: DylibSource,
    symbol: &str,
    frame: Size,
) -> PreviewResponse {
    // Load dylib in background thread to avoid blocking main thread
    let symbol = symbol.to_string();
    let load_result = match dylib {
        DylibSource::Bytes(data) => {
            tracing::info!("Loading new dylib ({} bytes) in background", data.len());

            // Spawn blocking work in background thread
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                #[cfg(unix)]
                let result = unsafe { PreviewLibrary::load_from_bytes(&data) }
                    .map_err(|e| PreviewError::DylibLoad(e.to_string()));
                #[cfg(not(unix))]
                let result: Result<PreviewLibrary, PreviewError> = Err(PreviewError::DylibLoad(
                    "Library loading not supported on this platform".to_string(),
                ));
                let _ = tx.send(result);
            });

            // Wait for result (this yields to executor while waiting)
            loop {
                match rx.try_recv() {
                    Ok(result) => break Some(result),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // Yield to allow other tasks to run
                        futures_lite::future::yield_now().await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break Some(Err(PreviewError::DylibLoad("Thread panicked".to_string())));
                    }
                }
            }
        }
        DylibSource::Cached => None,
    };

    // Update state with loaded library
    {
        let mut state_guard = state.borrow_mut();
        if let Some(result) = load_result {
            state_guard.library = Some(result?);
        } else if state_guard.library.is_none() {
            return Err(PreviewError::NoCachedDylib);
        }
    }

    // Get the library and load the view
    let view = {
        let state_guard = state.borrow();
        let library = state_guard.library.as_ref().expect("Library should be loaded");

        // Check if symbol exists
        if !library.has_symbol(&symbol) {
            return Err(PreviewError::SymbolNotFound(symbol.clone()));
        }

        // Load the view from the symbol
        unsafe { library.load_view(&symbol) }
            .map_err(|e| PreviewError::RenderFailed(e.to_string()))?
    };

    tracing::info!("Rendering view: {symbol} at {}x{}", frame.width, frame.height);

    // Get the ViewRenderer
    let renderer = get_view_renderer()
        .ok_or_else(|| PreviewError::RenderFailed("ViewRenderer not installed".to_string()))?;

    // Render the view to RGBA asynchronously on main thread
    let render_size = RenderSize::new(frame.width, frame.height);
    let timeout_ms = std::env::var("WATERUI_PREVIEW_RENDER_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30_000);

    let render_future = async { Ok(renderer.render(view, render_size).await) };
    let timeout_future = async {
        Timer::after(Duration::from_millis(timeout_ms)).await;
        Err(PreviewError::RenderFailed(format!(
            "Render timed out after {timeout_ms}ms"
        )))
    };

    let result = future::race(render_future, timeout_future).await?;

    // Convert RGBA to PNG
    let png_data = result.to_png();

    tracing::info!(
        "Rendered {}x{} image ({} bytes PNG)",
        result.width,
        result.height,
        png_data.len()
    );

    Ok(PreviewOutput { png_data })
}
