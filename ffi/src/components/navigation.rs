use crate::array::WuiArray;
use crate::components::text::WuiText;
use crate::{IntoFFI, WuiAnyView, impl_binding};
use crate::{closure::WuiFn, ffi_struct, ffi_view};
use alloc::vec::Vec;
use waterui::{Binding, Color, Computed};
use waterui_core::id::Id;
use waterui_navigation::tab::Tab;
use waterui_navigation::{Bar, NavigationLink, NavigationView, tab::TabsConfig};

pub struct WuiNavigationView {
    pub root: *mut WuiAnyView,
    pub bar: *mut Bar,
    pub links: *mut Vec<NavigationLink>,
    pub tabs: *mut Option<TabsConfig>,
    pub selection: *mut Option<Binding<Id>>,
}

pub struct WuiNavigationLink {
    pub label: *mut WuiAnyView,
    pub destination: *mut WuiFn<*mut WuiAnyView>,
}

/// C representation of a navigation Bar configuration
#[repr(C)]
pub struct WuiBar {
    /// Pointer to the title text
    pub title: WuiText,
    /// Pointer to the background color computed value
    pub color: *mut Computed<Color>,
    /// Pointer to the hidden state computed value
    pub hidden: *mut Computed<bool>,
}

// Implement struct conversions
ffi_struct!(Bar, WuiBar, title, color, hidden);

// FFI view bindings for navigation components
ffi_view!(NavigationView, waterui_navigation_view_id);
ffi_view!(NavigationLink, waterui_navigation_link_id);

#[repr(C)]
pub struct WuiTabs {
    /// The currently selected tab identifier.
    pub selection: *mut Binding<Id>,

    /// The collection of tabs to display.
    pub tabs: WuiArray<WuiTab>,
}

#[repr(C)]
pub struct WuiTab {
    /// The unique identifier for the tab.
    pub id: Id,

    /// Pointer to the tab's label view.
    pub label: *mut WuiAnyView,

    /// Pointer to the tab's content view.
    pub content: *mut WuiAnyView,
}

impl IntoFFI for Tab<Id> {
    type FFI = WuiTab;
    fn into_ffi(self) -> Self::FFI {
        WuiTab {
            id: self.label.tag,
            label: self.label.content.into_ffi(),
            content: todo!(),
        }
    }
}

ffi_struct!(TabsConfig, WuiTabs, selection, tabs);
