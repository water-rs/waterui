//! Static view dispatch: the [`Backend`] a view tree resolves into.
//!
//! [`View::dispatch`] turns a view into a backend's node through
//! monomorphized code. A composite expands its `body` in place; a
//! [`Native`](crate::Native) payload reaches [`Backend::native`] and a [`Metadata`]
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
use core::marker::PhantomData;

#[cfg(test)]
use crate::components::Native;
use crate::components::{Metadata, NativeView};
use crate::metadata::MetadataKey;
use crate::view::hook::{ComposeErased, ErasedHooks, HookList, HookableView, compose_erased};
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
    /// What a native payload's child views become before the payload reaches
    /// [`Backend::native`]: a payload holding views converts each into this
    /// slot in its [`View::dispatch`], so the backend realizes the child
    /// later, where it places it.
    type Slot: ChildSlot;

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
        metadata: Metadata<K, Self::Slot>,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output;

    /// Realizes an erased view, typically through
    /// [`AnyView::dispatch_dyn`].
    fn erased(&mut self, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output;

    /// Realizes `view` with `hooks` offered every native payload below it
    /// first, the scope a [`Hooks`](crate::view::hook::Hooks) modifier
    /// installs. A backend that realizes nothing — a probe — looks through
    /// the scope to `view`.
    fn scoped<H: HookList, V: View>(
        &mut self,
        hooks: H,
        view: V,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output;

    /// Realizes a [`HookableView`]: a hook in scope that targets it, or else
    /// its own composition.
    fn hookable<T: HookableView>(
        &mut self,
        view: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output;
}

/// A native payload holding child views in a backend's slot `S`, which can
/// move them into another slot.
///
/// A backend that decorates another — a hook scope — hands the payload on
/// with each child in a slot that re-enters the decorator, so the children
/// stay in its scope.
pub trait MapSlots<S>: Sized {
    /// The same payload with its children in the slot `T`.
    type With<T: 'static>;

    /// Moves every child through `f`, now or as the payload produces it.
    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> Self::With<T>;
}

/// A child view of a native payload, held until the backend realizes it.
///
/// A payload generic over its child views converts each into its backend's
/// slot when it dispatches, so the payload type a backend matches on is
/// concrete, while the child's own type is kept wherever the slot keeps it.
pub trait ChildSlot: 'static {
    /// The slot holding `view`.
    fn from_view<V: View>(view: V) -> Self;
}

/// The erased slot: the child dispatches through [`Backend::erased`].
impl ChildSlot for AnyView {
    fn from_view<V: View>(view: V) -> Self {
        Self::new(view)
    }
}

/// The object-safe face of a [`Backend`], for the one hop an [`AnyView`]
/// erases.
///
/// Each payload arrives as an [`ErasedPayload`], which the implementation
/// [`take`](ErasedPayload::take)s after matching the type.
pub trait DynBackend {
    /// Realizes the native payload `native` holds.
    fn native_dyn(
        &mut self,
        native: ErasedPayload<'_>,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any>;

    /// Realizes the metadata wrapper `metadata` holds.
    fn metadata_dyn(
        &mut self,
        metadata: ErasedPayload<'_>,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any>;

    /// Realizes a further erased view.
    fn erased_dyn(&mut self, view: AnyView, env: &Environment, cx: Box<dyn Any>) -> Box<dyn Any>;

    /// Realizes `view` inside the hook scope `hooks`, which an erased
    /// [`Backend::scoped`] handed back.
    fn scoped_dyn(
        &mut self,
        hooks: ErasedHooks,
        view: AnyView,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any>;

    /// Realizes the [`HookableView`] `view` holds, which an erased
    /// [`Backend::hookable`] handed back; `compose` draws it when no hook
    /// claims it.
    fn hookable_dyn(
        &mut self,
        view: ErasedPayload<'_>,
        compose: ComposeErased,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any>;
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
    type Slot = AnyView;

    fn native<T: NativeView + 'static>(
        &mut self,
        native: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.inner
            .native_dyn(ErasedPayload::new(&mut Some(native)), env, cx)
    }

    fn metadata<K: MetadataKey>(
        &mut self,
        metadata: Metadata<K, Self::Slot>,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.inner
            .metadata_dyn(ErasedPayload::new(&mut Some(metadata)), env, cx)
    }

    fn erased(&mut self, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output {
        self.inner.erased_dyn(view, env, cx)
    }

    fn scoped<H: HookList, V: View>(
        &mut self,
        hooks: H,
        view: V,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.inner
            .scoped_dyn(ErasedHooks::new().with(hooks), AnyView::new(view), env, cx)
    }

    fn hookable<T: HookableView>(
        &mut self,
        view: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.inner.hookable_dyn(
            ErasedPayload::new(&mut Some(view)),
            compose_erased::<T>,
            env,
            cx,
        )
    }
}

/// A payload an erased view hands back to its [`DynBackend`]: the value,
/// behind a slot the implementation takes it out of once it matched the
/// type, and the type's name for reporting one it does not realize.
pub struct ErasedPayload<'a> {
    slot: &'a mut dyn Any,
    type_name: &'static str,
}

impl<'a> ErasedPayload<'a> {
    /// The payload `slot` holds.
    pub fn new<T: 'static>(slot: &'a mut Option<T>) -> Self {
        Self {
            slot,
            type_name: core::any::type_name::<T>(),
        }
    }

    /// Takes the payload when it is a `T`, leaving the slot empty.
    pub fn take<T: 'static>(&mut self) -> Option<T> {
        self.slot.downcast_mut::<Option<T>>().and_then(Option::take)
    }

    /// Puts back a payload [`take`](Self::take) took, for the next
    /// receiver to take.
    ///
    /// # Panics
    ///
    /// Panics when `payload` is not of the slot's type.
    pub fn restore<T: 'static>(&mut self, payload: T) {
        let slot = self
            .slot
            .downcast_mut::<Option<T>>()
            .expect("a payload is restored into the slot it was taken from");
        *slot = Some(payload);
    }

    /// The payload type's name.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.type_name
    }
}

