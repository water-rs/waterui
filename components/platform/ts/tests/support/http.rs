//! A one-thread HTTP/1.1 server for the fetch tests: canned bodies by path,
//! every request reported on a channel.
//!
//! The update client is the real `zenwave` client over a real socket; what
//! the tests need is a server whose responses they choose and whose request
//! log they can read. That is a listener on a loopback port, a thread that
//! answers each connection once, and a channel the thread sends every
//! request path down. The thread lives until the test process exits, which
//! for a nextest process is the end of the test.
//!
//! Included with `#[path]` from the test that needs it.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

/// A server answering `GET` for a fixed set of paths.
pub struct Server {
    base: String,
    requests: mpsc::Receiver<String>,
}

impl Server {
    /// Serves `routes`, a map of request path to response body, on a fresh
    /// loopback port. Any other path is a 404.
    ///
    /// # Panics
    ///
    /// Panics when no loopback port can be bound.
    pub fn serve(routes: BTreeMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port binds");
        let address = listener.local_addr().expect("the listener has an address");
        let (sender, requests) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                answer(stream, &routes, &sender);
            }
        });
        Self {
            base: format!("http://{address}/"),
            requests,
        }
    }

    /// The URL of `path` on this server.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Every path requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests.try_iter().collect()
    }
}

/// Reads one request off `stream`, answers it, and closes.
fn answer(mut stream: TcpStream, routes: &BTreeMap<String, Vec<u8>>, log: &mpsc::Sender<String>) {
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
    log.send(path.clone()).expect("the test holds the receiver");
    let response = routes.get(&path).map_or_else(
        || b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        |body| {
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(body);
            response
        },
    );
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}
