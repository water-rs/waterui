//! This module provides type-erased view implementations to enable
//! heterogeneous collections of views and dynamic dispatch.
//!
//! The main type provided by this module is [`AnyView`], which wraps
//! any type implementing the [`View`] trait and erases its concrete type
//! while preserving its behavior.

use core::fmt;
use core::{
    any::{Any, TypeId, type_name},
    fmt::Debug,
    ptr,
};

use alloc::boxed::Box;
use waterui_meta::mark_reached;

use crate::backend::{Backend, DynBackend, Erased};
use crate::{Environment, View};

trait AnyViewImpl: 'static {
    fn dispatch_dyn(
        self: Box<Self>,
        backend: &mut dyn DynBackend,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any>;
    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }
}

impl<T: View> AnyViewImpl for T {
    fn dispatch_dyn(
        self: Box<Self>,
        backend: &mut dyn DynBackend,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        View::dispatch(*self, &mut Erased::new(backend), env, cx)
    }
}

/// A type-erased wrapper for a `View`.
///
/// This allows storing and passing around different view types uniformly.
///
/// # Contract
///
/// An erased view expands through [`View::dispatch`], which reaches
/// [`Backend::erased`]; `body` is unreachable, and reaching it panics.
#[must_use]
pub struct AnyView(Box<dyn AnyViewImpl>);

impl Debug for AnyView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            return any
                .downcast_mut::<Option<Self>>()
                .expect(
                    "the TypeId check above proved V is AnyView, so the slot is an Option<AnyView>",
                )
                .take()
                .expect("the slot was just filled with Some(view)"); // TODO: use downcast_mut_unchecked when it's stable
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

    /// Expands the erased view through `backend`: its own subtree
    /// dispatches statically, and only its native payloads and metadata
    /// reach `backend`, each as `&mut dyn Any`.
    pub fn dispatch_dyn(
        self,
        backend: &mut dyn DynBackend,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        mark_reached!("erased");
        self.0.dispatch_dyn(backend, env, cx)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn stable_ptr(&self) -> *const () {
        ptr::from_ref::<dyn AnyViewImpl>(&*self.0).cast::<()>()
    }

    /// Downcasts `AnyView` to a concrete view type without any runtime checks.
    ///
    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    #[must_use]
    pub unsafe fn downcast_unchecked<T: 'static>(self) -> Box<T> {
        // SAFETY: the caller contract requires the erased value to be a `T`, so the
        // box being re-typed has the layout it is cast to; ownership moves across
        // unchanged.
        unsafe { Box::from_raw(Box::into_raw(self.0).cast::<T>()) }
    }

    /// Returns a reference to the contained view without any runtime checks.
    ///
    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    #[must_use]
    pub const unsafe fn downcast_ref_unchecked<T: 'static>(&self) -> &T {
        // SAFETY: the caller contract requires the erased value to be a `T`; the
        // borrow is tied to `&self`.
        unsafe { &*(&raw const *self.0).cast::<T>() }
    }

    /// Returns a mutable reference to the contained view without any runtime checks.
    ///
    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    pub const unsafe fn downcast_mut_unchecked<T: 'static>(&mut self) -> &mut T {
        // SAFETY: the caller contract requires the erased value to be a `T`; the
        // borrow is tied to `&mut self`, so it is exclusive.
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
            // SAFETY: the `is::<T>()` check above proves the erased value is a `T`.
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
        // SAFETY: the closure only runs when `is::<T>()` holds, which is exactly the
        // unchecked accessor's requirement.
        unsafe { self.is::<T>().then(|| self.downcast_ref_unchecked()) }
    }

    /// Attempts to get a mutable reference to the contained view of a specific type.
    ///
    /// Returns `Some` if the types match, or `None` if they don't.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        // SAFETY: as above — the closure only runs when the type matches.
        unsafe { self.is::<T>().then(move || self.downcast_mut_unchecked()) }
    }
}

impl View for AnyView {
    fn body(self, _env: &Environment) -> impl View {
        never_expands(self.name());
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.erased(self, env, cx)
    }
}

fn never_expands(name: &'static str) {
    unreachable!(
        "erased view {name} reached `body`: an `AnyView` expands through `View::dispatch`"
    );
}

#[cfg(test)]
mod test {
    use core::any::TypeId;

    use crate::backend::{Backend, peel_native};
    use crate::{Environment, View};

    use super::AnyView;

    #[test]
    pub fn get_type_id() {
        assert_eq!(AnyView::new(()).type_id(), TypeId::of::<()>());
    }

    #[test]
    #[should_panic(expected = "reached `body`")]
    fn an_erased_view_never_expands() {
        let _ = AnyView::new(()).body(&Environment::new());
    }

    struct DispatchedOnly;

    fn body_must_not_run() {
        panic!("erased dispatch must not call body");
    }

    impl View for DispatchedOnly {
        fn body(self, _env: &Environment) -> impl View {
            body_must_not_run();
        }

        fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
            ().dispatch(backend, env, cx)
        }
    }

    #[test]
    fn an_erased_view_dispatches_without_calling_body() {
        let erased = AnyView::new(DispatchedOnly);
        assert_eq!(peel_native::<()>(erased, &Environment::new()), Some(()));
    }
}
