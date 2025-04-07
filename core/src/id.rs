//! Identity, tagging, and mapping functionality for UI components.
//!
//! This module provides various utilities for:
//! - Identifying and tagging UI elements with unique identifiers
//! - Creating mappings between values and numeric IDs
//! - Wrapping views with identifying information
//! - Converting between different ID types
//!
//! The primary types in this module include:
//! - `Identifable`: A trait for types that can be uniquely identified
//! - `TaggedView`: A view wrapper that includes an identifying tag
//! - `Mapping`: A bidirectional mapping between values and numeric IDs
//! - `UseId` and `SelfId`: Wrappers that implement different ID strategies

use core::hash::Hash;
use core::num::NonZeroI32;

use crate::{AnyView, View};

/// A non-zero i32 value used for identification purposes throughout the crate.
pub type Id = NonZeroI32;

/// Defines an interface for types that can be uniquely identified.
///
/// Implementors of this trait can provide a specific ID type and a way to retrieve
/// the ID from an instance.
pub trait Identifable {
    /// The type of ID to use, which must implement Hash and Ord traits.
    type Id: Hash + Ord;

    /// Retrieves the unique identifier for this instance.
    fn id(&self) -> Self::Id;
}

/// A wrapper that provides identity to a value through a function.
///
/// This allows attaching identity behavior to any type by providing a function
/// to extract an ID from the wrapped value.
#[derive(Debug)]
pub struct UseId<T, F> {
    /// The wrapped value
    value: T,
    /// Function to extract an ID from the value
    f: F,
}

impl<T, F, Id> Identifable for UseId<T, F>
where
    F: Fn(&T) -> Id,
    Id: Ord + Hash,
{
    type Id = Id;

    /// Applies the stored function to the wrapped value to generate an ID.
    fn id(&self) -> Self::Id {
        (self.f)(&self.value)
    }
}

/// A wrapper that uses the value itself as its own identifier.
///
/// This is useful for types that are already suitable as identifiers.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelfId<T>(T);

impl<T> SelfId<T> {
    /// Creates a new SelfId instance wrapping the given value.
    pub fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T: Hash + Ord + Clone> Identifable for SelfId<T> {
    type Id = T;

    /// Returns a clone of the wrapped value as the identifier.
    fn id(&self) -> Self::Id {
        self.0.clone()
    }
}

/// Extension trait that provides convenient methods for making types identifiable.
pub trait IdentifableExt: Sized {
    /// Wraps the value in a UseId with the provided identification function.
    fn use_id<F>(self, f: F) -> UseId<Self, F>;

    /// Wraps the value in a SelfId, making the value serve as its own identifier.
    fn self_id(self) -> SelfId<Self>;
}

impl<T> IdentifableExt for T {
    fn use_id<F>(self, f: F) -> UseId<Self, F> {
        UseId { value: self, f }
    }

    fn self_id(self) -> SelfId<Self> {
        SelfId(self)
    }
}

/// A view that includes an identifying tag of type T.
///
/// This allows tracking and identification of views within a UI hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedView<T, V> {
    /// The tag used to identify this view
    pub tag: T,
    /// The actual view content
    pub content: V,
}

impl<T, V: View> TaggedView<T, V> {
    /// Creates a new tagged view with the specified tag and content.
    pub fn new(tag: T, content: V) -> Self {
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

    /// Erases the specific view type, converting it to AnyView.
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
use waterui_reactive::Binding;

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
        let id = NonZeroI32::new(self.counter).unwrap();
        self.to_id.insert(value.clone(), id);
        self.from_id.insert(id, value);
        self.counter = self.counter.checked_add(1).unwrap();
        id
    }

    /// Attempts to find the ID for a given value.
    pub fn try_to_id(&self, value: &T) -> Option<Id> {
        self.to_id.get(value).cloned()
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
    pub fn to_data(&self, id: Id) -> Option<T> {
        self.0.borrow().to_data(id)
    }

    /// Creates a binding that maps between a value binding and an ID binding.
    ///
    /// This is useful for reactive UI systems where you need to work with IDs rather
    /// than the actual values but still maintain synchronization.
    pub fn binding(&self, source: Binding<T>) -> Binding<Id>
    where
        T: 'static,
    {
        let mapping = self.clone();
        let mapping2 = self.clone();
        Binding::mapping(
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
