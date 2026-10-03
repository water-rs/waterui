//! `Native<FixedContainer>` — the stack container, laid out by
//! `waterui-layout` in Rust.
//!
//! The platform object is `dev.waterui.android.RustViewGroup`, a `FrameLayout`
//! whose `onMeasure`/`onLayout` forward into this module. The container's own
//! measure answers `layout.size_that_fits` in dp, reported through JNI in px;
//! the layout pass answers `layout.place` and pushes each child's frame —
//! and the proposal the layout selected for it, per `docs/layout-spec.md`
//! rule L-2 — down to the child views.
//!
//! Scope of the skeleton: only `FixedContainer` (`vstack`/`hstack`/`zstack`
//! resolve into it). `LazyContainer` — collections that materialize children
//! on demand — needs the platform's own virtualization (`RecyclerView`), and
//! lands with the collection port.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use jni::objects::JObject;
use jni::sys::{jint, jlong};
use nami::watcher::BoxWatcherGuard;
use waterui_core::layout::{
    Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions, measure_layout,
    with_memoized_children,
};
use waterui_layout::container::FixedContainer;

use crate::contract::{Mounted, NativeLeaf, PlatformView};
use crate::dispatch::Dispatcher;
use crate::jvm::{self, Platform};

/// The shared state the JNI callbacks reach through the handle the
/// `RustViewGroup` carries.
struct ContainerState {
    /// The layout object the `Layout` impl produced.
    layout: Box<dyn Layout>,
    /// The rendered children, in container order.
    children: Vec<Mounted>,
    /// The proposal the parent selected for this container, delivered before
    /// the frame write — the L-2 channel.
    selected: Cell<Option<ProposalSize>>,
    /// `layout.watch_invalidation`'s guards — held, never read.
    invalidation_guards: Vec<BoxWatcherGuard>,
    /// The proposal sink's guard — held, never read.
    sink_guard: Option<crate::proposal::SinkGuard>,
    /// The runtime's JNI surface — the callbacks' way to unit conversion,
    /// the identifier table and the proposal map.
    platform: Rc<Platform>,
}

impl core::fmt::Debug for ContainerState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContainerState")
            .field("children", &self.children.len())
            .finish_non_exhaustive()
    }
}

/// The children's layout faces, in order.
fn children(state: &ContainerState) -> Vec<&dyn SubView> {
    state.children.iter().map(Mounted::layout).collect()
}

/// `RustViewGroup.onMeasure`: the spec pair becomes a proposal, the layout
/// answers its size, and px go back packed into a `jlong`.
///
/// # Safety
///
/// `handle` is the container's `ContainerState`, live for the view's life.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_RustViewGroup_nativeMeasure<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _this: JObject<'caller>,
    handle: jlong,
    width_spec: jint,
    height_spec: jint,
) -> jlong {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<jlong> {
        // SAFETY: the view's handle is set at construction and stays valid
        // until the leaf that owns the view is dropped — a detached view is
        // never measured.
        let state = unsafe {
            &*(usize::try_from(handle).expect("a container handle is an `Rc::into_raw` pointer")
                as *const RefCell<ContainerState>)
        };
        let platform = state.borrow().platform.clone();
        let proposal =
            crate::native_layout::proposal_from_specs(env, &platform, width_spec, height_spec)?;
        let state = state.borrow();
        let measured = measure_layout(&*state.layout, proposal, &children(&state));
        Ok(crate::native_layout::pack_measured(
            platform.dp_to_px(measured.size.width),
            platform.dp_to_px(measured.size.height),
        ))
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// `RustViewGroup.onLayout`: place the children inside the frame and write
/// each frame — plus its selected proposal — to the child view.
///
/// # Safety
///
/// `handle` is the container's `ContainerState`, live for the view's life.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_RustViewGroup_nativeLayout<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _this: JObject<'caller>,
    handle: jlong,
    left: jint,
    top: jint,
    right: jint,
    bottom: jint,
) {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        // SAFETY: same handle contract as `nativeMeasure`.
        let state = unsafe {
            &*(usize::try_from(handle).expect("a container handle is an `Rc::into_raw` pointer")
                as *const RefCell<ContainerState>)
        };
        let platform = state.borrow().platform.clone();
        let state = state.borrow();
        if state.children.is_empty() {
            return Ok(());
        }
        let bounds = Rect::new(
            Point::new(platform.px_to_dp(left), platform.px_to_dp(top)),
            Size::new(
                platform.px_to_dp(right - left),
                platform.px_to_dp(bottom - top),
            ),
        );
        // The proposal the parent selected, else the frame itself — the same
        // fallback `perform_fixed_layout` makes natively hosted.
        let proposal = state
            .selected
            .get()
            .unwrap_or_else(|| ProposalSize::new(Some(bounds.width()), Some(bounds.height())));
        let child_faces = children(&state);
        // `size_that_fits` warms the same measurement pass `place` reads —
        // `measure_layout`'s own call.
        let placements = with_memoized_children(&child_faces, |memoized| {
            let _ = state.layout.size_that_fits(proposal, memoized);
            state.layout.place(bounds, proposal, memoized)
        });
        assert_eq!(
            placements.len(),
            state.children.len(),
            "container layout returned {} placements for {} children",
            placements.len(),
            state.children.len()
        );
        for (child, placement) in state.children.iter().zip(placements.iter()) {
            // Rule L-2: the selected proposal reaches the child before its
            // frame does.
            platform
                .proposals()
                .deliver(child.view(), placement.proposal);
            platform.bindings().layout(
                env,
                child.view().as_ref(),
                platform.dp_to_px(placement.frame.x()),
                platform.dp_to_px(placement.frame.y()),
                platform.dp_to_px(placement.frame.x() + placement.frame.width()),
                platform.dp_to_px(placement.frame.y() + placement.frame.height()),
            )?;
        }
        Ok(())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// The container's `SubView` — what the parent measures and stretches.
struct ContainerSubView {
    state: Rc<RefCell<ContainerState>>,
}

impl SubView for ContainerSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = self.state.borrow();
        measure_layout(&*state.layout, proposal, &children(&state))
    }

    fn stretch_axis(&self) -> StretchAxis {
        let state = self.state.borrow();
        let axes: Vec<StretchAxis> = state
            .children
            .iter()
            .map(|child| child.layout().stretch_axis())
            .collect();
        state.layout.stretch_axis(&axes)
    }

    fn priority(&self) -> i32 {
        0
    }

    fn is_empty(&self) -> bool {
        let state = self.state.borrow();
        !state.children.is_empty() && state.children.iter().all(|child| child.layout().is_empty())
    }
}

