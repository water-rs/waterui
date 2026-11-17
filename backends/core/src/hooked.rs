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
//! A payload the decorator hands on carries its children re-wrapped through
//! [`MapSlots`](waterui_core::backend::MapSlots) into the inner family's
//! slot, each re-entering the decorator when the inner backend realizes it.

use core::marker::PhantomData;

use waterui_core::backend::ErasedPayload;
use waterui_core::view::hook::{ErasedHooks, HookList};
use waterui_core::{AnyView, Environment, NativeView};

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
}

impl<B: NativeBackend, H: HookList> Hooked<B, H> {
    /// `inner` under `hooks` and the erased scopes `erased`.
    pub const fn new(inner: B, hooks: H, erased: ErasedHooks) -> Self {
        Self {
            inner,
            hooks,
            erased,
        }
    }

    /// Runs `f` on this decorator with the backend it wraps reborrowed under
    /// the erased scopes `erased`: how a hook's view, or a child the inner
    /// backend realizes, re-enters the scope.
    fn reentered<R>(
        inner: &mut B,
        hooks: H,
        erased: ErasedHooks,
        f: impl for<'a> FnOnce(&mut Hooked<<B::Family as NativeBackendFamily>::Backend<'a>, H>) -> R,
    ) -> R {
        inner.with_family(|backend| {
            let mut hooked = Hooked::new(
                <B::Family as NativeBackendFamily>::reborrow(backend),
                hooks,
                erased,
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
        }
    }

    /// A mapping into [`rehook`](Self::rehook) that borrows nothing, for a
    /// payload that maps its children later.
    pub(crate) fn rehooker<C: ChildView<Self>>(
        &self,
    ) -> impl Fn(C) -> Rehook<B::Family, H> + Clone + 'static {
        let hooks = self.hooks.clone();
        let erased = self.erased.clone();
        move |child: C| Rehook {
            child: child.into_child(),
            hooks: hooks.clone(),
            erased: erased.clone(),
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
            if let Some((view, outer)) = self.erased.offer(&mut ErasedPayload::new(&mut slot), env) {
                return Ok(Self::reentered(
                    &mut self.inner,
                    self.hooks.clone(),
                    outer,
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

    /// The backend this decorator wraps.
    pub(crate) fn inner(&mut self) -> &mut B {
        &mut self.inner
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
        Hooked::new(
            F::reborrow(&mut backend.inner),
            backend.hooks.clone(),
            backend.erased.clone(),
        )
    }
}

/// A child held in a [`Hooked`] decorator's slot, handed to the backend the
/// decorator wraps: realizing it re-enters the decorator.
pub struct Rehook<F: NativeBackendFamily, H: HookList> {
    child: Child<HookedFamily<F, H>>,
    hooks: H,
    erased: ErasedHooks,
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
        } = self;
        Hooked::reentered(backend, hooks, erased, |hooked| {
            child.realize(hooked, env, cx)
        })
    }
}

impl<F: NativeBackendFamily, H: HookList> IntoChild<F> for Rehook<F, H> {
    fn into_child(self) -> Child<F> {
        Child::from_fn(move |backend, env, cx| {
            let Self {
                child,
                hooks,
                erased,
            } = self;
            let mut hooked = Hooked::new(F::reborrow(backend), hooks, erased);
            child.realize(&mut hooked, env, cx)
        })
    }
}
