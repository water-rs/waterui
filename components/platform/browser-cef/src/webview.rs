use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cef::{ImplBrowser, ImplBrowserHost, ImplFrame};
use cookie::{Expiration, SameSite, time::OffsetDateTime};
use num_traits::ToPrimitive as _;
use serde_json::{Value, json};
use waterui_core::{Computed, Signal};
use waterui_str::Str;
use waterui_webview::{
    Cookie, CustomWebViewController, ScriptInjectionTime, WebViewEvent, WebViewHandle,
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
                let payload = event
                    .params
                    .get("payload")
                    .and_then(Value::as_str)
                    .expect("CEF Runtime.bindingCalled must contain a string payload");
                let payload: Value = serde_json::from_str(payload)
                    .unwrap_or_else(|error| panic!("invalid WaterUI bridge payload: {error}"));
                let id = payload
                    .get("id")
                    .and_then(Value::as_u64)
                    .expect("WaterUI bridge payload must contain an integer id");
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .expect("WaterUI bridge payload must contain a handler name");
                let data = payload
                    .get("data")
                    .and_then(Value::as_array)
                    .expect("WaterUI bridge payload must contain a byte array")
                    .iter()
                    .map(|value| {
                        let byte = value
                            .as_u64()
                            .expect("WaterUI bridge data must contain unsigned bytes");
                        u8::try_from(byte).expect("WaterUI bridge byte exceeds u8")
                    })
                    .collect::<Vec<_>>();
                let response =
                    handlers.borrow().get(name).unwrap_or_else(|| {
                        panic!("CEF WebView handler `{name}` is not registered")
                    })(&data);
                let expression = format!(
                    "globalThis.__wateruiResolve({id}, {})",
                    serde_json::to_string(&response)
                        .expect("WaterUI bridge response bytes must serialize")
                );
                execute_without_result(
                    &session,
                    "Runtime.evaluate",
                    &json!({"expression": expression}),
                );
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

    fn go_to(&self, url: &str) {
        let url: waterui_url::Url = url
            .parse()
            .unwrap_or_else(|error| panic!("CEF WebView received an invalid URL: {error}"));
        self.page.navigate(&url);
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
        let previous = self
            .handlers
            .borrow_mut()
            .insert(name.to_string(), Rc::from(handler));
        assert!(
            previous.is_none(),
            "CEF WebView handler `{name}` is already registered"
        );
    }

    fn remove_handler(&self, name: &str) {
        assert!(
            self.handlers.borrow_mut().remove(name).is_some(),
            "CEF WebView handler `{name}` is not registered"
        );
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

    fn watch(&self, watcher: impl Fn(WebViewEvent) + 'static) {
        self.page.watch_webview(watcher);
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
