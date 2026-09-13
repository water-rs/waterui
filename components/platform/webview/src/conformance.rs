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

use serde_json::Value;
use waterui_core::Str;
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
    let webview = controller.open_with(WebViewConfig {
        asset_server: Some(bundled_site_server()),
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
