//! Shared TCP protocol between `water` CLI and the preview support app.

use serde::de::{Error as DeError, Visitor as DeVisitor};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use waterkit_fs::WaterFs;

/// Build commit hash for protocol compatibility checks.
pub const PREVIEW_PROTOCOL_COMMIT: &str = env!("WATERUI_PREVIEW_PROTOCOL_COMMIT");

#[must_use]
/// Return protocol metadata for handshake responses.
pub fn protocol_info(waterui_core_fingerprint: impl Into<String>) -> PreviewProtocolInfo {
    PreviewProtocolInfo {
        build_commit: PREVIEW_PROTOCOL_COMMIT.to_string(),
        waterui_core_fingerprint: waterui_core_fingerprint.into(),
    }
}

/// Protocol metadata exchanged during ping/pong handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewProtocolInfo {
    /// Build commit hash of the preview support app.
    pub build_commit: String,
    /// Fingerprint of the `waterui-core` package used by this preview app build.
    pub waterui_core_fingerprint: String,
}

pub mod registry {
    //! Preview support app instance registry shared between the CLI and local preview apps.

    use std::net::IpAddr;
    use std::path::PathBuf;

    use serde::{Deserialize, Serialize};

    use super::{SystemTime, UNIX_EPOCH, WaterFs};

    /// A registered local preview support app instance.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PreviewAppInstance {
        /// Operating-system process identifier of the support app.
        pub pid: u32,
        /// Host address the support app listens on.
        pub host: IpAddr,
        /// TCP port the support app listens on.
        pub port: u16,
        /// Runtime fingerprint of the support app build.
        pub waterui_core_fingerprint: String,
        /// Registration timestamp in milliseconds since the Unix epoch.
        pub registered_at_unix_ms: u64,
    }

    impl PreviewAppInstance {
        #[must_use]
        /// Create a registry entry for a support app instance.
        pub fn new(
            pid: u32,
            host: IpAddr,
            port: u16,
            waterui_core_fingerprint: impl Into<String>,
        ) -> Self {
            Self {
                pid,
                host,
                port,
                waterui_core_fingerprint: waterui_core_fingerprint.into(),
                registered_at_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
                    .unwrap_or_default(),
            }
        }
    }

    fn water_cache_dir() -> PathBuf {
        if let Some(cache_dir) = std::env::var_os("WATER_CACHE_DIR") {
            return PathBuf::from(cache_dir);
        }

        if let Some(cache_dir) = WaterFs::cache_dir() {
            return cache_dir.join("waterui");
        }

        std::env::temp_dir().join("waterui-cache")
    }

    #[must_use]
    /// Root cache directory for preview support assets.
    pub fn preview_cache_root_dir() -> PathBuf {
        water_cache_dir().join("preview")
    }

    #[must_use]
    /// Directory containing registered preview support app instances.
    pub fn preview_instance_registry_dir() -> PathBuf {
        preview_cache_root_dir().join("instances")
    }

    #[must_use]
    /// Path of the JSON registry file for a support app instance.
    pub fn preview_instance_registry_path(instance: &PreviewAppInstance) -> PathBuf {
        preview_instance_registry_dir().join(format!("{}-{}.json", instance.pid, instance.port))
    }
}

pub mod transport {
    //! Framed binary transport helpers.
    //!
    //! The preview protocol uses length-prefixed frames:
    //! `u32::to_be_bytes(len)` followed by `len` bytes of binary payload.

    use std::io;

    use futures_lite::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    /// Length prefix size for framed messages (big-endian `u32`).
    pub const LEN_PREFIX_BYTES: usize = 4;

