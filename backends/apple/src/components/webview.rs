//! The `webview` leaf: `Native<WebView>` rendered through the kit's typed
//! `WKWebView` controller, plus the late `WebViewController` environment
//! install the fallback's `installWebViewController` performed.
//!
//! Mirrors `WuiWebView`'s `WebViewWrapper` semantics exactly: the shared
//! bridge script rides a single `__wateruiSend` script-message transport,
//! handlers dispatch by name with replacement registration, bridge calls are
//! authenticated against the `OriginPolicy` in the main frame only,
//! `waterui://localhost` is answered through the asset server, replies travel
//! back through `bridge::Reply::resolve_script` + `evaluateJavaScript`, and
//! `emitWillNavigate` dedupes against the last navigation URL unless the
//! action repeats it (reload, back/forward, form submit). The measured
//! size is `WuiWebViewComponent.sizeThatFits`: the proposal's axes where
//! given, 320×480 otherwise; the leaf stretches both ways.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::{Cell, OnceCell, RefCell};

use cocoa_ui::{MainThreadMarker, web_kit};
use waterui::Str;
use waterui::reactive::watcher::BoxWatcherGuard;
use waterui::reactive::{Computed, Signal};
use waterui_backend_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_webview::{
    ASSET_HOST, ASSET_ORIGIN, ASSET_SCHEME, BackendEvent, Cookie, CustomWebViewController,
    DOCUMENT_START_SCRIPT, OriginPolicy, ScriptInjectionTime, ScriptMessageHandler, Url,
    WatcherGuard, WatcherSet, WebViewConfig, WebViewError, WebViewEvent, WebViewHandle, assets,
    bridge,
};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

/// The page-visible global the bridge script calls — `bridge::SEND_FUNCTION`,
/// which is also the name the script-message transport registers under.
const SEND_FUNCTION: &str = bridge::SEND_FUNCTION;

/// Adapts the shared bridge's one-function transport onto `WebKit`'s
/// message-handler channel — the same `window.webkit.messageHandlers`
/// shim `WebViewWrapper.transportScript` injected.
const TRANSPORT_SCRIPT: &str = concat!(
    "globalThis.__wateruiSend = function (envelope) {",
    "window.webkit.messageHandlers.__wateruiSend.postMessage(envelope);",
    "};"
);

/// The script key order must keep ahead of the bridge and every seed —
/// `WebViewWrapper`'s `waterui:wk-transport` and `waterui:bridge`.
const TRANSPORT_KEY: &str = "waterui:wk-transport";
const BRIDGE_KEY: &str = "waterui:bridge";

/// A user script as the leaf tracks it: the registry key the platform
/// cannot remove by, its source, and its injection time.
struct UserScript {
    key: String,
    source: String,
    time: web_kit::InjectionTime,
}

/// What the handle and the kit's delegate closures share.
///
/// The kit closures capture `Weak`s into this — a handler that outlives its
/// web view has nowhere to deliver a reply, which is exactly what upgrading
/// answers. `view` is a `OnceCell`: the controller is constructed with
/// closures that already point at this state, so it arrives last.
struct Shared {
    /// The kit controller; set once the `Handlers` closing over this state
    /// exist, before any callback can fire.
    view: OnceCell<web_kit::WebViewController>,
    /// `BackendEvent` subscribers.
    watchers: WatcherSet<BackendEvent>,
    /// Page-callable handlers by name; registering a name twice replaces.
    handlers: RefCell<BTreeMap<String, Rc<ScriptMessageHandler>>>,
    /// Which documents may reach the bridge; checked per message.
    bridge_origins: RefCell<Option<OriginPolicy>>,
    /// The last emitted `WillNavigate` URL — `emitWillNavigate`'s dedupe.
    last_navigation: RefCell<Option<String>>,
    /// `WebViewWrapper.redirectsEnabled`.
    redirects_enabled: Cell<bool>,
    /// The redirect signal and its guard, for `set_redirects_enabled`.
    redirects: RefCell<Option<(Computed<bool>, BoxWatcherGuard)>>,
    /// Whether the transport + bridge scripts and the message handler were
    /// installed — `ensureBridgeInstalled`.
    bridge_installed: Cell<bool>,
    /// Injected user scripts by key, in injection order — replacement
    /// rebuilds the whole list, and a stable order keeps transport ahead of
    /// bridge ahead of the seeds.
    scripts: RefCell<Vec<UserScript>>,
    /// The origin this view serves bundled assets under, when armed.
    asset_origin: Option<Url>,
}

