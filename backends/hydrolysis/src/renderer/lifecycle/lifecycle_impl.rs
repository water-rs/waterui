use super::*;

pub(crate) struct LifecycleState {
    pub(crate) disappear_previous: BTreeMap<usize, DeferredLifeCycleHook>,
    pub(crate) disappear_current: BTreeMap<usize, DeferredLifeCycleHook>,
    pub(crate) disappear_slot: usize,
    pub(crate) dynamic_nodes: BTreeMap<usize, DynamicNode>,
    pub(crate) dynamic_identities_current_frame: Vec<usize>,
    pub(crate) current_frame_retain: Vec<Retain>,
    pub(crate) previous_frame_retain: Vec<Retain>,
}

pub(crate) struct DeferredLifeCycleHook {
    pub(crate) env: Environment,
    pub(crate) hook: LifeCycleHook,
}

pub(crate) struct DynamicNode {
    pub(crate) pending_view: Rc<RefCell<Option<AnyView>>>,
    pub(crate) cached_subtree: Option<DynamicSubtree>,
    pub(crate) render_generation: Rc<Cell<u64>>,
    /// The render context and environment this node was last dispatched with, used to
    /// re-dispatch the node in isolation during a reactive patch.
    pub(crate) dispatch_ctx: Option<RenderContext>,
    pub(crate) dispatch_env: Option<Environment>,
}

pub(crate) struct DynamicSubtree {
    pub(crate) depth_base: usize,
    /// Draw operations in dispatch (painter's) order. Static scene segments are
    /// interleaved with the deferred dynamic draws so replay reproduces the exact
    /// z-order of the original dispatch: a `Dynamic`-node overlay dispatched after
    /// a sibling scroll composites *above* the scroll's content, not behind it.
    pub(crate) draw_ops: Vec<DynamicDrawOp>,
    pub(crate) retains: Vec<Retain>,
    pub(crate) pointer_targets: Vec<PointerTarget>,
    pub(crate) gesture_targets: Vec<GestureTarget>,
    pub(crate) cursor_targets: Vec<CursorTarget>,
    pub(crate) hover_targets: Vec<HoverTarget>,
    pub(crate) text_input_targets: Vec<TextInputTarget>,
    pub(crate) scroll_targets: Vec<ScrollTarget>,
    pub(crate) context_menu_targets: Vec<ContextMenuTarget>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility: DynamicAccessibilitySubtree,
}

/// One ordered draw step in a [`DynamicSubtree`]. Replayed in slice order, so the
/// relative z-order of static content and every kind of deferred dynamic draw is
/// preserved across capture and replay. This is the single source of painter's
/// order for a captured subtree — there is no separate per-type batching that
/// could reorder a dynamic draw relative to its static siblings.
pub(crate) enum DynamicDrawOp {
    /// A baked static scene segment: everything drawn between the previous and the
    /// next dynamic draw at this subtree level.
    Static(vello::Scene),
    /// An animated-transform fragment, re-sampled and composited at replay.
    Transform(DynamicTransformDraw),
    /// An animated-opacity layer, re-sampled and composited at replay.
    Opacity(DynamicOpacityDraw),
    /// A placed `Dynamic` node, composited from its retained cached subtree.
    Node(DynamicNodeDraw),
    /// A scroll view, composited from its offset-independent content cache.
    Scroll(DynamicScrollDraw),
    /// A reactive collection (`ForEach`/`List` in a non-virtualized layout),
    /// composited from its per-item retained subtree cache. A membership change
    /// reconciles only the changed items, mirroring the `Dynamic`-node patch.
    Collection(DynamicCollectionDraw),
}

#[cfg(feature = "accessibility")]
pub(crate) struct DynamicAccessibilitySubtree {
    pub(crate) nodes: Vec<(AccessibilityNodeId, AccessibilityNode)>,
    pub(crate) root_children: Vec<AccessibilityNodeId>,
    pub(crate) actions: BTreeMap<AccessibilityNodeId, AccessibilityActionTarget>,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self {
            disappear_previous: BTreeMap::new(),
            disappear_current: BTreeMap::new(),
            disappear_slot: 0,
            dynamic_nodes: BTreeMap::new(),
            dynamic_identities_current_frame: Vec::new(),
            current_frame_retain: Vec::new(),
            previous_frame_retain: Vec::new(),
        }
    }
}

