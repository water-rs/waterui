//! `Native<FixedContainer>` / `Native<LazyContainer>` — containers laid
//! out by `waterui-layout` in Rust.
//!
//! The platform object is `dev.waterui.android.RustViewGroup`, a `FrameLayout`
//! whose `onMeasure`/`onLayout` forward into this module. The container's own
//! measure answers `layout.size_that_fits` in dp, reported through JNI in px;
//! the layout pass answers `layout.place` and pushes each child's frame —
//! and the proposal the layout selected for it, per `docs/layout-spec.md`
//! rule L-2 — down to the child views.
//!
//! `FixedContainer` mounts its `Vec<AnyView>` once. `LazyContainer` carries
//! a live [`AnyViews`] collection instead: the leaf renders children
//! incrementally through the collection's `watch`/`get_view`/`get_id`
//! interface, keyed by item id — an id the collection still reports keeps
//! its mounted child, a new id materializes at its index, and a vanished
//! id's child is dropped — the id-keyed reconcile the Apple port calls
//! `syncChildren`.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use jni::objects::JObject;
use jni::sys::{jint, jlong};
use nami::Computed;
use nami::Signal as _;
use nami::watcher::BoxWatcherGuard;
use waterui::views::{AnyViews, AnyViewsSnapshot, ViewSnapshot, Views};
use waterui_backend_core::AnyView;
use waterui_core::layout::{
    Layout, LayoutDirection, Point, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions,
    measure_layout, with_memoized_children,
};
use waterui_layout::container::{FixedContainer, LazyContainer};

use crate::contract::{Mounted, NativeLeaf, PlatformView, Renderer};
use crate::dispatch::Dispatcher;
use crate::jvm::{self, Platform};

/// An item's erased collection id — the key `get_id` reports and
/// `rendered` is keyed by.
type ItemId = <AnyViews<AnyView> as Views>::Id;

/// The shared state the JNI callbacks reach through the handle the
/// `RustViewGroup` carries.
struct ContainerState {
    /// The layout object the `Layout` impl produced.
    layout: Box<dyn Layout>,
    /// The rendered children, in container order — the fixed path.
    children: Vec<Mounted>,
    /// The collection-backed child set — the lazy path.
    lazy: Option<LazyState>,
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
        let mut fmt = f.debug_struct("ContainerState");
        match &self.lazy {
            Some(lazy) => fmt.field("children", &lazy.order.len()),
            None => fmt.field("children", &self.children.len()),
        };
        fmt.finish_non_exhaustive()
    }
}

/// The lazy path's child set: the id-keyed mounted children, in the state
/// so the watch — and only the watch — mutates them. The collection
/// itself lives on the leaf's `KeepAlive`, not here.
struct LazyState {
    /// The render capability `get_view` results are realized through.
    renderer: Renderer,
    /// The container view children mount on.
    host: PlatformView,
    /// The materialized children by id — every mounted child lives here.
    rendered: HashMap<ItemId, Mounted>,
    /// The collection's current ids, in display order.
    order: Vec<ItemId>,
    /// The id sequence the last reconcile applied — a same-ids emission
    /// touches no platform child.
    item_ids: Vec<ItemId>,
    /// The resolved logical layout direction — the core hands it to the
    /// backend unwrapped, so placement mirrors the way `DirectionalLayout`
    /// does for a fixed container.
    direction: Computed<LayoutDirection>,
}

/// The watch side of the lazy path — outside `ContainerState`'s
/// `RefCell` on purpose: a child's own mount can emit back into this
/// container's collection while a reconcile holds the state, and the
/// reentrant emission has to park without touching the borrowed state —
/// the same requeue `reconcileChildren` makes on the main queue in the
/// Apple port.
struct LazySync {
    /// Emissions waiting for the in-flight reconcile to release.
    pending: RefCell<VecDeque<AnyViewsSnapshot<AnyView>>>,
    /// Whether a reconcile owns the queue right now.
    reconciling: Cell<bool>,
}

