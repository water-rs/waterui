//! The hook scope as a backend decorator.
//!
//! [`Hooked`] wraps a [`NativeBackend`] with a [`HookList`]: every native
//! payload the decorator receives is offered to the hooks first, and one no
//! hook claims reaches the backend it wraps. It is a [`NativeBackend`] of its
//! own family, so the children of every payload it receives are held in its
//! own slot and realized through it again: the scope covers the whole subtree
//! below the [`Hooks`](waterui_core::view::hook::Hooks) modifier, however
//! the backend places or defers those children.
//!
//! A payload or metadata key holding child views that the decorator hands on
//! carries its children re-wrapped through
//! [`MapSlots`] into the inner family's
//! slot, each re-entering the decorator when the inner backend realizes it.

use alloc::vec::Vec;
use core::any::TypeId;
use core::marker::PhantomData;

use waterui::component::button::{ButtonConfig, ButtonStyleTarget};
#[cfg(waterui_feature = "toggle")]
use waterui::component::toggle::{ToggleConfig, ToggleStyleTarget};
use waterui_core::backend::{ErasedPayload, MapSlots};
use waterui_core::view::hook::{ErasedHooks, HookList};
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::child::{Child, ChildView, IntoChild, NativeBackendFamily};
use crate::native_backend::{NativeBackend, Realize};

/// A [`NativeBackend`] that offers each native payload to `hooks` before the
/// backend it wraps.
///
/// Besides the typed list it keeps the lists installed below an
/// [`AnyView`] hop, matched at run time: a scope entered on the erased path
/// extends that list instead of nesting another decorator, which keeps the
/// set of decorator types finite.
#[derive(Debug)]
pub struct Hooked<B, H> {
    inner: B,
    hooks: H,
    erased: ErasedHooks,
    hooks_button: bool,
    hooks_toggle: bool,
}

impl<B: NativeBackend, H: HookList> Hooked<B, H> {
    /// `inner` under `hooks` and the erased scopes `erased`.
    pub fn new(inner: B, hooks: H, erased: ErasedHooks) -> Self {
        let mut targets = Vec::new();
        hooks.targets(&mut targets);
        let hooks_button = targets
            .iter()
            .any(|(id, _)| *id == TypeId::of::<ButtonStyleTarget>());
        #[cfg(waterui_feature = "toggle")]
        let hooks_toggle = targets
            .iter()
            .any(|(id, _)| *id == TypeId::of::<ToggleStyleTarget>());
        #[cfg(not(waterui_feature = "toggle"))]
        let hooks_toggle = false;
        Self {
            inner,
            hooks,
            erased,
            hooks_button,
            hooks_toggle,
        }
    }

    const fn with_flags(
        inner: B,
        hooks: H,
        erased: ErasedHooks,
        hooks_button: bool,
        hooks_toggle: bool,
    ) -> Self {
        Self {
            inner,
            hooks,
            erased,
            hooks_button,
            hooks_toggle,
        }
    }

