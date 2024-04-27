use core::ops::Deref;

use crate::utils::IdentifierMap;
use alloc::{rc::Rc, vec::Vec};
use waterui_reactive::Binding;
use waterui_view::{AnyView, View};
pub struct Picker<T> {
    items: Vec<PickerItem<T>>,
    selection: Binding<Option<T>>,
}

pub struct PickerItem<T> {
    label: AnyView,
    value: T,
}

#[non_exhaustive]
pub struct RawPickerItem {
    pub _label: AnyView,
    pub _value: usize,
}

#[non_exhaustive]
pub struct RawPicker {
    pub _items: Vec<RawPickerItem>,
    pub _selection: Binding<i32>,
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
                _label: item.label,
                _value: map.insert(item.value),
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
                    map.to_id(this).map(|v| v as i32).unwrap()
                } else {
                    -1
                }
            },
        );
        RawPicker {
            _items: items,
            _selection: selection,
        }
    }
}

raw_view!(RawPicker);
