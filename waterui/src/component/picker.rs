use core::cell::RefCell;
use core::num::NonZeroUsize;

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;
use waterui_core::raw_view;
use waterui_reactive::compute::{ToCompute, ToComputed};
use waterui_reactive::{Binding, ComputeExt, Computed};

use super::Text;

type ItemId = NonZeroUsize;

#[non_exhaustive]
#[derive(Debug)]
pub struct Picker {
    pub _items: Computed<Vec<PickerItem<ItemId>>>,
    pub _selection: Binding<Option<ItemId>>,
}

pub struct PickerItem<T> {
    label: Text,
    tag: T,
}

impl Picker {
    pub fn new<T: Ord + Clone + 'static>(
        items: impl ToComputed<Vec<PickerItem<T>>>,
        selection: &Binding<Option<T>>,
    ) -> Self {
        let items = items.to_computed();
        let map = Rc::new(RefCell::new(IdentifierMap::new()));
        let map2 = map.clone();
        let map3 = map.clone();

        let _items = items
            .to_compute()
            .map(move |items: Vec<PickerItem<T>>| {
                items
                    .into_iter()
                    .map(|item| PickerItem {
                        label: item.label,
                        tag: map.borrow_mut().register(item.tag),
                    })
                    .collect::<Vec<_>>()
            })
            .computed();

        let selection = selection.clone();
        let selection2 = selection.clone();

        let _selection = Binding::from_fn(
            move || selection.get().and_then(|v| map2.borrow().to_id(&v)),
            move |id| {
                let data = id.and_then(|id| map3.borrow().to_data(id));
                selection2.set(data);
            },
        );

        Self { _items, _selection }
    }
}

raw_view!(Picker);

struct IdentifierMap<T> {
    counter: ItemId,
    to_id: BTreeMap<T, ItemId>,
    from_id: BTreeMap<ItemId, T>,
}

impl<T: Ord + Clone> Default for IdentifierMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord + Clone> IdentifierMap<T> {
    pub fn new() -> Self {
        Self {
            counter: ItemId::MIN,
            to_id: BTreeMap::new(),
            from_id: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, value: T) -> ItemId {
        let id = self.counter;
        self.to_id.insert(value.clone(), id);
        self.from_id.insert(id, value);
        self.counter = self.counter.checked_add(1).unwrap();
        id
    }

    pub fn to_id(&self, value: &T) -> Option<ItemId> {
        self.to_id.get(value).cloned()
    }

    pub fn to_data(&self, id: ItemId) -> Option<T> {
        self.from_id.get(&id).cloned()
    }
}
