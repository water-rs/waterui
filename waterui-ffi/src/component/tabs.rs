use core::num::NonZero;

use alloc::vec::Vec;
use waterui::{
    tab::{Tab, TabsConfig},
    utils::Id,
    view::TaggedView,
    Computed,
};

use crate::{array::waterui_array, waterui_anyview, IntoFFI, IntoRust};

use super::{navigation::waterui_navigation_view_builder, waterui_binding_id};

#[repr(C)]
pub struct waterui_tab {
    label: *mut waterui_anyview,
    tag: usize,
    content: *mut waterui_navigation_view_builder,
}

impl IntoFFI for Tab<Id> {
    type FFI = waterui_tab;
    fn into_ffi(self) -> Self::FFI {
        waterui_tab {
            label: self.label.view.into_ffi(),
            tag: self.label.tag.into(),
            content: self.content.into_ffi(),
        }
    }
}

#[repr(C)]
pub struct waterui_tabs {
    selection: *mut waterui_binding_id,
    tabs: *mut waterui_computed_tabs,
}

impl IntoRust for waterui_tabs {
    type Rust = TabsConfig;
    unsafe fn into_rust(self) -> Self::Rust {
        TabsConfig {
            selection: self.selection.into_rust(),
            tabs: self.tabs.into_rust(),
        }
    }
}

impl IntoRust for waterui_tab {
    type Rust = Tab<Id>;
    unsafe fn into_rust(self) -> Self::Rust {
        Tab {
            label: TaggedView::new(NonZero::new_unchecked(self.tag), self.label.into_rust()),
            content: self.content.into_rust(),
        }
    }
}

ffi_type!(
    waterui_computed_tabs,
    Computed<Vec<Tab<Id>>>,
    waterui_drop_computed_tabs
);

impl_computed!(
    waterui_computed_tabs,
    waterui_array<waterui_tab>,
    waterui_read_computed_tabs,
    waterui_watch_computed_tabs
);

impl IntoFFI for TabsConfig {
    type FFI = waterui_tabs;
    fn into_ffi(self) -> Self::FFI {
        waterui_tabs {
            selection: self.selection.into_ffi(),
            tabs: self.tabs.into_ffi(),
        }
    }
}

native_view!(
    TabsConfig,
    waterui_tabs,
    waterui_view_force_as_tabs,
    waterui_view_tabs_id
);
