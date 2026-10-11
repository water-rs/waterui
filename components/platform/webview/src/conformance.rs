//! The checks every engine runs against the contracts this crate states.
//!
//! A backend crate — `waterui-browser-cef`, `waterui-browser-wpe`, the
//! platform web views — proves its engine against a real page in its own
//! real-engine suite. The contracts those suites have to agree on live here,
//! as functions that take the engine's evaluator and assert, so that two
//! engines cannot quietly satisfy two readings of the same sentence: the raw
//! evaluation reply once did, with one engine unquoting strings and another
//! encoding them, and a fixture reading `location.href` could not tell a URL
//! from a string that happened to look like one.
//!
//! Behind the `conformance` feature, because these are assertions and belong
//! in test binaries.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Waker;

use futures::channel::oneshot;
use serde_json::Value;
use waterui_core::{Binding, Str};
use waterui_url::Url;

use crate::assets::{ASSET_HTTPS_ORIGIN, ASSET_ORIGIN, AssetRequest, AssetResponse, AssetServer};
use crate::{
    BackendEvent, JsExpr, JsOutcome, WebViewConfig, WebViewController, WebViewError, WebViewEvent,
    run_wrapped_on,
};

/// The raw reply of [`WebViewHandle::run_javascript`](crate::WebViewHandle::run_javascript)
/// is the JSON encoding of the evaluated value.
///
/// `evaluate` is the engine's raw path — `|script| handle.run_javascript(script)`
/// on a handle, or the same call on a `WebView`, whichever the suite holds —
/// over a page that has loaded. Panics, naming the reply, when the engine
/// answers otherwise.
///
/// # Panics
///
/// When a string arrives unquoted, an object does not parse as JSON with its
/// fields intact, a number is not its literal, or `undefined` is anything but
/// `null`.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
pub async fn raw_evaluation_answers_json(evaluate: impl AsyncFn(&str) -> Result<Str, Str>) {
    let string = evaluate("'waterui'")
        .await
        .expect("a string literal evaluates");
    assert_eq!(
        string.as_str(),
        "\"waterui\"",
        "a string result arrives as JSON, quoted"
    );

    let number = evaluate("40 + 2")
        .await
        .expect("an arithmetic expression evaluates");
    assert_eq!(number.as_str(), "42", "a number arrives as its literal");

    let object = evaluate("({name: 'waterui', ok: true, items: [1, 2]})")
        .await
        .expect("an object literal evaluates");
    let decoded: Value = serde_json::from_str(&object)
        .unwrap_or_else(|error| panic!("run_javascript answered `{object}`, not JSON: {error}"));
    assert_eq!(
        decoded,
        serde_json::json!({"name": "waterui", "ok": true, "items": [1, 2]}),
        "an object arrives as JSON with its fields intact"
    );

    let nothing = evaluate("undefined").await.expect("undefined evaluates");
    assert_eq!(
        nothing.as_str(),
        "null",
        "JSON has no undefined; the typed path is where it stays distinct"
    );
}

/// The engine's asset origin serves the bundled site as a secure context.
///
/// A web view opened on it loads the module script, stylesheet and WASM the
/// case's server answers, and the serving layer answers only GET and HEAD
/// inside the asset root.
///
/// `controller` is the engine's [`WebViewController`]. The case drives the
/// same calls [`WebView::open_assets`](crate::WebView::open_assets) makes at
/// render — `open_with` carrying the [`AssetServer`], `asset_origin`, and
/// navigation to `index.html` under that origin — because the view-level call
/// needs a running renderer, which a conformance suite does not have.
///
/// The suite keeps the engine's event loop turning while this runs: the case
/// waits on the page's `Loaded` event, which an engine only emits while its
/// loop is pumped (a WPE suite pumps `WpeRuntime::iteration`, a GTK suite runs
/// its main context, a CDP suite drives its browser).
///
/// # Panics
///
/// When the engine reports no asset origin, the page never loads, the origin
/// is not a secure context, the stylesheet or WASM did not take effect, or a
/// request outside GET/HEAD or outside the asset root was served.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
pub async fn asset_origin_serves_bundled_content(controller: &WebViewController) {
    asset_origin_serves_bundled_content_with(controller, bundled_site_server()).await;
}

