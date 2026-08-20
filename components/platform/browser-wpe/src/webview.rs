use std::cell::RefCell;
use std::rc::Rc;

use cookie::{Cookie, SameSite, time::OffsetDateTime};
use nami::watcher::BoxWatcherGuard;
use serde::Deserialize;
use waterui_core::{Computed, Signal};
use waterui_url::Url;
use waterui_webview::{CustomWebViewController, ScriptInjectionTime, WatcherGuard, WebViewHandle};

use crate::{WpePage, WpeRuntime, WpeRuntimePaths};

/// Where a controller's runtime comes from.
///
/// A backend installs its controller into the environment while building the
/// application, long before it knows whether the tree contains a `WebView` at
/// all. Initializing the staged runtime there would make every application on
/// the platform — and every test binary whose features happen to enable this
/// engine — require a `water`-staged WPE install just to start. So the packaged
/// runtime is named now and loaded on first use.
#[derive(Debug)]
enum RuntimeSource {
    /// A runtime the caller already initialized.
    Ready(WpeRuntime),
    /// The runtime staged next to the executable, loaded when first needed.
    Packaged {
        paths: WpeRuntimePaths,
        runtime: RefCell<Option<WpeRuntime>>,
    },
}

/// Environment controller for standard `WebViews` backed by bundled WPE `WebKit`.
#[derive(Debug, Clone)]
pub struct WpeController {
    // Shared so every clone of the controller resolves the same runtime once.
    source: Rc<RuntimeSource>,
}

impl WpeController {
    /// Creates a controller from an initialized runtime.
    #[must_use]
    pub fn new(runtime: WpeRuntime) -> Self {
        Self {
            source: Rc::new(RuntimeSource::Ready(runtime)),
        }
    }

    /// Names the runtime staged by `water run` or `water package`.
    ///
    /// The staged install is not touched until the first web view is opened;
    /// see [`Self::runtime`] for what happens when it is missing.
    #[must_use]
    pub fn packaged() -> Self {
        Self {
            source: Rc::new(RuntimeSource::Packaged {
                paths: WpeRuntimePaths::packaged(),
                runtime: RefCell::new(None),
            }),
        }
    }

    /// The initialized runtime, loading the staged one on first call.
    ///
    /// # Panics
    ///
    /// Panics when the staged runtime is missing, invalid, or ABI-incompatible.
    /// That fast-fail is unchanged and just as loud as before — it now fires
    /// when an application actually opens a web view rather than when the
    /// backend installs the controller, so the requirement lands on the code
    /// that needs it instead of on every application built with this engine.
    fn runtime(&self) -> WpeRuntime {
        match self.source.as_ref() {
            RuntimeSource::Ready(runtime) => runtime.clone(),
            RuntimeSource::Packaged { paths, runtime } => runtime
                .borrow_mut()
                .get_or_insert_with(|| WpeRuntime::initialize(paths))
                .clone(),
        }
    }

    /// Dispatches all currently-ready WPE tasks.
    ///
    /// A controller whose runtime has not been loaded has no pages and so no
    /// work pending; pumping does not force the staged install to load.
    pub fn pump(&self) {
        let runtime = match self.source.as_ref() {
            RuntimeSource::Ready(runtime) => runtime.clone(),
            RuntimeSource::Packaged { runtime, .. } => {
                let Some(runtime) = runtime.borrow().clone() else {
                    return;
                };
                runtime
            }
        };
        while runtime.iteration() {}
    }
}

impl CustomWebViewController for WpeController {
    fn open(&self) -> impl WebViewHandle {
        WpeWebViewHandle::new(WpePage::new(self.runtime()))
    }
}

/// Adapts the shared bridge's one-function transport onto WPE's message handler.
const TRANSPORT_SCRIPT: &str = concat!(
    "globalThis.__wateruiSend = function (envelope) {",
    "globalThis.webkit.messageHandlers.__waterui.postMessage(envelope);",
    "};"
);

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
        // Transport first: the shared script calls `__wateruiSend`, so the adapter
        // onto WPE's single WebKit message handler has to exist before it runs.
        page.add_script(TRANSPORT_SCRIPT, false);
        page.add_script(waterui_webview::DOCUMENT_START_SCRIPT, false);
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

    fn add_handler(&self, name: &str, handler: Box<waterui_webview::ScriptMessageHandler>) {
        self.page.add_handler(name, handler);
    }

    fn set_bridge_origins(&self, policy: waterui_webview::OriginPolicy) {
        self.page.set_bridge_origins(policy);
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

    fn watch(&self, watcher: impl Fn(waterui_webview::BackendEvent) + 'static) -> WatcherGuard {
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
