//! Shared protocol between a running `WaterUI` app and the Inspector app.

use serde::{Deserialize, Serialize};

/// Build commit hash for protocol compatibility checks.
pub const INSPECTOR_PROTOCOL_COMMIT: &str = env!("WATERUI_INSPECTOR_PROTOCOL_COMMIT");

/// Return protocol metadata for handshake responses.
#[must_use]
pub fn protocol_info() -> InspectorProtocolInfo {
    InspectorProtocolInfo {
        build_commit: INSPECTOR_PROTOCOL_COMMIT.to_string(),
    }
}

/// Protocol metadata exchanged during handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectorProtocolInfo {
    /// Build commit hash of the runtime/inspector build.
    pub build_commit: String,
}

pub mod transport {
    //! Framed binary transport helpers.
    //!
    //! Message format: `u32::to_be_bytes(len)` + `len` bytes bincode payload.

    use std::io;

    use futures_lite::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    /// Length prefix size for framed messages.
    pub const LEN_PREFIX_BYTES: usize = 4;

    /// Max frame size override env var.
    const MAX_FRAME_ENV: &str = "WATERUI_INSPECTOR_MAX_FRAME_BYTES";

    /// Hard cap for single frames.
    #[must_use]
    pub fn max_frame_bytes() -> usize {
        const DEFAULT: usize = 16 * 1024 * 1024;
        std::env::var(MAX_FRAME_ENV)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT)
    }

    async fn write_encoded_frame<W>(writer: &mut W, data: &[u8]) -> io::Result<()>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let len: u32 = data.len().try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "inspector frame too large for u32 length",
            )
        })?;

        writer.write_all(&len.to_be_bytes()).await?;
        writer.write_all(data).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Read one framed message from an async reader.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when the frame is too large, malformed, or contains
    /// trailing bytes after decoding. Propagates I/O failures from the reader.
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
                format!("inspector frame too large: {len} bytes (max {max})"),
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
                "trailing bytes after inspector frame payload",
            ));
        }
        Ok(value)
    }

    /// Write one framed message to an async writer.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when the encoded payload exceeds the protocol frame
    /// size limit or cannot be serialized. Propagates I/O failures from the writer.
    pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> io::Result<()>
    where
        W: AsyncWrite + Unpin + Send,
        T: Serialize + Sync,
    {
        let config = bincode::config::standard();
        let data = bincode::serde::encode_to_vec(value, config)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        write_encoded_frame(writer, &data).await
    }

    /// Read one framed message from a blocking reader.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when the frame is too large, malformed, or contains
    /// trailing bytes after decoding. Propagates I/O failures from the reader.
    pub fn read_frame_blocking<R, T>(reader: &mut R) -> io::Result<T>
    where
        R: io::Read,
        T: DeserializeOwned,
    {
        let mut len_buf = [0u8; LEN_PREFIX_BYTES];
        reader.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;
        let max = max_frame_bytes();
        if len > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("inspector frame too large: {len} bytes (max {max})"),
            ));
        }

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;

        let config = bincode::config::standard();
        let (value, bytes_read): (T, usize) = bincode::serde::decode_from_slice(&buf, config)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if bytes_read != buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "trailing bytes after inspector frame payload",
            ));
        }
        Ok(value)
    }

    /// Write one framed message to a blocking writer.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` when the encoded payload exceeds the protocol frame
    /// size limit or cannot be serialized. Propagates I/O failures from the writer.
    pub fn write_frame_blocking<W, T>(writer: &mut W, value: &T) -> io::Result<()>
    where
        W: io::Write,
        T: Serialize,
    {
        let config = bincode::config::standard();
        let data = bincode::serde::encode_to_vec(value, config)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len: u32 = data.len().try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "inspector frame too large for u32 length",
            )
        })?;

        writer.write_all(&len.to_be_bytes())?;
        writer.write_all(&data)?;
        writer.flush()?;
        Ok(())
    }
}

pub mod tcp {
    //! TCP configuration for Inspector runtime endpoint.

    use std::net::{IpAddr, Ipv4Addr};
    use std::ops::RangeInclusive;

    use thiserror::Error;

    /// Default host for local inspector endpoint.
    pub const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    /// Default first port.
    pub const DEFAULT_PORT_START: u16 = 23106;
    /// Default number of ports to scan.
    pub const DEFAULT_PORT_RANGE: u16 = 20;

    /// TCP configuration shared by runtime and inspector.
    #[derive(Debug, Clone, Copy)]
    pub struct InspectorTcpConfig {
        /// IP address to bind/connect to.
        pub host: IpAddr,
        /// First port to try.
        pub port_start: u16,
        /// Number of consecutive ports to try.
        pub port_range: u16,
    }

    impl InspectorTcpConfig {
        /// Default localhost configuration.
        #[must_use]
        pub const fn default_localhost() -> Self {
            Self {
                host: DEFAULT_HOST,
                port_start: DEFAULT_PORT_START,
                port_range: DEFAULT_PORT_RANGE,
            }
        }

