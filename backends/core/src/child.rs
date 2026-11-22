//! Child views of native payloads, realized through the backend that
//! receives the payload.
//!
//! A payload holding views converts each into its backend's
//! [`Backend::Slot`](waterui_core::backend::Backend::Slot) when it
//! dispatches. Through [`Realize`] the slot is a [`Child`]: the view keeps
//! its own type behind one box, and realizing it later dispatches it
//! statically through the same [`NativeBackend`]. An erased view's payloads
//! carry [`AnyView`] slots instead, which realize through the erased path.

use alloc::boxed::Box;

use waterui_core::backend::ChildSlot;
use waterui_core::{AnyView, Environment, View};

use crate::{NativeBackend, Realize};

/// A [`NativeBackend`] type with its lifetimes left open, so a [`Child`]
/// built before the backend borrows anything can realize through it later.
pub trait NativeBackendFamily: 'static {
    /// What one view becomes.
    type Output: 'static;
    /// What a parent hands each child it realizes.
    type Ctx: 'static;
    /// The backend, borrowing for `'a`.
    type Backend<'a>: NativeBackend<Output = Self::Output, Ctx = Self::Ctx, Family = Self>;

    /// The backend `backend` borrows, for as long as this borrow of it: what
    /// a decorator holds by value while it decorates the backend.
    fn reborrow<'r>(backend: &'r mut Self::Backend<'_>) -> Self::Backend<'r>;
}

/// A payload's child view, held for the [`NativeBackend`] family `F`.
pub struct Child<F: NativeBackendFamily>(Box<dyn ChildFn<F>>);

impl<F: NativeBackendFamily> core::fmt::Debug for Child<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Child").finish_non_exhaustive()
    }
}

impl<F: NativeBackendFamily> ChildSlot for Child<F> {
    fn from_view<V: View>(view: V) -> Self {
        #[cfg(feature = "erased-dispatch")]
        {
            Self(Box::new(ViewChild(AnyView::new(view))))
        }
        #[cfg(not(feature = "erased-dispatch"))]
        {
            Self(Box::new(ViewChild(view)))
        }
    }
}

impl<F: NativeBackendFamily> Child<F> {
    /// A child realized by `realize`, for a child that is not a view of its
    /// own: one held in another family's slot, which `realize` re-enters.
    pub fn from_fn(
        realize: impl for<'a> FnOnce(&mut F::Backend<'a>, &Environment, F::Ctx) -> F::Output + 'static,
    ) -> Self {
        Self(Box::new(FnChild(realize)))
    }
}

struct FnChild<R>(R);

impl<F: NativeBackendFamily, R> ChildFn<F> for FnChild<R>
where
    R: for<'a> FnOnce(&mut F::Backend<'a>, &Environment, F::Ctx) -> F::Output,
{
    fn call(
        self: Box<Self>,
        backend: &mut F::Backend<'_>,
        env: &Environment,
        cx: F::Ctx,
    ) -> F::Output {
        (self.0)(backend, env, cx)
    }
}

/// Realizes the view a [`Child`] holds through a backend of its family.
trait ChildFn<F: NativeBackendFamily> {
    fn call(
        self: Box<Self>,
        backend: &mut F::Backend<'_>,
        env: &Environment,
        cx: F::Ctx,
    ) -> F::Output;
}

struct ViewChild<V>(V);

impl<F: NativeBackendFamily, V: View> ChildFn<F> for ViewChild<V> {
    fn call(
        self: Box<Self>,
        backend: &mut F::Backend<'_>,
        env: &Environment,
        cx: F::Ctx,
    ) -> F::Output {
        Realize::child(backend, self.0, env, cx)
    }
}

/// A child view a payload hands to `B`'s method, realized where the
/// backend places it.
pub trait ChildView<B: NativeBackend>: IntoChild<B::Family> {
    /// Realizes the child through `backend`.
    fn realize(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output;
}

/// A child view held as its backend family's [`Child`], for a backend that
/// realizes it later, outside the method that received it.
///
/// Separate from [`ChildView`] because the conversion names only the
/// family, which borrows nothing: a backend can hand it to code that keeps
/// it.
pub trait IntoChild<F: NativeBackendFamily>: 'static {
    /// The child as the family's [`Child`].
    fn into_child(self) -> Child<F>;
}

impl<B: NativeBackend> ChildView<B> for Child<B::Family> {
    fn realize(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.with_family(|backend| self.0.call(backend, env, cx))
    }
}

impl<F: NativeBackendFamily> IntoChild<F> for Child<F> {
    fn into_child(self) -> Self {
        self
    }
}

impl<B: NativeBackend> ChildView<B> for AnyView {
    fn realize(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        Realize::child(backend, self, env, cx)
    }
}

impl<F: NativeBackendFamily> IntoChild<F> for AnyView {
    fn into_child(self) -> Child<F> {
        Child::from_view(self)
    }
}
