use core::{cell::RefCell, num::NonZeroI32};

use alloc::{collections::btree_map::BTreeMap, rc::Rc};
use waterui_reactive::Binding;

pub type Id = NonZeroI32;
#[derive(Debug)]
struct MappingInner<T> {
    counter: i32,
    to_id: BTreeMap<T, Id>,
    from_id: BTreeMap<Id, T>,
}

impl<T: Ord + Clone> MappingInner<T> {
    pub const fn new() -> Self {
        Self {
            counter: 1,
            to_id: BTreeMap::new(),
            from_id: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, value: T) -> Id {
        let id = NonZeroI32::new(self.counter).unwrap();
        self.to_id.insert(value.clone(), id);
        self.from_id.insert(id, value);
        self.counter = self.counter.checked_add(1).unwrap();
        id
    }

    pub fn try_to_id(&self, value: &T) -> Option<Id> {
        self.to_id.get(value).cloned()
    }

    pub fn to_data(&self, id: Id) -> Option<T> {
        self.from_id.get(&id).cloned()
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn to_id(&mut self, value: T) -> Id {
        self.try_to_id(&value)
            .unwrap_or_else(|| self.register(value))
    }
}
#[derive(Debug)]
pub struct Mapping<T>(Rc<RefCell<MappingInner<T>>>);

impl<T> Clone for Mapping<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Ord + Clone> Default for Mapping<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord + Clone> Mapping<T> {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(MappingInner::new())))
    }

    pub fn register(&self, value: T) -> Id {
        self.0.borrow_mut().register(value)
    }

    pub fn try_to_id(&self, value: &T) -> Option<Id> {
        self.0.borrow().try_to_id(value)
    }
    pub fn to_id(&self, value: T) -> Id {
        self.0.borrow_mut().to_id(value)
    }

    pub fn to_data(&self, id: Id) -> Option<T> {
        self.0.borrow().to_data(id)
    }

    pub fn binding(&self, source: Binding<T>) -> Binding<Id>
    where
        T: 'static,
    {
        let mapping = self.clone();
        let mapping2 = self.clone();
        Binding::map(
            &source,
            move |value| mapping.to_id(value.clone()),
            move |binding, value| {
                binding.set(
                    mapping2
                        .to_data(value)
                        .expect("Invalid binding mapping : Data not found"),
                )
            },
        )
    }
}
