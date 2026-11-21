//! Hooks: realizations of a native payload that a subtree chooses.
//!
//! A [`Hook`] takes one native payload type — its [`Target`](Hook::Target) —
//! and returns the view that realizes it, or declines and gives the payload
//! back. A [`HookList`] is the set of hooks a subtree installs with
//! [`Hooks`], which reaches the backend through [`Backend::scoped`]: the
//! backend decorates itself with the list, so every payload below the
//! modifier is offered to the hooks before the backend realizes it.
//!
//! On a statically dispatched path the offer compares the payload's type with
//! each target by [`TypeId`], which folds to a constant once the code is
//! monomorphized: a payload no hook targets reaches the backend at no cost. An
//! [`AnyView`] hop keeps the scope through [`ErasedHooks`], the same list
//! matched at run time.
//!
//! A hook's view is realized by the backend the hooks decorate, not by the
//! decorated one, so a hook never receives the payloads its own view
//! produces.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::{TypeId, type_name};
use core::fmt;

use crate::backend::{Backend, ErasedPayload, cast};
use crate::components::NativeView;
use crate::{AnyView, Environment, View};

/// A realization of the native payload [`Target`](Self::Target).
pub trait Hook: Clone + 'static {
    /// The payload this hook realizes.
    type Target: NativeView + 'static;

    /// Realizes `target` as a view, or declines by returning it.
    ///
    /// # Errors
    ///
    /// Returns `target` unchanged when this hook does not realize it, so the
    /// next hook — or the backend itself — receives it.
    fn apply(&self, target: Self::Target, env: &Environment) -> Result<impl View, Self::Target>;
}

/// A payload with a realization of its own, composed from other views, that
/// a hook may replace.
///
/// It is a component no backend realizes natively but an application may want
/// to draw differently, such as a fenced code block.
///
/// It reaches the backend through [`Backend::hookable`]: a backend offers it
/// to the hooks in scope and, when none claims it, realizes
/// [`compose`](Self::compose).
pub trait HookableView: NativeView + Sized + 'static {
    /// The view this payload is drawn as when no hook claims it.
    fn compose(self, env: &Environment) -> impl View;
}

