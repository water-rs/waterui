//! What only a real engine can prove.
//!
//! These tests drive a genuine Chromium through the same [`WebViewController`]
//! an application is handed, load pages over a local HTTP server, and assert on
//! what crosses the bridge in both directions.
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
//! one that passed. Unlike the WPE sibling nothing has to be staged first: the
//! engine is the prebuilt CEF distribution `cef-dll-sys` downloads while
//! building this crate, and its path is compiled in by `build.rs`.
//!
//! ```sh
//! cargo test -p waterui-browser-cef --features real-engine --test real_engine
//! ```
//!
//! `cargo test` rather than `cargo nextest run`, and `harness = false`, because
//! of what CEF requires of the process it runs in — see below.
//!
//! # The process model
//!
//! CEF on macOS is not a library a test can call into from wherever it happens
//! to be running. Three constraints decide the shape of this file, and every
//! one of them was established by running the thing and reading the failure:
//!
//! * **The browser process must be the main thread.** `NSApplication` has to be
//!   the [`CefAppProtocol`][initialize_macos_application] subclass before
//!   anything asks for `sharedApplication`, and installing it asserts it is on
//!   the main thread. libtest runs test bodies on spawned threads, so there is
//!   no `#[test]` that can do this; `harness = false` puts the checks in `main`.
//! * **The browser process must be bundled.** Outside an application bundle
//!   `cef_initialize` traps inside the framework — an official-build `CHECK`,
//!   which carries no message. So `main` stages a bundle (see [`bundle`]) and
//!   re-executes itself from inside it.
//! * **Chromium's child processes are launched from helper bundles.** Without
//!   them the GPU and network processes fail to launch and Chromium aborts the
//!   browser process with `GPU process isn't usable. Goodbye.`. The helpers run
//!   this same executable, which dispatches to
//!   [`run_packaged_subprocess`] when Chromium passes it a `--type=`.
//!
//! One process therefore hosts one CEF runtime and every check runs against it,
//! each on a web view of its own.

#[cfg(not(target_os = "macos"))]
compile_error!(
    "the CEF real-engine tests drive a macOS application bundle, which is what CEF requires of a \
     browser process on this platform; run them on a macOS host"
);

mod bundle;

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
use waterui_browser_cef::{
    CefRuntime, CefRuntimeConfiguration, CefRuntimePaths, initialize_macos_application,
    run_packaged_subprocess,
};
use waterui_url::Url;
use waterui_webview::{
    BackendEvent, BridgeOrigins, IntoJsReply, JsReply, Json, OriginPolicy, ScriptInjectionTime,
    ScriptMessageHandler, WatcherGuard, WebView, WebViewController, WebViewEvent,
};

/// How long one wait may take before the check gives up.
///
/// Generous on purpose: a cold Chromium starts a GPU process, a network process
/// and a renderer before the first byte of the page is parsed, and CI runners
/// are slower than they look.
const TIMEOUT: Duration = Duration::from_secs(120);

/// `2^53 + 1`: the smallest integer a JavaScript number cannot hold.
const UNREPRESENTABLE: u64 = 9_007_199_254_740_993;

/// An integer a JavaScript number holds exactly, which must stay an ordinary
/// number rather than being tagged along with the one above.
const REPRESENTABLE: u64 = 42;

// The pages live with the shared webview crate because they exercise the shared
// bridge contract; every real-engine suite loads the same ones.
const FIRST_HTML: &str = include_str!("../../webview/tests/pages/first.html");
const SECOND_HTML: &str = include_str!("../../webview/tests/pages/second.html");
const CHECKS_JS: &str = include_str!("../../webview/tests/pages/checks.js");
const STATE_SEED_JS: &str = include_str!("../../webview/tests/pages/state_seed.js");

/// The local executor the page's handler replies are spawned onto.
///
/// The CEF bridge answers a `waterui.invoke` by spawning the handler's future
/// with `executor_core::spawn_local` and evaluating the reply over the
/// `DevTools` protocol when it resolves, so a test that never drives a local
/// executor would leave every call pending forever. The engine's own loop is pumped in the same
/// step, so the executor has to be one this test can tick rather than one that
/// blocks.
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

/// Wraps a synchronous answer in the asynchronous shape every handler has.
fn handler(
    answer: impl Fn(&[u8]) -> Result<JsReply, String> + 'static,
) -> Box<ScriptMessageHandler> {
    Box::new(move |payload: &[u8]| {
        let reply = answer(payload);
        Box::pin(async move { reply })
    })
}

