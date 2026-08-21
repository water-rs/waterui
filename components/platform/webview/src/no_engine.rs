//! The web view a build with no web engine gets.
//!
//! `WaterUI` runs on targets that have no web engine at all — a Hydrolysis or Dew
//! build compiled without a `webview-*` feature, an embedded target, a headless
//! CI renderer. [`WebView`](crate::WebView) already documents what should happen
//! there: the component keeps its layout slot and its accessibility node, and
//! only the page content is absent, because a missing engine is a missing
//! feature rather than a crash.
//!
//! That promise had nothing behind it. Every [`WebView`] comes from a
//! [`WebViewController`], every controller comes from an engine, so a build
//! without an engine could not construct the contentless view its own
//! documentation described — it panicked in
//! [`WebViewOpen::body`](crate::WebViewOpen) instead, and the contentless
//! rendering path was unreachable code.
//!
//! [`WebViewController::without_engine`] is that missing piece: a real
//! controller whose web views accept every configuration a page-backed one does,
//! report an empty history, and load nothing. It is installed deliberately, by
//! an application or a test that knows this build has no engine; nothing
//! installs it as a fallback, so a build that *expects* an engine and lacks one
//! still fails loudly at the point the web view is created.

use std::future::{Future, ready};

use cookie::Cookie;
use waterui_core::Signal;
use waterui_str::Str;
use waterui_url::Url;

use crate::{
    BackendEvent, CustomWebViewController, OriginPolicy, ScriptInjectionTime, ScriptMessageHandler,
    WatcherGuard, WatcherSet, WebViewHandle,
};

/// Opens web views that have no engine behind them.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoEngineController;

impl CustomWebViewController for NoEngineController {
    fn open(&self) -> impl WebViewHandle {
        NoEngineHandle {
            watchers: WatcherSet::new(),
        }
    }
}

/// A web view with nothing to render into.
///
/// Configuration is accepted and discarded: there is no page to inject a script
/// into, no bridge for a handler to be reachable from, and no navigation to
/// perform. The history is permanently empty, so `can_go_back` and
/// `can_go_forward` are both `false` and no event is ever emitted — an empty
/// history is the truth here, not a placeholder for one.
#[derive(Debug, Clone)]
struct NoEngineHandle {
    /// Kept so watcher guards register and unregister normally. Nothing emits
    /// into it, which is the point: a view that never loads has nothing to
    /// report.
    watchers: WatcherSet<BackendEvent>,
}

impl WebViewHandle for NoEngineHandle {
    fn go_back(&self) {}

    fn go_forward(&self) {}

    fn go_to(&self, _url: &Url) {}

    fn stop(&self) {}

    fn refresh(&self) {}

    fn set_user_agent(&self, _user_agent: &str) {}

    fn can_go_back(&self) -> bool {
        false
    }

    fn can_go_forward(&self) -> bool {
        false
    }

    fn inject_script(&self, _script: &str, _time: ScriptInjectionTime) {}

    fn add_handler(&self, _name: &str, _handler: Box<ScriptMessageHandler>) {}

    fn remove_handler(&self, _name: &str) {}

    fn set_bridge_origins(&self, _policy: OriginPolicy) {}

    fn set_cookie(&self, _cookie: Cookie<'static>) {}

    fn set_redirects_enabled(&self, _enabled: impl Signal<Output = bool>) {}

    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.watchers.insert(f)
    }

    fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> {
        ready(Vec::new())
    }

    /// Fails rather than answering, because there is no page to answer for.
    ///
    /// The configuration methods above can be discarded truthfully — a script
    /// injected into no page simply never runs — but a script *result* has no
    /// truthful empty value, and inventing one would make a caller believe it
    /// had run.
    #[expect(
        clippy::future_not_send,
        reason = "a web view and its handle are main-thread-affine"
    )]
    fn run_javascript(&self, _script: &str) -> impl Future<Output = Result<Str, Str>> {
        ready(Err(Str::from_static(
            "this build has no web engine, so there is no page to run JavaScript in",
        )))
    }
}
