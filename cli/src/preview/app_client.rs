//! TCP client for communicating with the preview support app.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use color_eyre::eyre::WrapErr as _;
use color_eyre::eyre::{Result, bail};
use futures::{FutureExt as _, pin_mut, select};
use smol::Timer;
use smol::net::TcpStream;

use super::protocol::{
    AppError, AppRequest, AppResponse, DylibId, DylibSource, PreviewTcpConfig, Size,
};

use waterui_preview_protocol::registry::{PreviewAppInstance, preview_instance_registry_dir};
use waterui_preview_protocol::transport::{read_frame, write_frame};

/// TCP client for the preview support app.
#[derive(Debug)]
pub struct PreviewAppClient {
    stream: TcpStream,
    /// Dylib ids known to be present in the app for this connection.
    present_dylibs: HashSet<DylibId>,
}

impl PreviewAppClient {
    /// Try to connect directly to a known preview app socket address.
    ///
    /// # Errors
    /// Returns an error if the address is unreachable or the preview handshake fails.
    pub async fn connect_addr(
        addr: SocketAddr,
        expected_waterui_core_fingerprint: &str,
    ) -> Result<Self> {
        Self::connect_to_addr(addr, expected_waterui_core_fingerprint)
            .await
            .ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "Could not connect to preview app at {addr} for runtime {expected_waterui_core_fingerprint}"
                )
            })
    }

    /// Try to connect to a registered local preview app instance.
    ///
    /// # Errors
    /// Returns an error if no matching live registered preview app is found.
    pub async fn connect_registered(expected_waterui_core_fingerprint: &str) -> Result<Self> {
        let expected = expected_waterui_core_fingerprint.to_string();
        let instances = smol::unblock(move || load_registered_instances_sync(&expected)).await?;
        tracing::info!(
            instance_count = instances.len(),
            "Preview loaded matching registered app instances"
        );

        for instance in instances {
            tracing::info!(pid = instance.pid, host = %instance.host, port = instance.port, "Preview trying registered app instance");
            let addr = SocketAddr::new(instance.host, instance.port);
            if let Some(client) =
                Self::connect_to_addr(addr, expected_waterui_core_fingerprint).await
            {
                return Ok(client);
            }
        }

        bail!(
            "Could not connect to a matching registered preview app. Launch a new preview support app for the current runtime."
        )
    }

    /// Try to connect to a running preview app.
    ///
    /// # Errors
    /// Returns an error if no preview app is found.
    pub async fn connect(
        config: PreviewTcpConfig,
        expected_waterui_core_fingerprint: &str,
    ) -> Result<Self> {
        for port in config.ports() {
            if let Some(client) =
                Self::connect_on_port(config, port, expected_waterui_core_fingerprint).await
            {
                return Ok(client);
            }
        }

        bail!(
            "Could not connect to preview app. Make sure it is running.\nThe preview app listens on ports {}..={}.",
            config.port_start,
            config.ports().end()
        )
    }

    async fn connect_on_port(
        config: PreviewTcpConfig,
        port: u16,
        expected_waterui_core_fingerprint: &str,
    ) -> Option<Self> {
        let addr = SocketAddr::new(config.host, port);
        Self::connect_to_addr(addr, expected_waterui_core_fingerprint).await
    }

    async fn connect_to_addr(
        addr: SocketAddr,
        expected_waterui_core_fingerprint: &str,
    ) -> Option<Self> {
        let stream = match connect_with_timeout(addr, connect_timeout()).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!("Preview TCP connect failed on {addr}: {error}");
                return None;
            }
        };

        tracing::info!("Connected to preview app on {addr}");
        let _ = stream.set_nodelay(true);

        let mut client = Self {
            stream,
            present_dylibs: HashSet::new(),
        };

        // Fast handshake: ensure the server is responsive (not just accepting TCP).
        //
        // Some failure modes leave the TCP listener alive while the single render worker
        // is wedged, causing all requests to hang. A short Ping roundtrip detects this.
        let handshake = AppRequest::Ping;
        match client
            .request_with_timeout(handshake, handshake_timeout())
            .await
        {
            Ok(AppResponse::Pong { protocol }) => {
                if protocol.waterui_core_fingerprint == expected_waterui_core_fingerprint {
                    return Some(client);
                }

                tracing::warn!(
                    "Preview runtime mismatch on {addr}: app waterui_core='{}' (build {}), expected='{}'",
                    protocol.waterui_core_fingerprint,
                    protocol.build_commit,
                    expected_waterui_core_fingerprint
                );
            }
            Ok(other) => {
                tracing::warn!("Preview handshake got unexpected response from {addr}: {other:?}");
            }
            Err(err) => {
                tracing::warn!("Preview handshake failed on {addr}: {err}");
            }
        }

        None
    }

    /// Render a view symbol to PNG bytes.
    ///
    /// # Errors
    /// Returns an error if the preview app rejects the request or the transport fails.
    pub async fn render(
        &mut self,
        dylib_id: DylibId,
        dylib_bytes: &[u8],
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Vec<u8>> {
        self.render_with_dylib_source(dylib_id, dylib_bytes, symbol, width, height)
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Preview app error: {e}"))
    }

    /// Render a view symbol, loading dylib bytes from file only when needed.
    ///
    /// # Errors
    /// Returns an error if the preview app cannot be queried or the dylib file cannot be read.
    pub async fn render_with_dylib_file(
        &mut self,
        dylib_id: DylibId,
        dylib_path: &Path,
        symbol: &str,
        width: f32,
        height: f32,
        prefer_local_path: bool,
    ) -> Result<Vec<u8>, AppError> {
        let total_start = Instant::now();
        if self.present_dylibs.contains(&dylib_id) {
            let png = self
                .render_with_source(DylibSource::Cached { id: dylib_id }, symbol, width, height)
                .await?;
            tracing::info!(
                dylib_id = %dylib_id,
                elapsed_ms = total_start.elapsed().as_millis(),
                "Preview rendered with in-connection cached dylib"
            );
            return Ok(png);
        }

        let has_dylib_start = Instant::now();
        let present = self
            .has_dylib(dylib_id)
            .await
            .map_err(|e| AppError::RenderFailed(format!("transport error: {e}")))?;
        tracing::info!(
            dylib_id = %dylib_id,
            present,
            elapsed_ms = has_dylib_start.elapsed().as_millis(),
            "Preview queried support-app dylib cache"
        );
        if present {
            self.present_dylibs.insert(dylib_id);
            let png = self
                .render_with_source(DylibSource::Cached { id: dylib_id }, symbol, width, height)
                .await?;
            tracing::info!(
                dylib_id = %dylib_id,
                elapsed_ms = total_start.elapsed().as_millis(),
                "Preview rendered with support-app cached dylib"
            );
            return Ok(png);
        }

        if prefer_local_path {
            if !dylib_path.is_absolute() {
                return Err(AppError::RenderFailed(format!(
                    "local preview dylib path must be absolute: {}",
                    dylib_path.display()
                )));
            }

            let render_start = Instant::now();
            let result = self
                .render_with_source(
                    DylibSource::LocalPath {
                        id: dylib_id,
                        path: dylib_path.to_path_buf(),
                    },
                    symbol,
                    width,
                    height,
                )
                .await;
            tracing::info!(
                dylib_id = %dylib_id,
                path = %dylib_path.display(),
                elapsed_ms = render_start.elapsed().as_millis(),
                total_elapsed_ms = total_start.elapsed().as_millis(),
                "Preview rendered after transferring dylib path"
            );

            if result.is_ok() || matches!(result, Err(AppError::SymbolNotFound(_))) {
                self.present_dylibs.insert(dylib_id);
            }

            return result;
        }

        let read_start = Instant::now();
        let dylib_bytes = smol::fs::read(dylib_path)
            .await
            .map_err(|e| AppError::RenderFailed(format!("failed to read dylib: {e}")))?;
        tracing::info!(
            dylib_id = %dylib_id,
            bytes = dylib_bytes.len(),
            elapsed_ms = read_start.elapsed().as_millis(),
            "Preview loaded dylib bytes from disk"
        );

        let render_start = Instant::now();
        let result = self
            .render_with_source(
                DylibSource::Bytes {
                    id: dylib_id,
                    bytes: dylib_bytes,
                },
                symbol,
                width,
                height,
            )
            .await;
        tracing::info!(
            dylib_id = %dylib_id,
            elapsed_ms = render_start.elapsed().as_millis(),
            total_elapsed_ms = total_start.elapsed().as_millis(),
            "Preview rendered after transferring dylib bytes"
        );

        if result.is_ok() || matches!(result, Err(AppError::SymbolNotFound(_))) {
            self.present_dylibs.insert(dylib_id);
        }

        result
    }

    /// Render a view symbol, returning structured app errors for caller handling.
    ///
    /// # Errors
    /// Returns an error if the preview app cannot render the symbol or the transport fails.
    pub async fn render_with_dylib_source(
        &mut self,
        dylib_id: DylibId,
        dylib_bytes: &[u8],
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Vec<u8>, AppError> {
        let dylib = if self.present_dylibs.contains(&dylib_id) {
            DylibSource::Cached { id: dylib_id }
        } else {
            let present = self
                .has_dylib(dylib_id)
                .await
                .map_err(|e| AppError::RenderFailed(format!("transport error: {e}")))?;
            if present {
                self.present_dylibs.insert(dylib_id);
                DylibSource::Cached { id: dylib_id }
            } else {
                DylibSource::Bytes {
                    id: dylib_id,
                    bytes: dylib_bytes.to_vec(),
                }
            }
        };

        let result = self.render_with_source(dylib, symbol, width, height).await;

        if result.is_ok() || matches!(result, Err(AppError::SymbolNotFound(_))) {
            self.present_dylibs.insert(dylib_id);
        } else if matches!(result, Err(AppError::UnknownDylibId(_))) {
            self.present_dylibs.remove(&dylib_id);
        }

        result
    }

    async fn render_with_source(
        &mut self,
        dylib: DylibSource,
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Vec<u8>, AppError> {
        let request = AppRequest::Render {
            dylib,
            symbol: symbol.to_string(),
            frame: Size::new(width, height),
        };

        let response = self
            .request(request)
            .await
            .map_err(|e| AppError::RenderFailed(format!("transport error: {e}")))?;

        match response {
            waterui_preview_protocol::PreviewResponse::Render { result } => result.map(|output| {
                tracing::info!(timings = ?output.timings, "Preview support app timing breakdown");
                output.png_data
            }),
            other => Err(AppError::RenderFailed(format!(
                "protocol error: unexpected response to Render: {other:?}"
            ))),
        }
    }

    /// Ask the preview app to shut down.
    ///
    /// # Errors
    /// Returns an error if the shutdown request cannot be sent or the app replies with an unexpected message.
    pub async fn shutdown(&mut self) -> Result<()> {
        let response = self.request(AppRequest::Shutdown).await?;
        match response {
            waterui_preview_protocol::PreviewResponse::Shutdown => Ok(()),
            other => bail!("Protocol error: unexpected response to Shutdown: {other:?}"),
        }
    }

    async fn has_dylib(&mut self, id: DylibId) -> Result<bool> {
        let response = self.request(AppRequest::HasDylib { id }).await?;
        match response {
            waterui_preview_protocol::PreviewResponse::HasDylib { present } => Ok(present),
            other => bail!("Protocol error: unexpected response to HasDylib: {other:?}"),
        }
    }

    async fn request(&mut self, request: AppRequest) -> Result<AppResponse> {
        let timeout = request_timeout_for(&request);
        self.request_with_timeout(request, timeout).await
    }

    async fn request_with_timeout(
        &mut self,
        request: AppRequest,
        timeout: Duration,
    ) -> Result<AppResponse> {
        let kind = request_kind(&request);
        let start = Instant::now();
        write_frame(&mut self.stream, &request)
            .await
            .wrap_err("Failed to send request")?;

        let recv = async {
            match read_frame::<_, AppResponse>(&mut self.stream).await {
                Ok(response) => Ok(response),
                Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => {
                    bail!(
                        "Preview app connection closed unexpectedly (the preview process likely crashed). Check crash logs in ~/Library/Logs/DiagnosticReports/WaterUIApp-*.ips"
                    );
                }
                Err(err) => Err(err).wrap_err("Failed to receive response"),
            }
        }
        .fuse();
        let timeout_fut = Timer::after(timeout).fuse();

        pin_mut!(recv);
        pin_mut!(timeout_fut);

        select! {
            result = recv => {
                if result.is_ok() {
                    tracing::info!(
                        request = kind,
                        elapsed_ms = start.elapsed().as_millis(),
                        "Preview app request completed"
                    );
                }
                result
            },
            _ = timeout_fut => {
                bail!("Preview app request timed out after {timeout:?} ({kind})");
            }
        }
    }
}

fn load_registered_instances_sync(
    expected_waterui_core_fingerprint: &str,
) -> io::Result<Vec<PreviewAppInstance>> {
    let dir = preview_instance_registry_dir();
    fs::create_dir_all(&dir)?;

    let mut matching = Vec::new();
    let mut stale_paths = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };

        let instance = match serde_json::from_slice::<PreviewAppInstance>(&bytes) {
            Ok(instance) => instance,
            Err(_) => {
                stale_paths.push(path);
                continue;
            }
        };

        if !is_pid_alive_sync(instance.pid) {
            stale_paths.push(path);
            continue;
        }

        if instance.waterui_core_fingerprint == expected_waterui_core_fingerprint {
            matching.push(instance);
        }
    }

    matching.sort_by(|left, right| right.registered_at_unix_ms.cmp(&left.registered_at_unix_ms));

    for path in stale_paths {
        let _ = fs::remove_file(path);
    }

    Ok(matching)
}

