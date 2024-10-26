use core::num::NonZeroI32;

use waterui::{
    component::navigation::tab::{Tab, TabsConfig},
    utils::Id,
    view::TaggedView,
};

use crate::{array::waterui_array, waterui_anyview, IntoFFI, IntoRust};

use super::{navigation::waterui_navigation_view_builder, waterui_binding_id};

#[repr(C)]
pub struct waterui_tab {
    label: *mut waterui_anyview,
    tag: i32,
    content: *mut waterui_navigation_view_builder,
}

impl IntoFFI for Tab<Id> {
    type FFI = waterui_tab;
    fn into_ffi(self) -> Self::FFI {
        waterui_tab {
            label: self.label.content.into_ffi(),
            tag: self.label.tag.into(),
            content: self.content.into_ffi(),
        }
    }
}

#[repr(C)]
pub struct waterui_tabs {
    selection: *mut waterui_binding_id,
    tabs: waterui_array<waterui_tab>,
}

impl IntoRust for waterui_tabs {
    type Rust = TabsConfig;
    unsafe fn into_rust(self) -> Self::Rust {
        TabsConfig::new(self.selection.into_rust(), self.tabs.into_rust())
    }
}

impl IntoRust for waterui_tab {
    type Rust = Tab<Id>;
    unsafe fn into_rust(self) -> Self::Rust {
        Tab {
            label: TaggedView::new(NonZeroI32::new(self.tag).unwrap(), self.label.into_rust()),
            content: self.content.into_rust(),
        }
    }
}

into_ffi!(TabsConfig, waterui_tabs, selection, tabs);

native_view!(
    TabsConfig,
    waterui_tabs,
    waterui_view_force_as_tabs,
    waterui_view_tabs_id
);
