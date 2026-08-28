//! Preview view component for the preview support app.
//!
//! This view starts a TCP server (localhost) and serializes all render work through a single
//! worker to satisfy platform constraints (e.g. macOS main-thread renderer requirements),
//! while still allowing multiple CLI clients to connect concurrently.

use std::cell::Cell;
use std::collections::HashSet;
use std::io;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use async_io::Async;
use executor_core::spawn_local;
use futures_lite::io::BufReader;
use waterui_core::view_renderer::{RenderSize, ViewRenderer};
use waterui_core::{Environment, Metadata, Retain, View};

use crate::library::{LoadError, PreviewLibrary};
use crate::renderer::RenderResultExt as _;
use waterui_preview_protocol::registry::{
    PreviewAppInstance, preview_instance_registry_dir, preview_instance_registry_path,
};
use waterui_preview_protocol::tcp::PreviewTcpConfig;
use waterui_preview_protocol::transport::{read_frame, write_frame};
use waterui_preview_protocol::{
    DylibId, DylibSource, PreviewDylibLoadTimings, PreviewError, PreviewOutput,
    PreviewRenderTimings, PreviewRequest, PreviewResponse, Size, protocol_info,
};

use crate::cache::{preview_dylib_cache_dir, preview_dylib_cache_path};
/// The main preview view - hosts the TCP server for CLI preview requests.
#[derive(Debug)]
pub struct Preview {
    waterui_core_fingerprint: &'static str,
}

impl Preview {
    #[must_use]
    /// Create the preview server view.
    pub const fn new() -> Self {
        Self::with_runtime_fingerprint("unknown")
    }

    #[must_use]
    /// Create the preview server view with the runtime `waterui-core` fingerprint.
    pub const fn with_runtime_fingerprint(waterui_core_fingerprint: &'static str) -> Self {
        Self {
            waterui_core_fingerprint,
        }
    }
}