/// The CEF runtime, the executor its replies are spawned onto, and the server
/// the pages are loaded from.
///
/// One per process: CEF initializes once, and everything below shares it.
struct Engine {
    server: Server,
    base: String,
    controller: WebViewController,
    executor: TestExecutor,
    runtime: CefRuntime,
}

impl Engine {
    /// Initializes CEF and starts the page server.
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").expect("bind the local page server");
        let address = server
            .server_addr()
            .to_ip()
            .expect("the local page server listens on a TCP port");
        let base = format!("http://{address}");

        let executor = TestExecutor(Rc::new(AsyncLocalExecutor::new()));
        executor_core::init_local_executor(executor.clone());

        initialize_macos_application();
        // `packaged` is the production resolution: the runtime staged beside the
        // executable inside the bundle. The cache is the one thing pointed
        // somewhere else, because a test writing a Chromium profile into the
        // user's cache directory is not a test that cleans up after itself.
        let runtime = CefRuntime::initialize(CefRuntimeConfiguration::new(
            CefRuntimePaths::packaged(),
            bundle::workspace().join("cache"),
        ));

        Self {
            server,
            base,
            controller: runtime.webview_controller(),
            executor,
            runtime,
        }
    }

    /// Answers whatever the engine has asked for, without blocking on it.
    ///
    /// Driven from the same loop that pumps CEF, so the page is served by this
    /// thread rather than by a second one racing it.
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
        let _ = self.runtime.pump();
        while self.executor.0.try_tick() {}
    }

    fn url(&self, path: &str) -> Url {
        format!("{}{path}", self.base)
            .parse()
            .expect("the local page server address is a URL")
    }

    /// Opens a web view carrying the same bridge configuration
    /// `WebView::open(..).handler(..).expose(..)` installs on one.
    fn page(&self) -> Page<'_> {
        Page::open(self)
    }
}

/// One web view, the handlers the page calls, and what they observed.
struct Page<'a> {
    engine: &'a Engine,
    webview: WebView,
    events: Rc<RefCell<Vec<BackendEvent>>>,
    reports: Rc<RefCell<Vec<Value>>>,
    echoed: Rc<RefCell<Option<Value>>>,
    /// Dropping this unsubscribes, and every wait below is on an event.
    _events: WatcherGuard,
}

impl<'a> Page<'a> {
    fn open(engine: &'a Engine) -> Self {
        let webview = engine.controller.open();
        let handle = webview.handle();

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

        // Exactly what `WebViewOpen::create` installs, in the same order: the
        // policy first, so no handler is ever reachable unguarded.
        handle.set_bridge_origins(OriginPolicy::new(BridgeOrigins::Initial, &engine.url("/")));
        handle.inject_script(
            "waterui:test-state-seed",
            STATE_SEED_JS,
            ScriptInjectionTime::DocumentStart,
        );

        let page = Self {
            engine,
            webview,
            events,
            reports,
            echoed,
            _events: guard,
        };
        page.await_bridge_installation();
        page
    }

