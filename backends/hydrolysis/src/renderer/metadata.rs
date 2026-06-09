//! Metadata view handlers: styling, transforms, interaction, lifecycle
//! and accessibility metadata wrappers around content views.

use super::*;

impl HydrolysisRenderer {
    pub(super) fn render_environment_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Environment>,
        env: &Environment,
    ) {
        let (content, scoped_env) = flatten_environment_metadata_owned(AnyView::new(metadata), env);
        renderer.dispatch_boxed_with_render_depth(content, &scoped_env, ctx);
    }

    pub(super) fn render_retain_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Retain>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        renderer.lifecycle.current_frame_retain.push(value);
        renderer.dispatch_boxed_with_render_depth(content, env, ctx);
    }

    pub(super) fn render_opacity_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Opacity>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        // Inside a dynamic-subtree capture, an animated opacity is captured as a
        // replayable dynamic layer (re-sampled at replay) instead of baked into the
        // scene, so animation-only frames can refresh by replay without re-dispatch.
        if renderer.dynamic_transform_capture_depth > 0 && value.value.identity().is_some() {
            let alpha = renderer
                .dynamic_transform_scalar_with_discriminator(&value.value, OPACITY_ANIMATION_KEY);
            renderer.capture_dynamic_opacity(ctx, env, content, alpha);
            return;
        }
        let alpha = renderer
            .resolve_animated_scalar_with_discriminator(&value.value, OPACITY_ANIMATION_KEY);
        renderer.push_layer_rect(alpha, ctx.transform, ctx.bounds);

        let previous_opacity = renderer.hit_test.hit_test_opacity;
        renderer.hit_test.hit_test_opacity = previous_opacity * alpha;
        renderer.dispatch_boxed_with_render_depth(content, env, ctx);
        renderer.hit_test.hit_test_opacity = previous_opacity;

        renderer.pop_layer();
    }

    pub(super) fn render_scale_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Scale>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        if renderer.dynamic_transform_capture_depth > 0
            && (value.x.identity().is_some() || value.y.identity().is_some())
        {
            let scale_x = renderer
                .dynamic_transform_scalar_with_discriminator(&value.x, SCALE_X_ANIMATION_KEY);
            let scale_y = renderer
                .dynamic_transform_scalar_with_discriminator(&value.y, SCALE_Y_ANIMATION_KEY);
            renderer.capture_dynamic_transform(
                ctx,
                env,
                content,
                DynamicTransformComponents::scale(scale_x, scale_y, center),
            );
            return;
        }
        let (scale_x, scale_y) = (
            renderer.resolve_animated_scalar_with_discriminator(&value.x, SCALE_X_ANIMATION_KEY),
            renderer.resolve_animated_scalar_with_discriminator(&value.y, SCALE_Y_ANIMATION_KEY),
        );
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::scale_non_uniform(f64::from(scale_x), f64::from(scale_y))
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(renderer, ctx.child(transform, ctx.bounds), env, content);
    }

    pub(super) fn render_rotation_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Rotation>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        if renderer.dynamic_transform_capture_depth > 0 && value.angle.identity().is_some() {
            let angle = renderer
                .dynamic_transform_scalar_with_discriminator(&value.angle, ROTATION_ANIMATION_KEY);
            renderer.capture_dynamic_transform(
                ctx,
                env,
                content,
                DynamicTransformComponents::rotation(angle, center),
            );
            return;
        }
        let radians = f64::from(
            renderer
                .resolve_animated_scalar_with_discriminator(&value.angle, ROTATION_ANIMATION_KEY),
        )
        .to_radians();
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::rotate(radians)
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(renderer, ctx.child(transform, ctx.bounds), env, content);
    }

    pub(super) fn render_offset_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Offset>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        if renderer.dynamic_transform_capture_depth > 0
            && (value.x.identity().is_some() || value.y.identity().is_some())
        {
            let offset_x = renderer
                .dynamic_transform_scalar_with_discriminator(&value.x, OFFSET_X_ANIMATION_KEY);
            let offset_y = renderer
                .dynamic_transform_scalar_with_discriminator(&value.y, OFFSET_Y_ANIMATION_KEY);
            renderer.capture_dynamic_transform(
                ctx,
                env,
                content,
                DynamicTransformComponents::offset(offset_x, offset_y),
            );
            return;
        }
        let (offset_x, offset_y) = (
            renderer.resolve_animated_scalar_with_discriminator(&value.x, OFFSET_X_ANIMATION_KEY),
            renderer.resolve_animated_scalar_with_discriminator(&value.y, OFFSET_Y_ANIMATION_KEY),
        );
        let transform = vello::kurbo::Affine::translate((f64::from(offset_x), f64::from(offset_y)));
        Self::dispatch_any(renderer, ctx.child(transform, ctx.bounds), env, content);
    }

    pub(super) fn render_clip_shape_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<ClipShape>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let clip_path = path_commands_to_path(value.commands(), ctx.bounds);
        renderer.push_layer_path(1.0, ctx.transform, clip_path);
        Self::dispatch_any(renderer, ctx, env, content);
        renderer.pop_layer();
    }

    pub(super) fn render_border_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Border>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let border = value;
        Self::dispatch_any(renderer, ctx, env, content);

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

    pub(super) fn render_shadow_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Shadow>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let shadow = value;
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
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_focused_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Focused>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let should_focus = renderer.read_signal(&value.0);
        let start = renderer.text_editing.text_input_targets.len();
        Self::dispatch_any(renderer, ctx, env, content);
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

    pub(super) fn render_hittable_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Hittable>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let enabled = renderer.read_signal(&value.enabled);
        let pointer_start = renderer.hit_test.pointer_targets.len();
        let gesture_start = renderer.gesture_engine.target_count();
        let cursor_start = renderer.hit_test.cursor_targets.len();
        let hover_start = renderer.hit_test.hover_targets.len();
        let hover_cursor_start = renderer.hit_test.interaction.hover_cursor();
        let scroll_start = renderer.hit_test.scroll_targets.len();
        let text_start = renderer.text_editing.text_input_targets.len();

        Self::dispatch_any(renderer, ctx, env, content);

        if enabled {
            return;
        }

        renderer.hit_test.pointer_targets.truncate(pointer_start);
        renderer.ensure_active_pointer_drag_target_is_live();
        renderer.gesture_engine.truncate_targets(gesture_start);
        renderer.hit_test.cursor_targets.truncate(cursor_start);
        renderer.hit_test.hover_targets.truncate(hover_start);
        renderer
            .hit_test
            .interaction
            .rewind_hover_to(hover_cursor_start);
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

    pub(super) fn render_cursor_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Cursor>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let style = renderer.read_signal(&value.style);
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_cursor_target(bounds, style);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_gesture_observer_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<GestureObserver>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let GestureObserver {
            gesture,
            mut action,
            ..
        } = value;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        #[cfg(feature = "accessibility")]
        if matches!(gesture, Gesture::Tap(_)) && env.get::<AccessibilityRole>().is_some() {
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
            );
            let default_label = renderer.accessibility_label_from_view(&content, env);
            if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
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
        let gesture_group_identity = gesture_group_identity(&content);
        let group_id = renderer.gesture_group_id_for_identity(gesture_group_identity);
        let captured_env = env.clone();
        let layered_action: BoxedAction<()> = Box::new(move |runtime_env: &Environment| {
            let action_env = captured_env.layered_on(runtime_env);
            action(&action_env);
        });
        renderer.register_gesture_target(bounds, group_id, gesture, layered_action);

        #[cfg(feature = "accessibility")]
        if env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants)
        {
            renderer.push_accessibility_suppression();
            Self::dispatch_any(renderer, ctx, env, content);
            renderer.pop_accessibility_suppression();
            return;
        }
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_on_event_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<OnEvent>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let event = value.event();
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        match event {
            Event::HoverEnter => {
                let mut handler = value;
                let captured_env = env.clone();
                renderer.register_hover_enter_target(bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.handle(&action_env);
                    true
                });
            }
            Event::HoverMove => {
                let mut handler = value;
                let captured_env = env.clone();
                renderer.register_hover_move_target(bounds, move |point, env| {
                    let hover_event = HoverEvent::new(waterui_core::layout::Point::new(
                        point.x as f32 - bounds.x0 as f32,
                        point.y as f32 - bounds.y0 as f32,
                    ));
                    let hover_env = captured_env.layered_on(&env.extending(hover_event));
                    handler.handle(&hover_env);
                    true
                });
            }
            Event::HoverExit => {
                let mut handler = value;
                let captured_env = env.clone();
                renderer.register_hover_exit_target(bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.handle(&action_env);
                    true
                });
            }
            _ => panic!("hydrolysis event variant is not supported"),
        }
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_context_menu_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<ResolvedContextMenu>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_context_menu_target(bounds, value.items);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_draggable_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Draggable>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_draggable_target(bounds, value.data);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_drop_destination_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<DropDestination>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_drop_destination_target(bounds, value, env);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_passthrough_metadata<T: MetadataKey>(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<T>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_passthrough_ignorable_metadata<T: MetadataKey>(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<T>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(super) fn render_accessibility_label_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityLabel>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    pub(super) fn render_accessibility_role_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityRole>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    pub(super) fn render_accessibility_hidden_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityHidden>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    pub(super) fn render_accessibility_children_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityChildren>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    pub(super) fn render_accessibility_state_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityState>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        if value.is_hidden() {
            local_env.insert(AccessibilityHidden::new(true));
        }
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    pub(super) fn render_accessibility_state_signal_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityStateSignal>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let state = value.state().get();
        let mut local_env = env.clone();
        if state.is_hidden() {
            local_env.insert(AccessibilityHidden::new(true));
        }
        local_env.insert(state);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }
}