/// Claims `Native<FixedContainer>`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<FixedContainer>(|container, ctx| {
        let platform = ctx.platform().clone();
        let (layout, contents) = container.into_inner();
        let group = jvm::with_env(|env| {
            let group = platform
                .new_rust_view_group(env)
                .expect("a RustViewGroup constructs against the host context");
            // Children may draw past their bounds — the layout writes frames
            // directly, it does not clip them.
            platform
                .bindings()
                .set_clip_children(env, group.as_ref(), false)
                .expect("setClipChildren must not throw");
            group
        });
        let group = jvm::retain(&group);

        let state = Rc::new(RefCell::new(ContainerState {
            layout,
            children: Vec::new(),
            selected: Cell::new(None),
            invalidation_guards: Vec::new(),
            sink_guard: None,
            platform: platform.clone(),
        }));

        // The proposal channel: the parent delivers the selected proposal,
        // the state stores it and asks for a relayout.
        let sink_guard = platform.proposals().register_sink(&group, {
            let state = Rc::clone(&state);
            let view = jvm::retain(&group);
            let platform = platform.clone();
            move |selected| {
                let state = state.borrow();
                if state.selected.get() != Some(selected) {
                    state.selected.set(Some(selected));
                    jvm::with_env(|env| {
                        platform
                            .bindings()
                            .request_layout(env, view.as_ref())
                            .expect("requestLayout must not throw");
                    });
                }
            }
        });
        state.borrow_mut().sink_guard = Some(sink_guard);

        // Layout invalidation: the layout's own signals changed — bump the
        // measure epoch and request a relayout.
        let invalidation_guards = {
            let state = state.borrow();
            state.layout.watch_invalidation(Rc::new({
                let view = jvm::retain(&group);
                let platform = platform.clone();
                move || {
                    platform.invalidate_measures();
                    jvm::with_env(|env| {
                        platform
                            .bindings()
                            .request_layout(env, view.as_ref())
                            .expect("requestLayout must not throw");
                    });
                }
            }))
        };
        state.borrow_mut().invalidation_guards = invalidation_guards;

        // Render and mount every child — the fixed path materializes once.
        let renderer = ctx.renderer();
        let children: Vec<Mounted> = contents
            .into_iter()
            .map(|view| renderer.render(view).mount(&group))
            .collect();
        state.borrow_mut().children = children;

        // The handle the Kotlin view hands back on every callback: the
        // shared state, kept alive by the leaf through `Mounted`.
        let state_ptr = Rc::into_raw(Rc::clone(&state));
        jvm::with_env(|env| {
            platform
                .bindings()
                .set_handle(env, group.as_ref(), state_ptr as jlong)
                .expect("setHandle must not throw");
        });

        let mut leaf = NativeLeaf::new(
            jvm::retain(&group),
            ContainerSubView {
                state: Rc::clone(&state),
            },
            &platform,
        );
        leaf.keep(OwnedHandle { group, state_ptr });
        leaf
    });
}

/// Owns the container view plus the raw `ContainerState` pointer the view
/// calls back through — dropping it releases the pointer after the view is
/// detached (`KeepAlive` drops last-kept first, and the `Mounted` handles
/// dropped earlier already detached children).
struct OwnedHandle {
    group: PlatformView,
    state_ptr: *const RefCell<ContainerState>,
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = &self.group;
        // SAFETY: the pointer came from `Rc::into_raw` above and is
        // released exactly once, here.
        drop(unsafe { Rc::from_raw(self.state_ptr) });
    }
}