/// The default realization of a [`HookableView`] an erased path carries: it
/// takes the payload out of `payload` and composes it.
pub type ComposeErased = fn(&mut ErasedPayload<'_>, &Environment) -> AnyView;

/// [`ComposeErased`] for `T`.
///
/// # Panics
///
/// Panics when `payload` holds no `T`: the function travels with the payload
/// it was made for.
pub fn compose_erased<T: HookableView>(
    payload: &mut ErasedPayload<'_>,
    env: &Environment,
) -> AnyView {
    let view = payload
        .take::<T>()
        .expect("a hookable payload's composer receives that payload");
    AnyView::new(view.compose(env))
}

/// The hooks one [`Hooks`] modifier installs: a single [`Hook`], or a tuple of
/// up to eight. Earlier hooks are offered a payload first.
pub trait HookList: Clone + 'static {
    /// Offers `payload` to each hook in order. The first hook that realizes
    /// it has its view dispatched through `backend`; when every hook declines,
    /// the payload and the context come back.
    ///
    /// # Errors
    ///
    /// Returns the payload and the context when no hook realizes it.
    fn offer<T: NativeView + 'static, B: Backend>(
        &self,
        payload: T,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, (T, B::Ctx)>;

    /// [`offer`](Self::offer) for a payload an [`AnyView`] handed back
    /// erased: a hook that realizes it returns its view erased, and a payload
    /// every hook declines stays in `payload`.
    fn offer_erased(&self, payload: &mut ErasedPayload<'_>, env: &Environment) -> Option<AnyView>;

    /// Appends each hook's target type, in order.
    fn targets(&self, targets: &mut Vec<(TypeId, &'static str)>);
}

impl HookList for () {
    fn offer<T: NativeView + 'static, B: Backend>(
        &self,
        payload: T,
        _backend: &mut B,
        _env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, (T, B::Ctx)> {
        Err((payload, cx))
    }

    fn offer_erased(
        &self,
        _payload: &mut ErasedPayload<'_>,
        _env: &Environment,
    ) -> Option<AnyView> {
        None
    }

    fn targets(&self, _targets: &mut Vec<(TypeId, &'static str)>) {}
}

impl<H: Hook> HookList for H {
    fn offer<T: NativeView + 'static, B: Backend>(
        &self,
        payload: T,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> Result<B::Output, (T, B::Ctx)> {
        let target = match cast::<T, H::Target>(payload) {
            Ok(target) => target,
            Err(payload) => return Err((payload, cx)),
        };
        match self.apply(target, env) {
            Ok(view) => Ok(view.dispatch(backend, env, cx)),
            Err(target) => Err((
                cast::<H::Target, T>(target)
                    .ok()
                    .expect("a payload a hook declined has the type it was offered as"),
                cx,
            )),
        }
    }

    fn offer_erased(&self, payload: &mut ErasedPayload<'_>, env: &Environment) -> Option<AnyView> {
        let target = payload.take::<H::Target>()?;
        match self.apply(target, env) {
            Ok(view) => Some(AnyView::new(view)),
            Err(target) => {
                payload.restore(target);
                None
            }
        }
    }

    fn targets(&self, targets: &mut Vec<(TypeId, &'static str)>) {
        targets.push((TypeId::of::<H::Target>(), type_name::<H::Target>()));
    }
}

macro_rules! tuple_hook_lists {
    ($(($($hook:ident),+))+) => {$(
        impl<$($hook: Hook),+> HookList for ($($hook,)+) {
            #[allow(non_snake_case, reason = "the bindings are the tuple's type parameters")]
            fn offer<T: NativeView + 'static, B: Backend>(
                &self,
                payload: T,
                backend: &mut B,
                env: &Environment,
                cx: B::Ctx,
            ) -> Result<B::Output, (T, B::Ctx)> {
                let ($($hook,)+) = self;
                let unclaimed = (payload, cx);
                $(
                    let unclaimed = match HookList::offer($hook, unclaimed.0, backend, env, unclaimed.1) {
                        Ok(output) => return Ok(output),
                        Err(unclaimed) => unclaimed,
                    };
                )+
                Err(unclaimed)
            }

            #[allow(non_snake_case, reason = "the bindings are the tuple's type parameters")]
            fn offer_erased(
                &self,
                payload: &mut ErasedPayload<'_>,
                env: &Environment,
            ) -> Option<AnyView> {
                let ($($hook,)+) = self;
                $(
                    if let Some(view) = HookList::offer_erased($hook, payload, env) {
                        return Some(view);
                    }
                )+
                None
            }

            #[allow(non_snake_case, reason = "the bindings are the tuple's type parameters")]
            fn targets(&self, targets: &mut Vec<(TypeId, &'static str)>) {
                let ($($hook,)+) = self;
                $(HookList::targets($hook, targets);)+
            }
        }
    )+};
}

tuple_hook_lists! {
    (H0)
    (H0, H1)
    (H0, H1, H2)
    (H0, H1, H2, H3)
    (H0, H1, H2, H3, H4)
    (H0, H1, H2, H3, H4, H5)
    (H0, H1, H2, H3, H4, H5, H6)
    (H0, H1, H2, H3, H4, H5, H6, H7)
}

/// A view whose subtree offers its native payloads to `hooks` first.
///
/// Built by [`ViewExt::hooks`](crate::view::View) callers through
/// [`Hooks::new`]; it reaches the backend through [`Backend::scoped`].
pub struct Hooks<V, H> {
    content: V,
    hooks: H,
}

impl<V, H: HookList> Hooks<V, H> {
    /// Installs `hooks` over `content`.
    ///
    /// Two hooks of one list that target one payload type are a contract
    /// violation: which of them realizes the payload would depend on the
    /// order they were written in.
    pub fn new(content: V, hooks: H) -> Self {
        let mut targets = Vec::new();
        hooks.targets(&mut targets);
        for (index, (target, name)) in targets.iter().enumerate() {
            if targets[..index].iter().any(|(seen, _)| seen == target) {
                crate::contract_violation!(
                    "a hook list holds two hooks for {name}; a payload type has at most one hook per list"
                );
            }
        }
        Self { content, hooks }
    }
}

impl<V, H> fmt::Debug for Hooks<V, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hooks")
            .field("hooks", &type_name::<H>())
            .finish_non_exhaustive()
    }
}

impl<V: View, H: HookList> View for Hooks<V, H> {
    /// The content alone: the hooks reach a backend only through
    /// [`dispatch`](View::dispatch).
    fn body(self, _env: &Environment) -> impl View {
        self.content
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        #[cfg(feature = "erased-dispatch")]
        {
            backend.scoped(self.hooks, AnyView::new(self.content), env, cx)
        }
        #[cfg(not(feature = "erased-dispatch"))]
        {
            backend.scoped(self.hooks, self.content, env, cx)
        }
    }
}

/// One erased hook list, matched at run time.
trait ErasedHookList {
    fn offer_erased(&self, payload: &mut ErasedPayload<'_>, env: &Environment) -> Option<AnyView>;
}

impl<H: HookList> ErasedHookList for H {
    fn offer_erased(&self, payload: &mut ErasedPayload<'_>, env: &Environment) -> Option<AnyView> {
        HookList::offer_erased(self, payload, env)
    }
}

/// The hook lists installed below an [`AnyView`] hop, innermost first.
///
/// A backend keeps one alongside its typed hooks: a scope entered on the
/// erased path extends it instead of nesting another decorator type.
#[derive(Clone, Default)]
pub struct ErasedHooks(Option<Rc<ErasedScope>>);

struct ErasedScope {
    hooks: Rc<dyn ErasedHookList>,
    outer: ErasedHooks,
}

impl fmt::Debug for ErasedHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErasedHooks")
            .field("scopes", &self.depth())
            .finish()
    }
}

impl ErasedHooks {
    /// No hooks.
    #[must_use]
    pub const fn new() -> Self {
        Self(None)
    }

    /// `hooks`, erased, inside this scope.
    #[must_use]
    pub fn with<H: HookList>(&self, hooks: H) -> Self {
        Self(Some(Rc::new(ErasedScope {
            hooks: Rc::new(hooks),
            outer: self.clone(),
        })))
    }

    /// The scopes of `inner`, innermost still first, inside this one.
    #[must_use]
    pub fn nest(&self, inner: &Self) -> Self {
        let mut scopes = Vec::new();
        let mut scope = inner.0.as_deref();
        while let Some(current) = scope {
            scopes.push(Rc::clone(&current.hooks));
            scope = current.outer.0.as_deref();
        }
        scopes.into_iter().rev().fold(self.clone(), |outer, hooks| {
            Self(Some(Rc::new(ErasedScope { hooks, outer })))
        })
    }

    /// Whether no hook list is installed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    fn depth(&self) -> usize {
        let mut depth = 0;
        let mut scope = self.0.as_deref();
        while let Some(current) = scope {
            depth += 1;
            scope = current.outer.0.as_deref();
        }
        depth
    }

    /// Offers `payload` to each scope, innermost first. The view of the hook
    /// that realizes it comes back with the scopes outside that hook's list,
    /// which realize the view.
    pub fn offer(
        &self,
        payload: &mut ErasedPayload<'_>,
        env: &Environment,
    ) -> Option<(AnyView, Self)> {
        let mut scope = self.0.as_deref();
        while let Some(current) = scope {
            if let Some(view) = ErasedHookList::offer_erased(&*current.hooks, payload, env) {
                return Some((view, current.outer.clone()));
            }
            scope = current.outer.0.as_deref();
        }
        None
    }
}
