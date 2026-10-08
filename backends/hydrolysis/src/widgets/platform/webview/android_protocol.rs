//! The JNI-free halves of the Android `WebView` bridge, unit-tested on the
//! host.
//!
//! Compiled on Android behind `hydrolysis_android_system_webview` and on the
//! host for tests, so the rule conversions, the cookie URL derivation, the
//! script compositions and the async-result parsing are checked without a
//! device. The Kotlin `HydrolysisWebView` mirrors these one-to-one — the
//! semantics here are `WebViewComponent.kt`'s, ported.

use std::collections::HashMap;

use waterui_core::{Computed, Signal, Str};
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
///
/// `generation` and `token` stamp the result envelope: the generation is
/// the document the call was issued against, so a result posted by an old
/// document can never settle a call the new one waits on, and the token is
/// the per-call secret an iframe forging `{id, ok, value}` cannot guess.
#[must_use]
pub fn compose_async_call(id: u64, generation: u64, token: u64, body: &str) -> String {
    format!("({ASYNC_CALL})({id}, {generation}, {token}, async function () {{\n{body}\n}});")
}

/// Install `apply` as `signal`'s watcher after running it once on the
/// current value. `Computed::watch` fires only on change — and a
/// constant's watch never fires — so the initial state would otherwise
/// never reach the view. `set_redirects_enabled` is the Android bridge's
/// one signal-driven setting; every other field crosses as an immediate
/// setter call.
pub fn watch_bool_with_initial(
    signal: &Computed<bool>,
    apply: impl Fn(bool) + 'static,
) -> nami::watcher::BoxWatcherGuard {
    apply(signal.snapshot());
    signal.watch(move |ctx| apply(ctx.into_value()))
}

/// A per-call token `compose_async_call` embeds and `settle_async`
/// requires back: `RandomState`'s per-process keys mean a page forging a
/// result envelope cannot guess the value the real call carries.
#[must_use]
pub fn fresh_async_token(id: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(id);
    hasher.finish()
}

/// Wraps a document-end script in the `DOMContentLoaded` hold.
#[must_use]
pub fn compose_document_end(script: &str) -> String {
    format!("({DOCUMENT_END})(function () {{\n{script}\n}});")
}

