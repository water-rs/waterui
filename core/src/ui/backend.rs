//! Static view dispatch: the [`Backend`] a view tree resolves into.
//!
//! [`View::dispatch`] turns a view into a backend's node through
//! monomorphized code. A composite expands its `body` in place; a
//! [`Native`] payload reaches [`Backend::native`] and a [`Metadata`]
//! wrapper [`Backend::metadata`], each with its concrete type, so LLVM
//! inlines across the tree and the linker drops what an application does
//! not reach. Core names no component: the leaf entry is generic, and the
//! backend decides what each payload type becomes.
//!
//! [`AnyView`] is the one place a tree is erased. It reaches
//! [`Backend::erased`], which a backend answers by handing the view a
//! [`DynBackend`] through [`AnyView::dispatch_dyn`]: the erased view's own
//! subtree still expands statically, through the [`Erased`] adapter, and
//! only its native payloads and metadata come back to the backend as
//! `&mut dyn Any`.

use alloc::boxed::Box;
use core::any::Any;

#[cfg(test)]
use crate::components::Native;
use crate::components::{Metadata, NativeView};
use crate::metadata::MetadataKey;
use crate::{AnyView, Environment, View};

/// A target a view tree resolves into: a platform's views, a retained
/// render tree, a probe that reads a fact off the tree.
///
/// `Output` is what one view becomes; `Ctx` is what a parent hands each
/// child it dispatches, such as the slot the child is placed in.
pub trait Backend {
    /// What one view becomes.
    type Output;
    /// What a parent hands each child it dispatches.
    type Ctx;

    /// Realizes a native payload.
    fn native<T: NativeView + 'static>(
        &mut self,
        native: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output;

    /// Realizes a metadata wrapper around its content.
    fn metadata<K: MetadataKey>(
        &mut self,
        metadata: Metadata<K>,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output;

    /// Realizes an erased view, typically through
    /// [`AnyView::dispatch_dyn`].
    fn erased(&mut self, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output;
}

/// The object-safe face of a [`Backend`], for the one hop an [`AnyView`]
/// erases.
///
/// Each payload arrives as `&mut dyn Any` holding an `Option` of its
/// concrete type — `Option<T>` for a native `T`, `Option<Metadata<K>>` for
/// metadata — which the implementation takes after matching the type.
pub trait DynBackend {
    /// Realizes the native payload `native` holds.
    fn native_dyn(
        &mut self,
        native: &mut dyn Any,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any>;

    /// Realizes the metadata wrapper `metadata` holds.
    fn metadata_dyn(
        &mut self,
        metadata: &mut dyn Any,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any>;

    /// Realizes a further erased view.
    fn erased_dyn(&mut self, view: AnyView, env: &Environment, cx: Box<dyn Any>) -> Box<dyn Any>;
}

/// The [`Backend`] an erased view expands through: every leaf it reaches is
/// handed back to the [`DynBackend`] it wraps.
pub struct Erased<'a> {
    inner: &'a mut dyn DynBackend,
}

impl<'a> Erased<'a> {
    /// Wraps `inner`.
    pub const fn new(inner: &'a mut dyn DynBackend) -> Self {
        Self { inner }
    }
}

impl core::fmt::Debug for Erased<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Erased").finish_non_exhaustive()
    }
}

impl Backend for Erased<'_> {
    type Output = Box<dyn Any>;
    type Ctx = Box<dyn Any>;

    fn native<T: NativeView + 'static>(
        &mut self,
        native: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.inner.native_dyn(&mut Some(native), env, cx)
    }

    fn metadata<K: MetadataKey>(
        &mut self,
        metadata: Metadata<K>,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.inner.metadata_dyn(&mut Some(metadata), env, cx)
    }

    fn erased(&mut self, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output {
        self.inner.erased_dyn(view, env, cx)
    }
}

/// Takes the `T` an `Option<T>` behind `slot` holds, when that is its type.
///
/// The counterpart of handing a payload through `&mut dyn Any`: the caller
/// matches the type and takes the value out, leaving `None` behind.
pub fn take<T: 'static>(slot: &mut dyn Any) -> Option<T> {
    slot.downcast_mut::<Option<T>>().and_then(Option::take)
}

