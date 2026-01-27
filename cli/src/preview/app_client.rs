//! TCP client for communicating with the preview support app.

use std::collections::HashSet;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use color_eyre::eyre::WrapErr as _;
use color_eyre::eyre::{Result, bail};
use futures::{FutureExt as _, pin_mut, select};
use smol::Timer;
use smol::net::TcpStream;

use super::protocol::{AppRequest, AppResponse, DylibId, DylibSource, PreviewTcpConfig, Size};

use waterui_preview_protocol::transport::{read_json_frame, write_json_frame};

/// TCP client for the preview support app.
#[derive(Debug)]
pub struct PreviewAppClient {
    stream: TcpStream,
    /// Dylib ids known to be present in the app for this connection.
    present_dylibs: HashSet<DylibId>,
}

impl PreviewAppClient {
    /// Try to connect to a running preview app.
    ///
    /// # Errors
    /// Returns an error if no preview app is found.
    pub async fn connect(config: PreviewTcpConfig) -> Result<Self> {
        for port in config.ports() {
            let addr = SocketAddr::new(config.host, port);
            if let Ok(stream) = connect_with_timeout(addr, connect_timeout()).await {
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
                if let Ok(waterui_preview_protocol::PreviewResponse::Pong) = client
                    .request_with_timeout(handshake, handshake_timeout())
                    .await
                {
                    return Ok(client);
                }

                // Not responsive - try the next port.
                continue;
            }
        }

        bail!(
            "Could not connect to preview app. Make sure it is running.\nThe preview app listens on ports {}..={}.",
            config.port_start,
            config.ports().end()
        )
    }

    /// Render a view symbol to PNG bytes.
    pub async fn render(
        &mut self,
        dylib_id: DylibId,
        dylib_bytes: &[u8],
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Vec<u8>> {
        let dylib = if self.present_dylibs.contains(&dylib_id) {
            DylibSource::Cached { id: dylib_id }
        } else {
            let present = self.has_dylib(dylib_id).await?;
            if present {
                self.present_dylibs.insert(dylib_id);
                DylibSource::Cached { id: dylib_id }
            } else {
                self.present_dylibs.insert(dylib_id);
                DylibSource::Bytes {
                    id: dylib_id,
                    bytes: dylib_bytes.to_vec(),
                }
            }
        };

        let request = AppRequest::Render {
            dylib,
            symbol: symbol.to_string(),
            frame: Size::new(width, height),
        };

        let response = self.request(request).await?;

        match response {
            waterui_preview_protocol::PreviewResponse::Render { result } => match result {
                Ok(output) => Ok(output.png_data),
                Err(e) => bail!("Preview app error: {e}"),
            },
            other => bail!("Protocol error: unexpected response to Render: {other:?}"),
        }
    }

    /// Ask the preview app to shut down.
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
        self.request_with_timeout(request, request_timeout()).await
    }

    async fn request_with_timeout(&mut self, request: AppRequest, timeout: Duration) -> Result<AppResponse> {
        let kind = request_kind(&request);
        write_json_frame(&mut self.stream, &request)
            .await
            .wrap_err("Failed to send request")?;

        let recv = async {
            read_json_frame::<_, AppResponse>(&mut self.stream)
                .await
                .wrap_err("Failed to receive response")
        }
        .fuse();
        let timeout_fut = Timer::after(timeout).fuse();

        pin_mut!(recv);
        pin_mut!(timeout_fut);

        select! {
            result = recv => result,
            _ = timeout_fut => {
                bail!("Preview app request timed out after {timeout:?} ({kind})");
            }
        }
    }
}

fn connect_timeout() -> Duration {
    const DEFAULT_MS: u64 = 100;
    std::env::var("WATERUI_PREVIEW_CONNECT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_MS))
}

fn handshake_timeout() -> Duration {
    const DEFAULT_MS: u64 = 500;
    std::env::var("WATERUI_PREVIEW_HANDSHAKE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_MS))
}

fn request_timeout() -> Duration {
    const DEFAULT_MS: u64 = 20_000;
    std::env::var("WATERUI_PREVIEW_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_MS))
}

fn request_kind(request: &AppRequest) -> &'static str {
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
