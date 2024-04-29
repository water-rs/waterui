use core::{cell::RefCell, ops::Deref};

use crate::utils::IdentifierMap;
use crate::{AnyView, Environment, View};
use alloc::{rc::Rc, vec::Vec};
use waterui_reactive::{Binding, ComputeExt, Computed};
pub struct Picker<T> {
    items: Computed<Vec<PickerItem<T>>>,
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
    pub _items: Computed<Vec<RawPickerItem>>,
    pub _selection: Binding<i32>,
}

impl<T> View for Picker<T>
where
    T: Ord + Clone + 'static,
{
    fn body(self, _env: Environment) -> impl View {
        let map = Rc::new(RefCell::new(IdentifierMap::new()));
        let items;
        {
            let map = map.clone();
            items = self
                .items
                .map(move |items| {
                    items
                        .into_iter()
                        .map(|item| RawPickerItem {
                            _label: item.label,
                            _value: map.borrow_mut().insert(item.value),
                        })
                        .collect::<Vec<_>>()
                })
                .computed();
        }

        let selection = self.selection.bridge(
            {
                let map = map.clone();
                move |new| Some(map.borrow().to_data(*new.get() as usize).cloned().unwrap())
            },
            move |this| {
                if let Some(this) = this.get().deref() {
                    map.borrow().to_id(this).map(|v| v as i32).unwrap()
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
