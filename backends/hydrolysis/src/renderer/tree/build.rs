//! Building the retained tree: [`RenderNode::build`] downcasts a dispatched
//! `View` onto the closed node set, plus the structural builders (wrapper,
//! env scope, collection, lazy stack, scene/GPU/effect, `Dynamic` host).

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::gpu_view::{ExternalFrameRuntime, GpuContentRuntime};
use crate::platform_view::PlatformView;
use core::any::Any;
use waterui_core::views::ViewSnapshot;
use waterui_layout::BackgroundLayout;

/// A retained node's layout-dirty mark and the callback its input
/// subscriptions fire: the callback sets the mark, which an enclosing
/// `RetainedSubview` consumes through [`RenderNode::take_layout_dirty`], and
/// marks the owning node's cell layout-dirty so the pump relayouts it.
fn layout_invalidation(
    cell: &Rc<NodeCell>,
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

impl RenderNode {
    /// Builds a node from a view, capturing live reactive inputs. Native
    /// leaves and layout containers map to concrete nodes; composite views
    /// expand via `body()` once and recurse. Then it wires the subtree's cell
    /// parent links (`attach_subtree`): the tree root attaches to the
    /// window's root cell, every other node to its structural parent's cell.
    pub(crate) fn build(view: AnyView, env: &Environment, renderer: &mut SemanticCore) -> Self {
        let node = Self::build_view(view, env, renderer);
        node.attach_subtree();
        node
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    fn build_view(view: AnyView, env: &Environment, renderer: &mut SemanticCore) -> Self {
        renderer.state.counters.semantic_builds += 1;
        let view = match view.downcast::<Native<Color>>() {
            Ok(color) => {
                return Self::Color(ColorNode {
                    core: renderer.new_core(),
                    color: (*color).into_inner().resolve(env),
                });
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<TextConfig>>() {
            Ok(text) => {
                let config = (*text).into_inner();
                let core = renderer.new_core();
                let (layout_dirty, invalidate) = layout_invalidation(&core.cell);
                let guards = [
                    config.content.watch({
                        let invalidate = Rc::clone(&invalidate);
                        move |_| invalidate()
                    }),
                    config.paragraph_alignment.watch(move |_| invalidate()),
                ];
                return Self::Text(Box::new(TextNode {
                    memo_gate: Cell::default(),
                    memo_slots: RefCell::default(),
                    accessibility_identity: Rc::new(()),
                    core,
                    content: config.content,
                    alignment: config.paragraph_alignment,
                    line_limit: config.line_limit.map(core::num::NonZeroUsize::get),
                    layout_dirty,
                    _guards: guards,
                }));
            }
            Err(view) => view,
        };
        // A plain `FixedContainer` has not run `body()` yet: its layout
        // object is still the one the modifier built — the only moment
        // `BackgroundLayout` is identifiable, before `body` wraps it in
        // `DirectionalLayout`. `FixedContainer::body` is what produces the
        // `Native<FixedContainer>` the next arm rebuilds.
        let view = match view.downcast::<FixedContainer>() {
            Ok(container) => {
                let background_slot = (container.as_parts().0 as &dyn Any)
                    .is::<BackgroundLayout>()
                    .then_some(0);
                let container = *AnyView::new(container.body(env))
                    .downcast::<Native<FixedContainer>>()
                    .expect("FixedContainer::body produces Native<FixedContainer>");
                return Self::build_fixed_container(container, env, renderer, background_slot);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<FixedContainer>>() {
            Ok(container) => {
                return Self::build_fixed_container(*container, env, renderer, None);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<LazyContainer>>() {
            Ok(container) => {
                let direction = container.as_inner().direction();
                let (layout, children) = (*container).into_inner().into_inner();
                // The collection's own layout inputs (stack spacing, absolute
                // pins, …) are signals too: subscribe them through
                // `watch_invalidation` like a FixedContainer's, so a change
                // marks the owning node layout-dirty. Which node that is —
                // collection or lazy stack — is decided after the watch
                // installs, so the guard marks through a shared target the
                // built node fills in.
                let layout_mark = Rc::new(RefCell::new(Weak::<NodeCell>::new()));
                let layout_guards = {
                    let layout_mark = Rc::clone(&layout_mark);
                    layout.watch_invalidation(Rc::new(move || {
                        if let Some(cell) = layout_mark.borrow().upgrade() {
                            cell.mark_layout();
                        }
                    }))
                };
                // A viewport-virtualizable stack layout (and not opting into a
                // membership transition, which must retain every item) becomes a
                // virtualized LazyStack: only visible rows are built/measured.
                let wants_transition = env
                    .get::<waterui_layout::collection_transition::CollectionTransition>()
                    .is_some();
                if let Some(axis) =
                    lazy_stack_axis_config(layout.as_ref(), direction).filter(|_| !wants_transition)
                {
                    return Self::build_lazy_stack(
                        axis,
                        &children,
                        env,
                        renderer,
                        layout_guards,
                        &layout_mark,
                    );
                }
                // A non-virtualizable layout (AbsoluteLayout/ZStack overlay) or a
                // transition collection: a retained reactive collection that
                // reconciles membership by id (recursing into each item, so inner
                // SceneView/Dynamic reach their dedicated nodes).
                return Self::build_collection(
                    layout,
                    &children,
                    env,
                    renderer,
                    layout_guards,
                    &layout_mark,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Opacity>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::Opacity(Box::new(OpacityNode {
                    core: renderer.new_core(),
                    value,
                    child: Self::build_view(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Scale>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::Scale(Box::new(ScaleNode {
                    core: renderer.new_core(),
                    value,
                    child: Self::build_view(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Rotation>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::Rotation(Box::new(RotationNode {
                    core: renderer.new_core(),
                    value,
                    child: Self::build_view(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Offset>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::Offset(Box::new(OffsetNode {
                    core: renderer.new_core(),
                    value,
                    child: Self::build_view(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Environment>>() {
            Ok(meta) => {
                let (content, scoped_env) =
                    flatten_environment_metadata_owned(AnyView::new(*meta), env);
                // The snapshot replaces the environment wholesale, so the
                // accessibility naming scope this build installed above it has
                // to be carried across or the name is lost — see
                // `restore_a11y_naming_scope`.
                let scoped_env = restore_a11y_naming_scope(env, scoped_env);
                // Carry the scoped environment in the node (not flattened away), so
                // it is also the env used at flush/measure/layout — text shaping and
                // a11y read env every frame.
                let child = Self::build_view(content, &scoped_env, renderer);
                return Self::Env(Box::new(EnvNode {
                    core: renderer.new_core(),
                    env: scoped_env,
                    child,
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Retain>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::Retain(Box::new(RetainNode {
                    core: renderer.new_core(),
                    _retain: value,
                    child: Self::build_view(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        // Accessibility metadata are environment-scoping wrappers (the dispatch
        // handlers just `env.insert(value)` then render the content), so the tree
        // models them as an `Env` node carrying the extended environment — the Text
        // node and a11y emission read these from env every flush, and the wrapped
        // (possibly reactive) content stays live instead of freezing in `Captured`.
        // The two *naming* wrappers additionally carry a scope identity, so a
        // container that no control spoke for can emit the node naming itself.
        // Accessibility state is stored in the scoped environment as a *live*
        // `AccessibilityStateSignal` (a static state becomes a constant signal):
        // descendants capture environment clones at build time, so a resolved
        // snapshot would freeze the state forever — a `when(selected, …)` chip
        // would keep emitting `selected == false` after every toggle. Emission
        // resolves the signal at flush time (`apply_state`), and node
        // registration subscribes it to the refresh pump. Only the *static*
        // state bakes a subtree-suppressing `AccessibilityHidden` (a constant
        // can never un-hide); a reactive signal must not — a build-time hidden
        // snapshot would freeze the subtree hidden after the signal turns
        // visible. A signal-hidden node is emitted with the accesskit hidden
        // flag instead, so it follows the signal every flush.
        // Which wrapper maps to which scoping is defined once in
        // `a11y_scoped_env_for_view` — the List-row hoist reads the same table.
        let view = match a11y_scoped_env_for_view(view, env) {
            Ok((content, scoped)) => {
                let child = Self::build_view(content, &scoped, renderer);
                return Self::Env(Box::new(EnvNode {
                    core: renderer.new_core(),
                    env: scoped,
                    child,
                }));
            }
            Err(view) => view,
        };
        // `.selected(...)` is strict `Metadata<Selected>` — it scopes into the
        // environment like the a11y metadata above: the outermost interactive
        // control binding under it claims it (`claim_selected`), and the
        // control's `SELECTED` flag and a11y selected state read it.
        let view = match view.downcast::<Metadata<waterui_core::interaction::Selected>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                let scoped = a11y_scoped_env(env, &value);
                let child = Self::build_view(content, &scoped, renderer);
                return Self::Env(Box::new(EnvNode {
                    core: renderer.new_core(),
                    env: scoped,
                    child,
                }));
            }
            Err(view) => view,
        };
        // Passthrough metadata: the dispatch handlers discard the value and just
        // render the content (no-ops in Hydrolysis), so the tree unwraps them to
        // the content directly — fully transparent, keeping reactive descendants live.
        let view = match view.downcast::<Metadata<Secure>>() {
            Ok(meta) => return Self::build_view(meta.content, env, renderer),
            Err(view) => view,
        };
        // Dynamic-range metadata used to scope a preference read by the retired
        // GPU-surface path; Cherenkov's engine owns headroom per surface, so
        // both pass through to the content like `Secure` above.
        let view = match view.downcast::<Metadata<StandardDynamicRange>>() {
            Ok(meta) => return Self::build_view(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<HighDynamicRange>>() {
            Ok(meta) => return Self::build_view(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<IgnoreSafeArea>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::IgnoreSafeArea(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<ContextMenu>>() {
            Ok(meta) => return Self::build_view(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Background>>() {
            Ok(meta) => return Self::build_view(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<NavigationTransitionSource>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::NavigationTransitionSource(value.id()),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<NavigationTransitionDestination>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::NavigationTransitionDestination(value.id()),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<MaterialBackground>>() {
            Ok(meta) => {
                let IgnorableMetadata {
                    content,
                    value: MaterialBackground(material),
                } = *meta;
                let runtime = crate::renderer::material::MaterialRuntime::new(
                    crate::renderer::material::WithinWindowLevel::of(material),
                    &waterui::theme::current_color_scheme(env),
                );
                return Self::build_wrapper(
                    WrapperEffect::Material(Rc::new(runtime)),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        // Transparent metadata wrappers: each applies its visual/interaction
        // effect every flush and recurses into the child node, so reactive
        // descendants reach their dedicated nodes (instead of freezing inside a
        // one-shot `Captured`). The effect is shared with the dispatch path via
        // the `apply_*` helpers in `metadata.rs`.
        let view = match view.downcast::<Metadata<ClipShape>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(WrapperEffect::Clip(value), content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Border>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(WrapperEffect::Border(value), content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Shadow>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(WrapperEffect::Shadow(value), content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<AnchoredOverlay>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                let AnchoredOverlay {
                    content: overlay_content,
                    is_presented,
                    placed_edge,
                    placement,
                    dismissal,
                } = value;
                return Self::build_wrapper(
                    WrapperEffect::AnchoredOverlay(AnchoredOverlayEffect {
                        content: Rc::new(RefCell::new(Some(RetainedSubview::new(overlay_content)))),
                        is_presented,
                        placed_edge,
                        placement,
                        dismissal,
                        marker: Rc::new(()),
                    }),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<PopupMenuSurface>>() {
            Ok(meta) => {
                return Self::build_wrapper(
                    WrapperEffect::PopupMenuSurface,
                    meta.content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<LayoutPriority>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::LayoutPriority(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Cursor>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(WrapperEffect::Cursor(value), content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Draggable>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::Draggable(Rc::new(value)),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<DropDestination>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::DropDestination(DropDestinationHandles::from_destination(value)),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<ResolvedContextMenu>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                let ResolvedContextMenu {
                    items,
                    preview,
                    accessory,
                    dismiss_requests,
                } = value;
                return Self::build_wrapper(
                    WrapperEffect::ContextMenu(ContextMenuEffect {
                        items,
                        dismiss_requests,
                        preview: Rc::new(RefCell::new(preview.map(RetainedSubview::new))),
                        accessory: Rc::new(RefCell::new(accessory.map(RetainedSubview::new))),
                    }),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Hittable>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(WrapperEffect::Hittable(value), content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<OnEvent>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::OnEvent(Rc::new(RefCell::new(value))),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<OnKeyPress>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(
                    WrapperEffect::OnKeyPress(Rc::new(RefCell::new(value))),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<GestureObserver>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                let GestureObserver {
                    gesture, action, ..
                } = value;
                // A node has no `content` at flush, so resolve the two
                // content-derived pieces now (the default a11y label string and
                // the gesture group identity) and store them in the effect.
                let effect = GestureObserverEffect {
                    gesture,
                    action: Rc::new(RefCell::new(action)),
                    #[cfg(feature = "accessibility")]
                    default_a11y_label: renderer.accessibility_label_from_view(&content, env),
                    gesture_group_identity: gesture_group_identity(&content),
                    gesture_target: Cell::new(None),
                };
                return Self::build_wrapper(
                    WrapperEffect::GestureObserver(effect),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Focused>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_wrapper(WrapperEffect::Focused(value), content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<LifeCycleHook>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return Self::build_lifecycle(value, content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ScrollView>>() {
            Ok(scroll) => {
                let ScrollViewParts {
                    axis,
                    content,
                    controller,
                    offset,
                    ..
                } = (*scroll).into_inner().into_inner();
                let content = normalize_layout_view(content, env);
                // A scroll surface owns §7.1 for its subtree: the layout
                // pass hands the surface its facts and the child lays out
                // with no safe-area context — the surface insets and
                // scrolls its own content instead.
                return Self::Scroll(Box::new(ScrollNode {
                    memo_gate: Cell::default(),
                    memo_slots: RefCell::default(),
                    accessibility_identity: Rc::new(()),
                    core: renderer.new_core(),
                    axis,
                    child: Self::build_view(content, env, renderer),
                    controller,
                    offset,
                    applied_scroll_generation: Cell::new(0),
                    handle: RefCell::new(None),
                    content_size: Size::zero(),
                    viewport: Size::zero(),
                    non_scrolling_minimum: Cell::new(None),
                    env: env.clone(),
                    surface: std::rc::Rc::default(),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SceneView>>() {
            Ok(scene_view) => {
                return Self::build_scene_view_node(*scene_view, renderer);
            }
            Err(view) => view,
        };
        // GPU/effect leaves and wrappers: each owns its effect runtime directly
        // (textures, setup state, redraw handle), so a reactive swap renders the
        // new content and a per-frame re-flush re-binds the *same* runtime. Holding
        // the runtime in a frame-ordered slot instead lets the ordering drift out of
        // step with the tree and hand a leaf another leaf's runtime.
        let view = match view.downcast::<Native<GpuContentView>>() {
            Ok(view) => {
                return Self::build_gpu_content((*view).into_inner(), renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ExternalFrameView>>() {
            Ok(view) => {
                return Self::build_external_frame((*view).into_inner(), renderer);
            }
            Err(view) => view,
        };
        // A platform-view embedding leaf: it records its frame onto the host's
        // `PlatformViewSink` each flush; mounting the real native child is the
        // host's work. No sink means this runner cannot embed — the leaf
        // panics at build naming the missing piece.
        let view = match view.downcast::<Native<PlatformView>>() {
            Ok(platform_view) => {
                return Self::build_platform_view(renderer, &(*platform_view).into_inner(), env);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<FilteredView>>() {
            Ok(filtered) => {
                return Self::build_filtered((*filtered).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Dynamic>>() {
            Ok(dynamic) => {
                return Self::build_dynamic_host((*dynamic).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        // A native widget leaf: build a `Widget` node that re-renders it every flush
        // from a retained, signal-holding config (action retained behind `Rc`), so
        // its handler re-reads live signals — reactive labels/values stay live
        // instead of freezing in a one-shot `Captured` bake.
        let view = match view.downcast::<Native<ButtonConfig>>() {
            Ok(button) => {
                return Self::build_button(renderer, (*button).into_inner(), env);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedMenu>>() {
            Ok(menu) => return Self::build_menu(renderer, (*menu).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ToggleConfig>>() {
            Ok(toggle) => return Self::build_toggle((*toggle).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SliderConfig>>() {
            Ok(slider) => return Self::build_slider((*slider).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<StepperConfig>>() {
            Ok(stepper) => {
                return Self::build_stepper((*stepper).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ProgressConfig>>() {
            Ok(progress) => {
                return Self::build_progress((*progress).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<DatePickerConfig>>() {
            Ok(date_picker) => {
                return Self::build_date_picker((*date_picker).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ColorPickerConfig>>() {
            Ok(color_picker) => {
                return Self::build_color_picker((*color_picker).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<PickerConfig>>() {
            Ok(picker) => {
                return Self::build_picker((*picker).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedTextFieldConfig>>() {
            Ok(text_field) => {
                return Self::build_text_field((*text_field).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SecureFieldConfig>>() {
            Ok(secure_field) => {
                return Self::build_secure_field((*secure_field).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<BadgeConfig>>() {
            Ok(badge) => return Self::build_badge((*badge).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ListConfig>>() {
            Ok(list) => return Self::build_list((*list).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<TableConfig>>() {
            Ok(table) => return Self::build_table(renderer, (*table).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SystemIcon>>() {
            Ok(icon) => unsupported_system_icon(icon.as_inner()),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<waterui_graphics::Gradient>>() {
            Ok(gradient) => return Self::build_gradient(renderer, (*gradient).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedShape>>() {
            Ok(shape) => return Self::build_shape(renderer, (*shape).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedMorphShape>>() {
            Ok(shape) => return Self::build_morph_shape(renderer, (*shape).into_inner(), env),
            Err(view) => view,
        };
        let Err(view) = view.downcast::<Native<MapConfig>>() else {
            unsupported_map()
        };
        // This backend bridges the platform's own web engine, and it only wins
        // by default: an application that linked a browser engine of its own
        // installed a `Hook<WebView>`, and taking the component by type here
        // would draw it with an engine the application did not pick — and hand
        // the macOS bridge a page handle from another engine.
        let view = match view.downcast::<WebView>() {
            Ok(webview) if env.get::<Hook<WebView>>().is_none() => {
                return Self::build_webview(*webview, env, renderer);
            }
            Ok(webview) => AnyView::new(*webview),
            Err(view) => view,
        };
        // Navigation containers (navigation view / split / stack / tabs): each is a
        // persistent `Widget` node re-rendered every flush from a retained config —
        // the navigation controller, transition slot, and tab-bar state stay live
        // and reactive route/tab selection keeps driving updates, instead of
        // freezing in a one-shot `Captured` bake.
        let view = match view.downcast::<Native<NavigationView>>() {
            Ok(navigation) => {
                return Self::build_navigation_view((*navigation).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<NavigationSplitLayout>>() {
            Ok(split) => {
                return Self::build_navigation_split((*split).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<NavigationStack<(), ()>>>() {
            Ok(stack) => {
                return Self::build_navigation_stack(renderer, (*stack).into_inner(), env);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<TabsLayout>>() {
            Ok(tabs) => return Self::build_tabs((*tabs).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Spacer>>() {
            Ok(spacer) => return Self::build_spacer(renderer, (*spacer).into_inner(), env),
            Err(view) => view,
        };
        // `Native<()>` carries no data — drop the wrapper and build the empty leaf.
        let Err(view) = view.downcast::<Native<()>>() else {
            return Self::build_empty(renderer, env);
        };
        // `Divider` and `Str` are registered renderers (not `Native<…>` leaves), so
        // they are downcast as their value type directly and built into persistent
        // `Widget` nodes that re-render from a retained cell each flush.
        let view = match view.downcast::<Divider>() {
            Ok(divider) => return Self::build_divider(renderer, *divider, env),
            Err(view) => view,
        };
        let view = match view.downcast::<Str>() {
            Ok(text) => return Self::build_str(renderer, *text, env),
            Err(view) => view,
        };
        // Every native leaf and metadata wrapper now has a dedicated `RenderNode`
        // build arm above. Anything reaching here is a composite, expanded via
        // `body()` once. A native leaf with no build arm panics here in `body()` —
        // the acceptable fast-fail for a missing arm.
        Self::build_view(AnyView::new(view.body(env)), env, renderer)
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
        effect: WrapperEffect,
        content: AnyView,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        let child = Self::build_view(content, env, renderer);
        let env = if effect.captures_environment() {
            Self::resolved_handler_env(&child, env).clone()
        } else {
            env.clone()
        };
        Self::Wrapper(Box::new(WrapperNode {
            accessibility_identity: Rc::new(()),
            core: renderer.new_core(),
            effect,
            env,
            released_offsets: Cell::default(),
            child,
        }))
    }

    /// The environment a metadata-carried callback captures: the one its
    /// modified view resolves in. Walks down the child's leading env-only
    /// wrappers — `Env` nodes (`With`/`Metadata<Environment>` installs and
    /// env-scoped metadata) and handler wrappers of the same modifier chain —
    /// until the first node that is neither. Resolving on the *built* node
    /// means each `With` was already expanded exactly once into the `Env` node
    /// that carries its scoped env.
    fn resolved_handler_env<'a>(node: &'a Self, env: &'a Environment) -> &'a Environment {
        match node {
            Self::Env(node) => Self::resolved_handler_env(&node.child, &node.env),
            Self::Wrapper(node) if node.effect.captures_environment() => {
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
        renderer: &mut SemanticCore,
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
            _ => panic!("hydrolysis lifecycle variant is not supported"),
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
        renderer: &mut SemanticCore,
        layout_guards: Vec<BoxWatcherGuard>,
        layout_mark: &Rc<RefCell<Weak<NodeCell>>>,
    ) -> Self {
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        let dirty = Rc::new(Cell::new(false));
        let dirty_key = Rc::new(());
        let key = Rc::as_ptr(&dirty_key) as usize;
        let signals = renderer.signals.clone();
        let replaced_ids = Rc::new(RefCell::new(std::collections::HashSet::new()));
        let replaced_for_watch = Rc::clone(&replaced_ids);
        // The node retains one immutable row set per applied event: the
        // watcher swaps in the snapshot each notification carried — captured
        // from the exact data that emitted it — so reconcile never re-reads
        // the live source while applying an older change.
        let applied = Rc::new(RefCell::new(views.snapshot()));
        let applied_for_watch = Rc::clone(&applied);
        // A rebuild in progress already recaptures the whole membership —
        // the same gate `mark_collection_dirty` applies to its dirty flag.
        let rebuild_active = renderer.rebuild_active_flag();
        let guard = views.watch(.., {
            let dirty = Rc::clone(&dirty);
            move |ctx, change| {
                dirty.set(true);
                let event_snapshot = ctx.into_value();
                collect_replaced_ids(
                    &event_snapshot,
                    &change,
                    &mut replaced_for_watch.borrow_mut(),
                );
                *applied_for_watch.borrow_mut() = event_snapshot;
                signals.mark_collection_dirty(key, 0);
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
            .map(|index| {
                let id = snapshot
                    .get_id(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"));
                let view = snapshot
                    .get_view(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                let entry_core = renderer.new_core();
                entry_core.cell.set_parent(&core.cell);
                let node = Self::build_view(normalize_layout_view(view, env), env, renderer);
                node.core().cell.set_parent(&entry_core.cell);
                CollectionEntry::stable(id, node, entry_core)
            })
            .collect();
        let transition = collection_transition_runtime(env, layout.as_ref(), &mut renderer.state);
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
            resolved: Rect::from_size(Size::zero()),
            transition,
            dirty,
            replaced_ids,
            _dirty_key: dirty_key,
            _guard: guard,
            _layout_guards: layout_guards,
        }))
    }

    /// Build a viewport-virtualized lazy stack: store the collection and subscribe
    /// to membership changes (a change schedules a refresh so the visible window
    /// re-resolves). Items are materialized lazily at flush, not here.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    fn build_lazy_stack(
        axis: LazyStackAxisConfig,
        views: &AnyViews<AnyView>,
        env: &Environment,
        renderer: &mut SemanticCore,
        layout_guards: Vec<BoxWatcherGuard>,
        layout_mark: &Rc<RefCell<Weak<NodeCell>>>,
    ) -> Self {
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        let dirty_key = Rc::new(());
        let dirty = Rc::new(Cell::new(true));
        let key = Rc::as_ptr(&dirty_key) as usize;
        let signals = renderer.signals.clone();
        let dirty_for_watch = Rc::clone(&dirty);
        let replaced_ids = Rc::new(RefCell::new(std::collections::HashSet::new()));
        let replaced_for_watch = Rc::clone(&replaced_ids);
        // The node retains one immutable row set per applied event: the
        // watcher swaps in the snapshot each notification carried, so the
        // window re-resolution, extent index, and materialization all read
        // the same coherent membership rather than the live source.
        let snapshot = Rc::new(RefCell::new(views.snapshot()));
        let snapshot_for_watch = Rc::clone(&snapshot);

        let rebuild_active = renderer.rebuild_active_flag();
        let guard = views.watch(.., {
            let cell = Weak::clone(&cell);
            move |ctx, change| {
                // Membership changed: request a fine-grained refresh; the flush
                // re-resolves the visible window over the event's own snapshot.
                // The reported replaced positions accumulate their ids so the
                // patch invalidates exactly those rows. Registration itself
                // fires the initial snapshot — gated like the collection's.
                dirty_for_watch.set(true);
                let event_snapshot = ctx.into_value();
                collect_replaced_ids(
                    &event_snapshot,
                    &change,
                    &mut replaced_for_watch.borrow_mut(),
                );
                *snapshot_for_watch.borrow_mut() = event_snapshot;
                signals.mark_collection_dirty(key, 0);
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
        Self::LazyStack(Box::new(LazyStackNode {
            memo_gate: Cell::default(),
            memo_slots: RefCell::default(),
            axis,
            snapshot,
            env: env.clone(),
            accessibility_identity: Rc::new(()),
            core,
            safe_area: None,
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
            _dirty_key: dirty_key,
            _guard: guard,
            _direction_guard: direction_guard,
            _layout_guards: layout_guards,
        }))
    }

    /// Build a self-drawn scene node owning its `SceneContent` (no effect slot).
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    fn build_scene_view_node(scene_view: Native<SceneView>, renderer: &mut SemanticCore) -> Self {
        let mut content = scene_view.into_inner().into_content();
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        let invalidator: waterui_graphics::SceneInvalidator = Rc::new(move || {
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
        }))
    }

    /// Build a `GpuContentView` node owning its [`GpuContentRuntime`] — the
    /// view keeps its UI-side hooks (input, frame pump, ime caret, a11y); the
    /// producer inside is taken exactly once, when the node's install layer
    /// first commits it on the window's engine.
    fn build_gpu_content(view: GpuContentView, renderer: &SemanticCore) -> Self {
        Self::GpuContent(Box::new(GpuContentNode {
            accessibility_identity: Rc::new(()),
            core: renderer.new_core(),
            runtime: Rc::new(RefCell::new(GpuContentRuntime::new(view))),
        }))
    }

    /// Build an `ExternalFrameView` node owning its [`ExternalFrameRuntime`] —
    /// the view keeps its UI-side hooks (measure, a11y); the compositor starts
    /// the stream's source the first time a persistent mount installs it.
    fn build_external_frame(view: ExternalFrameView, renderer: &SemanticCore) -> Self {
        Self::ExternalFrame(Box::new(ExternalFrameNode {
            accessibility_identity: Rc::new(()),
            core: renderer.new_core(),
            runtime: Rc::new(RefCell::new(ExternalFrameRuntime::new(view))),
        }))
    }

    /// Build a `FilteredView` node owning its [`FilteredRuntime`] and building
    /// its wrapped content as a persistent child [`RenderNode`], so reactive
    /// descendants inside the filtered subtree stay live.
    fn build_filtered(
        filtered: FilteredView,
        env: &Environment,
        renderer: &mut SemanticCore,
    ) -> Self {
        let FilteredView {
            content,
            effect,
            guards,
        } = filtered;
        let runtime = Rc::new(RefCell::new(FilteredRuntime::new(effect, guards)));
        let child = Self::build_view(normalize_layout_view(content, env), env, renderer);
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
        renderer: &mut SemanticCore,
    ) -> Self {
        let identity = dynamic.identity();
        let pending: Rc<RefCell<Option<AnyView>>> = Rc::new(RefCell::new(None));
        let source = dynamic.clone();
        let signals = renderer.signals.clone();
        let core = renderer.new_core();
        let cell = Rc::downgrade(&core.cell);
        // The connect itself delivers the pending initial content through
        // the receiver: the rebuild that produced this node already renders
        // exactly that view, so it is never a change to mark — the same gate
        // `mark_dynamic_dirty` applies to the dirty set.
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
                // flight covers it — the generation gate `mark_dynamic_dirty`
                // applies to its dirty flag.
                if !is_initial {
                    signals.mark_dynamic_dirty(identity, 0);
                    if !rebuild_active.get()
                        && let Some(cell) = cell.upgrade()
                    {
                        cell.mark(Dirty::STRUCTURE);
                    }
                }
            }
        });
        let initial = pending.borrow_mut().take();
        let child = match initial {
            Some(content) => Self::build_view(content, env, renderer),
            None => Self::build_view(AnyView::new(()), env, renderer),
        };
        let child = Rc::new(RefCell::new(child));
        // The dispatch measure (`measure_dynamic`) reaches this child through
        // the identity registry once the `Dynamic` has connected.
        renderer
            .state
            .measurement
            .register_dynamic_node(identity, &child);
        Self::Dynamic(Box::new(DynamicHostNode {
            core,
            safe_area: None,
            source,
            pending,
            env: env.clone(),
            child,
            layout_dirty: Cell::new(false),
        }))
    }
}
impl RenderNode {
    /// Builds the [`RenderNode::Container`] for a `Native<FixedContainer>`
    /// whose background slot was already identified — `Some(slot)` wraps the
    /// slot's fill in [`RenderNode::Fill`], the type §7.1's paint extension
    /// records on. The identification happens once here, while the layout
    /// type is still concrete (normalization rebuilds the `FixedContainer`
    /// from its parts instead of running `body` early); a declaration
    /// wrapping the fill (`.ignore_safe_area` on the fill itself) stops the
    /// predicate, so it replaces the default rather than stacking with it.
    fn build_fixed_container(
        container: Native<FixedContainer>,
        env: &Environment,
        renderer: &mut SemanticCore,
        background_slot: Option<usize>,
    ) -> Self {
        let (layout, children) = container.into_inner().into_inner();
        let core = renderer.new_core();
        let (layout_dirty, invalidate) = layout_invalidation(&core.cell);
        let guards = layout.watch_invalidation(invalidate);
        #[cfg(feature = "accessibility")]
        let accessibility_child_env = accessibility_container_child_environment(env);
        #[cfg(feature = "accessibility")]
        let child_env = accessibility_child_env.as_ref().unwrap_or(env);
        #[cfg(not(feature = "accessibility"))]
        let child_env = env;
        let children: Vec<Self> = children
            .into_iter()
            .enumerate()
            .map(|(index, child)| {
                let node =
                    Self::build_view(normalize_layout_view(child, child_env), child_env, renderer);
                if Some(index) == background_slot && is_background_fill_leaf(&node) {
                    Self::Fill(Box::new(FillNode::new(node, renderer.new_core())))
                } else {
                    node
                }
            })
            .collect();
        Self::Container(Box::new(ContainerNode {
            memo_gate: Cell::default(),
            memo_slots: RefCell::default(),
            accessibility_identity: Rc::new(()),
            core,
            layout,
            children,
            #[cfg(feature = "accessibility")]
            accessibility_child_env,
            placed: Vec::new(),
            #[cfg(feature = "accessibility")]
            resolved: Rect::from_size(Size::zero()),
            layout_dirty,
            _guards: guards,
        }))
    }
}

/// Whether the node a [`BackgroundLayout`] slot holds paints a fill §7.1
/// extends: a `Color` leaf, or the gradient's fill-widget leaf — read only
/// through wrappers that never read their bounds, so the extended rect
/// reaches nothing that draws or registers against it: `Opacity`,
/// `Env` (accessibility and other scoped metadata), `Retain`, and the
/// `Wrapper` effects with no geometry — `LayoutPriority`, `LifeCycle`,
/// `Focused` and `OnKeyPress`. So `Color.opacity(..)` or an
/// accessibility-scoped env in the slot is still a fill, while a clipped,
/// bordered, scaled or hit-registered color is just another background
/// view and never extends (§7.1's fill is a solid color, a gradient or a
/// material).
///
/// An `.ignore_safe_area` wrapper on the fill is NOT transparent to this:
/// a declaration on the fill replaces the default extension — the
/// wrapper's own release (§7.1 rule 3) is the whole extension, so the node
/// is not a fill and gets no [`RenderNode::Fill`].
fn is_background_fill_leaf(node: &RenderNode) -> bool {
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
            ) && is_background_fill_leaf(&node.child)
        }
        _ => false,
    }
}
