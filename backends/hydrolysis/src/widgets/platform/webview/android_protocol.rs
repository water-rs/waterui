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
use waterui_webview::{Cookie, OriginPolicy, Url, WebViewError, WebViewEvent};

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
pub fn compose_async_call(id: u64, generation: u64, token: &str, body: &str) -> String {
    format!("({ASYNC_CALL})({id}, {generation}, \"{token}\", async function () {{\n{body}\n}});")
}

/// Install `apply` as `signal`'s watcher after running it once on the
/// current value. `Computed::watch` fires only on change — and a
/// constant's watch never fires — so the initial state would otherwise
/// never reach the [`NavigationTracker`]. `set_redirects_enabled` is the
/// Android bridge's one signal-driven setting; every other field crosses as
/// an immediate setter call.
pub fn watch_bool_with_initial(
    signal: &Computed<bool>,
    apply: impl Fn(bool) + 'static,
) -> nami::watcher::BoxWatcherGuard {
    apply(signal.snapshot());
    signal.watch(move |ctx| apply(ctx.into_value()))
}

/// A per-call token `compose_async_call` embeds and `settle_async`
/// requires back. 128 bits from `getrandom` as a hex *string*: a page
/// forging a result envelope cannot guess it, and crossing as a string
/// means the JS number path's 2^53 precision cap never corrupts it.
#[must_use]
pub fn fresh_async_token() -> String {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("getrandom is available on every WaterUI target");
    let mut token = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write;
        write!(token, "{byte:02x}").expect("writing into a String cannot fail");
    }
    token
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
    /// The per-call token `compose_async_call` embedded in the script —
    /// a string so a JS round trip preserves every bit.
    pub token: String,
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
        token: String,
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
    /// every main-frame cross-document commit. Its honest scope: it is
    /// documentation made executable, not added rejection coverage — the
    /// drain plus unique call ids already reject everything a stale
    /// generation could name, since a drained call is gone from the map
    /// and a live call's id is never reused. What it buys is that an
    /// envelope carries the document the call ran in: a call racing a
    /// navigation reports "document replaced" even when the script
    /// actually ran, the same race `WKWebView` admits — the generation makes
    /// the losing document's late answer name why it was dropped.
    generation: u64,
    /// Set by [`Self::release`]: the view is gone (a `release`, or the
    /// render process died while the instance stays registered), so no
    /// Kotlin reply can ever arrive and `begin` settles new calls at once.
    dead: bool,
}

impl PendingCalls {
    /// Whether `release` ran — the handle dropped, or `nativeReleased`
    /// reported a dead render process — and every further call must settle
    /// without a dispatch. The Android
    /// handle gates every Kotlin-bound command on it (`begin` itself
    /// covers the call registry).
    #[cfg(hydrolysis_android_system_webview)]
    #[must_use]
    pub const fn dead(&self) -> bool {
        self.dead
    }

