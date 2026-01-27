//! Preview view component for the preview support app.
//!
//! This view starts a TCP server (localhost) and serializes all render work through a single
//! worker to satisfy platform constraints (e.g. macOS main-thread renderer requirements),
//! while still allowing multiple CLI clients to connect concurrently.

use std::io;
use std::num::NonZeroUsize;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use async_io::Async;
use executor_core::spawn_local;
use futures_lite::io::BufReader;
use waterui_core::view_renderer::{RenderSize, ViewRenderer};
use waterui_core::{Environment, Metadata, Retain, View};

use crate::library::PreviewLibrary;
use crate::renderer::RenderResultExt as _;
use waterui_preview_protocol::tcp::PreviewTcpConfig;
use waterui_preview_protocol::transport::{read_json_frame, write_json_frame};
use waterui_preview_protocol::{
    DylibId, DylibSource, PreviewError, PreviewOutput, PreviewRequest, PreviewResponse, Size,
};

/// The main preview view - hosts the TCP server for CLI preview requests.
#[derive(Debug, Default)]
pub struct Preview;

impl Preview {
    #[must_use]
    /// Create the preview server view.
    pub const fn new() -> Self {
        Self
    }
}

impl View for Preview {
    fn body(self, env: &Environment) -> impl View {
        use waterui_layout::spacer::Spacer;

        let env = env.clone();
        let server = PreviewServer::start(env);

        Metadata::new(Spacer::new(0.0), Retain::new(server))
    }
}

#[derive(Debug)]
struct PreviewServer {
    _task: executor_core::AnyLocalExecutorTask<()>,
}

impl PreviewServer {
    fn start(env: Environment) -> Self {
        let task = spawn_local(async move {
            if let Err(e) = run_tcp_server(env).await {
                tracing::error!("Preview TCP server stopped: {e}");
            }
        });

        Self { _task: task }
    }
}

#[derive(Debug)]
struct WorkerMessage {
    request: PreviewRequest,
    respond_to: Sender<PreviewResponse>,
}

async fn run_tcp_server(env: Environment) -> io::Result<()> {
    env.get::<ViewRenderer>()
        .expect("Preview support app must provide a ViewRenderer in Environment");

    let config =
        PreviewTcpConfig::from_env().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let listener = bind_first_available(config)?;
    #[cfg(target_os = "macos")]
    write_preview_pid_file(listener.local_addr()?.port()).await;
    tracing::info!(
        "Preview daemon listening on {}:{}",
        config.host,
        listener.local_addr()?.port()
    );

    let listener = Async::new(listener)?;

    let (worker_tx, worker_rx) = async_channel::unbounded::<WorkerMessage>();

    let _worker_task = spawn_local(render_worker(env, worker_rx));

    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::debug!("Preview client connected: {addr}");

        let worker_tx = worker_tx.clone();
        spawn_local(async move {
            if let Err(e) = handle_connection(stream, worker_tx).await {
                tracing::warn!("Preview client error ({addr}): {e}");
            }
        })
        .detach();
    }
}

#[cfg(target_os = "macos")]
async fn write_preview_pid_file(port: u16) {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return;
    };

    let pid_path = home
        .join(".water")
        .join("preview_support")
        .join(".water")
        .join("preview.pid");

    if let Some(parent) = pid_path.parent() {
        let _ = async_fs::create_dir_all(parent).await;
    }

    let pid = std::process::id();
    let content = format!("{pid}\n{port}\n");
    let _ = async_fs::write(pid_path, content).await;
}

fn bind_first_available(config: PreviewTcpConfig) -> io::Result<std::net::TcpListener> {
    for port in config.ports() {
        let addr = std::net::SocketAddr::new(config.host, port);
        match std::net::TcpListener::bind(addr) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                return Ok(listener);
            }
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => continue,
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("failed to bind preview TCP server on {addr}: {e}"),
                ));
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "failed to bind preview TCP server",
    ))
}

async fn handle_connection(
    stream: Async<std::net::TcpStream>,
    worker_tx: Sender<WorkerMessage>,
) -> io::Result<()> {
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;

    loop {
        let request = match read_json_frame::<_, PreviewRequest>(&mut reader).await {
            Ok(req) => req,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };

        let should_shutdown = matches!(request, PreviewRequest::Shutdown);

        let (resp_tx, resp_rx) = async_channel::bounded::<PreviewResponse>(1);
        worker_tx
            .send(WorkerMessage {
                request,
                respond_to: resp_tx,
            })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "preview worker exited"))?;

        let response = resp_rx
            .recv()
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "preview worker exited"))?;

        write_json_frame(&mut writer, &response).await?;

        if should_shutdown {
            spawn_local(async {
                async_io::Timer::after(Duration::from_millis(50)).await;
                std::process::exit(0);
            })
            .detach();
            return Ok(());
        }
    }
}

