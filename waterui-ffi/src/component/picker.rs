use core::num::NonZeroUsize;

use alloc::vec::Vec;
use waterui::{
    component::picker::{ItemId, Picker, PickerItem},
    Binding, Computed,
};

use crate::{array::waterui_array, IntoFFI, IntoRust};

use super::text::waterui_text;

#[repr(C)]
pub struct waterui_picker {
    items: *mut waterui_computed_picker_items,
    selection: *mut waterui_binding_picker_item_id,
}

ffi_type!(
    waterui_computed_picker_items,
    Computed<Vec<PickerItem<ItemId>>>,
    waterui_drop_computed_picker_items
);

impl_computed!(
    waterui_computed_picker_items,
    waterui_array<waterui_picker_item>,
    waterui_read_computed_picker_item,
    waterui_watch_computed_picker_item
);

impl IntoRust for waterui_picker_item {
    type Rust = PickerItem<ItemId>;
    unsafe fn into_rust(self) -> Self::Rust {
        PickerItem {
            _label: self.label.into_rust(),
            _tag: NonZeroUsize::new_unchecked(self.tag),
        }
    }
}

impl IntoFFI for PickerItem<NonZeroUsize> {
    type FFI = waterui_picker_item;
    fn into_ffi(self) -> Self::FFI {
        waterui_picker_item {
            label: self._label.into_ffi(),
            tag: self._tag.into(),
        }
    }
}

ffi_type!(
    waterui_binding_picker_item_id,
    Binding<Option<ItemId>>,
    waterui_drop_binding_picker_item_id
);

#[repr(C)]
pub struct waterui_picker_item {
    label: waterui_text,
    tag: usize,
}

impl IntoFFI for Picker {
    type FFI = waterui_picker;

    fn into_ffi(self) -> Self::FFI {
        Self::FFI {
            items: self._items.into_ffi(),
            selection: self._selection.into_ffi(),
        }
    }
}

ffi_view!(
    Picker,
    waterui_picker,
    waterui_view_force_as_picker,
    waterui_view_picker_id
);
