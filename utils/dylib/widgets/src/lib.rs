//! Widget layer of the layered `waterui` dylib chain.
//!
//! Sits on `waterui-dylib-graphics`; carries the widget tier (`layout`,
//! `text`, `form`, `controls`, `navigation`, `icon`), which depends on the
//! graphics crates and therefore cannot sit below them — but does not belong
//! in the graphics image, whose export count sits near the budget (#1615).

pub use waterui_dylib_graphics;

pub use waterui_controls;
pub use waterui_form;
pub use waterui_icon;
pub use waterui_layout;
pub use waterui_navigation;
pub use waterui_text;
