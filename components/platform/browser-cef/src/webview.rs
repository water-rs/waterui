use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cef::{ImplBrowser, ImplBrowserHost, ImplFrame};
use cookie::{Expiration, SameSite};
use serde_json::Value;
use suiteki::Str;
use waterui_core::{Computed, Signal};
use waterui_url::Url;
use waterui_webview::{
    ASSET_ORIGIN, Cookie, CustomWebViewController, OriginPolicy, ScriptInjectionTime, WatcherGuard,
    WebViewConfig, WebViewHandle, bridge,
};

use crate::cdp::{CefCdpSession, protocol};
use crate::page::{CefController, CefPageConfiguration, CefPageHandle, CefPageMode};
use crate::renderer::{RendererBundle, document_source};

type MessageHandler = waterui_webview::ScriptMessageHandler;

#[derive(Clone)]
/// Standard `WaterUI` `WebView` handle backed by a CEF page.
pub struct CefWebViewHandle {
    page: CefPageHandle,
    handlers: Rc<RefCell<HashMap<String, Rc<MessageHandler>>>>,
    /// Scripts injected after creation, by key, so injecting again under a
    /// key replaces the script rather than stacking another copy in front of
    /// it. The creation-time scripts live in every renderer's copy of the
    /// browser's `extra_info` instead; their keys never arrive here, because
    /// `AnyWebViewHandle::inject_script` refuses them.
    scripts: Rc<RefCell<HashMap<String, String>>>,
    /// Unregisters the CDP event watcher when the last clone goes away.
    ///
    /// Without it the watcher — which captures the page — stayed in the
    /// session's list forever, and page and watcher held each other up.
    _events: Rc<WatcherGuard>,
    /// Closes the browser when the last clone of this handle goes away.
    _close: Rc<CloseBrowserOnDrop>,
}

/// Closes the CEF browser when the web view that owned it is gone.
///
/// Nothing on the web view path used to close a browser at all: dropping the
/// handle released the Rust side and left the renderer process, its frame
/// production and its accelerated-paint sink running for the life of the
/// application — and `CefShutdown` then ran with live browsers, which CEF
/// documents as undefined behaviour.
struct CloseBrowserOnDrop {
    page: CefPageHandle,
}

impl Drop for CloseBrowserOnDrop {
    fn drop(&mut self) {
        // The close handshake completes on the CEF message loop; nothing here
        // waits for it, but the request itself must go out.
        self.page.request_close();
    }
}

impl core::fmt::Debug for CefWebViewHandle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CefWebViewHandle")
            .finish_non_exhaustive()
    }
}

impl CefWebViewHandle {
    /// Wraps a page whose browser was created with the bundle of `config`.
    ///
    /// The renderer decides which documents get the bridge; this side
    /// authenticates every call against the same admission rules, because the
    /// CDP binding reaches every context of every frame regardless.
    fn new(page: CefPageHandle, config: &WebViewConfig) -> Self {
        let handlers = Rc::new(RefCell::new(HashMap::<String, Rc<MessageHandler>>::new()));
        let origins = config.bridge_origins.clone();
        let contexts = Rc::new(RefCell::new(HashMap::<i64, TrackedContext>::new()));
        let main_frame = Rc::new(RefCell::new(None::<String>));
        let scripts = Rc::new(RefCell::new(HashMap::<String, String>::new()));
        let session = page.cdp();
        install_bridge(&session, &main_frame);
        let events = session.watch_events({
            let handlers = Rc::clone(&handlers);
            let contexts = Rc::clone(&contexts);
            let main_frame = Rc::clone(&main_frame);
            let session = session.clone();
            move |event| match event.method.as_str() {
                "Runtime.executionContextCreated" => track_context(&contexts, &event.params),
                "Page.frameNavigated" => track_main_frame(&main_frame, &event.params),
                "Runtime.executionContextDestroyed" => {
                    if let Some(id) = context_id(&event.params, "executionContextId") {
                        contexts.borrow_mut().remove(&id);
                    }
                }
                "Runtime.executionContextsCleared" => contexts.borrow_mut().clear(),
                "Runtime.bindingCalled" => {
                    let context = context_id(&event.params, "executionContextId");
                    if !frame_may_use_bridge(&contexts, &main_frame, &origins, context) {
                        tracing::warn!(
                            "a frame outside the bridge origin policy tried to call a WaterUI handler"
                        );
                        return;
                    }
                    dispatch_bridge_call(&session, &handlers, &contexts, &event.params, context);
                }
                _ => {}
            }
        });
        Self {
            _close: Rc::new(CloseBrowserOnDrop { page: page.clone() }),
            page,
            handlers,
            scripts,
            _events: Rc::new(events),
        }
    }