/// The same case with the caller's server answering.
///
/// The in-memory [`bundled_site_server`] is what most engines serve; a suite
/// proving `include_web!`'s serving layer instead passes a
/// [`DirectoryServer`](crate::DirectoryServer) over the site
/// [`write_bundled_site`] wrote to disk — the shape the staged bundle takes at
/// runtime — and reaches the identical assertions.
///
/// # Panics
/// Same contract as [`asset_origin_serves_bundled_content`].
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
pub async fn asset_origin_serves_bundled_content_with(
    controller: &WebViewController,
    server: AssetServer,
) {
    let webview = controller.open_with(WebViewConfig {
        asset_server: Some(server),
    });
    let handle = webview.handle().clone();
    let origin = handle
        .asset_origin()
        .expect("a web view opened with an asset server reports the origin it serves");
    assert!(
        origin.as_str() == ASSET_ORIGIN || origin.as_str() == ASSET_HTTPS_ORIGIN,
        "the asset origin is `{ASSET_ORIGIN}` or `{ASSET_HTTPS_ORIGIN}`, not `{origin}`"
    );

    let entry: Url = format!("{origin}/index.html")
        .parse()
        .expect("the entry URL");
    await_entry(&handle, &entry).await;

    assert_bundled_site(origin.as_str(), async |source| {
        eval::<Value>(&handle, source).await
    })
    .await;
}

/// The bundled-site assertions, driven through any JSON-answering evaluator.
///
/// `evaluate` answers the JSON value an expression produces, awaiting promises
/// — a `WebView`'s `run_javascript` path for one engine, a CDP
/// `Runtime.evaluate` for another — over a page that has already loaded the
/// site [`bundled_site_server`] serves under `origin`.
///
/// # Panics
///
/// When the module script was not served, the page is not a secure,
/// non-isolated context on `origin`, the stylesheet or WASM did not take
/// effect, or a request outside GET/HEAD or outside the asset root was served.
pub async fn assert_bundled_site(origin: &str, evaluate: impl AsyncFn(&str) -> Value) {
    let probe = evaluate(
        "(async () => ({\
            app: (await fetch('/app.js')).status,\
            appType: (await fetch('/app.js')).headers.get('content-type'),\
            resources: performance.getEntriesByType('resource').map((e) => e.name),\
        }))()",
    )
    .await;
    assert_eq!(
        probe.get("app").and_then(Value::as_u64),
        Some(200),
        "the module script is served: {probe}"
    );

    let title = evaluate("document.title").await;
    assert_eq!(
        title,
        Value::String(format!("{origin}|true|false")),
        "the page is a secure, non-isolated context on the engine's asset origin"
    );

    let color = evaluate("getComputedStyle(document.body).backgroundColor").await;
    assert_eq!(
        color,
        Value::String("rgb(18, 52, 86)".to_string()),
        "the bundled stylesheet applied"
    );

    // The module script instantiated the wasm it fetched; give it a bounded
    // window since `Loaded` does not order against its promise.
    let wasm = evaluate(
        "(async () => {\
            const deadline = Date.now() + 5000;\
            while (globalThis.__wateruiWasm === undefined && Date.now() < deadline)\
                await new Promise((resolve) => setTimeout(resolve, 10));\
            return globalThis.__wateruiWasm === true;\
        })()",
    )
    .await;
    assert_eq!(
        wasm,
        Value::Bool(true),
        "the module's WebAssembly.instantiateStreaming settled"
    );

    // A fetch URL's `..` segments — literal or `%2e`-spelled — are removed by
    // the engine's own URL parsing before the request reaches interception, so
    // the traversal that can actually arrive spells its separators `%2f`: the
    // server sees `/../index.html` only after the dispatcher's one decode, and
    // refuses it. The rule itself is covered by `assets::dispatch`'s tests.
    let statuses = evaluate(
        "(async () => ({\
            traversal: (await fetch('/a%2f..%2findex.html')).status,\
            encoded: (await fetch('/%2e%2e%2findex.html')).status,\
            post: (await fetch('/index.html', {method: 'POST'})).status,\
        }))()",
    )
    .await;
    assert_eq!(
        statuses.get("traversal").and_then(Value::as_u64),
        Some(404),
        "`..` after one decode is refused: {statuses}"
    );
    assert_eq!(
        statuses.get("encoded").and_then(Value::as_u64),
        Some(404),
        "an encoded `..` is refused: {statuses}"
    );
    assert_eq!(
        statuses.get("post").and_then(Value::as_u64),
        Some(405),
        "POST is refused: {statuses}"
    );
}

