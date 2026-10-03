//! The `dynamic` leaf: `Native<Dynamic>` rendered through a host
//! `FrameLayout`.
//!
//! Mirrors the Apple backend's `dynamic` port at skeleton scale: a host
//! container that owns at most one mounted child. `Dynamic::connect`
//! delivers each `set` as a `Context<AnyView>`; the receiver renders the
//! view through the captured [`Renderer`], drops the previous [`Mounted`]
//! (detaching its view), and mounts the new leaf. The leaf's layout face
//! delegates to the current child and measures empty until one arrives. A
//! render miss keeps the previous child.
//!
//! The host is a plain `FrameLayout`, not a `RustViewGroup`: the group is
//! the container's own bridge, whose `handle` is typed to `ContainerState`
//! — a dynamic leaf has one child filling its bounds, which is exactly
//! `FrameLayout`'s own `onMeasure`/`onLayout`, so no Kotlin bridge is
//! needed here.
//!
//! The receiver holds only a `Weak` into the leaf state: the handler's
//! `Rc` owns the receiver for the handler's life, so a strong capture
//! would keep the leaf's platform view alive after the leaf drops — the
//! `[weak self]` of the Swift port.

use alloc::rc::{Rc, Weak};
use core::cell::{Cell, RefCell};

use waterui::component::Dynamic;
use waterui_backend_core::AnyView;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf, PlatformView, Renderer};
use crate::dispatch::Dispatcher;
use crate::jvm::{self, Platform};

/// The leaf's live state.
struct DynamicState {
    /// The render capability each delivered `AnyView` is realized through.
    renderer: Renderer,
    /// The host view — the leaf's platform object.
    host: PlatformView,
    /// The mounted child the leaf's layout delegates to.
    child: Option<Mounted>,
    /// The proposal the parent layout selected when it placed this leaf —
    /// forwarded to the current child and re-applied to every replacement
    /// so a swapped-in child is never left with a stale offer.
    selected: Cell<Option<ProposalSize>>,
    /// The platform behind `renderer` — measure invalidation and proposal
    /// delivery go through it.
    platform: Rc<Platform>,
}

impl core::fmt::Debug for DynamicState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicState").finish_non_exhaustive()
    }
}

/// The leaf's layout face: delegates every answer to the current child's
/// `SubView`, measuring empty until a child is mounted.
struct DynamicSubView {
    /// The leaf's state.
    state: Rc<RefCell<DynamicState>>,
}

impl core::fmt::Debug for DynamicSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicSubView").finish_non_exhaustive()
    }
}

impl SubView for DynamicSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.as_ref().map_or_else(
            || ViewDimensions::new(Size::new(0.0, 0.0)),
            |child| child.layout().measure(proposal),
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state
            .borrow()
            .child
            .as_ref()
            .map_or(StretchAxis::None, |child| child.layout().stretch_axis())
    }

    fn priority(&self) -> i32 {
        self.state
            .borrow()
            .child
            .as_ref()
            .map_or(0, |child| child.layout().priority())
    }

    fn is_empty(&self) -> bool {
        self.state
            .borrow()
            .child
            .as_ref()
            .is_some_and(|child| child.layout().is_empty())
    }
}

/// Renders `view`, detaches the previous child, mounts the new leaf into
/// the host, then re-forwards the negotiated proposal and invalidates up
/// the hierarchy. A render miss keeps the previous child.
fn update_child(state: &Rc<RefCell<DynamicState>>, view: AnyView) {
    let (host, platform) = {
        let mut state = state.borrow_mut();
        let Some(leaf) = state.renderer.try_render(view) else {
            return;
        };
        // `Mounted`'s drop detaches the previous child's view.
        drop(state.child.take());
        let mounted = leaf.mount(&state.host);
        // The replacement inherits the last negotiated proposal until the
        // parent re-places us.
        if let Some(selected) = state.selected.get() {
            state.platform.proposals().deliver(mounted.view(), selected);
        }
        state.child = Some(mounted);
        // The delegate's answers may have changed: the parent must
        // re-measure, and the host must re-lay out its new child.
        state.platform.invalidate_measures();
        (jvm::retain(&state.host), state.platform.clone())
    };
    jvm::with_env(|env| {
        platform
            .bindings()
            .request_layout(env, host.as_ref())
            .expect("requestLayout must not throw");
    });
}

/// Installs the `dynamic` handler on the dispatcher: `Native<Dynamic>`
/// becomes a host whose single child the handler's receiver swaps per
/// delivered `Context<AnyView>`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Dynamic>(|dynamic, ctx| {
        let platform = ctx.platform().clone();
        let host = jvm::with_env(|env| {
            platform
                .new_frame_layout(env)
                .expect("a FrameLayout constructs against the host context")
        });

        let state = Rc::new(RefCell::new(DynamicState {
            renderer: ctx.renderer(),
            host: jvm::retain(&host),
            child: None,
            selected: Cell::new(None),
            platform: platform.clone(),
        }));

        // The proposal channel: store the negotiated offer and forward it
        // to the current child.
        let sink_guard = platform.proposals().register_sink(&host, {
            let state = Rc::clone(&state);
            let platform = platform.clone();
            move |selected| {
                let state = state.borrow();
                state.selected.set(Some(selected));
                if let Some(child) = &state.child {
                    platform.proposals().deliver(child.view(), selected);
                }
            }
        });

        // The leaf connects as the handler's receiver; a `Weak` capture so
        // the receiver never outlives the leaf's platform view. The
        // pre-connection view, if any, is delivered through the same queue.
        let weak: Weak<RefCell<DynamicState>> = Rc::downgrade(&state);
        dynamic.connect(move |ctx| {
            if let Some(state) = weak.upgrade() {
                update_child(&state, ctx.into_value());
            }
        });

        let mut leaf = NativeLeaf::new(
            host,
            DynamicSubView {
                state: Rc::clone(&state),
            },
            &platform,
        );
        leaf.keep(state);
        leaf.keep(sink_guard);
        leaf
    });
}
