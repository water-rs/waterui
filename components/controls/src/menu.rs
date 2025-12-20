use alloc::{rc::Rc, vec::Vec};
use nami::{Computed, impl_constant};
use waterui_core::{
    AnyView,
    handler::{HandlerFn, SharedHandler, into_handler},
};
use waterui_text::Text;

/// Configuration for a menu component.
#[derive(Debug)]
#[non_exhaustive]
pub struct MenuConfig {
    /// The label for the menu.
    pub label: AnyView,
    /// The items in the menu.
    pub items: Computed<Vec<MenuItem>>,
}

/// A single item in a menu.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MenuItem {
    /// The label for the menu item.
    pub label: Text,
    /// The action to perform when the menu item is selected.
    pub action: SharedHandler<()>,
}

impl_constant!(MenuItem);

impl MenuItem {
    /// Creates a new menu item with the given label and action.
    pub fn new<P: 'static>(label: impl Into<Text>, action: impl HandlerFn<P, ()>) -> Self {
        Self {
            label: label.into(),
            action: Rc::new(into_handler(action)),
        }
    }
}
