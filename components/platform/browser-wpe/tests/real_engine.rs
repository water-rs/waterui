//! What only a real engine can prove.
//!
//! These tests drive a genuine WPE `WebKit` runtime through the same handle
//! `WpeController::open` hands the renderer, load pages over a local HTTP
//! server, and assert on what crosses the bridge in both directions.
//!
//! They exist because three total breaks of the web view bridge shipped behind
//! a fully green Rust suite: every reply crossing as base64, a frozen `waterui`
//! object that made `waterui.state` and `waterui.watch` throw on install, and
//! integers past 2^53 losing their low bits in both directions. None of the
//! three is visible to a test that stops at the Rust side of the boundary.
//!
//! # Running them
//!
//! They are behind the `real-engine` feature, so an ordinary `cargo nextest run`
//! neither builds nor reports them — a test that cannot run must not look like
//! one that passed. With the feature on, a staged runtime is mandatory and its
//! absence is a loud failure, never a skip:
//!
//! ```sh
//! components/platform/browser-wpe/runtime/build-runtime.sh wpe-artifacts
//! unzip -q wpe-artifacts/waterui-wpe-*.zip -d wpe-runtime
//! WATERUI_WPE_RUNTIME="$PWD/wpe-runtime" \
//!     cargo nextest run -p waterui-browser-wpe --features real-engine
//! ```
//!
//! `build-runtime.sh` compiles WPE `WebKit` from the pinned source tarball,
//! which takes hours, and it discards its own build tree — the packaged archive
//! is what it leaves behind. Unpacking that archive is also what `water package`
//! does, so `WATERUI_WPE_RUNTIME` names exactly the layout
//! [`WpeRuntimePaths::packaged`] resolves next to a shipped executable.
#![cfg(target_os = "linux")]

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use executor_core::LocalExecutor;
use executor_core::async_executor::AsyncLocalExecutor;
use serde_json::Value;
use tiny_http::{Header, Response, Server};
use waterui_browser_wpe::{WpePage, WpeRuntime, WpeRuntimePaths, WpeWebViewHandle};
use waterui_url::Url;
use waterui_webview::{
    BackendEvent, IntoJsReply, JsReply, Json, ScriptInjectionTime, ScriptMessageHandler,
    WatcherGuard, WebViewEvent, WebViewHandle as _,
};

/// Names the staged WPE runtime root these tests drive.
const RUNTIME_ENV: &str = "WATERUI_WPE_RUNTIME";

/// How long one wait may take before the test gives up.
///
/// Generous on purpose: a cold WPE runtime spawns a web process and a network
/// process before the first byte of the page is parsed, and CI runners are
/// slower than they look.
const TIMEOUT: Duration = Duration::from_mins(1);

/// `2^53 + 1`: the smallest integer a JavaScript number cannot hold.
const UNREPRESENTABLE: u64 = 9_007_199_254_740_993;

/// An integer a JavaScript number holds exactly, which must stay an ordinary
/// number rather than being tagged along with the one above.
const REPRESENTABLE: u64 = 42;

const FIRST_HTML: &str = include_str!("pages/first.html");
const SECOND_HTML: &str = include_str!("pages/second.html");
const CHECKS_JS: &str = include_str!("pages/checks.js");
const STATE_SEED_JS: &str = include_str!("pages/state_seed.js");

