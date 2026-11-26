//! The one trait every mainline backend implements: a method for each
//! native component and each metadata key that needs the platform.
//!
//! The methods have no defaults, so a backend that misses one does not
//! compile. A component behind a facade feature has its method only when
//! the application enabled that feature (`waterui_feature` cfgs, set by
//! this crate's build script from the facade's `DEP_WATERUI_FEATURES`).
//!
//! A backend implements [`NativeBackend`] and realizes a tree through
//! [`Realize`], the core [`Backend`] that routes each payload to its
//! method. The routing compares `TypeId`s, which fold to constants after
//! monomorphization: a payload type reaches exactly one method through a
//! direct call, and a method no reached payload names is never
//! instantiated. An [`AnyView`] is the one place a tree erases; its leaves
//! come back through the same routing, by `TypeId` at run time.

use alloc::boxed::Box;
#[cfg(waterui_feature = "capture")]
use alloc::rc::Rc;
use core::any::Any;

#[cfg(waterui_feature = "capture")]
use waterui::ViewRenderer;
#[cfg(waterui_feature = "capture")]
use waterui::view_renderer::CaptureBinder;

use waterui_core::animation::FrameRateVote;
use waterui_core::backend::{Backend, DynBackend, ErasedPayload, MapSlots, cast};
use waterui_core::metadata::MetadataKey;
use waterui_core::view::hook::{ComposeErased, ErasedHooks, HookList, HookableView};
use waterui_core::{AnyView, Environment, Metadata, NativeView, View, contract_violation};
use waterui_meta::mark_reached;

use crate::child::{Child, ChildView, NativeBackendFamily};
use crate::hooked::{ErasedScope, Hooked, HookedFamily};

