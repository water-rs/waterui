//! What only a real `WKWebView` can prove.
//!
//! These drive a genuine WebKit engine through the same handle
//! `MacSystemWebViewController::open` hands the renderer, load pages over a
//! local HTTP server, and assert on what crosses the bridge in both directions —
//! the macOS sibling of `waterui-browser-wpe`'s `real_engine` suite, asserting
//! the same contract over the same pages.
//!
//! They exist for the same reason that suite does: three total breaks of the
//! web view bridge shipped behind a fully green Rust suite (replies crossing as
//! base64, a frozen `waterui` object, integers past 2^53 losing their low
//! bits), and none of the three is visible to a test that stops at the Rust
//! side of the boundary.
//!
//! # Why `harness = false`
//!
//! `WKWebView` is `MainThreadOnly` and libtest runs every test on a worker
//! thread, so this target owns its `main` and runs its scenarios sequentially
//! on the real process main thread, pumping the main `NSRunLoop` by hand.
//! nextest reports the whole binary as one test, which is accurate: the
//! scenarios share one process because the main thread is the resource under
//! test.

#[cfg(target_os = "macos")]
mod real {
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
    use hydrolysis::MacSystemWebViewController;
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
    use serde_json::Value;
    use tiny_http::{Header, Response, Server};
    use waterui_webview::{
        BackendEvent, BridgeOrigins, CustomWebViewController as _, IntoJsReply, JsReply, Json,
        OriginPolicy, ScriptInjectionTime, ScriptMessageHandler, Url, WatcherGuard, WebViewEvent,
        WebViewHandle,
    };

    /// How long one wait may take before a scenario gives up.
    ///
    /// Generous on purpose: WebKit spawns a web process and a network process
    /// before the first byte of the page is parsed, and CI runners are slower
    /// than they look.
    const TIMEOUT: Duration = Duration::from_secs(60);

    /// `2^53 + 1`: the smallest integer a JavaScript number cannot hold.
    const UNREPRESENTABLE: u64 = 9_007_199_254_740_993;

    /// An integer a JavaScript number holds exactly, which must stay an
    /// ordinary number rather than being tagged along with the one above.
    const REPRESENTABLE: u64 = 42;

    // The pages live with the shared webview crate because they exercise the
    // shared bridge contract; every real-engine suite loads the same ones.
    const FIRST_HTML: &str = include_str!("../../../components/platform/webview/tests/pages/first.html");
    const SECOND_HTML: &str =
        include_str!("../../../components/platform/webview/tests/pages/second.html");
    const CHECKS_JS: &str = include_str!("../../../components/platform/webview/tests/pages/checks.js");
    const STATE_SEED_JS: &str =
        include_str!("../../../components/platform/webview/tests/pages/state_seed.js");

    /// The local executor the page's handler replies are spawned onto.
    ///
    /// The engine answers a `waterui.invoke` by spawning the handler's future
    /// with `executor_core::spawn_local` and evaluating the reply script when
    /// it resolves, so a run that never ticks a local executor would leave
    /// every call pending forever.
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

    /// One `WKWebView`, and the server its pages are loaded from.
    struct RealEngine<H: WebViewHandle> {
        server: Server,
        base: String,
        handle: H,
        executor: TestExecutor,
        events: Rc<RefCell<Vec<BackendEvent>>>,
        reports: Rc<RefCell<Vec<Value>>>,
        echoed: Rc<RefCell<Option<Value>>>,
        /// Dropping this unsubscribes, and every wait below is on an event.
        _events: WatcherGuard,
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

    /// Starts the server, opens one web view and wires the shared contract.
    fn start(executor: &TestExecutor) -> RealEngine<impl WebViewHandle + use<>> {
        let server = Server::http("127.0.0.1:0").expect("bind the local page server");
        let address = server
            .server_addr()
            .to_ip()
            .expect("the local page server listens on a TCP port");
        let base = format!("http://{address}");

        // Exactly what the renderer gets: a genuine WKWebView behind the
        // shared handle contract.
        let handle = MacSystemWebViewController.open();

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

        handle.inject_script(
            "waterui:test-state-seed",
            STATE_SEED_JS,
            ScriptInjectionTime::DocumentStart,
        );

        // The default policy an opened view resolves to: the origin it was
        // opened at, which is the local server. This is the policy under which
        // the bridge admits the pages below and refuses everything else.
        let initial: Url = base.parse().expect("the local page server address is a URL");
        handle.set_bridge_origins(OriginPolicy::new(BridgeOrigins::Initial, &initial));

        RealEngine {
            server,
            base,
            handle,
            executor: executor.clone(),
            events,
            reports,
            echoed,
            _events: guard,
        }
    }