    /// Waits until the document-start scripts are registered with the engine.
    ///
    /// The bridge and the seed script are installed with `DevTools` commands
    /// nobody awaits, while navigation is an ordinary CEF call, so nothing
    /// orders the two. `DevTools` commands on one session are answered in the
    /// order they were issued, so an answer to a later one is the real signal
    /// that the earlier ones have been applied.
    fn await_bridge_installation(&self) {
        let evaluated = self
            .block_on(self.webview.run_javascript("'installed'"))
            .expect("the engine evaluates a string literal");
        assert_eq!(
            evaluated.as_str(),
            "installed",
            "the engine answered a trivial evaluation with something else"
        );
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
            self.engine.step();
            std::thread::yield_now();
        }
    }

    /// Drives one of the web view's futures to completion on this loop.
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

    fn loads(&self) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|event| matches!(event, BackendEvent::Event(WebViewEvent::Loaded)))
            .count()
    }

    /// Navigates, waits for the page to finish loading, and waits for the
    /// reactive history signals to agree with the browser.
    ///
    /// The second wait is what makes `can_go_back` readable here at all. CEF
    /// reports a load end and the resulting history state as two separate
    /// callbacks, so the signals fed by the second one lag the load by a pump.
    /// The browser answers for its own history synchronously, which makes
    /// "the two agree" a condition to wait on rather than a delay to guess.
    fn navigate(&self, what: &str, act: impl FnOnce()) {
        use waterui_core::Signal as _;

        let loads = self.loads();
        act();
        self.wait_for(what, || (self.loads() > loads).then_some(()));
        self.wait_for("the history signals to catch up with the browser", || {
            let handle = self.webview.handle();
            (self.webview.can_go_back().get() == handle.can_go_back()
                && self.webview.can_go_forward().get() == handle.can_go_forward())
            .then_some(())
        });
    }

    /// Loads `path` and returns the record its script sent back.
    fn open_page(&self, path: &str) -> Value {
        let reported = self.reports.borrow().len();
        self.navigate(path, || self.webview.go_to(self.engine.url(path)));
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
        self.block_on(self.webview.run_javascript("location.href"))
            .expect("location.href evaluates")
            .to_string()
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

fn navigation_reaches_each_url_and_history_moves_both_ways(engine: &Engine) {
    // Scoped to this check: `Signal::get` shadows `slice::get`, which the rest
    // of this file reads its recorded observations with.
    use waterui_core::Signal as _;

    let page = engine.page();

    let first = page.open_page("/first");
    assert_eq!(text(&first, "page"), "first");
    assert_eq!(text(&first, "location"), engine.url("/first").as_str());
    assert_eq!(page.location(), engine.url("/first").as_str());
    // Asserted because it is a difference, not because it is desirable: CEF
    // creates its browser on `about:blank` and *commits* that navigation, so the
    // first page an application opens already has a blank document behind it,
    // where WPE and the Apple backends start with an empty history. Pinning it
    // here means the day it changes is the day this line fails.
    assert!(
        page.webview.can_go_back().get(),
        "CEF has stopped committing the blank document it creates a browser on"
    );

    let second = page.open_page("/second");
    assert_eq!(text(&second, "page"), "second");
    assert_eq!(page.location(), engine.url("/second").as_str());
    assert!(page.webview.can_go_back().get());
    assert!(!page.webview.can_go_forward().get());

    page.navigate("the engine to go back", || page.webview.go_back());
    assert_eq!(page.location(), engine.url("/first").as_str());
    assert!(page.webview.can_go_forward().get());

    page.navigate("the engine to go forward", || page.webview.go_forward());
    assert_eq!(page.location(), engine.url("/second").as_str());
    assert!(!page.webview.can_go_forward().get());
}

/// The reply a handler returns has to arrive as the value it returned.
///
/// Every reply once crossed as base64, because the bridge had collapsed a
/// serialized value and a byte payload into the same `Vec<u8>` and could no
/// longer tell them apart. `await waterui.invoke(...)` then resolved to base64
/// text, which is a string of the right type and entirely the wrong value —
/// so the type alone is not the assertion, the value is.
fn a_handler_reply_reaches_the_page_as_its_value_and_not_as_base64(engine: &Engine) {
    let record = engine.page().open_page("/first");

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
fn the_page_can_reach_the_whole_waterui_object(engine: &Engine) {
    let record = engine.page().open_page("/first");

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
fn integers_beyond_two_to_the_fifty_third_cross_intact_both_ways(engine: &Engine) {
    let page = engine.page();
    let record = page.open_page("/first");

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
    let echoed = page
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

/// Every check, in the order they run.
type Check = (&'static str, fn(&Engine));

const CHECKS: [Check; 4] = [
    (
        "navigation_reaches_each_url_and_history_moves_both_ways",
        navigation_reaches_each_url_and_history_moves_both_ways,
    ),
    (
        "a_handler_reply_reaches_the_page_as_its_value_and_not_as_base64",
        a_handler_reply_reaches_the_page_as_its_value_and_not_as_base64,
    ),
    (
        "the_page_can_reach_the_whole_waterui_object",
        the_page_can_reach_the_whole_waterui_object,
    ),
    (
        "integers_beyond_two_to_the_fifty_third_cross_intact_both_ways",
        integers_beyond_two_to_the_fifty_third_cross_intact_both_ways,
    ),
];

fn main() {
    // Chromium launches its GPU, network and renderer processes from the helper
    // bundles staged around this same executable, so a `--type=` argument means
    // this process is one of them and never returns from here.
    if bundle::is_child_process() {
        std::process::exit(run_packaged_subprocess());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // CEF traps rather than returning an error when its browser process is not
    // bundled, so the checks run from a staged bundle around this executable.
    if !bundle::running_bundled() {
        let bundled = bundle::stage();
        tracing::info!(executable = %bundled.display(), "re-running from the staged bundle");
        let status = std::process::Command::new(&bundled)
            .status()
            .unwrap_or_else(|error| panic!("run {}: {error}", bundled.display()));
        assert!(status.success(), "the bundled real-engine run {status}");
        return;
    }

    let engine = Engine::start();
    for (name, check) in CHECKS {
        tracing::info!(check = name, "running");
        check(&engine);
        tracing::info!(check = name, "passed");
    }
    tracing::info!(checks = CHECKS.len(), "every real-engine check passed");
}
