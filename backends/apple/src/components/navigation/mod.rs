//! The navigation group: navigation stacks, split views, tab containers,
//! the navigation bar model, menus and the window's toolbar, plus the
//! metadata the group consumes.
//!
//! `Native<NavigationView>` is a page: chrome (title, items, search) plus
//! content. Inside a `NavigationStack` the platform's own navigation chrome
//! hosts the bar (`UINavigationBar` on `UIKit`, the window toolbar on
//! `AppKit`); standalone it draws an in-content bar. `Native<NavigationStack>`
//! is the stack container, `Native<NavigationSplitLayout>` a two- or
//! three-column split, `Native<TabsLayout>` the platform's tab container,
//! and `Native<ResolvedMenu>` a menu trigger.

use alloc::string::String;

use cocoa_ui::PlatformView;

pub mod bar;
pub mod menu;
pub mod metadata;
pub mod nav_view;
pub mod split;
pub mod stack;
pub mod tabs;

/// Installs every navigation handler on the dispatcher.
pub fn install(dispatcher: &mut crate::dispatch::Dispatcher) {
    metadata::install(dispatcher);
    menu::install(dispatcher);
    nav_view::install(dispatcher);
    stack::install(dispatcher);
    split::install(dispatcher);
    tabs::install(dispatcher);
}

/// The first plain-text string a rendered subtree's text views carry —
/// `extractNavigationTitleText`'s walk: labels on `UIKit`, text fields on
/// `AppKit`, depth-first.
#[cfg(target_os = "macos")]
pub fn extract_title_text(view: &PlatformView) -> Option<String> {
    use cocoa_ui::objc2_app_kit::NSTextField;
    if let Some(field) = view.downcast_ref::<NSTextField>() {
        let text = field.stringValue().to_string();
        if !text.is_empty() {
            return Some(text);
        }
        let attributed = field.attributedStringValue().string().to_string();
        if !attributed.is_empty() {
            return Some(attributed);
        }
    }
    for subview in &cocoa_ui::view::subviews(view) {
        if let Some(found) = extract_title_text(subview) {
            return Some(found);
        }
    }
    None
}

/// The first plain-text string a rendered subtree's text views carry —
/// `extractNavigationTitleText`'s walk on `UIKit`.
#[cfg(target_os = "ios")]
pub fn extract_title_text(view: &PlatformView) -> Option<String> {
    use cocoa_ui::objc2_ui_kit::UILabel;
    if let Some(label) = view.downcast_ref::<UILabel>() {
        let text = label
            .attributedText()
            .map(|text| text.string().to_string())
            .or_else(|| label.text().map(|text| text.to_string()));
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            return Some(text);
        }
    }
    for subview in &cocoa_ui::view::subviews(view) {
        if let Some(found) = extract_title_text(subview) {
            return Some(found);
        }
    }
    None
}
