//! TCP client for communicating with the preview support app.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use eyre::WrapErr as _;
use eyre::{Result, bail};
use futures_util::{FutureExt as _, pin_mut, select};
use smol::Timer;
use smol::net::TcpStream;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::protocol::{
    AppError, AppRequest, AppResponse, DylibId, DylibSource, PreviewProtocolInfo,
    PreviewRuntimePlatform, PreviewTcpConfig, Size,
};

use waterui_preview_protocol::registry::{PreviewAppInstance, preview_instance_registry_dir};
use waterui_preview_protocol::transport::{read_frame, write_frame};

/// TCP client for the preview support app.
#[derive(Debug)]
pub struct PreviewAppClient {
    stream: TcpStream,
    /// Dylib ids known to be present in the app for this connection.
    present_dylibs: HashSet<DylibId>,
    /// The preview session's host: timeout escape hatches read through it.
    host: crate::toolchain::Host,
}

/// What probing one or more candidate preview apps produced.
///
/// The three-way split is the whole point. "Nothing answered" and "something
/// answered and is the wrong build" are different failures with different
/// remedies, and collapsing them into one `None` is what let a stale `water`
/// binary present itself as a dead TCP server.
#[derive(Debug)]
pub enum PreviewProbe {
    /// An app answered the handshake and is a build this CLI can drive.
    Connected(Box<PreviewAppClient>),
    /// An app answered and was turned away. The string is the explanation to
    /// put in front of whoever ran `water preview`.
    Rejected(String),
    /// Nothing answered on any candidate address.
    Silent,
}

/// Why an app that answered is not one this CLI can drive.
///
/// Both halves are reported, because either alone is a half-diagnosis: the
/// protocol id says the support app and the CLI were built from different
/// checkouts of the protocol crate, and the runtime fingerprint says they link
/// different `waterui_core` builds.
fn describe_incompatible_app(
    addr: SocketAddr,
    app: &PreviewProtocolInfo,
    expected_core: &str,
    expected_protocol_commit: &str,
) -> String {
    let mut reasons = Vec::new();
    if app.build_commit != expected_protocol_commit {
        reasons.push(format!(
            "  preview protocol: app {app_build}, expected {expected_protocol_commit}",
            app_build = app.build_commit
        ));
    }
    if app.waterui_core_fingerprint != expected_core {
        reasons.push(format!(
            "  runtime: app {}, expected {expected_core}",
            app.waterui_core_fingerprint
        ));
    }
    format!(
        "A preview app is listening on {addr} and answering, but it is not a build this `water` \
         can drive:\n{}\nRebuild whichever of the two is older, so the CLI and the support app \
         come from one checkout.",
        reasons.join("\n")
    )
}

