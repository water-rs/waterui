use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cef::{ImplBrowser, ImplBrowserHost, ImplFrame};
use cookie::{Expiration, SameSite, time::OffsetDateTime};
use num_traits::ToPrimitive as _;
use serde_json::Value;
use waterui_core::{Computed, Signal};
use waterui_str::Str;
use waterui_url::Url;
use waterui_webview::{
    Cookie, CustomWebViewController, ScriptInjectionTime, WatcherGuard, WebViewHandle, bridge,
};

use crate::cdp::{CefCdpSession, protocol};
use crate::page::{CefController, CefPageConfiguration, CefPageHandle, CefPageMode};

type MessageHandler = waterui_webview::ScriptMessageHandler;

#[derive(Clone)]
/// Standard `WaterUI` `WebView` handle backed by a CEF page.
pub struct CefWebViewHandle {
    page: CefPageHandle,
    handlers: Rc<RefCell<HashMap<String, Rc<MessageHandler>>>>,
    /// Which documents may reach the bridge. Checked on every call, because the
    /// CDP binding is installed for every execution context and any frame in
    /// the page can reach it.
    origins: Rc<RefCell<Option<waterui_webview::OriginPolicy>>>,
    /// Document-start scripts by key, so injecting again under a key replaces
    /// the script rather than stacking another copy in front of it.
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
    fn new(page: CefPageHandle) -> Self {
        let handlers = Rc::new(RefCell::new(HashMap::<String, Rc<MessageHandler>>::new()));
        let origins: Rc<RefCell<Option<waterui_webview::OriginPolicy>>> =
            Rc::new(RefCell::new(None));
        let contexts = Rc::new(RefCell::new(HashMap::<i64, String>::new()));
        let scripts = Rc::new(RefCell::new(HashMap::<String, String>::new()));
        let session = page.cdp();
        install_bridge(&session);
        let events = session.watch_events({
            let handlers = Rc::clone(&handlers);
            let origins = Rc::clone(&origins);
            let contexts = Rc::clone(&contexts);
            let session = session.clone();
            move |event| match event.method.as_str() {
                "Runtime.executionContextCreated" => track_context(&contexts, &event.params),
                "Runtime.executionContextDestroyed" => {
                    if let Some(id) = context_id(&event.params, "executionContextId") {
                        contexts.borrow_mut().remove(&id);
                    }
                }
                "Runtime.executionContextsCleared" => contexts.borrow_mut().clear(),
                "Runtime.bindingCalled" => {
                    let context = context_id(&event.params, "executionContextId");
                    if !frame_may_use_bridge(&contexts, &origins, context) {
                        tracing::warn!(
                            "a frame outside the bridge origin policy tried to call a WaterUI handler"
                        );
                        return;
                    }
                    dispatch_bridge_call(&session, &handlers, &event.params, context);
                }
                _ => {}
            }
        });
        Self {
            _close: Rc::new(CloseBrowserOnDrop { page: page.clone() }),
            page,
            handlers,
            origins,
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
        let source = match time {
            ScriptInjectionTime::DocumentStart => script.to_string(),
            ScriptInjectionTime::DocumentEnd => format!(
                "globalThis.addEventListener('DOMContentLoaded',()=>globalThis.eval({}),{{once:true}});",
                serde_json::to_string(script).expect("WebView script must serialize")
            ),
        };
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

    fn set_bridge_origins(&self, policy: waterui_webview::OriginPolicy) {
        self.origins.replace(Some(policy));
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
    async fn get_cookies(&self) -> Vec<Cookie<'static>> {
        // The cookies of the current document, not every cookie in the profile:
        // `Network.getAllCookies` returned the whole store, which is not what
        // "the cookies for this web view" means on any other backend.
        let response = self
            .session()
            .execute(&protocol::GetCookies { urls: Vec::new() })
            .await
            .unwrap_or_else(|error| panic!("CEF failed to retrieve WebView cookies: {error}"));
        response.cookies.iter().map(cookie_from_cdp).collect()
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn run_javascript(&self, script: &str) -> Result<Str, Str> {
        self.evaluate(script).await
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn call_async_javascript(&self, body: &str) -> Result<Str, Str> {
        // `body` is a function body, so it cannot be evaluated as an
        // expression; wrapping it in an async IIFE gives CDP the promise it
        // awaits.
        let expression = format!("(async () => {{ {body} }})()");
        self.evaluate(&expression).await
    }
}

impl CefWebViewHandle {
    /// Evaluates `expression` in the main frame, awaiting a promise result.
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
                context_id: None,
            })
            .await
            .map_err(|error| Str::from(error.to_string()))?;
        if let Some(exception) = response.exception_details {
            return Err(Str::from(exception.text));
        }
        if let Some(value) = response.result.value {
            return Ok(Str::from(match value {
                Value::String(value) => value,
                value => value.to_string(),
            }));
        }
        Ok(Str::from(response.result.description.unwrap_or_default()))
    }
}

impl CustomWebViewController for CefController {
    fn open(&self) -> impl WebViewHandle {
        CefWebViewHandle::new(self.open_page(CefPageConfiguration::default(), CefPageMode::Visible))
    }
}

/// Records the origin of a newly created execution context.
fn track_context(contexts: &RefCell<HashMap<i64, String>>, params: &Value) {
    let Some(context) = params.get("context") else {
        return;
    };
    let Some(id) = context.get("id").and_then(Value::as_i64) else {
        return;
    };
    let origin = context
        .get("origin")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    contexts.borrow_mut().insert(id, origin);
}

/// Reads an execution-context id out of an event's parameters.
fn context_id(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(Value::as_i64)
}

/// Whether the frame a bridge call came from may use the bridge.
///
/// CEF installs the binding in every execution context, so this is the gate,
/// and it has to be answered about the *calling* frame. Reading the top
/// document's URL instead — which is what this used to do — meant a
/// cross-origin iframe inside an allowed page passed the check and could call
/// every registered handler.
fn frame_may_use_bridge(
    contexts: &RefCell<HashMap<i64, String>>,
    origins: &RefCell<Option<waterui_webview::OriginPolicy>>,
    context: Option<i64>,
) -> bool {
    let Some(policy) = origins.borrow().clone() else {
        // No policy installed yet means no handler has been registered either.
        return false;
    };
    // A call from a context we never saw created cannot be authenticated.
    let Some(context) = context else {
        return false;
    };
    let origin = contexts.borrow().get(&context).cloned();
    origin.is_some_and(|origin| policy.allows_origin(&origin))
}

fn dispatch_bridge_call(
    session: &CefCdpSession,
    handlers: &RefCell<HashMap<String, Rc<MessageHandler>>>,
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
                context_id: context,
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
        // Back into the context that called, not the default one: the pending
        // promise lives in the caller's frame, so a reply sent anywhere else
        // leaves it pending forever.
        execute_without_result(
            &session,
            &protocol::Evaluate {
                expression: &reply.resolve_script(request.id),
                await_promise: false,
                return_by_value: false,
                context_id: context,
            },
        );
    })
    .detach();
}

