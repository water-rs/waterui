use std::cell::RefCell;
use std::rc::Rc;

use cookie::{Cookie, SameSite, time::OffsetDateTime};
use nami::watcher::BoxWatcherGuard;
use serde::Deserialize;
use waterui_core::{Computed, Signal};
use waterui_url::Url;
use waterui_webview::{
    CustomWebViewController, ScriptInjectionTime, WatcherGuard, WebViewEvent, WebViewHandle,
};

use crate::{WpePage, WpeRuntime, WpeRuntimePaths};

/// Environment controller for standard `WebViews` backed by bundled WPE `WebKit`.
#[derive(Debug, Clone)]
pub struct WpeController {
    runtime: WpeRuntime,
}

impl WpeController {
    /// Creates a controller from an initialized runtime.
    #[must_use]
    pub const fn new(runtime: WpeRuntime) -> Self {
        Self { runtime }
    }

    /// Loads the runtime staged by `water run` or `water package`.
    #[must_use]
    pub fn packaged() -> Self {
        let paths = WpeRuntimePaths::packaged();
        Self::new(WpeRuntime::initialize(&paths))
    }

    /// Dispatches all currently-ready WPE tasks.
    pub fn pump(&self) {
        while self.runtime.iteration() {}
    }
}

impl CustomWebViewController for WpeController {
    fn open(&self) -> impl WebViewHandle {
        WpeWebViewHandle::new(WpePage::new(self.runtime.clone()))
    }
}

type RedirectSubscription = Option<(Computed<bool>, BoxWatcherGuard)>;

/// Standard `WaterUI` `WebView` handle backed by one WPE page.
#[derive(Clone)]
pub struct WpeWebViewHandle {
    page: WpePage,
    redirects: Rc<RefCell<RedirectSubscription>>,
}

impl core::fmt::Debug for WpeWebViewHandle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WpeWebViewHandle")
            .finish_non_exhaustive()
    }
}

impl WpeWebViewHandle {
    /// Creates a standard handle around `page`.
    #[must_use]
    pub fn new(page: WpePage) -> Self {
        page.add_script(include_str!("bridge.js"), false);
        Self {
            page,
            redirects: Rc::new(RefCell::new(None)),
        }
    }

    /// Returns the retained page for renderer integration.
    #[must_use]
    pub const fn page(&self) -> &WpePage {
        &self.page
    }
}

impl WebViewHandle for WpeWebViewHandle {
    fn go_back(&self) {
        self.page.go_back();
    }

    fn go_forward(&self) {
        self.page.go_forward();
    }

    fn go_to(&self, url: &Url) {
        self.page.load_uri(url.as_str());
    }

    fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        self.page
            .add_script(script, time == ScriptInjectionTime::DocumentEnd);
    }

    fn add_handler(&self, name: &str, handler: Box<dyn Fn(&[u8]) -> Vec<u8> + 'static>) {
        self.page.add_handler(name, handler);
    }

    fn remove_handler(&self, name: &str) {
        self.page.remove_handler(name);
    }

    fn stop(&self) {
        self.page.stop();
    }

    fn refresh(&self) {
        self.page.reload();
    }

    fn set_user_agent(&self, user_agent: &str) {
        self.page.set_user_agent(user_agent);
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        let enabled = Computed::new(enabled);
        self.page.set_redirects_enabled(enabled.get());
        let page = self.page.clone();
        let guard = enabled.watch(move |context| {
            page.set_redirects_enabled(context.into_value());
        });
        self.redirects.replace(Some((enabled, guard)));
    }

    fn watch(&self, watcher: impl Fn(WebViewEvent) + 'static) -> WatcherGuard {
        self.page.watch(watcher)
    }

    fn can_go_back(&self) -> bool {
        self.page.can_go_back()
    }

    fn can_go_forward(&self) -> bool {
        self.page.can_go_forward()
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        self.page.set_cookie(&cookie.to_string());
    }

    #[expect(
        clippy::future_not_send,
        reason = "WPE WebKit and WaterUI view state are confined to the UI thread"
    )]
    async fn get_cookies(&self) -> Vec<Cookie<'static>> {
        let records: Vec<CookieRecord> = serde_json::from_str(&self.page.cookies_json().await)
            .unwrap_or_else(|error| panic!("WPE returned invalid cookie JSON: {error}"));
        records.into_iter().map(CookieRecord::into_cookie).collect()
    }

    #[expect(
        clippy::future_not_send,
        reason = "WPE WebKit and WaterUI view state are confined to the UI thread"
    )]
    async fn run_javascript(&self, script: &str) -> Result<waterui_str::Str, waterui_str::Str> {
        self.page.run_javascript(script).await
    }
}

#[derive(Debug, Deserialize)]
struct CookieRecord {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    http_only: bool,
    same_site: Option<String>,
    expires: Option<i64>,
}

impl CookieRecord {
    fn into_cookie(self) -> Cookie<'static> {
        let mut builder = Cookie::build((self.name, self.value))
            .domain(self.domain)
            .path(self.path)
            .secure(self.secure)
            .http_only(self.http_only);
        if let Some(same_site) = self.same_site {
            builder = builder.same_site(match same_site.as_str() {
                "Strict" => SameSite::Strict,
                "Lax" => SameSite::Lax,
                "None" => SameSite::None,
                other => panic!("WPE returned unsupported cookie SameSite value `{other}`"),
            });
        }
        if let Some(expires) = self.expires {
            builder = builder.expires(
                OffsetDateTime::from_unix_timestamp(expires)
                    .expect("WPE cookie expiration exceeds OffsetDateTime"),
            );
        }
        builder.build()
    }
}
