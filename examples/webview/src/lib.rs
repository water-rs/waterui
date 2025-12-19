//! WebView Example - Demonstrates WaterUI's WebView component
//!
//! This example showcases:
//! - Opening a WebView to display web content
//! - Navigation to a URL

use waterui::app::App;
use waterui::prelude::*;
use waterui::webview::WebView;

fn main() -> impl View {
    WebView::open("https://waterui.dev")
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