/// The bundled site the asset-origin cases serve: `index.html`, its module
/// script, stylesheet and WASM module, and nothing else.
///
/// Shared with suites that open the site through a different surface — a
/// `ChromiumPage` over CDP `Fetch` interception serves the same bytes a
/// `WebView`'s native interception does.
#[must_use]
pub fn bundled_site_server() -> AssetServer {
    Arc::new(|request: &AssetRequest| {
        match request.path.as_str() {
            "/" | "/index.html" => AssetResponse::ok(
                "text/html",
                include_bytes!("conformance/index.html").to_vec(),
            ),
            "/app.js" => AssetResponse::ok(
                "text/javascript",
                include_bytes!("conformance/app.js").to_vec(),
            ),
            "/style.css" => {
                AssetResponse::ok("text/css", include_bytes!("conformance/style.css").to_vec())
            }
            // The smallest module: header only, no sections.
            "/app.wasm" => AssetResponse::ok("application/wasm", WASM_MODULE.to_vec()),
            _ => AssetResponse::not_found(),
        }
    })
}

/// Writes the site [`bundled_site_server`] serves into `dir` as real files.
///
/// `index.html`, `app.js`, `style.css`, and `app.wasm` land on disk so a
/// [`DirectoryServer`](crate::DirectoryServer) serves the same bytes and
/// [`assert_bundled_site`] holds verbatim.
///
/// # Panics
/// When a file cannot be written.
pub fn write_bundled_site(dir: &std::path::Path) {
    for (name, bytes) in [
        (
            "index.html",
            include_bytes!("conformance/index.html").as_slice(),
        ),
        ("app.js", include_bytes!("conformance/app.js").as_slice()),
        (
            "style.css",
            include_bytes!("conformance/style.css").as_slice(),
        ),
        ("app.wasm", WASM_MODULE),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, bytes)
            .unwrap_or_else(|error| panic!("writing {} failed: {error}", path.display()));
    }
}

/// The events an asset-origin view has emitted and the task waiting on them.
#[derive(Default)]
struct LoadState {
    events: Vec<BackendEvent>,
    waker: Option<Waker>,
}

