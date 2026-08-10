use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cef::{ImplBrowser, ImplBrowserHost, ImplFrame};
use cookie::{Expiration, SameSite, time::OffsetDateTime};
use num_traits::ToPrimitive as _;
use serde_json::{Value, json};
use waterui_core::{Computed, Signal};
use waterui_str::Str;
use waterui_url::Url;
use waterui_webview::{
    Cookie, CustomWebViewController, ScriptInjectionTime, WatcherGuard, WebViewEvent,
    WebViewHandle,
};

use crate::cdp::CefCdpSession;
use crate::page::{CefController, CefPageConfiguration, CefPageHandle, CefPageMode};

type MessageHandler = dyn Fn(&[u8]) -> Vec<u8> + 'static;

#[derive(Clone)]
/// Standard `WaterUI` `WebView` handle backed by a CEF page.
pub struct CefWebViewHandle {
    page: CefPageHandle,
    handlers: Rc<RefCell<HashMap<String, Rc<MessageHandler>>>>,
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
        let session = page.cdp();
        install_bridge(&session);
        session.watch_events({
            let handlers = Rc::clone(&handlers);
            let session = session.clone();
            move |event| {
                if event.method != "Runtime.bindingCalled" {
                    return;
                }
                dispatch_bridge_call(&session, &handlers, &event.params);
            }
        });
        Self { page, handlers }
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
            "Page.addScriptToEvaluateOnNewDocument",
            &json!({"source": source}),
        );
    }

    fn add_handler(&self, name: &str, handler: Box<dyn Fn(&[u8]) -> Vec<u8> + 'static>) {
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
            "Network.setUserAgentOverride",
            &json!({"userAgent": user_agent}),
        );
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        self.page.set_redirects_enabled(Computed::new(enabled));
    }

    fn watch(&self, watcher: impl Fn(WebViewEvent) + 'static) -> WatcherGuard {
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
        execute_without_result(
            &self.session(),
            "Network.setCookie",
            &json!({
                "name": cookie.name(),
                "value": cookie.value(),
                "url": current_url,
                "domain": cookie.domain(),
                "path": cookie.path(),
                "secure": cookie.secure(),
                "httpOnly": cookie.http_only(),
                "sameSite": same_site,
                "expires": expires,
            }),
        );
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn get_cookies(&self) -> Vec<Cookie<'static>> {
        let response = self
            .session()
            .execute_raw("Network.getAllCookies", &json!({}))
            .await
            .unwrap_or_else(|error| panic!("CEF failed to retrieve WebView cookies: {error}"));
        response
            .get("cookies")
            .and_then(Value::as_array)
            .expect("CEF cookie response must contain a cookie array")
            .iter()
            .map(cookie_from_cdp)
            .collect()
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    async fn run_javascript(&self, script: &str) -> Result<Str, Str> {
        let response = self
            .session()
            .execute_raw(
                "Runtime.evaluate",
                &json!({
                    "expression": script,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
            )
            .await
            .map_err(|error| Str::from(error.to_string()))?;
        if let Some(exception) = response.get("exceptionDetails") {
            return Err(Str::from(
                exception
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("JavaScript evaluation failed")
                    .to_string(),
            ));
        }
        let result = response.get("result").unwrap_or(&Value::Null);
        if let Some(value) = result.get("value") {
            return Ok(Str::from(match value {
                Value::String(value) => value.clone(),
                value => value.to_string(),
            }));
        }
        Ok(Str::from(
            result
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ))
    }
}

impl CustomWebViewController for CefController {
    fn open(&self) -> impl WebViewHandle {
        CefWebViewHandle::new(self.open_page(CefPageConfiguration::default(), CefPageMode::Visible))
    }
}

/// One `waterui.invoke(...)` request produced by `webview_bridge.js`.
///
/// The binding this arrives on is reachable from ordinary page script, so every
/// field is validated and a malformed request is rejected instead of aborting the
/// host process.
#[derive(Debug, serde::Deserialize)]
struct BridgeCall {
    id: u64,
    name: String,
    data: Vec<u8>,
}

fn dispatch_bridge_call(
    session: &CefCdpSession,
    handlers: &RefCell<HashMap<String, Rc<MessageHandler>>>,
    params: &Value,
) {
    let Some(payload) = params.get("payload").and_then(Value::as_str) else {
        tracing::warn!("CEF bridge binding fired without a string payload; ignoring");
        return;
    };
    let call: BridgeCall = match serde_json::from_str(payload) {
        Ok(call) => call,
        Err(error) => {
            tracing::warn!(%error, "page script sent a malformed WaterUI bridge request");
            return;
        }
    };
    // Resolve the handler and release the borrow before invoking it: a handler is
    // free to register or remove handlers on the same web view.
    let handler = handlers.borrow().get(&call.name).map(Rc::clone);
    let Some(handler) = handler else {
        tracing::warn!(
            handler = %call.name,
            "page script called a WaterUI handler that is not registered"
        );
        reply_to_bridge_call(
            session,
            call.id,
            false,
            &Value::from(format!("no WaterUI handler named `{}`", call.name)),
        );
        return;
    };
    let response = handler(&call.data);
    reply_to_bridge_call(session, call.id, true, &Value::from(response));
}

fn reply_to_bridge_call(session: &CefCdpSession, id: u64, ok: bool, payload: &Value) {
    let expression = format!(
        "globalThis.__wateruiResolve({id},{ok},{})",
        serde_json::to_string(payload).expect("WaterUI bridge reply must serialize")
    );
    execute_without_result(
        session,
        "Runtime.evaluate",
        &json!({ "expression": expression }),
    );
}

fn install_bridge(session: &CefCdpSession) {
    execute_without_result(session, "Runtime.enable", &json!({}));
    execute_without_result(
        session,
        "Runtime.addBinding",
        &json!({"name": "__wateruiInvoke"}),
    );
    execute_without_result(
        session,
        "Page.addScriptToEvaluateOnNewDocument",
        &json!({"source": include_str!("webview_bridge.js")}),
    );
}

fn execute_without_result(session: &CefCdpSession, method: &str, params: &Value) {
    let future = session.execute_raw(method, params);
    drop(future);
}

fn cookie_from_cdp(value: &Value) -> Cookie<'static> {
    let string = |name| {
        value
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("CEF cookie is missing string field `{name}`"))
    };
    let boolean = |name| {
        value
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("CEF cookie is missing boolean field `{name}`"))
    };
    let mut builder = Cookie::build((string("name").to_string(), string("value").to_string()))
        .domain(string("domain").to_string())
        .path(string("path").to_string())
        .secure(boolean("secure"))
        .http_only(boolean("httpOnly"));
    if let Some(same_site) = value.get("sameSite").and_then(Value::as_str) {
        builder = builder.same_site(match same_site {
            "Strict" => SameSite::Strict,
            "Lax" => SameSite::Lax,
            "None" => SameSite::None,
            other => panic!("CEF returned unsupported cookie SameSite value `{other}`"),
        });
    }
    if let Some(expires) = value
        .get("expires")
        .and_then(Value::as_f64)
        .filter(|expires| *expires > 0.0)
    {
        let expires = OffsetDateTime::from_unix_timestamp(
            expires
                .to_i64()
                .expect("CEF cookie expiration does not fit i64"),
        )
        .expect("CEF cookie expiration exceeds OffsetDateTime");
        builder = builder.expires(expires);
    }
    builder.build()
}
