use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::Environment;
use waterui_core::Str;
use waterui_webview::{
    CustomWebViewController, ScriptInjectionTime, WebViewController, WebViewEvent, WebViewHandle,
    cookie::Cookie,
};

#[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
mod webkitgtk {
    use std::ffi::CString;
    use std::os::raw::c_char;
    use std::ptr::NonNull;

    use gtk4::Widget;
    use gtk4::ffi::GtkWidget;
    use gtk4::glib;
    use gtk4::glib::translate::from_glib_full;

    #[repr(C)]
    pub struct WebKitWebView {
        _private: [u8; 0],
    }

    #[link(name = "webkitgtk-6.0")]
    extern "C" {
        fn webkit_web_view_new() -> *mut WebKitWebView;
        fn webkit_web_view_load_uri(web_view: *mut WebKitWebView, uri: *const c_char);
        fn webkit_web_view_go_back(web_view: *mut WebKitWebView);
        fn webkit_web_view_go_forward(web_view: *mut WebKitWebView);
        fn webkit_web_view_stop_loading(web_view: *mut WebKitWebView);
        fn webkit_web_view_reload(web_view: *mut WebKitWebView);
        fn webkit_web_view_can_go_back(web_view: *mut WebKitWebView) -> glib::ffi::gboolean;
        fn webkit_web_view_can_go_forward(web_view: *mut WebKitWebView) -> glib::ffi::gboolean;
    }

    pub(super) struct WebViewParts {
        pub widget: Widget,
        pub ptr: NonNull<WebKitWebView>,
    }

    pub(super) fn create_webview() -> Option<WebViewParts> {
        let ptr = unsafe { webkit_web_view_new() };
        let ptr = NonNull::new(ptr)?;

        // WebKitWebView implements GtkWidget; take ownership of the initial ref.
        let widget = unsafe { from_glib_full(ptr.as_ptr() as *mut GtkWidget) };
        Some(WebViewParts { widget, ptr })
    }

    pub(super) fn load_uri(ptr: NonNull<WebKitWebView>, uri: &str) {
        if let Ok(cstr) = CString::new(uri) {
            unsafe { webkit_web_view_load_uri(ptr.as_ptr(), cstr.as_ptr()) }
        }
    }

    pub(super) fn go_back(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_go_back(ptr.as_ptr()) }
    }

    pub(super) fn go_forward(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_go_forward(ptr.as_ptr()) }
    }

    pub(super) fn stop(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_stop_loading(ptr.as_ptr()) }
    }

    pub(super) fn reload(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_reload(ptr.as_ptr()) }
    }

    pub(super) fn can_go_back(ptr: NonNull<WebKitWebView>) -> bool {
        unsafe { webkit_web_view_can_go_back(ptr.as_ptr()) != 0 }
    }

    pub(super) fn can_go_forward(ptr: NonNull<WebKitWebView>) -> bool {
        unsafe { webkit_web_view_can_go_forward(ptr.as_ptr()) != 0 }
    }
}

pub fn ensure_webview_controller(env: &mut Environment) {
    if env.get::<WebViewController>().is_some() {
        return;
    }
    env.insert(WebViewController::new(GtkWebViewController));
}

#[derive(Debug, Default, Clone)]
pub(crate) struct GtkWebViewController;

impl CustomWebViewController for GtkWebViewController {
    fn open(&self) -> impl WebViewHandle {
        GtkWebViewHandle::new()
    }
}

#[derive(Clone)]
pub(crate) struct GtkWebViewHandle {
    widget: Widget,
    #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
    ptr: Option<std::ptr::NonNull<webkitgtk::WebKitWebView>>,
    watchers: Rc<RefCell<Vec<Rc<dyn Fn(WebViewEvent)>>>>,
    redirects_enabled: Cell<bool>,
}

impl core::fmt::Debug for GtkWebViewHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GtkWebViewHandle")
    }
}

impl GtkWebViewHandle {
    pub(crate) fn new() -> Self {
        let watchers: Rc<RefCell<Vec<Rc<dyn Fn(WebViewEvent)>>>> =
            Rc::new(RefCell::new(Vec::new()));

        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        let (widget, ptr) = match webkitgtk::create_webview() {
            Some(parts) => (parts.widget, Some(parts.ptr)),
            None => (gtk4::Label::new(Some("WebView unavailable")).upcast(), None),
        };

        #[cfg(not(all(feature = "webkitgtk", unix, not(target_os = "macos"))))]
        let (widget, _ptr) = (
            gtk4::Label::new(Some("WebView (enable `webkitgtk`)")).upcast(),
            (),
        );

        let this = Self {
            widget,
            #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
            ptr,
            watchers,
            redirects_enabled: Cell::new(true),
        };

        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        this.install_observers();

        this
    }

    pub(crate) fn widget(&self) -> Widget {
        self.widget.clone()
    }

