use core::ops::Deref;

use crate::utils::IdentifierMap;
use alloc::{rc::Rc, vec::Vec};
use waterui_reactive::{Binding, Int};
use waterui_view::{AnyView, View};
pub struct Picker<T> {
    items: Vec<PickerItem<T>>,
    selection: Binding<Option<T>>,
}

pub struct PickerItem<T> {
    label: AnyView,
    value: T,
}

pub struct RawPickerItem {
    label: AnyView,
    value: usize,
}

pub struct RawPicker {
    items: Vec<RawPickerItem>,
    selection: Binding<Int>,
}

impl<T> View for Picker<T>
where
    T: Ord + Clone + 'static,
{
    fn body(self, _env: waterui_view::Environment) -> impl View {
        let mut map = IdentifierMap::new();
        let items = self
            .items
            .into_iter()
            .map(|item| RawPickerItem {
                label: item.label,
                value: map.insert(item.value),
            })
            .collect::<Vec<_>>();
        let map = Rc::new(map);
        let selection = self.selection.bridge(
            {
                let map = map.clone();
                move |new| Some(map.to_data(*new.get() as usize).cloned().unwrap())
            },
            move |this| {
                if let Some(this) = this.get().deref() {
                    map.to_id(this).map(|v| v as Int).unwrap()
                } else {
                    -1
                }
            },
        );
        RawPicker { items, selection }
    }
}

raw_view!(RawPicker);

mod ffi {
    use alloc::vec::Vec;
    use waterui_ffi::{
        array::waterui_array, binding::waterui_binding_int, ffi_view, waterui_anyview, IntoFFI,
    };

    use super::{RawPicker, RawPickerItem};
    #[repr(C)]
    pub struct waterui_picker_item {
        label: *mut waterui_anyview,
        value: usize,
    }

    #[repr(C)]
    pub struct waterui_picker {
        items: waterui_array<waterui_picker_item>,
        selection: *const waterui_binding_int,
    }

    impl IntoFFI for RawPickerItem {
        type FFI = waterui_picker_item;
        fn into_ffi(self) -> Self::FFI {
            waterui_picker_item {
                label: self.label.into_ffi(),
                value: self.value,
            }
        }
    }

    impl IntoFFI for RawPicker {
        type FFI = waterui_picker;
        fn into_ffi(self) -> Self::FFI {
            waterui_picker {
                items: self
                    .items
                    .into_iter()
                    .map(|v| v.into_ffi())
                    .collect::<Vec<_>>()
                    .into_ffi(),
                selection: self.selection.into_ffi(),
            }
        }
    }

    ffi_view!(
        RawPicker,
        waterui_picker,
        waterui_view_force_as_picker,
        waterui_view_picker_id
    );
}