/// The children's `Mounted`s, in container order, for either path.
fn ordered_children(state: &ContainerState) -> Vec<&Mounted> {
    state.lazy.as_ref().map_or_else(
        || state.children.iter().collect(),
        |lazy| lazy.order.iter().map(|id| &lazy.rendered[id]).collect(),
    )
}

/// The children's layout faces, in order.
fn children(state: &ContainerState) -> Vec<&dyn SubView> {
    ordered_children(state)
        .into_iter()
        .map(Mounted::layout)
        .collect()
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
            &*crate::handle::jlong_to_pointer::<RefCell<ContainerState>>(handle).cast_const()
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
    outcome.resolve::<crate::policy::ThrowRuntimeExAndDefault>()
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
            &*crate::handle::jlong_to_pointer::<RefCell<ContainerState>>(handle).cast_const()
        };
        let platform = state.borrow().platform.clone();
        let state = state.borrow();
        let mounted = ordered_children(&state);
        if mounted.is_empty() {
            return Ok(());
        }
        // `onLayout` reports the group's rect in its parent's coordinate
        // space; the layout engine places children in bounds space and
        // `View.layout` writes them in the group's local space, so the
        // bounds handed to `place` must be the local rect (0, 0, w, h) —
        // the same space `view.bounds` supplies on Apple.
        let bounds = Rect::from_size(Size::new(
            platform.px_to_dp(right - left),
            platform.px_to_dp(bottom - top),
        ));
        // The proposal the parent selected, else the frame itself — the same
        // fallback `perform_fixed_layout` makes natively hosted.
        let proposal = state
            .selected
            .get()
            .unwrap_or_else(|| ProposalSize::new(Some(bounds.width()), Some(bounds.height())));
        let child_faces = children(&state);
        // `size_that_fits` warms the same measurement pass `place` reads —
        // `measure_layout`'s own call.
        let mut placements = with_memoized_children(&child_faces, |memoized| {
            let _ = state.layout.size_that_fits(proposal, memoized);
            state.layout.place(bounds, proposal, memoized)
        });
        assert_eq!(
            placements.len(),
            mounted.len(),
            "container layout returned {} placements for {} children",
            placements.len(),
            mounted.len()
        );
        // A lazy container's layout is not `DirectionalLayout`-wrapped —
        // the core exposes the direction signal so the backend mirrors,
        // the same formula `DirectionalLayout::mirror` applies.
        if let Some(lazy) = &state.lazy
            && lazy.direction.snapshot().is_right_to_left()
        {
            for placement in &mut placements {
                placement.frame = Rect::new(
                    Point::new(
                        bounds.min_x() + bounds.max_x() - placement.frame.max_x(),
                        placement.frame.y(),
                    ),
                    *placement.frame.size(),
                );
            }
        }
        for (child, placement) in mounted.iter().zip(placements.iter()) {
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
    outcome.resolve::<crate::policy::ThrowRuntimeExAndDefault>();
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
        // `LazyContainer::stretch_axis` answers `layout.stretch_axis(&[])`
        // — the collection cannot be enumerated without materializing it.
        if state.lazy.is_some() {
            return state.layout.stretch_axis(&[]);
        }
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
        let mounted = ordered_children(&state);
        !mounted.is_empty() && mounted.iter().all(|child| child.layout().is_empty())
    }
}

/// Claims `Native<FixedContainer>` and `Native<LazyContainer>`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<FixedContainer>(|container, ctx| {
        let (layout, contents) = container.into_inner();
        render_container(layout, contents, ctx)
    });
    dispatcher.register_native::<LazyContainer>(|container, ctx| {
        let direction = container.direction();
        let (layout, contents) = container.into_inner();
        render_lazy_container(layout, contents, direction, ctx)
    });
}