    /// Hard limit for a single frame to prevent OOM from malformed inputs.
    ///
    /// Override via `WATERUI_PREVIEW_MAX_FRAME_BYTES`.
    pub fn max_frame_bytes() -> usize {
        const DEFAULT: usize = 128 * 1024 * 1024;
        std::env::var("WATERUI_PREVIEW_MAX_FRAME_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT)
    }

    /// Read a single length-prefixed binary frame.
    pub async fn read_frame<R, T>(reader: &mut R) -> io::Result<T>
    where
        R: AsyncRead + Unpin,
        T: DeserializeOwned,
    {
        let mut len_buf = [0u8; LEN_PREFIX_BYTES];
        reader.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        let max = max_frame_bytes();
        if len > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("preview frame too large: {len} bytes (max {max})"),
            ));
        }

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await?;

        let config = bincode::config::standard();
        let (value, bytes_read): (T, usize) = bincode::serde::decode_from_slice(&buf, config)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if bytes_read != buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "trailing bytes after preview frame payload",
            ));
        }
        Ok(value)
    }

    /// Write a single length-prefixed binary frame.
    pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> io::Result<()>
    where
        W: AsyncWrite + Unpin,
        T: Serialize,
    {
        let config = bincode::config::standard();
        let data = bincode::serde::encode_to_vec(value, config)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len: u32 = data.len().try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "preview frame too large for u32 length",
            )
        })?;

        writer.write_all(&len.to_be_bytes()).await?;
        writer.write_all(&data).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Backward-compatible alias for older call sites.
    pub async fn read_json_frame<R, T>(reader: &mut R) -> io::Result<T>
    where
        R: AsyncRead + Unpin,
        T: DeserializeOwned,
    {
        read_frame(reader).await
    }

    /// Backward-compatible alias for older call sites.
    pub async fn write_json_frame<W, T>(writer: &mut W, value: &T) -> io::Result<()>
    where
        W: AsyncWrite + Unpin,
        T: Serialize,
    {
        write_frame(writer, value).await
    }
}

pub mod tcp {
    //! TCP configuration shared by the CLI and the preview support app.

    use std::net::{IpAddr, Ipv4Addr};
    use std::ops::RangeInclusive;

    use thiserror::Error;

    /// Default host the preview support app binds to.
    pub const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    /// Default TCP port range start.
    pub const DEFAULT_PORT_START: u16 = 2106;

    /// Default number of ports to try.
    pub const DEFAULT_PORT_RANGE: u16 = 50;

    /// TCP configuration shared by the CLI and preview app.
    #[derive(Debug, Clone, Copy)]
    pub struct PreviewTcpConfig {
        /// IP address to bind/connect to (defaults to localhost).
        pub host: IpAddr,
        /// First port to try.
        pub port_start: u16,
        /// Number of consecutive ports to try.
        pub port_range: u16,
    }

    impl PreviewTcpConfig {
        #[must_use]
        /// Default localhost configuration.
        pub const fn default_localhost() -> Self {
            Self {
                host: DEFAULT_HOST,
                port_start: DEFAULT_PORT_START,
                port_range: DEFAULT_PORT_RANGE,
            }
        }

        /// Build config from environment variables.
        ///
        /// - `WATERUI_PREVIEW_HOST` (IPv4/IPv6)
        /// - `WATERUI_PREVIEW_PORT_START` (u16)
        /// - `WATERUI_PREVIEW_PORT_RANGE` (u16)
        ///
        /// Missing variables use defaults; present-but-invalid values fail fast.
        pub fn from_env() -> Result<Self, ConfigError> {
            let mut cfg = Self::default_localhost();

            if let Some(host) = std::env::var("WATERUI_PREVIEW_HOST").ok() {
                cfg.host = host.parse().map_err(|_| ConfigError::InvalidHost)?;
            }
            if let Some(port_start) = std::env::var("WATERUI_PREVIEW_PORT_START").ok() {
                cfg.port_start = port_start
                    .parse()
                    .map_err(|_| ConfigError::InvalidPortStart)?;
            }
            if let Some(port_range) = std::env::var("WATERUI_PREVIEW_PORT_RANGE").ok() {
                cfg.port_range = port_range
                    .parse()
                    .map_err(|_| ConfigError::InvalidPortRange)?;
            }

            Ok(cfg)
        }

        #[must_use]
        /// Inclusive port range to scan/bind.
        pub fn ports(&self) -> RangeInclusive<u16> {
            let end = self
                .port_start
                .saturating_add(self.port_range.saturating_sub(1));
            self.port_start..=end
        }
    }

