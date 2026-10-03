//! The port contract every component handler compiles against.
//!
//! FROZEN: a component port may rely on exactly these items. Anything it needs
//! beyond them is a JNI wrapper addition (a change to `jvm`, never a
//! reactivity type), not a change to this file.
//!
//! The model: the dispatcher walks an [`AnyView`]; each registered handler
//! claims a type and answers a [`NativeLeaf`] — a platform view plus the
//! layout face a container lays out with. A handler that wraps other views
//! renders its children through [`RenderContext::render`] and mounts them
//! with [`NativeLeaf::mount`]; the returned [`Mounted`] detaches the child's
//! view when it drops, which is how a container releases a child it replaces.
//! Everything the leaf's reactivity needs lives in [`KeepAlive`]: when the
//! leaf drops, the watchers stop and the global reference releases.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use jni::objects::{Global, JObject};
use waterui::reactive::Signal;
use waterui::reactive::watcher::Context;
use waterui_backend_core::{AnyView, Environment};
use waterui_core::layout::SubView;

use crate::jvm::{self, MainThread};

/// A leaf's platform object: a JNI global reference the leaf owns for its
/// life — the Android mirror of the `Retained` object an Apple leaf holds.
pub(crate) type PlatformView = Global<JObject<'static>>;

/// What a rendered component owns beyond its platform view: watcher guards,
/// action targets, rendered children, the platform objects they act on.
///
/// Dropped in reverse insertion order (last kept, first dropped): a guard
/// kept after the object it fires on stops before that object is released.
#[derive(Default)]
pub struct KeepAlive(Vec<Box<dyn Any>>);

impl fmt::Debug for KeepAlive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeepAlive")
            .field("held", &self.0.len())
            .finish()
    }
}

impl Drop for KeepAlive {
    fn drop(&mut self) {
        while let Some(item) = self.0.pop() {
            drop(item);
        }
    }
}

impl KeepAlive {
    /// Keeps `value` alive for the leaf's lifetime.
    pub fn keep(&mut self, value: impl Any) {
        self.0.push(Box::new(value));
    }

    /// Subscribes `watcher` to `signal` for the leaf's lifetime.
    ///
    /// The imperative view call inside `watcher` is the whole reactivity
    /// story: signals never cross JNI, so each change arrives here and is
    /// pushed to the platform object imperatively.
    pub fn watch<S: Signal>(
        &mut self,
        signal: &S,
        watcher: impl Fn(Context<S::Output>) + 'static,
    ) {
        self.keep(signal.watch(watcher));
    }

    /// Applies `signal`'s current value now, then every change: the usual
    /// way a port pushes a reactive property to its platform object.
    pub fn bind<S: Signal>(&mut self, signal: &S, apply: impl Fn(S::Output) + 'static) {
        apply(signal.snapshot());
        self.watch(signal, move |change| apply(change.into_value()));
    }
}

/// A rendered component: its platform view, its layout face, and what keeps
/// its reactivity alive.
///
/// Drop order is fixed by field order: watchers and children stop first, the
/// layout face next, the global reference last. Dropping a leaf does not
/// detach its view from a parent; mount it through [`NativeLeaf::mount`] for
/// that.
pub struct NativeLeaf {
    keepalive: KeepAlive,
    layout: Rc<dyn SubView>,
    view: PlatformView,
}

impl fmt::Debug for NativeLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeLeaf")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl NativeLeaf {
    /// A leaf whose platform view is `view` — any `android.view.View`
    /// subclass — held by a global reference for the leaf's life.
    pub fn new(view: PlatformView, layout: impl SubView + 'static) -> Self {
        Self {
            keepalive: KeepAlive::default(),
            layout: Rc::new(crate::measure_memo::MemoizingSubView::new(Box::new(layout))),
            view,
        }
    }

    /// The platform view. Borrowed: the leaf owns it.
    #[must_use]
    pub fn view(&self) -> &PlatformView {
        &self.view
    }

    /// How the parent measures and stretches this leaf.
    #[must_use]
    pub fn layout(&self) -> &dyn SubView {
        &*self.layout
    }

    /// Keeps `value` — a watcher guard, a rendered child leaf, an
    /// environment clone — alive for this leaf's life.
    pub fn keep(&mut self, value: impl Any) {
        self.keepalive.keep(value);
    }

    /// Subscribes `watcher` to `signal` for this leaf's life.
    pub fn watch<S: Signal>(
        &mut self,
        signal: &S,
        watcher: impl Fn(Context<S::Output>) + 'static,
    ) {
        self.keepalive.watch(signal, watcher);
    }

    /// Applies `signal`'s current value now, then every change.
    pub fn bind<S: Signal>(&mut self, signal: &S, apply: impl Fn(S::Output) + 'static) {
        self.keepalive.bind(signal, apply);
    }

    /// Adds this leaf's view to `parent` and returns the handle that owns
    /// both; dropping the handle removes the view and releases the leaf.
    #[must_use]
    pub fn mount(self, parent: &PlatformView) -> Mounted {
        let parent = jvm::with_env(|env| {
            jvm::globals()
                .bindings()
                .add_view(env, parent.as_ref(), self.view.as_ref())
                .expect("addView on a mounted leaf must succeed");
            env.new_global_ref(parent.as_ref())
                .expect("the parent is a live view")
        });
        Mounted {
            leaf: Some(self),
            parent: Some(parent),
        }
    }

    /// Detaches the view from its parent and hands the leaf back, for moving
    /// it elsewhere.
    fn detach(&mut self, parent: Option<PlatformView>) {
        if let Some(parent) = parent {
            jvm::with_env(|env| {
                jvm::globals()
                    .bindings()
                    .remove_view(env, parent.as_ref(), self.view.as_ref())
                    .expect("removeView on a mounted leaf must succeed")
            });
        }
    }
}