    impl<H: WebViewHandle> RealEngine<H> {
        /// Answers whatever the engine has asked for, without blocking on it.
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
                    // Engines ask for things nobody linked, a favicon above
                    // all. Saying so is the correct answer, not a failure.
                    _ => None,
                };
                let response = match served {
                    Some((body, content_type)) => {
                        let header =
                            Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
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

        /// One turn of everything this run drives: the page server, WebKit's
        /// delegate callbacks on the main run loop, and the handler executor.
        fn step(&self) {
            self.serve();
            let deadline = NSDate::dateWithTimeIntervalSinceNow(0.02);
            // SAFETY: the main run loop is pumped from the main thread, which
            // `main` below is.
            unsafe {
                NSRunLoop::currentRunLoop().runMode_beforeDate(NSDefaultRunLoopMode, &deadline);
            }
            while self.executor.0.try_tick() {}
        }

        /// Spins until `ready` answers, failing loudly on a load error or a
        /// timeout.
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
        ///
        /// This backend marshals a JavaScript string as itself rather than as
        /// JSON, so the answer is used directly.
        fn location(&self) -> String {
            self.block_on(self.handle.run_javascript("location.href"))
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

    fn navigation_reaches_each_url_and_history_moves_both_ways(executor: &TestExecutor) {
        let engine = start(executor);

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

    /// The reply a handler returns has to arrive as the value it returned, not
    /// as the base64 of its JSON.
    fn a_handler_reply_reaches_the_page_as_its_value(executor: &TestExecutor) {
        let engine = start(executor);
        let record = engine.open("/first");

        assert_eq!(text(&record, "greetingType"), "string");
        assert_eq!(text(&record, "greeting"), "Hi Lexo");
        assert_ne!(
            text(&record, "greeting"),
            STANDARD.encode(br#""Hi Lexo""#),
            "the reply reached the page as the base64 of its JSON"
        );
    }

    /// `waterui.invoke`, `waterui.state` and `waterui.watch` have to exist in
    /// the page, and a seeded key has to read back.
    fn the_page_can_reach_the_whole_waterui_object(executor: &TestExecutor) {
        let engine = start(executor);
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
    fn integers_beyond_two_to_the_fifty_third_cross_intact(executor: &TestExecutor) {
        let engine = start(executor);
        let record = engine.open("/first");

        // Rust to the page, through a handler reply and through mirrored state.
        assert_eq!(text(&record, "largeType"), "bigint");
        assert_eq!(text(&record, "largeText"), UNREPRESENTABLE.to_string());
        assert_eq!(text(&record, "stateBigType"), "bigint");
        assert_eq!(text(&record, "stateBigText"), UNREPRESENTABLE.to_string());

        // A value a double does hold stays an ordinary number.
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

    /// The typed evaluation entry point has to *await* the shared wrapper.
    ///
    /// `__wateruiEval` is `async`, so the value of the expression is a promise.
    /// An engine API that does not await one hands back the promise object
    /// instead of the JSON envelope it resolves to — which is how every
    /// `eval!`/`exec!` failed while mirrored state silently stopped reaching
    /// the page on every backend that evaluated without awaiting.
    fn typed_evaluation_awaits_the_wrapper(executor: &TestExecutor) {
        let engine = start(executor);
        engine.open("/first");

        let answer = engine
            .block_on(
                engine
                    .handle
                    .call_async_javascript("return globalThis.__wateruiEval(async () => 6 * 7, []);"),
            )
            .expect("the awaiting evaluation path answers");
        let envelope: Value = serde_json::from_str(&answer).unwrap_or_else(|error| {
            panic!("the wrapper's promise resolved to `{answer}`, not its JSON envelope: {error}")
        });
        assert_eq!(envelope.get("ok"), Some(&Value::Bool(true)), "{envelope}");
        assert_eq!(
            envelope.get("value").and_then(Value::as_u64),
            Some(42),
            "{envelope}"
        );
    }

    /// Injecting under a key already in use replaces that script.
    ///
    /// The mirrored-state seed depends on this: it is re-rendered and
    /// re-injected under the same key before every navigation, and an engine
    /// that stacks copies instead runs every stale seed ahead of the current
    /// one. Each probe script *appends* to an array, so a stacked pair is
    /// visible as two entries where a replaced one leaves exactly the latest.
    fn a_keyed_injection_replaces_its_predecessor(executor: &TestExecutor) {
        let engine = start(executor);

        let probe = |tag: &str| {
            format!(
                "(globalThis.__wateruiProbe = globalThis.__wateruiProbe || []).push({tag:?});"
            )
        };
        engine
            .handle
            .inject_script("waterui:test-probe", &probe("stale"), ScriptInjectionTime::DocumentStart);
        engine.open("/first");
        engine
            .handle
            .inject_script("waterui:test-probe", &probe("current"), ScriptInjectionTime::DocumentStart);
        engine.open("/second");

        let recorded = engine
            .block_on(engine.handle.run_javascript("JSON.stringify(globalThis.__wateruiProbe)"))
            .expect("the probe array evaluates");
        let entries: Vec<String> = serde_json::from_str(&recorded)
            .unwrap_or_else(|error| panic!("the probe recorded `{recorded}`: {error}"));
        assert_eq!(
            entries,
            ["current"],
            "a keyed injection must replace its predecessor, not stack behind it"
        );
    }

    pub fn run() {
        let executor = TestExecutor(Rc::new(AsyncLocalExecutor::new()));
        executor_core::init_local_executor(executor.clone());

        navigation_reaches_each_url_and_history_moves_both_ways(&executor);
        a_handler_reply_reaches_the_page_as_its_value(&executor);
        the_page_can_reach_the_whole_waterui_object(&executor);
        integers_beyond_two_to_the_fifty_third_cross_intact(&executor);
        typed_evaluation_awaits_the_wrapper(&executor);
        a_keyed_injection_replaces_its_predecessor(&executor);
    }
}

#[cfg(target_os = "macos")]
fn main() {
    real::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {}
