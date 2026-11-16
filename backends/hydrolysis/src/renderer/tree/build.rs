//! Building the retained tree: [`RenderNode::build`] downcasts a dispatched
//! `View` onto the closed node set, plus the structural builders (wrapper,
//! env scope, collection, lazy stack, scene/GPU/effect, `Dynamic` host).

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
use super::collection::{item_id, item_view_or_empty};
#[allow(clippy::wildcard_imports)]
use super::*;
#[cfg(hydrolysis_gpu)]
use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::platform_view::PlatformView;
#[cfg(feature = "calendar")]
use waterui::component::form::CalendarGrid;
#[cfg(feature = "multi-date-picker")]
use waterui::component::form::picker::MultiDatePickerGrid;
use waterui_core::contract_violation;
use waterui_core::views::ViewSnapshot;

mod backend;

/// A retained node's layout-dirty mark and the callback its input
/// subscriptions fire: the callback sets the mark, which an enclosing
/// `RetainedSubview` consumes through [`RenderNode::take_layout_dirty`], and
/// marks the owning node's cell layout-dirty so the pump relayouts it.
fn layout_invalidation<E: crate::text::TextEngine>(
    cell: &Rc<NodeCell<E>>,
) -> (
    Rc<Cell<bool>>,
    waterui_core::layout::LayoutInvalidationCallback,
) {
    let layout_dirty = Rc::new(Cell::new(false));
    let invalidate = {
        let layout_dirty = Rc::clone(&layout_dirty);
        let cell = Rc::downgrade(cell);
        Rc::new(move || {
            layout_dirty.set(true);
            if let Some(cell) = cell.upgrade() {
                cell.mark_layout();
            }
        })
    };
    (layout_dirty, invalidate)
}

impl<E: crate::text::TextEngine> RenderNode<E> {
    /// Builds a node from a view, capturing live reactive inputs. Native
    /// leaves and layout containers map to concrete nodes; composite views
    /// expand via `body()` once and recurse. Then it wires the subtree's cell
    /// parent links (`attach_subtree`): the tree root attaches to the
    /// window's root cell, every other node to its structural parent's cell.
    pub(crate) fn build(view: AnyView, env: &Environment, renderer: &mut SemanticCore<E>) -> Self {
        let node = Self::build_view(view, env, renderer);
        node.attach_subtree();
        node
    }

    /// Builds `view` through [`backend::HydroBackend`]: `View::dispatch`
    /// expands composites and hands each payload to its method.
    fn build_view(view: impl View, env: &Environment, renderer: &mut SemanticCore<E>) -> Self {
        backend::HydroBackend::new(renderer).build(view, env)
    }

    /// Build a transparent wrapper node: capture the per-flush effect and recurse
    /// into the child so reactive descendants reach their own dedicated nodes.
    ///
    /// A handler-carrying effect (hover/`on_tap`/gesture, drop destination,
    /// context menu, anchored overlay, lifecycle hook — the ones capturing `env`
    /// for a callback) stores the environment its *content* resolves in, not the
    /// env at the wrapper itself: `.state(&s)` between the view and the handler
    /// is part of the handler's environment, so `v.state(&s).on_x(h)` and
    /// `v.on_x(h).state(&s)` dispatch identically (water-rs/waterui#1292). The
    /// stored env is also handed to the child at flush/measure — a peeled `Env`
    /// or handler child carries that same env on its own node, and an unpeeled
    /// first node means the env is unchanged, so the child sees no new scope.
    fn build_wrapper(
        effect: WrapperEffect<E>,
        content: AnyView,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        let child = Self::build_view(content, env, renderer);
        Self::wrap(effect, child, env, renderer)
    }

