//! Async TCP server for preview daemon.
//!
//! Handles incoming connections from the CLI and processes preview requests.

use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::net::TcpListener;
use std::pin::Pin;
use std::rc::Rc;

use async_io::Async;
use executor_core::spawn_local;
use futures_lite::io::{AsyncReadExt, AsyncWriteExt};

use crate::protocol::{PreviewRequest, PreviewResponse};

/// Starting port for preview daemon.
const PORT_START: u16 = 2006;
/// Number of ports to try before giving up.
const PORT_RANGE: u16 = 50;

/// Async callback type for handling preview requests.
pub type RequestHandler =
    Rc<dyn Fn(PreviewRequest) -> Pin<Box<dyn Future<Output = PreviewResponse> + 'static>>>;

/// Async TCP server for receiving preview requests from the CLI.
#[derive(Debug)]
pub struct TcpServer {
    /// Port the server is listening on.
    port: u16,
}

impl TcpServer {
    /// Bind to a port starting from `PORT_START`, trying subsequent ports if busy.
    ///
    /// Spawns an async accept loop using `spawn_local` to avoid thread-safety issues.
    ///
    /// # Panics
    ///
    /// Panics if no port in the range is available.
    #[must_use]
    pub fn bind(handler: RequestHandler) -> Self {
        Self::bind_starting_from(PORT_START, handler)
    }

    /// Bind to a port starting from the given port, trying subsequent ports if busy.
    ///
    /// # Panics
    ///
    /// Panics if no port in the range is available.
    #[must_use]
    pub fn bind_starting_from(start_port: u16, handler: RequestHandler) -> Self {
        for port in start_port..(start_port + PORT_RANGE) {
            match TcpListener::bind(("127.0.0.1", port)) {
                Ok(listener) => {
                    tracing::info!("Preview daemon listening on port {port}");

                    // Wrap in Async for non-blocking I/O
                    let async_listener = Async::new(listener)
                        .expect("Failed to create async listener");

                    // Spawn async accept loop
                    let handler = handler.clone();
                    spawn_local(async move {
                        accept_loop(async_listener, handler).await;
                    })
                    .detach();

                    return Self { port };
                }
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                    tracing::debug!("Port {port} in use, trying next");
                    continue;
                }
                Err(e) => {
                    tracing::warn!("Failed to bind to port {port}: {e}");
                    continue;
                }
            }
        }

        panic!(
            "Failed to bind TCP server to any port in range {start_port}..{}",
            start_port + PORT_RANGE
        );
    }

    /// Get the port the server is listening on.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

/// Async accept loop for incoming TCP connections.
async fn accept_loop(listener: Async<TcpListener>, handler: RequestHandler) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                tracing::debug!("Accepted connection from {addr}");
                let handler = handler.clone();

                // Handle each connection in its own spawned task
                spawn_local(async move {
                    if let Err(e) = handle_connection(stream, handler).await {
                        tracing::error!("Connection error: {e}");
                    }
                })
                .detach();
            }
            Err(e) => {
                tracing::error!("Accept error: {e}");
            }
        }
    }
}

/// Handle a single TCP connection asynchronously.
async fn handle_connection(
    stream: Async<std::net::TcpStream>,
    handler: RequestHandler,
) -> io::Result<()> {
    // Use a RefCell to allow borrowing both reader and writer
    let stream = RefCell::new(stream);

    // Read length-prefixed message
    let mut len_buf = [0u8; 4];
    stream.borrow_mut().read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    // Read message body
    let mut buf = vec![0u8; len];
    stream.borrow_mut().read_exact(&mut buf).await?;

    // Deserialize request
    let request: PreviewRequest = serde_json::from_slice(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    tracing::debug!("Received request: {request:?}");

    // Handle request using the provided async handler
    let response = handler(request).await;

    // Serialize response
    let response_buf = serde_json::to_vec(&response)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // Write length-prefixed response
    let mut stream = stream.borrow_mut();
    stream.write_all(&(response_buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&response_buf).await?;
    stream.flush().await?;

    tracing::debug!("Sent response");

    Ok(())
}
