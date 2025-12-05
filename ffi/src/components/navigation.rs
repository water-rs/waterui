use crate::array::WuiArray;
use crate::components::text::WuiText;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiAnyView, ffi_view};
use crate::closure::WuiFn;
use waterui::Color;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_navigation::tab::{Tab, TabPosition, Tabs};
use waterui_navigation::{Bar, NavigationStack, NavigationView};

into_ffi! {
    NavigationView,
    pub struct WuiNavigationView {
        bar: WuiBar,
        content: *mut WuiAnyView,
    }
}

pub struct WuiNavigationLink {
    pub label: *mut WuiAnyView,
    pub destination: *mut WuiFn<*mut WuiAnyView>,
}

into_ffi! {Bar,
    pub struct WuiBar {
        title: WuiText,
        color: *mut WuiComputed<Color>,
        hidden: *mut WuiComputed<bool>,
    }
}

// FFI view bindings for navigation components
ffi_view!(NavigationView, WuiNavigationView, navigation_view);

/// FFI struct for NavigationStack<(),()>
#[repr(C)]
pub struct WuiNavigationStack {
    /// The root view of the navigation stack.
    pub root: *mut WuiAnyView,
}

impl IntoFFI for NavigationStack<(), ()> {
    type FFI = WuiNavigationStack;
    fn into_ffi(self) -> Self::FFI {
        WuiNavigationStack {
            root: self.into_inner().into_ffi(),
        }
    }
}

ffi_view!(NavigationStack<(),()>, WuiNavigationStack, navigation_stack);

/// Position of the tab bar within the tab container.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiTabPosition {
    /// Tab bar is positioned at the top of the container.
    Top = 0,
    /// Tab bar is positioned at the bottom of the container.
    Bottom = 1,
}

impl From<TabPosition> for WuiTabPosition {
    fn from(pos: TabPosition) -> Self {
        match pos {
            TabPosition::Top => WuiTabPosition::Top,
            TabPosition::Bottom => WuiTabPosition::Bottom,
        }
    }
}

#[repr(C)]
pub struct WuiTabs {
    /// The currently selected tab identifier.
    pub selection: *mut WuiBinding<Id>,

    /// The collection of tabs to display.
    pub tabs: WuiArray<WuiTab>,

    /// Position of the tab bar (top or bottom).
    pub position: WuiTabPosition,
}

opaque!(WuiTabContent, AnyViewBuilder<NavigationView>, tab_content);

#[repr(C)]
pub struct WuiTab {
    /// The unique identifier for the tab (raw u64 for FFI compatibility).
    pub id: u64,

    /// Pointer to the tab's label view.
    pub label: *mut WuiAnyView,

    /// Pointer to the tab's content view.
    pub content: *mut WuiTabContent,
}

/// Creates a navigation view from tab content.
///
/// # Safety
///
/// This function is unsafe because:
/// - `handler` must be a valid, non-null pointer to a `WuiTabContent`
/// - Both pointers must remain valid for the duration of the function call
/// - The caller must ensure proper memory management of the returned view
pub unsafe extern "C" fn waterui_tab_content(handler: *mut WuiTabContent) -> WuiNavigationView {
    unsafe {
        let view = (&*handler).build();
        IntoFFI::into_ffi(view)
    }
}

impl IntoFFI for Tab<Id> {
    type FFI = WuiTab;
    fn into_ffi(self) -> Self::FFI {
        WuiTab {
            id: i32::from(self.label.tag) as u64,
            label: self.label.content.into_ffi(),
            content: self.content.into_ffi(),
        }
    }
}

impl IntoFFI for Tabs {
    type FFI = WuiTabs;
    fn into_ffi(self) -> Self::FFI {
        WuiTabs {
            selection: self.selection.into_ffi(),
            tabs: self.tabs.into_ffi(),
            position: self.position.into(),
        }
    }
}

// FFI view binding for Tabs
ffi_view!(Tabs, WuiTabs, tabs);