    /// Register `call` under `id` before the Kotlin call that answers it is
    /// issued — the reply may land on the same call. On a dead view the
    /// call settles at once and `false` is returned, so the caller skips
    /// issuing the Kotlin call it could never answer.
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
    /// the calls the old document can no longer answer. The contract a
    /// caller sees: a call that raced the commit reports "document
    /// replaced" whether or not its script ran — the `WebView` gives no way
    /// to learn which, so the failure is the honest answer (`WKWebView`'s
    /// `evaluateJavaScript` admits the same race).
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

/// Where the main frame's navigation stands.
#[derive(Debug)]
enum NavigationPhase {
    /// Nothing is loading: no navigation yet, or the last one finished.
    Idle,
    /// A navigation the application has been told about is in flight.
    /// `committed` turns true once its document commits — `onPageStarted`,
    /// or `doUpdateVisitedHistory` for a same-document history entry — and
    /// only a committed navigation can finish.
    Open { url: Url, committed: bool },
    /// The last navigation ended before it finished: a blocked redirect, a
    /// main-frame error, an SSL refusal, or `stop`. Its last event was
    /// already reported, so what the engine still says about *it* — the
    /// cancellation's error, its progress — is dropped until the next
    /// navigation opens. `error_page` is set only when it ended in an
    /// error: the engine then still commits its error page at `url`, one
    /// commit the tracker drops as still that navigation's. After a
    /// `stop` or a blocked redirect nothing more of it commits, so even a
    /// same-URL `onPageStarted` is a new navigation.
    Ended { url: Url, error_page: bool },
}

/// What a main-frame `shouldOverrideUrlLoading` resolves to: the events it
/// reports, and whether the engine must cancel the request.
#[derive(Debug, PartialEq)]
pub struct RequestDecision {
    /// The events to emit, in order.
    pub events: Vec<WebViewEvent>,
    /// `true` to cancel the request — `shouldOverrideUrlLoading`'s answer.
    pub block: bool,
}

/// The navigation event contract on top of the raw `WebViewClient` and
/// `WebChromeClient` callbacks.
///
/// The contract is the one the `WKWebView` bridge reports: a navigation
/// opens with `WillNavigate` then `Loading(0)`; a server redirect reports
/// `Redirect { from, to }` and, when allowed, `WillNavigate(to)`, while a
/// blocked one cancels the load and is the navigation's last event; the
/// commit reports the progress so far; the finish reports `Loading(1.0)`
/// then `Loaded`, so `Loaded` is always last; a main-frame error is the
/// last event of the navigation it ends.
///
/// Android's callbacks do not line up with that on their own.
/// `onProgressChanged` covers the whole page — iframes, cancelled loads,
/// loads that start after the finish — so it is forwarded only while a
/// navigation is open, and never its 100% report, which is `Loaded`'s job.
///
/// The finish is the first 100% report after the commit, not
/// `onPageFinished`. The system `WebView` also fires `onPageFinished` for a
/// same-document history update, including one the page makes while its
/// own load is still running: `www.google.com` replaces its history entry
/// at 70% and gets an `onPageFinished` there, ahead of the real one. The
/// 100% report comes only when the page's load stops, and it follows the
/// commit of every navigation that finishes — cross-document, same-document
/// and history steps alike.
/// A load the application starts (`loadUrl`, `goBack`, `goForward`,
/// `reload`) never reaches `shouldOverrideUrlLoading`, so the Kotlin
/// wrapper reports its target through [`Self::open`].
///
/// Every URL the tracker compares is the engine's spelling, never the
/// application's: the system `WebView` fixes up and canonicalizes what
/// `loadUrl` is given, and every callback carries the result, so a
/// `go_to("https://waterui.dev#b")` commits as `https://waterui.dev/#b`.
///
/// Every input returns the events it produced instead of emitting them, so
/// the caller emits after its borrow of the tracker ends — an application
/// handler that drives the view again cannot re-enter it.
#[derive(Debug)]
pub struct NavigationTracker {
    phase: NavigationPhase,
    /// Whether a server redirect may proceed — the `redirects_enabled`
    /// signal's current value.
    redirects_enabled: bool,
    /// The main-frame URL as last known: the open navigation's target, the
    /// committed document, or a same-document history entry. It is the
    /// cookie URL, and the `from` of a redirect whose navigation never
    /// reached `shouldOverrideUrlLoading` (a form `POST`).
    current_url: Option<Url>,
}

impl Default for NavigationTracker {
    fn default() -> Self {
        Self {
            phase: NavigationPhase::Idle,
            redirects_enabled: true,
            current_url: None,
        }
    }
}

impl NavigationTracker {
    /// The `redirects_enabled` signal changed.
    pub const fn set_redirects_enabled(&mut self, enabled: bool) {
        self.redirects_enabled = enabled;
    }

    /// The main-frame URL as last known; see the field.
    pub const fn current_url(&self) -> Option<&Url> {
        self.current_url.as_ref()
    }

    /// A navigation begins — one the application started, or a main-frame
    /// request `shouldOverrideUrlLoading` admitted. Supersedes whatever was
    /// open. `url` is the engine's spelling of the target: the commit is
    /// matched against it, and a redirect reports it as `from`.
    pub fn open(&mut self, url: Url) -> Vec<WebViewEvent> {
        self.phase = NavigationPhase::Open {
            url: url.clone(),
            committed: false,
        };
        self.current_url = Some(url.clone());
        vec![
            WebViewEvent::WillNavigate { url },
            WebViewEvent::Loading { progress: 0.0 },
        ]
    }

    /// A main-frame `shouldOverrideUrlLoading` — a subframe request is the
    /// page's own business, and the native passes it before its URL is even
    /// read. A request either opens a navigation or, as a server redirect,
    /// continues the open one.
    ///
    /// # Panics
    ///
    /// On a main-frame redirect before any main-frame URL was ever known —
    /// a redirect needs a navigation, and every navigation is either opened
    /// here or issued from a committed document.
    pub fn request(&mut self, url: Url, is_redirect: bool) -> RequestDecision {
        if !is_redirect {
            return RequestDecision {
                events: self.open(url),
                block: false,
            };
        }
        let from = match &self.phase {
            NavigationPhase::Open { url, .. } => url.clone(),
            NavigationPhase::Idle | NavigationPhase::Ended { .. } => self
                .current_url
                .clone()
                .expect("android webview: a main-frame redirect arrived before any main-frame URL"),
        };
        let mut events = vec![WebViewEvent::Redirect {
            from,
            to: url.clone(),
        }];
        if !self.redirects_enabled {
            self.phase = NavigationPhase::Ended {
                url,
                error_page: false,
            };
            return RequestDecision {
                events,
                block: true,
            };
        }
        self.phase = NavigationPhase::Open {
            url: url.clone(),
            committed: false,
        };
        self.current_url = Some(url.clone());
        events.push(WebViewEvent::WillNavigate { url });
        RequestDecision {
            events,
            block: false,
        }
    }