fn install_bridge(session: &CefCdpSession) {
    execute_without_result(session, &protocol::RuntimeEnable {});
    // Both enables come before the things that need them: Chromium only runs
    // document-start scripts while the page domain is enabled.
    execute_without_result(session, &protocol::PageEnable {});
    execute_without_result(
        session,
        &protocol::AddBinding {
            name: bridge::SEND_FUNCTION,
        },
    );
    execute_without_result(
        session,
        &protocol::AddScriptToEvaluateOnNewDocument {
            source: waterui_webview::DOCUMENT_START_SCRIPT,
        },
    );
}

/// Issues a command whose result nobody waits for.
///
/// A failure is still reported: `CefCdpSession` logs any error whose receiver
/// was dropped, so these do not vanish the way they used to.
fn execute_without_result<C: protocol::CdpCommand>(session: &CefCdpSession, command: &C) {
    drop(session.execute(command));
}

fn cookie_from_cdp(cookie: &protocol::Cookie) -> Cookie<'static> {
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
    if cookie.expires.abs() > f64::EPSILON
        && cookie.expires.is_sign_positive()
        && let Some(seconds) = cookie.expires.to_i64()
        && let Ok(expires) = OffsetDateTime::from_unix_timestamp(seconds)
    {
        builder = builder.expires(expires);
    }
    builder.build()
}
