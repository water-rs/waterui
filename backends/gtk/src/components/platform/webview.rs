//! Renders `WebView` through the platform's own `WebKitGTK` engine.
//!
//! Only the bridged system engine is handled here. A browser engine the
//! application links installs its own realization, and that draws through the
//! generic `GpuSurface` path instead of ever reaching a `Native<WebView>`.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_webview::WebView;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::webview::GtkWebViewHandle;

impl GtkComponent for Native<WebView> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let webview = self.into_inner();

        let Some(handle) = webview.handle().downcast_ref::<GtkWebViewHandle>() else {
            panic!("WebView handle type mismatch in GTK backend (fast-fail)");
        };

        let widget = handle.widget();
        widget.unparent();
        widget
    }
}