    /// `onPageStarted`: a main-frame document committed at `url`, with the
    /// page's progress at that moment. A commit nothing announced — a form
    /// `POST`, a page's own `location.reload()` — opens its navigation here;
    /// the error page a navigation that ended in an error commits at its own
    /// URL is part of that navigation and reports nothing.
    pub fn page_started(&mut self, url: Url, progress: u8) -> Vec<WebViewEvent> {
        let mut events = match &mut self.phase {
            NavigationPhase::Open {
                url: open,
                committed,
            } if !*committed || *open == url => {
                *committed = true;
                open.clone_from(&url);
                Vec::new()
            }
            NavigationPhase::Ended {
                url: ended,
                error_page,
            } if *ended == url && *error_page => {
                // The error page is the ended navigation's own last
                // commit — dropped once, so a later commit at the same URL
                // still opens a new navigation.
                *error_page = false;
                return Vec::new();
            }
            NavigationPhase::Idle
            | NavigationPhase::Open { .. }
            | NavigationPhase::Ended { .. } => {
                let events = self.open(url.clone());
                self.phase = NavigationPhase::Open {
                    url: url.clone(),
                    committed: true,
                };
                events
            }
        };
        self.current_url = Some(url);
        if progress > 0 && progress < 100 {
            events.push(WebViewEvent::Loading {
                progress: f32::from(progress) / 100.0,
            });
        }
        events
    }

    /// `doUpdateVisitedHistory`: the main frame's history entry is now
    /// `url`. It commits the open navigation when that is where it was
    /// going — a same-document history step has no `onPageStarted` — and
    /// otherwise only moves the current URL (`pushState`, a fragment).
    pub fn history_updated(&mut self, url: Url) {
        if let NavigationPhase::Open {
            url: open,
            committed,
        } = &mut self.phase
            && *open == url
        {
            *committed = true;
        }
        self.current_url = Some(url);
    }

    /// `onProgressChanged`, as a percentage. Forwarded only while a
    /// navigation is open. The 100% report finishes the open navigation
    /// once it committed; before the commit it is the previous document's
    /// load stopping, and with nothing open it is a load that is not a
    /// navigation — an iframe, a late report for one already over.
    pub fn progress(&mut self, progress: u8) -> Vec<WebViewEvent> {
        match self.phase {
            NavigationPhase::Open { .. } if progress < 100 => vec![WebViewEvent::Loading {
                progress: f32::from(progress) / 100.0,
            }],
            NavigationPhase::Open {
                committed: true, ..
            } => {
                self.phase = NavigationPhase::Idle;
                vec![
                    WebViewEvent::Loading { progress: 1.0 },
                    WebViewEvent::Loaded,
                ]
            }
            NavigationPhase::Open { .. }
            | NavigationPhase::Idle
            | NavigationPhase::Ended { .. } => Vec::new(),
        }
    }

    /// A main-frame `onReceivedError` for the request at `url` — the native
    /// drops every subframe error. It ends the open navigation it names, and
    /// with nothing open it still reports: an unannounced `POST` fails the
    /// same way. One that names a different request — the cancellation of a
    /// navigation that already ended, or a late error for a navigation a
    /// newer `open` superseded — is dropped: `WebViewClient` callbacks are
    /// posted to the looper, so a `stop` or `go_to` delivers the old
    /// request's error after the next navigation is already open.
    pub fn received_error(&mut self, url: Url, error: WebViewError) -> Vec<WebViewEvent> {
        match &self.phase {
            // The cancellation of the request that already ended.
            NavigationPhase::Ended { url: ended, .. } if *ended == url => Vec::new(),
            // A late error for a navigation a newer `open` superseded —
            // not the open navigation's failure.
            NavigationPhase::Open { url: open, .. } if *open != url => Vec::new(),
            _ => {
                self.phase = NavigationPhase::Ended {
                    url,
                    error_page: true,
                };
                vec![WebViewEvent::Error(error)]
            }
        }
    }