/// The `{id, generation, token, ok, value}` envelope the async-result
/// object posts.
#[derive(Debug, serde::Deserialize)]
pub struct AsyncResult {
    /// The call the result settles.
    pub id: u64,
    /// The document generation the call was issued against — stale when
    /// `onPageStarted` committed a new document since.
    pub generation: u64,
    /// The per-call token `compose_async_call` embedded in the script.
    pub token: u64,
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

/// A pending `run_javascript`/`call_async_javascript`/`get_cookies` call
/// waiting on its matching native, keyed by call id. The Kotlin runtime's
/// `JsCompletion` trio, flattened into one registry: Kotlin reports every
/// settlement — completed or abandoned — through `nativeJsResult`, and only
/// a live async call hears from `nativeAsyncResult` afterwards.
///
/// The settlers are `FnOnce` rather than a channel so this module stays
/// JNI- and executor-free: the Android side wraps a `oneshot::Sender`, the
/// tests collect what they are handed.
pub enum PendingCall {
    /// `run_javascript`: the synchronous `evaluateJavascript` reply is the
    /// script's JSON answer.
    JavaScript(Box<dyn FnOnce(Result<Str, Str>)>),
    /// `call_async_javascript`: a reply of [`ASYNC_CALL_STARTED`] means the
    /// promise was posted and the real result comes through
    /// `nativeAsyncResult`; anything else failed at launch. `token` is the
    /// per-call secret the result envelope must echo — a page that can read
    /// the call id cannot forge the token.
    Async {
        settle: Box<dyn FnOnce(Result<Str, Str>)>,
        token: u64,
    },
    /// `get_cookies`: the reply is the `CookieManager` header the native
    /// splits into pairs.
    Cookies(Box<dyn FnOnce(Vec<Cookie<'static>>)>),
}

impl PendingCall {
    /// Settle as `reason`-failed — a document replacement or teardown
    /// answer, not a script result. A cookie call gets the empty jar, the
    /// answer a closed view can still give.
    fn abandon(self, reason: &str) {
        match self {
            Self::JavaScript(settle) | Self::Async { settle, .. } => {
                settle(Err(Str::from(reason.to_string())));
            }
            Self::Cookies(settle) => settle(Vec::new()),
        }
    }
}

/// The in-flight calls of one web view, keyed by call id — the single place
/// every settlement path consults, so navigation, release and teardown can
/// settle all of them at once and a receiver is never left unsettled.
#[derive(Default)]
pub struct PendingCalls {
    calls: HashMap<u64, PendingCall>,
    /// The document generation live calls were issued against, bumped on
    /// every main-frame cross-document commit — an async result envelope
    /// stamped with an older generation is a stale page's, never a live
    /// call's answer.
    generation: u64,
    /// Set by [`Self::release`]: the view is gone (a `release`, or the
    /// render process died while the instance stays registered), so no
    /// Kotlin reply can ever arrive and `begin` settles new calls at once.
    dead: bool,
}

impl PendingCalls {
    /// Register `call` under `id` before the Kotlin call that answers it is
    /// issued — the reply may land on the same call.
    ///
    /// On a dead view the call settles at once and `false` is returned, so
    /// the caller skips issuing the Kotlin call it could never answer.
    pub fn begin(&mut self, id: u64, call: PendingCall) -> bool {
        if self.dead {
            call.abandon("the web view was closed");
            return false;
        }
        self.calls.insert(id, call);
        true
    }

    /// The generation a new call is issued against — `compose_async_call`
    /// embeds it into the script and `settle_async` requires it back.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// `onPageStarted` committed a new main-frame document: bump the
    /// generation so the old document's results die with it, and hand back
    /// the calls the old document can no longer answer.
    ///
    /// The calls are returned, not settled: the settlement closures run in
    /// [`Self::settle_many`] after the registry's borrow ends, so a settler
    /// can never re-enter the registry while it is mutably borrowed.
    pub fn document_replaced(&mut self) -> Vec<(u64, PendingCall)> {
        self.generation += 1;
        std::mem::take(&mut self.calls).into_iter().collect()
    }

    /// `release` — Rust-driven or render-process-gone — marks the registry
    /// dead and hands back the calls it drained; [`Self::settle_many`]
    /// settles them outside the borrow.
    pub fn release(&mut self) -> Vec<(u64, PendingCall)> {
        self.dead = true;
        std::mem::take(&mut self.calls).into_iter().collect()
    }

    /// Settle the calls `document_replaced`/`release`/drop drained, each as
    /// `reason`-failed.
    pub fn settle_many(calls: Vec<(u64, PendingCall)>, reason: &str) {
        for (id, call) in calls {
            tracing::debug!("android webview: call {id} abandoned: {reason}");
            call.abandon(reason);
        }
    }

    /// A synchronous `evaluateJavascript`-style reply (`nativeJsResult`,
    /// `nativeCookies`, or Kotlin's per-call abandonment): `ok`/`value`
    /// carry the JavaScript outcome for a script call, the failure message
    /// for an abandoned call, and the cookie header for `get_cookies`.
    /// An async call whose reply is the started sentinel is re-armed under
    /// the same id for `settle_async`.
    pub fn settle(&mut self, id: u64, ok: bool, value: &str) {
        let Some(call) = self.calls.remove(&id) else {
            tracing::debug!("android webview: a reply for call {id} arrived with nothing pending");
            return;
        };
        match call {
            PendingCall::JavaScript(settle) => settle(if ok {
                Ok(Str::from(value.to_string()))
            } else {
                Err(Str::from(value.to_string()))
            }),
            PendingCall::Async { settle, token } => {
                if ok && value == ASYNC_CALL_STARTED {
                    // The promise posted: wait for `nativeAsyncResult`,
                    // which carries the same id.
                    self.calls.insert(id, PendingCall::Async { settle, token });
                } else {
                    // A launch failure — a parse error, a page gone under
                    // the call, or Kotlin's abandonment of a live call.
                    settle(Err(Str::from(value.to_string())));
                }
            }
            PendingCall::Cookies(settle) => settle(parse_cookie_header(value)),
        }
    }

    /// Settle an async call from its posted `{id, generation, token, ok,
    /// value}` envelope. Three checks reject what a page could forge: the
    /// generation must be the live document's (an old document's envelope
    /// can never settle a new call), the id must name a re-armed
    /// [`PendingCall::Async`], and the token must be the one
    /// `compose_async_call` embedded — the Kotlin listener's `isMainFrame`
    /// check is transport plumbing, this is the admission control.
    pub fn settle_async(&mut self, payload: &str) {
        let result = match parse_async_result(payload) {
            Ok(result) => result,
            Err(error) => {
                // A malformed envelope cannot identify its call; surface
                // the breach rather than guess.
                tracing::warn!("android webview: malformed async result: {error}");
                return;
            }
        };
        if result.generation != self.generation {
            tracing::warn!(
                "android webview: an async result for document {} arrived in document {}",
                result.generation,
                self.generation
            );
            return;
        }
        match self.calls.get(&result.id) {
            Some(PendingCall::Async { token, .. }) => {
                if *token != result.token {
                    tracing::warn!(
                        "android webview: an async result for call {} carried the wrong token",
                        result.id
                    );
                    return;
                }
                let Some(PendingCall::Async { settle, .. }) = self.calls.remove(&result.id) else {
                    unreachable!("the entry was checked above");
                };
                let value = Str::from(result.value);
                settle(if result.ok { Ok(value) } else { Err(value) });
            }
            Some(_) => {
                tracing::warn!(
                    "android webview: an async result addressed call {}, not an async call",
                    result.id
                );
            }
            None => tracing::debug!(
                "android webview: an async result for call {} arrived with nothing pending",
                result.id
            ),
        }
    }

}

/// The `HydrolysisWebView` members Rust calls by name — the webview module's
/// half of `runner::android_methods`' host contract. `install_controller`
/// resolves each entry with `GetMethodID` once per session, so a member R8
/// stripped or renamed fails the create, not the first call that reaches for
/// it; Kotlin keeps the same set under `@CalledFromNative` and the agreement
/// test below reads them back out of `HydrolysisWebView.kt`.
///
/// A method's id in [`WEBVIEW_METHODS`] is its [`WebViewMethodId`] position —
/// the only handle a call site names a wrapper method by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebViewMethodId {
    GoBack,
    GoForward,
    LoadUrl,
    StopLoading,
    Reload,
    SetUserAgent,
    CanGoBack,
    CanGoForward,
    SetDocumentStartScripts,
    SetBridgeOrigins,
    SetRedirectsEnabled,
    SetCookie,
    GetCookies,
    Evaluate,
    EvaluateBridgeScript,
    Release,
}

impl WebViewMethodId {
    /// Every id, in table order.
    pub const ALL: [Self; 16] = [
        Self::GoBack,
        Self::GoForward,
        Self::LoadUrl,
        Self::StopLoading,
        Self::Reload,
        Self::SetUserAgent,
        Self::CanGoBack,
        Self::CanGoForward,
        Self::SetDocumentStartScripts,
        Self::SetBridgeOrigins,
        Self::SetRedirectsEnabled,
        Self::SetCookie,
        Self::GetCookies,
        Self::Evaluate,
        Self::EvaluateBridgeScript,
        Self::Release,
    ];
}

/// One `HydrolysisWebView` method the native side calls: its JNI name and
/// signature, resolved into a cached `jmethodID` once per session.
pub struct WebViewMethod {
    pub name: &'static str,
    pub signature: &'static str,
}

/// The instance-method contract, in `WebViewMethodId` order.
pub const WEBVIEW_METHODS: &[WebViewMethod] = &[
    WebViewMethod {
        name: "goBack",
        signature: "()V",
    },
    WebViewMethod {
        name: "goForward",
        signature: "()V",
    },
    WebViewMethod {
        name: "loadUrl",
        signature: "(Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "stopLoading",
        signature: "()V",
    },
    WebViewMethod {
        name: "reload",
        signature: "()V",
    },
    WebViewMethod {
        name: "setUserAgent",
        signature: "(Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "canGoBack",
        signature: "()Z",
    },
    WebViewMethod {
        name: "canGoForward",
        signature: "()Z",
    },
    WebViewMethod {
        name: "setDocumentStartScripts",
        signature: "([Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "setBridgeOrigins",
        signature: "([Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "setRedirectsEnabled",
        signature: "(Z)V",
    },
    WebViewMethod {
        name: "setCookie",
        signature: "(Ljava/lang/String;Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "getCookies",
        signature: "(J)V",
    },
    WebViewMethod {
        name: "evaluate",
        signature: "(Ljava/lang/String;J)V",
    },
    WebViewMethod {
        name: "evaluateBridgeScript",
        signature: "(Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "release",
        signature: "()V",
    },
];

/// `create` is the companion's static factory — `GetStaticMethodID`, beside
/// the instance table. The trailing strings are the bridge and async-result
/// object names, the local-file origin rule and the asset host Kotlin
/// applies verbatim.
pub const WEBVIEW_CREATE: WebViewMethod = WebViewMethod {
    name: "create",
    signature: "(Ldev/waterui/hydrolysis/HydrolysisSession;JJJLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ldev/waterui/hydrolysis/webview/HydrolysisWebView;",
};

/// The `AssetResponse` constructor `nativeAssetRespond` calls by name.
pub const WEBVIEW_ASSET_RESPONSE_INIT: WebViewMethod = WebViewMethod {
    name: "<init>",
    signature: "(ILjava/lang/String;[B)V",
};

const _: () = assert!(
    WEBVIEW_METHODS.len() == WebViewMethodId::ALL.len(),
    "WEBVIEW_METHODS and WebViewMethodId disagree"
);

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
        let call = compose_async_call(7, 3, 42, "return 1;");
        assert!(call.contains(ASYNC_CALL_SENTINEL));
        assert!(call.contains(ASYNC_RESULT_OBJECT));
        assert!(call.contains("async function () {\nreturn 1;\n}"));
        assert!(call.contains("(7, 3, 42,"));

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
        let settled =
            parse_async_result(r#"{"id":3,"generation":2,"token":9,"ok":true,"value":"\"done\""}"#)
                .expect("a well-formed envelope parses");
        assert_eq!(settled.id, 3);
        assert_eq!(settled.generation, 2);
        assert_eq!(settled.token, 9);
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

    // ---- PendingCalls: the sequences the shipped code runs ----

    type Results = std::rc::Rc<std::cell::RefCell<Vec<Result<String, String>>>>;
    type CookieResults = std::rc::Rc<std::cell::RefCell<Vec<Vec<Cookie<'static>>>>>;

    fn collect() -> (Results, impl Fn(Result<Str, Str>)) {
        let out = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = std::rc::Rc::clone(&out);
        (out, move |result: Result<Str, Str>| {
            sink.borrow_mut()
                .push(result.map(|v| v.to_string()).map_err(|e| e.to_string()));
        })
    }

    fn collect_cookies() -> (CookieResults, impl Fn(Vec<Cookie<'static>>)) {
        let out = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = std::rc::Rc::clone(&out);
        (out, move |cookies| sink.borrow_mut().push(cookies))
    }

    /// The async-call issuance the Android side performs: `begin` under the
    /// call id with the token `compose_async_call` embeds in the script.
    fn begin_async(pending: &mut PendingCalls, id: u64, settle: impl Fn(Result<Str, Str>) + 'static) -> u64 {
        let token = fresh_async_token(id);
        assert!(pending.begin(id, PendingCall::Async { settle: Box::new(settle), token }));
        token
    }

    /// The JSON envelope the composed script posts for a call.
    fn envelope(id: u64, generation: u64, token: u64, ok: bool, value: &str) -> String {
        format!(
            r#"{{"id":{id},"generation":{generation},"token":{token},"ok":{ok},"value":"{value}"}}"#
        )
    }

    #[test]
    fn pending_calls_the_sentinel_rearms_and_the_result_settles() {
        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let token = begin_async(&mut pending, 1, settle);
        // The synchronous reply is the started sentinel: still in flight.
        pending.settle(1, true, ASYNC_CALL_STARTED);
        assert!(results.borrow().is_empty());
        // The posted envelope then settles it.
        pending.settle_async(&envelope(1, 0, token, true, "\\\"v\\\""));
        assert_eq!(results.borrow().as_slice(), &[Ok("\"v\"".to_owned())]);
    }

    #[test]
    fn pending_calls_a_cross_document_commit_settles_everything() {
        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let (cookies, settle_cookies) = collect_cookies();
        let _ = begin_async(&mut pending, 1, settle);
        assert!(pending.begin(2, PendingCall::Cookies(Box::new(settle_cookies))));
        // A synchronous `evaluate` call settles with the same drain —
        // nothing the old document owed survives the commit.
        let (results_js, settle_js) = collect();
        assert!(pending.begin(3, PendingCall::JavaScript(Box::new(settle_js))));
        let generation = pending.generation();
        let calls = pending.document_replaced();
        assert_eq!(pending.generation(), generation + 1);
        PendingCalls::settle_many(calls, "the document was replaced before the script ran");
        assert_eq!(
            results.borrow().as_slice(),
            &[Err(
                "the document was replaced before the script ran".to_owned()
            )]
        );
        assert_eq!(
            results_js.borrow().as_slice(),
            &[Err(
                "the document was replaced before the script ran".to_owned()
            )]
        );
        assert!(cookies.borrow().iter().all(Vec::is_empty));
    }

    #[test]
    fn pending_calls_navigations_that_do_not_commit_keep_everything() {
        // `nativeWillNavigate` is an event only: `pushState`, a hash change,
        // a 204, a download and a cancelled redirect all reach Kotlin
        // without `onPageStarted`, and the live document still owes the
        // calls. Only `document_replaced` settles — nothing else may.
        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let token = begin_async(&mut pending, 1, settle);
        pending.settle(1, true, ASYNC_CALL_STARTED);
        // The call is live and its envelope still settles it.
        pending.settle_async(&envelope(1, 0, token, true, "ok"));
        assert_eq!(results.borrow().as_slice(), &[Ok("ok".to_owned())]);
    }

    #[test]
    fn pending_calls_a_stale_generation_result_is_rejected() {
        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let _ = begin_async(&mut pending, 1, settle);
        pending.settle(1, true, ASYNC_CALL_STARTED);
        // A commit drains the call and bumps the generation; the old
        // document's late envelope can no longer address a call.
        let calls = pending.document_replaced();
        PendingCalls::settle_many(calls, "replaced");
        assert_eq!(results.borrow().as_slice(), &[Err("replaced".to_owned())]);
        // A stale-generation envelope for a NEW call id is rejected too.
        let (results2, settle2) = collect();
        let token2 = begin_async(&mut pending, 2, settle2);
        pending.settle_async(&envelope(2, 0, token2, true, "stale"));
        assert!(results2.borrow().is_empty());
        pending.settle_async(&envelope(2, 1, token2, true, "live"));
        assert_eq!(results2.borrow().as_slice(), &[Ok("live".to_owned())]);
    }

    #[test]
    fn pending_calls_a_forged_async_result_never_settles_a_call() {
        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let token = begin_async(&mut pending, 1, settle);
        pending.settle(1, true, ASYNC_CALL_STARTED);
        // An envelope without the call's token — the shape a page forging
        // `{id, ok, value}` produces — is dropped, and the call waits on.
        pending.settle_async(&envelope(1, 0, token.wrapping_add(1), true, "forged"));
        assert!(results.borrow().is_empty());
        pending.settle_async(&envelope(1, 0, token, true, "real"));
        assert_eq!(results.borrow().as_slice(), &[Ok("real".to_owned())]);
    }

    #[test]
    fn pending_calls_a_release_settles_then_a_dead_view_answers_at_once() {
        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let _ = begin_async(&mut pending, 1, settle);
        pending.settle(1, true, ASYNC_CALL_STARTED);
        // release(): the still-live async call fails at once.
        let calls = pending.release();
        PendingCalls::settle_many(calls, "the web view was closed");
        assert_eq!(
            results.borrow().as_slice(),
            &[Err("the web view was closed".to_owned())]
        );
        // A reply for an already-settled id is dropped, not settled again.
        pending.settle_async(&envelope(1, 0, 0, true, "late"));
        assert_eq!(results.borrow().len(), 1);
        // A call begun on the dead view settles immediately, without a
        // Kotlin call ever being issued for it.
        let (results2, settle2) = collect();
        assert!(!pending.begin(
            2,
            PendingCall::Async { settle: Box::new(settle2), token: 0 }
        ));
        assert_eq!(
            results2.borrow().as_slice(),
            &[Err("the web view was closed".to_owned())]
        );
    }

    #[test]
    fn pending_calls_a_malformed_async_result_settles_nothing() {
        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let _ = begin_async(&mut pending, 1, settle);
        pending.settle_async("not json");
        assert!(results.borrow().is_empty());
    }

    #[test]
    fn watch_with_initial_pushes_the_current_value_once() {
        // A constant `false` — `watch` alone would never fire, and Kotlin
        // would keep its `true` default.
        let applied = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = std::rc::Rc::clone(&applied);
        let _guard =
            watch_bool_with_initial(&Computed::new(false), move |enabled| {
                sink.borrow_mut().push(enabled);
            });
        assert_eq!(applied.borrow().as_slice(), &[false]);
    }

    // ---- the WEBVIEW_METHODS ↔ HydrolysisWebView.kt agreement ----

    /// The wrapper methods android.webkit.WebView already declares — JNI
    /// resolves them through inheritance and the framework keeps them, so
    /// they carry no `@CalledFromNative`.
    const PLATFORM_INHERITED: &[&str] = &[
        "goBack",
        "goForward",
        "loadUrl",
        "stopLoading",
        "reload",
        "canGoBack",
        "canGoForward",
    ];

    /// `HydrolysisWebView.kt`, read at compile time — a moved file is a
    /// compile error, not a skipped test.
    const WRAPPER_KT: &str = include_str!(
        "../../../../android/webview/src/main/java/dev/waterui/hydrolysis/webview/HydrolysisWebView.kt"
    );

    /// The Kotlin parameter/return types the webview JNI contract spells.
    fn kotlin_type_to_jni(kotlin: &str, member: &str) -> &'static str {
        match kotlin.trim() {
            "Boolean" => "Z",
            "Long" => "J",
            "Int" => "I",
            "String" => "Ljava/lang/String;",
            "ByteArray" => "[B",
            "Array<String>" => "[Ljava/lang/String;",
            "HydrolysisSession" => "Ldev/waterui/hydrolysis/HydrolysisSession;",
            "HydrolysisWebView" => "Ldev/waterui/hydrolysis/webview/HydrolysisWebView;",
            "Unit" => "V",
            other => panic!("@CalledFromNative member {member} uses unmapped Kotlin type {other}"),
        }
    }

    /// Every `@CalledFromNative` member in `source` as `name → signature`.
    /// Handles `fun name(...)`/`: ReturnType`, `@JvmStatic fun`s and the
    /// `class X @CalledFromNative constructor(...)` form.
    fn annotated_members(source: &str) -> std::collections::BTreeMap<String, String> {
        let mut members = std::collections::BTreeMap::new();
        let mut rest = source;
        while let Some(at) = rest.find("@CalledFromNative") {
            rest = &rest[at + "@CalledFromNative".len()..];
            // Skip annotations until `fun` or `constructor`.
            loop {
                rest = rest.trim_start();
                if rest.starts_with('@') {
                    let end = rest.find(['\n', ' ']).unwrap_or(rest.len());
                    rest = &rest[end..];
                } else {
                    break;
                }
            }
            if let Some(ctor_rest) = rest.strip_prefix("constructor") {
                // `class X @CalledFromNative constructor(` — the annotation
                // sits between the class name and its primary constructor.
                let (params, _) = take_parens(ctor_rest);
                let mut sig = String::from("(");
                push_params(&mut sig, &params, "AssetResponse");
                sig.push_str(")V");
                members.insert("<init>".to_owned(), sig);
                continue;
            }
            if let Some(class_rest) = rest.strip_prefix("class") {
                // `class Name ... constructor(` — take ctor params.
                let Some(ctor_start) = class_rest
                    .find("constructor(")
                    .map(|index| &class_rest[index..])
                else {
                    panic!("@CalledFromNative class with no constructor: {class_rest:.80}")
                };
                let (params, _) = take_parens(&ctor_start["constructor".len()..]);
                let mut sig = String::from("(");
                push_params(&mut sig, &params, "AssetResponse");
                sig.push_str(")V");
                members.insert("<init>".to_owned(), sig);
                continue;
            }
            let Some(fn_rest) = rest.strip_prefix("fun") else {
                panic!("@CalledFromNative followed by neither fun nor constructor: {rest:.80}")
            };
            let fn_rest = fn_rest.trim_start();
            // `create(...)` companion function: name then params.
            let name_end = fn_rest.find('(').expect("@CalledFromNative fun has params");
            let name = fn_rest[..name_end].trim().to_owned();
            let (params, after) = take_parens(&fn_rest[name_end..]);
            let mut sig = String::from("(");
            push_params(&mut sig, &params, &name);
            sig.push(')');
            let after = after.trim_start();
            let return_sig = after.strip_prefix(':').map_or("Unit", |return_ty| {
                return_ty.split(['\n', '{', '=']).next().unwrap().trim()
            });
            sig.push_str(kotlin_type_to_jni(return_sig, &name));
            members.insert(name, sig);
        }
        members
    }

    /// `(...)`, balance-aware; returns the inside and what follows `)`.
    fn take_parens(mut s: &str) -> (String, &str) {
        assert!(s.trim_start().starts_with('('), "expected '(' at {s:.40}");
        s = s.trim_start();
        let mut depth = 0;
        for (i, c) in s.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return (s[1..i].to_owned(), &s[i + 1..]);
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced parens in {s:.80}");
    }

    fn push_params(sig: &mut String, params: &str, member: &str) {
        for param in params.split(',') {
            let param = param.trim();
            if param.is_empty() {
                continue;
            }
            let param = param
                .strip_prefix("private ")
                .unwrap_or(param)
                .trim_start_matches("val ")
                .trim_start_matches("var ")
                .trim();
            let (_, ty) = param.split_once(':').unwrap_or_else(|| {
                panic!("@CalledFromNative parameter {param} on {member} has no type")
            });
            sig.push_str(kotlin_type_to_jni(ty, member));
        }
    }

    #[test]
    fn the_webview_table_and_the_annotated_kotlin_members_agree() {
        for (id, method) in WebViewMethodId::ALL.iter().zip(WEBVIEW_METHODS.iter()) {
            // camelCase the PascalCase id: SetUserAgent → setUserAgent.
            let debug = format!("{id:?}");
            let mut chars = debug.chars();
            let expected = chars
                .next()
                .unwrap()
                .to_lowercase()
                .next()
                .unwrap()
                .to_string()
                + chars.as_str();
            assert_eq!(
                method.name, expected,
                "WEBVIEW_METHODS must stay in WebViewMethodId order"
            );
        }

        // Members the platform `WebView` already declares — HydrolysisWebView
        // inherits them, JNI resolves them through the class, and the
        // framework keeps them. Only Kotlin-declared members need the
        // `@CalledFromNative` keep.
        let annotated = annotated_members(WRAPPER_KT);
        let declared: std::collections::BTreeMap<String, String> = WEBVIEW_METHODS
            .iter()
            .filter(|method| !PLATFORM_INHERITED.contains(&method.name))
            .chain([WEBVIEW_CREATE, WEBVIEW_ASSET_RESPONSE_INIT].iter())
            .map(|method| (method.name.to_owned(), method.signature.to_owned()))
            .collect();
        assert_eq!(
            declared, annotated,
            "every member Rust calls on HydrolysisWebView/AssetResponse needs a matching @CalledFromNative member in HydrolysisWebView.kt, and vice versa"
        );
    }
}
