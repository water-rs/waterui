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
    /// CDP binding is installed process-wide and any page can reach it.
    origins: Rc<RefCell<Option<waterui_webview::OriginPolicy>>>,
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
        let session = page.cdp();
        install_bridge(&session);
        session.watch_events({
            let handlers = Rc::clone(&handlers);
            let origins = Rc::clone(&origins);
            let session = session.clone();
            let page = page.clone();
            move |event| {
                if event.method != "Runtime.bindingCalled" {
                    return;
                }
                if !document_may_use_bridge(&page, &origins) {
                    tracing::warn!("a document outside the bridge origin policy tried to call a WaterUI handler");
                    return;
                }
                dispatch_bridge_call(&session, &handlers, &event.params);
            }
        });
        Self {
            page,
            handlers,
            origins,
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

    fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        let source = match time {
            ScriptInjectionTime::DocumentStart => script.to_string(),
            ScriptInjectionTime::DocumentEnd => format!(
                "globalThis.addEventListener('DOMContentLoaded',()=>globalThis.eval({}),{{once:true}});",
                serde_json::to_string(script).expect("WebView script must serialize")
            ),
        };
        execute_without_result(
            &self.session(),
            &protocol::AddScriptToEvaluateOnNewDocument { source: &source },
        );
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
        let response = self
            .session()
            .execute(&protocol::Evaluate {
                expression: script,
                await_promise: true,
                return_by_value: true,
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

/// Whether the document currently loaded may use the bridge.
///
/// CEF installs the binding for the whole browser, so this is the gate: a page
/// the view navigated to is not automatically entitled to the handlers the
/// application registered.
fn document_may_use_bridge(
    page: &CefPageHandle,
    origins: &RefCell<Option<waterui_webview::OriginPolicy>>,
) -> bool {
    let Some(policy) = origins.borrow().clone() else {
        // No policy installed yet means no handler has been registered either.
        return false;
    };
    let Some(browser) = page.host().browser() else {
        return false;
    };
    let Some(frame) = browser.main_frame() else {
        return false;
    };
    let url = cef::CefString::from(&frame.url()).to_string();
    url.parse().is_ok_and(|url| policy.allows(&url))
}

fn dispatch_bridge_call(
    session: &CefCdpSession,
    handlers: &RefCell<HashMap<String, Rc<MessageHandler>>>,
    params: &Value,
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
            },
        );
    })
    .detach();
}

fn install_bridge(session: &CefCdpSession) {
    execute_without_result(session, &protocol::RuntimeEnable {});
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
