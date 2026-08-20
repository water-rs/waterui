//! Renders `WebView` through whichever browser engine this build selected.

use gtk4::Widget;
#[cfg(feature = "webview-system")]
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_webview::WebView;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::webview::GtkWebViewHandle;

impl GtkComponent for Native<WebView> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let webview = self.into_inner();

        let Some(handle) = webview.handle().downcast_ref::<GtkWebViewHandle>() else {
            panic!("WebView handle type mismatch in GTK backend (fast-fail)");
        };

        #[cfg(feature = "webview-system")]
        {
            let _ = env;
            let widget = handle.widget();
            widget.unparent();
            widget
        }
        #[cfg(any(feature = "webview-wpe", feature = "webview-cef"))]
        {
            crate::webview::render_webview(handle, env)
        }
    }
}