impl Shared {
    /// The kit controller.
    fn view(&self) -> &web_kit::WebViewController {
        self.view
            .get()
            .expect("the controller is installed before any callback can run")
    }

    /// `emitEvent` — every user-facing event wraps in `BackendEvent::Event`.
    fn emit(&self, event: WebViewEvent) {
        self.watchers.emit(&BackendEvent::Event(event));
    }

    /// `emitStateChanged` — the navigation-state event is not user-facing.
    fn emit_state_changed(&self) {
        self.watchers.emit(&BackendEvent::NavigationState {
            can_go_back: self.view().can_go_back(),
            can_go_forward: self.view().can_go_forward(),
        });
    }

    /// `emitWillNavigate`: empty URLs never emit, and a repeat of the last
    /// navigation only emits for the action kinds that legitimately reload
    /// the same URL.
    fn emit_will_navigate(&self, url: &str, allow_repeat: bool) {
        if url.is_empty() {
            return;
        }
        {
            let mut last = self.last_navigation.borrow_mut();
            if !allow_repeat && last.as_deref() == Some(url) {
                return;
            }
            *last = Some(url.to_string());
        }
        self.emit(WebViewEvent::WillNavigate {
            url: parse_url(url),
        });
    }

    /// `ensureBridgeInstalled` — transport first, then the shared bridge
    /// script, then the single `__wateruiSend` handler every message rides.
    fn ensure_bridge_installed(self: &Rc<Self>) {
        if self.bridge_installed.get() {
            return;
        }
        self.bridge_installed.set(true);
        self.inject_script(
            TRANSPORT_KEY,
            TRANSPORT_SCRIPT,
            web_kit::InjectionTime::DocumentStart,
        );
        self.inject_script(
            BRIDGE_KEY,
            DOCUMENT_START_SCRIPT,
            web_kit::InjectionTime::DocumentStart,
        );
        let shared = Rc::downgrade(self);
        self.view().add_script_message_handler(
            SEND_FUNCTION,
            Rc::new(move |message| {
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                shared.on_script_message(&message);
            }),
        );
    }

    /// `injectScript` — keyed replacement that preserves each key's original
    /// position, since `WKUserContentController` has no per-script removal.
    fn inject_script(&self, key: &str, source: &str, time: web_kit::InjectionTime) {
        let mut scripts = self.scripts.borrow_mut();
        if let Some(entry) = scripts.iter_mut().find(|entry| entry.key == key) {
            entry.source = source.to_string();
            entry.time = time;
            self.view().remove_all_user_scripts();
            for entry in scripts.iter() {
                self.view().add_user_script(&entry.source, entry.time, true);
            }
        } else {
            scripts.push(UserScript {
                key: key.to_string(),
                source: source.to_string(),
                time,
            });
            self.view().add_user_script(source, time, true);
        }
    }

    /// `frameMayUseBridge`: main frame only, origin authenticated by the
    /// engine and matched against the policy's rules.
    fn frame_may_use_bridge(&self, message: &web_kit::ScriptMessage) -> bool {
        if !message.frame_is_main {
            return false;
        }
        let candidate = origin_candidate(
            &message.frame_scheme,
            &message.frame_host,
            message.frame_port,
        );
        self.bridge_origins
            .borrow()
            .as_ref()
            .is_some_and(|policy| policy.allows_origin(&candidate))
    }