fn is_pid_alive_sync(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .is_ok_and(|status| status.success())
}

fn connect_timeout() -> Duration {
    const DEFAULT_MS: u64 = 100;
    std::env::var("WATERUI_PREVIEW_CONNECT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or_else(|| Duration::from_millis(DEFAULT_MS), Duration::from_millis)
}

fn handshake_timeout() -> Duration {
    const DEFAULT_MS: u64 = 500;
    std::env::var("WATERUI_PREVIEW_HANDSHAKE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or_else(|| Duration::from_millis(DEFAULT_MS), Duration::from_millis)
}

fn request_timeout() -> Duration {
    const DEFAULT_MS: u64 = 20_000;
    std::env::var("WATERUI_PREVIEW_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or_else(|| Duration::from_millis(DEFAULT_MS), Duration::from_millis)
}

fn render_request_timeout() -> Duration {
    const DEFAULT_MS: u64 = 120_000;
    std::env::var("WATERUI_PREVIEW_RENDER_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or_else(|| Duration::from_millis(DEFAULT_MS), Duration::from_millis)
}

fn request_timeout_for(request: &AppRequest) -> Duration {
    match request {
        AppRequest::Render { .. } => render_request_timeout(),
        _ => request_timeout(),
    }
}

const fn request_kind(request: &AppRequest) -> &'static str {
    match request {
        AppRequest::Ping => "Ping",
        AppRequest::HasDylib { .. } => "HasDylib",
        AppRequest::Render { .. } => "Render",
        AppRequest::Shutdown => "Shutdown",
    }
}

async fn connect_with_timeout(addr: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
    let connect = TcpStream::connect(addr).fuse();
    let timeout_fut = Timer::after(timeout).fuse();

    pin_mut!(connect);
    pin_mut!(timeout_fut);

    select! {
        result = connect => result,
        _ = timeout_fut => Err(io::Error::new(io::ErrorKind::TimedOut, "preview TCP connect timed out")),
    }
}