impl LifecycleState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.previous_frame_retain.clear();
        self.current_frame_retain.clear();
        self.disappear_current.clear();
        self.disappear_slot = 0;
        self.dynamic_identities_current_frame.clear();
    }

    pub(crate) fn finish_rebuild_frame(
        &mut self,
        state: &mut HydroState,
        preserve_inactive_state: bool,
    ) {
        self.previous_frame_retain = core::mem::take(&mut self.current_frame_retain);

        let previous_hooks = core::mem::take(&mut self.disappear_previous);
        for (slot, hook) in previous_hooks {
            if !self.disappear_current.contains_key(&slot) {
                hook.call();
            }
        }
        self.disappear_previous = core::mem::take(&mut self.disappear_current);

        let active_dynamic_identities = self.dynamic_identities_current_frame.clone();
        if !preserve_inactive_state {
            self.dynamic_nodes
                .retain(|identity, _| active_dynamic_identities.contains(identity));
            state.measurement.retain_dynamic_identities(|identity| {
                active_dynamic_identities.contains(&identity)
            });
        }
    }

    pub(crate) fn drop_all_hooks(&mut self) {
        for (_, hook) in core::mem::take(&mut self.disappear_previous) {
            hook.call();
        }
        for (_, hook) in core::mem::take(&mut self.disappear_current) {
            hook.call();
        }
    }
}

impl DeferredLifeCycleHook {
    pub(crate) fn new(hook: LifeCycleHook, env: Environment) -> Self {
        Self { env, hook }
    }

    pub(crate) fn call(self) {
        self.hook.handle(&self.env);
    }
}

impl DynamicSubtree {
    /// Fresh, empty per-subtree storage about to receive a capture.
    pub(crate) fn for_capture(depth_base: usize) -> Self {
        Self {
            depth_base,
            draw_ops: Vec::new(),
            retains: Vec::new(),
            pointer_targets: Vec::new(),
            gesture_targets: Vec::new(),
            cursor_targets: Vec::new(),
            hover_targets: Vec::new(),
            text_input_targets: Vec::new(),
            scroll_targets: Vec::new(),
            context_menu_targets: Vec::new(),
            #[cfg(feature = "accessibility")]
            accessibility: DynamicAccessibilitySubtree {
                nodes: Vec::new(),
                root_children: Vec::new(),
                actions: BTreeMap::new(),
            },
        }
    }

    /// The animated-transform fragments in this subtree, in draw order.
    pub(crate) fn transform_draws(&self) -> impl Iterator<Item = &DynamicTransformDraw> {
        self.draw_ops.iter().filter_map(|op| match op {
            DynamicDrawOp::Transform(draw) => Some(draw),
            _ => None,
        })
    }

    /// The animated-opacity layers in this subtree, in draw order.
    pub(crate) fn opacity_draws(&self) -> impl Iterator<Item = &DynamicOpacityDraw> {
        self.draw_ops.iter().filter_map(|op| match op {
            DynamicDrawOp::Opacity(draw) => Some(draw),
            _ => None,
        })
    }

    /// The placed `Dynamic` nodes in this subtree, in draw order.
    pub(crate) fn node_draws(&self) -> impl Iterator<Item = &DynamicNodeDraw> {
        self.draw_ops.iter().filter_map(|op| match op {
            DynamicDrawOp::Node(draw) => Some(draw),
            _ => None,
        })
    }

    /// The scroll views in this subtree, in draw order.
    pub(crate) fn scroll_draws(&self) -> impl Iterator<Item = &DynamicScrollDraw> {
        self.draw_ops.iter().filter_map(|op| match op {
            DynamicDrawOp::Scroll(draw) => Some(draw),
            _ => None,
        })
    }

    /// The reactive collections in this subtree, in draw order.
    pub(crate) fn collection_draws(&self) -> impl Iterator<Item = &DynamicCollectionDraw> {
        self.draw_ops.iter().filter_map(|op| match op {
            DynamicDrawOp::Collection(draw) => Some(draw),
            _ => None,
        })
    }
}

/// Parks the surrounding frame's capture state while a dynamic subtree is
/// dispatched into fresh storage.
///
/// [`SubtreeCaptureScope::exchange_with`] is the single source of truth for
/// which renderer state participates in a capture: `begin` exchanges the
/// renderer's live state with fresh storage, the caller dispatches the
/// subtree content, and `finish` exchanges back and yields the captured
/// [`DynamicSubtree`]. Begin/finish are deliberately explicit (not a `Drop`
/// guard): a panic mid-dispatch is fatal to the renderer by design.
struct SubtreeCaptureScope {
    parked: DynamicSubtree,
    /// The parent capture's current static scene segment, parked while the child
    /// subtree accumulates into a fresh scene. The child's draw ops own their own
    /// static segments, so the subtree itself carries no standalone scene.
    parked_scene: vello::Scene,
    hover_controller: HoverController,
    #[cfg(feature = "accessibility")]
    saved_next_node_id: u64,
    #[cfg(feature = "accessibility")]
    saved_suppression_depth: usize,
}

