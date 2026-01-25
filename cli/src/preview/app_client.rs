//! TCP client for communicating with the preview app.
//!
//! The preview app runs a TCP server on ports 2006+. This client connects
//! to it and sends render requests.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use color_eyre::eyre::{Context, Result, bail};

use super::protocol::{AppRequest, AppResponse, DylibSource, Size};

/// Starting port for preview app TCP server.
/// Keep this distinct from the hot reload port range (2006+).
const PORT_START: u16 = 2106;
/// Number of ports to try.
const PORT_RANGE: u16 = 50;

/// TCP client for the preview app.
#[derive(Debug)]
pub struct PreviewAppClient {
    stream: TcpStream,
    /// Whether we've sent a dylib to this app (for caching).
    has_sent_dylib: bool,
}

impl PreviewAppClient {
    /// Try to connect to a running preview app.
    ///
    /// Scans ports 2106-2155 for a listening preview app.
    ///
    /// # Errors
    /// Returns an error if no preview app is found.
    pub fn connect() -> Result<Self> {
        for port in PORT_START..(PORT_START + PORT_RANGE) {
            if let Ok(stream) = TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}").parse().unwrap(),
                Duration::from_millis(100),
            ) {
                tracing::info!("Connected to preview app on port {port}");

                // Disable Nagle's algorithm for faster small writes
                let _ = stream.set_nodelay(true);

                // Longer timeouts to handle serialized requests when multiple
                // clients are waiting in queue
                stream.set_read_timeout(Some(Duration::from_secs(120)))?;
                stream.set_write_timeout(Some(Duration::from_secs(120)))?;

                return Ok(Self {
                    stream,
                    has_sent_dylib: false,
                });
            }
        }

        bail!(
            "Could not connect to preview app. Make sure it is running.\n\
             The preview app listens on ports {PORT_START}-{}.",
            PORT_START + PORT_RANGE - 1
        )
    }

    /// Render a view and return PNG bytes.
    ///
    /// # Arguments
    /// * `dylib_data` - The dylib bytes (only sent if changed)
    /// * `needs_reload` - Whether the dylib has changed since last request
    /// * `symbol` - The preview symbol name
    /// * `width` - Frame width in points
    /// * `height` - Frame height in points
    ///
    /// # Errors
    /// Returns an error if rendering fails.
    pub fn render(
        &mut self,
        dylib_data: &[u8],
        needs_reload: bool,
        symbol: &str,
        width: f32,
        height: f32,
    ) -> Result<Vec<u8>> {
        let dylib = if needs_reload || !self.has_sent_dylib {
            self.has_sent_dylib = true;
            DylibSource::Bytes(dylib_data.to_vec())
        } else {
            DylibSource::Cached
        };

        let request = AppRequest::Render {
            dylib,
            symbol: symbol.to_string(),
            frame: Size::new(width, height),
        };

        self.send_request(&request)?;
        let response = self.recv_response()?;

        match response {
            Ok(output) => Ok(output.png_data),
            Err(e) => bail!("Preview app error: {e}"),
        }
    }

    /// Send a request to the preview app.
    fn send_request(&mut self, request: &AppRequest) -> Result<()> {
        let data = serde_json::to_vec(request)?;
        let len = (data.len() as u32).to_be_bytes();

        self.stream.write_all(&len)?;
        self.stream.write_all(&data)?;
        self.stream.flush()?;

        Ok(())
    }

    /// Receive a response from the preview app.
    fn recv_response(&mut self) -> Result<AppResponse> {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        self.stream
            .read_exact(&mut len_buf)
            .wrap_err("Failed to read response length")?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read response body
        let mut buf = vec![0u8; len];
        self.stream
            .read_exact(&mut buf)
            .wrap_err("Failed to read response body")?;

        let response: AppResponse =
            serde_json::from_slice(&buf).wrap_err("Failed to parse response")?;
        Ok(response)
    }

    /// Shutdown the preview app.
    ///
    /// # Errors
    /// Returns an error if the shutdown request fails.
    pub fn shutdown(&mut self) -> Result<()> {
        self.send_request(&AppRequest::Shutdown)?;
        // Read and discard response to allow graceful shutdown.
        let _ = self.recv_response();
        Ok(())
    }
}
