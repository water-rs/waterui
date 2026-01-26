use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_webview::WebView;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::webview::GtkWebViewHandle;

impl GtkComponent for Native<WebView> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let webview = self.into_inner();

        let Some(handle) = webview.handle().downcast_ref::<GtkWebViewHandle>() else {
            return gtk4::Label::new(Some("WebView handle mismatch")).upcast();
        };

        let widget = handle.widget();
        widget.unparent();
        widget
    }
}