impl Default for Preview {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Preview {
    fn body(self, env: &Environment) -> impl View {
        use waterui_layout::spacer::Spacer;

        let env = env.clone();
        let server = PreviewServer::start(env, self.waterui_core_fingerprint);

        Metadata::new(Spacer::new(0.0), Retain::new(server))
    }
}

#[derive(Debug)]
struct PreviewServer {
    _task: executor_core::AnyLocalExecutorTask<()>,
}

impl PreviewServer {
    fn start(env: Environment, waterui_core_fingerprint: &'static str) -> Self {
        let waterui_core_fingerprint = waterui_core_fingerprint.to_string();
        let task = spawn_local(async move {
            if let Err(e) = run_tcp_server(env, waterui_core_fingerprint).await {
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

#[expect(
    clippy::future_not_send,
    reason = "preview runtime drives this on the main-thread support-app executor; `Environment` is `!Send` by design"
)]
async fn run_tcp_server(env: Environment, waterui_core_fingerprint: String) -> io::Result<()> {
    env.get::<ViewRenderer>()
        .expect("Preview support app must provide a ViewRenderer in Environment");

    let config = PreviewTcpConfig::from_env().map_err(io::Error::other)?;
    let listener = bind_first_available(config)?;
    let registration_path =
        register_preview_instance(listener.local_addr()?, &waterui_core_fingerprint).await?;
    tracing::info!(
        "Preview support app listening on {}:{}",
        config.host,
        listener.local_addr()?.port()
    );

    let listener = Async::new(listener)?;

    let (worker_tx, worker_rx) = async_channel::bounded::<WorkerMessage>(1);
    let (activity, activity_events) = PreviewActivity::new();

    let worker_fingerprint = waterui_core_fingerprint.clone();
    let _worker_task = spawn_local(render_worker(env, worker_rx, worker_fingerprint));
    spawn_local(idle_shutdown_task(
        activity.clone(),
        activity_events,
        registration_path.clone(),
    ))
    .detach();

    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::debug!("Preview client connected: {addr}");

        let worker_tx = worker_tx.clone();
        let activity = activity.clone();
        let registration_path = registration_path.clone();
        let waterui_core_fingerprint = waterui_core_fingerprint.clone();
        spawn_local(async move {
            if let Err(e) = handle_connection(
                stream,
                worker_tx,
                activity,
                registration_path,
                &waterui_core_fingerprint,
            )
            .await
            {
                tracing::warn!("Preview client error ({addr}): {e}");
            }
        })
        .detach();
    }
}

#[derive(Debug)]
struct PreviewActivity {
    active_connections: Cell<usize>,
    last_activity: Cell<Instant>,
    events: Sender<()>,
}

impl PreviewActivity {
    fn new() -> (Rc<Self>, Receiver<()>) {
        let (events, event_receiver) = async_channel::bounded(1);
        (
            Rc::new(Self {
                active_connections: Cell::new(0),
                last_activity: Cell::new(Instant::now()),
                events,
            }),
            event_receiver,
        )
    }

    fn begin_connection(&self) {
        self.touch();
        let active_connections = self
            .active_connections
            .get()
            .checked_add(1)
            .expect("preview active connection count overflowed");
        self.active_connections.set(active_connections);
    }

    fn end_connection(&self) {
        self.touch();
        let active_connections = self
            .active_connections
            .get()
            .checked_sub(1)
            .expect("preview active connection count underflowed");
        self.active_connections.set(active_connections);
    }

    fn touch(&self) {
        self.last_activity.set(Instant::now());
        match self.events.try_send(()) {
            Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
            Err(async_channel::TrySendError::Closed(())) => {
                panic!("preview activity task exited while the TCP server is still running")
            }
        }
    }

    const fn active_connections(&self) -> usize {
        self.active_connections.get()
    }

    fn idle_for(&self) -> Duration {
        self.last_activity.get().elapsed()
    }
}

#[derive(Debug)]
struct ActiveConnectionGuard {
    activity: Rc<PreviewActivity>,
}

impl ActiveConnectionGuard {
    fn new(activity: Rc<PreviewActivity>) -> Self {
        activity.begin_connection();
        Self { activity }
    }
}

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.activity.end_connection();
    }
}

fn preview_idle_shutdown_after() -> Duration {
    const DEFAULT_SECONDS: u64 = 900;
    let seconds = match std::env::var("WATERUI_PREVIEW_IDLE_SHUTDOWN_SECS") {
        Ok(value) => value.parse::<u64>().unwrap_or_else(|error| {
            panic!("invalid WATERUI_PREVIEW_IDLE_SHUTDOWN_SECS value `{value}`: {error}")
        }),
        Err(std::env::VarError::NotPresent) => DEFAULT_SECONDS,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("WATERUI_PREVIEW_IDLE_SHUTDOWN_SECS must be valid UTF-8")
        }
    };
    Duration::from_secs(seconds)
}

#[expect(
    clippy::future_not_send,
    reason = "preview lifecycle state is confined to the support app's main-thread executor"
)]
async fn idle_shutdown_task(
    activity: Rc<PreviewActivity>,
    activity_events: Receiver<()>,
    registration_path: Rc<PathBuf>,
) {
    let idle_shutdown_after = preview_idle_shutdown_after();

    loop {
        if activity.active_connections() != 0 {
            activity_events
                .recv()
                .await
                .expect("preview activity event channel must remain open");
            continue;
        }

        let idle_for = activity.idle_for();
        if idle_for >= idle_shutdown_after {
            tracing::info!(
                idle_for_ms = idle_for.as_millis(),
                idle_shutdown_after_ms = idle_shutdown_after.as_millis(),
                "Preview support app is idle and will exit"
            );
            exit_process(registration_path.as_ref());
        }

        let remaining = idle_shutdown_after
            .checked_sub(idle_for)
            .expect("preview idle duration was checked above");
        futures_lite::future::race(
            async {
                activity_events
                    .recv()
                    .await
                    .expect("preview activity event channel must remain open");
            },
            async {
                async_io::Timer::after(remaining).await;
            },
        )
        .await;
    }
}

