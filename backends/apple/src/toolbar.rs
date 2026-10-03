//! Window toolbar item promotion — the chrome behaviour
//! `WuiWindowToolbar.setWindowContent` carried, now owned by the backend.
//!
//! A declared window-toolbar child promotes to a real `NSToolbarItem` when
//! it is a button whose label draws a platform symbol or renders into a
//! template image; every other child is hosted as the view it is.

#![cfg(target_os = "macos")]

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use cocoa_ui::Retained;
use cocoa_ui::appkit::{HostedItem, ToolbarChild};
use cocoa_ui::objc2_app_kit::{NSView, NSWindow};
use objc2::msg_send;
use objc2_foundation::NSString;

/// The point size a rendered label rasterizes into a toolbar icon.
const TOOLBAR_ICON_SIZE: f64 = 18.0;

/// Installs `view` — the rendered window-toolbar host — as `window`'s
/// toolbar items: lone-child wrappers are descended and the first
/// multi-child view's children become the items, as
/// `WuiWindowToolbar.setWindowContent` answered them.
pub fn install_toolbar_items(window: &NSWindow, view: &NSView) {
    let mut node = cocoa_ui::view::retain_base(view);
    loop {
        let subs = cocoa_ui::view::subviews(&node);
        if subs.len() != 1 {
            break;
        }
        node = subs[0].clone();
    }
    let subs = cocoa_ui::view::subviews(&node);
    let children: Vec<Retained<NSView>> = if subs.is_empty() {
        alloc::vec![node]
    } else {
        subs
    };
    cocoa_ui::appkit::WindowToolbar::attached(window)
        .set_window_items(children.iter().map(toolbar_child).collect());
}

/// Describes one declared toolbar child to the kit: a button whose label
/// draws a platform symbol becomes a real `NSToolbarItem` — icon in the
/// capsule, name kept for the overflow menu, tooltip and assistive
/// technology, running the button's action — exactly as a navigation
/// action does. A label that is not a platform symbol renders into a
/// template image the toolbar tints like its own items, and any other
/// child is hosted as the view it is.
fn toolbar_child(view: &Retained<NSView>) -> ToolbarChild {
    let hosted = |view: &Retained<NSView>| HostedItem {
        view: view.clone(),
        size: view.fittingSize().into(),
    };
    let Some(button) = cocoa_ui::appkit::first_button(view) else {
        return ToolbarChild {
            view: hosted(view),
            icon: None,
            label: String::new(),
            bordered: false,
            action: None,
        };
    };
    // The button's label view is its sibling: the button leaf mounts the
    // button and its label container into the same parent.
    let label_view = {
        let button_view: *const NSView = &raw const ****button;
        // SAFETY: `superview`/`subviews` are ordinary main-thread reads.
        unsafe { button.superview() }.and_then(|parent| {
            parent
                .subviews()
                .into_iter()
                .find(|subview| !core::ptr::eq::<NSView>(&raw const **subview, button_view))
        })
    };
    let icon = label_view
        .as_ref()
        .and_then(|label| cocoa_ui::appkit::first_symbol_view(label))
        .and_then(|symbol_view| symbol_view.symbol_name())
        .and_then(|name| cocoa_ui::appkit::symbol_image(&name))
        .or_else(|| {
            label_view
                .as_ref()
                .and_then(|label| cocoa_ui::bitmap::view_template_image(label, TOOLBAR_ICON_SIZE))
        });
    // SAFETY: `accessibilityLabel` is a plain getter on the main thread.
    let text: Option<Retained<NSString>> = unsafe { msg_send![&button, accessibilityLabel] };
    let label = text.map_or_else(String::new, |label| label.to_string());
    // The label's own button carries the handler, so the toolbar item runs
    // the same action the view would have.
    let action = Rc::new(move || {
        // SAFETY: toolbar actions fire on the main thread.
        unsafe { cocoa_ui::appkit::activate(button.control()) };
    });
    ToolbarChild {
        view: hosted(view),
        icon,
        label,
        bordered: true,
        action: Some(action),
    }
}