impl SubtreeCaptureScope {
    fn begin(renderer: &mut HydrolysisRenderer) -> Self {
        let mut scope = Self {
            parked: DynamicSubtree::for_capture(renderer.render_depth),
            parked_scene: vello::Scene::new(),
            hover_controller: HoverController::default(),
            #[cfg(feature = "accessibility")]
            saved_next_node_id: 0,
            #[cfg(feature = "accessibility")]
            saved_suppression_depth: 0,
        };
        scope.exchange_with(renderer);
        #[cfg(feature = "accessibility")]
        {
            scope.saved_next_node_id = renderer.accessibility.next_node_id;
            scope.saved_suppression_depth = renderer.accessibility.suppression_depth;
            renderer.accessibility.next_node_id = ACCESSIBILITY_FIRST_NODE_ID;
            renderer.accessibility.suppression_depth = 0;
        }
        scope
    }

    fn finish(mut self, renderer: &mut HydrolysisRenderer) -> DynamicSubtree {
        #[cfg(feature = "accessibility")]
        {
            renderer.accessibility.suppression_depth = self.saved_suppression_depth;
            renderer.accessibility.next_node_id = self.saved_next_node_id;
        }
        // Seal the trailing static segment (everything drawn after the last
        // dynamic draw) before handing the renderer's live state back.
        renderer.flush_static_segment();
        self.exchange_with(renderer);
        self.parked
    }

    /// Exchange the renderer's live capture state with this scope's parked
    /// state. Called exactly twice per capture, so every field listed here is
    /// guaranteed to be restored.
    fn exchange_with(&mut self, renderer: &mut HydrolysisRenderer) {
        let parked = &mut self.parked;
        core::mem::swap(&mut renderer.scene, &mut self.parked_scene);
        core::mem::swap(&mut renderer.draw_ops, &mut parked.draw_ops);
        core::mem::swap(
            &mut renderer.lifecycle.current_frame_retain,
            &mut parked.retains,
        );
        core::mem::swap(
            &mut renderer.hit_test.pointer_targets,
            &mut parked.pointer_targets,
        );
        renderer
            .gesture_engine
            .swap_targets(&mut parked.gesture_targets);
        core::mem::swap(
            &mut renderer.hit_test.cursor_targets,
            &mut parked.cursor_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.hover_targets,
            &mut parked.hover_targets,
        );
        renderer
            .hit_test
            .interaction
            .swap_hover_controller(&mut self.hover_controller);
        core::mem::swap(
            &mut renderer.text_editing.text_input_targets,
            &mut parked.text_input_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.scroll_targets,
            &mut parked.scroll_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.context_menu_targets,
            &mut parked.context_menu_targets,
        );
        #[cfg(feature = "accessibility")]
        renderer.accessibility.swap_render_state(
            &mut parked.accessibility.nodes,
            &mut parked.accessibility.root_children,
            &mut parked.accessibility.actions,
        );
    }
}