    /// Returns the underlying CEF page for renderer integration.
    #[must_use]
    pub const fn page(&self) -> &CefPageHandle {
        &self.page
    }

    fn session(&self) -> CefCdpSession {
        self.page.cdp()
    }
}

impl WebViewHandle for CefWebViewHandle {
    fn go_back(&self) {
        self.page
            .host()
            .browser()
            .expect("CEF WebView host must expose its browser")
            .go_back();
    }

    fn go_forward(&self) {
        self.page
            .host()
            .browser()
            .expect("CEF WebView host must expose its browser")
            .go_forward();
    }

    fn go_to(&self, url: &Url) {
        self.page.navigate(url);
    }

    fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        // `Page.addScriptToEvaluateOnNewDocument` runs in every document of
        // every frame. The renderer defines `waterui` in exactly the documents
        // it admitted — the main frame's default world of a document the rules
        // admit — and does so before Chromium runs these sources, so its
        // presence at document start is the admission decision, made once.
        let source = format!(
            "if(globalThis.waterui!==undefined){{{}}}",
            document_source(script, time)
        );
        let session = self.session();
        // Add before removing, so a document that commits between the two runs
        // one of the versions rather than none.
        let added =
            session.execute(&protocol::AddScriptToEvaluateOnNewDocument { source: &source });
        let previous = self.scripts.borrow().get(key).cloned();
        if let Some(identifier) = previous {
            execute_without_result(
                &session,
                &protocol::RemoveScriptToEvaluateOnNewDocument {
                    identifier: &identifier,
                },
            );
        }
        let scripts = Rc::clone(&self.scripts);
        let key = key.to_string();
        executor_core::spawn_local(async move {
            match added.await {
                Ok(script) => {
                    scripts.borrow_mut().insert(key, script.identifier);
                }
                Err(error) => {
                    tracing::warn!(%error, "CEF refused a WaterUI document-start script");
                }
            }
        })
        .detach();
    }

    fn add_handler(&self, name: &str, handler: Box<waterui_webview::ScriptMessageHandler>) {
        assert!(
            !name.is_empty(),
            "CEF WebView handler name must not be empty"
        );
        // Registering the same name twice replaces the handler, matching every other
        // backend; the previous one is simply dropped.
        self.handlers
            .borrow_mut()
            .insert(name.to_string(), Rc::from(handler));
    }

    fn remove_handler(&self, name: &str) {
        // Removing a name that was never registered is a no-op, matching every other
        // backend.
        self.handlers.borrow_mut().remove(name);
    }

    fn stop(&self) {
        self.page.stop();
    }

    fn refresh(&self) {
        self.page.reload();
    }

    fn set_user_agent(&self, user_agent: &str) {
        execute_without_result(
            &self.session(),
            &protocol::SetUserAgentOverride { user_agent },
        );
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        self.page.set_redirects_enabled(Computed::new(enabled));
    }

    fn watch(&self, watcher: impl Fn(waterui_webview::BackendEvent) + 'static) -> WatcherGuard {
        self.page.watch_webview(watcher)
    }

    fn can_go_back(&self) -> bool {
        self.page
            .host()
            .browser()
            .expect("CEF WebView host must expose its browser")
            .can_go_back()
            == 1
    }

    fn can_go_forward(&self) -> bool {
        self.page
            .host()
            .browser()
            .expect("CEF WebView host must expose its browser")
            .can_go_forward()
            == 1
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        let browser = self
            .page
            .host()
            .browser()
            .expect("CEF WebView host must expose its browser");
        let current_url = browser
            .main_frame()
            .expect("CEF WebView must expose its main frame")
            .url();
        let current_url = cef::CefString::from(&current_url).to_string();
        let same_site = cookie.same_site().map(|same_site| match same_site {
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        });
        let expires = match cookie.expires() {
            Some(Expiration::DateTime(time)) => Some(time.unix_timestamp()),
            Some(Expiration::Session) | None => None,
        };
        // A cookie that names its own domain is stored against that domain; only
        // one without falls back to the document's URL. Sending both let the URL
        // win, which quietly stored cross-domain cookies on the wrong domain.
        let domain = cookie.domain();
        execute_without_result(
            &self.session(),
            &protocol::SetCookie {
                name: cookie.name(),
                value: cookie.value(),
                domain,
                url: domain.is_none().then_some(current_url.as_str()),
                path: cookie.path(),
                secure: cookie.secure().unwrap_or(false),
                http_only: cookie.http_only().unwrap_or(false),
                same_site,
                expires,
            },
        );
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn get_cookies(&self) -> Result<Vec<Cookie<'static>>, waterui_core::Error> {
        // The cookies of the current document, not every cookie in the profile:
        // `Network.getAllCookies` returned the whole store, which is not what
        // "the cookies for this web view" means on any other backend.
        let response = self
            .session()
            .execute(&protocol::GetCookies { urls: Vec::new() })
            .await
            .map_err(|error| {
                waterui_core::Error::new(error).context("CEF failed to retrieve WebView cookies")
            })?;
        response
            .cookies
            .iter()
            .map(cookie_from_cdp)
            .collect::<Result<Vec<_>, _>>()
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn run_javascript(&self, script: &str) -> Result<Str, Str> {
        self.evaluate(&waterui_webview::raw_evaluation(script))
            .await
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn call_async_javascript(&self, body: &str) -> Result<Str, Str> {
        // `body` is a function body, so it cannot be evaluated as an
        // expression; wrapping it in an async IIFE gives CDP the promise it
        // awaits.
        self.evaluate(&format!("(async () => {{ {body} }})()"))
            .await
    }
}

impl CefWebViewHandle {
    /// Evaluates `expression` in the main frame, awaiting a promise result,
    /// and answers the string it produced.
    ///
    /// Both paths evaluate scripts that encode their answer in the page — the
    /// raw path through `waterui_webview::raw_evaluation`, the awaited one
    /// through the shared wrapper's envelope — so the value is a string,
    /// returned as it is. Anything else means the script broke that contract
    /// and is reported rather than re-encoded here.
    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn evaluate(&self, expression: &str) -> Result<Str, Str> {
        let response = self
            .session()
            .execute(&protocol::Evaluate {
                expression,
                await_promise: true,
                return_by_value: true,
                unique_context_id: None,
            })
            .await
            .map_err(|error| Str::from(error.to_string()))?;
        if let Some(exception) = response.exception_details {
            return Err(Str::from(exception.text));
        }
        match response.result.value {
            Some(Value::String(value)) => Ok(Str::from(value)),
            Some(value) => Err(Str::from(format!(
                "the script evaluated to `{value}`, not the string it must answer"
            ))),
            None => Err(Str::from(format!(
                "the script evaluated to {}, not the string it must answer",
                response
                    .result
                    .description
                    .as_deref()
                    .unwrap_or("`undefined`")
            ))),
        }
    }
}

impl CustomWebViewController for CefController {
    fn open(&self, config: WebViewConfig) -> impl WebViewHandle {
        let page = self.open_page(
            CefPageConfiguration {
                renderer: Some(RendererBundle::new(&config)),
                ..CefPageConfiguration::default()
            },
            CefPageMode::Visible,
        );
        let handle = CefWebViewHandle::new(page, &config);
        // The factory goes onto the page's private request context before any
        // asset URL can navigate — the view still sits on `about:blank` here.
        if let Some(server) = config.asset_server {
            crate::assets::register_scheme_handler(&handle.page.request_context(), server);
        }
        handle
    }

    fn asset_origin(&self) -> Option<Url> {
        Some(Url::new(ASSET_ORIGIN))
    }
}

/// What the host keeps about one live execution context.
///
/// `origin`, `is_default` and `frame_id` answer the bridge admission check;
/// `unique_id` names the context to `Runtime.evaluate` when a reply goes back
/// to it. The map is keyed by the
/// numeric `executionContextId` because that is what `Runtime.bindingCalled`
/// and the destruction events report — but the number is a per-renderer
/// handle, and after a cross-process navigation another renderer can reuse it,
/// so the reply itself is addressed by the unique id.
struct TrackedContext {
    /// The context's origin, for [`frame_may_use_bridge`].
    origin: String,
    /// The context's `uniqueId` — the `uniqueContextId` a reply is sent to.
    unique_id: String,
    /// Whether the context is its frame's default world. An isolated world
    /// reaches the binding too, and must not reach the bridge.
    is_default: bool,
    /// The frame the context belongs to; only the main frame's contexts may
    /// send.
    frame_id: String,
}

/// Records what the bridge needs about a newly created execution context.
///
/// Everything arrives in the `context` object of
/// `Runtime.executionContextCreated`; a context whose `uniqueId`, world or
/// frame is missing can neither be authenticated for a call nor addressed for
/// a reply, so it is not tracked at all.
fn track_context(contexts: &RefCell<HashMap<i64, TrackedContext>>, params: &Value) {
    let Some(context) = params.get("context") else {
        return;
    };
    let Some(id) = context.get("id").and_then(Value::as_i64) else {
        return;
    };
    let Some(unique_id) = context.get("uniqueId").and_then(Value::as_str) else {
        return;
    };
    let Some(aux) = context.get("auxData") else {
        return;
    };
    let Some(is_default) = aux.get("isDefault").and_then(Value::as_bool) else {
        return;
    };
    let Some(frame_id) = aux.get("frameId").and_then(Value::as_str) else {
        return;
    };
    let origin = context
        .get("origin")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    contexts.borrow_mut().insert(
        id,
        TrackedContext {
            origin,
            unique_id: unique_id.to_string(),
            is_default,
            frame_id: frame_id.to_string(),
        },
    );
}

/// Records the main frame's id from `Page.frameNavigated`: the frame with no
/// `parentId` is the main one.
fn track_main_frame(main_frame: &RefCell<Option<String>>, params: &Value) {
    let Some(frame) = params.get("frame") else {
        return;
    };
    if frame.get("parentId").is_some() {
        return;
    }
    if let Some(id) = frame.get("id").and_then(Value::as_str) {
        main_frame.replace(Some(id.to_string()));
    }
}

/// Reads an execution-context id out of an event's parameters.
fn context_id(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(Value::as_i64)
}

/// Whether the frame a bridge call came from may use the bridge.
///
/// CEF installs the binding in every execution context, so this is the gate,
/// and it has to be answered about the *calling* context: the main frame's
/// default world, at an admitted origin. A subframe of an admitted page, an
/// isolated world of the main frame and a context nobody saw created are all
/// refused.
fn frame_may_use_bridge(
    contexts: &RefCell<HashMap<i64, TrackedContext>>,
    main_frame: &RefCell<Option<String>>,
    policy: &OriginPolicy,
    context: Option<i64>,
) -> bool {
    let Some(context) = context else {
        return false;
    };
    let contexts = contexts.borrow();
    let Some(tracked) = contexts.get(&context) else {
        return false;
    };
    tracked.is_default
        && main_frame.borrow().as_deref() == Some(tracked.frame_id.as_str())
        && policy.allows_origin(&tracked.origin)
}

fn dispatch_bridge_call(
    session: &CefCdpSession,
    handlers: &RefCell<HashMap<String, Rc<MessageHandler>>>,
    contexts: &RefCell<HashMap<i64, TrackedContext>>,
    params: &Value,
    context: Option<i64>,
) {
    let Some(envelope) = params.get("payload").and_then(Value::as_str) else {
        tracing::warn!("CEF bridge binding fired without a string payload; ignoring");
        return;
    };
    let request = match bridge::Request::parse(envelope) {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(%error, "page script sent a malformed WaterUI bridge request");
            return;
        }
    };
    // The reply has to find the document that made the call. The numeric
    // `executionContextId` is a per-renderer handle, and after a
    // cross-process navigation another renderer can reuse it — evaluating
    // against the number would deliver the reply into a different document.
    // The unique id cannot be reused, so the calling context is resolved to
    // it while the call is being dispatched. `frame_may_use_bridge` admitted
    // the call, which means the context is a tracked one. Chromium refuses an
    // evaluation addressed to a unique id whose context is gone, so a reply
    // whose document navigated away is dropped by the engine.
    let unique_context_id = context
        .and_then(|id| {
            contexts
                .borrow()
                .get(&id)
                .map(|tracked| tracked.unique_id.clone())
        })
        .expect("an admitted bridge call comes from a tracked execution context");
    // Resolve the handler and release the borrow before invoking it: a handler is
    // free to register or remove handlers on the same web view.
    let handler = handlers.borrow().get(&request.name).map(Rc::clone);
    let Some(handler) = handler else {
        tracing::warn!(
            handler = %request.name,
            "page script called a WaterUI handler that is not registered"
        );
        let reply = bridge::Reply::failure(&format!("no WaterUI handler named `{}`", request.name));
        execute_without_result(
            session,
            &protocol::Evaluate {
                expression: &reply.resolve_script(request.id),
                await_promise: false,
                return_by_value: false,
                unique_context_id: Some(unique_context_id.as_str()),
            },
        );
        return;
    };

    // Handlers are asynchronous, so the promise settles when the future
    // completes rather than when the transport returns.
    let future = handler(&request.payload);
    let session = session.clone();
    executor_core::spawn_local(async move {
        let reply = match future.await {
            Ok(reply) => bridge::Reply::from(reply),
            Err(message) => bridge::Reply::Failure(message),
        };
        execute_without_result(
            &session,
            &protocol::Evaluate {
                expression: &reply.resolve_script(request.id),
                await_promise: false,
                return_by_value: false,
                unique_context_id: Some(unique_context_id.as_str()),
            },
        );
    })
    .detach();
}