/// Navigates to `entry` and returns once its `Loaded` has been reported.
///
/// Awaiting, not blocking: the suite drives the case on the same thread that
/// pumps the engine, so a blocking wait would starve the loop that delivers
/// `Loaded`. The waker keeps the wait honest under an executor too. An engine
/// that commits a document at creation — CEF loads `about:blank` — can emit a
/// `Loaded` that is not this navigation's, so the wait ends only once the
/// loaded document is `entry`.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
async fn await_entry(handle: &crate::AnyWebViewHandle, entry: &Url) {
    let state = Rc::new(RefCell::new(LoadState::default()));
    let _watcher = handle.watch({
        let state = Rc::clone(&state);
        move |event| {
            let mut state = state.borrow_mut();
            state.events.push(event);
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
    });
    // The document-start installs are commands nobody awaits, while navigation
    // is an ordinary engine call, so nothing orders the two. A raw evaluation
    // answered before `go_to` is the real signal that they have been applied:
    // the engine answers commands in the order they were issued.
    handle
        .run_javascript("'installed'")
        .await
        .expect("the engine evaluates on its initial document");
    handle.go_to(entry);
    let mut loaded_seen = 0_usize;
    loop {
        loaded_seen = std::future::poll_fn(|context| {
            let mut state = state.borrow_mut();
            for event in &state.events {
                if let BackendEvent::Event(WebViewEvent::Error(error)) = event {
                    panic!("the asset page failed to load: {}", describe_error(error));
                }
            }
            let loaded = state
                .events
                .iter()
                .filter(|event| matches!(event, BackendEvent::Event(WebViewEvent::Loaded)))
                .count();
            if loaded > loaded_seen {
                return std::task::Poll::Ready(loaded);
            }
            state.waker = Some(context.waker().clone());
            std::task::Poll::Pending
        })
        .await;
        if eval::<String>(handle, "location.href").await == entry.as_str() {
            break;
        }
    }
}

/// The smallest WebAssembly module: magic and version, no sections.
const WASM_MODULE: &[u8] = b"\0asm\x01\x00\x00\x00";

/// Evaluates `source` through the engine's awaiting path and decodes the value.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
async fn eval<T: serde::de::DeserializeOwned>(handle: &crate::AnyWebViewHandle, source: &str) -> T {
    let call = JsExpr::raw(source.to_string()).wrapped_call();
    let outcome: JsOutcome = run_wrapped_on(handle, &call)
        .await
        .unwrap_or_else(|error| panic!("`{source}` failed to run: {error}"));
    outcome.decode().unwrap_or_else(|error| {
        panic!(
            "`{source}` produced no `{0}`: {error}",
            core::any::type_name::<T>()
        )
    })
}

fn describe_error(error: &WebViewError) -> String {
    match error {
        WebViewError::Network(message) => format!("network: {message}"),
        WebViewError::Ssl { url, message } => format!("ssl at {url}: {message}"),
        WebViewError::LoadFailed(message) => format!("load: {message}"),
    }
}

/// Mirrored state and bridge replies reach only the documents the admission
/// policy admits, and admitted documents get every change.
///
/// One origin is admitted and a second, on another port, is refused. A
/// subject view walks through the admitted origin and then the refused one;
/// a witness view stays on the admitted origin, exposing the same binding.
///
/// - The subject's first document is seeded, and gets a change.
/// - It still gets one after a same-document navigation, a fragment change
///   and a `pushState`, which the engine may report as a navigation.
/// - The next document is seeded with the current value and gets a change.
/// - Going back restores the first document from the back/forward cache
///   (its `pageshow` is `persisted`); it catches up on the change it missed
///   while frozen, and gets the next one.
/// - Before leaving, the first document calls a handler the case holds open,
///   and the case answers it only once the refused document has replaced it.
/// - The refused document defines the bridge's globals itself, so anything
///   evaluated into it lands in a recorder, and asks for the state and a
///   handler's reply on every transport it can reach; it must reach at least
///   one, and every request it makes there must be refused. It must see no
///   state, no reply, and no evaluation — the held call's reply included. The
///   witness proves a change was delivered while the refused document was
///   showing, and the subject is then read back after a round trip ordered
///   behind anything the change or the held reply could have sent it.
///   Engines that inject nothing into a refused document pass the seeding
///   check without running the seed's origin guard; the guard itself is
///   covered by the bridge script's own tests.
/// - Navigating on to an admitted document, the subject is seeded with the
///   current value and gets the next change.
///
/// `controller` is the engine's [`WebViewController`]. The views are built
/// through the same path [`WebView::open`](crate::WebView::open) takes at
/// render. The suite keeps the engine's event loop and the local executor
/// turning while this runs, and bounds the whole case with its own deadline:
/// every wait here is on an event the engine or a page reports.
///
/// # Panics
///
/// When an admitted document misses a change, the engine does not restore the
/// first document from its back/forward cache, or the refused document
/// received the state, a reply, or an evaluation.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
pub async fn mirrored_state_reaches_only_admitted_documents(controller: &WebViewController) {
    let admitted = PageServer::start(&[
        ("/admitted.js", "text/javascript", STATE_ADMITTED_JS),
        ("/witness", "text/html", STATE_ADMITTED_HTML),
        ("/one", "text/html", STATE_ADMITTED_HTML),
        ("/two", "text/html", STATE_ADMITTED_HTML),
        ("/three", "text/html", STATE_ADMITTED_HTML),
    ]);
    let refused = PageServer::start(&[("/hostile", "text/html", STATE_HOSTILE_HTML)]);

    let count = Binding::container(0_i64);
    let reports = Rc::new(Reports::default());
    let held = Rc::new(Held::default());
    let witness = state_view(controller, admitted.origin(), &count, &reports, &held).await;
    let subject = state_view(controller, admitted.origin(), &count, &reports, &held).await;

    let witness_url = admitted.url("/witness");
    witness.go_to(witness_url.clone());
    reports
        .next(0, |report| shown(report, &witness_url, false, 0))
        .await;

    // Seeded, then changed.
    let one = admitted.url("/one");
    let mark = reports.mark();
    subject.go_to(one.clone());
    reports
        .next(mark, |report| shown(report, &one, false, 0))
        .await;
    set_observed(&reports, &count, 1, &[&one, &witness_url]).await;

    // Same-document navigations keep the document's pull alive.
    let _: Value = eval(
        subject.handle(),
        "(() => { location.hash = 'moved'; history.pushState(null, '', '/one?pushed'); return location.href; })()",
    )
    .await;
    let pushed = admitted.url("/one?pushed");
    set_observed(&reports, &count, 2, &[&pushed]).await;

    // A cross-document navigation is seeded with the current value.
    let two = admitted.url("/two");
    let mark = reports.mark();
    subject.go_to(two.clone());
    reports
        .next(mark, |report| shown(report, &two, false, 2))
        .await;
    set_observed(&reports, &count, 3, &[&two]).await;

    // Back to the first document, restored from the back/forward cache with
    // the value it was frozen with; it catches up, and keeps up.
    let mark = reports.mark();
    subject.go_back();
    let restored = reports
        .next(mark, |report| {
            report["kind"] == "shown" && report["href"] == pushed.as_str()
        })
        .await;
    assert_eq!(
        restored["persisted"],
        Value::Bool(true),
        "going back must restore the first document from the back/forward cache, \
         or the restore path is not exercised: {restored}"
    );
    reports
        .next(mark, |report| observed(report, &pushed, 3))
        .await;
    set_observed(&reports, &count, 4, &[&pushed]).await;

    // A call whose document is gone by the time it is answered.
    let mark = reports.mark();
    let _ = raw_eval(subject.handle(), "(globalThis.waterui.invoke('hold'), 0)").await;
    reports.next(mark, |report| report["kind"] == "held").await;

    // The refused document.
    let hostile = refused.url("/hostile");
    load_refused(subject.handle(), &hostile).await;
    // Every message the document sent while it loaded has reached the host
    // once an evaluation issued after its load is answered: the engine
    // delivers a document's messages and its evaluation replies in order.
    let _ = raw_eval(subject.handle(), "0").await;
    // The handler reports its release in the same turn its reply is handed to
    // the engine, so by the time the case sees the report the reply is ahead
    // of every evaluation that follows.
    let mark = reports.mark();
    held.release();
    reports
        .next(mark, |report| report["kind"] == "released")
        .await;
    set_observed(&reports, &count, 5, &[&witness_url]).await;
    // The witness has seen the change, so the subject's own reply to it —
    // were there one — was handed to the engine before this evaluation.
    let _ = raw_eval(subject.handle(), "0").await;
    assert_untouched_by_the_host(&raw_eval(subject.handle(), "globalThis.__hostile").await);

    // Onward to an admitted document: seeded with the current value, and kept
    // up to date.
    let three = admitted.url("/three");
    let mark = reports.mark();
    subject.go_to(three.clone());
    reports
        .next(mark, |report| shown(report, &three, false, 5))
        .await;
    set_observed(&reports, &count, 6, &[&three]).await;
}

/// Asserts that the refused document's recorder saw the host do nothing: no
/// seed, no evaluation into its bridge globals, and no answer to any request
/// it made, while it did reach a transport and every promise-returning one
/// refused each request.
fn assert_untouched_by_the_host(record: &Value) {
    assert_eq!(
        record["injected"]["stateKeys"],
        serde_json::json!([]),
        "a refused document must not be seeded: {record}"
    );
    assert_eq!(
        record["calls"],
        serde_json::json!([]),
        "nothing may be evaluated into a refused document's bridge globals — \
         no state, no reply, and not the reply to its predecessor's held call: {record}"
    );
    let transports = record["transports"]
        .as_array()
        .unwrap_or_else(|| panic!("the refused document records its transports: {record}"));
    assert!(
        !transports.is_empty(),
        "the refused document must reach a transport, or its requests test nothing: {record}"
    );
    let outcomes = record["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("the refused document records its outcomes: {record}"));
    assert!(
        outcomes.iter().all(|outcome| outcome[2] != "resolved"),
        "a refused document's request must never be answered: {record}"
    );
    // A transport that returns a promise settles it, so each request it
    // carried was delivered and refused rather than lost.
    for transport in transports
        .iter()
        .filter(|transport| *transport == "webkit" || *transport == "native")
    {
        for id in 1..=3 {
            assert!(
                outcomes.iter().any(|outcome| outcome[0] == *transport
                    && outcome[1] == id
                    && outcome[2] == "rejected"),
                "request {id} on the `{transport}` transport must be refused: {record}"
            );
        }
    }
}

const STATE_ADMITTED_HTML: &str = include_str!("conformance/state/admitted.html");
const STATE_ADMITTED_JS: &str = include_str!("conformance/state/admitted.js");
const STATE_HOSTILE_HTML: &str = include_str!("conformance/state/hostile.html");

/// Opens a blank view that mirrors `count` and admits only `origin`, built the
/// way [`WebView::open`](crate::WebView::open) builds one.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
async fn state_view(
    controller: &WebViewController,
    origin: &str,
    count: &Binding<i64>,
    reports: &Rc<Reports>,
    held: &Rc<Held>,
) -> crate::WebView {
    let blank: Url = "about:blank".parse().expect("about:blank is a URL");
    let hold_handler = {
        let reports = Rc::clone(reports);
        let held = Rc::clone(held);
        move |crate::Json(_): crate::Json<Value>| {
            let released = held.hold();
            reports.push(serde_json::json!({ "kind": "held" }));
            let reports = Rc::clone(&reports);
            async move {
                released
                    .await
                    .expect("the conformance case releases every call it holds");
                reports.push(serde_json::json!({ "kind": "released" }));
                crate::Json(Value::from("held"))
            }
        }
    };
    let reports = Rc::clone(reports);
    let view = crate::WebView::open(blank)
        .bridge_origins([Str::from(origin.to_owned())])
        .expose("count", count.clone())
        .handler(
            "echo",
            |crate::Json(value): crate::Json<Value>| async move { crate::Json(value) },
        )
        .handler("report", move |crate::Json(report): crate::Json<Value>| {
            reports.push(report);
            async {}
        })
        .handler("hold", hold_handler)
        .create(controller, &waterui_core::Environment::new());
    // The installs above are commands nobody awaits; an evaluation answered
    // after them orders them before the first navigation.
    let _ = raw_eval(view.handle(), "0").await;
    view
}

/// Sets `count` to `value` and waits until every document in `documents` has
/// observed it.
#[expect(
    clippy::future_not_send,
    reason = "reports arrive on the UI thread, which is the one that waits for them"
)]
async fn set_observed(reports: &Reports, count: &Binding<i64>, value: i64, documents: &[&Url]) {
    let mark = reports.mark();
    count.set(value);
    for document in documents {
        reports
            .next(mark, |report| observed(report, document, value))
            .await;
    }
}

