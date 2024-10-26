pub mod color;

use core::num::NonZeroI32;

use alloc::vec::Vec;
use waterui::{
    component::picker::{ItemId, PickerConfig, PickerItem},
    view::{ConfigurableView, TaggedView},
};

use crate::{array::waterui_array, IntoFFI, IntoRust};

use super::{text::waterui_text, waterui_binding_id};

#[repr(C)]
pub struct waterui_picker {
    items: *mut waterui_computed_picker_items,
    selection: *mut waterui_binding_id,
}

impl_computed!(
    waterui_computed_picker_items,
    Vec<PickerItem<ItemId>>,
    waterui_array<waterui_picker_item>,
    waterui_read_computed_picker_items,
    waterui_watch_computed_picker_items,
    waterui_drop_computed_picker_items
);

impl IntoRust for waterui_picker_item {
    type Rust = PickerItem<ItemId>;
    unsafe fn into_rust(self) -> Self::Rust {
        TaggedView::new(NonZeroI32::new(self.tag).unwrap(), self.label.into_rust())
    }
}

impl IntoFFI for PickerItem<NonZeroI32> {
    type FFI = waterui_picker_item;
    fn into_ffi(self) -> Self::FFI {
        waterui_picker_item {
            label: self.content.config().into_ffi(),
            tag: self.tag.into(),
        }
    }
}

#[repr(C)]
pub struct waterui_picker_item {
    label: waterui_text,
    tag: i32,
}

into_ffi!(PickerConfig, waterui_picker, items, selection);

native_view!(
    PickerConfig,
    waterui_picker,
    waterui_view_force_as_picker,
    waterui_view_picker_id
);