    /// `onReceivedSslError`, which the bridge always cancels. The error is
    /// always reported — `SslError` does not say which frame it is for —
    /// and when it names the open navigation's URL it ends that navigation.
    pub fn ssl_error(&mut self, url: Option<Url>, message: Str) -> Vec<WebViewEvent> {
        let error = match url {
            Some(url) => {
                if let NavigationPhase::Open { url: open, .. } = &self.phase
                    && *open == url
                {
                    self.phase = NavigationPhase::Ended {
                        url: url.clone(),
                        error_page: true,
                    };
                }
                WebViewError::Ssl { url, message }
            }
            None => WebViewError::Network(message),
        };
        vec![WebViewEvent::Error(error)]
    }

    /// `stop`: the open navigation ends where it stands, with no further
    /// event — the `WKWebView` bridge drops the cancellation the same way.
    pub fn stopped(&mut self) {
        if let NavigationPhase::Open { url, .. } = &self.phase {
            self.phase = NavigationPhase::Ended {
                url: url.clone(),
                error_page: false,
            };
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
    NavigateTo,
    NavigateBack,
    NavigateForward,
    NavigateReload,
    StopLoading,
    SetUserAgent,
    CanGoBack,
    CanGoForward,
    SetDocumentStartScripts,
    SetBridgeOrigins,
    SetCookie,
    GetCookies,
    Evaluate,
    PostBridgeReply,
    Release,
}

impl WebViewMethodId {
    /// Every id, in table order.
    pub const ALL: [Self; 15] = [
        Self::NavigateTo,
        Self::NavigateBack,
        Self::NavigateForward,
        Self::NavigateReload,
        Self::StopLoading,
        Self::SetUserAgent,
        Self::CanGoBack,
        Self::CanGoForward,
        Self::SetDocumentStartScripts,
        Self::SetBridgeOrigins,
        Self::SetCookie,
        Self::GetCookies,
        Self::Evaluate,
        Self::PostBridgeReply,
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
        name: "navigateTo",
        signature: "(Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "navigateBack",
        signature: "()V",
    },
    WebViewMethod {
        name: "navigateForward",
        signature: "()V",
    },
    WebViewMethod {
        name: "navigateReload",
        signature: "()V",
    },
    WebViewMethod {
        name: "stopLoading",
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
        name: "postBridgeReply",
        signature: "(Landroidx/webkit/JavaScriptReplyProxy;Ljava/lang/String;)V",
    },
    WebViewMethod {
        name: "release",
        signature: "()J",
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
        let call = compose_async_call(7, 3, "ab01", "return 1;");
        assert!(call.contains(ASYNC_CALL_SENTINEL));
        assert!(call.contains(ASYNC_RESULT_OBJECT));
        assert!(call.contains("async function () {\nreturn 1;\n}"));
        assert!(call.contains("(7, 3, \"ab01\","));

        let held = compose_document_end("document.title = 'x';");
        assert!(held.contains("DOMContentLoaded"));
        assert!(held.contains("document.title = 'x';"));
    }

    #[test]
    fn each_js_file_names_the_constant_it_relies_on() {
        assert!(TRANSPORT_SCRIPT.contains(BRIDGE_OBJECT));
        assert!(ASYNC_CALL.contains(ASYNC_RESULT_OBJECT));
        assert!(ASYNC_CALL.contains(ASYNC_CALL_SENTINEL));
        assert!(DOCUMENT_END.contains("function (run)"));
    }

    /// The composed call must survive the trip through a real JS engine:
    /// `JSON.stringify` in `async_call.js` posts back every field the
    /// literal carried, so a token or id that loses precision inside a JS
    /// number — anything past 2^53 — shows up here as a never-settling
    /// call. The CI job that runs ignored tests installs `bun`.
    #[test]
    #[ignore = "needs bun"]
    fn the_composed_async_call_survives_a_real_js_round_trip() {
        let engine = std::process::Command::new("bun")
            .arg("--version")
            .output()
            .map(|_| "bun")
            .or_else(|_| {
                std::process::Command::new("node")
                    .arg("--version")
                    .output()
                    .map(|_| "node")
            })
            .expect("the JS round-trip test needs bun or node on PATH");

        let mut pending = PendingCalls::default();
        let (results, settle) = collect();
        let token = begin_async(&mut pending, 7, settle);
        let call = compose_async_call(7, pending.generation(), &token, r#"return "resolved";"#);
        // `call` evaluates to the started sentinel; the envelope arrives on
        // `__wateruiAsyncResult.postMessage` a task later. The script embeds
        // as a JSON string, so a quoting slip fails here, not in the page.
        let driver = format!(
            "var __posted = null;\n\
             var __wateruiAsyncResult = {{ postMessage: function (m) {{ __posted = m; }} }};\n\
             if (eval({script}) !== {sentinel}) {{ throw new Error(\"the call did not start\"); }}\n\
             setTimeout(function () {{ console.log(__posted); }}, 0);\n",
            script = serde_json::to_string(&call).expect("a script serializes to JSON"),
            sentinel =
                serde_json::to_string(ASYNC_CALL_SENTINEL).expect("a string serializes to JSON"),
        );
        let output = std::process::Command::new(engine)
            .arg("-e")
            .arg(&driver)
            .output()
            .expect("the JS engine runs the driver");
        assert!(
            output.status.success(),
            "the driver failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope = String::from_utf8(output.stdout).expect("the envelope is UTF-8");
        pending.settle_async(envelope.trim());
        assert_eq!(results.borrow().as_slice(), &[Ok("resolved".to_owned())]);
    }

    #[test]
    fn async_result_parsing() {
        let settled = parse_async_result(
            r#"{"id":3,"generation":2,"token":"deadbeef","ok":true,"value":"\"done\""}"#,
        )
        .expect("a well-formed envelope parses");
        assert_eq!(settled.id, 3);
        assert_eq!(settled.generation, 2);
        assert_eq!(settled.token, "deadbeef");
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
    fn begin_async(
        pending: &mut PendingCalls,
        id: u64,
        settle: impl Fn(Result<Str, Str>) + 'static,
    ) -> String {
        let token = fresh_async_token();
        assert!(pending.begin(
            id,
            PendingCall::Async {
                settle: Box::new(settle),
                token: token.clone()
            }
        ));
        token
    }

    /// The JSON envelope the composed script posts for a call.
    fn envelope(id: u64, generation: u64, token: &str, ok: bool, value: &str) -> String {
        format!(
            r#"{{"id":{id},"generation":{generation},"token":"{token}","ok":{ok},"value":"{value}"}}"#
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
        pending.settle_async(&envelope(1, 0, &token, true, "\\\"v\\\""));
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
        pending.settle_async(&envelope(1, 0, &token, true, "ok"));
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
        pending.settle_async(&envelope(2, 0, &token2, true, "stale"));
        assert!(results2.borrow().is_empty());
        pending.settle_async(&envelope(2, 1, &token2, true, "live"));
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
        pending.settle_async(&envelope(1, 0, "forged-token", true, "forged"));
        assert!(results.borrow().is_empty());
        pending.settle_async(&envelope(1, 0, &token, true, "real"));
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
        pending.settle_async(&envelope(1, 0, "late-token", true, "late"));
        assert_eq!(results.borrow().len(), 1);
        // A call begun on the dead view settles immediately, without a
        // Kotlin call ever being issued for it.
        let (results2, settle2) = collect();
        assert!(!pending.begin(
            2,
            PendingCall::Async {
                settle: Box::new(settle2),
                token: String::new()
            }
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
        let _guard = watch_bool_with_initial(&Computed::new(false), move |enabled| {
            sink.borrow_mut().push(enabled);
        });
        assert_eq!(applied.borrow().as_slice(), &[false]);
    }

    // ---- NavigationTracker: the callback orders the device delivers ----

    fn url(text: &str) -> Url {
        text.parse().expect("a valid test URL")
    }

    fn will_navigate(text: &str) -> WebViewEvent {
        WebViewEvent::WillNavigate { url: url(text) }
    }

    fn loading(percent: u8) -> WebViewEvent {
        WebViewEvent::Loading {
            progress: f32::from(percent) / 100.0,
        }
    }

    fn redirect(from: &str, to: &str) -> WebViewEvent {
        WebViewEvent::Redirect {
            from: url(from),
            to: url(to),
        }
    }

    const fn allowed(events: Vec<WebViewEvent>) -> RequestDecision {
        RequestDecision {
            events,
            block: false,
        }
    }

    /// Feeds one raw callback and appends what it reports, so a test reads
    /// as the callback order with the whole event sequence asserted once.
    struct Drive {
        tracker: NavigationTracker,
        events: Vec<WebViewEvent>,
    }

    impl Drive {
        fn new() -> Self {
            Self {
                tracker: NavigationTracker::default(),
                events: Vec::new(),
            }
        }

        fn open(&mut self, target: &str) -> &mut Self {
            let events = self.tracker.open(url(target));
            self.events.extend(events);
            self
        }

        fn request(&mut self, target: &str, is_redirect: bool) -> bool {
            let decision = self.tracker.request(url(target), is_redirect);
            self.events.extend(decision.events);
            decision.block
        }

        fn started(&mut self, target: &str, percent: u8) -> &mut Self {
            let events = self.tracker.page_started(url(target), percent);
            self.events.extend(events);
            self
        }

        fn progress(&mut self, percent: u8) -> &mut Self {
            let events = self.tracker.progress(percent);
            self.events.extend(events);
            self
        }

        fn history(&mut self, target: &str) -> &mut Self {
            self.tracker.history_updated(url(target));
            self
        }

        fn error(&mut self, target: &str, message: &str) -> &mut Self {
            let events = self.tracker.received_error(
                url(target),
                WebViewError::Network(Str::from(message.to_owned())),
            );
            self.events.extend(events);
            self
        }

        fn take(&mut self) -> Vec<WebViewEvent> {
            std::mem::take(&mut self.events)
        }
    }

    #[test]
    fn navigation_a_plain_load_ends_with_loaded() {
        // The order a Pixel 9 Pro (WebView 153) reports for `go_to`.
        let mut drive = Drive::new();
        drive
            .open("https://waterui.dev/")
            .progress(10)
            .started("https://waterui.dev/", 20)
            .history("https://waterui.dev/")
            .progress(20)
            .progress(80)
            .progress(100)
            .progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://waterui.dev/"),
                loading(0),
                loading(10),
                loading(20),
                loading(20),
                loading(80),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );
        assert_eq!(
            drive.tracker.current_url(),
            Some(&url("https://waterui.dev/"))
        );
    }

    #[test]
    fn navigation_an_allowed_redirect_reports_redirect_then_will_navigate() {
        // The order the device reports for `google.com` with redirects
        // allowed. The page replaces its history entry at 70% — the system
        // WebView fires `onPageFinished` there, ahead of the real one — and
        // again after the finish, with its own progress cycle.
        let mut drive = Drive::new();
        drive.open("https://google.com/");
        assert!(!drive.request("https://www.google.com/", true));
        drive
            .started("https://www.google.com/", 30)
            .history("https://www.google.com/")
            .progress(24)
            .progress(70)
            .history("https://www.google.com/");
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://google.com/"),
                loading(0),
                redirect("https://google.com/", "https://www.google.com/"),
                will_navigate("https://www.google.com/"),
                loading(30),
                loading(24),
                loading(70),
            ]
        );
        drive.progress(100).progress(100);
        assert_eq!(drive.take(), [loading(100), WebViewEvent::Loaded]);
        drive
            .progress(10)
            .history("https://www.google.com/?zx=1")
            .progress(100)
            .progress(100);
        assert_eq!(drive.take(), [] as [WebViewEvent; 0]);
        assert_eq!(
            drive.tracker.current_url(),
            Some(&url("https://www.google.com/?zx=1"))
        );
    }

    #[test]
    fn navigation_a_blocked_redirect_is_the_last_event() {
        // The order the device reports for `go_to("https://google.com")`
        // with redirects blocked: the cancelled load never commits, and the
        // 100% that follows is the previous document's. The wrapper opens
        // the engine's spelling of the target, so the redirect reports it.
        let mut drive = Drive::new();
        drive.tracker.set_redirects_enabled(false);
        drive.open("https://google.com/").progress(10);
        assert!(drive.request("https://www.google.com/", true));
        drive.progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://google.com/"),
                loading(0),
                loading(10),
                redirect("https://google.com/", "https://www.google.com/"),
            ]
        );
        // The blocked target never became the document.
        assert_eq!(
            drive.tracker.current_url(),
            Some(&url("https://google.com/"))
        );
    }

    #[test]
    fn navigation_a_same_document_go_to_commits_through_the_history_update() {
        // `go_to("https://waterui.dev#b")` from `https://waterui.dev/`, in
        // the device's order: no `onPageStarted`, `doUpdateVisitedHistory`
        // commits it — `onPageFinished` feeds the tracker nothing — and the
        // 100% report finishes it. The wrapper opens the engine's spelling,
        // the one `doUpdateVisitedHistory` repeats.
        let mut drive = Drive::new();
        drive
            .open("https://waterui.dev/")
            .started("https://waterui.dev/", 0)
            .progress(100);
        drive.take();
        drive
            .open("https://waterui.dev/#b")
            .progress(10)
            .history("https://waterui.dev/#b")
            .progress(100)
            .progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://waterui.dev/#b"),
                loading(0),
                loading(10),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );
        assert_eq!(
            drive.tracker.current_url(),
            Some(&url("https://waterui.dev/#b"))
        );
    }

    #[test]
    fn navigation_an_iframe_progress_cycle_after_the_finish_reports_nothing() {
        let mut drive = Drive::new();
        drive
            .open("https://waterui.dev/")
            .started("https://waterui.dev/", 50)
            .progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://waterui.dev/"),
                loading(0),
                loading(50),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );
        // An iframe loads after the page finished. `onProgressChanged`
        // covers the whole page, so its cycle arrives here; none of it is a
        // main-frame navigation.
        drive.progress(10).progress(60).progress(100);
        assert_eq!(drive.take(), [] as [WebViewEvent; 0]);
    }

    #[test]
    fn navigation_back_and_forward_open_their_history_targets() {
        let mut drive = Drive::new();
        drive
            .open("https://a.dev/")
            .started("https://a.dev/", 0)
            .progress(100);
        drive
            .open("https://b.dev/")
            .started("https://b.dev/", 0)
            .progress(100);
        drive.take();

        // Back across documents, in the device's order: the Kotlin wrapper
        // reports the history target, the restored document commits and
        // finishes.
        drive
            .open("https://a.dev/")
            .progress(10)
            .started("https://a.dev/", 20)
            .history("https://a.dev/")
            .progress(28)
            .progress(100)
            .progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://a.dev/"),
                loading(0),
                loading(10),
                loading(20),
                loading(28),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );

        // Forward to a same-document entry: no `onPageStarted`, the history
        // update commits it and the 100% report follows.
        drive
            .open("https://a.dev/#section")
            .progress(10)
            .history("https://a.dev/#section")
            .progress(100)
            .progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://a.dev/#section"),
                loading(0),
                loading(10),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );

