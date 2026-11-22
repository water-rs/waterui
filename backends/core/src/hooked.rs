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

use core::any::TypeId;
use core::marker::PhantomData;

use waterui::component::button::{ButtonConfig, ButtonStyleTarget};
#[cfg(waterui_feature = "toggle")]
use waterui::component::toggle::{ToggleConfig, ToggleStyleTarget};
use waterui_core::backend::{Backend, ErasedPayload, MapSlots};
use waterui_core::view::hook::{ErasedHooks, HookList};
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::child::{Child, ChildView, IntoChild, NativeBackendFamily};
use crate::native_backend::{NativeBackend, Realize};

#[inline]
fn has_target<H: HookList, T: 'static>() -> bool {
    let target = TypeId::of::<T>();
    let mut index = 0;
    while index < H::TARGET_IDS.len() {
        if H::TARGET_IDS[index] == target {
            return true;
        }
        index += 1;
    }
    false
}

/// Type-level distinction between a pure static hook scope (`NoErased`,
/// installed by `Realize::scoped`) and an erased scope (`ErasedHooks`,
/// entered below an [`AnyView`] hop).
///
/// Keeping `NoErased` distinct in the type lets `Hooked::offer` and
/// unhooked `Hooked::hook_button` / `Hooked::hook_toggle` stay free of
/// `AnyView::new(SlotView)` and `RealizeDyn` instantiation on the static path.
pub trait ErasedScope: Clone + core::fmt::Debug + 'static {
    /// Checks at compile time or run time whether any erased hook targets `T`.
    fn targets_type<T: 'static>(&self) -> bool;

    /// Offers `payload` to any erased hooks first, then to `hooked.hooks`.
    #[expect(
        clippy::missing_errors_doc,
        reason = "internal dispatch helper returning the unclaimed payload and context"
    )]
    fn offer_then_typed<B: NativeBackend, H: HookList, T: NativeView + 'static>(
        hooked: &mut Hooked<B, H, Self>,
        payload: T,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, (T, B::Ctx)>;

    /// Offers an already-erased `payload` to any erased hooks first, then to `hooked.hooks`.
    #[expect(
        clippy::missing_errors_doc,
        reason = "internal dispatch helper returning the caller context when unclaimed"
    )]
    fn offer_erased_then_typed<B: NativeBackend, H: HookList>(
        hooked: &mut Hooked<B, H, Self>,
        payload: &mut ErasedPayload<'_>,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, B::Ctx>;

    /// Extends `self` with `hooks` entered below an erased hop.
    fn nest_erased(&self, hooks: &ErasedHooks) -> ErasedHooks;
}

/// Zero-sized erased scope for pure static `Hooks` scopes with no enclosing
/// [`AnyView`] hook hop.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoErased;

impl ErasedScope for NoErased {
    #[inline]
    fn targets_type<T: 'static>(&self) -> bool {
        false
    }

    #[inline]
    fn offer_then_typed<B: NativeBackend, H: HookList, T: NativeView + 'static>(
        hooked: &mut Hooked<B, H, Self>,
        payload: T,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, (T, B::Ctx)> {
        hooked
            .hooks
            .offer(payload, &mut Realize::new(&mut hooked.inner), env, cx)
    }

    fn offer_erased_then_typed<B: NativeBackend, H: HookList>(
        hooked: &mut Hooked<B, H, Self>,
        payload: &mut ErasedPayload<'_>,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, B::Ctx> {
        match hooked.hooks.offer_erased(payload, env) {
            Some(view) => Ok(Realize::child(&mut hooked.inner, view, env, cx)),
            None => Err(cx),
        }
    }

    fn nest_erased(&self, hooks: &ErasedHooks) -> ErasedHooks {
        ErasedHooks::new().nest(hooks)
    }
}

