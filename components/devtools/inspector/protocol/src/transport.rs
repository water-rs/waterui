//! Framed binary transport.
//!
//! Message format: `u32::to_be_bytes(len)` followed by `len` bytes of bincode
//! payload. Both an async and a blocking pair are provided because the target
//! application serves the socket from a dedicated thread while the inspector
//! app reads it from its UI executor.

use std::io;

use futures_lite::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Size of the length prefix on every frame.
pub const LEN_PREFIX_BYTES: usize = 4;

/// Environment variable overriding the frame size cap.
const MAX_FRAME_ENV: &str = "WATERUI_INSPECTOR_MAX_FRAME_BYTES";

/// Hard cap on a single frame.
#[must_use]
pub fn max_frame_bytes() -> usize {
    const DEFAULT: usize = 16 * 1024 * 1024;
    std::env::var(MAX_FRAME_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT)
}

fn encode<T: Serialize + ?Sized>(value: &T) -> io::Result<Vec<u8>> {
    let config = bincode::config::standard();
    bincode::serde::encode_to_vec(value, config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn length_prefix(data: &[u8]) -> io::Result<[u8; LEN_PREFIX_BYTES]> {
    let len: u32 = data.len().try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "inspector frame too large for u32 length",
        )
    })?;
    Ok(len.to_be_bytes())
}

fn decode<T: DeserializeOwned>(buffer: &[u8]) -> io::Result<T> {
    let config = bincode::config::standard();
    let (value, bytes_read): (T, usize) = bincode::serde::decode_from_slice(buffer, config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes_read != buffer.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trailing bytes after inspector frame payload",
        ));
    }
    Ok(value)
}

fn check_frame_length(len: usize) -> io::Result<()> {
    let max = max_frame_bytes();
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("inspector frame too large: {len} bytes (max {max})"),
        ));
    }
    Ok(())
}

/// Reads one framed message from an async reader.
///
/// # Errors
///
/// Returns `InvalidData` when the frame is too large, malformed, or has trailing
/// bytes after decoding. Propagates I/O failures from the reader.
pub async fn read_frame<R, T>(reader: &mut R) -> io::Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; LEN_PREFIX_BYTES];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    check_frame_length(len)?;

    let mut buffer = vec![0u8; len];
    reader.read_exact(&mut buffer).await?;
    decode(&buffer)
}

/// Writes one framed message to an async writer.
///
/// # Errors
///
/// Returns `InvalidData` when the payload exceeds the frame size limit or cannot
/// be serialized. Propagates I/O failures from the writer.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin + Send,
    T: Serialize + Sync,
{
    let data = encode(value)?;
    let prefix = length_prefix(&data)?;
    writer.write_all(&prefix).await?;
    writer.write_all(&data).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one framed message from a blocking reader.
///
/// # Errors
///
/// Returns `InvalidData` when the frame is too large, malformed, or has trailing
/// bytes after decoding. Propagates I/O failures from the reader.
pub fn read_frame_blocking<R, T>(reader: &mut R) -> io::Result<T>
where
    R: io::Read,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; LEN_PREFIX_BYTES];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    check_frame_length(len)?;

    let mut buffer = vec![0u8; len];
    reader.read_exact(&mut buffer)?;
    decode(&buffer)
}

/// Writes one framed message to a blocking writer.
///
/// # Errors
///
/// Returns `InvalidData` when the payload exceeds the frame size limit or cannot
/// be serialized. Propagates I/O failures from the writer.
pub fn write_frame_blocking<W, T>(writer: &mut W, value: &T) -> io::Result<()>
where
    W: io::Write,
    T: Serialize,
{
    let data = encode(value)?;
    let prefix = length_prefix(&data)?;
    writer.write_all(&prefix)?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}