/// The local executor the page's handler replies are spawned onto.
///
/// `WpePage` answers a `waterui.invoke` by spawning the handler's future with
/// `executor_core::spawn_local` and evaluating the reply script when it
/// resolves, so a test that never drives a local executor would leave every
/// call pending forever. The engine's own loop is pumped in the same step, so
/// the executor has to be one this test can tick rather than one that blocks.
#[derive(Clone, Debug)]
struct TestExecutor(Rc<AsyncLocalExecutor<'static>>);

impl LocalExecutor for TestExecutor {
    type Task<T: 'static> = <AsyncLocalExecutor<'static> as LocalExecutor>::Task<T>;

    fn spawn_local<Fut>(&self, future: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        self.0.spawn_local(future)
    }
}

/// One WPE runtime, one page, and the server the page is loaded from.
struct RealEngine {
    server: Server,
    base: String,
    handle: WpeWebViewHandle,
    executor: TestExecutor,
    events: Rc<RefCell<Vec<BackendEvent>>>,
    reports: Rc<RefCell<Vec<Value>>>,
    echoed: Rc<RefCell<Option<Value>>>,
    /// Dropping this unsubscribes, and every wait below is on an event.
    _events: WatcherGuard,
}

impl core::fmt::Debug for RealEngine {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RealEngine")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// Wraps a synchronous answer in the asynchronous shape every handler has.
fn handler(
    answer: impl Fn(&[u8]) -> Result<JsReply, String> + 'static,
) -> Box<ScriptMessageHandler> {
    Box::new(move |payload: &[u8]| {
        let reply = answer(payload);
        Box::pin(async move { reply })
    })
}

/// The staged runtime, or a failure that says how to stage one.
fn runtime_paths() -> WpeRuntimePaths {
    let root = std::env::var_os(RUNTIME_ENV).unwrap_or_else(|| {
        panic!(
            "{RUNTIME_ENV} is not set. These tests drive a real WPE WebKit runtime rather than a \
             stand-in: build one with components/platform/browser-wpe/runtime/build-runtime.sh and \
             point {RUNTIME_ENV} at the staged root holding lib/libwaterui_wpe.so, runtime.json \
             and licenses/."
        )
    });
    WpeRuntimePaths::new(root)
}

impl RealEngine {
    /// Starts the server, initializes the runtime and opens one page.
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").expect("bind the local page server");
        let address = server
            .server_addr()
            .to_ip()
            .expect("the local page server listens on a TCP port");
        let base = format!("http://{address}");

        let executor = TestExecutor(Rc::new(AsyncLocalExecutor::new()));
        executor_core::init_local_executor(executor.clone());

        let page = WpePage::new(WpeRuntime::initialize(&runtime_paths()));
        // A viewport the engine would lay a document out in; the default is the
        // one-pixel toplevel the bridge creates a view with.
        page.resize(1024, 768, 1.0);
        // Exactly what `WpeController::open` builds: the handle installs the
        // transport adapter and the shared document-start bridge script.
        let handle = WpeWebViewHandle::new(page);

        let events = Rc::new(RefCell::new(Vec::new()));
        let guard = handle.watch({
            let events = Rc::clone(&events);
            move |event| events.borrow_mut().push(event)
        });

        let reports = Rc::new(RefCell::new(Vec::new()));
        let echoed = Rc::new(RefCell::new(None));

        handle.add_handler(
            "greet",
            handler(|payload| {
                let request: Value =
                    serde_json::from_slice(payload).map_err(|error| error.to_string())?;
                let name = request
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| String::from("greet payload has no `name` string"))?;
                format!("Hi {name}").into_js_reply()
            }),
        );
        handle.add_handler(
            "largest-id",
            handler(|_| Json(UNREPRESENTABLE).into_js_reply()),
        );
        handle.add_handler("small-id", handler(|_| Json(REPRESENTABLE).into_js_reply()));
        handle.add_handler("echo-id", {
            let echoed = Rc::clone(&echoed);
            handler(move |payload| {
                let request: Value =
                    serde_json::from_slice(payload).map_err(|error| error.to_string())?;
                echoed.replace(Some(request));
                String::from_utf8(payload.to_vec())
                    .map_err(|error| error.to_string())?
                    .into_js_reply()
            })
        });
        handle.add_handler("report", {
            let reports = Rc::clone(&reports);
            handler(move |payload| {
                let record: Value =
                    serde_json::from_slice(payload).map_err(|error| error.to_string())?;
                reports.borrow_mut().push(record);
                ().into_js_reply()
            })
        });

        handle.inject_script(STATE_SEED_JS, ScriptInjectionTime::DocumentStart);

        Self {
            server,
            base,
            handle,
            executor,
            events,
            reports,
            echoed,
            _events: guard,
        }
    }

    /// Answers whatever the engine has asked for, without blocking on it.
    ///
    /// Driven from the same loop that pumps the engine, so the page is served
    /// by this thread rather than by a second one racing it.
    fn serve(&self) {
        while let Some(request) = self
            .server
            .try_recv()
            .expect("the local page server stopped accepting requests")
        {
            let served = match request.url() {
                "/first" => Some((FIRST_HTML, "text/html; charset=utf-8")),
                "/second" => Some((SECOND_HTML, "text/html; charset=utf-8")),
                "/checks.js" => Some((CHECKS_JS, "text/javascript; charset=utf-8")),
                // Engines ask for things nobody linked, a favicon above all.
                // Saying so is the correct answer, not a test failure.
                _ => None,
            };
            let response = match served {
                Some((body, content_type)) => {
                    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
                        .expect("a static content type is a valid header");
                    Response::from_string(body).with_header(header)
                }
                None => Response::from_string(String::new()).with_status_code(404),
            };
            request
                .respond(response)
                .expect("the engine closed the connection before its page arrived");
        }
    }

    /// One turn of everything this test drives.
    fn step(&self) {
        self.serve();
        self.handle.page().pump();
        while self.executor.0.try_tick() {}
    }

    /// Spins until `ready` answers, failing loudly on a load error or a timeout.
    fn wait_for<T>(&self, what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(value) = ready() {
                return value;
            }
            self.assert_no_load_error();
            assert!(
                Instant::now() < deadline,
                "timed out after {TIMEOUT:?} waiting for {what}"
            );
            self.step();
            std::thread::yield_now();
        }
    }

    /// Drives one of the handle's futures to completion on this loop.
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        let mut future = pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        self.wait_for("an engine call to answer", || {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => Some(value),
                Poll::Pending => None,
            }
        })
    }

    fn assert_no_load_error(&self) {
        for event in self.events.borrow().iter() {
            assert!(
                !matches!(event, BackendEvent::Event(WebViewEvent::Error(_))),
                "the engine reported a load failure: {event:?}"
            );
        }
    }

    fn url(&self, path: &str) -> Url {
        format!("{}{path}", self.base)
            .parse()
            .expect("the local page server address is a URL")
    }

    fn loads(&self) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|event| matches!(event, BackendEvent::Event(WebViewEvent::Loaded)))
            .count()
    }

    /// Navigates and waits for the page to finish loading.
    fn navigate(&self, what: &str, act: impl FnOnce()) {
        let loads = self.loads();
        act();
        self.wait_for(what, || (self.loads() > loads).then_some(()));
    }

    /// Loads `path` and returns the record its script sent back.
    fn open(&self, path: &str) -> Value {
        let reported = self.reports.borrow().len();
        self.navigate(path, || self.handle.go_to(&self.url(path)));
        let record = self.wait_for(&format!("{path} to report over the bridge"), || {
            self.reports.borrow().get(reported).cloned()
        });
        if let Some(failure) = record.get("failure") {
            panic!("the page's checks raised {failure}");
        }
        record
    }

    /// What the engine says the current document's URL is.
    fn location(&self) -> String {
        let json = self
            .block_on(self.handle.run_javascript("location.href"))
            .expect("location.href evaluates");
        serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("run_javascript answered `{json}`, not JSON: {error}"))
    }
}

