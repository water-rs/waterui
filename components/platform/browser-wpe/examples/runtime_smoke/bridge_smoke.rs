use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use suiteki::Str;
use waterui_browser_wpe::WpeRuntime;
use waterui_url::Url;
use waterui_webview::{BridgeOrigins, JsReply, OriginPolicy};

use super::executor::{SmokeExecutor, SmokePage};

const IO_TIMEOUT: Duration = Duration::from_secs(10);

struct ServerRequest {
    path: String,
    tag: Option<String>,
}

struct ResponseGate {
    released: Mutex<bool>,
    changed: Condvar,
}

impl ResponseGate {
    const fn new() -> Self {
        Self {
            released: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    fn block_response(&self) {
        let mut released = self.released.lock().expect("response gate lock");
        while !*released {
            released = self.changed.wait(released).expect("response gate wait");
        }
        drop(released);
    }

    fn release(&self) {
        *self.released.lock().expect("response gate lock") = true;
        self.changed.notify_all();
    }
}

struct LocalHttpServer {
    address: SocketAddr,
    events: Receiver<ServerRequest>,
    backlog: Vec<ServerRequest>,
    shutdown: Arc<AtomicBool>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
    listener: Option<JoinHandle<()>>,
    response_gate: Arc<ResponseGate>,
}

impl LocalHttpServer {
    fn new() -> Self {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("bind local WPE smoke HTTP server");
        let address = listener.local_addr().expect("HTTP server address");
        let (sender, events) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let workers = Arc::new(Mutex::new(Vec::new()));
        let response_gate = Arc::new(ResponseGate::new());

        let listener_shutdown = Arc::clone(&shutdown);
        let listener_workers = Arc::clone(&workers);
        let listener_gate = Arc::clone(&response_gate);
        let listener = thread::spawn(move || {
            loop {
                if listener_shutdown.load(Ordering::Acquire) {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        let sender = sender.clone();
                        let gate = Arc::clone(&listener_gate);
                        let worker = thread::spawn(move || {
                            serve_request(stream, &sender, &gate);
                        });
                        listener_workers
                            .lock()
                            .expect("HTTP worker list lock")
                            .push(worker);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });

        Self {
            address,
            events,
            backlog: Vec::new(),
            shutdown,
            workers,
            listener: Some(listener),
            response_gate,
        }
    }

    const fn port(&self) -> u16 {
        self.address.port()
    }

    fn take_matching(
        &mut self,
        predicate: &impl Fn(&ServerRequest) -> bool,
    ) -> Option<ServerRequest> {
        if let Some(index) = self.backlog.iter().position(predicate) {
            return Some(self.backlog.remove(index));
        }
        loop {
            match self.events.try_recv() {
                Ok(request) if predicate(&request) => return Some(request),
                Ok(request) => self.backlog.push(request),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    panic!("local WPE smoke HTTP server stopped unexpectedly");
                }
            }
        }
    }

    fn wait_for(
        &mut self,
        page: &SmokePage,
        predicate: impl Fn(&ServerRequest) -> bool,
        deadline: Instant,
        purpose: &str,
    ) -> ServerRequest {
        loop {
            page.pump();
            if let Some(request) = self.take_matching(&predicate) {
                return request;
            }
            assert!(
                Instant::now() < deadline,
                "WPE bridge smoke timed out waiting for HTTP evidence: {purpose}"
            );
            thread::yield_now();
        }
    }

    fn wait_for_tag(&mut self, page: &SmokePage, tag: &str, deadline: Instant) -> ServerRequest {
        let tag = tag.to_owned();
        let purpose = format!("marker {tag}");
        self.wait_for(
            page,
            move |request| request.tag.as_deref() == Some(&tag),
            deadline,
            &purpose,
        )
    }
}

impl Drop for LocalHttpServer {
    fn drop(&mut self) {
        self.response_gate.release();
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(listener) = self.listener.take() {
            listener
                .join()
                .expect("local WPE smoke HTTP listener thread");
        }
        for worker in self
            .workers
            .lock()
            .expect("HTTP worker list lock")
            .drain(..)
        {
            worker.join().expect("local WPE smoke HTTP worker thread");
        }
    }
}

fn serve_request(
    mut stream: TcpStream,
    events: &Sender<ServerRequest>,
    response_gate: &ResponseGate,
) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let path = target.split('?').next().unwrap_or("/").to_owned();
    loop {
        line.clear();
        if reader.read_line(&mut line).is_err() || line.is_empty() || line == "\r\n" {
            break;
        }
    }
    let tag = target
        .split_once('?')
        .and_then(|(_, query)| {
            query
                .split('&')
                .find_map(|parameter| parameter.strip_prefix("tag="))
        })
        .map(str::to_owned);
    let _ = events.send(ServerRequest {
        path: path.clone(),
        tag,
    });
    if path == "/hold-a" {
        response_gate.block_response();
    }

    let body = match path.as_str() {
        "/a" | "/e" | "/hold-a" => {
            "<!doctype html><meta charset=utf-8><title>WPE bridge smoke</title><body>ready</body>"
        }
        _ => "ok",
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream = reader.into_inner();
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Installs the WPE transport and the shared bridge script, as the WPE web view
/// controller does.
pub fn install_bridge_scripts(page: &SmokePage) {
    page.add_script(
        "waterui:wpe-transport",
        include_str!("../../src/transport.js"),
        false,
    );
    page.add_script(
        "waterui:bridge",
        waterui_webview::DOCUMENT_START_SCRIPT,
        false,
    );
}

fn install_bridge(page: &SmokePage) {
    install_bridge_scripts(page);
    page.add_script(
        "waterui:wpe-smoke-observer",
        r#"
          (() => {
            const site = location.hostname === "localhost" ? "E" : "A";
            const route = location.pathname.slice(1) || "root";
            const tag = `loaded-${site}-${route}`;
            window.postMessage({source: "waterui-wpe-smoke", tag}, "*");
            fetch(`/marker?tag=${tag}`, {cache: "no-store"}).catch(() => {});
          })();
        "#,
        false,
    );
}

fn tagged_payload(payload: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|payload| {
            payload
                .get("tag")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "invalid".to_owned())
}

fn install_probe_handler(page: &SmokePage, calls: Arc<Mutex<Vec<String>>>) {
    page.add_handler(
        "probe",
        Box::new(move |payload| {
            let tag = tagged_payload(payload);
            calls
                .lock()
                .expect("handler call list lock")
                .push(tag.clone());
            let reply = serde_json::json!({"tag": tag}).to_string().into_bytes();
            Box::pin(async move { Ok(JsReply::Json(reply)) })
        }),
    );
}

/// Runs `body` as an async page function and returns its JSON result.
pub fn await_page_script(page: &SmokePage, body: &str, deadline: Instant) -> serde_json::Value {
    let script_page = page.clone();
    let body = body.to_owned();
    let result = page
        .block_on(
            async move { script_page.call_async_javascript(&body).await },
            deadline,
            "a page script",
        )
        .unwrap_or_else(|error| panic!("WPE page script failed: {error}"));
    serde_json::from_str(result.as_str())
        .unwrap_or_else(|error| panic!("WPE page script returned invalid JSON: {error}"))
}

fn await_load(
    page: &SmokePage,
    server: &mut LocalHttpServer,
    url: &str,
    expected_marker: &str,
    deadline: Instant,
) {
    page.load(url, deadline);
    server.wait_for_tag(page, expected_marker, deadline);
}

fn await_process_identifier(page: &SmokePage, deadline: Instant) -> String {
    let identified_page = page.clone();
    page.block_on(
        async move { identified_page.web_process_identifier().await },
        deadline,
        "the WebProcess identifier",
    )
    .unwrap_or_else(|error| panic!("WPE process identifier query failed: {error}"))
}

fn await_channel<T>(
    page: &SmokePage,
    receiver: &Receiver<T>,
    deadline: Instant,
    purpose: &str,
) -> T {
    loop {
        page.pump();
        match receiver.try_recv() {
            Ok(value) => return value,
            Err(TryRecvError::Disconnected) => {
                panic!("WPE bridge smoke channel closed before {purpose}");
            }
            Err(TryRecvError::Empty) => {}
        }
        assert!(
            Instant::now() < deadline,
            "WPE bridge smoke timed out waiting for {purpose}"
        );
        thread::yield_now();
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "keeps the cross-origin security scenario sequence explicit"
)]
pub fn run(runtime: &WpeRuntime, executor: &SmokeExecutor, deadline: Instant) {
    let mut server = LocalHttpServer::new();
    let port = server.port();
    let admitted_origin = format!("http://127.0.0.1:{port}");
    let admitted_url = format!("{admitted_origin}/hold-a");
    let excluded_url = format!("http://localhost:{port}/e");
    let initial_url = admitted_url
        .parse::<Url>()
        .expect("parse admitted WPE smoke URL");
    let page = SmokePage::new(runtime.clone(), executor);
    page.set_bridge_origins(OriginPolicy::new(
        BridgeOrigins::Allowed(vec![Str::from(admitted_origin)]),
        &initial_url,
    ));
    install_bridge(&page);

    let handler_calls = Arc::new(Mutex::new(Vec::<String>::new()));
    install_probe_handler(&page, Arc::clone(&handler_calls));

    let (release_reply, reply_gate) = oneshot::channel::<()>();
    let reply_gate = Rc::new(RefCell::new(Some(reply_gate)));
    let (handler_entered, handler_entries) = mpsc::channel::<String>();
    let (handler_completed, handler_completions) = mpsc::channel::<()>();
    let completion = Rc::new(RefCell::new(Some(handler_completed)));
    let delayed_handler_calls = Arc::clone(&handler_calls);
    page.add_handler(
        "delayed",
        Box::new(move |payload| {
            let tag = tagged_payload(payload);
            delayed_handler_calls
                .lock()
                .expect("handler call list lock")
                .push(tag.clone());
            handler_entered
                .send(tag.clone())
                .expect("smoke handler entry receiver");
            let gate = reply_gate
                .borrow_mut()
                .take()
                .expect("delayed handler is invoked once");
            let completion = completion
                .borrow_mut()
                .take()
                .expect("delayed handler completes once");
            let reply = serde_json::json!({"tag": tag}).to_string().into_bytes();
            Box::pin(async move {
                let _ = gate.await;
                completion
                    .send(())
                    .expect("delayed handler completion receiver");
                Ok(JsReply::Json(reply))
            })
        }),
    );

    await_load(&page, &mut server, &excluded_url, "loaded-E-e", deadline);
    let excluded = await_page_script(
        &page,
        r#"
          let result = "resolved";
          try {
            await waterui.invoke("probe", {tag: "E-loaded"});
          } catch {
            result = "rejected";
          }
          await fetch("/marker?tag=E-loaded-rejected", {cache: "no-store"});
          history.replaceState({}, "", `${location.pathname}?same-document=1`);
          location.hash = "same-document";
          let historyResult = "resolved";
          try {
            await waterui.invoke("probe", {tag: "E-after-history"});
          } catch {
            historyResult = "rejected";
          }
          await fetch("/marker?tag=E-after-history-rejected", {cache: "no-store"});
          return {result, historyResult};
        "#,
        deadline,
    );
    assert_eq!(excluded["result"], "rejected");
    assert_eq!(excluded["historyResult"], "rejected");
    server.wait_for_tag(&page, "E-loaded-rejected", deadline);
    server.wait_for_tag(&page, "E-after-history-rejected", deadline);
    assert!(
        handler_calls
            .lock()
            .expect("handler call list lock")
            .is_empty(),
        "excluded E reached a Rust bridge handler"
    );

    let excluded_process = await_process_identifier(&page, deadline);
    assert_eq!(excluded_process, await_process_identifier(&page, deadline));
    page.load_uri(&admitted_url);
    server.wait_for(
        &page,
        |request| request.path == "/hold-a",
        deadline,
        "held admitted A response",
    );
    let provisional = await_page_script(
        &page,
        r#"
          const origin = location.origin;
          let result = "resolved";
          try {
            await waterui.invoke("probe", {tag: "E-provisional"});
          } catch {
            result = "rejected";
          }
          await fetch(`/marker?tag=E-provisional-${result}`, {cache: "no-store"});
          await fetch("/marker?tag=E-provisional-attempted", {cache: "no-store"});
          return {origin, result};
        "#,
        deadline,
    );
    assert_eq!(provisional["origin"], format!("http://localhost:{port}"));
    assert_eq!(provisional["result"], "rejected");
    let provisional_outcome = server.wait_for(
        &page,
        |request| {
            matches!(
                request.tag.as_deref(),
                Some("E-provisional-rejected" | "E-provisional-resolved")
            )
        },
        deadline,
        "E provisional bridge rejection",
    );
    assert_eq!(
        provisional_outcome.tag.as_deref(),
        Some("E-provisional-rejected"),
        "excluded E resolved its provisional bridge call"
    );
    server.wait_for_tag(&page, "E-provisional-attempted", deadline);
    server.response_gate.release();
    server.wait_for_tag(&page, "loaded-A-hold-a", deadline);

    let admitted_process = await_process_identifier(&page, deadline);
    assert_ne!(
        excluded_process, admitted_process,
        "localhost to 127.0.0.1 committed without a WebProcess swap"
    );
    let admitted = await_page_script(
        &page,
        r#"
          const value = await waterui.invoke("probe", {tag: "A-after-provisional"});
          return {tag: value.tag};
        "#,
        deadline,
    );
    assert_eq!(admitted["tag"], "A-after-provisional");

    let same_document = await_page_script(
        &page,
        r#"
          history.replaceState({}, "", `${location.pathname}?same-document=1`);
          location.hash = "same-document";
          const value = await waterui.invoke("probe", {tag: "A-same-document"});
          return {tag: value.tag};
        "#,
        deadline,
    );
    assert_eq!(same_document["tag"], "A-same-document");
    assert_eq!(admitted_process, await_process_identifier(&page, deadline));

    let delayed_started = await_page_script(
        &page,
        r#"
          globalThis.__wateruiDelayedReply =
            waterui.invoke("delayed", {tag: "A-delayed"});
          return {started: true};
        "#,
        deadline,
    );
    assert_eq!(delayed_started["started"], true);
    assert_eq!(
        await_channel(&page, &handler_entries, deadline, "delayed handler entry"),
        "A-delayed"
    );

    await_load(&page, &mut server, &excluded_url, "loaded-E-e", deadline);
    let excluded_after_swap = await_process_identifier(&page, deadline);
    assert_ne!(
        admitted_process, excluded_after_swap,
        "127.0.0.1 to localhost committed without a WebProcess swap"
    );
    let rejected_after_swap = await_page_script(
        &page,
        r#"
          const originalResolve = globalThis.__wateruiResolve;
          globalThis.__wateruiSpoofResolverCalls = 0;
          globalThis.__wateruiResolve = function (id, ok, payload) {
            globalThis.__wateruiSpoofResolverCalls += 1;
            fetch("/marker?tag=spoof-resolver-invoked", {cache: "no-store"}).catch(() => {});
            return originalResolve(id, ok, payload);
          };
          let result = "resolved";
          try {
            await waterui.invoke("probe", {tag: "E-after-navigation"});
          } catch {
            result = "rejected";
          }
          await fetch("/marker?tag=E-after-navigation-rejected", {cache: "no-store"});
          return {result, calls: globalThis.__wateruiSpoofResolverCalls};
        "#,
        deadline,
    );
    assert_eq!(rejected_after_swap["result"], "rejected");
    assert_eq!(rejected_after_swap["calls"], 1);
    server.wait_for_tag(&page, "spoof-resolver-invoked", deadline);
    server.wait_for_tag(&page, "E-after-navigation-rejected", deadline);

    release_reply
        .send(())
        .expect("delayed WPE bridge reply receiver");
    await_channel(
        &page,
        &handler_completions,
        deadline,
        "delayed Rust handler completion",
    );
    let after_reply_roundtrip = await_page_script(
        &page,
        r#"
          await fetch("/marker?tag=E-after-reply-roundtrip", {cache: "no-store"});
          return {
            host: location.host,
            calls: globalThis.__wateruiSpoofResolverCalls
          };
        "#,
        deadline,
    );
    assert_eq!(after_reply_roundtrip["host"], format!("localhost:{port}"));
    assert_eq!(
        after_reply_roundtrip["calls"], 1,
        "the A reply invoked E's replacement resolver"
    );
    server.wait_for_tag(&page, "E-after-reply-roundtrip", deadline);

    let calls = handler_calls
        .lock()
        .expect("handler call list lock")
        .clone();
    assert_eq!(
        calls
            .iter()
            .filter(|tag| tag.as_str() == "A-after-provisional")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|tag| tag.as_str() == "A-same-document")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|tag| tag.as_str() == "A-delayed")
            .count(),
        1
    );
    assert!(
        !calls.iter().any(|tag| tag.starts_with("E-")),
        "an excluded or provisional E request reached a Rust handler: {calls:?}"
    );
    tracing::info!(
        "PASS excluded document: bridge rejection before and after same-document URI change"
    );
    tracing::info!("PASS E->A provisional navigation: E rejected while A response is held");
    tracing::info!("PASS E->A process-swap commit: distinct WebProcess identifiers");
    tracing::info!("PASS admitted-page bridge call and reply after E->A commit");
    tracing::info!("PASS admitted document: bridge call and reply after same-document URI change");
    tracing::info!("PASS A->E navigation: process swap and excluded-document rejection");
    tracing::info!(
        "PASS document-bound reply: delayed A reply cannot invoke E's replacement resolver"
    );
    tracing::info!("PASS handler audit: no excluded-document request reached Rust");
}
