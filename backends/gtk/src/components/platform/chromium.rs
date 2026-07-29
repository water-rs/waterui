use gtk4::Widget;
use waterui_chromium::{ChromiumView, PageMode};
use waterui_core::Environment;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for ChromiumView {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        assert_eq!(
            self.page().mode(),
            PageMode::Visible,
            "headless Chromium pages cannot be rendered as a GTK component"
        );
        let handle = self
            .page()
            .handle()
            .downcast_ref::<waterui_browser_cef::CefPageHandle>()
            .expect("Chromium page handle type mismatch in GTK backend");
        crate::browser_cef::render_page(handle.clone(), env, true)
    }
}
