//! The JNI-free halves of the Android `WebView` bridge, unit-tested on the
//! host.
//!
//! Compiled on Android behind `hydrolysis_android_system_webview` and on the
//! host for tests, so the rule conversions, the cookie URL derivation, the
//! script compositions and the async-result parsing are checked without a
//! device. The Kotlin `HydrolysisWebView` mirrors these one-to-one — the
//! semantics here are `WebViewComponent.kt`'s, ported.

use waterui_webview::{Cookie, OriginPolicy};

/// The token `OriginRule::LocalFiles` crosses as; Android spells a local-file
/// origin rule `file://`.
const LOCAL_FILES_TOKEN: &str = "file:";

/// The origin rule Android's `WebViewCompat` APIs take for a local document —
/// an origin is `scheme://host[:port]` and a local document's host is empty.
pub const ANDROID_LOCAL_FILE_RULE: &str = "file://";

/// The name the bridge object is injected under — the transport script posts
/// envelopes to it.
pub const BRIDGE_OBJECT: &str = "__wateruiBridge";

/// The name the async-result object is injected under.
pub const ASYNC_RESULT_OBJECT: &str = "__wateruiAsyncResult";

/// The string a launched `call_async_javascript` synchronously evaluates to.
#[cfg(test)]
pub const ASYNC_CALL_SENTINEL: &str = "__wateruiAsyncCallStarted";

/// [`ASYNC_CALL_SENTINEL`] as `evaluateJavascript` delivers it — JSON-quoted.
pub const ASYNC_CALL_STARTED: &str = "\"__wateruiAsyncCallStarted\"";

/// The transport adapter, injected before the shared bridge script so
/// `__wateruiSend` exists when it runs.
pub const TRANSPORT_SCRIPT: &str = include_str!("android/transport.js");

/// The async-evaluation wrapper, a function expression the caller composes
/// with [`compose_async_call`].
pub const ASYNC_CALL: &str = include_str!("android/async_call.js");

/// The `DOMContentLoaded` hold for document-end scripts, a function
/// expression the caller composes with [`compose_document_end`].
pub const DOCUMENT_END: &str = include_str!("android/document_end.js");

/// The policy rendered into the origin-rule strings `WebViewCompat`'s
/// document-start injection and web-message listener take.
///
/// Ported from `WebViewComponent.injectionRules`: the tokens already carry
/// the `scheme://host[:port]` shape Android's *origin* rules match; only the
/// local-file token differs, because Android spells it `file://`. The URI
/// globs `OriginRule::injection_pattern` renders are for engines that filter
/// on a URL pattern — a path is not part of an origin, and Android rejects a
/// rule that carries one.
///
/// An empty policy yields an empty set, which Kotlin turns into "nothing is
/// installed at all" rather than a filter that admits nothing.
#[must_use]
pub fn androidx_origin_rules(policy: &OriginPolicy) -> Vec<String> {
    policy
        .rules()
        .iter()
        .map(|rule| {
            let token = rule.as_token();
            if token == LOCAL_FILES_TOKEN {
                ANDROID_LOCAL_FILE_RULE.to_owned()
            } else {
                token.to_owned()
            }
        })
        .collect()
}

/// Whether a frame reporting `origin` may use the bridge, decided against the
/// converted rules [`androidx_origin_rules`] produces.
///
/// The comparison is `OriginPolicy::allows_origin`'s, spelled over the
/// Android rules: `*` admits every origin, `file://` any local document, and
/// an exact rule the `scheme://host[:port]` string the frame reports. An
/// opaque origin — a `data:` document, a sandboxed frame — reports an empty
/// origin and matches nothing but `*`, which is the point: it cannot be
/// authenticated.
#[cfg(test)]
#[must_use]
pub fn origin_may_use_bridge(rules: &[String], origin: &str) -> bool {
    rules.iter().any(|rule| match rule.as_str() {
        "*" => true,
        ANDROID_LOCAL_FILE_RULE => origin.starts_with(ANDROID_LOCAL_FILE_RULE),
        rule => rule == origin,
    })
}