    /// `handleScriptMessage`: authenticate the frame, parse the envelope,
    /// dispatch by name; anything else is a rejected promise, not a panic —
    /// page script reaches this transport directly.
    fn on_script_message(self: &Rc<Self>, message: &web_kit::ScriptMessage) {
        if !self.frame_may_use_bridge(message) {
            tracing::warn!(
                "a document outside the bridge origin policy tried to call a WaterUI handler"
            );
            return;
        }
        let Some(body) = message.body.as_deref() else {
            tracing::warn!("WaterUI bridge received a non-string message body");
            return;
        };
        let request = match bridge::Request::parse(body) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(%error, "WaterUI bridge rejected a malformed envelope");
                return;
            }
        };
        // Release the borrow before invoking: a handler may register or
        // remove handlers on the same view.
        let handler = self.handlers.borrow().get(&request.name).map(Rc::clone);
        let Some(handler) = handler else {
            tracing::warn!(
                handler = %request.name,
                "page script called a WaterUI handler that is not registered"
            );
            self.deliver_reply(
                &bridge::Reply::failure(&format!("no WaterUI handler named `{}`", request.name)),
                request.id,
            );
            return;
        };

        // Handlers are asynchronous; the page's promise settles when the
        // future completes. The reply path holds a `Weak` — a view that died
        // mid-await has no page to answer.
        let future = handler(&request.payload);
        let shared = Rc::downgrade(self);
        executor_core::spawn_local(async move {
            let reply = match future.await {
                Ok(reply) => bridge::Reply::from(reply),
                Err(message) => bridge::Reply::Failure(message),
            };
            if let Some(shared) = shared.upgrade() {
                shared.deliver_reply(&reply, request.id);
            }
        })
        .detach();
    }

    /// `MessageReplyContext`: render the reply script and evaluate it in the
    /// top frame, where the pending promise lives.
    fn deliver_reply(&self, reply: &bridge::Reply, request_id: u64) {
        self.view()
            .evaluate_javascript(&reply.resolve_script(request_id), |_| {});
    }
}

/// The `WebViewHandle` a `WkWebViewHandle` hands out — the `WebViewWrapper`
/// FFI surface, implemented directly on the kit controller.
pub struct WkWebViewHandle {
    shared: Rc<Shared>,
}

impl core::fmt::Debug for WkWebViewHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WkWebViewHandle").finish_non_exhaustive()
    }
}

impl WkWebViewHandle {
    /// Creates a handle owning a fresh kit controller.
    ///
    /// `scheme` is the `waterui://localhost` interception to register while
    /// the `WKWebViewConfiguration` exists — it cannot be added later.
    fn new(
        mtm: MainThreadMarker,
        scheme: Option<web_kit::SchemeHandler>,
        asset_origin: Option<Url>,
    ) -> Self {
        let shared = Rc::new(Shared {
            view: OnceCell::new(),
            watchers: WatcherSet::new(),
            handlers: RefCell::new(BTreeMap::new()),
            bridge_origins: RefCell::new(None),
            last_navigation: RefCell::new(None),
            // `WebViewWrapper`'s default: redirects allowed.
            redirects_enabled: Cell::new(true),
            redirects: RefCell::new(None),
            bridge_installed: Cell::new(false),
            scripts: RefCell::new(Vec::new()),
            asset_origin,
        });
        let weak = Rc::downgrade(&shared);

        let handlers = web_kit::Handlers {
            decide_action: Some(Box::new({
                let weak = weak.clone();
                move |_view, info| {
                    let Some(shared) = weak.upgrade() else {
                        return web_kit::Policy::Allow;
                    };
                    shared.decide_action(info)
                }
            })),
            decide_response: Some(Box::new({
                let weak = weak.clone();
                move |_view, info| {
                    let Some(shared) = weak.upgrade() else {
                        return web_kit::Policy::Allow;
                    };
                    shared.decide_response(info)
                }
            })),
            authentication: Some(Box::new({
                let weak = weak.clone();
                move |_view, challenge| {
                    let Some(shared) = weak.upgrade() else {
                        return web_kit::ChallengeDecision::PerformDefault;
                    };
                    shared.decide_authentication(challenge)
                }
            })),
            event: Some(Box::new(move |_view, event| {
                let Some(shared) = weak.upgrade() else {
                    return;
                };
                shared.on_event(event);
            })),
        };

        let controller = web_kit::WebViewController::new(mtm, handlers, scheme);
        shared
            .view
            .set(controller)
            .expect("a controller is installed exactly once");
        Self { shared }
    }