fn shown(report: &Value, url: &Url, persisted: bool, value: i64) -> bool {
    report["kind"] == "shown"
        && report["href"] == url.as_str()
        && report["persisted"] == persisted
        && report["value"] == value
}

fn observed(report: &Value, url: &Url, value: i64) -> bool {
    report["kind"] == "observed" && report["href"] == url.as_str() && report["value"] == value
}

/// The handler call the case holds open, answered when the case releases it.
#[derive(Default)]
struct Held {
    release: RefCell<Option<oneshot::Sender<()>>>,
}

impl Held {
    /// Holds a new call, which completes once [`release`](Self::release)
    /// runs.
    fn hold(&self) -> oneshot::Receiver<()> {
        let (release, released) = oneshot::channel();
        let previous = self.release.borrow_mut().replace(release);
        assert!(
            previous.is_none(),
            "the conformance case holds one call at a time"
        );
        released
    }

    /// Answers the held call.
    fn release(&self) {
        self.release
            .borrow_mut()
            .take()
            .expect("a call is held")
            .send(())
            .expect("the held call is still waiting for its answer");
    }
}

/// What admitted documents reported through the `report` handler.
#[derive(Default)]
struct Reports {
    entries: RefCell<Vec<Value>>,
    waker: RefCell<Option<Waker>>,
}

