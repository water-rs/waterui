//! The port contract every component handler compiles against.
//!
//! FROZEN: a component port may rely on exactly these items. Anything it needs
//! beyond them is a kit addition (a change to `cocoa-ui`, never a reactivity
//! type), not a change to this file.
//!
//! The model: the dispatcher walks an [`AnyView`]; each registered handler
//! claims a type and answers a [`NativeLeaf`] — a platform view plus the
//! layout face a container lays out with. A handler that wraps other views
//! renders its children through [`RenderContext::render`] and mounts them
//! with [`NativeLeaf::mount`]; the returned [`Mounted`] detaches the child's
//! view when it drops, which is how a container releases a child it replaces.
//! Everything the leaf's reactivity needs lives in [`KeepAlive`]: when the
//! leaf drops, the watchers stop and the platform object releases.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use cocoa_ui::PlatformView;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;
use waterui::reactive::Signal;
use waterui::reactive::watcher::Context;
use waterui_backend_core::{AnyView, Environment};
use waterui_core::layout::SubView;

use cocoa_ui::Retained;

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
    /// The imperative kit call inside `watcher` is the whole reactivity
    /// story: signals never cross into the kit, so each change arrives here
    /// and is pushed to the platform object imperatively.
    pub fn watch<S: Signal>(&mut self, signal: &S, watcher: impl Fn(Context<S::Output>) + 'static) {
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
/// layout face next, the platform view last. Dropping a leaf does not detach
/// its view from a superview; mount it through [`NativeLeaf::mount`] for
/// that.
pub struct NativeLeaf {
    keepalive: KeepAlive,
    layout: Rc<dyn SubView>,
    view: Retained<PlatformView>,
    /// The view controllers [`mount`] adopted from this leaf's subtree —
    /// `UIKit` only forwards appearance and layout callbacks down a real
    /// containment chain.
    #[cfg(target_os = "ios")]
    attached_controllers: Vec<Retained<cocoa_ui::objc2_ui_kit::UIViewController>>,
}

impl fmt::Debug for NativeLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeLeaf")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl NativeLeaf {
    /// A leaf whose platform view is `view` (any `NSView`/`UIView`
    /// subclass), retained for the leaf's life.
    pub fn new<V: AsRef<PlatformView> + ?Sized>(view: &V, layout: impl SubView + 'static) -> Self {
        Self {
            keepalive: KeepAlive::default(),
            layout: Rc::new(crate::measure_memo::MemoizingSubView::new(Box::new(layout))),
            view: cocoa_ui::view::retain_base(view),
            #[cfg(target_os = "ios")]
            attached_controllers: Vec::new(),
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

    /// Mirrors the leaf's layout face onto the view's intrinsic measure,
    /// so an Auto Layout parent — the toggle's row is one — can size the
    /// mounted child. Without it a `HostView` leaf reports no intrinsic
    /// content size and the constraint system collapses it to zero.
    fn install_intrinsic_measure(view: &PlatformView, layout: &Rc<dyn SubView>) {
        if let Some(host) = view.downcast_ref::<HostView>() {
            let layout = Rc::clone(layout);
            host.set_measure_handler(move |_host, proposal| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "the layout contract is f32; measured points always fit"
                )]
                let measured = layout.measure(waterui_core::layout::ProposalSize::new(
                    proposal.width.map(|width| width as f32),
                    proposal.height.map(|height| height as f32),
                ));
                cocoa_ui::geometry::Size::new(
                    f64::from(measured.size.width),
                    f64::from(measured.size.height),
                )
            });
        }
    }

    /// Keeps `value` — a watcher guard, a rendered child leaf, an
    /// environment clone — alive for this leaf's life.
    pub fn keep(&mut self, value: impl Any) {
        self.keepalive.keep(value);
    }

    /// Subscribes `watcher` to `signal` for this leaf's life.
    pub fn watch<S: Signal>(&mut self, signal: &S, watcher: impl Fn(Context<S::Output>) + 'static) {
        self.keepalive.watch(signal, watcher);
    }

    /// Applies `signal`'s current value now, then every change.
    pub fn bind<S: Signal>(&mut self, signal: &S, apply: impl Fn(S::Output) + 'static) {
        self.keepalive.bind(signal, apply);
    }

    /// Adds this leaf's view to `parent` and returns the handle that owns
    /// both; dropping the handle removes the view and releases the leaf.
    #[must_use]
    #[cfg_attr(
        not(target_os = "ios"),
        expect(unused_mut, reason = "mount mutates only on iOS")
    )]
    pub fn mount(mut self, parent: &PlatformView) -> Mounted {
        Self::install_intrinsic_measure(&self.view, &self.layout);
        cocoa_ui::view::add_subview(parent, &self.view);
        #[cfg(target_os = "ios")]
        {
            self.attached_controllers = adopt_controllers(&self.view);
            for controller in &self.attached_controllers {
                cocoa_ui::uikit::view_controller::did_move_to_parent(controller);
            }
        }
        Mounted(Some(self))
    }

    /// Detaches the view — ending `UIKit` containment first when `mount`
    /// established it — and hands the leaf back, for moving it elsewhere.
    #[cfg_attr(
        not(target_os = "ios"),
        expect(
            clippy::needless_pass_by_ref_mut,
            reason = "the contained-controller take is iOS-only"
        )
    )]
    fn detach(&mut self) {
        #[cfg(target_os = "ios")]
        let controllers = std::mem::take(&mut self.attached_controllers);
        #[cfg(target_os = "ios")]
        for controller in &controllers {
            cocoa_ui::uikit::view_controller::will_move_to_parent(controller);
        }
        cocoa_ui::view::remove_from_superview(&self.view);
        #[cfg(target_os = "ios")]
        for controller in controllers.iter().rev() {
            cocoa_ui::uikit::view_controller::remove_from_parent(controller);
        }
    }
}