impl core::fmt::Debug for ErasedPayload<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ErasedPayload")
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
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
    ErasedPayload::new(&mut slot).take::<U>().map_or_else(
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

/// Reads the first [`Metadata<K>`] wrapper `view` resolves to, with its
/// content erased, or `None` when the first leaf is anything else.
///
/// Composites expand on the way, erased views included, so this sees what a
/// backend sees. It is a probe for code that inspects a tree, such as tests
/// asserting what a modifier emits.
pub fn peel_metadata<K: MetadataKey>(
    view: impl View,
    env: &Environment,
) -> Option<Metadata<K, AnyView>> {
    view.dispatch(&mut Peel::<Metadata<K, AnyView>>(PhantomData), env, ())
}

/// Reads the native payload `T` `view` resolves to, its child views erased,
/// or `None` when the first leaf is anything else. See [`peel_metadata`].
pub fn peel_native<T: NativeView + 'static>(view: impl View, env: &Environment) -> Option<T> {
    view.dispatch(&mut Peel::<T>(PhantomData), env, ())
}

/// The probe behind [`peel_metadata`] and [`peel_native`]: it answers `Some`
/// for the one leaf type `T` and `None` for any other.
struct Peel<T>(PhantomData<fn() -> T>);

impl<T: 'static> Backend for Peel<T> {
    type Output = Option<T>;
    type Ctx = ();
    type Slot = AnyView;

    fn native<N: NativeView + 'static>(
        &mut self,
        native: N,
        _env: &Environment,
        (): (),
    ) -> Option<T> {
        cast(native).ok()
    }

    fn metadata<K: MetadataKey>(
        &mut self,
        metadata: Metadata<K, AnyView>,
        _env: &Environment,
        (): (),
    ) -> Option<T> {
        cast(metadata).ok()
    }

    fn erased(&mut self, view: AnyView, env: &Environment, (): ()) -> Option<T> {
        *view
            .dispatch_dyn(self, env, Box::new(()))
            .downcast::<Option<T>>()
            .expect("the erased path returns this probe's own output")
    }

    fn scoped<H: HookList, V: View>(
        &mut self,
        _hooks: H,
        view: V,
        env: &Environment,
        (): (),
    ) -> Option<T> {
        view.dispatch(self, env, ())
    }

    fn hookable<N: HookableView>(&mut self, view: N, _env: &Environment, (): ()) -> Option<T> {
        cast(view).ok()
    }
}

impl<T: 'static> DynBackend for Peel<T> {
    fn native_dyn(
        &mut self,
        mut native: ErasedPayload<'_>,
        _env: &Environment,
        _cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(native.take::<T>())
    }

    fn metadata_dyn(
        &mut self,
        mut metadata: ErasedPayload<'_>,
        _env: &Environment,
        _cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(metadata.take::<T>())
    }

    fn erased_dyn(&mut self, view: AnyView, env: &Environment, _cx: Box<dyn Any>) -> Box<dyn Any> {
        Box::new(Backend::erased(self, view, env, ()))
    }

    fn scoped_dyn(
        &mut self,
        _hooks: ErasedHooks,
        view: AnyView,
        env: &Environment,
        _cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(Backend::erased(self, view, env, ()))
    }

    fn hookable_dyn(
        &mut self,
        mut view: ErasedPayload<'_>,
        _compose: ComposeErased,
        _env: &Environment,
        _cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(view.take::<T>())
    }
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
        type Slot = AnyView;

        fn native<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {
            self.leaves.push(type_name::<T>());
        }

        fn metadata<K: MetadataKey>(
            &mut self,
            metadata: Metadata<K, AnyView>,
            env: &Environment,
            (): (),
        ) {
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

        fn scoped<H: HookList, V: View>(&mut self, _hooks: H, view: V, env: &Environment, (): ()) {
            view.dispatch(self, env, ());
        }

        fn hookable<T: HookableView>(&mut self, _view: T, _env: &Environment, (): ()) {
            self.leaves.push(type_name::<T>());
        }
    }

    /// The recorder's object-safe face: every payload it can name.
    struct DynRecorder<'a>(&'a mut Recorder);

    impl DynBackend for DynRecorder<'_> {
        fn native_dyn(
            &mut self,
            mut native: ErasedPayload<'_>,
            _env: &Environment,
            _cx: Box<dyn Any>,
        ) -> Box<dyn Any> {
            let leaf = if native.take::<Leaf>().is_some() {
                type_name::<Leaf>()
            } else {
                "unknown native"
            };
            self.0.leaves.push(leaf);
            Box::new(())
        }

        fn metadata_dyn(
            &mut self,
            mut metadata: ErasedPayload<'_>,
            env: &Environment,
            _cx: Box<dyn Any>,
        ) -> Box<dyn Any> {
            let metadata = metadata
                .take::<Metadata<Key, AnyView>>()
                .expect("only `Key` is attached");
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

        fn scoped_dyn(
            &mut self,
            _hooks: ErasedHooks,
            view: AnyView,
            env: &Environment,
            _cx: Box<dyn Any>,
        ) -> Box<dyn Any> {
            self.0.erased(view, env, ());
            Box::new(())
        }

        fn hookable_dyn(
            &mut self,
            view: ErasedPayload<'_>,
            _compose: ComposeErased,
            _env: &Environment,
            _cx: Box<dyn Any>,
        ) -> Box<dyn Any> {
            self.0.leaves.push(view.type_name());
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