impl Reports {
    fn push(&self, report: Value) {
        self.entries.borrow_mut().push(report);
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }

    /// Where the reports that follow will start.
    fn mark(&self) -> usize {
        self.entries.borrow().len()
    }

    /// The first report at or after `mark` that `matches` accepts, once one
    /// arrives.
    #[expect(
        clippy::future_not_send,
        reason = "reports arrive on the UI thread, which is the one that waits for them"
    )]
    async fn next(&self, mark: usize, matches: impl Fn(&Value) -> bool) -> Value {
        std::future::poll_fn(|context| {
            if let Some(report) = self.entries.borrow()[mark..]
                .iter()
                .find(|report| matches(report))
            {
                return std::task::Poll::Ready(report.clone());
            }
            *self.waker.borrow_mut() = Some(context.waker().clone());
            std::task::Poll::Pending
        })
        .await
    }
}

/// Navigates to a document the bridge refuses and returns once it has loaded.
///
/// The document cannot report through the bridge, so this waits on the
/// engine's `Loaded` and then confirms which document it is.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
async fn load_refused(handle: &crate::AnyWebViewHandle, url: &Url) {
    let state = Rc::new(RefCell::new(LoadState::default()));
    let _watcher = handle.watch({
        let state = Rc::clone(&state);
        move |event| {
            let mut state = state.borrow_mut();
            state.events.push(event);
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
    });
    handle.go_to(url);
    let mut loaded_seen = 0_usize;
    loop {
        loaded_seen = std::future::poll_fn(|context| {
            let mut state = state.borrow_mut();
            let loaded = state
                .events
                .iter()
                .filter(|event| matches!(event, BackendEvent::Event(WebViewEvent::Loaded)))
                .count();
            if loaded > loaded_seen {
                return std::task::Poll::Ready(loaded);
            }
            state.waker = Some(context.waker().clone());
            std::task::Poll::Pending
        })
        .await;
        if raw_eval(handle, "location.href").await == url.as_str() {
            break;
        }
    }
}

/// Evaluates `source` through the engine's raw path, which works in any
/// document — a refused one has no `__wateruiEval` of ours to go through —
/// and decodes the JSON it answers.
#[expect(
    clippy::future_not_send,
    reason = "web views are confined to the UI thread, and so is the suite that drives them"
)]
async fn raw_eval(handle: &crate::AnyWebViewHandle, source: &str) -> Value {
    let raw = handle
        .run_javascript(source)
        .await
        .unwrap_or_else(|error| panic!("`{source}` failed to run: {error}"));
    serde_json::from_str(raw.as_str())
        .unwrap_or_else(|error| panic!("`{source}` answered `{raw}`, not JSON: {error}"))
}