/// Reads one field of the page's record, saying what was there when it is
/// missing or the wrong shape.
fn field<'a>(record: &'a Value, name: &str) -> &'a Value {
    record
        .get(name)
        .unwrap_or_else(|| panic!("the page recorded no `{name}`; it recorded {record}"))
}

fn text<'a>(record: &'a Value, name: &str) -> &'a str {
    field(record, name)
        .as_str()
        .unwrap_or_else(|| panic!("the page's `{name}` is not text; it recorded {record}"))
}

#[test]
fn navigation_reaches_each_url_and_history_moves_both_ways() {
    let engine = RealEngine::start();

    let first = engine.open("/first");
    assert_eq!(text(&first, "page"), "first");
    assert_eq!(text(&first, "location"), engine.url("/first").as_str());
    assert_eq!(engine.location(), engine.url("/first").as_str());
    assert!(
        !engine.handle.can_go_back(),
        "the first document in a page's history has nothing behind it"
    );

    let second = engine.open("/second");
    assert_eq!(text(&second, "page"), "second");
    assert_eq!(engine.location(), engine.url("/second").as_str());
    assert!(engine.handle.can_go_back());
    assert!(!engine.handle.can_go_forward());

    engine.navigate("the engine to go back", || engine.handle.go_back());
    assert_eq!(engine.location(), engine.url("/first").as_str());
    assert!(engine.handle.can_go_forward());

    engine.navigate("the engine to go forward", || engine.handle.go_forward());
    assert_eq!(engine.location(), engine.url("/second").as_str());
    assert!(!engine.handle.can_go_forward());
}