/// A child leaf attached to a parent view. Dropping it detaches the view
/// from its superview, then drops the leaf: this is how a container
/// releases a child it replaces or removes.
#[derive(Debug)]
pub struct Mounted(Option<NativeLeaf>);

impl Mounted {
    /// The child's platform view.
    ///
    /// # Panics
    ///
    /// When called on a `Mounted` that is already unmounting — impossible
    /// outside `Drop`.
    #[must_use]
    pub fn view(&self) -> &PlatformView {
        self.0.as_ref().expect("a live Mounted").view()
    }

    /// The child's layout face.
    ///
    /// # Panics
    ///
    /// When called on a `Mounted` that is already unmounting.
    #[must_use]
    pub fn layout(&self) -> &dyn SubView {
        self.0.as_ref().expect("a live Mounted").layout()
    }

    /// Detaches the view and hands the leaf back, for moving it elsewhere.
    ///
    /// # Panics
    ///
    /// When called on a `Mounted` that is already unmounting.
    #[must_use]
    pub fn unmount(mut self) -> NativeLeaf {
        let mut leaf = self.0.take().expect("a live Mounted");
        leaf.detach();
        leaf
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        if let Some(mut leaf) = self.0.take() {
            leaf.detach();
            drop(leaf);
        }
    }
}

/// Adopts every unparented `UIViewController` whose root view sits inside
/// `root`'s subtree under the controller enclosing that view's superview.
/// A leaf's own view is often a `HostView` wrapper around the controller's
/// root view, so checking only the top view misses the controller one level
/// down. Without the containment chain `UIKit` never calls
/// `viewWillLayoutSubviews` or the appearance callbacks on the embedded
/// controller — a `UISearchController` installed on its `navigationItem`
/// then collapses to zero height.
///
/// Returns the adopted controllers in pre-order; the caller sends
/// `didMoveToParentViewController:` once the views are in place, and unwinds
/// the list in reverse on detach.
#[cfg(target_os = "ios")]
pub(crate) fn adopt_controllers(
    root: &PlatformView,
) -> Vec<Retained<cocoa_ui::objc2_ui_kit::UIViewController>> {
    use cocoa_ui::uikit::view_controller::{add_child, enclosing_controller, owning_controller};
    let mut adopted = Vec::new();
    let mut stack = vec![cocoa_ui::view::retain_base(root)];
    while let Some(view) = stack.pop() {
        if let Some(controller) = owning_controller(&view)
            && controller.parentViewController().is_none()
            && let Some(parent_view) = view.superview()
            && let Some(enclosing) = enclosing_controller(&parent_view)
            && Retained::as_ptr(&enclosing) != Retained::as_ptr(&controller)
        {
            add_child(&enclosing, &controller);
            adopted.push(controller);
        }
        stack.extend(
            view.subviews()
                .iter()
                .map(|subview| cocoa_ui::view::retain_base(&subview)),
        );
    }
    adopted
}

/// What a handler sees while it renders.
pub struct RenderContext<'a> {
    env: &'a Environment,
    dispatcher: Rc<crate::dispatch::Dispatcher>,
    mtm: cocoa_ui::MainThreadMarker,
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
        mtm: cocoa_ui::MainThreadMarker,
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

    /// Proof of the main thread, for kit calls that require one.
    #[must_use]
    pub const fn mtm(&self) -> cocoa_ui::MainThreadMarker {
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
    mtm: cocoa_ui::MainThreadMarker,
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