impl ErasedScope for ErasedHooks {
    fn targets_type<T: 'static>(&self) -> bool {
        Self::targets_type::<T>(self)
    }

    fn offer_then_typed<B: NativeBackend, H: HookList, T: NativeView + 'static>(
        hooked: &mut Hooked<B, H, Self>,
        payload: T,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, (T, B::Ctx)> {
        let payload = if hooked.erased.is_empty() {
            payload
        } else {
            let mut slot = Some(payload);
            if let Some((view, outer)) =
                hooked.erased.offer(&mut ErasedPayload::new(&mut slot), env)
            {
                return Ok(Hooked::<B, H, Self>::reentered_erased(
                    &mut hooked.inner,
                    hooked.hooks.clone(),
                    outer,
                    |reentered| Realize::child(reentered, view, env, cx),
                ));
            }
            slot.expect("a payload every erased hook declined stays in its slot")
        };
        hooked
            .hooks
            .offer(payload, &mut Realize::new(&mut hooked.inner), env, cx)
    }

    fn offer_erased_then_typed<B: NativeBackend, H: HookList>(
        hooked: &mut Hooked<B, H, Self>,
        payload: &mut ErasedPayload<'_>,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, B::Ctx> {
        if let Some((view, outer)) = hooked.erased.offer(payload, env) {
            return Ok(Hooked::<B, H, Self>::reentered_erased(
                &mut hooked.inner,
                hooked.hooks.clone(),
                outer,
                |reentered| Realize::child(reentered, view, env, cx),
            ));
        }
        match hooked.hooks.offer_erased(payload, env) {
            Some(view) => Ok(Realize::child(&mut hooked.inner, view, env, cx)),
            None => Err(cx),
        }
    }

    fn nest_erased(&self, hooks: &ErasedHooks) -> ErasedHooks {
        self.nest(hooks)
    }
}

/// A [`NativeBackend`] that offers each native payload to `hooks` before the
/// backend it wraps.
///
/// Besides the typed list it keeps the lists installed below an
/// [`AnyView`] hop, matched at run time: a scope entered on the erased path
/// extends that list instead of nesting another decorator, which keeps the
/// set of decorator types finite.
#[derive(Debug)]
pub struct Hooked<B, H, E = NoErased> {
    inner: B,
    hooks: H,
    erased: E,
}

impl<B: NativeBackend, H: HookList> Hooked<B, H, NoErased> {
    /// `inner` under the static hook scope `hooks`.
    pub const fn new(inner: B, hooks: H) -> Self {
        Self::with_scope(inner, hooks, NoErased)
    }
}

impl<B: NativeBackend, H: HookList, E: ErasedScope> Hooked<B, H, E> {
    /// `inner` under `hooks` and the erased scopes `erased`.
    pub const fn with_scope(inner: B, hooks: H, erased: E) -> Self {
        Self {
            inner,
            hooks,
            erased,
        }
    }

    /// Runs `f` on this decorator with the backend it wraps reborrowed under
    /// the erased scopes `erased`: how a hook's view, or a child the inner
    /// backend realizes, re-enters the scope.
    fn reentered_erased<R>(
        inner: &mut B,
        hooks: H,
        erased: ErasedHooks,
        f: impl for<'a> FnOnce(
            &mut Hooked<<B::Family as NativeBackendFamily>::Backend<'a>, H, ErasedHooks>,
        ) -> R,
    ) -> R {
        inner.with_family(|backend| {
            let mut hooked = Hooked::with_scope(
                <B::Family as NativeBackendFamily>::reborrow(backend),
                hooks,
                erased,
            );
            f(&mut hooked)
        })
    }

    /// Runs `f` on this decorator with the backend it wraps reborrowed under `erased`.
    fn reentered_with<R>(
        inner: &mut B,
        hooks: H,
        erased: E,
        f: impl for<'a> FnOnce(&mut Hooked<<B::Family as NativeBackendFamily>::Backend<'a>, H, E>) -> R,
    ) -> R {
        inner.with_family(|backend| {
            let mut hooked = Hooked::with_scope(
                <B::Family as NativeBackendFamily>::reborrow(backend),
                hooks,
                erased,
            );
            f(&mut hooked)
        })
    }

    /// `child`, held in this decorator's slot, as a child of the backend it
    /// wraps that re-enters the decorator.
    pub(crate) fn rehook<C: ChildView<Self>>(&self, child: C) -> Rehook<B::Family, H, E> {
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
    ) -> impl Fn(C) -> Rehook<B::Family, H, E> + Clone + 'static {
        let hooks = self.hooks.clone();
        let erased = self.erased.clone();
        move |child: C| Rehook {
            child: child.into_child(),
            hooks: hooks.clone(),
            erased: erased.clone(),
        }
    }

    #[inline]
    fn targets_type<T: 'static>(&self) -> bool {
        has_target::<H, T>() || self.erased.targets_type::<T>()
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
        E::offer_then_typed(self, payload, env, cx)
    }

    /// [`offer`](Self::offer) for a payload an [`AnyView`] handed back
    /// erased; a payload every hook declines stays in `payload`.
    pub(crate) fn offer_erased(
        &mut self,
        payload: &mut ErasedPayload<'_>,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, B::Ctx> {
        E::offer_erased_then_typed(self, payload, env, cx)
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
        let nested = self.erased.nest_erased(hooks);
        Self::reentered_erased(&mut self.inner, self.hooks.clone(), nested, |hooked| {
            Realize::child(hooked, view, env, cx)
        })
    }

    /// This decorator's hooks and erased scopes.
    pub(crate) fn scope(&self) -> (H, E) {
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
        if native.has_explicit_style() || !self.targets_type::<ButtonStyleTarget>() {
            let native = MapSlots::map_slots(native, self.rehooker::<C>());
            return self.inner().button(native, env, cx);
        }
        self.offer_button_style(native, env, cx)
    }

    #[inline(never)]
    fn offer_button_style<C: ChildView<Self>>(
        &mut self,
        native: ButtonConfig<C>,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        let rehook = self.rehooker::<C>();
        let (target, pending) = native.into_style_pending(move |slot| {
            waterui::component::StyleSlot::new(rehook(slot).into_child())
        });
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
        if native.has_explicit_style() || !self.targets_type::<ToggleStyleTarget>() {
            let native = MapSlots::map_slots(native, self.rehooker::<C>());
            return self.inner().toggle(native, env, cx);
        }
        self.offer_toggle_style(native, env, cx)
    }

    #[cfg(waterui_feature = "toggle")]
    #[inline(never)]
    fn offer_toggle_style<C: ChildView<Self>>(
        &mut self,
        native: ToggleConfig<C>,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        let rehook = self.rehooker::<C>();
        let (target, pending) = native.into_style_pending(move |slot| {
            waterui::component::StyleSlot::new(rehook(slot).into_child())
        });
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
    #[cfg(any(waterui_feature = "capture", waterui_feature = "webview"))]
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
    #[cfg(waterui_feature = "capture")]
    pub(crate) const fn new(child: Child<F>) -> Self {
        Self(child)
    }

    pub(crate) fn into_child(self) -> Child<F> {
        self.0
    }
}