/// The reply a handler returns has to arrive as the value it returned.
///
/// Every reply once crossed as base64, because the bridge had collapsed a
/// serialized value and a byte payload into the same `Vec<u8>` and could no
/// longer tell them apart. `await waterui.invoke(...)` then resolved to base64
/// text, which is a string of the right type and entirely the wrong value —
/// so the type alone is not the assertion, the value is.
#[test]
fn a_handler_reply_reaches_the_page_as_its_value_and_not_as_base64() {
    let engine = RealEngine::start();
    let record = engine.open("/first");

    assert_eq!(text(&record, "greetingType"), "string");
    assert_eq!(text(&record, "greeting"), "Hi Lexo");
    assert_ne!(
        text(&record, "greeting"),
        STANDARD.encode(br#""Hi Lexo""#),
        "the reply reached the page as the base64 of its JSON"
    );
}

/// `waterui.state` and `waterui.watch` have to exist in the page.
///
/// `bridge.js` froze the `waterui` object, and `state.js` runs after it and
/// defines both properties on that same object, so installing threw and the
/// entire mirrored-state feature was unreachable from JavaScript for the whole
/// life of the feature. Reading a seeded key back is the part that proves the
/// mirror is wired rather than merely present.
#[test]
fn the_page_can_reach_the_whole_waterui_object() {
    let engine = RealEngine::start();
    let record = engine.open("/first");

    assert_eq!(text(&record, "invokeType"), "function");
    assert_eq!(text(&record, "stateType"), "object");
    assert_eq!(text(&record, "watchType"), "function");
    assert_eq!(text(&record, "stateTheme"), "dark");
    assert_eq!(
        text(&record, "watchResultType"),
        "function",
        "watching a key answers with the function that stops watching it"
    );
}

/// Integers past 2^53 have to survive the crossing in both directions.
///
/// JSON has one numeric type and JavaScript reads it as a double, so these lost
/// their low bits silently and symmetrically: a value seeded into a page read
/// back rounded, and a page that returned the value it was given returned the
/// rounded one. Both ends looked correct the whole time.
#[test]
fn integers_beyond_two_to_the_fifty_third_cross_intact_both_ways() {
    let engine = RealEngine::start();
    let record = engine.open("/first");

    // Rust to the page, through a handler reply and through mirrored state.
    assert_eq!(text(&record, "largeType"), "bigint");
    assert_eq!(text(&record, "largeText"), UNREPRESENTABLE.to_string());
    assert_eq!(text(&record, "stateBigType"), "bigint");
    assert_eq!(text(&record, "stateBigText"), UNREPRESENTABLE.to_string());

    // A value a double does hold stays an ordinary number, so page arithmetic
    // on everyday values keeps working.
    assert_eq!(text(&record, "smallType"), "number");
    assert_eq!(field(&record, "smallValue").as_u64(), Some(REPRESENTABLE));

    // The page back to Rust.
    let echoed = engine
        .echoed
        .borrow()
        .clone()
        .expect("the page called the echo handler");
    assert_eq!(
        echoed
            .pointer("/id/__wateruiBigInt")
            .and_then(Value::as_str),
        Some(UNREPRESENTABLE.to_string().as_str()),
        "the page's BigInt reached Rust as {echoed}"
    );
    assert_eq!(
        echoed.pointer("/small").and_then(Value::as_u64),
        Some(REPRESENTABLE)
    );
}