    /// Wraps the already built `child` in a [`WrapperNode`] applying
    /// `effect`, for an effect that reads the child it wraps.
    fn wrap(
        effect: WrapperEffect<E>,
        child: Self,
        env: &Environment,
        renderer: &SemanticCore<E>,
    ) -> Self {
        let env = if effect.captures_environment() {
            Self::resolved_handler_env(&child, env).clone()
        } else {
            env.clone()
        };
        let is_material_group = matches!(effect, WrapperEffect::MaterialGroup);
        let node = Self::Wrapper(Box::new(WrapperNode {
            accessibility_identity: Rc::new(()),
            core: renderer.new_core(),
            effect,
            env,
            released_offsets: Cell::default(),
            child,
        }));
        if is_material_group {
            // The marker is a property of the node, not of a record: flag the
            // cell at build so a partial descent into a descendant subtree can
            // find every enclosing scope from the ancestry (#2268).
            node.core().cell.material_group_scope.set(true);
        }
        node
    }

    /// The identity gesture observers on `self` group by: the first node
    /// below the layout-transparent wrappers (environment scopes, retention,
    /// effect wrappers and animated transforms), so every observer stacked
    /// on one view joins one group.
    fn gesture_group_identity(&self) -> usize {
        match self {
            Self::Env(node) => node.child.gesture_group_identity(),
            Self::Retain(node) => node.child.gesture_group_identity(),
            Self::Wrapper(node) => node.child.gesture_group_identity(),
            Self::Opacity(node) => node.child.gesture_group_identity(),
            Self::Scale(node) => node.child.gesture_group_identity(),
            Self::Rotation(node) => node.child.gesture_group_identity(),
            Self::Offset(node) => node.child.gesture_group_identity(),
            _ => Rc::as_ptr(&self.core().cell) as usize,
        }
    }