/// An HTTP server for one origin, answering a fixed set of pages from its own
/// thread until it is dropped.
///
/// Its own thread, because the suites drive their engines from different
/// loops, and a server none of them has to turn works under every one.
struct PageServer {
    server: Arc<tiny_http::Server>,
    origin: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PageServer {
    fn start(pages: &[(&'static str, &'static str, &'static str)]) -> Self {
        let server = Arc::new(
            tiny_http::Server::http("127.0.0.1:0").expect("bind a conformance page server"),
        );
        let address = server
            .server_addr()
            .to_ip()
            .expect("the page server listens on a TCP port");
        let pages = pages.to_vec();
        let thread = std::thread::spawn({
            let server = Arc::clone(&server);
            move || {
                for request in server.incoming_requests() {
                    let page = pages
                        .iter()
                        .find(|(path, _, _)| request.url().split('?').next() == Some(*path));
                    let response = match page {
                        Some((_, content_type, body)) => {
                            let header = tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                format!("{content_type}; charset=utf-8").as_bytes(),
                            )
                            .expect("a static content type is a valid header");
                            tiny_http::Response::from_string(*body).with_header(header)
                        }
                        // Engines ask for things nobody linked, a favicon
                        // above all; saying so is the correct answer.
                        None => {
                            tiny_http::Response::from_string(String::new()).with_status_code(404)
                        }
                    };
                    // An engine may drop a connection it no longer needs, such
                    // as a favicon request for a page it navigated away from.
                    let _ = request.respond(response);
                }
            }
        });
        Self {
            server,
            origin: format!("http://{address}"),
            thread: Some(thread),
        }
    }

    fn origin(&self) -> &str {
        &self.origin
    }

    fn url(&self, path: &str) -> Url {
        format!("{}{path}", self.origin)
            .parse()
            .expect("a page server URL")
    }
}

impl Drop for PageServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the page server thread exits cleanly");
        }
    }
}