    /// Runs `f` on this decorator with the backend it wraps reborrowed under
    /// the erased scopes `erased`: how a hook's view, or a child the inner
    /// backend realizes, re-enters the scope.
    fn reentered<R>(
        inner: &mut B,
        hooks: H,
        erased: ErasedHooks,
        hooks_button: bool,
        hooks_toggle: bool,
        f: impl for<'a> FnOnce(&mut Hooked<<B::Family as NativeBackendFamily>::Backend<'a>, H>) -> R,
    ) -> R {
        inner.with_family(|backend| {
            let mut hooked = Hooked::with_flags(
                <B::Family as NativeBackendFamily>::reborrow(backend),
                hooks,
                erased,
                hooks_button,
                hooks_toggle,
            );
            f(&mut hooked)
        })
    }

    /// `child`, held in this decorator's slot, as a child of the backend it
    /// wraps that re-enters the decorator.
    pub(crate) fn rehook<C: ChildView<Self>>(&self, child: C) -> Rehook<B::Family, H> {
        Rehook {
            child: child.into_child(),
            hooks: self.hooks.clone(),
            erased: self.erased.clone(),
            hooks_button: self.hooks_button,
            hooks_toggle: self.hooks_toggle,
        }
    }

    /// A mapping into [`rehook`](Self::rehook) that borrows nothing, for a
    /// payload that maps its children later.
    pub(crate) fn rehooker<C: ChildView<Self>>(
        &self,
    ) -> impl Fn(C) -> Rehook<B::Family, H> + Clone + 'static {
        let hooks = self.hooks.clone();
        let erased = self.erased.clone();
        let hooks_button = self.hooks_button;
        let hooks_toggle = self.hooks_toggle;
        move |child: C| Rehook {
            child: child.into_child(),
            hooks: hooks.clone(),
            erased: erased.clone(),
            hooks_button,
            hooks_toggle,
        }
    }

    /// Offers `payload` to the erased scopes, innermost first, then to the
    /// typed hooks: the output of the hook that claims it, or the payload
    /// back with the context.
    pub(crate) fn offer<T: NativeView + 'static>(
        &mut self,
        payload: T,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, (T, B::Ctx)> {
        let payload = if self.erased.is_empty() {
            payload
        } else {
            let mut slot = Some(payload);
            if let Some((view, outer)) = self.erased.offer(&mut ErasedPayload::new(&mut slot), env)
            {
                return Ok(Self::reentered(
                    &mut self.inner,
                    self.hooks.clone(),
                    outer,
                    self.hooks_button,
                    self.hooks_toggle,
                    |hooked| Realize::child(hooked, view, env, cx),
                ));
            }
            slot.expect("a payload every erased hook declined stays in its slot")
        };
        self.hooks
            .offer(payload, &mut Realize::new(&mut self.inner), env, cx)
    }

    /// [`offer`](Self::offer) for a payload an [`AnyView`] handed back
    /// erased; a payload every hook declines stays in `payload`.
    pub(crate) fn offer_erased(
        &mut self,
        payload: &mut ErasedPayload<'_>,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, B::Ctx> {
        if let Some((view, outer)) = self.erased.offer(payload, env) {
            return Ok(Self::reentered(
                &mut self.inner,
                self.hooks.clone(),
                outer,
                self.hooks_button,
                self.hooks_toggle,
                |hooked| Realize::child(hooked, view, env, cx),
            ));
        }
        match self.hooks.offer_erased(payload, env) {
            Some(view) => Ok(Realize::child(&mut self.inner, view, env, cx)),
            None => Err(cx),
        }
    }

    /// Realizes `view` with the erased scope `hooks` inside this decorator's
    /// own.
    pub(crate) fn scoped_erased(
        &mut self,
        hooks: &ErasedHooks,
        view: AnyView,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        let outer = self.erased.clone();
        self.erased = outer.nest(hooks);
        let output = Realize::child(self, view, env, cx);
        self.erased = outer;
        output
    }

    /// This decorator's hooks and erased scopes.
    pub(crate) fn scope(&self) -> (H, ErasedHooks) {
        (self.hooks.clone(), self.erased.clone())
    }

    /// Offers a [`ButtonConfig`] to subtree button-style hooks when eligible,
    /// keeping unhooked buttons and declined hooks strictly typed.
    pub(crate) fn hook_button<C: ChildView<Self>>(
        &mut self,
        native: ButtonConfig<C>,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        if native.has_explicit_style() || (!self.hooks_button && self.erased.is_empty()) {
            let native = MapSlots::map_slots(native, self.rehooker::<C>());
            return self.inner().button(native, env, cx);
        }
        let rehook = self.rehooker::<C>();
        let (target, pending) = native
            .into_style_pending(move |slot| AnyView::new(SlotView(rehook(slot).into_child())));
        match self.offer(target, env, cx) {
            Ok(output) => output,
            Err((_, cx)) => {
                let restored = MapSlots::map_slots(pending.restore(), self.rehooker::<C>());
                self.inner().button(restored, env, cx)
            }
        }
    }

    /// Offers a [`ToggleConfig`] to subtree toggle-style hooks when eligible,
    /// keeping unhooked toggles and declined hooks strictly typed.
    #[cfg(waterui_feature = "toggle")]
    pub(crate) fn hook_toggle<C: ChildView<Self>>(
        &mut self,
        native: ToggleConfig<C>,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        if native.has_explicit_style() || (!self.hooks_toggle && self.erased.is_empty()) {
            let native = MapSlots::map_slots(native, self.rehooker::<C>());
            return self.inner().toggle(native, env, cx);
        }
        let rehook = self.rehooker::<C>();
        let (target, pending) = native
            .into_style_pending(move |slot| AnyView::new(SlotView(rehook(slot).into_child())));
        match self.offer(target, env, cx) {
            Ok(output) => output,
            Err((_, cx)) => {
                let restored = MapSlots::map_slots(pending.restore(), self.rehooker::<C>());
                self.inner().toggle(restored, env, cx)
            }
        }
    }

    /// The backend this decorator wraps.
    pub(crate) const fn inner(&mut self) -> &mut B {
        &mut self.inner
    }

    /// Shared reference to the backend this decorator wraps.
    #[cfg(waterui_feature = "capture")]
    pub(crate) const fn inner_ref(&self) -> &B {
        &self.inner
    }
}