async fn register_preview_instance(
    addr: std::net::SocketAddr,
    waterui_core_fingerprint: &str,
) -> io::Result<Rc<PathBuf>> {
    let instance = PreviewAppInstance::new(
        std::process::id(),
        addr.ip(),
        addr.port(),
        waterui_core_fingerprint,
    );
    let path = preview_instance_registry_path(&instance);
    let registry_dir = preview_instance_registry_dir();
    tracing::info!(path = %registry_dir.display(), pid = instance.pid, port = instance.port, "Preview registry create_dir_all");
    async_fs::create_dir_all(&registry_dir).await?;

    let bytes = serde_json::to_vec_pretty(&instance)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    tracing::info!(path = %temp.display(), final_path = %path.display(), "Preview registry writing instance file");
    async_fs::write(&temp, bytes).await?;
    async_fs::rename(&temp, &path).await?;
    tracing::info!(path = %path.display(), "Preview registry wrote instance file");
    Ok(Rc::new(path))
}

fn cleanup_registration_file(path: &std::path::Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => panic!(
            "failed to remove preview registration file {}: {error}",
            path.display()
        ),
    }
}

fn exit_process(registration_path: &std::path::Path) -> ! {
    cleanup_registration_file(registration_path);
    std::process::exit(0)
}

fn bind_first_available(config: PreviewTcpConfig) -> io::Result<std::net::TcpListener> {
    for port in config.ports() {
        let addr = std::net::SocketAddr::new(config.host, port);
        match std::net::TcpListener::bind(addr) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                return Ok(listener);
            }
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {}
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

#[expect(
    clippy::future_not_send,
    reason = "preview connections share lifecycle state on the support app's main-thread executor"
)]
async fn handle_connection(
    stream: Async<std::net::TcpStream>,
    worker_tx: Sender<WorkerMessage>,
    activity: Rc<PreviewActivity>,
    registration_path: Rc<PathBuf>,
    waterui_core_fingerprint: &str,
) -> io::Result<()> {
    let _connection_guard = ActiveConnectionGuard::new(activity.clone());
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;

    loop {
        let request = match read_frame::<_, PreviewRequest>(&mut reader).await {
            Ok(req) => req,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };

        activity.touch();

        let should_shutdown = matches!(request, PreviewRequest::Shutdown);

        let response = match request {
            PreviewRequest::Ping => PreviewResponse::Pong {
                protocol: protocol_info(waterui_core_fingerprint),
            },
            PreviewRequest::Shutdown => PreviewResponse::Shutdown,
            request @ (PreviewRequest::HasDylib { .. } | PreviewRequest::Render { .. }) => {
                let (resp_tx, resp_rx) = async_channel::bounded::<PreviewResponse>(1);
                worker_tx
                    .send(WorkerMessage {
                        request,
                        respond_to: resp_tx,
                    })
                    .await
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::BrokenPipe, "preview worker exited")
                    })?;

                resp_rx.recv().await.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "preview worker exited")
                })?
            }
        };
        activity.touch();

        write_frame(&mut writer, &response).await?;
        activity.touch();

        if should_shutdown {
            drop(reader);
            let _ = writer;
            exit_process(registration_path.as_ref());
        }
    }
}

struct DylibCache {
    capacity: NonZeroUsize,
    libraries: indexmap::IndexMap<DylibId, PreviewLibrary>,
    disk_present: HashSet<DylibId>,
}

impl DylibCache {
    async fn load() -> io::Result<Self> {
        let capacity = dylib_cache_capacity();
        let disk_present = load_disk_dylibs().await?;
        Ok(Self {
            capacity,
            libraries: indexmap::IndexMap::new(),
            disk_present,
        })
    }

    fn contains(&self, id: &DylibId) -> bool {
        self.libraries.contains_key(id) || self.disk_present.contains(id)
    }

    fn get(&mut self, id: &DylibId) -> Option<&PreviewLibrary> {
        self.touch(id);
        self.libraries.get(id)
    }

    /// Records a library that also exists on disk.
    ///
    /// Loading a preview dylib is a unix-only path — the preview system targets
    /// macOS, the iOS Simulator and Android — so both callers sit behind
    /// `cfg(unix)` and this is dead code anywhere else.
    #[cfg(unix)]
    fn insert_persistent(&mut self, id: DylibId, library: PreviewLibrary) {
        self.disk_present.insert(id);
        self.insert_loaded(id, library);
    }

