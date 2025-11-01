use crate::array::WuiArray;
use crate::components::text::WuiText;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiAnyView};
use crate::{closure::WuiFn, ffi_view};
use waterui::Color;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_navigation::tab::Tab;
use waterui_navigation::{Bar, NavigationView};

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
ffi_view!(NavigationView, WuiNavigationView);

#[repr(C)]
pub struct WuiTabs {
    /// The currently selected tab identifier.
    pub selection: *mut WuiBinding<Id>,

    /// The collection of tabs to display.
    pub tabs: WuiArray<WuiTab>,
}

opaque!(WuiTabContent, AnyViewBuilder<NavigationView>, tab_content);

#[repr(C)]
pub struct WuiTab {
    /// The unique identifier for the tab.
    pub id: Id,

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
            id: self.label.tag,
            label: self.label.content.into_ffi(),
            content: self.content.into_ffi(),
        }
    }
}