struct DylibCache {
    capacity: NonZeroUsize,
    libraries: indexmap::IndexMap<DylibId, PreviewLibrary>,
}

impl DylibCache {
    fn new() -> Self {
        let capacity = dylib_cache_capacity();
        Self {
            capacity,
            libraries: indexmap::IndexMap::new(),
        }
    }

    fn contains(&self, id: &DylibId) -> bool {
        self.libraries.contains_key(id) || dylib_cache_path(*id).exists()
    }

    fn get(&mut self, id: &DylibId) -> Option<&PreviewLibrary> {
        self.touch(id);
        self.libraries.get(id)
    }

    fn insert(&mut self, id: DylibId, library: PreviewLibrary) {
        if self.libraries.contains_key(&id) {
            self.libraries.insert(id, library);
            self.touch(&id);
            return;
        }

        if self.libraries.len() == self.capacity.get() {
            let _ = self.libraries.shift_remove_index(0);
        }
        self.libraries.insert(id, library);
    }

    fn touch(&mut self, id: &DylibId) {
        let Some(index) = self.libraries.get_index_of(id) else {
            return;
        };
        let last = self.libraries.len().saturating_sub(1);
        if index != last {
            self.libraries.move_index(index, last);
        }
    }

    async fn ensure_loaded(&mut self, id: DylibId) -> Result<(), PreviewError> {
        if self.libraries.contains_key(&id) {
            self.touch(&id);
            return Ok(());
        }

        let path = dylib_cache_path(id);
        if !path.exists() {
            return Err(PreviewError::UnknownDylibId(id));
        }

        let library = unsafe { PreviewLibrary::load_from_path(&path) }
            .await
            .map_err(|e| PreviewError::DylibLoad(e.to_string()))?;
        self.insert(id, library);
        Ok(())
    }
}

fn dylib_cache_capacity() -> NonZeroUsize {
    const DEFAULT: usize = 8;
    std::env::var("WATERUI_PREVIEW_DYLIB_CACHE_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .and_then(NonZeroUsize::new)
        .unwrap_or_else(|| NonZeroUsize::new(DEFAULT).expect("DEFAULT is non-zero"))
}

fn dylib_cache_path(id: DylibId) -> std::path::PathBuf {
    let hex = id.to_string();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let base = home
        .unwrap_or_else(|| std::env::temp_dir())
        .join(".water")
        .join("cache")
        .join("preview")
        .join("dylibs");
    base.join(format!("{hex}.dylib"))
}

async fn render_worker(env: Environment, worker_rx: Receiver<WorkerMessage>) {
    let mut cache = DylibCache::new();

    while let Ok(msg) = worker_rx.recv().await {
        let response = handle_request(&env, &mut cache, msg.request).await;
        let _ = msg.respond_to.send(response).await;
    }
}

async fn handle_request(
    env: &Environment,
    cache: &mut DylibCache,
    request: PreviewRequest,
) -> PreviewResponse {
    match request {
        PreviewRequest::HasDylib { id } => PreviewResponse::HasDylib {
            present: cache.contains(&id),
        },
        PreviewRequest::Render {
            dylib,
            symbol,
            frame,
        } => PreviewResponse::Render {
            result: handle_render(env, cache, dylib, &symbol, frame).await,
        },
        PreviewRequest::Shutdown => PreviewResponse::Shutdown,
    }
}

async fn handle_render(
    env: &Environment,
    cache: &mut DylibCache,
    dylib: DylibSource,
    symbol: &str,
    frame: Size,
) -> Result<PreviewOutput, PreviewError> {
    let id = match dylib {
        DylibSource::Bytes { id, bytes } => {
            if !cache.contains(&id) {
                #[cfg(unix)]
                {
                    let library = unsafe { PreviewLibrary::load_from_bytes(id, &bytes) }
                        .await
                        .map_err(|e| PreviewError::DylibLoad(e.to_string()))?;
                    cache.insert(id, library);
                }
                #[cfg(not(unix))]
                {
                    let _ = bytes;
                    return Err(PreviewError::DylibLoad(
                        "library loading not supported on this platform".to_string(),
                    ));
                }
            }
            id
        }
        DylibSource::Cached { id } => id,
    };

    let view = {
        if let Err(e) = cache.ensure_loaded(id).await {
            return Err(e);
        }
        let library = cache.get(&id).ok_or(PreviewError::UnknownDylibId(id))?;

        if !library.has_symbol(symbol) {
            return Err(PreviewError::SymbolNotFound(symbol.to_string()));
        }

        unsafe { library.load_view(symbol) }
            .map_err(|e| PreviewError::RenderFailed(e.to_string()))?
    };

    let renderer = env
        .get::<ViewRenderer>()
        .expect("ViewRenderer missing in Environment");
    let render_size = RenderSize::new(frame.width, frame.height);

    let result = renderer.render(view, render_size).await;
    let png_data = result.into_png();

    Ok(PreviewOutput { png_data })
}