/// View wrapper keeping a backend child slot typed across subtree style hooks.
#[doc(hidden)]
pub struct SlotView<F: NativeBackendFamily>(Child<F>);

impl<F: NativeBackendFamily> core::fmt::Debug for SlotView<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SlotView")
    }
}

impl<F: NativeBackendFamily> SlotView<F> {
    pub(crate) fn into_child(self) -> Child<F> {
        self.0
    }
}

impl<F: NativeBackendFamily> NativeView for SlotView<F> {}

impl<F: NativeBackendFamily> View for SlotView<F> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }
}

/// The family of a [`Hooked`] decorator over the backend family `F`.
pub struct HookedFamily<F, H>(PhantomData<fn() -> (F, H)>);

impl<F, H> core::fmt::Debug for HookedFamily<F, H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HookedFamily").finish()
    }
}

impl<F: NativeBackendFamily, H: HookList> NativeBackendFamily for HookedFamily<F, H> {
    type Output = F::Output;
    type Ctx = F::Ctx;
    type Backend<'a> = Hooked<F::Backend<'a>, H>;

    fn reborrow<'r>(backend: &'r mut Hooked<F::Backend<'_>, H>) -> Hooked<F::Backend<'r>, H> {
        Hooked::with_flags(
            F::reborrow(&mut backend.inner),
            backend.hooks.clone(),
            backend.erased.clone(),
            backend.hooks_button,
            backend.hooks_toggle,
        )
    }
}

/// A child held in a [`Hooked`] decorator's slot, handed to the backend the
/// decorator wraps: realizing it re-enters the decorator.
pub struct Rehook<F: NativeBackendFamily, H: HookList> {
    child: Child<HookedFamily<F, H>>,
    hooks: H,
    erased: ErasedHooks,
    hooks_button: bool,
    hooks_toggle: bool,
}

impl<F: NativeBackendFamily, H: HookList> core::fmt::Debug for Rehook<F, H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Rehook").finish_non_exhaustive()
    }
}

impl<B: NativeBackend, H: HookList> ChildView<B> for Rehook<B::Family, H> {
    fn realize(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let Self {
            child,
            hooks,
            erased,
            hooks_button,
            hooks_toggle,
        } = self;
        Hooked::reentered(
            backend,
            hooks,
            erased,
            hooks_button,
            hooks_toggle,
            |hooked| child.realize(hooked, env, cx),
        )
    }
}

impl<F: NativeBackendFamily, H: HookList> IntoChild<F> for Rehook<F, H> {
    fn into_child(self) -> Child<F> {
        Child::from_fn(move |backend, env, cx| {
            let Self {
                child,
                hooks,
                erased,
                hooks_button,
                hooks_toggle,
            } = self;
            let mut hooked = Hooked::with_flags(
                F::reborrow(backend),
                hooks,
                erased,
                hooks_button,
                hooks_toggle,
            );
            child.realize(&mut hooked, env, cx)
        })
    }
}