/// The URL a `Set-Cookie` value is stored under.
///
/// Ported from `WebViewComponent.cookieUrl`: a cookie naming a `Domain`
/// attribute is stored against `https://<domain>` — `https` so a `Secure`
/// cookie is accepted — and only a cookie without one falls back to the
/// document's URL, which is `None` before the first navigation. Returning
/// `None` is the same refusal the Kotlin runtime logs.
#[must_use]
pub fn cookie_url(cookie: &Cookie<'static>, current_url: Option<&str>) -> Option<String> {
    cookie.domain().map_or_else(
        || current_url.map(ToOwned::to_owned),
        |domain| Some(format!("https://{}", domain.trim_start_matches('.'))),
    )
}

/// Composes the script a `call_async_javascript` call evaluates.
///
/// The composition `components/platform/webview/src/script.rs` spells: the
/// function expression applied to the call id and the body wrapped in an
/// `async` function. The synchronous reply settles the launch — it is
/// [`ASYNC_CALL_STARTED`] when the promise was actually posted.
#[must_use]
pub fn compose_async_call(id: u64, body: &str) -> String {
    format!("({ASYNC_CALL})({id}, async function () {{\n{body}\n}});")
}

/// Wraps a document-end script in the `DOMContentLoaded` hold.
#[must_use]
pub fn compose_document_end(script: &str) -> String {
    format!("({DOCUMENT_END})(function () {{\n{script}\n}});")
}

/// The `{id, ok, value}` envelope the async-result object posts.
#[derive(Debug, serde::Deserialize)]
pub struct AsyncResult {
    /// The call the result settles.
    pub id: u64,
    /// Whether the promise resolved rather than rejected.
    pub ok: bool,
    /// The settled payload — the shared wrapper's JSON envelope, or the
    /// rejection message. Required, as the Kotlin runtime's
    /// `envelope.getString(...)` requires it.
    pub value: String,
}

/// Parses one async-result envelope.
///
/// # Errors
///
/// The serde error, unchanged — a malformed payload is a contract break the
/// caller surfaces, never a call to settle silently.
pub fn parse_async_result(payload: &str) -> Result<AsyncResult, serde_json::Error> {
    serde_json::from_str(payload)
}