    /// The group identity of a gesture observer wrapper, with the node it
    /// wraps; `None` for any other node.
    #[cfg(test)]
    pub(crate) fn observed_gesture_group(&self) -> Option<(usize, &Self)> {
        match self {
            Self::Wrapper(node) => match &node.effect {
                WrapperEffect::GestureObserver(effect) => {
                    Some((effect.gesture_group_identity, &node.child))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The environment the content of `self` resolves in below its leading
    /// environment scopes — `Env` nodes and `.material_group()` markers —
    /// or `None` when `self` opens with neither.
    pub(crate) fn leading_scope_env(&self) -> Option<&Environment> {
        match self {
            Self::Env(node) => Some(node.child.leading_scope_env().unwrap_or(&node.env)),
            Self::Wrapper(node) if matches!(node.effect, WrapperEffect::MaterialGroup) => {
                node.child.leading_scope_env()
            }
            _ => None,
        }
    }

    /// The environment a metadata-carried callback captures: the one its
    /// modified view resolves in. Walks down the child's leading env-only
    /// wrappers — `Env` nodes (`With`/`Metadata<Environment>` installs and
    /// env-scoped metadata), `.material_group()` scope markers and handler
    /// wrappers of the same modifier chain — until the first node that is
    /// none of them. Resolving on the *built* node means each `With` was
    /// already expanded exactly once into the `Env` node that carries its
    /// scoped env.
    fn resolved_handler_env<'a>(node: &'a Self, env: &'a Environment) -> &'a Environment {
        match node {
            Self::Env(node) => Self::resolved_handler_env(&node.child, &node.env),
            // Retention is transparent to handler resolution.
            Self::Retain(node) => Self::resolved_handler_env(&node.child, env),
            Self::Wrapper(node) if node.effect.captures_environment() => {
                Self::resolved_handler_env(&node.child, &node.env)
            }
            // A `.material_group()` is a pure scope marker — a handler
            // wrapped through it still captures the env its modifier chain
            // resolved.
            Self::Wrapper(node) if matches!(node.effect, WrapperEffect::MaterialGroup) => {
                Self::resolved_handler_env(&node.child, &node.env)
            }
            _ => env,
        }
    }

    /// Build a node-owned lifecycle hook wrapper. An appear hook is retained until
    /// the child's first flush, so the child has installed its reactive subscriptions
    /// before the callback can update them. A disappear hook is retained in the
    /// effect and fired from its `Drop` when the node leaves the retained tree. No
    /// frame-diff slot cursor — structural presence/removal drives both events.
    ///
    /// The hook's environment is the one the content resolves in — the same
    /// resolution `build_wrapper` applies to the other handler effects — so a
    /// `.state(&s)` between the view and `.on_appear` reaches the hook.
    fn build_lifecycle(
        hook: LifeCycleHook,
        content: AnyView,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        let child = Self::build_view(content, env, renderer);
        let env = Self::resolved_handler_env(&child, env).clone();
        let effect = match hook.lifecycle() {
            LifeCycle::Appear => LifeCycleEffect {
                appear: Cell::new(Some(DeferredLifeCycleHook::new(hook, env.clone()))),
                disappear: None,
            },
            LifeCycle::Disappear => LifeCycleEffect {
                appear: Cell::new(None),
                disappear: Some(DeferredLifeCycleHook::new(hook, env.clone())),
            },
            other => {
                contract_violation!(
                    "hydrolysis lifecycle variant {other:?} is not supported; its hook never fires"
                );
                LifeCycleEffect {
                    appear: Cell::new(None),
                    disappear: None,
                }
            }
        };
        Self::Wrapper(Box::new(WrapperNode {
            accessibility_identity: Rc::new(()),
            core: renderer.new_core(),
            effect: WrapperEffect::LifeCycle(effect),
            env,
            released_offsets: Cell::default(),
            child,
        }))
    }

    /// Build a retained reactive collection (non-virtualized): materialize every
    /// current item keyed by id and subscribe to membership changes (a change marks
    /// it dirty and schedules a refresh, which reconciles by id).
    fn build_collection(
        layout: Box<dyn Layout>,
        views: &AnyViews<AnyView>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
        layout_guards: Vec<BoxWatcherGuard>,
        layout_mark: &Rc<RefCell<Weak<NodeCell<E>>>>,
    ) -> Self {
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        let dirty = Rc::new(Cell::new(false));
        let replaced_ids = Rc::new(RefCell::new(std::collections::HashSet::new()));
        let replaced_for_watch = Rc::clone(&replaced_ids);
        // The node retains one immutable row set per applied event: the
        // watcher swaps in the snapshot delivered with each notification, so
        // reconcile never re-reads the live source while applying a change.
        // Registering delivers nothing; the initial membership is this
        // snapshot.
        let applied = Rc::new(RefCell::new(views.snapshot()));
        let applied_for_watch = Rc::clone(&applied);
        // A rebuild in progress already recaptures the whole membership —
        // a fire inside one must not mark the cell the build is writing.
        let rebuild_active = renderer.rebuild_active_flag();
        let guard = views.watch(.., {
            let dirty = Rc::clone(&dirty);
            move |ctx, change| {
                dirty.set(true);
                let event_snapshot = ctx.into_value();
                collect_replaced_ids(
                    &event_snapshot,
                    change,
                    &mut replaced_for_watch.borrow_mut(),
                );
                *applied_for_watch.borrow_mut() = event_snapshot;
                if !rebuild_active.get()
                    && let Some(cell) = cell.upgrade()
                {
                    cell.mark(Dirty::STRUCTURE);
                }
            }
        });
        // A collection carrying accessibility naming metadata is a container like
        // any other: it emits the node naming itself, and its items are built and
        // flushed under the shielded environment so the name is not repeated on
        // every item.
        #[cfg(feature = "accessibility")]
        let item_env = accessibility_container_child_environment(env);
        #[cfg(feature = "accessibility")]
        let accessibility_container_env = item_env.as_ref().map(|_| env.clone());
        #[cfg(feature = "accessibility")]
        let env = item_env.as_ref().unwrap_or(env);
        // The initial membership renders at rest — only items added or removed
        // by a *later* change animate (`reconcile` marks phases). Every entry
        // is built from the one snapshot, so ids and views stay coherent even
        // if a nested build mutates the source mid-materialization.
        *layout_mark.borrow_mut() = Rc::downgrade(&core.cell);
        let snapshot = applied.borrow().clone();
        let entries = snapshot
            .range()
            .filter_map(|index| {
                let id = item_id(&snapshot, index)?;
                let view = item_view_or_empty(&snapshot, index);
                let entry_core = renderer.new_core();
                entry_core.cell.set_parent(&core.cell);
                let node = Self::build_view(view, env, renderer);
                node.core().cell.set_parent(&entry_core.cell);
                Some(CollectionEntry::new(id, node, entry_core))
            })
            .collect();
        Self::Collection(Box::new(CollectionNode {
            memo_gate: Cell::default(),
            memo_slots: RefCell::default(),
            core,
            layout,
            snapshot: applied,
            env: env.clone(),
            accessibility_identity: Rc::new(()),
            #[cfg(feature = "accessibility")]
            accessibility_container_env,
            entries,
            placed: Vec::new(),
            #[cfg(feature = "accessibility")]
            geometry: None,
            ghosts: Vec::new(),
            ghost_placed: Vec::new(),
            reduce_motion: env
                .get::<waterui_structural_transition::ReduceMotion>()
                .is_some_and(|flag| flag.0),
            direction: waterui_layout::layout_direction(env).snapshot(),
            dirty,
            replaced_ids,
            _guard: guard,
            _layout_guards: layout_guards,
        }))
    }

    /// Build a viewport-virtualized lazy stack: store the collection and subscribe
    /// to membership changes (a change schedules a refresh so the visible window
    /// re-resolves). Items are materialized lazily at flush, not here.
    fn build_lazy_stack(
        axis: LazyStackAxisConfig,
        views: &AnyViews<AnyView>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
        layout_guards: Vec<BoxWatcherGuard>,
        layout_mark: &Rc<RefCell<Weak<NodeCell<E>>>>,
    ) -> Self {
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        let dirty = Rc::new(Cell::new(true));
        let dirty_for_watch = Rc::clone(&dirty);
        let replaced_ids = Rc::new(RefCell::new(std::collections::HashSet::new()));
        let replaced_for_watch = Rc::clone(&replaced_ids);
        // The node retains one immutable row set per applied event: the
        // watcher swaps in the snapshot each notification carried, so the
        // window re-resolution, extent index, and materialization all read
        // the same coherent membership rather than the live source.
        let snapshot = Rc::new(RefCell::new(views.snapshot()));
        let snapshot_for_watch = Rc::clone(&snapshot);
        let membership = Rc::new(Cell::new(false));
        let membership_for_watch = Rc::clone(&membership);

        let rebuild_active = renderer.rebuild_active_flag();
        let guard = views.watch(.., {
            let cell = Weak::clone(&cell);
            move |ctx, change| {
                // Membership changed: request a fine-grained refresh; the flush
                // re-resolves the visible window over the event's own snapshot.
                // The reported in-place updates accumulate their ids so the
                // patch invalidates exactly those rows. Registration delivers
                // nothing: the initial window resolves from the snapshot read
                // above.
                dirty_for_watch.set(true);
                membership_for_watch.set(true);
                let event_snapshot = ctx.into_value();
                collect_replaced_ids(
                    &event_snapshot,
                    change,
                    &mut replaced_for_watch.borrow_mut(),
                );
                *snapshot_for_watch.borrow_mut() = event_snapshot;
                if !rebuild_active.get()
                    && let Some(cell) = cell.upgrade()
                {
                    cell.mark(Dirty::STRUCTURE);
                }
            }
        });
        let direction_guard = axis.direction().watch({
            let cell = Weak::clone(&cell);
            move |_| {
                if let Some(cell) = cell.upgrade() {
                    cell.mark_layout();
                }
            }
        });
        // As for [`RenderNode::build_collection`]: naming metadata names the stack,
        // and its rows are materialized under the shielded environment.
        #[cfg(feature = "accessibility")]
        let item_env = accessibility_container_child_environment(env);
        #[cfg(feature = "accessibility")]
        let accessibility_container_env = item_env.as_ref().map(|_| env.clone());
        #[cfg(feature = "accessibility")]
        let env = item_env.as_ref().unwrap_or(env);
        *layout_mark.borrow_mut() = Rc::downgrade(&core.cell);
        let transitions = LazyTransitions::new(env, snapshot.borrow().clone(), membership);
        let first_snapshot = snapshot.borrow().clone();
        let node = Box::new(LazyStackNode {
            memo_gate: Cell::default(),
            memo_slots: RefCell::default(),
            axis,
            snapshot,
            env: env.clone(),
            accessibility_identity: Rc::new(()),
            core,
            safe_area: None,
            #[cfg(feature = "accessibility")]
            geometry: None,
            #[cfg(feature = "accessibility")]
            accessibility_container_env,
            extent_index: RefCell::new(VirtualExtentIndex::default()),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            visible_range: RefCell::new(0..0),
            visible_span: Cell::new(None),
            estimate: Cell::new(0.0),
            estimate_sample: Cell::new(None),
            floor_sample: Cell::new(None),
            dirty,
            replaced_ids,
            _guard: guard,
            _direction_guard: direction_guard,
            _layout_guards: layout_guards,
            transitions,
        });
        node.realize_first_item(&first_snapshot, renderer);
        Self::LazyStack(node)
    }

    /// Build a self-drawn scene node owning its `SceneContent` (no effect slot).
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    fn build_scene_view_node(
        mut content: Box<dyn waterui_graphics::backend::SceneContent>,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        let invalidator: waterui_graphics::backend::SceneInvalidator = Rc::new(move || {
            if let Some(cell) = cell.upgrade() {
                cell.mark(Dirty::PAINT);
            }
        });
        content.set_invalidator(Some(Rc::clone(&invalidator)));
        Self::SceneView(Box::new(SceneViewNode {
            accessibility_identity: Rc::new(()),
            core,
            content: Rc::new(RefCell::new(content)),
            invalidator,
            association: Rc::new(RefCell::new(None)),
            resize: Rc::new(RefCell::new(crate::renderer::ResizeTracker::new())),
            measured: Cell::new(None),
        }))
    }

    /// Build a `GpuView` node owning its [`GpuContentRuntime`] — the
    /// view keeps its UI-side hooks (input, frame pump, ime caret, a11y); the
    /// producer inside is taken exactly once, when the node's install layer
    /// first commits it on the window's engine.
    #[cfg(hydrolysis_gpu)]
    fn build_gpu_content(
        view: GpuView,
        renderer: &SemanticCore<E>,
        #[cfg_attr(not(hydrolysis_hwui_gpu), expect(unused_variables))] env: &Environment,
    ) -> Self {
        Self::GpuContent(Box::new(GpuContentNode {
            accessibility_identity: Rc::new(()),
            core: renderer.new_core(),
            runtime: Rc::new(RefCell::new(GpuContentRuntime::new(
                view,
                #[cfg(hydrolysis_hwui_gpu)]
                crate::runner::android::hosted_gpu::HostedLayer::in_environment(env),
            ))),
            measured: Cell::new(None),
        }))
    }

    /// Build an `ExternalFrameView` node owning its [`ExternalFrameRuntime`] —
    /// the view keeps its UI-side hooks (measure, a11y); the compositor starts
    /// the stream's source the first time a persistent mount installs it.
    #[cfg(hydrolysis_gpu)]
    fn build_external_frame(
        view: ExternalFrameView,
        renderer: &SemanticCore<E>,
        #[cfg_attr(not(hydrolysis_hwui_gpu), expect(unused_variables))] env: &Environment,
    ) -> Self {
        Self::ExternalFrame(Box::new(ExternalFrameNode {
            accessibility_identity: Rc::new(()),
            core: renderer.new_core(),
            runtime: Rc::new(RefCell::new(ExternalFrameRuntime::new(
                view,
                #[cfg(hydrolysis_hwui_gpu)]
                crate::runner::android::hosted_gpu::HostedLayer::in_environment(env),
            ))),
            measured: Cell::new(None),
        }))
    }

    /// Build a `FilteredView` node owning its [`FilteredRuntime`] and building
    /// its wrapped content as a persistent child [`RenderNode`], so reactive
    /// descendants inside the filtered subtree stay live.
    fn build_filtered(
        filtered: FilteredView,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        let FilteredView {
            content,
            effect,
            guards,
        } = filtered;
        let runtime = Rc::new(RefCell::new(FilteredRuntime::new(effect, guards)));
        let child = Self::build_view(content, env, renderer);
        Self::Filtered(Box::new(FilteredNode {
            core: renderer.new_core(),
            runtime,
            child,
            env: env.clone(),
        }))
    }

    /// Build a reactive `Dynamic` host: connect to receive content updates, build
    /// the initial child, and wrap it so later content changes patch in isolation.
    fn build_dynamic_host(
        dynamic: waterui_core::dynamic::Dynamic,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) -> Self {
        let pending: Rc<RefCell<Option<AnyView>>> = Rc::new(RefCell::new(None));
        let signals = renderer.signals.clone();
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        // The connect itself delivers the pending initial content through
        // the receiver: the rebuild that produced this node already renders
        // exactly that view, so it is never a change to mark.
        let render_generation = signals.rebuild_generation();
        let rebuild_active = renderer.rebuild_active_flag();
        dynamic.connect_with_pending_view(Rc::clone(&pending), {
            let pending = Rc::clone(&pending);
            move |update| {
                let is_initial = update
                    .metadata()
                    .try_get::<DynamicInitialContent>()
                    .is_some()
                    || signals.initial_dynamic_content_already_rendered(render_generation);
                *pending.borrow_mut() = Some(update.into_value());
                // A real content change marks the host structure-dirty; the
                // flush rebuilds only this node's child. A rebuild already in
                // flight covers it, so the mark is gated on one not running.
                if !is_initial
                    && !rebuild_active.get()
                    && let Some(cell) = cell.upgrade()
                {
                    cell.mark(Dirty::STRUCTURE);
                }
            }
        });
        let initial = pending.borrow_mut().take();
        let child = match initial {
            Some(content) => Self::build_view(content, env, renderer),
            None => Self::build_view(AnyView::new(()), env, renderer),
        };
        let child = Rc::new(RefCell::new(child));
        let host = DynamicHostNode {
            core,
            safe_area: None,
            pending,
            env: env.clone(),
            child,
            layout_dirty: Cell::new(false),
            transitions: DynamicTransitions::new(env),
        };
        host.claim_initial();
        Self::Dynamic(Box::new(host))
    }
}

/// Whether the node a [`BackgroundLayout`] slot holds paints a fill §7.1
/// extends: a `Color` leaf, or the gradient's fill-widget leaf — read only
/// through wrappers that never read their bounds, so the extended rect
/// reaches nothing that draws or registers against it: `Opacity`,
/// `Env` (accessibility and other scoped metadata), `Retain`, and the
/// `Wrapper` effects with no geometry — `LayoutPriority`, `LifeCycle`,
/// `Focused`, `OnKeyPress` and `MaterialGroup` (a pure scope marker).
/// So `Color.opacity(..)` or an accessibility-scoped env in the slot is
/// still a fill, while a clipped, bordered, scaled or hit-registered
/// color is just another background view and never extends (§7.1's fill
/// is a solid color, a gradient or a material).
///
/// An `.ignore_safe_area` wrapper on the fill is NOT transparent to this:
/// a declaration on the fill replaces the default extension — the
/// wrapper's own release (§7.1 rule 3) is the whole extension, so the node
/// is not a fill and gets no [`RenderNode::Fill`].
fn is_background_fill_leaf<E: crate::text::TextEngine>(node: &RenderNode<E>) -> bool {
    match node {
        RenderNode::Color(_) => true,
        RenderNode::Widget(widget) => widget.fill_leaf,
        RenderNode::Opacity(node) => is_background_fill_leaf(&node.child),
        RenderNode::Retain(node) => is_background_fill_leaf(&node.child),
        RenderNode::Env(node) => is_background_fill_leaf(&node.child),
        RenderNode::Wrapper(node) => {
            matches!(
                node.effect,
                WrapperEffect::LayoutPriority(_)
                    | WrapperEffect::LifeCycle(_)
                    | WrapperEffect::Focused(_)
                    | WrapperEffect::OnKeyPress(_)
                    | WrapperEffect::MaterialGroup
            ) && is_background_fill_leaf(&node.child)
        }
        _ => false,
    }
}