    /// The kit controller.
    fn controller(&self) -> &web_kit::WebViewController {
        self.shared.view()
    }
}

/// `decidePolicyForNavigationAction`.
impl Shared {
    /// `decidePolicyForNavigationAction`: a window-open request has no
    /// target frame — emit, load it here, and cancel; a main-frame action
    /// emits `WillNavigate`; everything is allowed.
    fn decide_action(&self, info: &web_kit::ActionInfo) -> web_kit::Policy {
        let url = info.url_string().unwrap_or_default();
        let allow_repeat = matches!(
            info.navigation_type,
            web_kit::NavigationType::Reload
                | web_kit::NavigationType::BackForward
                | web_kit::NavigationType::FormSubmitted
                | web_kit::NavigationType::FormResubmitted
        );
        if !info.has_target_frame {
            self.emit_will_navigate(&url, allow_repeat);
            self.view().load_request(&info.request);
            return web_kit::Policy::Cancel;
        }
        if info.target_is_main {
            self.emit_will_navigate(&url, allow_repeat);
        }
        web_kit::Policy::Allow
    }

    /// `decidePolicyForNavigationResponse`: a main-frame 3xx while redirects
    /// are disabled emits `Redirect`, stops the load, and cancels.
    fn decide_response(&self, info: &web_kit::ResponseInfo) -> web_kit::Policy {
        if !info.is_main_frame {
            return web_kit::Policy::Allow;
        }
        let Some(status) = info.status_code else {
            return web_kit::Policy::Allow;
        };
        if !(300..400).contains(&status) {
            return web_kit::Policy::Allow;
        }
        if self.redirects_enabled.get() {
            return web_kit::Policy::Allow;
        }
        let to = info
            .redirect_location
            .as_deref()
            .map_or_else(String::new, |location| {
                web_kit::resolve_redirect(&info.url, location)
            });
        self.emit(WebViewEvent::Redirect {
            from: parse_url(&info.url),
            to: parse_url(&to),
        });
        self.view().stop_loading();
        web_kit::Policy::Cancel
    }

    /// `didReceiveAuthenticationChallenge`: a server-trust challenge that
    /// verifies takes the credential; one that does not emits an `Ssl`
    /// error and cancels. Everything else falls to default handling.
    fn decide_authentication(
        &self,
        challenge: &web_kit::ChallengeInfo,
    ) -> web_kit::ChallengeDecision {
        if challenge.method == "NSURLAuthenticationMethodServerTrust"
            && let Some(trust) = challenge.server_trust.as_ref()
        {
            match web_kit::evaluate_server_trust(trust) {
                Ok(()) => return web_kit::ChallengeDecision::UseCredential,
                Err(message) => {
                    self.emit(WebViewEvent::Error(WebViewError::Ssl {
                        url: parse_url(&challenge.current_url),
                        message: Str::from(message),
                    }));
                    return web_kit::ChallengeDecision::Cancel;
                }
            }
        }
        web_kit::ChallengeDecision::PerformDefault
    }

    /// The notification events the delegate emits.
    fn on_event(&self, event: web_kit::Event) {
        match event {
            web_kit::Event::StartedProvisional => {
                let url = self.view().url_string().unwrap_or_default();
                self.emit_will_navigate(&url, false);
                self.emit(WebViewEvent::Loading { progress: 0.0 });
                self.emit_state_changed();
            }
            web_kit::Event::ServerRedirect => {
                let to = self.view().url_string().unwrap_or_default();
                let from = self.last_navigation.borrow().clone().unwrap_or_default();
                if !self.redirects_enabled.get() {
                    self.emit(WebViewEvent::Redirect {
                        from: parse_url(&from),
                        to: parse_url(&to),
                    });
                    self.view().stop_loading();
                    return;
                }
                self.emit_will_navigate(&to, true);
            }
            web_kit::Event::Finished => {
                self.emit(WebViewEvent::Loaded);
                self.emit_state_changed();
            }
            web_kit::Event::Failed(message) | web_kit::Event::ProvisionalFailed(message) => {
                self.emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
                    message,
                ))));
            }
            web_kit::Event::Progress(progress) => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "progress is a 0–1 ratio; f32 holds it exactly"
                )]
                self.emit(WebViewEvent::Loading {
                    progress: progress as f32,
                });
            }
            web_kit::Event::OpenRequest(request) => {
                self.view().load_request(&request);
            }
        }
    }
}