    #[derive(Debug, Error)]
    /// Errors returned by [`PreviewTcpConfig::from_env`].
    pub enum ConfigError {
        #[error("invalid WATERUI_PREVIEW_HOST")]
        /// Host env var is present but invalid.
        InvalidHost,
        #[error("invalid WATERUI_PREVIEW_PORT_START")]
        /// Port start env var is present but invalid.
        InvalidPortStart,
        #[error("invalid WATERUI_PREVIEW_PORT_RANGE")]
        /// Port range env var is present but invalid.
        InvalidPortRange,
    }
}

/// Frame size for rendering.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Size {
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
}

impl Size {
    /// Create a new size.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Stable identifier for a preview dylib payload.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DylibId([u8; 32]);

impl DylibId {
    #[must_use]
    /// Create a dylib id from raw identifier bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    /// Borrow the raw identifier bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    /// Compute a dylib id from the dylib payload bytes.
    pub fn from_payload(bytes: &[u8]) -> Self {
        use sha2::Digest as _;
        let hash: [u8; 32] = sha2::Sha256::digest(bytes).into();
        Self(hash)
    }
}

impl fmt::Debug for DylibId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DylibId({})", self)
    }
}

impl fmt::Display for DylibId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl FromStr for DylibId {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|_| "invalid hex")?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| "expected 32 bytes")?;
        Ok(DylibId(bytes))
    }
}

impl Serialize for DylibId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for DylibId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl DeVisitor<'_> for Visitor {
            type Value = DylibId;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a 64-char hex string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                let bytes = hex::decode(v).map_err(|_| E::custom("invalid hex"))?;
                let bytes: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| E::custom("expected 32 bytes"))?;
                Ok(DylibId(bytes))
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

/// How to provide the dylib used for rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DylibSource {
    /// Inline dylib bytes (used when the app doesn't have `id` yet).
    ///
    /// `id` must equal `sha256(bytes)`.
    Bytes {
        /// Dylib payload identifier.
        id: DylibId,
        /// Raw dylib bytes.
        bytes: Vec<u8>,
    },
    /// Reuse a previously loaded dylib by id.
    Cached {
        /// Dylib payload identifier.
        id: DylibId,
    },
    /// Absolute local dylib path on the same host as the preview support app.
    ///
    /// This avoids retransmitting large dylibs over TCP when the CLI and support app share
    /// a filesystem, such as macOS local preview.
    LocalPath {
        /// Dylib payload identifier.
        id: DylibId,
        /// Absolute local dylib path.
        path: std::path::PathBuf,
    },
}

/// Request from CLI to preview support app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreviewRequest {
    /// Fast liveness probe.
    ///
    /// Used by the CLI to confirm the app is responsive (not just accepting TCP).
    Ping,
    /// Ask whether a dylib id is present in the app cache.
    HasDylib {
        /// Dylib id to query.
        id: DylibId,
    },
    /// Render a view function.
    Render {
        /// Dylib source to use for rendering.
        dylib: DylibSource,
        /// Symbol name (e.g. `waterui_preview_my_crate_sidebar`).
        symbol: String,
        /// Frame size for rendering.
        frame: Size,
    },
    /// Shutdown the preview app.
    Shutdown,
}

/// Successful render output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewOutput {
    /// PNG image bytes.
    pub png_data: Vec<u8>,
    /// Support-app timing breakdown for this render request.
    pub timings: PreviewRenderTimings,
}

/// Timing breakdown for loading a preview dylib into the support app process.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct PreviewDylibLoadTimings {
    /// Time spent materializing the cached dylib file on disk.
    pub cache_file_ms: u64,
    /// Total time spent loading the library from disk.
    pub load_library_ms: u64,
    /// Time spent in the initial `dlopen`.
    pub initial_dlopen_ms: u64,
    /// Time spent verifying an existing code signature, if the initial `dlopen` failed.
    pub codesign_verify_ms: Option<u64>,
    /// Time spent codesigning the dylib, if needed.
    pub codesign_ms: Option<u64>,
    /// Time spent reloading the dylib after codesigning, if needed.
    pub reload_after_codesign_ms: Option<u64>,
}

