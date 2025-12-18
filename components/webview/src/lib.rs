mod controller;
pub use controller::*;
mod handler;
pub use handler::*;
use std::{pin::Pin, rc::Rc};
use waterui_core::reactive::CustomBinding;

use waterui_core::{
    Computed, Signal, Str, View,
    binding::Container,
    configurable,
    env::use_env,
    impl_debug, impl_extractor,
    layout::StretchAxis,
    reactive::watcher::{BoxWatcherGuard, WatcherGuard},
};

use crate::controller::WebViewController;

#[derive(Debug, Clone)]
pub enum WebViewEvent {
    None,
    WillNavigate { url: Str },
    Loading { progress: f32 },
    Loaded,
    Error { code: i32, message: String },
}

#[derive(Clone, Debug)]
pub struct WebView {
    event: Container<WebViewEvent>,
    handle: AnyWebViewHandle,
    can_go_back: Computed<bool>,
    can_go_forward: Computed<bool>,
}

impl WebView {
    pub fn open(f: impl FnOnce(Self) + 'static) -> impl View {
        use_env(|controller: WebViewController| {
            let handler = controller.open();

            let webview = Self::new(handler);

            f(webview);
        })
    }
    pub fn new(handle: AnyWebViewHandle) -> Self {
        // For demonstration purposes, we'll use dummy computed values.
        let can_go_back = Computed::constant(true);
        let can_go_forward = Computed::constant(true);
        let container = Container::new(WebViewEvent::None);

        handle.watch({
            let container = container.clone();
            move |event| {
                container.set(event);
            }
        });

        Self {
            handle,
            can_go_back,
            can_go_forward,
            event: container,
        }
    }

    /// Creates a new WebView component with the given handle.
    pub fn refresh(&self) {
        self.handle.refresh();
    }

    /// Stops the current loading operation.
    pub fn stop(&self) {
        self.handle.stop();
    }

    /// Navigates back in the web view's history.
    pub fn go_back(&self) {
        self.handle.go_back();
    }

    /// Navigates forward in the web view's history.
    pub fn go_forward(&self) {
        self.handle.go_forward();
    }

    /// Runs the given JavaScript code in the web view and returns the result.
    pub async fn run_javascript(&self, script: &str) -> Result<String, String> {
        self.handle.run_javascript(script).await
    }

    /// Checks if the web view can navigate back.
    pub fn can_go_back(&self) -> Computed<bool> {
        self.can_go_back.clone()
    }

    /// Checks if the web view can navigate forward.
    pub fn handle(&self) -> AnyWebViewHandle {
        self.handle.clone()
    }

    /// Checks if the web view can navigate forward.
    pub fn can_go_forward(&self) -> Computed<bool> {
        self.can_go_forward.clone()
    }
}