    fn insert_loaded(&mut self, id: DylibId, library: PreviewLibrary) {
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

    async fn ensure_loaded(
        &mut self,
        id: DylibId,
    ) -> Result<Option<PreviewDylibLoadTimings>, PreviewError> {
        if self.libraries.contains_key(&id) {
            self.touch(&id);
            return Ok(None);
        }

        if !self.disk_present.contains(&id) {
            return Err(PreviewError::UnknownDylibId(id));
        }

        let path = preview_dylib_cache_path(id);
        // SAFETY: loading a preview dylib runs its initializers and trusts its
        // exported symbols; `path` is a dylib this toolchain just built for preview.
        let (library, timings) = match unsafe { PreviewLibrary::load_from_path(&path) }.await {
            Ok(loaded) => loaded,
            Err(LoadError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                self.disk_present.remove(&id);
                return Err(PreviewError::UnknownDylibId(id));
            }
            Err(error) => return Err(PreviewError::DylibLoad(error.to_string())),
        };
        self.insert_loaded(id, library);
        Ok(Some(timings))
    }
}

async fn load_disk_dylibs() -> io::Result<HashSet<DylibId>> {
    use futures_lite::stream::StreamExt as _;

    let dir = preview_dylib_cache_dir();
    async_fs::create_dir_all(&dir).await?;

    let mut entries = async_fs::read_dir(&dir).await?;
    let mut present = HashSet::new();
    while let Some(entry) = entries.try_next().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("dylib") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid preview dylib cache entry (non-utf8 filename): {}",
                    path.display()
                ),
            ));
        };

        let id = stem.parse::<DylibId>().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid preview dylib cache entry {}: {e}", path.display()),
            )
        })?;
        present.insert(id);
    }
    Ok(present)
}

fn dylib_cache_capacity() -> NonZeroUsize {
    const DEFAULT: usize = 8;
    let capacity = match std::env::var("WATERUI_PREVIEW_DYLIB_CACHE_SIZE") {
        Ok(value) => value.parse::<usize>().unwrap_or_else(|error| {
            panic!("invalid WATERUI_PREVIEW_DYLIB_CACHE_SIZE value `{value}`: {error}")
        }),
        Err(std::env::VarError::NotPresent) => DEFAULT,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("WATERUI_PREVIEW_DYLIB_CACHE_SIZE must be valid UTF-8")
        }
    };
    NonZeroUsize::new(capacity).expect("WATERUI_PREVIEW_DYLIB_CACHE_SIZE must be greater than zero")
}

#[expect(
    clippy::future_not_send,
    reason = "preview runtime drives this on the main-thread support-app executor; `Environment` is `!Send` by design"
)]
async fn render_worker(
    env: Environment,
    worker_rx: Receiver<WorkerMessage>,
    waterui_core_fingerprint: String,
) {
    let mut cache = DylibCache::load()
        .await
        .expect("failed to initialize preview dylib cache");

    while let Ok(msg) = worker_rx.recv().await {
        let response =
            handle_request(&env, &mut cache, msg.request, &waterui_core_fingerprint).await;
        let _ = msg.respond_to.send(response).await;
    }
}

