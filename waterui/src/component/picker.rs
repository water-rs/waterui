use core::ops::Deref;

use crate::AnyView;
use alloc::collections::BTreeMap;
use alloc::{rc::Rc, vec::Vec};
use waterui_core::raw_view;
use waterui_reactive::{Binding, Compute};

#[derive(Debug)]
pub struct TaggedView<T> {
    content: AnyView,
    tag: T,
}

#[non_exhaustive]
#[derive(Debug)]
pub struct Picker {
    pub _items: Vec<(AnyView, i32)>,
    pub _selection: Binding<i32>,
}

impl Picker {
    pub fn new<T: Ord + Clone + 'static>(
        items: impl Into<Vec<TaggedView<T>>>,
        selection: &Binding<T>,
    ) -> Self {
        let items = items.into();
        let mut map = IdentifierMap::new();
        let items = items
            .into_iter()
            .map(|v| (v.content, map.insert(v.tag)))
            .collect();

        let map = Rc::new(map);

        let selection = selection.bridge(
            {
                let map = map.clone();
                move |new| map.to_data(new.compute()).unwrap().clone()
            },
            move |old| map.to_id(old.get().deref()).unwrap(),
        );

        Self {
            _items: items,
            _selection: selection,
        }
    }
}

raw_view!(Picker);

struct IdentifierMap<T> {
    counter: i32,
    to_id: BTreeMap<T, i32>,
    from_id: BTreeMap<i32, T>,
}

impl<T: Ord + Clone> Default for IdentifierMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord + Clone> IdentifierMap<T> {
    pub fn new() -> Self {
        Self {
            counter: 0,
            to_id: BTreeMap::new(),
            from_id: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, value: T) -> i32 {
        let id = self.counter;
        self.to_id.insert(value.clone(), id);
        self.from_id.insert(id, value);
        self.counter += 1;
        id
    }

    pub fn to_id(&self, value: &T) -> Option<i32> {
        self.to_id.get(value).cloned()
    }

    pub fn to_data(&self, id: i32) -> Option<&T> {
        self.from_id.get(&id)
    }
}
