//! A one-thread HTTP/1.1 server for the fetch tests: canned bodies by path,
//! every request reported on a channel.
//!
//! The update client is the real `zenwave` client over a real socket; what
//! the tests need is a server whose responses they choose and whose request
//! log they can read. That is a listener on a loopback port, a thread that
//! answers each connection once, and a channel the thread sends every
//! request path down. Dropping the server stops it: the thread is told to
//! stop, woken by one connection from the drop itself, and joined, so the
//! port is free and the thread gone before the test returns.
//!
//! Included with `#[path]` from the test that needs it.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

/// How a response says where its body ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// A `Content-Length` header, as a real server sends one.
    ContentLength,
    /// No length at all: the body runs until the server closes the
    /// connection. What a client sees from a server that streams without
    /// knowing the size — and the one way to hand it more bytes than a
    /// header promised.
    CloseDelimited,
}

/// A server answering `GET` for a fixed set of paths.
pub struct Server {
    address: SocketAddr,
    requests: mpsc::Receiver<String>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Serves `routes`, a map of request path to response body, on a fresh
    /// loopback port, each body framed as `framing` says. Any other path is
    /// a 404.
    ///
    /// # Panics
    ///
    /// Panics when no loopback port can be bound.
    pub fn serve(routes: BTreeMap<String, Vec<u8>>, framing: Framing) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port binds");
        let address = listener.local_addr().expect("the listener has an address");
        let (sender, requests) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let routes = Arc::new(routes);
        let thread = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    // Each connection is answered on its own thread, so a
                    // client that connects and never writes cannot hold the
                    // accept loop — and with it `Drop`'s join — hostage.
                    std::thread::spawn({
                        let routes = Arc::clone(&routes);
                        let sender = sender.clone();
                        move || answer(stream, &routes, framing, &sender)
                    });
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// The URL of `path` on this server.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{path}", self.address)
    }

    /// Every path requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests.try_iter().collect()
    }
}

impl Drop for Server {
    /// Stops the thread and waits for it: the stop flag is raised, one
    /// connection wakes the accept loop so it sees the flag, and the thread
    /// is joined.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // The accept loop checks the flag once per connection; this is that
        // connection. A refused connect means the loop is already gone.
        drop(TcpStream::connect(self.address));
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the server thread does not panic");
        }
    }
}

/// A URL on a loopback port nothing listens on: bound to learn a free port,
/// then released. Connecting to it is refused.
pub fn unreachable_url(path: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port binds");
    let address = listener.local_addr().expect("the listener has an address");
    drop(listener);
    format!("http://{address}/{path}")
}

/// Reads one request off `stream`, answers it, and closes.
fn answer(
    mut stream: TcpStream,
    routes: &BTreeMap<String, Vec<u8>>,
    framing: Framing,
    log: &mpsc::Sender<String>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("the stream clones"));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    // Drain the headers; the body is never needed for a GET.
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) => break,
            Ok(_) if header == "\r\n" || header == "\n" => break,
            Ok(_) => {}
            Err(_) => return,
        }
    }
    // The receiver may be gone when the request arrives after the test's
    // `Server` was dropped; a request nobody will read is not an error.
    drop(log.send(path.clone()));
    let response = routes.get(&path).map_or_else(
        || b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        |body| {
            let head = match framing {
                Framing::ContentLength => format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ),
                Framing::CloseDelimited => {
                    String::from("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                }
            };
            let mut response = head.into_bytes();
            response.extend_from_slice(body);
            response
        },
    );
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}