/// A child leaf attached to a parent view. Dropping it detaches the view
/// from its parent, then drops the leaf: this is how a container releases a
/// child it replaces or removes.
#[derive(Debug)]
pub struct Mounted {
    leaf: Option<NativeLeaf>,
    /// The parent the view was added to — `removeView` needs it back.
    parent: Option<PlatformView>,
}

impl Mounted {
    /// The child's platform view.
    ///
    /// # Panics
    ///
    /// When called on a `Mounted` that is already unmounting — impossible
    /// outside `Drop`.
    #[must_use]
    pub fn view(&self) -> &PlatformView {
        self.leaf.as_ref().expect("a live Mounted").view()
    }

    /// The child's layout face.
    ///
    /// # Panics
    ///
    /// When called on a `Mounted` that is already unmounting.
    #[must_use]
    pub fn layout(&self) -> &dyn SubView {
        self.leaf.as_ref().expect("a live Mounted").layout()
    }

    /// Detaches the view and hands the leaf back, for moving it elsewhere.
    ///
    /// # Panics
    ///
    /// When called on a `Mounted` that is already unmounting.
    #[must_use]
    pub fn unmount(mut self) -> NativeLeaf {
        let mut leaf = self.leaf.take().expect("a live Mounted");
        leaf.detach(self.parent.take());
        leaf
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        if let Some(mut leaf) = self.leaf.take() {
            leaf.detach(self.parent.take());
            drop(leaf);
        }
    }
}

/// What a handler sees while it renders.
pub struct RenderContext<'a> {
    env: &'a Environment,
    dispatcher: Rc<crate::dispatch::Dispatcher>,
    mtm: MainThread,
}

impl fmt::Debug for RenderContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderContext").finish_non_exhaustive()
    }
}

impl<'a> RenderContext<'a> {
    pub(crate) const fn new(
        env: &'a Environment,
        dispatcher: Rc<crate::dispatch::Dispatcher>,
        mtm: MainThread,
    ) -> Self {
        Self {
            env,
            dispatcher,
            mtm,
        }
    }

    /// The environment this subtree resolves against.
    #[must_use]
    pub const fn env(&self) -> &'a Environment {
        self.env
    }

    /// Proof of the main thread — the `MainThreadMarker` counterpart.
    #[must_use]
    pub const fn mtm(&self) -> MainThread {
        self.mtm
    }

    /// Renders `view` into a leaf: registered handlers claim it, composers
    /// expand through `body()`, and anything the backend does not own yet
    /// crosses to the fallback.
    ///
    /// # Panics
    ///
    /// When the fallback also declines the view — the same contract
    /// `WuiAnyView` enforced with a fatal error.
    #[must_use]
    pub fn render(&self, view: impl Into<AnyView>) -> NativeLeaf {
        self.try_render(view)
            .expect("native view has no backend handler")
    }

    /// Renders `view`, answering `None` when nothing claims it.
    #[must_use]
    pub fn try_render(&self, view: impl Into<AnyView>) -> Option<NativeLeaf> {
        self.dispatcher.render(view.into(), self.env, self.mtm)
    }

    /// The same context under another environment — for metadata handlers
    /// that overlay the environment for their subtree:
    /// `let env = ctx.env().clone(); env.insert(..); ctx.with_env(&env).render(content)`
    /// — and keep the clone in the leaf's [`KeepAlive`] when the subtree's
    /// signals may resolve through it after the handler returns.
    #[must_use]
    pub fn with_env<'b>(&self, env: &'b Environment) -> RenderContext<'b> {
        RenderContext::new(env, self.dispatcher.clone(), self.mtm)
    }

    /// An owned handle that can render after the handler returns — from a
    /// watcher that swaps a child (`Dynamic`, conditionals, lists,
    /// navigation).
    #[must_use]
    pub fn renderer(&self) -> Renderer {
        Renderer {
            env: self.env.clone(),
            dispatcher: self.dispatcher.clone(),
            mtm: self.mtm,
        }
    }
}

/// A `'static` render capability: an environment clone, the dispatcher and
/// the main-thread proof. `!Send`, so it can only be used on the main
/// thread.
#[derive(Debug, Clone)]
pub struct Renderer {
    env: Environment,
    dispatcher: Rc<crate::dispatch::Dispatcher>,
    mtm: MainThread,
}

impl Renderer {
    /// Renders `view` under the captured environment.
    ///
    /// # Panics
    ///
    /// When nothing claims the view — same contract as
    /// [`RenderContext::render`].
    #[must_use]
    pub fn render(&self, view: impl Into<AnyView>) -> NativeLeaf {
        self.try_render(view)
            .expect("native view has no backend handler")
    }

    /// Renders `view`, answering `None` when nothing claims it.
    #[must_use]
    pub fn try_render(&self, view: impl Into<AnyView>) -> Option<NativeLeaf> {
        self.dispatcher.render(view.into(), &self.env, self.mtm)
    }

    /// The captured environment and dispatcher as a context.
    #[must_use]
    pub fn context(&self) -> RenderContext<'_> {
        RenderContext::new(&self.env, self.dispatcher.clone(), self.mtm)
    }
}