impl WebViewHandle for WkWebViewHandle {
    fn go_back(&self) {
        self.controller().go_back();
    }

    fn go_forward(&self) {
        self.controller().go_forward();
    }

    fn go_to(&self, url: &Url) {
        self.controller().load_url(url.as_str());
    }

    fn stop(&self) {
        self.controller().stop_loading();
    }

    fn refresh(&self) {
        self.controller().reload();
    }

    fn set_user_agent(&self, user_agent: &str) {
        let trimmed = user_agent.trim();
        self.controller()
            .set_custom_user_agent((!trimmed.is_empty()).then_some(trimmed));
    }

    fn can_go_back(&self) -> bool {
        self.controller().can_go_back()
    }

    fn can_go_forward(&self) -> bool {
        self.controller().can_go_forward()
    }

    fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        let time = match time {
            ScriptInjectionTime::DocumentStart => web_kit::InjectionTime::DocumentStart,
            ScriptInjectionTime::DocumentEnd => web_kit::InjectionTime::DocumentEnd,
        };
        self.shared.inject_script(key, script, time);
    }

    fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>) {
        self.shared
            .handlers
            .borrow_mut()
            .insert(name.to_string(), Rc::from(handler));
        self.shared.ensure_bridge_installed();
    }

    fn remove_handler(&self, name: &str) {
        self.shared.handlers.borrow_mut().remove(name);
    }

    fn set_bridge_origins(&self, policy: OriginPolicy) {
        self.shared.bridge_origins.replace(Some(policy));
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        let header = cookie.to_string();
        self.controller()
            .set_cookie_header(&header)
            .expect("WebView received an invalid Set-Cookie value");
    }

    fn asset_origin(&self) -> Option<Url> {
        self.shared.asset_origin.clone()
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        let enabled = Computed::new(enabled);
        self.shared.redirects_enabled.set(enabled.snapshot());
        let shared = Rc::clone(&self.shared);
        let guard = enabled.watch(move |context| {
            shared.redirects_enabled.set(context.into_value());
        });
        self.shared.redirects.replace(Some((enabled, guard)));
    }

    fn watch(&self, watcher: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.shared.watchers.insert(watcher)
    }

    #[expect(
        clippy::future_not_send,
        reason = "WebKit and WaterUI view state are confined to the UI thread"
    )]
    async fn get_cookies(&self) -> Vec<Cookie<'static>> {
        let (sender, receiver) = async_channel::bounded(1);
        self.controller().all_cookies(move |records| {
            let cookies = records
                .into_iter()
                .map(cookie_record::into_cookie)
                .collect();
            let _ = sender.try_send(cookies);
        });
        receiver.recv().await.unwrap_or_default()
    }

    #[expect(
        clippy::future_not_send,
        reason = "WebKit and WaterUI view state are confined to the UI thread"
    )]
    async fn run_javascript(&self, script: &str) -> Result<Str, Str> {
        let (sender, receiver) = async_channel::bounded(1);
        self.controller()
            .evaluate_javascript(script, move |result| {
                let _ = sender.try_send(result.map(Str::from).map_err(Str::from));
            });
        receiver.recv().await.unwrap_or_else(|error| {
            Err(Str::from(format!(
                "WebView JavaScript channel closed: {error}"
            )))
        })
    }

    #[expect(
        clippy::future_not_send,
        reason = "WebKit and WaterUI view state are confined to the UI thread"
    )]
    async fn call_async_javascript(&self, body: &str) -> Result<Str, Str> {
        let (sender, receiver) = async_channel::bounded(1);
        self.controller()
            .call_async_javascript(body, move |result| {
                let _ = sender.try_send(result.map(Str::from).map_err(Str::from));
            });
        receiver.recv().await.unwrap_or_else(|error| {
            Err(Str::from(format!(
                "WebView JavaScript channel closed: {error}"
            )))
        })
    }
}

