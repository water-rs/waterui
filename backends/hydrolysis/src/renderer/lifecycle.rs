use super::*;

use core::any::TypeId;
use std::collections::BTreeSet;

type LocalStateKey = (u64, usize);

#[derive(Clone)]
pub(super) struct LocalStateSlot {
    pub(super) type_id: TypeId,
    pub(super) type_name: &'static str,
    pub(super) value: Rc<dyn Any>,
}

#[derive(Default)]
pub(super) struct LocalStateRegistry {
    pub(super) slots: BTreeMap<LocalStateKey, LocalStateSlot>,
    pub(super) active_keys: BTreeSet<LocalStateKey>,
}

pub(super) struct LifecycleState {
    pub(super) disappear_previous: BTreeMap<usize, DeferredLifeCycleHook>,
    pub(super) disappear_current: BTreeMap<usize, DeferredLifeCycleHook>,
    pub(super) disappear_slot: usize,
    pub(super) dynamic_nodes: BTreeMap<usize, DynamicNode>,
    pub(super) dynamic_identities_current_frame: Vec<usize>,
    pub(super) local_state_registry: Rc<RefCell<LocalStateRegistry>>,
    pub(super) current_frame_retain: Vec<Retain>,
    pub(super) previous_frame_retain: Vec<Retain>,
}

pub(super) struct DeferredLifeCycleHook {
    pub(super) env: Environment,
    pub(super) hook: LifeCycleHook,
}

pub(super) struct DynamicNode {
    pub(super) pending_view: Rc<RefCell<Option<AnyView>>>,
    pub(super) cached_subtree: Option<DynamicSubtree>,
    pub(super) render_generation: Rc<Cell<u64>>,
}

pub(super) struct DynamicSubtree {
    pub(super) scene: vello::Scene,
    pub(super) depth_base: usize,
    pub(super) pointer_targets: Vec<PointerTarget>,
    pub(super) gesture_targets: Vec<GestureTarget>,
    pub(super) cursor_targets: Vec<CursorTarget>,
    pub(super) hover_targets: Vec<HoverTarget>,
    pub(super) text_input_targets: Vec<TextInputTarget>,
    pub(super) scroll_targets: Vec<ScrollTarget>,
    #[cfg(feature = "accessibility")]
    pub(super) accessibility: DynamicAccessibilitySubtree,
}

#[cfg(feature = "accessibility")]
pub(super) struct DynamicAccessibilitySubtree {
    pub(super) nodes: Vec<(AccessibilityNodeId, AccessibilityNode)>,
    pub(super) root_children: Vec<AccessibilityNodeId>,
    pub(super) actions: BTreeMap<AccessibilityNodeId, AccessibilityActionTarget>,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self {
            disappear_previous: BTreeMap::new(),
            disappear_current: BTreeMap::new(),
            disappear_slot: 0,
            dynamic_nodes: BTreeMap::new(),
            dynamic_identities_current_frame: Vec::new(),
            local_state_registry: Rc::new(RefCell::new(LocalStateRegistry::default())),
            current_frame_retain: Vec::new(),
            previous_frame_retain: Vec::new(),
        }
    }
}

impl LocalStateRegistry {
    pub(super) fn begin_rebuild_frame(&mut self) {
        self.active_keys.clear();
    }

    pub(super) fn finish_rebuild_frame(&mut self) {
        self.slots.retain(|key, _| self.active_keys.contains(key));
    }

    pub(super) fn bind_slot(
        &mut self,
        path: u64,
        index: usize,
        type_id: TypeId,
        current_type_name: &'static str,
        init: &dyn Fn() -> Rc<dyn Any>,
    ) -> Rc<dyn Any> {
        let key = (path, index);
        self.active_keys.insert(key);
        if let Some(slot) = self.slots.get(&key) {
            assert!(
                slot.type_id == type_id,
                "hydrolysis local state slot type mismatch at path {} slot {}: existing={}, requested={}",
                path,
                index,
                slot.type_name,
                current_type_name
            );
            return Rc::clone(&slot.value);
        }
        let value = init();
        self.slots.insert(
            key,
            LocalStateSlot {
                type_id,
                type_name: current_type_name,
                value: Rc::clone(&value),
            },
        );
        value
    }
}