/// Opens the `DevTools` side of the bridge: the transport binding and the
/// events that authenticate its calls. The page-facing scripts are the
/// renderer's to inject.
fn install_bridge(session: &CefCdpSession, main_frame: &Rc<RefCell<Option<String>>>) {
    // Both enables come before the things that need them: context and frame
    // events authenticate calls, and Chromium only runs later document-start
    // scripts while the page domain is enabled.
    execute_without_result(session, &protocol::RuntimeEnable {});
    execute_without_result(session, &protocol::PageEnable {});
    execute_without_result(
        session,
        &protocol::AddBinding {
            name: bridge::SEND_FUNCTION,
        },
    );
    // The main frame keeps its id for the page's life, so it is read once,
    // before anything navigates; `Page.frameNavigated` only ever repeats it.
    let tree = session.execute(&protocol::GetFrameTree {});
    let main_frame = Rc::clone(main_frame);
    executor_core::spawn_local(async move {
        let tree = tree
            .await
            .expect("CEF must describe the frame tree of a page it just created");
        main_frame.replace(Some(tree.frame_tree.frame.id));
    })
    .detach();
}

/// Issues a command whose result nobody waits for.
///
/// A failure is still reported: `CefCdpSession` logs any error whose receiver
/// was dropped, so these do not vanish the way they used to.
fn execute_without_result<C: protocol::CdpCommand>(session: &CefCdpSession, command: &C) {
    drop(session.execute(command));
}

fn cookie_from_cdp(cookie: &protocol::Cookie) -> Result<Cookie<'static>, waterui_core::Error> {
    let mut builder = Cookie::build((cookie.name.clone(), cookie.value.clone()))
        .domain(cookie.domain.clone())
        .path(cookie.path.clone())
        .secure(cookie.secure)
        .http_only(cookie.http_only);
    if let Some(same_site) = cookie.same_site.as_deref() {
        // An unrecognised policy is Chromium's business, not a reason to abort:
        // the cookie is still usable without it.
        match same_site {
            "Strict" => builder = builder.same_site(SameSite::Strict),
            "Lax" => builder = builder.same_site(SameSite::Lax),
            "None" => builder = builder.same_site(SameSite::None),
            other => tracing::warn!(
                same_site = other,
                "ignoring an unknown cookie SameSite value"
            ),
        }
    }
    // CDP reports a non-positive `expires` for a session cookie; anything
    // else, NaN included, is an expiry that must convert.
    if cookie.expires.is_nan() || cookie.expires > 0.0 {
        builder = builder.expires(waterui_webview::cookie_expiry(
            &cookie.name,
            cookie.expires,
        )?);
    }
    Ok(builder.build())
}