#[expect(
    clippy::future_not_send,
    reason = "preview runtime drives this on the main-thread support-app executor; `Environment` is `!Send` by design"
)]
async fn handle_request(
    env: &Environment,
    cache: &mut DylibCache,
    request: PreviewRequest,
    waterui_core_fingerprint: &str,
) -> PreviewResponse {
    match request {
        PreviewRequest::Ping => PreviewResponse::Pong {
            protocol: protocol_info(waterui_core_fingerprint),
        },
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

async fn ensure_dylib_cached(
    cache: &mut DylibCache,
    dylib: DylibSource,
) -> Result<EnsuredDylib, PreviewError> {
    let mut load_timings = None;
    let id = match dylib {
        DylibSource::Bytes { id, bytes } => {
            if !cache.contains(&id) {
                #[cfg(unix)]
                {
                    // SAFETY: as above — `bytes` are the preview dylib this session
                    // received for `id`.
                    let (library, timings) = unsafe { PreviewLibrary::load_from_bytes(id, &bytes) }
                        .await
                        .map_err(|e| PreviewError::DylibLoad(e.to_string()))?;
                    load_timings = Some(timings);
                    cache.insert_persistent(id, library);
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
        DylibSource::LocalPath { id, path } => {
            if !cache.contains(&id) {
                #[cfg(unix)]
                {
                    let (library, timings) =
                        // SAFETY: as above — a locally built preview dylib for `id`.
                        unsafe { PreviewLibrary::load_from_local_path(id, &path) }
                            .await
                            .map_err(|e| PreviewError::DylibLoad(e.to_string()))?;
                    load_timings = Some(timings);
                    cache.insert_persistent(id, library);
                }
                #[cfg(not(unix))]
                {
                    let _ = path;
                    return Err(PreviewError::DylibLoad(
                        "local-path preview loading not supported on this platform".to_string(),
                    ));
                }
            }
            id
        }
        DylibSource::Cached { id } => id,
    };

    if load_timings.is_none() {
        load_timings = cache.ensure_loaded(id).await?;
    }
    Ok(EnsuredDylib { id, load_timings })
}

#[derive(Debug)]
struct EnsuredDylib {
    id: DylibId,
    load_timings: Option<PreviewDylibLoadTimings>,
}

fn load_preview_view(
    cache: &mut DylibCache,
    id: DylibId,
    symbol: &str,
) -> Result<waterui_core::AnyView, PreviewError> {
    let library = cache.get(&id).ok_or(PreviewError::UnknownDylibId(id))?;
    if !library.has_symbol(symbol) {
        return Err(PreviewError::SymbolNotFound(symbol.to_string()));
    }

    // SAFETY: `symbol` is a `waterui_preview_*` name, which the `#[preview]` macro
    // only ever generates with the signature `load_view` expects.
    unsafe { library.load_view(symbol) }.map_err(|e| PreviewError::RenderFailed(e.to_string()))
}

#[expect(
    clippy::future_not_send,
    reason = "preview runtime drives this on the main-thread support-app executor; `Environment` is `!Send` by design"
)]
async fn handle_render(
    env: &Environment,
    cache: &mut DylibCache,
    dylib: DylibSource,
    symbol: &str,
    frame: Size,
) -> Result<PreviewOutput, PreviewError> {
    let total_start = Instant::now();
    let cache_start = Instant::now();
    let ensured = ensure_dylib_cached(cache, dylib).await?;
    let cache_elapsed_ms = elapsed_ms(cache_start);
    let id = ensured.id;
    tracing::info!(
        dylib_id = %id,
        elapsed_ms = cache_elapsed_ms,
        "Preview support app ensured dylib is cached and loaded"
    );

    let load_view_start = Instant::now();
    let view = load_preview_view(cache, id, symbol)?;
    let load_view_ms = elapsed_ms(load_view_start);
    tracing::info!(
        dylib_id = %id,
        symbol,
        elapsed_ms = load_view_ms,
        "Preview support app resolved preview symbol"
    );

    let renderer = env
        .get::<ViewRenderer>()
        .expect("ViewRenderer missing in Environment");
    let render_size = RenderSize::new(frame.width, frame.height);

    let render_start = Instant::now();
    let result = renderer.render(view, render_size).await;
    let render_ms = elapsed_ms(render_start);
    tracing::info!(
        dylib_id = %id,
        symbol,
        elapsed_ms = render_ms,
        "Preview support app rendered view"
    );
    let png_start = Instant::now();
    let png_data = result.into_png().map_err(PreviewError::RenderFailed)?;
    let png_encode_ms = elapsed_ms(png_start);
    let total_ms = elapsed_ms(total_start);
    tracing::info!(
        dylib_id = %id,
        symbol,
        png_bytes = png_data.len(),
        elapsed_ms = png_encode_ms,
        total_elapsed_ms = total_ms,
        "Preview support app encoded PNG"
    );

    Ok(PreviewOutput {
        png_data,
        timings: PreviewRenderTimings {
            ensure_dylib_cached_ms: cache_elapsed_ms,
            dylib_load: ensured.load_timings,
            load_view_ms,
            render_ms,
            png_encode_ms,
            total_ms,
        },
    })
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}