/// A `CookieRecord` as a `cookie::Cookie` — the same conversion WPE writes,
/// with unknown `SameSite` values dropped like it does.
mod cookie_record {
    use super::{Cookie, web_kit};
    use cookie::time::OffsetDateTime;

    /// Converts the kit's cookie record into the cookie type the handle
    /// contract answers with.
    pub(super) fn into_cookie(record: web_kit::CookieRecord) -> Cookie<'static> {
        let mut builder = Cookie::build((record.name, record.value))
            .domain(record.domain)
            .path(record.path)
            .secure(record.secure)
            .http_only(record.http_only);
        if let Some(same_site) = record.same_site {
            match same_site.as_str() {
                "Strict" => builder = builder.same_site(cookie::SameSite::Strict),
                "Lax" => builder = builder.same_site(cookie::SameSite::Lax),
                "None" => builder = builder.same_site(cookie::SameSite::None),
                other => {
                    tracing::warn!(
                        same_site = other,
                        "ignoring an unknown cookie SameSite value"
                    );
                }
            }
        }
        if let Some(expires) = record.expires {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "cookie expiries are positive unix seconds that fit i64"
            )]
            match OffsetDateTime::from_unix_timestamp(expires as i64) {
                Ok(expires) => builder = builder.expires(expires),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "ignoring a cookie expiry that is outside the representable range"
                    );
                }
            }
        }
        builder.build()
    }
}

/// The platform `WebViewController` the late install puts into the
/// environment — `installWebViewController`'s `WebViewWrapper` factory.
struct AppleWebViewController;

impl core::fmt::Debug for AppleWebViewController {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppleWebViewController").finish()
    }
}

impl CustomWebViewController for AppleWebViewController {
    fn open(&self, config: WebViewConfig) -> impl WebViewHandle {
        let mtm = MainThreadMarker::new().expect("a WebView opens on the main thread");
        // Interception can only be registered while the configuration is
        // being assembled, so the asset server arrives at creation.
        let scheme = config.asset_server.map(|server| asset_scheme(&server));
        let asset_origin = scheme.as_ref().map(|_| Url::new(ASSET_ORIGIN));
        WkWebViewHandle::new(mtm, scheme, asset_origin)
    }
}

/// A URL the event payloads carry: backend-emitted strings that do not
/// parse are a bug loud enough to stop for, matching the FFI layer.
fn parse_url(string: &str) -> Url {
    string.parse().unwrap_or_else(|error| {
        panic!("the web view backend emitted an unparseable URL {string:?}: {error}")
    })
}

/// The leaf's layout face: `WuiWebViewComponent.sizeThatFits` — the
/// proposal's axes where given, 320×480 otherwise, stretching both ways.
struct WebViewSubView;

impl core::fmt::Debug for WebViewSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WebViewSubView").finish()
    }
}

impl SubView for WebViewSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(320.0),
            proposal.height.unwrap_or(480.0),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `webview` handler: `Native<WebView>` mounts the kit view the
/// handle owns, keeping the `WebView` — and with it the event, navigation
/// and signal subscriptions — alive for the leaf's life.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<waterui_webview::WebView>(|webview, _ctx| {
        let handle = webview.handle().downcast_ref::<WkWebViewHandle>().expect(
            "a WebView leaf was handed a handle from another engine; the \
                 WebViewController in this environment is not the Apple backend's",
        );
        let mut leaf = NativeLeaf::new(&**handle.controller().view(), WebViewSubView);
        // The component owns the subscriptions that drive `can_go_back`,
        // `can_go_forward` and the event signal; dropping it would leave a
        // live view whose reactive state had gone dead.
        leaf.keep(webview);
        leaf
    });
}

