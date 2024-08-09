use core::num::NonZeroUsize;

use alloc::vec::Vec;
use waterui::{
    component::picker::{ItemId, PickerConfig, PickerItem},
    view::{ConfigurableView, TaggedView},
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
        TaggedView::new(
            NonZeroUsize::new_unchecked(self.tag),
            self.label.into_rust(),
        )
    }
}

impl IntoFFI for PickerItem<NonZeroUsize> {
    type FFI = waterui_picker_item;
    fn into_ffi(self) -> Self::FFI {
        waterui_picker_item {
            label: self.view.config().into_ffi(),
            tag: self.tag.into(),
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

impl IntoFFI for PickerConfig {
    type FFI = waterui_picker;

    fn into_ffi(self) -> Self::FFI {
        Self::FFI {
            items: self.items.into_ffi(),
            selection: self.selection.into_ffi(),
        }
    }
}

native_view!(
    PickerConfig,
    waterui_picker,
    waterui_view_force_as_picker,
    waterui_view_picker_id
);