        /// Build config from env vars.
        ///
        /// - `WATERUI_INSPECTOR_HOST`
        /// - `WATERUI_INSPECTOR_PORT_START`
        /// - `WATERUI_INSPECTOR_PORT_RANGE`
        ///
        /// # Errors
        ///
        /// Returns an error when any configured env var fails to parse.
        pub fn from_env() -> Result<Self, ConfigError> {
            let mut cfg = Self::default_localhost();

            if let Ok(host) = std::env::var("WATERUI_INSPECTOR_HOST") {
                cfg.host = host.parse().map_err(|_| ConfigError::InvalidHost)?;
            }
            if let Ok(port_start) = std::env::var("WATERUI_INSPECTOR_PORT_START") {
                cfg.port_start = port_start
                    .parse()
                    .map_err(|_| ConfigError::InvalidPortStart)?;
            }
            if let Ok(port_range) = std::env::var("WATERUI_INSPECTOR_PORT_RANGE") {
                cfg.port_range = port_range
                    .parse()
                    .map_err(|_| ConfigError::InvalidPortRange)?;
            }

            Ok(cfg)
        }

        /// Inclusive port range to scan/bind.
        #[must_use]
        pub const fn ports(&self) -> RangeInclusive<u16> {
            let end = self
                .port_start
                .saturating_add(self.port_range.saturating_sub(1));
            self.port_start..=end
        }
    }

    /// Env config errors.
    #[derive(Debug, Error)]
    pub enum ConfigError {
        /// Host env var is invalid.
        #[error("invalid WATERUI_INSPECTOR_HOST")]
        InvalidHost,
        /// Port start env var is invalid.
        #[error("invalid WATERUI_INSPECTOR_PORT_START")]
        InvalidPortStart,
        /// Port range env var is invalid.
        #[error("invalid WATERUI_INSPECTOR_PORT_RANGE")]
        InvalidPortRange,
    }
}

/// Runtime -> inspector handshake/init messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InspectorClientMessage {
    /// Authenticate and optionally request replay from a sequence number.
    Hello {
        /// One-time session token generated by CLI/runtime.
        token: String,
        /// Replay events strictly after this sequence number.
        from_seq: Option<u64>,
    },
    /// Liveness ping.
    Ping,
}

/// Runtime -> inspector server messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InspectorServerMessage {
    /// Handshake ack.
    HelloAck {
        /// Protocol metadata.
        protocol: InspectorProtocolInfo,
        /// Runtime process id.
        app_pid: u32,
    },
    /// One runtime event.
    Event {
        /// Sequenced event envelope.
        envelope: InspectorEventEnvelope,
    },
    /// Liveness response.
    Pong,
    /// Non-fatal protocol error.
    Error {
        /// Human-readable error message.
        message: String,
    },
}

/// Sequenced event envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectorEventEnvelope {
    /// Monotonic sequence id for replay/resume.
    pub seq: u64,
    /// UTC unix timestamp in milliseconds.
    pub timestamp_unix_ms: u64,
    /// Event payload.
    pub event: InspectorEvent,
}

/// Typed runtime events exposed to Inspector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InspectorEvent {
    /// Per-poll runtime sample.
    RuntimePoll(RuntimePollEvent),
    /// Warning-level stall sample (derived from poll data).
    MainThreadStall(MainThreadStallEvent),
}

/// Poll-level main-thread sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePollEvent {
    /// Future type name.
    pub task_type: String,
    /// Whether this poll completed the future.
    pub poll_ready: bool,
    /// Wall time spent in poll.
    pub wall_us: u64,
    /// CPU time spent in poll.
    pub cpu_us: u64,
    /// Frame budget used for thresholding.
    pub budget_us: u64,
    /// Display refresh rate used for budget derivation.
    pub refresh_hz: f64,
}

/// Stall event (warn threshold reached).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainThreadStallEvent {
    /// Future type name.
    pub task_type: String,
    /// Whether this poll completed the future.
    pub poll_ready: bool,
    /// Wall time spent in poll.
    pub wall_us: u64,
    /// CPU time spent in poll.
    pub cpu_us: u64,
    /// Frame budget used for thresholding.
    pub budget_us: u64,
    /// Overrun in microseconds.
    pub overrun_us: u64,
    /// Percent of frame budget used.
    pub usage_pct: f64,
    /// Optional captured backtrace summary string.
    pub backtrace: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_blocking() {
        let mut bytes = Vec::new();
        let value = InspectorServerMessage::Pong;
        transport::write_frame_blocking(&mut bytes, &value).unwrap();
        let decoded: InspectorServerMessage =
            transport::read_frame_blocking(&mut bytes.as_slice()).unwrap();
        match decoded {
            InspectorServerMessage::Pong => {}
            other => panic!("unexpected decoded frame: {other:?}"),
        }
    }
}