    #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
    fn install_observers(&self) {
        // Avoid panics if WebKit isn't actually available at runtime.
        if self.widget.find_property("uri").is_none()
            || self
                .widget
                .find_property("estimated-load-progress")
                .is_none()
            || self.widget.find_property("can-go-back").is_none()
            || self.widget.find_property("can-go-forward").is_none()
        {
            return;
        }

        let watchers = self.watchers.clone();
        self.widget.connect_notify(Some("uri"), move |obj, _| {
            let uri = obj.property::<String>("uri");
            if uri.is_empty() {
                return;
            }
            if let Some(url) = waterui_webview::Url::parse(&uri) {
                let snapshot = watchers.borrow().clone();
                for cb in snapshot {
                    cb(WebViewEvent::WillNavigate { url: url.clone() })
                }
            }
        });

        let watchers = self.watchers.clone();
        self.widget
            .connect_notify(Some("estimated-load-progress"), move |obj, _| {
                let progress = obj.property::<f64>("estimated-load-progress") as f32;
                let snapshot = watchers.borrow().clone();
                for cb in snapshot.iter() {
                    cb(WebViewEvent::Loading { progress })
                }
                if progress >= 1.0 {
                    for cb in snapshot {
                        cb(WebViewEvent::Loaded)
                    }
                }
            });

        let watchers = self.watchers.clone();
        self.widget
            .connect_notify(Some("can-go-back"), move |obj, _| {
                let back = obj.property::<bool>("can-go-back");
                let forward = obj.property::<bool>("can-go-forward");
                let snapshot = watchers.borrow().clone();
                for cb in snapshot {
                    cb(WebViewEvent::StateChanged {
                        can_go_back: back,
                        can_go_forward: forward,
                    })
                }
            });

        let watchers = self.watchers.clone();
        self.widget
            .connect_notify(Some("can-go-forward"), move |obj, _| {
                let back = obj.property::<bool>("can-go-back");
                let forward = obj.property::<bool>("can-go-forward");
                let snapshot = watchers.borrow().clone();
                for cb in snapshot {
                    cb(WebViewEvent::StateChanged {
                        can_go_back: back,
                        can_go_forward: forward,
                    })
                }
            });
    }
}

impl WebViewHandle for GtkWebViewHandle {
    fn go_back(&self) {
        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        if let Some(ptr) = self.ptr {
            webkitgtk::go_back(ptr);
        }
    }

    fn go_forward(&self) {
        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        if let Some(ptr) = self.ptr {
            webkitgtk::go_forward(ptr);
        }
    }

    fn go_to(&self, _url: &str) {
        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        if let Some(ptr) = self.ptr {
            webkitgtk::load_uri(ptr, _url);
        }
    }

    fn inject_script(&self, _script: &str, _time: ScriptInjectionTime) {
        tracing::warn!("inject_script is not yet implemented for GTK WebView");
    }

    fn add_handler(&self, _name: &str, _handler: Box<dyn Fn(&[u8]) -> Vec<u8> + 'static>) {
        tracing::warn!("add_handler is not yet implemented for GTK WebView");
    }

    fn remove_handler(&self, _name: &str) {
        tracing::warn!("remove_handler is not yet implemented for GTK WebView");
    }

    fn stop(&self) {
        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        if let Some(ptr) = self.ptr {
            webkitgtk::stop(ptr);
        }
    }

    fn refresh(&self) {
        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        if let Some(ptr) = self.ptr {
            webkitgtk::reload(ptr);
        }
    }

    fn set_user_agent(&self, _user_agent: &str) {
        tracing::warn!("set_user_agent is not yet implemented for GTK WebView");
    }

    fn set_redirects_enabled(&self, enabled: bool) {
        self.redirects_enabled.set(enabled);
    }

    fn watch(&self, f: impl Fn(WebViewEvent) + 'static) {
        self.watchers.borrow_mut().push(Rc::new(f));
    }

    fn can_go_back(&self) -> bool {
        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        if let Some(ptr) = self.ptr {
            return webkitgtk::can_go_back(ptr);
        }
        false
    }

    fn can_go_forward(&self) -> bool {
        #[cfg(all(feature = "webkitgtk", unix, not(target_os = "macos")))]
        if let Some(ptr) = self.ptr {
            return webkitgtk::can_go_forward(ptr);
        }
        false
    }

    fn set_cookie(&self, _cookie: Cookie<'static>) {
        tracing::warn!("set_cookie is not yet implemented for GTK WebView");
    }

    fn get_cookies(&self) -> Vec<Cookie<'static>> {
        tracing::warn!("get_cookies is not yet implemented for GTK WebView");
        Vec::new()
    }

    async fn run_javascript(&self, _script: &str) -> Result<Str, Str> {
        Err(Str::from_static(
            "run_javascript is not yet implemented for GTK WebView",
        ))
    }
}