impl LifecycleState {
    pub(super) fn begin_rebuild_frame(&mut self) {
        self.current_frame_retain.clear();
        self.disappear_current.clear();
        self.disappear_slot = 0;
        self.dynamic_identities_current_frame.clear();
        self.local_state_registry.borrow_mut().begin_rebuild_frame();
    }

    pub(super) fn finish_rebuild_frame(&mut self, state: &mut HydroState) {
        self.previous_frame_retain = core::mem::take(&mut self.current_frame_retain);

        let previous_hooks = core::mem::take(&mut self.disappear_previous);
        for (slot, hook) in previous_hooks {
            if !self.disappear_current.contains_key(&slot) {
                hook.call();
            }
        }
        self.disappear_previous = core::mem::take(&mut self.disappear_current);

        self.local_state_registry
            .borrow_mut()
            .finish_rebuild_frame();
        let active_dynamic_identities = self.dynamic_identities_current_frame.clone();
        self.dynamic_nodes
            .retain(|identity, _| active_dynamic_identities.contains(identity));
        state
            .dynamic_intrinsic_cache
            .retain(|identity, _| active_dynamic_identities.contains(identity));
        state
            .dynamic_dimensions_cache
            .retain(|(identity, _, _), _| active_dynamic_identities.contains(identity));
    }

    pub(super) fn install_local_state_env(&self, env: &Environment) -> Environment {
        let mut local_env = env.clone();
        let local_state_registry = Rc::clone(&self.local_state_registry);
        local_env.insert(
            env.get::<LocalStateScope>()
                .cloned()
                .unwrap_or_else(LocalStateScope::root),
        );
        local_env.insert(LocalStateStore::new(
            move |path, index, type_id, type_name, init| {
                local_state_registry
                    .borrow_mut()
                    .bind_slot(path, index, type_id, type_name, &*init)
            },
        ));
        local_env
    }

    pub(super) fn with_local_state_env<R>(
        &self,
        env: &Environment,
        f: impl FnOnce(&Environment) -> R,
    ) -> R {
        let local_env = self.install_local_state_env(env);
        let scope = local_env
            .get::<LocalStateScope>()
            .unwrap_or_else(|| panic!("hydrolysis local state environment missing LocalStateScope"))
            .clone();
        let store = local_env
            .get::<LocalStateStore>()
            .unwrap_or_else(|| panic!("hydrolysis local state environment missing LocalStateStore"))
            .clone();
        with_local_binding_factory(
            Rc::new(move |type_id, type_name, init| {
                store.get_or_init_dynamic(&scope, type_id, type_name, init)
            }),
            || f(&local_env),
        )
    }

    pub(super) fn drop_all_hooks(&mut self) {
        for (_, hook) in core::mem::take(&mut self.disappear_previous) {
            hook.call();
        }
        for (_, hook) in core::mem::take(&mut self.disappear_current) {
            hook.call();
        }
    }
}

impl DeferredLifeCycleHook {
    pub(super) fn new(hook: LifeCycleHook, env: Environment) -> Self {
        Self { env, hook }
    }

    pub(super) fn call(self) {
        self.hook.handle(&self.env);
    }
}