/// The container's platform object: a `RustViewGroup` that does not clip —
/// the layout writes frames directly, it does not clip them.
fn new_group(platform: &Rc<Platform>) -> PlatformView {
    let group = jvm::with_env(|env| {
        let group = platform
            .new_rust_view_group(env)
            .expect("a RustViewGroup constructs against the host context");
        platform
            .bindings()
            .set_clip_children(env, group.as_ref(), false)
            .expect("setClipChildren must not throw");
        group
    });
    jvm::retain(&group)
}

/// The state plus the two watches every container installs: the L-2
/// proposal sink and the layout's own invalidation signal.
fn wire_container_state(
    layout: Box<dyn Layout>,
    lazy: Option<LazyState>,
    platform: &Rc<Platform>,
    group: &PlatformView,
) -> Rc<RefCell<ContainerState>> {
    let state = Rc::new(RefCell::new(ContainerState {
        layout,
        children: Vec::new(),
        lazy,
        selected: Cell::new(None),
        invalidation_guards: Vec::new(),
        sink_guard: None,
        platform: platform.clone(),
    }));

    // The proposal channel: the parent delivers the selected proposal,
    // the state stores it and asks for a relayout.
    let sink_guard = platform.proposals().register_sink(group, {
        let state = Rc::clone(&state);
        let view = jvm::retain(group);
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
            let view = jvm::retain(group);
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
    state
}

/// The leaf assembly every path ends with: the JNI handle the
/// `RustViewGroup` calls back through, then the leaf that owns it.
fn finish_container(
    state: &Rc<RefCell<ContainerState>>,
    group: PlatformView,
    platform: &Rc<Platform>,
) -> NativeLeaf {
    // The handle the Kotlin view hands back on every callback: the
    // shared state, kept alive by the leaf through `OwnedHandle`.
    let state_ptr = Rc::into_raw(Rc::clone(state));
    jvm::with_env(|env| {
        platform
            .bindings()
            .set_handle(
                env,
                group.as_ref(),
                crate::handle::pointer_to_jlong(state_ptr),
            )
            .expect("setHandle must not throw");
    });

    let mut leaf = NativeLeaf::new(
        jvm::retain(&group),
        ContainerSubView {
            state: Rc::clone(state),
        },
        platform,
    );
    leaf.keep(OwnedHandle { group, state_ptr });
    leaf
}

/// The fixed path: `layout` plus the already-materialized child views
/// become one `RustViewGroup` leaf.
fn render_container(
    layout: Box<dyn Layout>,
    contents: Vec<AnyView>,
    ctx: &crate::contract::RenderContext<'_>,
) -> NativeLeaf {
    let platform = ctx.platform().clone();
    let group = new_group(&platform);
    let state = wire_container_state(layout, None, &platform, &group);

    let renderer = ctx.renderer();
    let children: Vec<Mounted> = contents
        .into_iter()
        .map(|view| renderer.render(view).mount(&group))
        .collect();
    state.borrow_mut().children = children;

    finish_container(&state, group, &platform)
}

/// The lazy path: the live collection renders incrementally — the watch
/// reconciles mounted children by id on every emission.
fn render_lazy_container(
    layout: Box<dyn Layout>,
    contents: AnyViews<AnyView>,
    direction: Computed<LayoutDirection>,
    ctx: &crate::contract::RenderContext<'_>,
) -> NativeLeaf {
    let platform = ctx.platform().clone();
    let group = new_group(&platform);
    let contents = Rc::new(contents);
    let lazy = LazyState {
        renderer: ctx.renderer(),
        host: jvm::retain(&group),
        rendered: HashMap::new(),
        order: Vec::new(),
        item_ids: Vec::new(),
        direction,
    };
    let state = wire_container_state(layout, Some(lazy), &platform, &group);

    // `watchAnyViewsIds` — the collection's membership watch. Each
    // emission hands over an owning snapshot covering the watched range;
    // reconcile the mounted set by id against it. An emission arriving
    // while a reconcile holds the state parks in `pending` — the queue
    // sits outside the state's RefCell so the reentrant push never
    // borrows it. `watch` runs through `contents` — a registration-time
    // `populated` emission must not meet a held borrow of `state`.
    let sync = Rc::new(LazySync {
        pending: RefCell::new(VecDeque::new()),
        reconciling: Cell::new(false),
    });
    let watcher = contents.watch(.., {
        let state = Rc::clone(&state);
        let sync = Rc::clone(&sync);
        move |ctx, _change| {
            sync.pending.borrow_mut().push_back(ctx.into_value());
            // An in-flight reconcile drains the queue — return.
            if sync.reconciling.replace(true) {
                return;
            }
            while let Some(snapshot) = sync.pending.borrow_mut().pop_front() {
                sync_children(&mut state.borrow_mut(), &snapshot);
            }
            sync.reconciling.set(false);
        }
    });

    // `reloadChildrenFromRust` — the initial population, through the same
    // queue the watch drains: an emission raised from inside a mounted
    // child parks behind it instead of racing the state's borrow.
    sync.pending.borrow_mut().push_back(contents.snapshot());
    sync.reconciling.replace(true);
    while let Some(snapshot) = sync.pending.borrow_mut().pop_front() {
        sync_children(&mut state.borrow_mut(), &snapshot);
    }
    sync.reconciling.set(false);

    let mut leaf = finish_container(&state, group, &platform);
    leaf.keep(watcher);
    // The `Rc` the watch was registered through: as long as the leaf
    // lives, the collection and its id mapping stay alive for the
    // subscriptions the guard holds.
    leaf.keep(contents);
    leaf
}

/// `syncChildren`: the id-keyed reconcile — every id the snapshot still
/// reports keeps its mounted child, every new id materializes at its
/// index, every vanished id's `Mounted` drops (detaching the view), and
/// the host's subviews become exactly the ordered set.
fn sync_children(state: &mut ContainerState, snapshot: &AnyViewsSnapshot<AnyView>) {
    let platform = state.platform.clone();
    let lazy = state.lazy.as_mut().expect("syncing a lazy container");
    let range = snapshot.range();
    let ids: Vec<ItemId> = range
        .clone()
        .filter_map(|index| snapshot.get_id(index))
        .collect();
    if ids == lazy.item_ids {
        // Membership and order already match — a content-only emission
        // touches no platform child; row data stays live through its own
        // signals.
        return;
    }
    let mut seen = HashSet::with_capacity(ids.len());
    for (offset, &id) in ids.iter().enumerate() {
        assert!(
            seen.insert(id),
            "duplicate child view id in container: {id:?}"
        );
        if !lazy.rendered.contains_key(&id) {
            let index = range.start + offset;
            let view = snapshot
                .get_view(index)
                .expect("a live lazy index materializes");
            lazy.rendered
                .insert(id, lazy.renderer.render(view).mount(&lazy.host));
        }
    }
    lazy.rendered.retain(|id, _| seen.contains(id));

    // Make the host's subviews exactly the ordered set. `Mounted`'s drops
    // above already removed the departures and `mount` appended the
    // arrivals, so a child already at its index needs no work — the tail
    // append is the common zero-reorder path.
    let bindings = platform.bindings();
    jvm::with_env(|env| {
        for (index, &id) in ids.iter().enumerate() {
            let wanted =
                jint::try_from(index).expect("a container holds fewer than i32::MAX children");
            let view = lazy.rendered[&id].view();
            if bindings.index_of_child(env, lazy.host.as_ref(), view.as_ref())? != wanted {
                bindings.remove_view(env, lazy.host.as_ref(), view.as_ref())?;
                bindings.add_view_at(env, lazy.host.as_ref(), view.as_ref(), wanted)?;
            }
        }
        jni::errors::Result::Ok(())
    })
    .expect("lazy child reorder must not throw");

    lazy.item_ids = ids.clone();
    lazy.order = ids;
    platform.invalidate_measures();
    jvm::with_env(|env| {
        platform
            .bindings()
            .request_layout(env, lazy.host.as_ref())
            .expect("requestLayout must not throw");
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
