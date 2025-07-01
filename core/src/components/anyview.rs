//! This module provides type-erased view implementations to enable
//! heterogeneous collections of views and dynamic dispatch.
//!
//! The main type provided by this module is [`AnyView`], which wraps
//! any type implementing the [`View`] trait and erases its concrete type
//! while preserving its behavior.
use core::{
    any::{Any, TypeId, type_name},
    fmt::Debug,
};

use alloc::boxed::Box;

use crate::{Environment, View};

trait AnyViewImpl: 'static {
    fn body(self: Box<Self>, env: Environment) -> AnyView;
    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }
}

impl<T: View> AnyViewImpl for T {
    fn body(self: Box<Self>, env: Environment) -> AnyView {
        AnyView::new(View::body(*self, &env))
    }
}

/// A type-erased wrapper for a `View`.
///
/// This allows storing and passing around different view types uniformly.
#[must_use]
pub struct AnyView(Box<dyn AnyViewImpl>);

impl Debug for AnyView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("AnyView({})", self.name()))
    }
}

impl Default for AnyView {
    fn default() -> Self {
        Self::new(())
    }
}

impl AnyView {
    /// Creates a new `AnyView` from any type that implements `View`.
    ///
    /// If the provided view is already an `AnyView`, it will be unwrapped
    /// to avoid unnecessary nesting.
    pub fn new<V: View>(view: V) -> Self {
        #[allow(clippy::missing_panics_doc)]
        if TypeId::of::<V>() == TypeId::of::<Self>() {
            let any = &mut Some(view) as &mut dyn Any;
            return any.downcast_mut::<Option<Self>>().unwrap().take().unwrap(); // TODO: use downcast_mut_unchecked when it's stable
        }

        Self(Box::new(view))
    }

    /// Checks if the contained view is of type `T`.
    #[must_use]
    pub fn is<T: 'static>(&self) -> bool {
        self.type_id() == TypeId::of::<T>()
    }

    /// Returns the `TypeId` of the contained view.
    #[must_use]
    pub fn type_id(&self) -> TypeId {
        AnyViewImpl::type_id(&*self.0)
    }

    /// Returns the type name of the contained view.
    #[must_use]
    pub fn name(&self) -> &'static str {
        AnyViewImpl::name(&*self.0)
    }

    /// Downcasts `AnyView` to a concrete view type without any runtime checks.
    ///
    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    #[must_use]
    pub unsafe fn downcast_unchecked<T: 'static>(self) -> Box<T> {
        unsafe { Box::from_raw(Box::into_raw(self.0).cast::<T>()) }
    }

    /// Returns a reference to the contained view without any runtime checks.
    ///
    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    #[must_use]
    pub const unsafe fn downcast_ref_unchecked<T: 'static>(&self) -> &T {
        unsafe { &*(&raw const *self.0).cast::<T>() }
    }

    /// Returns a mutable reference to the contained view without any runtime checks.
    ///
    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    pub const unsafe fn downcast_mut_unchecked<T: 'static>(&mut self) -> &mut T {
        unsafe { &mut *(&raw mut *self.0).cast::<T>() }
    }

    /// Attempts to downcast `AnyView` to a concrete view type.
    ///
    /// Returns `Ok` with the boxed value if the types match, or
    /// `Err` with the original `AnyView` if the types don't match.
    ///
    /// # Errors
    ///
    /// Returns `Err(Self)` if the contained type does not match `T`.
    pub fn downcast<T: 'static>(self) -> Result<Box<T>, Self> {
        if self.is::<T>() {
            unsafe { Ok(self.downcast_unchecked()) }
        } else {
            Err(self)
        }
    }

    /// Attempts to get a reference to the contained view of a specific type.
    ///
    /// Returns `Some` if the types match, or `None` if they don't.
    #[must_use]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        unsafe { self.is::<T>().then(|| self.downcast_ref_unchecked()) }
    }

    /// Attempts to get a mutable reference to the contained view of a specific type.
    ///
    /// Returns `Some` if the types match, or `None` if they don't.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        unsafe { self.is::<T>().then(move || self.downcast_mut_unchecked()) }
    }
}

impl View for AnyView {
    fn body(self, env: &Environment) -> impl View {
        self.0.body(env.clone())
    }
}

#[cfg(test)]
mod test {
    use core::any::TypeId;

    use super::AnyView;

    #[test]
    pub fn get_type_id() {
        assert_eq!(AnyView::new(()).type_id(), TypeId::of::<()>());
    }
}