/// Parses the cookie request-header form `name=value; name2=value2` that
/// `CookieManager.getCookie` returns — already split into lines by Kotlin,
/// one `name=value` per line.
///
/// `CookieManager` exposes only this form, so every cookie carries a name
/// and a value and nothing else (`WebViewHandle::get_cookies` documents it).
#[must_use]
pub fn parse_cookie_header(header: &str) -> Vec<Cookie<'static>> {
    header
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            Some(Cookie::new(name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_webview::{BridgeOrigins, Url};

    fn policy(origins: BridgeOrigins, initial: &str) -> OriginPolicy {
        let initial: Url = initial.parse().expect("a valid initial URL");
        OriginPolicy::new(origins, &initial)
    }

    #[test]
    fn origin_rules_cover_every_policy_shape() {
        // Every `OriginPolicy` shape, its converted rules and the admission
        // they imply — the two are checked together because Kotlin applies
        // them as one.
        let cases: &[(&[String], &str, bool)] = &[
            (&["*".to_owned()], "https://evil.example", true),
            (&["file://".to_owned()], "file://", true),
            (&["file://".to_owned()], "file:///tmp/app.html", true),
            (&["file://".to_owned()], "https://app.dev", false),
            (
                &["https://app.dev".to_owned()],
                "https://app.dev:8443",
                false,
            ),
            (
                &["http://localhost:3000".to_owned()],
                "http://localhost",
                false,
            ),
            (
                &["http://localhost:3000".to_owned()],
                "http://localhost:3000",
                true,
            ),
            (&["https://app.dev".to_owned()], "https://app.dev", true),
            // An opaque origin reports empty and matches nothing but `*`.
            (&["https://app.dev".to_owned()], "", false),
            (&["*".to_owned()], "", true),
        ];
        for (rules, origin, expected) in cases {
            assert_eq!(
                origin_may_use_bridge(rules, origin),
                *expected,
                "rules {rules:?} against {origin:?}"
            );
        }

        assert_eq!(
            androidx_origin_rules(&policy(BridgeOrigins::Any, "https://a.dev")),
            vec!["*"],
        );
        assert_eq!(
            androidx_origin_rules(&policy(BridgeOrigins::LocalFiles, "file:///a.html")),
            vec!["file://"],
        );
        assert_eq!(
            androidx_origin_rules(&policy(BridgeOrigins::Initial, "https://app.dev/start",)),
            vec!["https://app.dev"],
        );
        assert_eq!(
            androidx_origin_rules(&policy(
                BridgeOrigins::Allowed(vec!["https://app.dev".into(), "https://docs.dev".into()]),
                "https://app.dev",
            )),
            vec!["https://app.dev", "https://docs.dev"],
        );
        // Deny-all stays distinguishable from allow-all.
        assert_eq!(
            androidx_origin_rules(&policy(BridgeOrigins::Initial, "file:///a.html")),
            [] as [String; 0]
        );
    }

    #[test]
    fn cookie_url_derivation() {
        // A `Domain` attribute — including the leading-dot form — is stored
        // under `https://<domain>`; without one the cookie goes to the
        // document's own URL, and before any navigation there is nowhere to
        // put it.
        let domain = Cookie::build(("n", "v")).domain(".example.com").build();
        assert_eq!(
            cookie_url(&domain, None).as_deref(),
            Some("https://example.com")
        );

        let host_only = Cookie::new("n", "v");
        assert_eq!(
            cookie_url(&host_only, Some("https://app.dev/page")).as_deref(),
            Some("https://app.dev/page"),
        );
        assert_eq!(cookie_url(&host_only, None), None);
    }

    #[test]
    fn compositions_carry_the_sentinel_and_object_names() {
        let call = compose_async_call(7, "return 1;");
        assert!(call.contains(ASYNC_CALL_SENTINEL));
        assert!(call.contains(ASYNC_RESULT_OBJECT));
        assert!(call.contains("async function () {\nreturn 1;\n}"));
        assert!(call.contains("(7,"));

        let held = compose_document_end("document.title = 'x';");
        assert!(held.contains("DOMContentLoaded"));
        assert!(held.contains("document.title = 'x';"));
    }

    #[test]
    fn each_js_file_names_the_constant_it_relies_on() {
        assert!(TRANSPORT_SCRIPT.contains(BRIDGE_OBJECT));
        assert!(ASYNC_CALL.contains(ASYNC_RESULT_OBJECT));
        assert!(ASYNC_CALL.contains(ASYNC_CALL_SENTINEL));
        assert!(DOCUMENT_END.contains("(function (run)"));
    }

    #[test]
    fn async_result_parsing() {
        let settled = parse_async_result(r#"{"id":3,"ok":true,"value":"\"done\""}"#)
            .expect("a well-formed envelope parses");
        assert_eq!(settled.id, 3);
        assert!(settled.ok);
        assert_eq!(settled.value, "\"done\"");

        // A malformed payload is an error, not a settled call.
        for bad in [
            "{}",
            "not json",
            r#"{"id":1,"ok":true}"#,
            r#"{"id":"x","ok":true,"value":""}"#,
        ] {
            assert!(parse_async_result(bad).is_err(), "{bad} must not parse");
        }
    }

    #[test]
    fn cookie_replies_parse_to_name_and_value_only() {
        // The Kotlin reply is a newline-joined request header — every
        // attribute the real `Set-Cookie` carried is already gone, so the
        // parsed cookies carry exactly name and value.
        let cookies = parse_cookie_header("session=abc123\nempty=\n\nx-a-b = =v=");
        assert_eq!(cookies.len(), 3);
        assert_eq!(cookies[0].name(), "session");
        assert_eq!(cookies[0].value(), "abc123");
        assert_eq!(cookies[1].name(), "empty");
        assert_eq!(cookies[1].value(), "");
        assert_eq!(cookies[2].name(), "x-a-b");
        assert_eq!(cookies[2].value(), "=v=");
        assert_eq!(parse_cookie_header(""), [] as [Cookie<'_>; 0]);
        assert_eq!(parse_cookie_header("no-equals"), [] as [Cookie<'_>; 0]);
    }

    #[test]
    fn the_started_reply_is_the_json_quoted_sentinel() {
        // `evaluateJavascript` hands replies back as JSON, so the settle
        // path matches the quoted form of the wrapper's return value.
        assert_eq!(ASYNC_CALL_STARTED, format!("\"{ASYNC_CALL_SENTINEL}\""));
    }
}