impl PreviewAppClient {
    /// Probe a known preview app socket address.
    pub async fn probe_addr(
        host: &crate::toolchain::Host,
        addr: SocketAddr,
        expected_waterui_core_fingerprint: &str,
        expected_platform: PreviewRuntimePlatform,
        expected_protocol_commit: &str,
    ) -> PreviewProbe {
        let stream = match connect_with_timeout(addr, connect_timeout(host)).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!("Preview TCP connect failed on {addr}: {error}");
                return PreviewProbe::Silent;
            }
        };

        tracing::info!("Connected to preview app on {addr}");
        let _ = stream.set_nodelay(true);

        let mut client = Self {
            stream,
            present_dylibs: HashSet::new(),
            host: host.clone(),
        };

        // Fast handshake: ensure the server is responsive (not just accepting TCP).
        //
        // Some failure modes leave the TCP listener alive while the single render worker
        // is wedged, causing all requests to hang. A short Ping roundtrip detects this.
        let handshake = AppRequest::Ping;
        match client
            .request_with_timeout(handshake, handshake_timeout(host))
            .await
        {
            Ok(AppResponse::Pong { protocol }) => {
                if protocol_is_compatible(
                    &protocol,
                    expected_waterui_core_fingerprint,
                    expected_platform,
                    expected_protocol_commit,
                ) {
                    return PreviewProbe::Connected(Box::new(client));
                }

                tracing::warn!(
                    "Preview runtime mismatch on {addr}: app waterui_core='{}' platform={:?} protocol={}, expected waterui_core='{}' platform={:?} protocol={}",
                    protocol.waterui_core_fingerprint,
                    protocol.platform,
                    protocol.build_commit,
                    expected_waterui_core_fingerprint,
                    expected_platform,
                    expected_protocol_commit,
                );
                return PreviewProbe::Rejected(describe_incompatible_app(
                    addr,
                    &protocol,
                    expected_waterui_core_fingerprint,
                    expected_protocol_commit,
                ));
            }
            Ok(other) => {
                tracing::warn!("Preview handshake got unexpected response from {addr}: {other:?}");
            }
            Err(err) => {
                tracing::warn!("Preview handshake failed on {addr}: {err}");
            }
        }

        PreviewProbe::Silent
    }

    /// Probe every live registered local preview app instance.
    ///
    /// # Errors
    /// Returns an error if the instance registry cannot be read.
    pub async fn probe_registered(
        host: &crate::toolchain::Host,
        expected_waterui_core_fingerprint: &str,
        expected_platform: PreviewRuntimePlatform,
        expected_protocol_commit: &str,
    ) -> Result<PreviewProbe> {
        let instances = smol::unblock(load_live_registered_instances).await?;
        tracing::info!(
            instance_count = instances.len(),
            "Preview loaded registered app instances"
        );

        // An app that answered and was turned away is the one worth reporting:
        // "nothing is listening" sends a reader to the network, and this is
        // never the network.
        let mut rejection = None;
        for (instance, registration) in instances {
            // Instances registered under another runtime fingerprint are the
            // pooled support apps of other sessions — alive, correctly built,
            // and not ours to touch.
            if instance.waterui_core_fingerprint != expected_waterui_core_fingerprint {
                continue;
            }
            tracing::info!(pid = instance.pid, host = %instance.host, port = instance.port, "Preview trying registered app instance");
            let addr = SocketAddr::new(instance.host, instance.port);
            match Self::probe_addr(
                host,
                addr,
                expected_waterui_core_fingerprint,
                expected_platform,
                expected_protocol_commit,
            )
            .await
            {
                PreviewProbe::Connected(client) => return Ok(PreviewProbe::Connected(client)),
                PreviewProbe::Rejected(reason) => {
                    // An instance claiming our runtime that speaks a different
                    // protocol or build is a leftover from an older pair; it
                    // can never serve this CLI, so evict it — its port and
                    // registration go to the app about to launch (#197).
                    tracing::warn!(
                        pid = instance.pid,
                        "Preview evicting a stale registered app instance: {reason}"
                    );
                    let instance = instance.clone();
                    smol::unblock(move || terminate_registered_instance(&instance, &registration))
                        .await?;
                    rejection.get_or_insert(reason);
                }
                PreviewProbe::Silent => {}
            }
        }

        Ok(rejection.map_or(PreviewProbe::Silent, PreviewProbe::Rejected))
    }

    /// Probe the configured port range for a running preview app.
    pub async fn probe_ports(
        host: &crate::toolchain::Host,
        config: PreviewTcpConfig,
        expected_waterui_core_fingerprint: &str,
        expected_platform: PreviewRuntimePlatform,
        expected_protocol_commit: &str,
    ) -> PreviewProbe {
        // Same reasoning as `probe_registered`: an app that answered and was
        // turned away outranks every silent port, because silence is the
        // expected state of a port and an answer is the finding.
        let mut rejection = None;
        for port in config.ports() {
            let addr = SocketAddr::new(config.host, port);
            match Self::probe_addr(
                host,
                addr,
                expected_waterui_core_fingerprint,
                expected_platform,
                expected_protocol_commit,
            )
            .await
            {
                PreviewProbe::Connected(client) => return PreviewProbe::Connected(client),
                PreviewProbe::Rejected(reason) => {
                    rejection.get_or_insert(reason);
                }
                PreviewProbe::Silent => {}
            }
        }

        rejection.map_or(PreviewProbe::Silent, PreviewProbe::Rejected)
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
            .map_err(|e| eyre::eyre!("Preview app error: {e}"))
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
        if let Some(png) = self
            .render_cached_if_present(dylib_id, symbol, width, height)
            .await?
        {
            tracing::info!(
                dylib_id = %dylib_id,
                elapsed_ms = total_start.elapsed().as_millis(),
                "Preview rendered with cached dylib"
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

            self.record_rendered_dylib(dylib_id, &result);

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

        self.record_rendered_dylib(dylib_id, &result);

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
        if let Some(png) = self
            .render_cached_if_present(dylib_id, symbol, width, height)
            .await?
        {
            return Ok(png);
        }

        let result = self
            .render_with_source(
                DylibSource::Bytes {
                    id: dylib_id,
                    bytes: dylib_bytes.to_vec(),
                },
                symbol,
                width,
                height,
            )
            .await;
        self.record_rendered_dylib(dylib_id, &result);
        result
    }

    async fn render_cached_if_present(
        &mut self,
        dylib_id: DylibId,
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Option<Vec<u8>>, AppError> {
        if self.present_dylibs.insert(dylib_id) {
            let query_start = Instant::now();
            let present = match self.has_dylib(dylib_id).await {
                Ok(present) => present,
                Err(error) => {
                    self.present_dylibs.remove(&dylib_id);
                    return Err(AppError::RenderFailed(format!("transport error: {error}")));
                }
            };
            tracing::info!(
                dylib_id = %dylib_id,
                present,
                elapsed_ms = query_start.elapsed().as_millis(),
                "Preview queried support-app dylib cache"
            );
            if !present {
                self.present_dylibs.remove(&dylib_id);
                return Ok(None);
            }
        }

        match self
            .render_with_source(DylibSource::Cached { id: dylib_id }, symbol, width, height)
            .await
        {
            Ok(png) => Ok(Some(png)),
            Err(AppError::UnknownDylibId(_)) => {
                self.present_dylibs.remove(&dylib_id);
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn record_rendered_dylib(&mut self, dylib_id: DylibId, result: &Result<Vec<u8>, AppError>) {
        match result {
            Ok(_) | Err(AppError::SymbolNotFound(_)) => {
                self.present_dylibs.insert(dylib_id);
            }
            Err(AppError::UnknownDylibId(_)) => {
                self.present_dylibs.remove(&dylib_id);
            }
            Err(AppError::DylibLoad(_) | AppError::RenderFailed(_)) => {}
        }
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
            other => {
                bail!("Protocol error: unexpected response to Shutdown: {other:?}");
            }
        }
    }

    async fn has_dylib(&mut self, id: DylibId) -> Result<bool> {
        let response = self.request(AppRequest::HasDylib { id }).await?;
        match response {
            waterui_preview_protocol::PreviewResponse::HasDylib { present } => Ok(present),
            other => {
                bail!("Protocol error: unexpected response to HasDylib: {other:?}");
            }
        }
    }

    async fn request(&mut self, request: AppRequest) -> Result<AppResponse> {
        let timeout = request_timeout_for(&self.host, &request);
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
                        "Preview app connection closed unexpectedly (the preview process likely crashed). Check crash logs in ~/Library/Logs/DiagnosticReports/, filed under the preview application's own name"
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

fn protocol_is_compatible(
    protocol: &PreviewProtocolInfo,
    expected_waterui_core_fingerprint: &str,
    expected_platform: PreviewRuntimePlatform,
    expected_protocol_commit: &str,
) -> bool {
    protocol.waterui_core_fingerprint == expected_waterui_core_fingerprint
        && protocol.platform == expected_platform
        && protocol.build_commit == expected_protocol_commit
}

fn load_live_registered_instances() -> io::Result<Vec<(PreviewAppInstance, std::path::PathBuf)>> {
    let dir = preview_instance_registry_dir();
    fs::create_dir_all(&dir)?;

    let mut candidates = Vec::new();
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

        let Ok(instance) = serde_json::from_slice::<PreviewAppInstance>(&bytes) else {
            stale_paths.push(path);
            continue;
        };

        candidates.push((instance, path));
    }

    let mut matching = Vec::with_capacity(candidates.len());
    if !candidates.is_empty() {
        let mut processes = System::new();
        processes.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        for (instance, path) in candidates {
            if processes.process(Pid::from_u32(instance.pid)).is_some() {
                matching.push((instance, path));
            } else {
                stale_paths.push(path);
            }
        }
    }

    matching.sort_by_key(|(registration, _)| std::cmp::Reverse(registration.registered_at_unix_ms));

    for path in stale_paths {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    Ok(matching)
}

/// Terminate a registered app instance and drop its registry entry.
///
/// The pid is the instance's own claim, checked live by the registry load
/// that produced it; `kill` answers false only when the process exited in
/// the meantime, which is the desired end state anyway.
fn terminate_registered_instance(
    instance: &PreviewAppInstance,
    registration: &Path,
) -> io::Result<()> {
    let mut processes = System::new();
    processes.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );
    if let Some(process) = processes.process(Pid::from_u32(instance.pid)) {
        process.kill();
    }
    match fs::remove_file(registration) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn connect_timeout(host: &crate::toolchain::Host) -> Duration {
    const DEFAULT_MS: u64 = 100;
    timeout_from_host(host, "WATERUI_PREVIEW_CONNECT_TIMEOUT_MS", DEFAULT_MS)
}

fn handshake_timeout(host: &crate::toolchain::Host) -> Duration {
    // 500 ms is plenty over loopback, but an `adb forward` channel to a
    // network-connected device rides the adb transport: every frame pays the
    // remote round trip, so a Ping/Pong handshake measures in the hundreds of
    // milliseconds and can exceed a tight cap even on a healthy app.
    const DEFAULT_MS: u64 = 5000;
    timeout_from_host(host, "WATERUI_PREVIEW_HANDSHAKE_TIMEOUT_MS", DEFAULT_MS)
}

fn request_timeout(host: &crate::toolchain::Host) -> Duration {
    const DEFAULT_MS: u64 = 20_000;
    timeout_from_host(host, "WATERUI_PREVIEW_REQUEST_TIMEOUT_MS", DEFAULT_MS)
}

fn render_request_timeout(host: &crate::toolchain::Host) -> Duration {
    const DEFAULT_MS: u64 = 120_000;
    timeout_from_host(host, "WATERUI_PREVIEW_RENDER_TIMEOUT_MS", DEFAULT_MS)
}

fn timeout_from_host(host: &crate::toolchain::Host, name: &str, default_ms: u64) -> Duration {
    host.env(name).map_or_else(
        || Duration::from_millis(default_ms),
        |value| {
            let value = value
                .to_str()
                .unwrap_or_else(|| panic!("{name} must be valid UTF-8"));
            Duration::from_millis(
                value
                    .parse::<u64>()
                    .unwrap_or_else(|error| panic!("invalid {name} value `{value}`: {error}")),
            )
        },
    )
}

fn request_timeout_for(host: &crate::toolchain::Host, request: &AppRequest) -> Duration {
    match request {
        AppRequest::Render { .. } => render_request_timeout(host),
        _ => request_timeout(host),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::protocol::PREVIEW_PROTOCOL_COMMIT;

    #[test]
    fn protocol_match_requires_exact_preview_build() {
        let protocol = PreviewProtocolInfo {
            build_commit: PREVIEW_PROTOCOL_COMMIT.to_string(),
            waterui_core_fingerprint: "runtime-fingerprint".to_string(),
            platform: PreviewRuntimePlatform::Macos,
        };
        assert!(protocol_is_compatible(
            &protocol,
            "runtime-fingerprint",
            PreviewRuntimePlatform::Macos,
            PREVIEW_PROTOCOL_COMMIT,
        ));

        let stale = PreviewProtocolInfo {
            build_commit: "stale-preview-build".to_string(),
            ..protocol
        };
        assert!(!protocol_is_compatible(
            &stale,
            "runtime-fingerprint",
            PreviewRuntimePlatform::Macos,
            PREVIEW_PROTOCOL_COMMIT,
        ));
    }

    /// The compatibility gate rides the revision the project's framework
    /// resolved, not the revision this CLI's own protocol crate was pinned
    /// at — the two disagree exactly when the pair goes stale, and the
    /// handshake must reject rather than drop mid-session (#197).
    #[test]
    fn protocol_match_tracks_the_resolved_framework_revision() {
        let protocol = PreviewProtocolInfo {
            build_commit: "framework-resolved-rev".to_string(),
            waterui_core_fingerprint: "runtime-fingerprint".to_string(),
            platform: PreviewRuntimePlatform::Macos,
        };
        assert!(protocol_is_compatible(
            &protocol,
            "runtime-fingerprint",
            PreviewRuntimePlatform::Macos,
            "framework-resolved-rev",
        ));
        assert!(!protocol_is_compatible(
            &protocol,
            "runtime-fingerprint",
            PreviewRuntimePlatform::Macos,
            PREVIEW_PROTOCOL_COMMIT,
        ));
    }

    #[test]
    fn rejection_names_both_halves_of_the_mismatch() {
        let addr: SocketAddr = "127.0.0.1:9123".parse().unwrap();
        let app = PreviewProtocolInfo {
            build_commit: "app-protocol-build".to_string(),
            waterui_core_fingerprint: "app-runtime".to_string(),
            platform: PreviewRuntimePlatform::Macos,
        };

        let explanation =
            describe_incompatible_app(addr, &app, "cli-runtime", PREVIEW_PROTOCOL_COMMIT);

        assert!(explanation.contains("127.0.0.1:9123"), "{explanation}");
        assert!(explanation.contains("app-protocol-build"), "{explanation}");
        assert!(
            explanation.contains(PREVIEW_PROTOCOL_COMMIT),
            "{explanation}"
        );
        assert!(explanation.contains("app-runtime"), "{explanation}");
        assert!(explanation.contains("cli-runtime"), "{explanation}");
    }

    #[test]
    fn rejection_reports_only_the_half_that_differs() {
        let addr: SocketAddr = "127.0.0.1:9123".parse().unwrap();
        let app = PreviewProtocolInfo {
            build_commit: PREVIEW_PROTOCOL_COMMIT.to_string(),
            waterui_core_fingerprint: "app-runtime".to_string(),
            platform: PreviewRuntimePlatform::Macos,
        };

        let explanation =
            describe_incompatible_app(addr, &app, "cli-runtime", PREVIEW_PROTOCOL_COMMIT);

        assert!(!explanation.contains("preview protocol:"), "{explanation}");
        assert!(
            explanation.contains("runtime: app app-runtime"),
            "{explanation}"
        );
    }
}
