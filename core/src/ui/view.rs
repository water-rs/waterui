//! # View Module
//!
//! This module provides the core abstractions for building user interfaces.
//!
//! The primary types include:
//! - `View`: The fundamental trait for UI components
//! - `ViewSequence`: A trait for sequences of child views (tuples, arrays, `Vec`)
//!
//! These abstractions support a declarative and composable approach to UI building, allowing
//! for flexible combinations of views and transformations.

pub mod hook;

#[cfg(feature = "erased-dispatch")]
use crate::AnyView;
use crate::backend::{Backend, ChildSlot};
use crate::{Environment, OneOf2};
use alloc::vec::Vec;

/// View represents a part of the user interface.
///
/// You can create your custom view by implementing this trait. You just need to implement fit.
///
/// Users can also create a View using a function that returns another View. This allows for more
/// flexible and composable UI designs.
///
/// # Example
///
/// ```rust
/// use waterui_core::View;
///
/// fn greeting() -> impl View {
///     "Hello, World!" // &'static str implements View
/// }
///
#[must_use]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a WaterUI view",
    label = "expected a view",
    note = "Any `'static` type implementing `View::body` is a view, as is a function returning `impl View` or a bare `&'static str`, `String`, or `Str`. A `ForEach` is a collection of views, not a view — hand it to a container such as `VStack::for_each` or `List::for_each`, and unify differing view types with `OneOf2`..`OneOf8`."
)]
pub trait View: 'static {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    fn body(self, _env: &Environment) -> impl View;

    /// Turns this view into `backend`'s node.
    ///
    /// A composite expands its `body` in place, so a tree of composites
    /// resolves through monomorphized code down to the native payloads and
    /// metadata the backend realizes. Native and metadata wrappers override
    /// it to reach the backend; nothing else does.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output
    where
        Self: Sized,
    {
        self.body(env).dispatch(backend, env, cx)
    }
}

impl<F: 'static + FnOnce() -> V, V: View> View for F {
    fn body(self, _env: &Environment) -> impl View {
        self()
    }
}

impl<V: View, E: View> View for Result<V, E> {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Ok(view) => OneOf2::A(view),
            Err(view) => OneOf2::B(view),
        }
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        match self {
            Ok(view) => view.dispatch(backend, env, cx),
            Err(view) => view.dispatch(backend, env, cx),
        }
    }
}

impl<V: View> View for Option<V> {
    fn body(self, _env: &Environment) -> impl View {
        self.map_or_else(|| OneOf2::B(()), OneOf2::A)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        match self {
            Some(view) => view.dispatch(backend, env, cx),
            None => ().dispatch(backend, env, cx),
        }
    }
}

/// The children of a container: a tuple, an array or a `Vec` of views.
///
/// A container keeps its children's own types until it reaches a backend,
/// which receives each child in its [`Backend::Slot`]: a typed child stays
/// statically dispatched, and only an erased backend sees `AnyView`s.
pub trait ViewSequence: 'static {
    /// How many views the sequence holds.
    fn len(&self) -> usize;

    /// Whether the sequence holds no view.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends each view, in order, to `slots` as a slot of type `S`.
    fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>);

    /// The views, in order, each in a slot of type `S`.
    fn into_slots<S: ChildSlot>(self) -> Vec<S>
    where
        Self: Sized,
    {
        let mut slots = Vec::with_capacity(self.len());
        self.push_slots(&mut slots);
        slots
    }
}

fn slot_from_view<S: ChildSlot, V: View>(view: V) -> S {
    #[cfg(feature = "erased-dispatch")]
    {
        S::from_view(AnyView::new(view))
    }
    #[cfg(not(feature = "erased-dispatch"))]
    {
        S::from_view(view)
    }
}

impl<V: View> ViewSequence for Vec<V> {
    fn len(&self) -> usize {
        self.len()
    }

    fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>) {
        slots.extend(self.into_iter().map(slot_from_view));
    }
}

impl<V: View, const N: usize> ViewSequence for [V; N] {
    fn len(&self) -> usize {
        N
    }

    fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>) {
        slots.extend(self.into_iter().map(slot_from_view));
    }
}

macro_rules! impl_tuple_views {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <$($ty:View,)*> ViewSequence for ($($ty,)*){
            fn len(&self) -> usize {
                let views: &[&str] = &[$(stringify!($ty)),*];
                views.len()
            }

            fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>) {
                // The trailing comma matters: `let (T) = self` is a parenthesized
                // binding, not a one-element tuple pattern, so without it a
                // single-child container would hold the tuple itself instead
                // of the view inside it.
                let ($($ty,)*) = self;
                $(slots.push(slot_from_view($ty));)*
            }
        }
    };
}

tuples!(impl_tuple_views);

raw_view!(());

impl<V: View> View for (V,) {
    fn body(self, _env: &Environment) -> impl View {
        self.0
    }
}

#[cfg(feature = "nightly")]
impl View for ! {
    fn body(self, _env: &Environment) -> impl View {}
}
