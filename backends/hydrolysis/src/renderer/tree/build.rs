//! Building the retained tree: [`RenderNode::build`] downcasts a dispatched
//! `View` onto the closed node set, plus the structural builders (wrapper,
//! env scope, collection, lazy stack, scene/GPU/effect, `Dynamic` host).

use super::*;

impl RenderNode {
    /// Build a node from a view, capturing live reactive inputs. Native leaves
    /// and layout containers map to concrete nodes; composite views expand via
    /// `body()` once and recurse.
    pub(crate) fn build(
        view: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let view = match view.downcast::<Native<ResolvedColor>>() {
            Ok(color) => {
                return RenderNode::Color(ColorNode {
                    color: resolved_color_to_peniko((*color).into_inner()),
                });
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<TextConfig>>() {
            Ok(text) => {
                let config = (*text).into_inner();
                return RenderNode::Text(Box::new(TextNode {
                    content: config.content,
                    alignment: config.paragraph_alignment,
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<FixedContainer>>() {
            Ok(container) => {
                let (layout, children) = (*container).into_inner().into_inner();
                let children = children
                    .into_iter()
                    .map(|child| {
                        RenderNode::build(normalize_layout_view(child, env), env, renderer)
                    })
                    .collect();
                return RenderNode::Container(Box::new(ContainerNode {
                    layout,
                    children,
                    placed: Vec::new(),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<LazyContainer>>() {
            Ok(container) => {
                let (layout, children) = (*container).into_inner().into_inner();
                // A viewport-virtualizable stack layout (and not opting into a
                // membership transition, which must retain every item) becomes a
                // virtualized LazyStack: only visible rows are built/measured.
                let wants_transition = env
                    .get::<waterui_layout::collection_transition::CollectionTransition>()
                    .is_some();
                if let Some(axis) =
                    lazy_stack_axis_config(layout.as_ref()).filter(|_| !wants_transition)
                {
                    return RenderNode::build_lazy_stack(axis, children, env, renderer);
                }
                // A non-virtualizable layout (AbsoluteLayout/ZStack overlay) or a
                // transition collection: a retained reactive collection that
                // reconciles membership by id (recursing into each item, so inner
                // SceneView/Dynamic reach their dedicated nodes).
                return RenderNode::build_collection(layout, children, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Opacity>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Opacity(Box::new(OpacityNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Scale>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Scale(Box::new(ScaleNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Rotation>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Rotation(Box::new(RotationNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Offset>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Offset(Box::new(OffsetNode {
                    value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Environment>>() {
            Ok(meta) => {
                let (content, scoped_env) =
                    flatten_environment_metadata_owned(AnyView::new(*meta), env);
                // Carry the scoped environment in the node (not flattened away), so
                // it is also the env used at flush/measure/layout — text shaping and
                // a11y read env every frame.
                let child = RenderNode::build(content, &scoped_env, renderer);
                return RenderNode::Env(Box::new(EnvNode {
                    env: scoped_env,
                    child,
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Retain>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::Retain(Box::new(RetainNode {
                    retain: value,
                    child: RenderNode::build(content, env, renderer),
                }));
            }
            Err(view) => view,
        };
        // Accessibility metadata are environment-scoping wrappers (the dispatch
        // handlers just `env.insert(value)` then render the content), so the tree
        // models them as an `Env` node carrying the extended environment — the Text
        // node and a11y emission read these from env every flush, and the wrapped
        // (possibly reactive) content stays live instead of freezing in `Captured`.
        let view = match view.downcast::<IgnorableMetadata<AccessibilityLabel>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityRole>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityHidden>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityChildren>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
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
        let view = match view.downcast::<IgnorableMetadata<AccessibilityState>>() {
            Ok(meta) => {
                let IgnorableMetadata { content, value } = *meta;
                let scoped = a11y_scoped_env_for_state(env, &value);
                let child = RenderNode::build(content, &scoped, renderer);
                return RenderNode::Env(Box::new(EnvNode { env: scoped, child }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<AccessibilityStateSignal>>() {
            Ok(meta) => return RenderNode::build_env_scoped(*meta, env, renderer),
            Err(view) => view,
        };
        // Passthrough metadata: the dispatch handlers discard the value and just
        // render the content (no-ops in Hydrolysis), so the tree unwraps them to
        // the content directly — fully transparent, keeping reactive descendants live.
        let view = match view.downcast::<Metadata<Secure>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<StandardDynamicRange>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<HighDynamicRange>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<IgnoreSafeArea>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<ContextMenu>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Background>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<IgnorableMetadata<MaterialBackground>>() {
            Ok(meta) => return RenderNode::build(meta.content, env, renderer),
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
                return RenderNode::build_wrapper(
                    WrapperEffect::Clip(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Border>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Border(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Shadow>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Shadow(value),
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
                return RenderNode::build_wrapper(
                    WrapperEffect::Cursor(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<Draggable>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::Draggable(value),
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
                return RenderNode::build_wrapper(
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
                return RenderNode::build_wrapper(
                    WrapperEffect::ContextMenu(value),
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
                return RenderNode::build_wrapper(
                    WrapperEffect::Hittable(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<OnEvent>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_wrapper(
                    WrapperEffect::OnEvent(Rc::new(RefCell::new(value))),
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
                };
                return RenderNode::build_wrapper(
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
                return RenderNode::build_wrapper(
                    WrapperEffect::Focused(value),
                    content,
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<LifeCycleHook>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_lifecycle(value, content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ScrollView>>() {
            Ok(scroll) => {
                let (axis, content) = (*scroll).into_inner().into_inner();
                let content = normalize_layout_view(content, env);
                return RenderNode::Scroll(Box::new(ScrollNode {
                    axis,
                    child: RenderNode::build(content, env, renderer),
                    handle: None,
                    content_size: Size::zero(),
                    viewport: Size::zero(),
                    env: env.clone(),
                }));
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SceneView>>() {
            Ok(scene_view) => {
                return RenderNode::build_scene_view_node(*scene_view, renderer);
            }
            Err(view) => view,
        };
        // GPU/effect leaves and wrappers: each owns its effect runtime directly
        // (textures, setup state, redraw handle), so a reactive swap renders the
        // new content and a per-frame re-flush re-binds the *same* runtime — no
        // cursor-ordered effect slot to desync (the chart Bug 1 fix, generalized).
        let view = match view.downcast::<Native<GpuSurface>>() {
            Ok(surface) => {
                return RenderNode::build_gpu_surface((*surface).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ViewEffectErased>>() {
            Ok(effect) => {
                return RenderNode::build_view_effect((*effect).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Metadata<AppliedFilter>>() {
            Ok(meta) => {
                let Metadata { content, value } = *meta;
                return RenderNode::build_applied_filter(value, content, env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Dynamic>>() {
            Ok(dynamic) => {
                return RenderNode::build_dynamic_host((*dynamic).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        // A native widget leaf: build a `Widget` node that re-renders it every flush
        // from a retained, signal-holding config (action retained behind `Rc`), so
        // its handler re-reads live signals — reactive labels/values stay live
        // instead of freezing in a one-shot `Captured` bake.
        let view = match view.downcast::<Native<ButtonConfig>>() {
            Ok(button) => {
                return RenderNode::build_button((*button).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedMenu>>() {
            Ok(menu) => return RenderNode::build_menu((*menu).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ToggleConfig>>() {
            Ok(toggle) => return RenderNode::build_toggle((*toggle).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SliderConfig>>() {
            Ok(slider) => return RenderNode::build_slider((*slider).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<StepperConfig>>() {
            Ok(stepper) => {
                return RenderNode::build_stepper((*stepper).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ProgressConfig>>() {
            Ok(progress) => {
                return RenderNode::build_progress((*progress).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<DatePickerConfig>>() {
            Ok(date_picker) => {
                return RenderNode::build_date_picker((*date_picker).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ColorPickerConfig>>() {
            Ok(color_picker) => {
                return RenderNode::build_color_picker((*color_picker).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<PickerConfig>>() {
            Ok(picker) => {
                return RenderNode::build_picker((*picker).into_inner(), env);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedTextFieldConfig>>() {
            Ok(text_field) => {
                return RenderNode::build_text_field((*text_field).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SecureFieldConfig>>() {
            Ok(secure_field) => {
                return RenderNode::build_secure_field((*secure_field).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<BadgeConfig>>() {
            Ok(badge) => return RenderNode::build_badge((*badge).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ListConfig>>() {
            Ok(list) => return RenderNode::build_list((*list).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<TableConfig>>() {
            Ok(table) => return RenderNode::build_table((*table).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<SystemIcon>>() {
            Ok(icon) => return RenderNode::build_icon((*icon).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedGradient>>() {
            Ok(gradient) => return RenderNode::build_gradient((*gradient).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedShape>>() {
            Ok(shape) => return RenderNode::build_shape((*shape).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<ResolvedMorphShape>>() {
            Ok(shape) => return RenderNode::build_morph_shape((*shape).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<MapConfig>>() {
            Ok(map) => return RenderNode::build_map((*map).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<WebView>() {
            Ok(webview) => return RenderNode::build_webview(*webview, env, renderer),
            Err(view) => view,
        };
        // Navigation containers (navigation view / split / stack / tabs): each is a
        // persistent `Widget` node re-rendered every flush from a retained config —
        // the navigation controller, transition slot, and tab-bar state stay live
        // and reactive route/tab selection keeps driving updates, instead of
        // freezing in a one-shot `Captured` bake.
        let view = match view.downcast::<Native<NavigationView>>() {
            Ok(navigation) => {
                return RenderNode::build_navigation_view(
                    (*navigation).into_inner(),
                    env,
                    renderer,
                );
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<NavigationSplitLayout>>() {
            Ok(split) => {
                return RenderNode::build_navigation_split((*split).into_inner(), env, renderer);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<NavigationStack<(), ()>>>() {
            Ok(stack) => {
                return RenderNode::build_navigation_stack((*stack).into_inner(), env);
            }
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Tabs>>() {
            Ok(tabs) => return RenderNode::build_tabs((*tabs).into_inner(), env, renderer),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<Spacer>>() {
            Ok(spacer) => return RenderNode::build_spacer((*spacer).into_inner(), env),
            Err(view) => view,
        };
        let view = match view.downcast::<Native<()>>() {
            // `Native<()>` carries no data — drop the wrapper and build the empty leaf.
            Ok(_) => return RenderNode::build_empty(env),
            Err(view) => view,
        };
        // `Divider` and `Str` are registered renderers (not `Native<…>` leaves), so
        // they are downcast as their value type directly and built into persistent
        // `Widget` nodes that re-render from a retained cell each flush.
        let view = match view.downcast::<Divider>() {
            Ok(divider) => return RenderNode::build_divider(*divider, env),
            Err(view) => view,
        };
        let view = match view.downcast::<Str>() {
            Ok(text) => return RenderNode::build_str(*text, env),
            Err(view) => view,
        };
        // Every native leaf and metadata wrapper now has a dedicated `RenderNode`
        // build arm above. Anything reaching here is a composite, expanded via
        // `body()` once. A native leaf with no build arm panics here in `body()` —
        // the acceptable fast-fail for a missing arm.
        RenderNode::build(AnyView::new(view.body(env)), env, renderer)
    }

    /// Build a transparent wrapper node: capture the per-flush effect and recurse
    /// into the child so reactive descendants reach their own dedicated nodes.
    fn build_wrapper(
        effect: WrapperEffect,
        content: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        RenderNode::Wrapper(Box::new(WrapperNode {
            effect,
            env: env.clone(),
            child: RenderNode::build(content, env, renderer),
        }))
    }

    /// Build a node-owned lifecycle hook wrapper. An appear hook fires its callback
    /// once now (before the child subtree is built, matching the old dispatch
    /// order); a disappear hook is retained in the effect and fired from its `Drop`
    /// when the node leaves the retained tree. No frame-diff slot cursor — structural
    /// removal alone drives disappearance.
    fn build_lifecycle(
        hook: LifeCycleHook,
        content: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let effect = match hook.lifecycle() {
            LifeCycle::Appear => {
                hook.handle(env);
                LifeCycleEffect { disappear: None }
            }
            LifeCycle::Disappear => LifeCycleEffect {
                disappear: Some(DeferredLifeCycleHook::new(hook, env.clone())),
            },
            _ => panic!("hydrolysis lifecycle variant is not supported"),
        };
        RenderNode::build_wrapper(WrapperEffect::LifeCycle(effect), content, env, renderer)
    }

    /// Build an environment-scoping `Env` node from an [`IgnorableMetadata`] whose
    /// only effect is `env.insert(value)` (the accessibility metadata wrappers): the
    /// extended environment travels with the node so it is read at every flush, and
    /// the wrapped content recurses so reactive descendants stay live.
    fn build_env_scoped<T: MetadataKey + Clone + 'static>(
        meta: IgnorableMetadata<T>,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let IgnorableMetadata { content, value } = meta;
        let scoped = a11y_scoped_env(env, &value);
        let child = RenderNode::build(content, &scoped, renderer);
        RenderNode::Env(Box::new(EnvNode { env: scoped, child }))
    }

    /// Build a retained reactive collection (non-virtualized): materialize every
    /// current item keyed by id and subscribe to membership changes (a change marks
    /// it dirty and schedules a refresh, which reconciles by id).
    fn build_collection(
        layout: Box<dyn Layout>,
        views: AnyViews<AnyView>,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let dirty = Rc::new(Cell::new(false));
        let dirty_key = Rc::new(());
        let key = Rc::as_ptr(&dirty_key) as usize;
        let signals = renderer.signals.clone();
        let guard = views.watch(.., {
            let dirty = Rc::clone(&dirty);
            move |_changed| {
                dirty.set(true);
                signals.mark_collection_dirty(key, 0);
            }
        });
        let len = views.len().get();
        let items = (0..len)
            .map(|index| {
                let id = views
                    .get_id(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"));
                let view = views
                    .get_view(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                (
                    id,
                    RenderNode::build(normalize_layout_view(view, env), env, renderer),
                )
            })
            .collect();
        RenderNode::Collection(Box::new(CollectionNode {
            layout,
            views,
            env: env.clone(),
            items,
            placed: Vec::new(),
            dirty,
            dirty_key,
            guard,
        }))
    }

    /// Build a viewport-virtualized lazy stack: store the collection and subscribe
    /// to membership changes (a change schedules a refresh so the visible window
    /// re-resolves). Items are materialized lazily at flush, not here.
    fn build_lazy_stack(
        axis: LazyStackAxisConfig,
        views: AnyViews<AnyView>,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let dirty_key = Rc::new(());
        let key = Rc::as_ptr(&dirty_key) as usize;
        let signals = renderer.signals.clone();
        let guard = views.watch(.., move |_changed| {
            // Membership changed: request a fine-grained refresh; the flush re-reads
            // the collection length/items and re-resolves the visible window.
            signals.mark_collection_dirty(key, 0);
        });
        RenderNode::LazyStack(Box::new(LazyStackNode {
            axis,
            views,
            env: env.clone(),
            item_extents: RefCell::new(Vec::new()),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            estimate: Cell::new(0.0),
            dirty_key,
            guard,
        }))
    }

    /// Build a self-drawn scene node owning its `SceneContent` (no effect slot).
    fn build_scene_view_node(
        scene_view: Native<SceneView>,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let mut content = scene_view.into_inner().into_content();
        let signals = renderer.signals.clone();
        content.set_invalidator(Some(Rc::new(move || {
            signals.request_next_frame_rebuild();
        })));
        RenderNode::SceneView(Box::new(SceneViewNode {
            content: RefCell::new(content),
        }))
    }

    /// Build an embedded `GpuSurface` node owning its
    /// [`EmbeddedGpuSurfaceRuntime`] directly. The runtime is shared with the
    /// renderer's node-surface registry so its off-thread redraw handle is polled
    /// even on frames that do not re-flush the tree.
    fn build_gpu_surface(
        surface: GpuSurface,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let runtime = Rc::new(RefCell::new(EmbeddedGpuSurfaceRuntime::new(surface, env)));
        renderer.register_node_gpu_surface(Rc::clone(&runtime));
        RenderNode::GpuSurface(Box::new(GpuSurfaceNode { runtime }))
    }

    /// Build a `ViewEffect` node owning its [`ViewEffectRuntime`] and building its
    /// captured content as a persistent child [`RenderNode`] (recursed into, not
    /// baked), so reactive descendants inside the effect stay live.
    fn build_view_effect(
        mut effect: ViewEffectErased,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let content = effect.take_content();
        let child = RenderNode::build(normalize_layout_view(content, env), env, renderer);
        RenderNode::ViewEffect(Box::new(ViewEffectNode {
            runtime: RefCell::new(ViewEffectRuntime::new(effect)),
            child: RefCell::new(child),
            laid_out: Cell::new(Size::zero()),
            env: env.clone(),
        }))
    }

    /// Build an `AppliedFilter` node owning its [`AppliedFilterRuntime`]
    /// (input/output textures) and building its wrapped content as a persistent
    /// child [`RenderNode`]. The runtime is registered with the renderer so
    /// animated filters refresh on redraw-only frames.
    fn build_applied_filter(
        filter: AppliedFilter,
        content: AnyView,
        env: &Environment,
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let runtime = Rc::new(RefCell::new(AppliedFilterRuntime::new(filter)));
        renderer.register_node_applied_filter(Rc::clone(&runtime));
        let child = RenderNode::build(normalize_layout_view(content, env), env, renderer);
        RenderNode::AppliedFilter(Box::new(AppliedFilterNode {
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
        renderer: &mut HydrolysisRenderer,
    ) -> RenderNode {
        let identity = dynamic.identity();
        let pending: Rc<RefCell<Option<AnyView>>> = Rc::new(RefCell::new(None));
        let source = dynamic.clone();
        let signals = renderer.signals.clone();
        dynamic.connect_with_pending_view(Rc::clone(&pending), {
            let pending = Rc::clone(&pending);
            move |update| {
                let is_initial = update
                    .metadata()
                    .try_get::<DynamicInitialContent>()
                    .is_some();
                *pending.borrow_mut() = Some(update.into_value());
                // A real content change schedules a fine-grained patch; the
                // render tree rebuilds only this node's child on the next frame.
                if !is_initial {
                    signals.mark_dynamic_dirty(identity, 0);
                }
            }
        });
        let initial = pending.borrow_mut().take();
        let child = match initial {
            Some(content) => RenderNode::build(content, env, renderer),
            None => RenderNode::build(AnyView::new(()), env, renderer),
        };
        RenderNode::Dynamic(Box::new(DynamicHostNode {
            source,
            pending,
            env: env.clone(),
            child,
        }))
    }
}
