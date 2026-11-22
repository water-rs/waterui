//! Typed child-slot builders and backend-provided probes shared across controls
//! and navigation without `AnyView` erasure.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::Any;
use core::fmt;

use waterui_core::backend::{Backend, ChildSlot};
use waterui_core::metadata::MetadataKey;
use waterui_core::view::hook::{HookList, HookableView};
use waterui_core::{AnyView, Environment, Metadata, Native, NativeView, View};

/// A zero-cost [`ChildSlot`] that discards child views without monomorphizing
/// their dispatch, used by non-rendering probes and chord inspection.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSlot;

impl ChildSlot for NoopSlot {
    fn from_view<V: View>(_view: V) -> Self {
        Self
    }
}

/// Builds one child view in a backend's child slot `T`.
pub struct SlotBuilder<T>(Rc<dyn Fn() -> T>);

impl<T> SlotBuilder<T> {
    /// Wraps a slot factory closure.
    #[doc(hidden)]
    #[must_use]
    pub fn new(build: impl Fn() -> T + 'static) -> Self {
        Self(Rc::new(build))
    }

    /// Wraps an already-shared slot factory closure.
    #[doc(hidden)]
    #[must_use]
    pub fn from_rc(build: Rc<dyn Fn() -> T>) -> Self {
        Self(build)
    }

    /// Builds the slot value.
    #[must_use]
    pub fn build(&self) -> T {
        (self.0)()
    }
}

impl<T: 'static> SlotBuilder<T> {
    /// Maps this builder's output through `f`.
    #[must_use]
    pub fn map_slot<U>(self, f: impl Fn(T) -> U + 'static) -> SlotBuilder<U> {
        SlotBuilder::new(move || f(self.build()))
    }
}

impl<T> Clone for SlotBuilder<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> fmt::Debug for SlotBuilder<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SlotBuilder")
    }
}

/// Probe backend that offers only the view's native payload to
/// [`NativeView::realize_provided`] without expanding child slots or touching
/// `AnyView` dispatch.
struct ProvidedProbeBackend<'a> {
    context: &'a mut dyn Any,
    matched: bool,
}

impl Backend for ProvidedProbeBackend<'_> {
    type Output = ();
    type Ctx = ();
    type Slot = NoopSlot;

    fn native<T: NativeView + 'static>(&mut self, native: T, env: &Environment, (): ()) {
        if native.realize_provided(self.context, env).is_ok() {
            self.matched = true;
        }
    }

    fn metadata<K: MetadataKey>(
        &mut self,
        _metadata: Metadata<K, Self::Slot>,
        _env: &Environment,
        (): (),
    ) {
    }

    fn erased(&mut self, _view: AnyView, _env: &Environment, (): ()) {}

    fn scoped<H: HookList, V: View>(&mut self, _hooks: H, view: V, env: &Environment, (): ()) {
        view.dispatch(self, env, ());
    }

    fn hookable<T: HookableView>(&mut self, view: T, env: &Environment, (): ()) {
        if view.realize_provided(self.context, env).is_ok() {
            self.matched = true;
        }
    }
}

fn probe_view_provided<V: View>(view: V, context: &mut dyn Any, env: &Environment) -> bool {
    let mut backend = ProvidedProbeBackend {
        context,
        matched: false,
    };
    view.dispatch(&mut backend, env, ());
    backend.matched
}

type ProbeFn = Rc<dyn Fn(&mut dyn Any, &Environment) -> bool>;

/// A label's icon lifted into a backend's child slot `S`, preserving both
/// static slot realization and backend-provided probing (such as platform
/// symbols) without `AnyView` erasure.
pub struct IconSlot<S> {
    build: Rc<dyn Fn() -> S>,
    probe: ProbeFn,
}

impl<S: ChildSlot> IconSlot<S> {
    /// Captures a concrete icon view into child slot `S` without erasing to
    /// [`AnyView`].
    #[must_use]
    pub fn from_view<V: View + Clone>(view: V) -> Self {
        let for_build = view.clone();
        Self {
            build: Rc::new(move || S::from_view(for_build.clone())),
            probe: Rc::new(move |context, env| probe_view_provided(view.clone(), context, env)),
        }
    }
}

impl<S> IconSlot<S> {
    /// Materializes the icon into child slot `S`.
    #[must_use]
    pub fn build(&self) -> S {
        (self.build)()
    }

    /// Offers the icon's native payload to [`NativeView::realize_provided`]
    /// using backend-specific probe `context` without mounting or rasterizing
    /// a view.
    pub fn probe(&self, context: &mut dyn Any, env: &Environment) -> bool {
        (self.probe)(context, env)
    }
}

impl<S: 'static> IconSlot<S> {
    /// Maps the produced child slot through `f`, keeping the probe intact.
    #[must_use]
    pub fn map_slot<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> IconSlot<T> {
        let build = self.build;
        IconSlot {
            build: Rc::new(move || f(build())),
            probe: self.probe,
        }
    }
}

impl<S> Clone for IconSlot<S> {
    fn clone(&self) -> Self {
        Self {
            build: Rc::clone(&self.build),
            probe: Rc::clone(&self.probe),
        }
    }
}

impl<S> fmt::Debug for IconSlot<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IconSlot")
    }
}

/// Backend child slot carried across subtree style hooks (`ButtonStyleHook` /
/// `ToggleStyleHook`) without erasing through [`AnyView`].
#[doc(hidden)]
pub struct StyleSlot(Box<dyn Any>);

impl StyleSlot {
    /// Wraps a backend-family child slot.
    #[must_use]
    pub fn new<T: 'static>(slot: T) -> Self {
        Self(Box::new(slot))
    }

    /// Recovers the concrete backend-family child slot.
    ///
    /// # Errors
    ///
    /// Returns `self` unchanged if `T` does not match the stored slot type.
    pub fn downcast<T: 'static>(self) -> Result<T, Self> {
        self.0.downcast::<T>().map(|boxed| *boxed).map_err(Self)
    }
}

impl fmt::Debug for StyleSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StyleSlot")
    }
}

impl NativeView for StyleSlot {}

impl View for StyleSlot {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self, env, cx)
    }
}
