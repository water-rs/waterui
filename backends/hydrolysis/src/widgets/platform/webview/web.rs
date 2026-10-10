//! The `WebView` leaf on the web: an `<iframe>`.
//!
//! A browser embeds another document in an `<iframe>`, and that is the web's
//! own web view. The element is hosted content: the engine places it on a DOM
//! plane at the leaf's laid-out frame, clip and stacking order.
//!
//! The embedder of a cross-origin document can navigate it and learn when a
//! load finished, and nothing more: the page's script, cookies, user agent,
//! history and loading state are the other origin's. So the handle implements
//! navigation and the load event, and `waterui-webview` compiles none of the
//! other capabilities for the web. A site that refuses to be framed shows the
//! browser's own refusal in the frame; the embedder is not told.

use std::cell::RefCell;
use std::rc::Rc;

use nami::{Binding, Computed, Signal};
use wasm_bindgen::{JsCast, closure::Closure};
use waterui_core::Environment;
use waterui_webview::{
    BackendEvent, CustomWebViewController, Url, WatcherGuard, WatcherSet, WebViewConfig,
    WebViewController, WebViewEvent, WebViewHandle,
};
use web_sys::{HtmlElement, HtmlIFrameElement};

use crate::{HostedContent, HostedObject, HostedOcclusion};

/// The controller behind this backend's web views on the web. The backend
/// installs it as the default controller.
#[derive(Debug, Clone, Copy)]
pub struct WebSystemWebViewController;

impl CustomWebViewController for WebSystemWebViewController {
    fn open(&self, _config: WebViewConfig) -> impl WebViewHandle {
        WebIframeHandle::new()
    }
}

/// Installs the `<iframe>` controller, unless the application installed one of
/// its own.
pub fn install(env: &mut Environment) {
    if env.get::<WebViewController>().is_some() {
        return;
    }
    env.insert(WebViewController::new(WebSystemWebViewController));
}

/// A web view's `<iframe>`, its watchers and its focus state.
#[derive(Clone)]
pub struct WebIframeHandle {
    inner: Rc<Inner>,
}

struct Inner {
    iframe: HtmlIFrameElement,
    watchers: WatcherSet<BackendEvent>,
    focused: Binding<bool>,
    listeners: RefCell<Vec<Closure<dyn FnMut(web_sys::Event)>>>,
}

impl std::fmt::Debug for WebIframeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebIframeHandle")
            .field("src", &self.inner.iframe.src())
            .finish_non_exhaustive()
    }
}

impl WebIframeHandle {
    fn new() -> Self {
        let iframe: HtmlIFrameElement = web_sys::window()
            .and_then(|window| window.document())
            .expect("hydrolysis web view: the page has no document")
            .create_element("iframe")
            .expect("hydrolysis web view: failed to create an <iframe> element")
            .unchecked_into();
        iframe
            .style()
            .set_property("border", "none")
            .expect("hydrolysis web view: failed to style the <iframe>");
        let handle = Self {
            inner: Rc::new(Inner {
                iframe,
                watchers: WatcherSet::new(),
                focused: nami::binding(false),
                listeners: RefCell::new(Vec::new()),
            }),
        };
        handle.listen();
        handle
    }

    /// The frame's load event, and focus moving into and out of the frame.
    fn listen(&self) {
        let weak = Rc::downgrade(&self.inner);
        let handlers: [(&str, fn(&Inner)); 3] = [
            ("load", |inner| {
                inner
                    .watchers
                    .emit(&BackendEvent::Event(WebViewEvent::Loaded));
            }),
            ("focus", |inner| inner.focused.set(true)),
            ("blur", |inner| inner.focused.set(false)),
        ];
        let mut listeners = self.inner.listeners.borrow_mut();
        for (name, handler) in handlers {
            let weak = weak.clone();
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event| {
                if let Some(inner) = weak.upgrade() {
                    handler(&inner);
                }
            });
            self.inner
                .iframe
                .add_event_listener_with_callback(name, closure.as_ref().unchecked_ref())
                .unwrap_or_else(|_| panic!("hydrolysis web view: failed to listen for {name}"));
            listeners.push(closure);
        }
    }
}

impl WebViewHandle for WebIframeHandle {
    fn go_to(&self, url: &Url) {
        self.inner
            .watchers
            .emit(&BackendEvent::Event(WebViewEvent::WillNavigate {
                url: url.clone(),
            }));
        self.inner.iframe.set_src(url.as_str());
    }

    fn watch(&self, watcher: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.inner.watchers.insert(watcher)
    }
}

impl HostedContent for WebIframeHandle {
    fn mount(&self, _occlusion: HostedOcclusion) -> HostedObject {
        let element: HtmlElement = self.inner.iframe.clone().unchecked_into();
        cherenkov_gpu::interop::web::HostedElement::new(element)
    }

    /// Stops the page: the frame leaves the page with its binding, and a
    /// blank document releases whatever the page was doing until then.
    fn unmount(&self) {
        self.inner.iframe.set_src("about:blank");
    }

    fn focused(&self) -> Computed<bool> {
        self.inner.focused.clone().into()
    }

    fn request_focus(&self) {
        let _ = self.inner.iframe.focus();
    }
}