/// The `SchemeHandler` an `asset_origin` view registers — `AssetSchemeHandler`,
/// routing through `assets::dispatch` so method enforcement and traversal
/// refusal are the shared implementation's.
fn asset_scheme(server: &assets::AssetServer) -> web_kit::SchemeHandler {
    let server = Arc::clone(server);
    web_kit::SchemeHandler {
        scheme: String::from(ASSET_SCHEME),
        host: String::from(ASSET_HOST),
        respond: Arc::new(move |request: &web_kit::SchemeRequest| {
            let response = assets::dispatch(
                &server,
                &request.method,
                &request.path,
                request.query.as_deref().or(Some("")),
            );
            web_kit::SchemeResponse {
                status: response.status,
                headers: response
                    .headers
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
                body: response.body,
            }
        }),
    }
}

/// Installs the platform `WebViewController` into `env` when the application
/// left the slot empty — the late fill `waterui_swift_install_webview` /
/// `installWebViewController` performed, so an application bundling its own
/// engine keeps the controller it installed during `app(env)`.
///
pub(crate) fn install_service(env: &mut Environment) {
    if env.get::<waterui_webview::WebViewController>().is_none() {
        env.insert(waterui_webview::WebViewController::new(
            AppleWebViewController,
        ));
    }
}

/// The `scheme://host[:port]` candidate `frame_may_use_bridge` matches —
/// `WebViewWrapper.frameMayUseBridge`'s formatting: the port only when the
/// engine reports a non-default one.
fn origin_candidate(scheme: &str, host: &str, port: i64) -> String {
    if port == 0 {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_candidate_formats_port_only_when_explicit() {
        assert_eq!(
            origin_candidate("https", "example.com", 0),
            "https://example.com"
        );
        assert_eq!(
            origin_candidate("https", "example.com", 8443),
            "https://example.com:8443"
        );
    }

    #[test]
    fn origin_policy_matches_candidates() {
        use waterui_webview::BridgeOrigins;
        let initial: Url = "http://localhost:3000/app".parse().expect("parses");
        let policy = OriginPolicy::new(BridgeOrigins::Initial, &initial);

        // `frame_may_use_bridge` formats what the engine reports; the policy
        // matches the port-bearing origin, not a port-stripped one.
        assert!(policy.allows_origin(&origin_candidate("http", "localhost", 3000)));
        assert!(!policy.allows_origin(&origin_candidate("http", "localhost", 0)));
        assert!(!policy.allows_origin(&origin_candidate("https", "localhost", 3000)));
    }

    #[test]
    fn cookie_record_roundtrips_attributes() {
        let record = web_kit::CookieRecord {
            name: String::from("session"),
            value: String::from("abc"),
            domain: String::from("example.com"),
            path: String::from("/"),
            expires: Some(4_102_444_800.0),
            secure: true,
            http_only: true,
            same_site: Some(String::from("Strict")),
        };
        let cookie = cookie_record::into_cookie(record);
        let header = cookie.to_string();
        assert!(header.contains("session=abc"));
        assert!(header.contains("Domain=example.com"));
        assert!(header.contains("Path=/"));
        assert!(header.contains("Secure"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Strict"));
        assert!(header.contains("Expires="));
    }

    #[test]
    fn cookie_record_ignores_unknown_same_site() {
        let record = web_kit::CookieRecord {
            name: String::from("a"),
            value: String::from("b"),
            domain: String::from("example.com"),
            path: String::from("/"),
            expires: None,
            secure: false,
            http_only: false,
            same_site: Some(String::from("Unrecognized")),
        };
        assert!(
            !cookie_record::into_cookie(record)
                .to_string()
                .contains("SameSite")
        );
    }

    #[test]
    fn bridge_envelope_parse_and_reply() {
        // The transport contract: a JSON envelope parses into id/name/payload,
        // and a reply renders back into a script the page can evaluate.
        let request =
            bridge::Request::parse(r#"{"id":7,"name":"alert","payload":{"message":"hi"}}"#)
                .expect("a well-formed envelope parses");
        assert_eq!(request.id, 7);
        assert_eq!(request.name, "alert");

        let script = bridge::Reply::failure("boom").resolve_script(request.id);
        assert!(script.contains("boom"));
    }
}