pub(super) fn local_state_body_env(env: &Environment) -> Environment {
    env.get::<LocalStateScope>()
        .map_or_else(|| env.clone(), |scope| env.extending(scope.reset()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_local_state_env_preserves_existing_scope() {
        let lifecycle = LifecycleState::default();
        let root_env = lifecycle.install_local_state_env(&Environment::new());
        let root_scope = root_env
            .get::<LocalStateScope>()
            .expect("root local state scope should exist")
            .clone();
        let root_store = root_env
            .get::<LocalStateStore>()
            .expect("root local state store should exist")
            .clone();
        let root_binding = root_store.binding(&root_scope, || 1_i32);
        assert_eq!(root_binding.get(), 1);

        let child_input_env = local_state_child_env(&root_env, 7);
        let child_env = lifecycle.install_local_state_env(&child_input_env);
        let child_scope = child_env
            .get::<LocalStateScope>()
            .expect("child local state scope should exist")
            .clone();
        let child_store = child_env
            .get::<LocalStateStore>()
            .expect("child local state store should exist")
            .clone();
        let child_binding = child_store.binding(&child_scope, || 2_i32);

        assert_eq!(
            child_binding.get(),
            2,
            "install_local_state_env must preserve child scopes instead of rebasing them to root"
        );
    }
}

pub(super) fn local_state_body_content_env(env: &Environment) -> Environment {
    env.get::<LocalStateScope>()
        .map_or_else(|| env.clone(), |scope| env.extending(scope.child(0)))
}

pub(super) fn local_state_child_env(env: &Environment, index: usize) -> Environment {
    env.get::<LocalStateScope>()
        .map_or_else(|| env.clone(), |scope| env.extending(scope.child(index)))
}

pub(super) fn local_state_overlay_env(base: &Environment, current: &Environment) -> Environment {
    current
        .get::<LocalStateScope>()
        .map_or_else(|| base.clone(), |scope| base.extending(scope.clone()))
}

pub(crate) fn local_shared<T: 'static>(
    env: &Environment,
    init: impl FnOnce() -> T + 'static,
) -> Rc<T> {
    let scope = env
        .get::<LocalStateScope>()
        .unwrap_or_else(|| {
            panic!("hydrolysis requires renderer LocalStateScope support for internal state")
        })
        .clone();
    env.get::<LocalStateStore>()
        .unwrap_or_else(|| {
            panic!("hydrolysis requires renderer LocalStateStore support for internal state")
        })
        .get_or_init(&scope, init)
}

impl HydrolysisRenderer {
    pub(super) fn render_dynamic_subtree(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> DynamicSubtree {
        let subtree_depth_base = renderer.render_depth;
        let mut subtree_scene = vello::Scene::new();
        let mut subtree_pointer_targets = Vec::new();
        let mut subtree_gesture_targets = Vec::new();
        let mut subtree_cursor_targets = Vec::new();
        let mut subtree_hover_targets = Vec::new();
        let mut subtree_text_input_targets = Vec::new();
        let mut subtree_scroll_targets = Vec::new();
        let mut subtree_hover_controller = HoverController::default();
        #[cfg(feature = "accessibility")]
        let mut subtree_accessibility_nodes = Vec::new();
        #[cfg(feature = "accessibility")]
        let mut subtree_accessibility_root_children = Vec::new();
        #[cfg(feature = "accessibility")]
        let mut subtree_accessibility_actions = BTreeMap::new();

        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        core::mem::swap(
            &mut renderer.hit_test.pointer_targets,
            &mut subtree_pointer_targets,
        );
        renderer
            .gesture_engine
            .swap_targets(&mut subtree_gesture_targets);
        core::mem::swap(
            &mut renderer.hit_test.cursor_targets,
            &mut subtree_cursor_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.hover_targets,
            &mut subtree_hover_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.hover_controller,
            &mut subtree_hover_controller,
        );
        core::mem::swap(
            &mut renderer.text_editing.text_input_targets,
            &mut subtree_text_input_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.scroll_targets,
            &mut subtree_scroll_targets,
        );
        #[cfg(feature = "accessibility")]
        {
            renderer.accessibility.swap_render_state(
                &mut subtree_accessibility_nodes,
                &mut subtree_accessibility_root_children,
                &mut subtree_accessibility_actions,
            );

            let previous_next_node_id = renderer.accessibility.next_node_id;
            let previous_suppression_depth = renderer.accessibility.suppression_depth;
            renderer.accessibility.next_node_id = ACCESSIBILITY_FIRST_NODE_ID;
            renderer.accessibility.suppression_depth = 0;
            renderer.dispatch_with_render_depth(content, env, ctx);
            renderer.accessibility.suppression_depth = previous_suppression_depth;
            renderer.accessibility.next_node_id = previous_next_node_id;

            renderer.accessibility.swap_render_state(
                &mut subtree_accessibility_nodes,
                &mut subtree_accessibility_root_children,
                &mut subtree_accessibility_actions,
            );
        }
        #[cfg(not(feature = "accessibility"))]
        renderer.dispatch_with_render_depth(content, env, ctx);

        core::mem::swap(
            &mut renderer.hit_test.scroll_targets,
            &mut subtree_scroll_targets,
        );
        core::mem::swap(
            &mut renderer.text_editing.text_input_targets,
            &mut subtree_text_input_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.hover_controller,
            &mut subtree_hover_controller,
        );
        core::mem::swap(
            &mut renderer.hit_test.hover_targets,
            &mut subtree_hover_targets,
        );
        core::mem::swap(
            &mut renderer.hit_test.cursor_targets,
            &mut subtree_cursor_targets,
        );
        renderer
            .gesture_engine
            .swap_targets(&mut subtree_gesture_targets);
        core::mem::swap(
            &mut renderer.hit_test.pointer_targets,
            &mut subtree_pointer_targets,
        );
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);

        DynamicSubtree {
            scene: subtree_scene,
            depth_base: subtree_depth_base,
            pointer_targets: subtree_pointer_targets,
            gesture_targets: subtree_gesture_targets,
            cursor_targets: subtree_cursor_targets,
            hover_targets: subtree_hover_targets,
            text_input_targets: subtree_text_input_targets,
            scroll_targets: subtree_scroll_targets,
            #[cfg(feature = "accessibility")]
            accessibility: DynamicAccessibilitySubtree {
                nodes: subtree_accessibility_nodes,
                root_children: subtree_accessibility_root_children,
                actions: subtree_accessibility_actions,
            },
        }
    }

    pub(super) fn replay_dynamic_subtree(&mut self, ctx: RenderContext, subtree: &DynamicSubtree) {
        self.scene.append(&subtree.scene, Some(ctx.transform));
        let mut gesture_group_remap = BTreeMap::new();

        for target in &subtree.pointer_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            self.register_pointer_target_action(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.captures_drag,
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
            self.register_hover_target(
                bounds,
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
            let accessibility_node_id = target.accessibility_node_id.map(|node_id| {
                *accessibility_node_map.get(&node_id).unwrap_or_else(|| {
                    panic!(
                        "hydrolysis dynamic text input accessibility node {:?} is missing remap entry",
                        node_id
                    )
                })
            });
            self.register_text_input_target_data(
                transformed_rect(ctx.hit_transform, target.bounds),
                transformed_rect(ctx.hit_transform, target.cursor_area),
                transformed_rect(ctx.hit_transform, target.text_bounds),
                target.layout.clone(),
                target.purpose,
                target.model.clone(),
                Rc::clone(&target.selection),
                depth,
                target.focus_binding.clone(),
                #[cfg(feature = "accessibility")]
                accessibility_node_id,
            );
        }
        for target in &subtree.scroll_targets {
            self.register_scroll_target_action(
                transformed_rect(ctx.hit_transform, target.bounds),
                Rc::clone(&target.action),
            );
        }
    }

    pub(super) fn render_lifecycle_hook_metadata(
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

    pub fn with_local_state_env<R>(
        &self,
        env: &Environment,
        f: impl FnOnce(&Environment) -> R,
    ) -> R {
        self.lifecycle.with_local_state_env(env, f)
    }
}