/// Timing breakdown for a single preview render request inside the support app.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct PreviewRenderTimings {
    /// Time spent ensuring the requested dylib is cached and loaded.
    pub ensure_dylib_cached_ms: u64,
    /// Detailed dylib-load timings when this request had to load a new dylib.
    pub dylib_load: Option<PreviewDylibLoadTimings>,
    /// Time spent resolving the requested preview symbol into a view.
    pub load_view_ms: u64,
    /// Time spent rendering the view to an in-memory image.
    pub render_ms: u64,
    /// Time spent encoding the rendered image to PNG.
    pub png_encode_ms: u64,
    /// Total request time inside the support app.
    pub total_ms: u64,
}

/// Errors that can occur during preview rendering.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum PreviewError {
    /// Requested dylib id is not loaded.
    #[error("Unknown dylib id: {0}")]
    UnknownDylibId(DylibId),
    /// Failed to load dylib.
    #[error("Failed to load dylib: {0}")]
    DylibLoad(String),
    /// Symbol not found in dylib.
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),
    /// Rendering failed.
    #[error("Render failed: {0}")]
    RenderFailed(String),
}

/// Response from preview support app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreviewResponse {
    /// Response to [`PreviewRequest::Ping`].
    Pong {
        /// Protocol metadata for compatibility handshake.
        protocol: PreviewProtocolInfo,
    },
    /// Response to [`PreviewRequest::HasDylib`].
    HasDylib {
        /// Whether the dylib id is present.
        present: bool,
    },
    /// Response to [`PreviewRequest::Render`].
    Render {
        /// Render result or error.
        result: Result<PreviewOutput, PreviewError>,
    },
    /// Response to [`PreviewRequest::Shutdown`].
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    enum LegacyDylibSource {
        Bytes { id: DylibId, bytes: Vec<u8> },
        Cached { id: DylibId },
    }

    #[test]
    fn dylib_id_roundtrip_hex() {
        let id = DylibId::from_bytes([0xAB; 32]);
        let json = serde_json::to_string(&id).unwrap();
        let de: DylibId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, de);
    }

    #[test]
    fn cached_variant_stays_backward_compatible_for_bincode() {
        let id = DylibId::from_bytes([0xCD; 32]);
        let source = DylibSource::Cached { id };
        let bytes = bincode::serde::encode_to_vec(&source, bincode::config::standard())
            .expect("encode cached source");
        let (decoded, consumed): (LegacyDylibSource, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                .expect("decode cached source with legacy layout");
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, LegacyDylibSource::Cached { id });
    }

    #[test]
    fn bytes_variant_stays_backward_compatible_for_bincode() {
        let id = DylibId::from_bytes([0xEF; 32]);
        let source = DylibSource::Bytes {
            id,
            bytes: vec![1, 2, 3, 4],
        };
        let bytes = bincode::serde::encode_to_vec(&source, bincode::config::standard())
            .expect("encode bytes source");
        let (decoded, consumed): (LegacyDylibSource, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                .expect("decode bytes source with legacy layout");
        assert_eq!(consumed, bytes.len());
        assert_eq!(
            decoded,
            LegacyDylibSource::Bytes {
                id,
                bytes: vec![1, 2, 3, 4],
            }
        );
    }

    #[test]
    fn local_path_variant_uses_new_tag_after_legacy_variants() {
        let id = DylibId::from_bytes([0x12; 32]);
        let source = DylibSource::LocalPath {
            id,
            path: PathBuf::from("/tmp/example.dylib"),
        };
        let bytes = bincode::serde::encode_to_vec(&source, bincode::config::standard())
            .expect("encode local path source");
        let err = bincode::serde::decode_from_slice::<LegacyDylibSource, _>(
            &bytes,
            bincode::config::standard(),
        )
        .expect_err("legacy layout must reject new local-path tag");
        assert!(
            err.to_string()
                .contains("expected variant index 0 <= i < 2")
        );
    }
}
