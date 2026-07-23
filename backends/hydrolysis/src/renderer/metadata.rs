//! Metadata view handlers: styling, transforms, interaction, lifecycle
//! and accessibility metadata wrappers around content views.

use super::*;

impl HydrolysisRenderer {
    /// Apply a clip-shape layer around the given content render. Shared by the
    /// dispatch handler and the retained `Wrapper` node so the clip effect lives
    /// in exactly one place.
    pub(super) fn apply_clip_shape(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        value: &ClipShape,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let clip_path = path_commands_to_path(value.commands(), ctx.bounds);
        renderer.push_layer_path(1.0, ctx.transform, clip_path);
        render_content(renderer);
        renderer.pop_layer();
    }

    /// Render the given content then stroke the border over it, mirroring the
    /// historical order (content first, border on top). Shared by the dispatch
    /// handler and the retained `Wrapper` node. The border color resolves against
    /// `env`, so it is threaded through.
    pub(super) fn apply_border(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        border: &Border,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        render_content(renderer);

        if border.width <= 0.0 {
            return;
        }

        let brush = resolved_color_to_peniko(border.color.resolve(env).get());
        let width = f64::from(border.width);

        if border.edges.all() && border.corner_radius > 0.0 {
            let rounded =
                vello::kurbo::RoundedRect::from_rect(ctx.bounds, f64::from(border.corner_radius));
            let stroke = vello::kurbo::Stroke::new(width);
            renderer
                .scene
                .stroke(&stroke, ctx.transform, brush, None, &rounded);
            return;
        }

        if border.edges.top {
            let top = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y0 + width,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &top,
            );
        }
        if border.edges.bottom {
            let bottom = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y1 - width,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &bottom,
            );
        }
        if border.edges.leading {
            let leading = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x0 + width,
                ctx.bounds.y1,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &leading,
            );
        }
        if border.edges.trailing {
            let trailing = vello::kurbo::Rect::new(
                ctx.bounds.x1 - width,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &trailing,
            );
        }
    }

    /// Draw the shadow first, then render the given content over it (matching the
    /// historical order). Shared by the dispatch handler and the retained
    /// `Wrapper` node. The shadow color resolves against `env`.
    pub(super) fn apply_shadow(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        shadow: &Shadow,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let blur = f64::from(shadow.radius.max(0.0));
        let offset_x = f64::from(shadow.offset.x);
        let offset_y = f64::from(shadow.offset.y);
        let shadow_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0 + offset_x,
            ctx.bounds.y0 + offset_y,
            ctx.bounds.x1 + offset_x,
            ctx.bounds.y1 + offset_y,
        );
        let shadow_color = resolved_color_to_peniko(shadow.color.resolve(env).get());

        renderer.scene.draw_blurred_rounded_rect(
            ctx.transform,
            shadow_rect,
            shadow_color,
            blur,
            blur,
        );
        render_content(renderer);
    }

    /// Render the wrapped content, then bind the single text input it registered to
    /// the `.focused(binding)` binding and reconcile focus state. Shared by the
    /// dispatch handler and the retained `Wrapper` node ([`WrapperEffect::Focused`]):
    /// the binding is read through `read_signal` so a change schedules a frame, and
    /// the target bookkeeping counts inputs registered during the content render, so
    /// it works identically whether the content is dispatched or node-flushed.
    pub(super) fn apply_focused(
        renderer: &mut HydrolysisRenderer,
        value: &Focused,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let should_focus = renderer.read_signal(&value.0);
        let start = renderer.text_editing.text_input_targets.len();
        render_content(renderer);
        let end = renderer.text_editing.text_input_targets.len();
        let focus_target_count = end - start;
        assert!(
            focus_target_count == 1,
            "hydrolysis .focused() requires exactly one TextField or SecureField in the wrapped subtree, found {focus_target_count}"
        );
        let target = renderer
            .text_editing
            .text_input_targets
            .get_mut(start)
            .expect("hydrolysis focused metadata missing registered text input target");
        assert!(
            target.focus_binding.is_none(),
            "hydrolysis does not allow multiple .focused() modifiers to target the same control"
        );
        target.focus_binding = Some(value.0.clone());

        if should_focus {
            renderer.set_focused_text_input(Some(start));
            return;
        }

        if matches!(
            renderer.text_editing.focused_text_input.get(),
            Some(index) if index >= start && index < end
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    /// Render the given content and, when hit-testing is disabled, truncate every
    /// interaction-target vector back to its pre-render length (and clear focus if
    /// the focused text input fell inside the wrapped range). Shared by the dispatch
    /// handler and the retained `Wrapper` node. The bookkeeping counts targets
    /// registered during the content render, so it works identically whether the
    /// content is dispatched or node-flushed.
    pub(super) fn apply_hittable(
        renderer: &mut HydrolysisRenderer,
        value: &Hittable,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let enabled = renderer.read_signal(&value.enabled);
        let pointer_start = renderer.hit_test.pointer_targets.len();
        let gesture_start = renderer.gesture_engine.target_count();
        let cursor_start = renderer.hit_test.cursor_targets.len();
        let hover_start = renderer.hit_test.hover_targets.len();
        let scroll_start = renderer.hit_test.scroll_targets.len();
        let text_start = renderer.text_editing.text_input_targets.len();

        render_content(renderer);

        if enabled {
            return;
        }

        renderer.hit_test.pointer_targets.truncate(pointer_start);
        renderer.ensure_active_pointer_drag_target_is_live();
        renderer.gesture_engine.truncate_targets(gesture_start);
        renderer.hit_test.cursor_targets.truncate(cursor_start);
        let removed_hover: Vec<_> = renderer.hit_test.hover_targets[hover_start..]
            .iter()
            .map(|target| (target.slot.clone(), target.handles.clone()))
            .collect();
        let now = renderer.frame_instant();
        for (slot, handles) in removed_hover {
            renderer.hit_test.interaction.set_hovering(&slot, false);
            if let Some(handles) = handles {
                handles.set_hovering(false, now);
            }
        }
        renderer.hit_test.hover_targets.truncate(hover_start);
        renderer.hit_test.scroll_targets.truncate(scroll_start);
        let text_end = renderer.text_editing.text_input_targets.len();
        renderer
            .text_editing
            .text_input_targets
            .truncate(text_start);

        if matches!(
            renderer.text_editing.focused_text_input.get(),
            Some(index) if index >= text_start && index < text_end
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    /// Register the cursor hit-target, then render the given content. Shared by
    /// the dispatch handler and the retained `Wrapper` node.
    pub(super) fn apply_cursor(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        value: &Cursor,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let style = renderer.read_signal(&value.style);
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_cursor_target(bounds, style);
        render_content(renderer);
    }

    /// Register the gesture target (and, for a tappable view with a role, its
    /// accessibility node), then render the given content under accessibility
    /// suppression when the role excludes descendants. Shared by the dispatch
    /// handler and the retained `Wrapper` node.
    ///
    /// The build-resolved state lives in [`GestureObserverEffect`] (the two pieces
    /// derived from `content` — the default a11y label and the gesture group
    /// identity — are resolved at build time, since a node has no `content` at
    /// flush). Everything else is re-resolved against `env` each call (role/label
    /// overrides, suppression), matching the dispatch path. The action is shared
    /// so the node can re-register the same action every flush.
    pub(super) fn apply_gesture_observer(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        effect: &GestureObserverEffect,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        #[cfg(feature = "accessibility")]
        if matches!(effect.gesture, Gesture::Tap(_)) && env.get::<AccessibilityRole>().is_some() {
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
            );
            if let Some(label) =
                renderer.resolve_accessibility_label(env, effect.default_a11y_label.clone())
            {
                node.set_label(label);
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            let activation_point = accessibility_activation_point(bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::PointerPrimaryClick {
                    point: activation_point,
                }),
            );
        }
        let group_id = renderer.gesture_group_id_for_identity(effect.gesture_group_identity);
        let captured_env = env.clone();
        let action = Rc::clone(&effect.action);
        let layered_action: BoxedAction<()> = Box::new(move |runtime_env: &Environment| {
            let action_env = captured_env.layered_on(runtime_env);
            action.borrow_mut()(&action_env);
        });
        renderer.register_gesture_target(bounds, group_id, effect.gesture.clone(), layered_action);

        #[cfg(feature = "accessibility")]
        if env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants)
        {
            renderer.push_accessibility_suppression();
            render_content(renderer);
            renderer.pop_accessibility_suppression();
            return;
        }
        render_content(renderer);
    }

    /// Register the hover-enter/move/exit target for `handler`, then render the
    /// given content. Shared by the dispatch handler and the retained `Wrapper`
    /// node. The handler is shared (`Rc<RefCell<OnEvent>>`) so the node can
    /// re-register the same handler every flush; the dispatch handler wraps its
    /// owned value once. The registered closure resolves the action environment
    /// against `env` exactly as before.
    pub(super) fn apply_on_event(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        handler: Rc<RefCell<OnEvent>>,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let event = handler.borrow().event();
        let interaction_key = InteractionKey::for_rc(&handler, 0);
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        match event {
            Event::HoverEnter => {
                let captured_env = env.clone();
                renderer.register_hover_enter_target(interaction_key, bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.borrow_mut().handle(&action_env);
                    true
                });
            }
            Event::HoverMove => {
                let captured_env = env.clone();
                renderer.register_hover_move_target(interaction_key, bounds, move |point, env| {
                    let hover_event = HoverEvent::new(waterui_core::layout::Point::new(
                        point.x as f32 - bounds.x0 as f32,
                        point.y as f32 - bounds.y0 as f32,
                    ));
                    let hover_env = captured_env.layered_on(&env.extending(hover_event));
                    handler.borrow_mut().handle(&hover_env);
                    true
                });
            }
            Event::HoverExit => {
                let captured_env = env.clone();
                renderer.register_hover_exit_target(interaction_key, bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.borrow_mut().handle(&action_env);
                    true
                });
            }
            _ => panic!("hydrolysis event variant is not supported"),
        }
        render_content(renderer);
    }

    /// Register the context-menu hit-target, then render the given content. Shared
    /// by the dispatch handler and the retained `Wrapper` node. The node owns the
    /// [`ResolvedContextMenu`] by reference, so the menu items are cloned for
    /// registration.
    pub(super) fn apply_context_menu(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        value: &ResolvedContextMenu,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_context_menu_target(bounds, value.items.clone());
        render_content(renderer);
    }

    /// Register the draggable hit-target, then render the given content. Shared by
    /// the dispatch handler and the retained `Wrapper` node. The node owns the
    /// [`Draggable`] by reference, so the data provider is cloned for registration.
    pub(super) fn apply_draggable(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        value: &Draggable,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_draggable_target(bounds, value.data.clone());
        render_content(renderer);
    }

    /// Register the drop-destination hit-target from pre-wrapped handler handles,
    /// then render the given content. Shared by the dispatch handler and the
    /// retained `Wrapper` node (which holds the handles by value and re-registers
    /// the same `Rc`s every flush).
    pub(super) fn apply_drop_destination(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        handles: &DropDestinationHandles,
        render_content: impl FnOnce(&mut HydrolysisRenderer),
    ) {
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_drop_destination_handles(bounds, handles, env);
        render_content(renderer);
    }
}