/// Converts a payload of type `T` to the same value typed `U`, when `T` and
/// `U` are the same type. The check is a `TypeId` comparison, which folds
/// to a constant after monomorphization.
///
/// # Errors
///
/// Returns `value` unchanged when `T` is not `U`.
///
/// # Panics
///
/// Never: a matching slot holds the value it was just given, and a slot of
/// another type keeps it.
pub fn cast<T: 'static, U: 'static>(value: T) -> Result<U, T> {
    let mut slot = Some(value);
    take::<U>(&mut slot).map_or_else(
        || Err(slot.expect("a slot of another type keeps its value")),
        Ok,
    )
}

/// Dispatches `view` through `backend`.
///
/// The free form of [`View::dispatch`], for code that holds a backend and a
/// view of any type.
pub fn dispatch<V: View, B: Backend>(
    view: V,
    backend: &mut B,
    env: &Environment,
    cx: B::Ctx,
) -> B::Output {
    view.dispatch(backend, env, cx)
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::any::type_name;

    use super::*;

    /// Records the leaves a tree reaches, by type name.
    #[derive(Default)]
    struct Recorder {
        leaves: Vec<&'static str>,
    }

    impl Backend for Recorder {
        type Output = ();
        type Ctx = ();

        fn native<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {
            self.leaves.push(type_name::<T>());
        }

        fn metadata<K: MetadataKey>(&mut self, metadata: Metadata<K>, env: &Environment, (): ()) {
            self.leaves.push(type_name::<K>());
            metadata.content.dispatch(self, env, ());
        }

        fn erased(&mut self, view: AnyView, env: &Environment, (): ()) {
            let output = view.dispatch_dyn(&mut DynRecorder(self), env, Box::new(()));
            assert!(
                output.is::<()>(),
                "the erased path returns the backend's output"
            );
        }
    }

    /// The recorder's object-safe face: every payload it can name.
    struct DynRecorder<'a>(&'a mut Recorder);

    impl DynBackend for DynRecorder<'_> {
        fn native_dyn(
            &mut self,
            native: &mut dyn Any,
            _env: &Environment,
            _cx: Box<dyn Any>,
        ) -> Box<dyn Any> {
            let leaf = if take::<Leaf>(native).is_some() {
                type_name::<Leaf>()
            } else {
                "unknown native"
            };
            self.0.leaves.push(leaf);
            Box::new(())
        }

        fn metadata_dyn(
            &mut self,
            metadata: &mut dyn Any,
            env: &Environment,
            _cx: Box<dyn Any>,
        ) -> Box<dyn Any> {
            let metadata = take::<Metadata<Key>>(metadata).expect("only `Key` is attached");
            self.0.leaves.push(type_name::<Key>());
            metadata.content.dispatch(self.0, env, ());
            Box::new(())
        }

        fn erased_dyn(
            &mut self,
            view: AnyView,
            env: &Environment,
            _cx: Box<dyn Any>,
        ) -> Box<dyn Any> {
            self.0.erased(view, env, ());
            Box::new(())
        }
    }

    struct Leaf;
    impl NativeView for Leaf {}
    impl View for Leaf {
        fn body(self, _env: &Environment) -> impl View {
            Native::new(self)
        }
    }

    struct Key;
    impl MetadataKey for Key {}

    /// A composite: two layers of `body` over a leaf.
    struct Outer;
    impl View for Outer {
        fn body(self, _env: &Environment) -> impl View {
            Inner
        }
    }
    struct Inner;
    impl View for Inner {
        fn body(self, _env: &Environment) -> impl View {
            Metadata::new(Leaf, Key)
        }
    }

    #[test]
    fn composites_expand_to_their_leaves() {
        let mut recorder = Recorder::default();
        Outer.dispatch(&mut recorder, &Environment::new(), ());
        assert_eq!(recorder.leaves, [type_name::<Key>(), type_name::<Leaf>()]);
    }

    #[test]
    fn an_erased_view_expands_through_the_dyn_face() {
        let mut recorder = Recorder::default();
        AnyView::new(Outer).dispatch(&mut recorder, &Environment::new(), ());
        assert_eq!(recorder.leaves, [type_name::<Key>(), type_name::<Leaf>()]);
    }

    #[test]
    fn cast_matches_only_the_same_type() {
        assert_eq!(cast::<u8, u8>(7), Ok(7));
        assert_eq!(cast::<u8, String>(7), Err(7));
    }
}