/// Declares [`NativeBackend`] and both routers from one list, so the trait
/// and the routing never disagree.
macro_rules! native_backend {
    (
        natives {
            $( $(#[cfg($ncfg:meta)])* $(#[doc = $ndoc:literal])* $native:ident: $native_ty:ty, )*
        }
        parents {
            $( $(#[cfg($pcfg:meta)])* $(#[doc = $pdoc:literal])* $parent:ident<$child:ident>: $parent_ty:ty, )*
        }
        metadata {
            $( $(#[cfg($mcfg:meta)])* $(#[doc = $mdoc:literal])* $meta:ident: $meta_ty:ty, )*
        }
        parent_metadata {
            $( $(#[cfg($pmcfg:meta)])* $(#[doc = $pmdoc:literal])* $pmeta:ident<$pmchild:ident>: $pmeta_ty:ty, )*
        }
    ) => {
        /// A backend's realization of every native component and every
        /// metadata key that needs the platform.
        ///
        /// Each `natives` method receives its payload by value; each
        /// metadata method receives the wrapper with its content, which it
        /// realizes through [`Realize::child`].
        pub trait NativeBackend: Sized {
            /// What one view becomes.
            type Output: 'static;
            /// What a parent hands each child it realizes.
            type Ctx: 'static;
            /// This backend type with its lifetimes left open, for the
            /// [`Child`] views of the payloads it receives.
            type Family: NativeBackendFamily<Output = Self::Output, Ctx = Self::Ctx>;
            #[cfg(waterui_feature = "webview")]
            /// The compile-time selected [`waterui::webview::WebViewController`]
            /// used to realize [`waterui::webview::WebViewOpen`].
            type WebViewController: waterui::webview::WebViewController;

            /// The backend's offscreen view-to-RGBA renderer.
            #[cfg(waterui_feature = "capture")]
            type Offscreen: ViewRenderer;

            /// Runs `f` on this backend as its family names it. Every
            /// implementation is `f(self)`; the signature is what ties the
            /// backend to its family.
            fn with_family<R>(
                &mut self,
                f: impl for<'a> FnOnce(&mut <Self::Family as NativeBackendFamily>::Backend<'a>) -> R,
            ) -> R;

            #[cfg(waterui_feature = "webview")]
            /// Returns the backend's [`WebViewController`](Self::WebViewController).
            fn web_view_controller(&self) -> Self::WebViewController;

            /// Creates an offscreen renderer configured from `env`.
            #[cfg(waterui_feature = "capture")]
            fn offscreen(&self, env: &Environment) -> Self::Offscreen;

            /// Wires a [`CaptureBinder`] to [`Self::offscreen`] and realizes
            /// its wrapped content.
            #[cfg(waterui_feature = "capture")]
            #[doc(hidden)]
            fn capture_binder<S: ChildView<Self>, C: ChildView<Self>>(
                &mut self,
                metadata: Metadata<CaptureBinder<S>, C>,
                env: &Environment,
                cx: Self::Ctx,
            ) -> Self::Output {
                mark_reached!("capture_binder");
                let Metadata { content, value } = metadata;
                let offscreen = Rc::new(self.offscreen(env));
                let value: CaptureBinder<Child<Self::Family>> =
                    MapSlots::map_slots(value, S::into_child);
                value.bind(Rc::new(
                    move |slot: Child<Self::Family>, render_env: &Environment, request| {
                        let offscreen = Rc::clone(&offscreen);
                        let render_env = render_env.clone();
                        Box::pin(async move {
                            offscreen
                                .render_to_rgba($crate::hooked::SlotView::new(slot), &render_env, request)
                                .await
                        })
                    },
                ));
                content.realize(self, env, cx)
            }

            $(
                $(#[cfg($ncfg)])*
                $(#[doc = $ndoc])*
                fn $native(&mut self, native: $native_ty, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            $(
                $(#[cfg($pcfg)])*
                $(#[doc = $pdoc])*
                fn $parent<$child: ChildView<Self>>(&mut self, native: $parent_ty, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            $(
                $(#[cfg($mcfg)])*
                $(#[doc = $mdoc])*
                fn $meta<C: ChildView<Self>>(&mut self, metadata: Metadata<$meta_ty, C>, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            $(
                $(#[cfg($pmcfg)])*
                $(#[doc = $pmdoc])*
                fn $pmeta<$pmchild: ChildView<Self>, C: ChildView<Self>>(&mut self, metadata: Metadata<$pmeta_ty, C>, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            /// Realizes a native payload no method names: a component this
            /// backend provides itself, which it matches by type, or a payload
            /// it does not realize, which it reports through
            /// [`report_unprovided`] before returning an empty view's output.
            fn provided<T: NativeView + 'static>(&mut self, native: T, env: &Environment, cx: Self::Ctx) -> Self::Output;

            /// [`provided`](Self::provided) for a payload an [`AnyView`]
            /// handed back erased: the backend [`take`](ErasedPayload::take)s its
            /// own types out of `native`.
            fn provided_dyn(&mut self, native: ErasedPayload<'_>, env: &Environment, cx: Self::Ctx) -> Self::Output;

            /// Realizes a metadata wrapper no method names: a key this backend
            /// provides itself, or one it reports through
            /// [`report_unprovided`].
            fn provided_metadata<K: MetadataKey, C: ChildView<Self>>(&mut self, metadata: Metadata<K, C>, env: &Environment, cx: Self::Ctx) -> Self::Output;

            /// [`provided_metadata`](Self::provided_metadata) for a wrapper an
            /// [`AnyView`] handed back erased.
            fn provided_metadata_dyn(&mut self, metadata: ErasedPayload<'_>, env: &Environment, cx: Self::Ctx) -> Self::Output;

            /// Realizes `view` inside the hook scope `hooks` an erased
            /// [`Backend::scoped`] handed back. A backend realizes it under a
            /// [`Hooked`] decorator holding the scope; a decorator extends its
            /// own.
            #[doc(hidden)]
            fn scoped_erased(&mut self, hooks: &ErasedHooks, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output {
                self.with_family(|backend| {
                    let mut hooked = Hooked::with_scope(
                        <Self::Family as NativeBackendFamily>::reborrow(backend),
                        (),
                        ErasedHooks::new().nest(hooks),
                    );
                    Realize::child(&mut hooked, view, env, cx)
                })
            }

            /// Realizes a [`HookableView`] no hook claimed: its own
            /// composition.
            #[doc(hidden)]
            fn hookable<T: HookableView>(&mut self, view: T, env: &Environment, cx: Self::Ctx) -> Self::Output {
                let view = view.compose(env);
                Realize::child(self, view, env, cx)
            }

            /// [`hookable`](Self::hookable) for a payload an [`AnyView`]
            /// handed back erased, which `compose` draws.
            #[doc(hidden)]
            fn hookable_dyn(&mut self, mut view: ErasedPayload<'_>, compose: ComposeErased, env: &Environment, cx: Self::Ctx) -> Self::Output {
                let view = compose(&mut view, env);
                Realize::child(self, view, env, cx)
            }
        }

        impl<B: NativeBackend, H: HookList, E: ErasedScope> NativeBackend for Hooked<B, H, E> {
            type Output = B::Output;
            type Ctx = B::Ctx;
            type Family = HookedFamily<B::Family, H, E>;
            #[cfg(waterui_feature = "webview")]
            type WebViewController = B::WebViewController;
            #[cfg(waterui_feature = "capture")]
            type Offscreen = B::Offscreen;

            fn with_family<R>(
                &mut self,
                f: impl for<'a> FnOnce(&mut <Self::Family as NativeBackendFamily>::Backend<'a>) -> R,
            ) -> R {
                // The decorator as its family names it: the same scope over
                // the inner backend as the inner family names that.
                let (hooks, erased) = self.scope();
                self.inner().with_family(|backend| {
                    f(&mut Hooked::with_scope(
                        <B::Family as NativeBackendFamily>::reborrow(backend),
                        hooks,
                        erased,
                    ))
                })
            }

            #[cfg(waterui_feature = "webview")]
            fn web_view_controller(&self) -> Self::WebViewController {
                self.inner_ref().web_view_controller()
            }

            #[cfg(waterui_feature = "capture")]
            fn offscreen(&self, env: &Environment) -> Self::Offscreen {
                self.inner_ref().offscreen(env)
            }

            #[cfg(waterui_feature = "capture")]
            fn capture_binder<S: ChildView<Self>, C: ChildView<Self>>(
                &mut self,
                metadata: Metadata<CaptureBinder<S>, C>,
                env: &Environment,
                cx: Self::Ctx,
            ) -> Self::Output {
                let Metadata { content, value } = metadata;
                let metadata = Metadata::new(
                    self.rehook(content),
                    MapSlots::map_slots(value, self.rehooker::<S>()),
                );
                self.inner().capture_binder(metadata, env, cx)
            }

            $(
                $(#[cfg($ncfg)])*
                fn $native(&mut self, native: $native_ty, env: &Environment, cx: Self::Ctx) -> Self::Output {
                    match self.offer(native, env, cx) {
                        Ok(output) => output,
                        Err((native, cx)) => self.inner().$native(native, env, cx),
                    }
                }
            )*

            $(
                $(#[cfg($pcfg)])*
                fn $parent<$child: ChildView<Self>>(&mut self, native: $parent_ty, env: &Environment, cx: Self::Ctx) -> Self::Output {
                    let native = match cast::<$parent_ty, waterui::component::button::ButtonConfig<$child>>(native) {
                        Ok(config) => return self.hook_button(config, env, cx),
                        Err(native) => native,
                    };
                    #[cfg(waterui_feature = "toggle")]
                    let native = match cast::<$parent_ty, waterui::component::toggle::ToggleConfig<$child>>(native) {
                        Ok(config) => return self.hook_toggle(config, env, cx),
                        Err(native) => native,
                    };
                    let native = MapSlots::map_slots(native, self.rehooker::<$child>());
                    self.inner().$parent(native, env, cx)
                }
            )*

            $(
                $(#[cfg($mcfg)])*
                fn $meta<C: ChildView<Self>>(&mut self, metadata: Metadata<$meta_ty, C>, env: &Environment, cx: Self::Ctx) -> Self::Output {
                    let metadata = metadata.map_content(|content| self.rehook(content));
                    self.inner().$meta(metadata, env, cx)
                }
            )*

            $(
                $(#[cfg($pmcfg)])*
                fn $pmeta<$pmchild: ChildView<Self>, C: ChildView<Self>>(&mut self, metadata: Metadata<$pmeta_ty, C>, env: &Environment, cx: Self::Ctx) -> Self::Output {
                    let Metadata { content, value } = metadata;
                    let metadata = Metadata::new(
                        self.rehook(content),
                        MapSlots::map_slots(value, self.rehooker::<$pmchild>()),
                    );
                    self.inner().$pmeta(metadata, env, cx)
                }
            )*

            fn provided<T: NativeView + 'static>(&mut self, native: T, env: &Environment, cx: Self::Ctx) -> Self::Output {
                let native = match cast::<T, $crate::hooked::SlotView<B::Family>>(native) {
                    Ok(slot) => return slot.into_child().realize(self.inner(), env, cx),
                    Err(native) => native,
                };
                let native = match cast::<T, waterui::component::StyleSlot>(native) {
                    Ok(slot) => match slot.downcast::<$crate::Child<B::Family>>() {
                        Ok(child) => return child.realize(self.inner(), env, cx),
                        Err(slot) => return self.inner().provided(slot, env, cx),
                    },
                    Err(native) => native,
                };
                match self.offer(native, env, cx) {
                    Ok(output) => output,
                    Err((native, cx)) => self.inner().provided(native, env, cx),
                }
            }

            fn provided_dyn(&mut self, mut native: ErasedPayload<'_>, env: &Environment, cx: Self::Ctx) -> Self::Output {
                if let Some(slot) = native.take::<$crate::hooked::SlotView<B::Family>>() {
                    return slot.into_child().realize(self.inner(), env, cx);
                }
                if let Some(slot) = native.take::<waterui::component::StyleSlot>() {
                    return match slot.downcast::<$crate::Child<B::Family>>() {
                        Ok(child) => child.realize(self.inner(), env, cx),
                        Err(slot) => self.inner().provided(slot, env, cx),
                    };
                }
                match self.offer_erased(&mut native, env, cx) {
                    Ok(output) => output,
                    Err(cx) => self.inner().provided_dyn(native, env, cx),
                }
            }

            fn provided_metadata<K: MetadataKey, C: ChildView<Self>>(&mut self, metadata: Metadata<K, C>, env: &Environment, cx: Self::Ctx) -> Self::Output {
                let metadata = metadata.map_content(|content| self.rehook(content));
                self.inner().provided_metadata(metadata, env, cx)
            }

            fn provided_metadata_dyn(&mut self, metadata: ErasedPayload<'_>, env: &Environment, cx: Self::Ctx) -> Self::Output {
                self.inner().provided_metadata_dyn(metadata, env, cx)
            }

            fn scoped_erased(&mut self, hooks: &ErasedHooks, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output {
                Hooked::scoped_erased(self, hooks, view, env, cx)
            }

            fn hookable<T: HookableView>(&mut self, view: T, env: &Environment, cx: Self::Ctx) -> Self::Output {
                match self.offer(view, env, cx) {
                    Ok(output) => output,
                    Err((view, cx)) => self.inner().hookable(view, env, cx),
                }
            }

            fn hookable_dyn(&mut self, mut view: ErasedPayload<'_>, compose: ComposeErased, env: &Environment, cx: Self::Ctx) -> Self::Output {
                match self.offer_erased(&mut view, env, cx) {
                    Ok(output) => output,
                    Err(cx) => self.inner().hookable_dyn(view, compose, env, cx),
                }
            }
        }

        fn route_native<B: NativeBackend, T: NativeView + 'static>(
            backend: &mut B,
            native: T,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            let native = match cast::<T, $crate::hooked::SlotView<B::Family>>(native) {
                Ok(slot) => return slot.into_child().realize(backend, env, cx),
                Err(native) => native,
            };
            let native = match cast::<T, waterui::component::StyleSlot>(native) {
                Ok(slot) => match slot.downcast::<$crate::Child<B::Family>>() {
                    Ok(child) => return child.realize(backend, env, cx),
                    Err(slot) => return backend.provided(slot, env, cx),
                },
                Err(native) => native,
            };
            $(
                $(#[cfg($ncfg)])*
                let native = match cast::<T, $native_ty>(native) {
                    Ok(native) => return backend.$native(native, env, cx),
                    Err(native) => native,
                };
            )*
            $(
                $(#[cfg($pcfg)])*
                let native = {
                    // The payload with its children in the slot `$child`.
                    fn route<B: NativeBackend, $child: ChildView<B>, T: 'static>(
                        backend: &mut B,
                        native: T,
                        env: &Environment,
                        cx: B::Ctx,
                    ) -> Result<B::Output, (T, B::Ctx)> {
                        match cast::<T, $parent_ty>(native) {
                            Ok(native) => Ok(backend.$parent(native, env, cx)),
                            Err(native) => Err((native, cx)),
                        }
                    }
                    let (native, cx_) = match route::<B, Child<B::Family>, T>(backend, native, env, cx) {
                        Ok(output) => return output,
                        Err(unmatched) => unmatched,
                    };
                    match route::<B, AnyView, T>(backend, native, env, cx_) {
                        Ok(output) => return output,
                        Err(unmatched) => unmatched,
                    }
                };
                $(#[cfg($pcfg)])*
                let (native, cx) = native;
            )*
            backend.provided(native, env, cx)
        }

        fn route_metadata<B: NativeBackend, K: MetadataKey>(
            backend: &mut B,
            metadata: Metadata<K, Child<B::Family>>,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($mcfg)])*
                let metadata = match cast::<Metadata<K, Child<B::Family>>, Metadata<$meta_ty, Child<B::Family>>>(metadata) {
                    Ok(metadata) => return backend.$meta(metadata, env, cx),
                    Err(metadata) => metadata,
                };
            )*
            $(
                $(#[cfg($pmcfg)])*
                let metadata = {
                    // The metadata key with its children in the slot `$pmchild`.
                    fn route<B: NativeBackend, $pmchild: ChildView<B>, K: MetadataKey>(
                        backend: &mut B,
                        metadata: Metadata<K, Child<B::Family>>,
                        env: &Environment,
                        cx: B::Ctx,
                    ) -> Result<B::Output, (Metadata<K, Child<B::Family>>, B::Ctx)>
                    where
                        $pmeta_ty: MetadataKey,
                    {
                        match cast::<Metadata<K, Child<B::Family>>, Metadata<$pmeta_ty, Child<B::Family>>>(metadata) {
                            Ok(metadata) => Ok(backend.$pmeta(metadata, env, cx)),
                            Err(metadata) => Err((metadata, cx)),
                        }
                    }
                    let (metadata, cx_) = match route::<B, Child<B::Family>, K>(backend, metadata, env, cx) {
                        Ok(output) => return output,
                        Err(unmatched) => unmatched,
                    };
                    match route::<B, AnyView, K>(backend, metadata, env, cx_) {
                        Ok(output) => return output,
                        Err(unmatched) => unmatched,
                    }
                };
                $(#[cfg($pmcfg)])*
                let (metadata, cx) = metadata;
            )*
            #[cfg(waterui_feature = "capture")]
            let (metadata, cx) = {
                fn route_capture<B: NativeBackend, C: ChildView<B>, K: MetadataKey>(
                    backend: &mut B,
                    metadata: Metadata<K, Child<B::Family>>,
                    env: &Environment,
                    cx: B::Ctx,
                ) -> Result<B::Output, (Metadata<K, Child<B::Family>>, B::Ctx)> {
                    match cast::<
                        Metadata<K, Child<B::Family>>,
                        Metadata<CaptureBinder<C>, Child<B::Family>>,
                    >(metadata) {
                        Ok(metadata) => Ok(backend.capture_binder(metadata, env, cx)),
                        Err(metadata) => Err((metadata, cx)),
                    }
                }
                let (metadata, cx_) =
                    match route_capture::<B, Child<B::Family>, K>(backend, metadata, env, cx) {
                        Ok(output) => return output,
                        Err(unmatched) => unmatched,
                    };
                match route_capture::<B, AnyView, K>(backend, metadata, env, cx_) {
                    Ok(output) => return output,
                    Err(unmatched) => unmatched,
                }
            };
            backend.provided_metadata(metadata, env, cx)
        }

        fn route_native_dyn<B: NativeBackend>(
            backend: &mut B,
            mut native: ErasedPayload<'_>,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            if let Some(slot) = native.take::<$crate::hooked::SlotView<B::Family>>() {
                return slot.into_child().realize(backend, env, cx);
            }
            if let Some(slot) = native.take::<waterui::component::StyleSlot>() {
                return match slot.downcast::<$crate::Child<B::Family>>() {
                    Ok(child) => child.realize(backend, env, cx),
                    Err(slot) => backend.provided(slot, env, cx),
                };
            }
            $(
                $(#[cfg($ncfg)])*
                if let Some(native) = native.take::<$native_ty>() {
                    return backend.$native(native, env, cx);
                }
            )*
            $(
                $(#[cfg($pcfg)])*
                {
                    // The payload with its children in `Child<B::Family>` or `AnyView`.
                    fn take<$child: 'static>(native: &mut ErasedPayload<'_>) -> Option<$parent_ty> {
                        native.take::<$parent_ty>()
                    }
                    if let Some(native) = take::<Child<B::Family>>(&mut native) {
                        return backend.$parent(native, env, cx);
                    }
                    if let Some(native) = take::<AnyView>(&mut native) {
                        return backend.$parent(native, env, cx);
                    }
                }
            )*
            backend.provided_dyn(native, env, cx)
        }

        fn route_metadata_dyn<B: NativeBackend>(
            backend: &mut B,
            mut metadata: ErasedPayload<'_>,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($mcfg)])*
                if let Some(metadata) = metadata.take::<Metadata<$meta_ty, AnyView>>() {
                    return backend.$meta(metadata, env, cx);
                }
            )*
            $(
                $(#[cfg($mcfg)])*
                if let Some(metadata) = metadata.take::<Metadata<$meta_ty, Child<B::Family>>>() {
                    return backend.$meta(metadata, env, cx);
                }
            )*
            $(
                $(#[cfg($pmcfg)])*
                {
                    // The metadata key with its children and content in the erased or family slot.
                    fn take<$pmchild: 'static, C: 'static>(
                        metadata: &mut ErasedPayload<'_>,
                    ) -> Option<Metadata<$pmeta_ty, C>>
                    where
                        $pmeta_ty: MetadataKey,
                    {
                        metadata.take::<Metadata<$pmeta_ty, C>>()
                    }
                    if let Some(metadata) = take::<Child<B::Family>, Child<B::Family>>(&mut metadata) {
                        return backend.$pmeta(metadata, env, cx);
                    }
                    if let Some(metadata) = take::<AnyView, AnyView>(&mut metadata) {
                        return backend.$pmeta(metadata, env, cx);
                    }
                }
            )*
            #[cfg(waterui_feature = "capture")]
            if let Some(metadata) = metadata.take::<Metadata<CaptureBinder<Child<B::Family>>, Child<B::Family>>>() {
                return backend.capture_binder(metadata, env, cx);
            }
            #[cfg(waterui_feature = "capture")]
            if let Some(metadata) = metadata.take::<Metadata<CaptureBinder<AnyView>, AnyView>>() {
                return backend.capture_binder(metadata, env, cx);
            }
            backend.provided_metadata_dyn(metadata, env, cx)
        }

        #[cfg(test)]
        #[cfg(test)]
        impl NativeBackendFamily for tests::Recorder {
            type Output = ();
            type Ctx = ();
            type Backend<'a> = Self;

            fn reborrow(backend: &mut Self) -> Self {
                backend.clone()
            }
        }

        #[cfg(test)]
        impl NativeBackend for tests::Recorder {
            type Output = ();
            type Ctx = ();
            type Family = Self;
            #[cfg(waterui_feature = "webview")]
            type WebViewController = waterui::webview::UnavailableWebViewController;
            #[cfg(waterui_feature = "capture")]
            type Offscreen = tests::RecorderOffscreen;

            fn with_family<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
                f(self)
            }

            #[cfg(waterui_feature = "webview")]
            fn web_view_controller(&self) -> Self::WebViewController {
                waterui::webview::UnavailableWebViewController
            }

            #[cfg(waterui_feature = "capture")]
            fn offscreen(&self, _env: &Environment) -> Self::Offscreen {
                tests::RecorderOffscreen(self.clone())
            }

            $(
                $(#[cfg($ncfg)])*
                fn $native(&mut self, _native: $native_ty, _env: &Environment, (): ()) {
                    self.reached.borrow_mut().push(stringify!($native));
                }
            )*

            $(
                $(#[cfg($pcfg)])*
                fn $parent<$child: ChildView<Self>>(&mut self, native: $parent_ty, env: &Environment, (): ()) {
                    self.reached.borrow_mut().push(stringify!($parent));
                    tests::Recorder::realize_children(self, tests::Parent::children(native), env);
                }
            )*

            $(
                $(#[cfg($mcfg)])*
                fn $meta<C: ChildView<Self>>(&mut self, metadata: Metadata<$meta_ty, C>, env: &Environment, (): ()) {
                    self.reached.borrow_mut().push(stringify!($meta));
                    metadata.content.realize(self, env, ());
                }
            )*

            $(
                $(#[cfg($pmcfg)])*
                fn $pmeta<$pmchild: ChildView<Self>, C: ChildView<Self>>(&mut self, metadata: Metadata<$pmeta_ty, C>, env: &Environment, (): ()) {
                    self.reached.borrow_mut().push(stringify!($pmeta));
                    metadata.content.realize(self, env, ());
                    tests::Recorder::realize_children(self, tests::Parent::children(metadata.value), env);
                }
            )*

            fn provided<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {
                self.reached.borrow_mut().push(core::any::type_name::<T>());
            }

            fn provided_dyn(&mut self, native: ErasedPayload<'_>, _env: &Environment, (): ()) {
                self.reached.borrow_mut().push(native.type_name());
            }

            fn provided_metadata<K: MetadataKey, C: ChildView<Self>>(&mut self, metadata: Metadata<K, C>, env: &Environment, (): ()) {
                self.reached.borrow_mut().push(core::any::type_name::<K>());
                metadata.content.realize(self, env, ());
            }

            fn provided_metadata_dyn(&mut self, metadata: ErasedPayload<'_>, _env: &Environment, (): ()) {
                self.reached.borrow_mut().push(metadata.type_name());
            }
        }
    };
}

/// Reports that `payload` reached `backend`, which neither `NativeBackend`
/// nor the backend's own components realize: a contract violation.
pub fn report_unprovided(backend: &str, payload: &str) {
    contract_violation!(
        "{payload} reached {backend}, which realizes it neither through a `NativeBackend` method nor as a component it provides"
    );
}

/// How many erased views may expand into one another before any payload is
/// reached. Inherited from the recursion budget of Hydrolysis's former layout
/// normalization, the pass this check replaces; a finite composition never
/// comes near it, so only a body that erases itself again trips it.
const ERASED_HOP_BUDGET: usize = 64;

/// The core [`Backend`] a [`NativeBackend`] realizes a tree through.
#[derive(Debug)]
pub struct Realize<'a, B> {
    backend: &'a mut B,
    /// Erased views expanded since the last payload this realization reached.
    erased_hops: usize,
}

impl<'a, B: NativeBackend> Realize<'a, B> {
    /// Wraps `backend`.
    pub const fn new(backend: &'a mut B) -> Self {
        Self {
            backend,
            erased_hops: 0,
        }
    }

    /// Realizes `view` through `backend`.
    pub fn child<V: View>(backend: &mut B, view: V, env: &Environment, cx: B::Ctx) -> B::Output {
        waterui_core::backend::dispatch(view, &mut Realize::new(backend), env, cx)
    }
}

impl<B: NativeBackend> Backend for Realize<'_, B> {
    type Output = B::Output;
    type Ctx = B::Ctx;
    type Slot = Child<B::Family>;

    fn native<T: NativeView + 'static>(
        &mut self,
        native: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        route_native(self.backend, native, env, cx)
    }

    fn metadata<K: MetadataKey>(
        &mut self,
        metadata: Metadata<K, Self::Slot>,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        route_metadata(self.backend, metadata, env, cx)
    }

    fn scoped<H: HookList, V: View>(
        &mut self,
        hooks: H,
        view: V,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.backend.with_family(|backend| {
            let mut hooked =
                Hooked::new(<B::Family as NativeBackendFamily>::reborrow(backend), hooks);
            Realize::child(&mut hooked, view, env, cx)
        })
    }

    fn hookable<T: HookableView>(
        &mut self,
        view: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        self.backend.hookable(view, env, cx)
    }

    fn erased(&mut self, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output {
        mark_reached!("erased");
        if self.erased_hops == ERASED_HOP_BUDGET {
            contract_violation!(
                "{} expanded through {ERASED_HOP_BUDGET} erased views without reaching a payload; its body erases itself, so it realizes as an empty view",
                view.name()
            );
            return route_native(self.backend, (), env, cx);
        }
        let mut erased = RealizeDyn {
            backend: self.backend,
            erased_hops: self.erased_hops + 1,
        };
        let output = view.dispatch_dyn(&mut erased, env, Box::new(cx));
        *output
            .downcast::<B::Output>()
            .expect("the erased path returns the backend's own output")
    }
}

/// The object-safe face of a [`NativeBackend`], for the leaves of an
/// [`AnyView`].
struct RealizeDyn<'a, B> {
    backend: &'a mut B,
    erased_hops: usize,
}

impl<B: NativeBackend> RealizeDyn<'_, B> {
    fn cx(cx: Box<dyn Any>) -> B::Ctx {
        *cx.downcast::<B::Ctx>()
            .expect("the erased path carries the backend's own context")
    }
}

impl<B: NativeBackend> DynBackend for RealizeDyn<'_, B> {
    fn native_dyn(
        &mut self,
        native: ErasedPayload<'_>,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(route_native_dyn(self.backend, native, env, Self::cx(cx)))
    }

    fn metadata_dyn(
        &mut self,
        metadata: ErasedPayload<'_>,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(route_metadata_dyn(
            self.backend,
            metadata,
            env,
            Self::cx(cx),
        ))
    }

    fn erased_dyn(&mut self, view: AnyView, env: &Environment, cx: Box<dyn Any>) -> Box<dyn Any> {
        let mut realize = Realize {
            backend: &mut *self.backend,
            erased_hops: self.erased_hops,
        };
        Box::new(realize.erased(view, env, Self::cx(cx)))
    }

    fn scoped_dyn(
        &mut self,
        hooks: ErasedHooks,
        view: AnyView,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(self.backend.scoped_erased(&hooks, view, env, Self::cx(cx)))
    }

    fn hookable_dyn(
        &mut self,
        view: ErasedPayload<'_>,
        compose: ComposeErased,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(self.backend.hookable_dyn(view, compose, env, Self::cx(cx)))
    }
}

native_backend! {
    natives {
        /// The empty view.
        empty: (),
        /// A run of styled text.
        text: waterui::text::TextConfig,
        /// A plain string label.
        label: waterui_core::Str,
        /// A solid color fill.
        color: waterui::Color,
        /// A flexible gap in a stack.
        spacer: waterui::layout::Spacer,
        /// A line separating content.
        divider: waterui::layout::Divider,
        /// A resolved shape.
        shape: waterui::shape::ResolvedShape,
        /// A gradient fill.
        gradient: waterui::graphics::Gradient,
        #[cfg(any(waterui_feature = "gpu", waterui_feature = "cpu"))]
        /// A WGSL fragment shader fill.
        shader_paint: waterui::graphics::ShaderPaintView,
        #[cfg(waterui_feature = "canvas")]
        /// A canvas.
        canvas: waterui::graphics::canvas::CanvasView,
        #[cfg(waterui_feature = "gpu")]
        /// GPU content.
        gpu: waterui::graphics::backend::GpuContentLeaf,
        #[cfg(waterui_feature = "gpu")]
        /// Frames from an external source.
        external_frame: waterui::graphics::gpu::external::ExternalFrameView,
        #[cfg(waterui_feature = "gpu")]
        /// A retained scene.
        scene: waterui::graphics::backend::SceneView,
        #[cfg(waterui_feature = "gpu")]
        /// A recorded picture.
        picture: waterui::graphics::backend::Picture,
        #[cfg(waterui_feature = "video")]
        /// A video without controls.
        video: waterui::video::video::NativeVideoConfig,
        #[cfg(waterui_feature = "video")]
        /// A video player.
        video_player: waterui::video::video::NativeVideoPlayerConfig,
        #[cfg(waterui_feature = "webview")]
        /// A web view.
        web_view: waterui::webview::WebViewOpen,
        #[cfg(waterui_feature = "map")]
        /// A map.
        map: waterui::map::MapConfig,
    }
    parents {
        #[cfg(waterui_feature = "avatar")]
        /// An avatar.
        avatar<C>: waterui::widget::avatar::AvatarConfig<C>,
        /// A button.
        button<C>: waterui::component::button::ButtonConfig<C>,
        /// A badge.
        badge<C>: waterui::component::badge::BadgeConfig<C>,
        /// A card container.
        card<C>: waterui::widget::card::CardConfig<C>,
        /// A disclosure group.
        disclosure<C>: waterui::widget::disclosure::DisclosureConfig<C>,
        /// A progress indicator.
        progress<C>: waterui::component::progress::ProgressConfig<C>,
        #[cfg(waterui_feature = "toggle")]
        /// A toggle.
        toggle<C>: waterui::component::toggle::ToggleConfig<C>,
        #[cfg(waterui_feature = "slider")]
        /// A slider.
        slider<C>: waterui::component::slider::SliderConfig<C>,
        #[cfg(waterui_feature = "stepper")]
        /// A stepper.
        stepper<C>: waterui::component::stepper::StepperConfig<C>,
        #[cfg(waterui_feature = "text-field")]
        /// A text field.
        text_field<C>: waterui::component::text_field::TextFieldConfig<C>,
        #[cfg(waterui_feature = "secure-field")]
        /// A secure text field.
        secure_field<C>: waterui::form::secure::SecureFieldConfig<C>,
        #[cfg(waterui_feature = "picker")]
        /// A picker.
        picker<C>: waterui::form::picker::PickerConfig<C>,
        #[cfg(waterui_feature = "color-picker")]
        /// A color picker.
        color_picker<C>: waterui::form::picker::color::ColorPickerConfig<C>,
        #[cfg(waterui_feature = "date-picker")]
        /// A date picker.
        date_picker<C>: waterui::form::picker::date::DatePickerConfig<C>,
        #[cfg(waterui_feature = "multi-date-picker")]
        /// A picker of several dates.
        multi_date_picker<C>: waterui::form::picker::multi_date::MultiDatePickerConfig<C>,
        #[cfg(waterui_feature = "calendar")]
        /// A calendar.
        calendar<C>: waterui::form::calendar::CalendarConfig<C>,
        #[cfg(waterui_feature = "menu")]
        /// A menu.
        menu<C>: waterui::component::menu::ResolvedMenu<C>,
        /// A scroll view.
        scroll<C>: waterui::layout::scroll::ScrollView<C>,
        /// A container with a fixed child list and a layout.
        fixed_container<C>: waterui::layout::container::FixedContainer<alloc::vec::Vec<C>>,
        /// A view whose content a handler replaces.
        dynamic<C>: waterui_core::Dynamic<C>,
        /// A container whose children are produced lazily.
        lazy_container<C>: waterui::layout::container::LazyContainer<C>,
        #[cfg(waterui_feature = "effects")]
        /// A view drawn through a filter.
        filtered<C>: waterui::graphics::backend::FilteredView<C>,
        #[cfg(waterui_feature = "list")]
        /// A list, each row's content a child.
        list<C>: waterui::component::list::ListConfig<C>,
        #[cfg(waterui_feature = "table")]
        /// A table, each row's cells children.
        table<C>: waterui::component::table::TableConfig<C>,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation bar around its content.
        navigation_view<C>: waterui::navigation::ResolvedNavigationView<C>,
        #[cfg(waterui_feature = "navigation")]
        /// A split navigation layout.
        navigation_split<C>: waterui::navigation::split::NavigationSplitLayout<C>,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation stack.
        navigation_stack<C>: waterui::navigation::NavigationStackLayout<C>,
        #[cfg(waterui_feature = "navigation")]
        /// Tabs.
        tabs<C>: waterui::navigation::tab::TabsLayout<C>,
    }
    metadata {
        /// An environment scope over its content.
        environment: Environment,
        /// A value kept alive with its content.
        retain: waterui_core::metadata::Retain,
        /// A layout priority.
        layout_priority: waterui_core::layout::LayoutPriority,
        /// A view subtree's refresh-rate vote.
        frame_rate: FrameRateVote,
        /// A lifecycle hook.
        lifecycle_hook: waterui_interaction::event::LifeCycleHook,
        /// A view-scoped asynchronous task hook.
        task_hook: waterui_interaction::event::TaskHook,
        /// An event handler.
        on_event: waterui_interaction::event::OnEvent,
        /// A key-press handler.
        on_key_press: waterui_interaction::key::OnKeyPress,
        /// A gesture observer.
        gesture_observer: waterui_interaction::gesture::GestureObserver,
        /// Selection state.
        selected: waterui_interaction::interaction::Selected,
        /// The accessibility children.
        accessibility_children: waterui_accessibility::AccessibilityChildren,
        /// The accessibility geometry.
        accessibility_geometry: waterui_accessibility::AccessibilityGeometry,
        /// Hidden from accessibility.
        accessibility_hidden: waterui_accessibility::AccessibilityHidden,
        /// An accessibility identifier.
        accessibility_identifier: waterui_accessibility::AccessibilityIdentifier,
        /// An accessibility label.
        accessibility_label: waterui_accessibility::AccessibilityLabel,
        /// An accessibility role.
        accessibility_role: waterui_accessibility::AccessibilityRole,
        /// Accessibility state.
        accessibility_state: waterui_accessibility::AccessibilityState,
        /// Accessibility state from a signal.
        accessibility_state_signal: waterui_accessibility::AccessibilityStateSignal,
        /// An accessibility value.
        accessibility_value: waterui_accessibility::AccessibilityValue,
        /// A structural transition.
        transition: waterui_structural_transition::TransitionSpec,
        /// Opacity.
        opacity: waterui::filter::Opacity,
        /// A scale transform.
        scale: waterui::style::Scale,
        /// A rotation transform.
        rotation: waterui::style::Rotation,
        /// An offset.
        offset: waterui::style::Offset,
        /// A shadow.
        shadow: waterui::style::Shadow,
        /// A border.
        border: waterui::border::Border,
        /// A clip shape.
        clip_shape: waterui::shape::ClipShape,
        /// A material background.
        material_background: waterui::background::MaterialBackground,
        /// A glass background.
        glass_background: waterui::background::GlassBackground,
        /// A group sharing one backdrop capture.
        material_group: waterui::background::MaterialGroup,
        /// Safe-area edges the content ignores.
        ignore_safe_area: waterui::layout::safe_area::IgnoreSafeArea,
        /// A cursor.
        cursor: waterui::cursor::Cursor,
        /// A drag source.
        draggable: waterui::drag_drop::Draggable,
        /// A drop destination.
        drop_destination: waterui::drag_drop::DropDestination,
        /// Hit-testing.
        hittable: waterui::interaction::Hittable,
        /// Focus.
        focused: waterui::component::focus::Focused,
        /// Secure content.
        secure: waterui::metadata::secure::Secure,
        /// Dynamic range.
        dynamic_range: waterui::metadata::secure::DynamicRangeMetadata,
        /// An anchored overlay.
        anchored_overlay: waterui::metadata::anchored_overlay::AnchoredOverlay,
        /// A window-modal sheet.
        sheet: waterui::sheet::Sheet,
        #[cfg(waterui_feature = "dialog")]
        /// A dialog.
        dialog: waterui::dialog::Dialog,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation link hint.
        navigation_link_hint: waterui::navigation::NavigationLinkHint,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation transition source.
        navigation_transition_source: waterui::navigation::NavigationTransitionSource,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation transition destination.
        navigation_transition_destination: waterui::navigation::NavigationTransitionDestination,
    }
    parent_metadata {
        #[cfg(waterui_feature = "menu")]
        /// A resolved context menu.
        resolved_context_menu<S>: waterui::metadata::context_menu::ResolvedContextMenu<S>,
        #[cfg(waterui_feature = "snackbar")]
        /// A snackbar host overlay.
        snackbar_host<S>: waterui::snackbar::SnackbarHostConfig<S>,
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use waterui::layout::Spacer;
    use waterui::layout::scroll::scroll;
    use waterui_core::layout::LayoutPriority;
    use waterui_core::view::hook::{Hook, Hooks};

    use super::*;

    /// Records the method each payload reaches.
    #[derive(Debug, Default, Clone)]
    pub(super) struct Recorder {
        pub(super) reached: alloc::rc::Rc<core::cell::RefCell<Vec<&'static str>>>,
    }

    #[cfg(waterui_feature = "capture")]
    pub(super) struct RecorderOffscreen(pub(super) Recorder);

    #[cfg(waterui_feature = "capture")]
    impl ViewRenderer for RecorderOffscreen {
        fn render_to_rgba<V: View>(
            &self,
            view: V,
            env: &Environment,
            _request: waterui::RenderRequest,
        ) -> impl core::future::Future<Output = Result<waterui::RenderResult, waterui::RenderError>>
        {
            let mut recorder = self.0.clone();
            Realize::child(&mut recorder, view, env, ());
            core::future::ready(Ok(waterui::RenderResult::new(Vec::new(), 1, 1)))
        }
    }

    /// A payload that holds child views, taken apart into them.
    pub(super) trait Parent<C> {
        fn children(self) -> Vec<C>;
    }

    impl<C> Parent<C> for waterui::component::button::ButtonConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "avatar")]
    impl<C> Parent<C> for waterui::widget::avatar::AvatarConfig<C> {
        fn children(mut self) -> Vec<C> {
            self.take_fallback().into_iter().collect()
        }
    }

    impl<C> Parent<C> for waterui::component::badge::BadgeConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.content]
        }
    }

    impl<C> Parent<C> for waterui::widget::card::CardConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.content]
        }
    }

    impl<C> Parent<C> for waterui::widget::disclosure::DisclosureConfig<C> {
        fn children(self) -> Vec<C> {
            let (header, content, _) = self.into_parts();
            alloc::vec![header.build(), content.build()]
        }
    }

    impl<C> Parent<C> for waterui::component::progress::ProgressConfig<C> {
        fn children(self) -> Vec<C> {
            let (label, value_label, ..) = self.into_parts();
            alloc::vec![label, value_label]
        }
    }

    #[cfg(waterui_feature = "toggle")]
    impl<C> Parent<C> for waterui::component::toggle::ToggleConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "slider")]
    impl<C> Parent<C> for waterui::component::slider::SliderConfig<C> {
        fn children(self) -> Vec<C> {
            let (label, min_value_label, max_value_label, ..) = self.into_parts();
            alloc::vec![label.build(), min_value_label, max_value_label]
        }
    }

    #[cfg(waterui_feature = "stepper")]
    impl<C> Parent<C> for waterui::component::stepper::StepperConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "text-field")]
    impl<C> Parent<C> for waterui::component::text_field::TextFieldConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "secure-field")]
    impl<C> Parent<C> for waterui::form::secure::SecureFieldConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "picker")]
    impl<C> Parent<C> for waterui::form::picker::PickerConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "color-picker")]
    impl<C> Parent<C> for waterui::form::picker::color::ColorPickerConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "date-picker")]
    impl<C> Parent<C> for waterui::form::picker::date::DatePickerConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "multi-date-picker")]
    impl<C> Parent<C> for waterui::form::picker::multi_date::MultiDatePickerConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "calendar")]
    impl<C> Parent<C> for waterui::form::calendar::CalendarConfig<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label().build()]
        }
    }

    #[cfg(waterui_feature = "menu")]
    impl<C> Parent<C> for waterui::component::menu::ResolvedMenu<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.label.build()]
        }
    }

    impl<C> Parent<C> for waterui::layout::container::FixedContainer<Vec<C>> {
        fn children(self) -> Vec<C> {
            self.into_inner().1
        }
    }

    impl<C> Parent<C> for waterui::layout::scroll::ScrollView<C> {
        fn children(self) -> Vec<C> {
            let (_, content, _, _) = self.into_inner().into_parts();
            alloc::vec![content]
        }
    }

    /// A lazy container's children are the items it currently holds.
    impl<C: 'static> Parent<C> for waterui::layout::container::LazyContainer<C> {
        fn children(self) -> Vec<C> {
            let snapshot = self.into_inner().1.snapshot();
            snapshot
                .range()
                .filter_map(|index| snapshot.get_view(index))
                .collect()
        }
    }

    #[cfg(waterui_feature = "effects")]
    impl<C> Parent<C> for waterui::graphics::backend::FilteredView<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.content]
        }
    }

    /// A list's children are its rows' content.
    #[cfg(waterui_feature = "list")]
    impl<C: 'static> Parent<C> for waterui::component::list::ListConfig<C> {
        fn children(self) -> Vec<C> {
            let snapshot = self.contents().snapshot();
            snapshot
                .range()
                .filter_map(|index| snapshot.get_view(index))
                .collect()
        }
    }

    /// A table's children are its rows' cells.
    #[cfg(waterui_feature = "table")]
    impl<C: 'static> Parent<C> for waterui::component::table::TableConfig<C> {
        fn children(self) -> Vec<C> {
            let snapshot = self.rows().snapshot();
            snapshot
                .range()
                .filter_map(|index| snapshot.get_view(index))
                .flat_map(waterui::component::table::TableRow::into_cells)
                .collect()
        }
    }

    #[cfg(waterui_feature = "navigation")]
    impl<C> Parent<C> for waterui::navigation::ResolvedNavigationView<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.content]
        }
    }

    /// A stack's child is its root page's content.
    #[cfg(waterui_feature = "navigation")]
    impl<C> Parent<C> for waterui::navigation::NavigationStackLayout<C> {
        fn children(self) -> Vec<C> {
            alloc::vec![self.resolve(&Environment::new()).content]
        }
    }

    /// A split navigation layout's children are its sidebar, placeholder, and
    /// any currently selected middle/detail column content.
    #[cfg(waterui_feature = "navigation")]
    impl<C: 'static> Parent<C> for waterui::navigation::split::NavigationSplitLayout<C> {
        fn children(self) -> Vec<C> {
            use nami::Signal as _;

            let (primary, placeholder, primary_sel, content, secondary_sel, detail, ..) =
                self.into_parts();
            let mut children = alloc::vec![primary.build(), placeholder.build()];
            if let Some(selected) = primary_sel.snapshot() {
                if let Some(content) = &content {
                    children.push(content.build(selected).content);
                }
                if secondary_sel.is_none() {
                    children.push(detail.build(selected).content);
                }
            }
            if let Some(selected) = secondary_sel.as_ref().and_then(nami::Signal::snapshot) {
                children.push(detail.build(selected).content);
            }
            children
        }
    }

    /// Tabs' children are each tab's root content and the accessory.
    #[cfg(waterui_feature = "navigation")]
    impl<C: 'static> Parent<C> for waterui::navigation::tab::TabsLayout<C> {
        fn children(self) -> Vec<C> {
            let (_, tabs, _, _, accessory) = self.into_parts();
            let snapshot = tabs.snapshot();
            snapshot
                .range()
                .filter_map(|index| snapshot.get_view(index))
                .map(|tab| tab.content().build())
                .chain(accessory)
                .collect()
        }
    }

    /// A context menu's children are its preview and accessory.
    #[cfg(waterui_feature = "menu")]
    impl<C> Parent<C> for waterui::metadata::context_menu::ResolvedContextMenu<C> {
        fn children(self) -> Vec<C> {
            self.preview.into_iter().chain(self.accessory).collect()
        }
    }

    /// A snackbar host's children are the icon views of active presentations.
    #[cfg(waterui_feature = "snackbar")]
    impl<C: 'static> Parent<C> for waterui::snackbar::SnackbarHostConfig<C> {
        fn children(self) -> Vec<C> {
            use nami::Signal as _;
            use waterui::widget::disclosure::SlotBuilder;

            self.presentations()
                .snapshot()
                .into_iter()
                .filter_map(|presentation| presentation.icon().map(SlotBuilder::build))
                .collect()
        }
    }

    /// A dynamic's children are the views it delivers once connected.
    impl<C: 'static> Parent<C> for waterui_core::Dynamic<C> {
        fn children(self) -> Vec<C> {
            use nami::watcher::Context;

            use crate::DynamicNode;

            struct Collector<T>(Rc<RefCell<Vec<T>>>);

            impl<T: 'static> DynamicNode<T> for Collector<T> {
                fn update(&self, update: Context<T>) {
                    self.0.borrow_mut().push(update.into_value());
                }
            }

            let delivered = Rc::new(RefCell::new(Vec::new()));
            Collector(Rc::clone(&delivered)).connect(self);
            delivered.take()
        }
    }

    impl Recorder {
        /// Realizes each child view a payload held, recording whether it
        /// arrived typed or erased.
        pub(super) fn realize_children<C: ChildView<Self>>(
            &mut self,
            children: Vec<C>,
            env: &Environment,
        ) {
            for child in children {
                self.reached.borrow_mut().push(
                    if core::any::TypeId::of::<C>() == core::any::TypeId::of::<AnyView>() {
                        "erased child"
                    } else {
                        "typed child"
                    },
                );
                child.realize(self, env, ());
            }
        }
    }

    fn tree() -> impl View {
        Metadata::new(Spacer::flexible(), LayoutPriority::new(1))
    }

    #[test]
    fn each_payload_reaches_its_method() {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, tree(), &Environment::new(), ());
        assert_eq!(
            recorder.reached.borrow().as_slice(),
            ["layout_priority", "spacer"]
        );
    }

    #[derive(Debug)]
    struct ErasesItself;

    impl View for ErasesItself {
        fn body(self, _env: &Environment) -> impl View {
            AnyView::new(self)
        }
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "erased views without reaching a payload")
    )]
    fn a_body_that_erases_itself_realizes_as_an_empty_view() {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, ErasesItself, &Environment::new(), ());
        assert_eq!(recorder.reached.borrow().as_slice(), ["empty"]);
    }

    #[cfg(not(feature = "erased-dispatch"))]
    const UNHOOKED_CHILD: &str = "typed child";
    #[cfg(feature = "erased-dispatch")]
    const UNHOOKED_CHILD: &str = "erased child";

    #[test]
    fn a_payload_child_realizes_through_the_same_backend() {
        let mut recorder = Recorder::default();
        let env = Environment::new();
        Realize::child(&mut recorder, scroll(tree()), &env, ());
        Realize::child(&mut recorder, AnyView::new(scroll(tree())), &env, ());
        Realize::child(&mut recorder, scroll(AnyView::new(tree())), &env, ());
        // A typed tree's child stays typed unless `erased-dispatch` is
        // enabled; an erased scroll's payload always carries an erased child.
        assert_eq!(
            recorder.reached.borrow().as_slice(),
            [
                [
                    "scroll",
                    UNHOOKED_CHILD,
                    "environment",
                    "layout_priority",
                    "spacer"
                ],
                [
                    "scroll",
                    "erased child",
                    "environment",
                    "layout_priority",
                    "spacer"
                ],
                [
                    "scroll",
                    UNHOOKED_CHILD,
                    "environment",
                    "layout_priority",
                    "spacer"
                ],
            ]
            .concat()
        );
    }

    #[cfg(waterui_feature = "effects")]
    #[test]
    fn a_filtered_view_keeps_its_content_typed() {
        use waterui::graphics::FilterViewExt as _;

        let env = Environment::new();
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, tree().blur(4.0_f32), &env, ());
        Realize::child(&mut recorder, AnyView::new(tree().blur(4.0_f32)), &env, ());
        assert_eq!(
            recorder.reached.borrow().as_slice(),
            [
                ["filtered", UNHOOKED_CHILD, "layout_priority", "spacer"],
                ["filtered", "erased child", "layout_priority", "spacer"],
            ]
            .concat()
        );
    }

    #[test]
    fn a_card_keeps_its_content_typed() {
        use waterui::widget::card;

        let env = Environment::new();
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, card(tree()), &env, ());
        Realize::child(&mut recorder, AnyView::new(card(tree())), &env, ());
        assert_eq!(
            recorder.reached.borrow().as_slice(),
            [
                ["card", UNHOOKED_CHILD, "layout_priority", "spacer"],
                ["card", "erased child", "layout_priority", "spacer"],
            ]
            .concat()
        );
    }

    #[test]
    fn an_erased_tree_reaches_the_same_methods() {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, AnyView::new(tree()), &Environment::new(), ());
        assert_eq!(
            recorder.reached.borrow().as_slice(),
            ["layout_priority", "spacer"]
        );
    }
    /// Draws every spacer as a divider.
    #[derive(Clone)]
    struct SpacerAsDivider;

    impl Hook for SpacerAsDivider {
        type Target = Spacer;

        fn apply(&self, _target: Spacer, _env: &Environment) -> Result<impl View, Spacer> {
            Ok::<_, Spacer>(waterui::layout::Divider)
        }
    }

    /// Draws a spacer as itself inside a scroll view: a view holding its own
    /// target.
    #[derive(Clone)]
    struct SpacerInScroll;

    impl Hook for SpacerInScroll {
        type Target = Spacer;

        fn apply(&self, target: Spacer, _env: &Environment) -> Result<impl View, Spacer> {
            Ok::<_, Spacer>(scroll(target))
        }
    }

    /// Declines every spacer.
    #[derive(Clone)]
    struct DeclinesSpacers;

    impl Hook for DeclinesSpacers {
        type Target = Spacer;

        fn apply(&self, target: Spacer, _env: &Environment) -> Result<impl View, Spacer> {
            Err::<(), _>(target)
        }
    }

    fn reached(view: impl View) -> Vec<&'static str> {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, view, &Environment::new(), ());
        recorder.reached.borrow().clone()
    }

    /// Inside a scope every child a payload holds is the decorator's own, so
    /// the recorder sees it typed even where the tree erased it.
    #[test]
    fn a_hook_realizes_its_target_in_its_scope() {
        assert_eq!(
            reached(Hooks::new(tree(), SpacerAsDivider)),
            ["layout_priority", "divider"]
        );
        assert_eq!(reached(tree()), ["layout_priority", "spacer"]);
    }

    #[test]
    fn the_scope_reaches_children_a_payload_holds() {
        assert_eq!(
            reached(Hooks::new(scroll(scroll(tree())), SpacerAsDivider)),
            [
                "scroll",
                "typed child",
                "environment",
                "scroll",
                "typed child",
                "environment",
                "layout_priority",
                "divider"
            ]
        );
    }

    #[test]
    fn the_scope_reaches_through_an_erased_view() {
        assert_eq!(
            reached(Hooks::new(scroll(AnyView::new(tree())), SpacerAsDivider)),
            [
                "scroll",
                "typed child",
                "environment",
                "layout_priority",
                "divider"
            ]
        );
        assert_eq!(
            reached(Hooks::new(AnyView::new(scroll(tree())), SpacerAsDivider)),
            [
                "scroll",
                "typed child",
                "environment",
                "layout_priority",
                "divider"
            ]
        );
    }

    #[test]
    fn a_scope_entered_on_the_erased_path_reaches_its_subtree() {
        assert_eq!(
            reached(AnyView::new(Hooks::new(scroll(tree()), SpacerAsDivider))),
            [
                "scroll",
                "typed child",
                "environment",
                "layout_priority",
                "divider"
            ]
        );
    }

    #[test]
    fn a_hook_does_not_receive_its_own_target_back() {
        assert_eq!(
            reached(Hooks::new(tree(), SpacerInScroll)),
            [
                "layout_priority",
                "scroll",
                "typed child",
                "environment",
                "spacer"
            ]
        );
        assert_eq!(
            reached(AnyView::new(Hooks::new(tree(), SpacerInScroll))),
            [
                "layout_priority",
                "scroll",
                "typed child",
                "environment",
                "spacer"
            ]
        );
    }

    #[test]
    fn a_declined_payload_reaches_the_next_hook_and_then_the_backend() {
        assert_eq!(
            reached(Hooks::new(
                Hooks::new(tree(), DeclinesSpacers),
                SpacerAsDivider
            )),
            ["layout_priority", "divider"]
        );
        assert_eq!(
            reached(Hooks::new(tree(), DeclinesSpacers)),
            ["layout_priority", "spacer"]
        );
    }

    #[test]
    fn an_inner_scope_is_offered_a_payload_before_an_outer_one() {
        assert_eq!(
            reached(Hooks::new(
                Hooks::new(tree(), SpacerAsDivider),
                SpacerInScroll
            )),
            ["layout_priority", "divider"]
        );
        assert_eq!(
            reached(Hooks::new(
                AnyView::new(Hooks::new(tree(), SpacerAsDivider)),
                SpacerInScroll
            )),
            ["layout_priority", "divider"]
        );
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "two hooks for"))]
    fn a_list_with_two_hooks_for_one_target_is_refused() {
        let _ = Hooks::new(tree(), (SpacerAsDivider, DeclinesSpacers));
    }

    /// A payload drawn as a spacer unless a hook claims it.
    struct Fence;

    impl NativeView for Fence {}

    impl HookableView for Fence {
        fn compose(self, _env: &Environment) -> impl View {
            Spacer::flexible()
        }
    }

    impl View for Fence {
        fn body(self, env: &Environment) -> impl View {
            self.compose(env)
        }

        fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
            backend.hookable(self, env, cx)
        }
    }

    /// Draws a fence as a divider.
    #[derive(Clone)]
    struct FenceAsDivider;

    impl Hook for FenceAsDivider {
        type Target = Fence;

        fn apply(&self, _target: Fence, _env: &Environment) -> Result<impl View, Fence> {
            Ok::<_, Fence>(waterui::layout::Divider)
        }
    }

    #[test]
    fn a_hookable_view_draws_its_composition_unless_claimed() {
        assert_eq!(reached(Fence), ["spacer"]);
        assert_eq!(reached(AnyView::new(Fence)), ["spacer"]);
        assert_eq!(reached(Hooks::new(Fence, FenceAsDivider)), ["divider"]);
        assert_eq!(
            reached(Hooks::new(scroll(AnyView::new(Fence)), FenceAsDivider)),
            ["scroll", "typed child", "environment", "divider"]
        );
        assert_eq!(
            reached(AnyView::new(Hooks::new(Fence, FenceAsDivider))),
            ["divider"]
        );
    }

    #[cfg(waterui_feature = "menu")]
    #[test]
    fn context_menu_preview_and_accessory_stay_typed_and_reenter_hooks() {
        use waterui::ViewExt as _;
        use waterui::metadata::context_menu::ContextMenu;

        let authored = || {
            ().context_menu(
                ContextMenu::new(())
                    .preview(Spacer::flexible())
                    .accessory(Spacer::flexible()),
            )
        };

        assert_eq!(
            reached(authored()),
            [
                "resolved_context_menu",
                "empty",
                UNHOOKED_CHILD,
                "spacer",
                UNHOOKED_CHILD,
                "environment",
                "spacer",
            ]
        );
        assert_eq!(
            reached(AnyView::new(authored())),
            [
                "resolved_context_menu",
                "empty",
                "erased child",
                "spacer",
                "erased child",
                "environment",
                "spacer",
            ]
        );
        assert_eq!(
            reached(Hooks::new(authored(), SpacerAsDivider)),
            [
                "resolved_context_menu",
                "empty",
                "typed child",
                "divider",
                "typed child",
                "environment",
                "divider",
            ]
        );
    }

    #[cfg(waterui_feature = "snackbar")]
    #[test]
    fn a_snackbar_host_keeps_its_content_typed() {
        use core::time::Duration;

        use waterui::snackbar::{Snackbar, SnackbarManager};

        let authored = || {
            SnackbarManager::host(|manager: SnackbarManager| {
                manager.show(Snackbar::new("Saved").duration(Duration::ZERO));
                Spacer::flexible()
            })
        };

        assert_eq!(
            reached(authored()),
            ["snackbar_host", "environment", "spacer"]
        );
        assert_eq!(
            reached(AnyView::new(authored())),
            ["snackbar_host", "environment", "spacer"]
        );
        assert_eq!(
            reached(Hooks::new(authored(), SpacerAsDivider)),
            ["snackbar_host", "environment", "divider"]
        );

        let with_icon = || {
            SnackbarManager::host(|manager: SnackbarManager<Spacer>| {
                manager.show(
                    Snackbar::new("Saved")
                        .icon(Spacer::flexible)
                        .duration(Duration::ZERO),
                );
                Spacer::flexible()
            })
        };

        assert_eq!(
            reached(with_icon()),
            [
                "snackbar_host",
                "environment",
                "spacer",
                UNHOOKED_CHILD,
                "spacer",
            ]
        );
        assert_eq!(
            reached(AnyView::new(with_icon())),
            [
                "snackbar_host",
                "environment",
                "spacer",
                "erased child",
                "spacer",
            ]
        );
        assert_eq!(
            reached(Hooks::new(with_icon(), SpacerAsDivider)),
            [
                "snackbar_host",
                "environment",
                "divider",
                "typed child",
                "divider",
            ]
        );
    }

    #[test]
    fn control_labels_stay_typed_and_reenter_hooks() {
        use waterui::component::badge::Badge;
        use waterui::component::button::button;
        use waterui::component::label::Label;
        use waterui::component::progress::Progress;

        let authored = || {
            Badge::new(
                1_u32,
                button(Label::new("Save", Spacer::flexible)).action(|| {}),
            )
        };

        assert_eq!(
            reached(authored()),
            ["badge", UNHOOKED_CHILD, "button", UNHOOKED_CHILD, "spacer",]
        );
        assert_eq!(
            reached(AnyView::new(authored())),
            ["badge", "erased child", "button", "erased child", "spacer",]
        );
        assert_eq!(
            reached(Hooks::new(authored(), SpacerAsDivider)),
            ["badge", "typed child", "button", "typed child", "divider",]
        );

        let progress = || Progress::new(0.5).label(Label::new("Progress", Spacer::flexible));
        assert_eq!(
            reached(Hooks::new(progress(), SpacerAsDivider)),
            ["progress", "typed child", "divider", "typed child", "text",]
        );

        #[cfg(waterui_feature = "slider")]
        {
            use nami::Binding;
            use waterui::component::slider::slider;

            let value = Binding::new(0.5_f64);
            let slider_view = || {
                slider(Label::new("Volume", Spacer::flexible), &value)
                    .min_value_label(Label::new("Min", Spacer::flexible))
                    .max_value_label(Label::new("Max", Spacer::flexible))
            };
            assert_eq!(
                reached(Hooks::new(slider_view(), SpacerAsDivider)),
                [
                    "slider",
                    "typed child",
                    "divider",
                    "typed child",
                    "divider",
                    "typed child",
                    "divider",
                ]
            );
        }
    }

    #[cfg(waterui_feature = "navigation")]
    #[test]
    fn navigation_split_stays_typed_and_reenters_hooks() {
        use nami::Binding;
        use waterui::navigation::{NavigationSplitView, NavigationView};

        let selection = Binding::new(Some(1_i32));
        let authored = || {
            NavigationSplitView::new(&selection, Spacer::flexible(), |_id: i32| {
                NavigationView::new("Detail", Spacer::flexible())
            })
            .placeholder(Spacer::flexible())
        };

        assert_eq!(
            reached(authored()),
            [
                "navigation_split",
                UNHOOKED_CHILD,
                "spacer",
                UNHOOKED_CHILD,
                "spacer",
                UNHOOKED_CHILD,
                "spacer",
            ]
        );
        assert_eq!(
            reached(AnyView::new(authored())),
            [
                "navigation_split",
                "erased child",
                "spacer",
                "erased child",
                "spacer",
                "erased child",
                "spacer",
            ]
        );
        assert_eq!(
            reached(Hooks::new(authored(), SpacerAsDivider)),
            [
                "navigation_split",
                "typed child",
                "divider",
                "typed child",
                "divider",
                "typed child",
                "divider",
            ]
        );
    }

    #[test]
    fn subtree_button_styles_apply_across_containers_and_anyview() {
        use alloc::rc::Rc;
        use core::cell::RefCell;
        use waterui::ViewExt as _;
        use waterui::component::badge::Badge;
        use waterui::component::button::{
            ButtonRole, ButtonStyleConfiguration, ButtonStyleTarget, CustomButtonStyle,
            PrimitiveButtonStyle, PrimitiveButtonStyleConfiguration, button,
        };
        use waterui::component::label::Label;

        struct TaggedButtonStyle {
            seen_roles: Rc<RefCell<Vec<Option<ButtonRole>>>>,
        }

        impl CustomButtonStyle for TaggedButtonStyle {
            fn make_body<L: View>(
                &self,
                configuration: ButtonStyleConfiguration<L>,
            ) -> impl View + use<L> {
                self.seen_roles.borrow_mut().push(configuration.role());
                scroll(configuration.into_label())
            }
        }

        struct TaggedPrimitiveStyle;

        impl PrimitiveButtonStyle for TaggedPrimitiveStyle {
            fn make_body<L: View>(
                &self,
                configuration: PrimitiveButtonStyleConfiguration<L>,
            ) -> impl View + use<L> {
                configuration.into_label()
            }
        }

        #[derive(Clone)]
        struct DecliningButtonStyleHook;

        impl Hook for DecliningButtonStyleHook {
            type Target = ButtonStyleTarget;

            fn apply(
                &self,
                target: Self::Target,
                _env: &Environment,
            ) -> Result<impl View, Self::Target> {
                if false {
                    return Ok(());
                }
                Err(target)
            }
        }

        // A declining button-style hook preserves the typed child slot ("typed child").
        assert_eq!(
            reached(Hooks::new(
                Badge::new(
                    1_u32,
                    button(Label::new("Save", Spacer::flexible)).action(|| {})
                ),
                DecliningButtonStyleHook,
            )),
            ["badge", "typed child", "button", "typed child", "spacer"]
        );

        // Subtree `.button_style(...)` reaches buttons nested in containers and AnyView hops,
        // and inner hooks like `SpacerAsDivider` still re-enter the button's label.
        let roles = Rc::new(RefCell::new(Vec::new()));
        let tree = scroll(AnyView::new(
            button(Label::new("Delete", Spacer::flexible))
                .role(ButtonRole::Destructive)
                .action(|| {}),
        ))
        .button_style(TaggedButtonStyle {
            seen_roles: Rc::clone(&roles),
        });
        assert_eq!(
            reached(Hooks::new(tree, SpacerAsDivider)),
            [
                "scroll",
                "typed child",
                "environment",
                "environment",
                "accessibility_role",
                "accessibility_label",
                "gesture_observer",
                "scroll",
                "typed child",
                "environment",
                "divider"
            ]
        );
        assert_eq!(*roles.borrow(), alloc::vec![Some(ButtonRole::Destructive)]);

        // Primitive style also installs cleanly via `.button_style(...)`.
        let primitive_tree = AnyView::new(
            button("Go")
                .action(|| {})
                .button_style(TaggedPrimitiveStyle),
        );
        assert_eq!(
            reached(primitive_tree),
            [
                "environment",
                "accessibility_role",
                "accessibility_label",
                "text"
            ]
        );
    }

    #[test]
    fn subtree_button_explicit_style_and_inner_builtin_override_win() {
        use alloc::rc::Rc;
        use core::cell::RefCell;
        use waterui::ViewExt as _;
        use waterui::component::button::{
            ButtonStyle, ButtonStyleConfiguration, CustomButtonStyle, button,
        };
        use waterui::component::label::Label;

        struct TaggedButtonStyle {
            seen: Rc<RefCell<u32>>,
        }

        impl CustomButtonStyle for TaggedButtonStyle {
            fn make_body<L: View>(
                &self,
                configuration: ButtonStyleConfiguration<L>,
            ) -> impl View + use<L> {
                *self.seen.borrow_mut() += 1;
                scroll(configuration.into_label())
            }
        }

        // Explicit `.style(ButtonStyle::Bordered)` wins over outer `.button_style(...)`.
        let seen_explicit = Rc::new(RefCell::new(0));
        let explicit = button(Label::new("Save", Spacer::flexible))
            .style(ButtonStyle::Bordered)
            .action(|| {})
            .button_style(TaggedButtonStyle {
                seen: Rc::clone(&seen_explicit),
            });
        assert_eq!(reached(explicit), ["button", "typed child", "spacer"]);
        assert_eq!(*seen_explicit.borrow(), 0);

        // Inner `.button_style(ButtonStyle::Bordered)` overrides outer `.button_style(Custom)`.
        let seen_override = Rc::new(RefCell::new(0));
        let overridden = button(Label::new("Save", Spacer::flexible))
            .action(|| {})
            .button_style(ButtonStyle::Bordered)
            .button_style(TaggedButtonStyle {
                seen: Rc::clone(&seen_override),
            });
        assert_eq!(
            reached(Hooks::new(overridden, SpacerAsDivider)),
            ["button", "typed child", "divider"]
        );
        assert_eq!(*seen_override.borrow(), 0);
    }

    #[cfg(waterui_feature = "toggle")]
    #[test]
    fn subtree_toggle_styles_apply_across_containers_and_respect_overrides() {
        use alloc::rc::Rc;
        use core::cell::RefCell;
        use nami::Binding;
        use waterui::ViewExt as _;
        use waterui::component::label::Label;
        use waterui::component::toggle::{
            CustomToggleStyle, Toggle, ToggleStyle, ToggleStyleConfiguration,
        };

        struct CustomToggleMarkerStyle {
            seen: Rc<RefCell<u32>>,
        }

        impl CustomToggleStyle for CustomToggleMarkerStyle {
            fn make_body<L: View>(
                &self,
                configuration: ToggleStyleConfiguration<L>,
            ) -> impl View + use<L> {
                *self.seen.borrow_mut() += 1;
                scroll(configuration.into_label())
            }
        }

        let on = Binding::new(false);
        let seen = Rc::new(RefCell::new(0));
        let toggle_tree = Hooks::new(
            scroll(AnyView::new(Toggle::new(
                Label::new("Wi-Fi", Spacer::flexible),
                &on,
            )))
            .toggle_style(CustomToggleMarkerStyle {
                seen: Rc::clone(&seen),
            }),
            SpacerAsDivider,
        );
        assert_eq!(
            reached(toggle_tree),
            [
                "scroll",
                "typed child",
                "environment",
                "environment",
                "accessibility_role",
                "accessibility_label",
                "accessibility_state_signal",
                "gesture_observer",
                "scroll",
                "typed child",
                "environment",
                "divider"
            ]
        );
        assert_eq!(*seen.borrow(), 1);

        // Inner `.toggle_style(ToggleStyle::Checkbox)` overrides outer custom toggle style.
        let seen_override = Rc::new(RefCell::new(0));
        let toggle_override = Toggle::new(Label::new("Sync", Spacer::flexible), &on)
            .toggle_style(ToggleStyle::Checkbox)
            .toggle_style(CustomToggleMarkerStyle {
                seen: Rc::clone(&seen_override),
            });
        assert_eq!(
            reached(toggle_override),
            ["toggle", "typed child", "spacer"]
        );
        assert_eq!(*seen_override.borrow(), 0);
    }

    #[cfg(waterui_feature = "capture")]
    #[test]
    fn capture_handle_wires_typed_offscreen_renderer_and_reenters_hooks() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Poll, Waker};
        use core::time::Duration;
        use waterui::{CaptureHandle, RenderRequest, ViewExt as _};
        use waterui_layout::Size;

        fn block_on_ready<F: Future>(future: F) -> F::Output {
            let mut future = pin!(future);
            let mut cx = Context::from_waker(Waker::noop());
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(output) => output,
                Poll::Pending => panic!("recorder offscreen future must resolve immediately"),
            }
        }

        let handle = CaptureHandle::new();
        assert!(!handle.is_wired());
        let env = Environment::new();
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, Spacer::flexible().capture(&handle), &env, ());
        assert!(handle.is_wired());
        assert_eq!(recorder.reached.borrow().as_slice(), ["spacer"]);

        recorder.reached.borrow_mut().clear();
        let request = RenderRequest::new(Size::new(32.0, 32.0), Duration::ZERO);
        let result = block_on_ready(handle.render_to_rgba(tree(), request))
            .expect("wired capture handle renders");
        assert_eq!(result.width(), 1);
        assert_eq!(
            recorder.reached.borrow().as_slice(),
            ["layout_priority", "spacer"]
        );

        let hooked_handle = CaptureHandle::new();
        let mut hooked_recorder = Recorder::default();
        Realize::child(
            &mut hooked_recorder,
            Hooks::new(Spacer::flexible().capture(&hooked_handle), SpacerAsDivider),
            &env,
            (),
        );
        hooked_recorder.reached.borrow_mut().clear();
        block_on_ready(hooked_handle.render_to_rgba(tree(), request))
            .expect("hooked capture handle renders");
        assert_eq!(
            hooked_recorder.reached.borrow().as_slice(),
            ["layout_priority", "divider"]
        );
    }

    #[cfg(waterui_feature = "gpu")]
    #[test]
    fn typed_gpu_view_reaches_native_backend_on_static_and_erased_paths() {
        use waterui::graphics::{Context, Frame, GpuContent, GpuView};

        struct ProbeContent;

        impl GpuContent for ProbeContent {
            type Params = ();

            fn setup(&mut self, _gpu: &Context<'_>) {}

            fn render(&mut self, _frame: &mut Frame<'_>) {}
        }

        assert_eq!(reached(GpuView::new(ProbeContent)), ["gpu"]);
        assert_eq!(reached(AnyView::new(GpuView::new(ProbeContent))), ["gpu"]);
    }

    #[cfg(not(waterui_feature = "capture"))]
    #[test]
    fn capture_binder_is_not_routed_without_capture_feature() {
        use waterui::view_renderer::CaptureHandle;

        let handle = CaptureHandle::<Spacer>::new();
        let scope = handle.attach(Spacer::flexible());
        let env = Environment::new();

        let mut static_recorder = Recorder::default();
        Realize::child(&mut static_recorder, scope, &env, ());
        assert!(!handle.is_wired());
        #[cfg(not(feature = "erased-dispatch"))]
        assert_eq!(
            static_recorder.reached.borrow().first().copied(),
            Some(core::any::type_name::<
                waterui::view_renderer::CaptureBinder<Child<Recorder>>,
            >())
        );
        #[cfg(feature = "erased-dispatch")]
        assert_eq!(
            static_recorder.reached.borrow().as_slice(),
            [core::any::type_name::<
                Metadata<waterui::view_renderer::CaptureBinder<AnyView>, AnyView>,
            >()]
        );

        let erased_handle = CaptureHandle::<Spacer>::new();
        let mut erased_recorder = Recorder::default();
        Realize::child(
            &mut erased_recorder,
            AnyView::new(erased_handle.attach(Spacer::flexible())),
            &env,
            (),
        );
        assert!(!erased_handle.is_wired());
        assert_eq!(
            erased_recorder.reached.borrow().as_slice(),
            [core::any::type_name::<
                Metadata<waterui::view_renderer::CaptureBinder<AnyView>, AnyView>,
            >()]
        );
    }

    #[cfg(waterui_feature = "avatar")]
    #[test]
    fn an_avatar_keeps_its_fallback_typed() {
        use waterui::widget::avatar::{Avatar, avatar};

        let env = Environment::new();
        let reached = |view| {
            let mut recorder = Recorder::default();
            Realize::child(&mut recorder, view, &env, ());
            recorder.reached.into_inner()
        };

        assert_eq!(reached(avatar("Ada Lovelace")), ["dynamic_range", "avatar"]);

        let custom = || Avatar::new("Katherine Johnson", Spacer::flexible);
        assert_eq!(
            reached(custom()),
            ["dynamic_range", "avatar", "typed child", "spacer"]
        );
        assert_eq!(
            reached(AnyView::new(custom())),
            ["dynamic_range", "avatar", "erased child", "spacer"]
        );
        assert_eq!(
            reached(Hooks::new(custom(), SpacerAsDivider)),
            ["dynamic_range", "avatar", "typed child", "divider"]
        );
    }
}