impl<F: NativeBackendFamily> NativeView for SlotView<F> {}

impl<F: NativeBackendFamily> View for SlotView<F> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self, env, cx)
    }
}

type HookedFamilyMarker<F, H, E> = fn() -> (F, H, E);

/// The family of a [`Hooked`] decorator over the backend family `F`.
pub struct HookedFamily<F, H, E = NoErased>(PhantomData<HookedFamilyMarker<F, H, E>>);

impl<F, H, E> core::fmt::Debug for HookedFamily<F, H, E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HookedFamily").finish()
    }
}

impl<F: NativeBackendFamily, H: HookList, E: ErasedScope> NativeBackendFamily
    for HookedFamily<F, H, E>
{
    type Output = F::Output;
    type Ctx = F::Ctx;
    type Backend<'a> = Hooked<F::Backend<'a>, H, E>;

    fn reborrow<'r>(backend: &'r mut Hooked<F::Backend<'_>, H, E>) -> Hooked<F::Backend<'r>, H, E> {
        Hooked::with_scope(
            F::reborrow(&mut backend.inner),
            backend.hooks.clone(),
            backend.erased.clone(),
        )
    }
}

/// A child held in a [`Hooked`] decorator's slot, handed to the backend the
/// decorator wraps: realizing it re-enters the decorator.
pub struct Rehook<F: NativeBackendFamily, H: HookList, E: ErasedScope> {
    child: Child<HookedFamily<F, H, E>>,
    hooks: H,
    erased: E,
}

impl<F: NativeBackendFamily, H: HookList, E: ErasedScope> core::fmt::Debug for Rehook<F, H, E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Rehook").finish_non_exhaustive()
    }
}

impl<B: NativeBackend, H: HookList, E: ErasedScope> ChildView<B> for Rehook<B::Family, H, E> {
    fn realize(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let Self {
            child,
            hooks,
            erased,
        } = self;
        Hooked::reentered_with(backend, hooks, erased, |hooked| {
            child.realize(hooked, env, cx)
        })
    }
}

impl<F: NativeBackendFamily, H: HookList, E: ErasedScope> IntoChild<F> for Rehook<F, H, E> {
    fn into_child(self) -> Child<F> {
        Child::from_fn(move |backend, env, cx| {
            let Self {
                child,
                hooks,
                erased,
            } = self;
            let mut hooked = Hooked::with_scope(F::reborrow(backend), hooks, erased);
            child.realize(&mut hooked, env, cx)
        })
    }
}
