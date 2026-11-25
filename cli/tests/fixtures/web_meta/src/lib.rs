//! Fixture for `artifact_symbols`: a real `include_web!` invocation whose
//! `waterui_meta_bundle_web` static the CLI must find in the built rlib.

use waterui::include_web;
use waterui::webview::WebViewOpen;

/// Never called — the expansion only has to compile; evaluating it would go
/// looking for a staged bundle this test process does not have.
pub fn web_view() -> WebViewOpen {
    include_web!("web")
}
