//! The handle of a web view its engine could not open.
//!
//! A controller that fails to open a view reports a [`WebViewOpenError`]; the
//! facade then keeps the application running with this handle in its place.
//! Every operation does nothing and every answer is the empty one, so code
//! written against a [`WebView`](crate::WebView) needs no second path, and a
//! backend that mounts the view recognizes it by
//! [`AnyWebViewHandle::is_unavailable`](crate::AnyWebViewHandle::is_unavailable)
//! and draws a placeholder instead of a page.

#[cfg(not(target_arch = "wasm32"))]
use std::future::{Future, ready};

#[cfg(not(target_arch = "wasm32"))]
use cookie::Cookie;
#[cfg(not(target_arch = "wasm32"))]
use suiteki::Str;
#[cfg(not(target_arch = "wasm32"))]
use waterui_core::Signal;
use waterui_url::Url;
use waterui_watcher_set::{WatcherGuard, WatcherSet};

use crate::BackendEvent;
use crate::WebViewHandle;
#[cfg(not(target_arch = "wasm32"))]
use crate::{ScriptInjectionTime, ScriptMessageHandler};

/// Why an engine could not open a web view.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WebViewOpenError {
    /// The platform engine only creates views on the main thread.
    #[error("a web view can only be opened on the main thread")]
    MainThreadRequired,
    /// The engine refused or failed to create the view.
    #[error("the web engine could not create a view: {0}")]
    Engine(String),
}

/// The handle behind a [`WebView`](crate::WebView) that could not be opened.
///
/// No page exists, so navigation, scripts, handlers and cookies are accepted
/// and dropped, history is empty, and evaluation answers an error. The watcher
/// set is kept so guards register and unregister normally; nothing ever emits
/// into it.
#[derive(Debug, Default)]
pub struct UnavailableWebViewHandle {
    watchers: WatcherSet<BackendEvent>,
}

#[cfg(not(target_arch = "wasm32"))]
const UNAVAILABLE: &str = "the web view could not be opened, so it has no page";

impl WebViewHandle for UnavailableWebViewHandle {
    #[cfg(not(target_arch = "wasm32"))]
    fn go_back(&self) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn go_forward(&self) {}

    fn go_to(&self, _url: &Url) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn stop(&self) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn refresh(&self) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn set_user_agent(&self, _user_agent: &str) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn can_go_back(&self) -> bool {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn can_go_forward(&self) -> bool {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn inject_script(&self, _key: &str, _script: &str, _time: ScriptInjectionTime) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn add_handler(&self, _name: &str, _handler: Box<ScriptMessageHandler>) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn remove_handler(&self, _name: &str) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn set_cookie(&self, _cookie: Cookie<'static>) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn set_redirects_enabled(&self, _enabled: impl Signal<Output = bool>) {}

    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.watchers.insert(f)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn get_cookies(
        &self,
    ) -> impl Future<Output = Result<Vec<Cookie<'static>>, waterui_core::Error>> {
        ready(Ok(Vec::new()))
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[expect(
        clippy::future_not_send,
        reason = "a web view and its handle are main-thread-affine"
    )]
    fn run_javascript(&self, _script: &str) -> impl Future<Output = Result<Str, Str>> {
        ready(Err(Str::from_static(UNAVAILABLE)))
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[expect(
        clippy::future_not_send,
        reason = "a web view and its handle are main-thread-affine"
    )]
    fn call_async_javascript(&self, _body: &str) -> impl Future<Output = Result<Str, Str>> {
        ready(Err(Str::from_static(UNAVAILABLE)))
    }
}