        // A `pushState` with nothing open moves only the current URL.
        drive
            .progress(10)
            .history("https://a.dev/pushed")
            .progress(100);
        assert_eq!(drive.take(), [] as [WebViewEvent; 0]);
        assert_eq!(
            drive.tracker.current_url(),
            Some(&url("https://a.dev/pushed"))
        );
    }

    #[test]
    fn navigation_a_main_frame_error_ends_the_navigation() {
        let mut drive = Drive::new();
        drive
            .open("https://nowhere.invalid/")
            .progress(10)
            .error("https://nowhere.invalid/", "net::ERR_NAME_NOT_RESOLVED")
            // The error page commits at the failed URL and finishes.
            .started("https://nowhere.invalid/", 10)
            .progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://nowhere.invalid/"),
                loading(0),
                loading(10),
                WebViewEvent::Error(WebViewError::Network(Str::from_static(
                    "net::ERR_NAME_NOT_RESOLVED"
                ))),
            ]
        );

        // The next navigation opens normally.
        drive.open("https://waterui.dev/");
        assert_eq!(
            drive.take(),
            [will_navigate("https://waterui.dev/"), loading(0)]
        );
    }

    #[test]
    fn navigation_page_initiated_loads_open_through_the_request_or_the_commit() {
        let mut drive = Drive::new();
        drive
            .open("https://a.dev/")
            .started("https://a.dev/", 0)
            .progress(100);
        drive.take();

        // A link: `shouldOverrideUrlLoading` opens it.
        assert_eq!(
            drive.tracker.request(url("https://a.dev/next"), false),
            allowed(vec![will_navigate("https://a.dev/next"), loading(0)])
        );
        drive.started("https://a.dev/next", 0).progress(100);
        drive.take();

        // A form `POST` skips `shouldOverrideUrlLoading`: its redirect
        // comes from the committed document, and its commit opens it.
        assert!(!drive.request("https://a.dev/done", true));
        drive.started("https://a.dev/done", 20).progress(100);
        assert_eq!(
            drive.take(),
            [
                redirect("https://a.dev/next", "https://a.dev/done"),
                will_navigate("https://a.dev/done"),
                loading(20),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );

        // A page's own `location.reload()` reaches only the commit.
        drive.started("https://a.dev/done", 0).progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://a.dev/done"),
                loading(0),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );
    }

    #[test]
    fn navigation_stop_and_ssl_refusal_end_without_loaded() {
        let mut drive = Drive::new();
        drive.open("https://slow.dev/").progress(30);
        drive.tracker.stopped();
        drive.progress(100);
        assert_eq!(
            drive.take(),
            [will_navigate("https://slow.dev/"), loading(0), loading(30)]
        );

        drive.open("https://expired.dev/");
        let ssl = drive.tracker.ssl_error(
            Some(url("https://expired.dev/")),
            Str::from_static("expired"),
        );
        drive.events.extend(ssl);
        drive
            .error("https://expired.dev/", "net::ERR_FAILED")
            .progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://expired.dev/"),
                loading(0),
                WebViewEvent::Error(WebViewError::Ssl {
                    url: url("https://expired.dev/"),
                    message: Str::from_static("expired"),
                }),
            ]
        );

        // An SSL error with no URL still reports, as a network failure.
        assert_eq!(
            drive.tracker.ssl_error(None, Str::from_static("bad")),
            [WebViewEvent::Error(WebViewError::Network(
                Str::from_static("bad")
            ))]
        );
    }

    #[test]
    fn navigation_a_late_error_for_a_superseded_navigation_reports_nothing() {
        // `WebViewClient` callbacks are posted to the looper, so a `go_to(B)`
        // while A's error is still in flight delivers the error after B
        // already opened. The error is A's: it ends nothing and reports
        // nothing, and B's own sequence runs undisturbed.
        let mut drive = Drive::new();
        drive.open("https://a.dev/").progress(40);
        drive.take();
        drive.open("https://b.dev/");
        drive.error("https://a.dev/", "net::ERR_ABORTED");
        drive.started("https://b.dev/", 30).progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://b.dev/"),
                loading(0),
                loading(30),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );
    }

    #[test]
    fn navigation_an_ended_navigation_drops_only_its_own_errors() {
        // After `stop` the cancellation of that same request is dropped, but
        // an error for a different request still reports — a form `POST`
        // never reaches `shouldOverrideUrlLoading`, so its failure arrives
        // here unannounced.
        let mut drive = Drive::new();
        drive.open("https://a.dev/").progress(20);
        drive.tracker.stopped();
        drive.error("https://a.dev/", "net::ERR_ABORTED");
        drive.error("https://a.dev/form", "net::ERR_FAILED");
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://a.dev/"),
                loading(0),
                loading(20),
                WebViewEvent::Error(WebViewError::Network(Str::from_static("net::ERR_FAILED"))),
            ]
        );
    }

    #[test]
    fn navigation_a_same_url_commit_after_stop_opens_a_new_navigation() {
        // `stop` produces no error page: a commit at the stopped URL — the
        // page's own `location.reload()` — is a new navigation, not the
        // ended one's remains.
        let mut drive = Drive::new();
        drive.open("https://a.dev/").progress(30);
        drive.tracker.stopped();
        drive.error("https://a.dev/", "net::ERR_ABORTED");
        drive.started("https://a.dev/", 0).progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://a.dev/"),
                loading(0),
                loading(30),
                will_navigate("https://a.dev/"),
                loading(0),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );
    }

    #[test]
    fn navigation_the_error_page_is_dropped_once_then_same_url_commits_report() {
        // After a main-frame error the engine commits its error page at the
        // failed URL — still that navigation, so it reports nothing — but a
        // second commit at the same URL is a new navigation.
        let mut drive = Drive::new();
        drive
            .open("https://a.dev/")
            .error("https://a.dev/", "net::ERR_FAILED");
        drive.started("https://a.dev/", 10).progress(100);
        drive.take();
        drive.started("https://a.dev/", 0).progress(100);
        assert_eq!(
            drive.take(),
            [
                will_navigate("https://a.dev/"),
                loading(0),
                loading(100),
                WebViewEvent::Loaded,
            ]
        );
    }

    // ---- the WEBVIEW_METHODS ↔ HydrolysisWebView.kt agreement ----

    /// The wrapper methods android.webkit.WebView already declares — JNI
    /// resolves them through inheritance and the framework keeps them, so
    /// they carry no `@CalledFromNative`.
    const PLATFORM_INHERITED: &[&str] = &["stopLoading", "canGoBack", "canGoForward"];

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
            "JavaScriptReplyProxy" => "Landroidx/webkit/JavaScriptReplyProxy;",
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

    #[test]
    fn webview_render_process_gone_marks_the_registry_dead_before_reporting() {
        // `releaseAfterRenderProcessGone` must run `nativeReleased` first so
        // the `Error` `nativeRenderProcessGone` then emits is answered by a
        // registry already dead: an event handler's `go_to`/`refresh` gets
        // its dead-view value instead of dispatching `loadUrl`/`reload` onto
        // the view `tearDown` is about to destroy.
        let start = WRAPPER_KT
            .find("private fun releaseAfterRenderProcessGone")
            .expect("HydrolysisWebView.kt keeps releaseAfterRenderProcessGone");
        let end = WRAPPER_KT[start..]
            .find("private fun tearDown")
            .expect("releaseAfterRenderProcessGone precedes tearDown");
        let body = &WRAPPER_KT[start..start + end];
        let released = body
            .find("nativeReleased(handle)")
            .expect("releaseAfterRenderProcessGone calls nativeReleased");
        let reported = body
            .find("nativeRenderProcessGone(handle, message)")
            .expect("releaseAfterRenderProcessGone calls nativeRenderProcessGone");
        assert!(
            released < reported,
            "the call registry must be dead before the render-process-gone error is reported"
        );
    }
}
