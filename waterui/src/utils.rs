use core::{cell::RefCell, fmt::Debug, num::NonZeroUsize};

use alloc::{collections::btree_map::BTreeMap, rc::Rc};
use waterui_reactive::Binding;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub opacity: f64,
}

impl Default for Color {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENCY: Self = Self::rgba(0, 0, 0, 0.0);
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, 1.0)
    }

    pub const DEFAULT: Self = Self::rgba(0, 0, 0, f64::NAN);

    pub const fn rgba(red: u8, green: u8, blue: u8, opacity: f64) -> Self {
        Self {
            red,
            green,
            blue,
            opacity,
        }
    }

    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Background {
    Default,
    Color(Color),
}

impl_from!(Background, Color);

impl Default for Background {
    fn default() -> Self {
        Self::Default
    }
}

pub trait HandleBorrowed<F> {
    fn handle(self, f: F) -> impl Fn() + 'static;
}

macro_rules! impl_handle_borrowed {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <F,$($ty:Clone+'static,)*>HandleBorrowed<F> for ($(&$ty),*)
        where
            F: Fn($(&$ty),*) + 'static{

            fn handle(self, f: F) -> impl Fn() + 'static {
                let ($($ty),*) = self;
                let ($($ty),*) = ($($ty.clone()),*);

                move || f($(&$ty),*)
            }
        }
    };
}

tuples!(impl_handle_borrowed);
pub type Id = NonZeroUsize;
struct MappingInner<T> {
    counter: Id,
    to_id: BTreeMap<T, Id>,
    from_id: BTreeMap<Id, T>,
}

impl<T: Ord + Clone> MappingInner<T> {
    pub const fn new() -> Self {
        Self {
            counter: Id::MIN,
            to_id: BTreeMap::new(),
            from_id: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, value: T) -> Id {
        let id = self.counter;
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
        source.map(
            move |value| mapping.to_id(value),
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
