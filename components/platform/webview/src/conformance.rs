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

use serde_json::Value;
use waterui_core::Str;

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