impl HydrolysisRenderer {
    pub(crate) fn render_dynamic_subtree(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> DynamicSubtree {
        let scope = SubtreeCaptureScope::begin(renderer);
        renderer.dispatch_boxed_with_render_depth(content, env, ctx);
        scope.finish(renderer)
    }

    pub(crate) fn render_dynamic_subtree_with_local_interactions(
        renderer: &mut HydrolysisRenderer,
        replay_ctx: RenderContext,
        local_ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> DynamicSubtree {
        let active_press_origin = renderer.hit_test.active_press_origin;
        let active_press_bounds = renderer.hit_test.active_press_bounds;
        let inverse_hit_transform = replay_ctx.hit_transform.inverse();
        renderer.hit_test.active_press_origin =
            active_press_origin.map(|origin| inverse_hit_transform * origin);
        renderer.hit_test.active_press_bounds =
            active_press_bounds.map(|bounds| transformed_rect(inverse_hit_transform, bounds));
        let subtree = Self::render_dynamic_subtree(renderer, local_ctx, env, content);
        renderer.hit_test.active_press_origin = active_press_origin;
        renderer.hit_test.active_press_bounds = active_press_bounds;
        subtree
    }

    pub(crate) fn replay_dynamic_subtree(&mut self, ctx: RenderContext, subtree: &DynamicSubtree) {
        let _retained_watcher_count = subtree.retains.len();
        // Replay every draw op in dispatch order, so static content and each kind
        // of deferred dynamic draw composite in their original painter's order.
        for op in &subtree.draw_ops {
            match op {
                DynamicDrawOp::Static(scene) => self.scene.append(scene, Some(ctx.transform)),
                DynamicDrawOp::Transform(draw) => self.draw_dynamic_transform(ctx, draw),
                DynamicDrawOp::Opacity(draw) => self.draw_dynamic_opacity(ctx, draw),
                DynamicDrawOp::Node(placement) => {
                    self.replay_dynamic_node_placement(ctx, placement);
                }
                DynamicDrawOp::Scroll(draw) => self.replay_dynamic_scroll_draw(ctx, draw),
                DynamicDrawOp::Collection(draw) => self.replay_dynamic_collection(ctx, draw),
            }
        }
        let mut gesture_group_remap = BTreeMap::new();

        for target in &subtree.pointer_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            let press_slot = if target.press_slot.is_some()
                && self.hit_test.hit_test_opacity > HIT_TEST_ALPHA_THRESHOLD
            {
                let slot = self.hit_test.interaction.bind_press_slot();
                if let Some(handles) = &target.interaction {
                    // The handle bundle travels with the target across replays;
                    // re-attach it to the freshly bound slot. The press origin is
                    // kept in window space, so no per-replay transform refresh is
                    // needed — the ripple maps it to local at draw time.
                    self.hit_test
                        .interaction
                        .attach_handles(slot, Rc::clone(handles));
                }
                Some(slot)
            } else {
                None
            };
            self.register_pointer_target_action(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.captures_drag,
                press_slot,
                Rc::clone(&target.action),
                depth,
            );
        }
        for target in &subtree.gesture_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            let group_id = *gesture_group_remap
                .entry(target.group_id)
                .or_insert_with(|| self.allocate_gesture_group_id());
            self.register_gesture_target_recognizer(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.clone(),
                depth,
                group_id,
            );
        }
        for target in &subtree.cursor_targets {
            self.register_cursor_target_style(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.style,
            );
        }
        for target in &subtree.hover_targets {
            let bounds = transformed_rect(ctx.hit_transform, target.bounds);
            self.register_hover_target_with_handles(
                bounds,
                target.handles.clone(),
                target.on_enter.clone(),
                target.on_move.clone(),
                target.on_exit.clone(),
            );
        }
        #[cfg(feature = "accessibility")]
        let accessibility_node_map =
            self.replay_dynamic_accessibility_subtree(ctx.hit_transform, &subtree.accessibility);
        for target in &subtree.text_input_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            #[cfg(feature = "accessibility")]
            let accessibility_node_id = target
                .accessibility_node_id
                .map(|node_id| accessibility_node_map.map(node_id));
            self.register_text_input_target_data(text_editing::TextInputTargetData {
                target: TextInputTargetRegistration {
                    bounds: transformed_rect(ctx.hit_transform, target.bounds),
                    cursor_area: transformed_rect(ctx.hit_transform, target.cursor_area),
                    text_bounds: transformed_rect(ctx.hit_transform, target.text_bounds),
                    text_clip_bounds: transformed_rect(ctx.hit_transform, target.text_clip_bounds),
                    content_alpha: target.content_alpha,
                    layout: target.layout.clone(),
                    purpose: target.purpose,
                    model: target.model.clone(),
                    selection: Rc::clone(&target.selection),
                },
                depth,
                focus_binding: target.focus_binding.clone(),
                #[cfg(feature = "accessibility")]
                accessibility_node_id,
            });
        }
        for target in &subtree.scroll_targets {
            self.register_scroll_target_action(
                transformed_rect(ctx.hit_transform, target.bounds),
                Rc::clone(&target.action),
            );
        }
        for target in &subtree.context_menu_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            self.register_context_menu_target_data(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.items.clone(),
                depth,
            );
        }
    }

    pub(crate) fn render_lifecycle_hook_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<LifeCycleHook>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        match value.lifecycle() {
            LifeCycle::Appear => value.handle(env),
            LifeCycle::Disappear => {
                let slot = renderer.lifecycle.disappear_slot;
                renderer.lifecycle.disappear_slot += 1;
                renderer
                    .lifecycle
                    .disappear_current
                    .insert(slot, DeferredLifeCycleHook::new(value, env.clone()));
            }
            _ => panic!("hydrolysis lifecycle variant is not supported"),
        }
        Self::dispatch_any(renderer, ctx, env, content);
    }
}
