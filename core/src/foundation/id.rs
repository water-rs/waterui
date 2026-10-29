//! Identity, tagging, and mapping functionality for UI components.
//!
//! This module provides various utilities for:
//! - Identifying and tagging UI elements with unique identifiers
//! - Creating mappings between values and numeric IDs
//! - Wrapping views with identifying information
//! - Converting between different ID types
//!
//! The primary types in this module include:
//! - `Identifiable`: A trait for types that can be uniquely identified
//! - `TaggedView`: A view wrapper that includes an identifying tag
//! - `Mapping`: A bidirectional mapping between values and numeric IDs
//! - `UseId` and `SelfId`: Wrappers that implement different ID strategies

use core::num::NonZeroI32;
use core::num::TryFromIntError;

use crate::{AnyView, View};

/// A non-zero i32 value used for identification purposes throughout the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub(crate) NonZeroI32);

impl From<Id> for i32 {
    fn from(val: Id) -> Self {
        val.0.get()
    }
}

impl From<NonZeroI32> for Id {
    fn from(value: NonZeroI32) -> Self {
        Self(value)
    }
}

impl TryFrom<i32> for Id {
    type Error = TryFromIntError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        NonZeroI32::try_from(value).map(Id)
    }
}

/// Item identity, shared with `nami`'s collections: a `List` of
/// [`Identifiable`] items diffs them by id, and so does every view
/// collection built on it.
pub use nami::{Identifiable, IdentifiableExt, SelfId, UseId};

/// A view that includes an identifying tag of type T.
///
/// This allows tracking and identification of views within a UI hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaggedView<T, V> {
    /// The tag used to identify this view
    pub tag: T,
    /// The actual view content
    pub content: V,
}

impl<T, V: View> TaggedView<T, V> {
    /// Creates a new tagged view with the specified tag and content.
    pub const fn new(tag: T, content: V) -> Self {
        Self { tag, content }
    }

    /// Transforms the tag of this view using the provided function.
    pub fn map<F, T2>(self, f: F) -> TaggedView<T2, V>
    where
        F: Fn(T) -> T2,
    {
        TaggedView {
            tag: f(self.tag),
            content: self.content,
        }
    }

    /// Converts the tag to an Id using the provided mapping.
    pub fn mapping(self, mapping: &Mapping<T>) -> TaggedView<Id, V>
    where
        T: Ord + Clone,
    {
        self.map(move |v| mapping.register(v))
    }

    /// Erases the specific view type, converting it to [`AnyView`].
    ///
    /// This is useful for storing heterogeneous views in a collection.
    pub fn erase(self) -> TaggedView<T, AnyView> {
        TaggedView {
            tag: self.tag,
            content: AnyView::new(self.content),
        }
    }
}

use core::cell::RefCell;

use alloc::{collections::btree_map::BTreeMap, rc::Rc};
use nami::Binding;

/// Internal implementation of the mapping functionality.
///
/// Handles the bidirectional mapping between values and IDs.
#[derive(Debug)]
struct MappingInner<T> {
    /// Counter used to generate new IDs
    counter: i32,
    /// Maps from values to their assigned IDs
    to_id: BTreeMap<T, Id>,
    /// Maps from IDs back to their associated values
    from_id: BTreeMap<Id, T>,
}

impl<T: Ord + Clone> MappingInner<T> {
    /// Creates a new empty mapping with counter starting at 1.
    pub const fn new() -> Self {
        Self {
            counter: 1,
            to_id: BTreeMap::new(),
            from_id: BTreeMap::new(),
        }
    }

    /// Registers a new value in the mapping and returns its assigned ID.
    pub fn register(&mut self, value: T) -> Id {
        let id = Id(NonZeroI32::new(self.counter).expect("counter should not be zero"));
        self.to_id.insert(value.clone(), id);
        self.from_id.insert(id, value);
        self.counter = self
            .counter
            .checked_add(1)
            .expect("counter should not overflow");
        id
    }

    /// Attempts to find the ID for a given value.
    pub fn try_to_id(&self, value: &T) -> Option<Id> {
        self.to_id.get(value).copied()
    }

    /// Retrieves the data associated with an ID.
    pub fn to_data(&self, id: Id) -> Option<T> {
        self.from_id.get(&id).cloned()
    }

    /// Gets the ID for a value, registering it if not already present.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_id(&mut self, value: T) -> Id {
        self.try_to_id(&value)
            .unwrap_or_else(|| self.register(value))
    }
}

/// A mapping between values and IDs.
///
/// This structure allows for bidirectional lookup between values and their
/// assigned numeric IDs, with interior mutability for shared access.
#[derive(Debug)]
pub struct Mapping<T>(Rc<RefCell<MappingInner<T>>>);

impl<T> Clone for Mapping<T> {
    /// Creates a new reference to the same underlying mapping.
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Ord + Clone> Default for Mapping<T> {
    /// Creates a new empty mapping.
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord + Clone> Mapping<T> {
    /// Creates a new empty mapping.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(MappingInner::new())))
    }

    /// Registers a new value in the mapping and returns its assigned ID.
    pub fn register(&self, value: T) -> Id {
        self.0.borrow_mut().register(value)
    }

    /// Attempts to find the ID for a given value.
    pub fn try_to_id(&self, value: &T) -> Option<Id> {
        self.0.borrow().try_to_id(value)
    }

    /// Gets the ID for a value, registering it if not already present.
    pub fn to_id(&self, value: T) -> Id {
        self.0.borrow_mut().to_id(value)
    }

    /// Retrieves the data associated with an ID.
    #[must_use]
    pub fn to_data(&self, id: Id) -> Option<T> {
        self.0.borrow().to_data(id)
    }

    /// Creates a binding that maps between a value binding and an ID binding.
    ///
    /// This is useful for reactive UI systems where you need to work with IDs rather
    /// than the actual values but still maintain synchronization.
    ///
    /// # Panics
    ///
    /// Panics if the provided `Id` does not correspond to any value in the mapping.
    #[must_use]
    pub fn binding(&self, source: &Binding<T>) -> Binding<Id>
    where
        T: 'static,
    {
        let mapping = self.clone();
        let mapping2 = self.clone();
        Binding::mapping(
            source,
            move |value| mapping.to_id(value),
            move |binding, value| {
                binding.set(
                    mapping2
                        .to_data(value)
                        .expect("Invalid binding mapping : Data not found"),
                );
            },
        )
    }

    /// Creates a binding that maps between an optional value binding and an optional ID binding.
    ///
    /// This is useful for selection-driven controls whose "no selection" state must survive
    /// the type-erased boundary without collapsing to an arbitrary sentinel in Rust space.
    ///
    /// # Panics
    ///
    /// Panics if the provided `Id` does not correspond to any value in the mapping.
    #[must_use]
    pub fn optional_binding(&self, source: &Binding<Option<T>>) -> Binding<Option<Id>>
    where
        T: 'static,
    {
        let mapping = self.clone();
        let mapping2 = self.clone();
        Binding::mapping(
            source,
            move |value| value.map(|value| mapping.to_id(value)),
            move |binding, value| {
                binding.set(value.map(|value| {
                    mapping2
                        .to_data(value)
                        .expect("Invalid optional binding mapping : Data not found")
                }));
            },
        )
    }
}
