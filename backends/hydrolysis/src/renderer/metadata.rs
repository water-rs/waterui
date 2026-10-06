//! Metadata view handlers: styling, transforms, interaction, lifecycle
//! and accessibility metadata wrappers around content views.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use waterui_graphics::draw::Draw as _;

impl HydrolysisRenderer {
    /// Apply a clip-shape layer around the given content render. Shared by the
    /// dispatch handler and the retained `Wrapper` node so the clip effect lives
    /// in exactly one place.
    pub(super) fn apply_clip_shape(
        renderer: &mut Self,
        ctx: RenderContext,
        value: &ClipShape,
        render_content: impl FnOnce(&mut Self),
    ) {
        // Resolve from the structured kind, exactly as a fill of the same shape
        // does, and fall back to the unit-space commands only for a custom path.
        // The commands are normalized per axis, so resolving them against a
        // non-square rect makes every circular corner elliptical.
        let clip_path = shape_kind_path(value.kind(), ctx.bounds)
            .unwrap_or_else(|| path_commands_to_path(value.commands(), ctx.bounds));
        if let Some(regular_clip) = kind_clip_shape(value.kind(), ctx.bounds)
            .or_else(|| regular_clip_shape(value.commands(), ctx.bounds))
        {
            match regular_clip {
                RegularClipShape::Rect(rect) => {
                    renderer.with_clip_rect_scope(
                        1.0,
                        ctx.local,
                        rect,
                        crate::renderer::ScopeDelta::RECORD_SPACE,
                        render_content,
                    );
                }
                RegularClipShape::RoundedRect {
                    rect,
                    corner_width,
                    corner_height,
                } => renderer.with_clip_rounded_rect_scope(
                    1.0,
                    ctx.local,
                    clip_path,
                    rect,
                    corner_width,
                    corner_height,
                    crate::renderer::ScopeDelta::RECORD_SPACE,
                    render_content,
                ),
            }
        } else {
            renderer.with_clip_path_scope(
                1.0,
                ctx.local,
                clip_path,
                crate::renderer::ScopeDelta::RECORD_SPACE,
                render_content,
            );
        }
    }

    /// Render the given content then stroke the border over it, mirroring the
    /// historical order (content first, border on top). Shared by the dispatch
    /// handler and the retained `Wrapper` node. The border color resolves against
    /// `env`, so it is threaded through.
    pub(super) fn apply_border(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        border: &Border,
        render_content: impl FnOnce(&mut Self),
    ) {
        render_content(renderer);

        if border.width <= 0.0 {
            return;
        }

        let paint = || Paint::Solid(border.color.resolve(env).snapshot());
        let width = f64::from(border.width);

        if border.edges.all() && border.corner_radius > 0.0 {
            let rounded =
                kurbo::RoundedRect::from_rect(ctx.bounds, f64::from(border.corner_radius));
            let stroke = kurbo::Stroke::new(width);
            renderer
                .scene
                .stroke_paint(&stroke, ctx.local, paint(), &rounded);
            return;
        }

        if border.edges.top {
            let top = kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y0 + width,
            );
            renderer
                .scene
                .fill_paint(peniko::Fill::NonZero, ctx.local, paint(), &top);
        }
        if border.edges.bottom {
            let bottom = kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y1 - width,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            renderer
                .scene
                .fill_paint(peniko::Fill::NonZero, ctx.local, paint(), &bottom);
        }
        if border.edges.leading {
            let leading = kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x0 + width,
                ctx.bounds.y1,
            );
            renderer
                .scene
                .fill_paint(peniko::Fill::NonZero, ctx.local, paint(), &leading);
        }
        if border.edges.trailing {
            let trailing = kurbo::Rect::new(
                ctx.bounds.x1 - width,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            renderer
                .scene
                .fill_paint(peniko::Fill::NonZero, ctx.local, paint(), &trailing);
        }
    }

    /// Draw the shadow first, then render the given content over it (matching the
    /// historical order). Shared by the dispatch handler and the retained
    /// `Wrapper` node. The shadow color resolves against `env`.
    pub(super) fn apply_shadow(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        shadow: &Shadow,
        render_content: impl FnOnce(&mut Self),
    ) {
        let blur = f64::from(shadow.radius.max(0.0));
        let offset_x = f64::from(shadow.offset.x);
        let offset_y = f64::from(shadow.offset.y);
        let shadow_rect = kurbo::Rect::new(
            ctx.bounds.x0 + offset_x,
            ctx.bounds.y0 + offset_y,
            ctx.bounds.x1 + offset_x,
            ctx.bounds.y1 + offset_y,
        );
        let shadow_color = shadow.color.resolve(env).snapshot();

        // The silhouette states the caster's shape. `kind_clip_shape` — the
        // same resolver a clip uses to decide between the uniform rounded-rect
        // fast path and the general path route — answers whether the shadow can
        // feed the engine's blurred-rounded-rect primitive directly.
        let silhouette = &shadow.silhouette;
        let uniform_radius = match kind_clip_shape(silhouette.kind(), shadow_rect) {
            Some(RegularClipShape::RoundedRect { corner_width, .. }) => Some(corner_width),
            Some(RegularClipShape::Rect(_)) => Some(0.0),
            None => None,
        };
        match uniform_radius {
            Some(corner_radius) => renderer.scene.blurred_rounded_rect(
                ctx.local,
                shadow_rect,
                shadow_color,
                corner_radius,
                blur,
            ),
            None => Self::draw_blurred_silhouette(
                renderer,
                ctx.local,
                silhouette,
                shadow_rect,
                shadow_color,
                blur,
            ),
        }
        render_content(renderer);
    }

    /// Draw a non-rounded-rect silhouette's blurred shadow into `rect` — the
    /// general route for silhouettes `kind_clip_shape` cannot express as a
    /// uniform rounded rect (ellipse, non-square circle, uneven corners,
    /// custom path). The engine rasterizes and caches the blur; Hydrolysis
    /// keeps no pixmap cache of its own.
    fn draw_blurred_silhouette(
        renderer: &mut Self,
        transform: kurbo::Affine,
        silhouette: &ClipShape,
        rect: kurbo::Rect,
        color: WorkingColor,
        blur: f64,
    ) {
        // The same resolution `apply_clip_shape` performs — structured kind
        // first, unit-space commands only for a custom path — but against the
        // rect normalized to the origin: the rect's own position is applied
        // at draw time.
        let local_rect = kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height());
        let local_path = shape_kind_path(silhouette.kind(), local_rect)
            .unwrap_or_else(|| path_commands_to_path(silhouette.commands(), local_rect));
        let placement = transform * kurbo::Affine::translate((rect.x0, rect.y0));

        if blur <= 0.0 {
            renderer.scene.fill_paint(
                peniko::Fill::NonZero,
                placement,
                Paint::Solid(color),
                &local_path,
            );
            return;
        }

        // `blur` is already in device pixels; the engine scales `sigma` by
        // the transform's axis length, so the path arrives pre-transformed
        // and the op transform is identity.
        renderer.scene.shadow(
            kurbo::Affine::IDENTITY,
            &(placement * &local_path),
            blur,
            color,
        );
    }

    /// Draw the theme's context-menu panel behind the wrapped menu rows,
    /// then render the content over it. This is the popup-window menu's
    /// surface ([`WrapperEffect::PopupMenuSurface`]): on Material 3 themes it
    /// is `md.comp.menu.container.color` (`surface-container`), the extra-small
    /// 4 dp `md.sys.shape.corner.extra-small` shape and
    /// `md.comp.menu.container.elevation` level 2 — drawn by the theme's
    /// `draw_text_context_menu_panel`. The window leaves the panel's shadow
    /// room inside its own bounds via `POPUP_MENU_PANEL_MARGIN`.
    pub(super) fn apply_popup_menu_surface(
        renderer: &mut Self,
        ctx: RenderContext,
        render_content: impl FnOnce(&mut Self),
    ) {
        {
            let theme = renderer.theme();
            renderer.scene.record_picture(ctx.local, |draw| {
                theme.draw_text_context_menu_panel(&mut *draw, ctx.bounds);
            });
        }
        render_content(renderer);
    }

    /// Render the wrapped content, then bind the single focusable target — a
    /// text input or an input surface — it registered to the
    /// `.focused(binding)` binding and reconcile focus state. Shared by the
    /// dispatch handler and the retained `Wrapper` node ([`WrapperEffect::Focused`]):
    /// the binding is read through `read_signal` so a change schedules a frame, and
    /// the target bookkeeping counts targets registered during the content render, so
    /// it works identically whether the content is dispatched or node-flushed.
    pub(super) fn apply_focused(
        renderer: &mut Self,
        value: &Focused,
        render_content: impl FnOnce(&mut Self),
    ) {
        let should_focus = renderer.read_signal(&value.0);
        let scope = renderer
            .reader_cell()
            .expect("hydrolysis .focused() applied outside a record");
        render_content(renderer);
        renderer.wire_focused_target(value, should_focus, &scope);
    }

    /// The semantic counterpart of [`Self::apply_focused`]: the same focus
    /// wiring over the text-input and surface targets the semantic walk
    /// registered, with no renderer in hand.
    #[cfg(feature = "accessibility")]
    pub(super) fn apply_focused_semantic(
        renderer: &mut SemanticCore,
        value: &Focused,
        render_content: impl FnOnce(&mut SemanticCore),
    ) {
        let should_focus = renderer.read_signal(&value.0);
        let scope = renderer
            .reader_cell()
            .expect("hydrolysis .focused() applied outside a record");
        render_content(renderer);
        renderer.wire_focused_target(value, should_focus, &scope);
    }

    /// Render the given content and, when hit-testing is disabled, truncate every
    /// interaction-target vector back to its pre-render length (and clear focus if
    /// the focused text input fell inside the wrapped range). Shared by the dispatch
    /// handler and the retained `Wrapper` node. The bookkeeping counts targets
    /// registered during the content render, so it works identically whether the
    /// content is dispatched or node-flushed.
    /// `.hittable(false)`: the subtree's retained registrations sit under a
    /// [`HitGate::Unhittable`](crate::renderer::HitGate::Unhittable) scope,
    /// which removes exactly the input kinds dev's per-frame truncation
    /// took (drop and context-menu targets, back targets, modal scopes and
    /// native-view occlusions stay). Materialization clears the hover of a
    /// target the gate removed, as the truncation did; focus needs no
    /// eager pass — `finish_rebuild_frame`'s absent-target sweep retires a
    /// focused target the materialized list no longer emits.
    pub(super) fn apply_hittable(
        renderer: &mut Self,
        value: &Hittable,
        render_content: impl FnOnce(&mut Self),
    ) {
        let enabled = renderer.read_signal(&value.enabled);
        if enabled {
            render_content(renderer);
            return;
        }
        // Regions flushed inside carry the unhittable scope on their
        // chain, so materialization drops their input kinds; a stale
        // in-flight drag against the content clears in the materialization
        // signature check.
        renderer.push_hit_gate_scope(crate::renderer::HitGate::Unhittable);
        render_content(renderer);
        renderer.pop_placement_scope();
    }

    /// Register the cursor hit-target, then render the given content. Shared by
    /// the dispatch handler and the retained `Wrapper` node.
    pub(super) fn apply_cursor(
        renderer: &mut Self,
        ctx: RenderContext,
        value: &Cursor,
        render_content: impl FnOnce(&mut Self),
    ) {
        let style = renderer.read_signal(&value.style);
        let bounds = ctx.bounds;
        renderer.register_cursor_target(bounds, style);
        render_content(renderer);
    }

    /// The `Click` → `Activate` wiring of a tap gesture's accessibility node:
    /// invoke the gesture's own action with the same layered environment the
    /// pointer path uses (`captured_env.layered_on(runtime_env)`).
    #[cfg(feature = "accessibility")]
    fn tap_accessibility_activation(
        env: &Environment,
        action: &Rc<RefCell<BoxedAction<()>>>,
    ) -> AccessibilityActivation {
        let captured_env = env.clone();
        let action = Rc::clone(action);
        Rc::new(RefCell::new(
            move |_renderer: &mut crate::renderer::SemanticCore, runtime_env: &Environment| {
                let action_env = captured_env.layered_on(runtime_env);
                action.borrow_mut()(&action_env);
                true
            },
        ))
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
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(super) fn apply_gesture_observer(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        effect: &GestureObserverEffect,
        render_content: impl FnOnce(&mut Self, &Environment),
    ) {
        // The action environment contract (water-rs/hydrolysis#177,
        // water-rs/waterui#1292): the handler resolves against `env` as seen
        // here — the environment the observer's *content* resolves in — layered
        // over the runtime env at dispatch. The caller already resolved the
        // content's leading `.state(&v)`/handler layers, so a `.state` install
        // reaches the handler whether it sits before or after `.gesture` in the
        // modifier chain. Every dispatch arm — a11y Activate, `layered_action`,
        // the press slot, keyboard activation, hover — applies this same
        // `captured_env.layered_on(runtime_env)` rule.
        let bounds = ctx.bounds;
        let disabled = env
            .get::<waterui_core::interaction::Disabled>()
            .is_some_and(|disabled| renderer.read_signal(disabled.signal()));
        #[cfg(feature = "accessibility")]
        let mut claimed_naming_node = None;
        #[cfg(feature = "accessibility")]
        if matches!(effect.gesture, Gesture::Tap(_)) {
            if env.get::<AccessibilityRole>().is_some()
                && !renderer.accessibility_scope_is_claimed(env)
            {
                let mut node = AccessibilityNode::new(
                    crate::renderer::SemanticCore::resolve_accessibility_role(
                        env,
                        AccessibilityNodeRole::Button,
                    ),
                );
                if let Some(label) =
                    renderer.resolve_accessibility_label(env, effect.default_a11y_label.clone())
                {
                    node.set_label(label);
                }
                if let Some(value) = renderer.resolve_accessibility_value(env, None) {
                    node.set_value(value);
                }
                node.add_action(AccessibilityAction::Focus);
                if renderer.control_selected(env, &InteractionKey::for_rc(&effect.action, 0)) {
                    node.set_selected(true);
                }
                let action_target = if disabled {
                    node.set_disabled();
                    None
                } else {
                    node.add_action(AccessibilityAction::Click);
                    // Direct semantic activation: invoke the gesture's own
                    // action with the same layered environment the pointer
                    // path uses.
                    Some(AccessibilityActionTarget::Activate {
                        action: Self::tap_accessibility_activation(env, &effect.action),
                    })
                };
                claimed_naming_node =
                    renderer.register_accessibility_node(node, bounds, env, action_target);
            } else if !disabled && let Some(scope) = env.get::<ScopedAccessibilitySemantics>() {
                // The scope names this view's representative — registering a
                // second node would emit a silenced duplicate — so the tap
                // delegates its activation to the scope instead, which the
                // representative drains when its subtree has been walked. An
                // unclaimed scope (a `List` row's, whose node the row
                // registers itself) receives the same donation. The donation
                // carries the gesture's own hit region and clip — the
                // interaction owner whose geometry the activation-point
                // query projects (water-rs/waterui#1323 §5).
                scope.delegate_activation(
                    Self::tap_accessibility_activation(env, &effect.action),
                    Some(NodePlacement {
                        region: crate::renderer::mount::Region {
                            local: bounds,
                            placement: renderer.current_placement(),
                        },
                    }),
                );
            }
        }
        // A gesture node that claimed the naming scope represents the wrapped
        // view: its content walks under the same shielded environment a naming
        // container hands its children, so a leaf cannot repeat the claim's
        // role and label as a second node.
        #[cfg(feature = "accessibility")]
        let content_env = if claimed_naming_node.is_some() {
            accessibility_container_child_environment(env).unwrap_or_else(|| env.clone())
        } else {
            env.clone()
        };
        #[cfg(feature = "accessibility")]
        let content_env = &content_env;
        #[cfg(not(feature = "accessibility"))]
        let content_env = env;
        #[cfg(feature = "accessibility")]
        let scope_claimed = claimed_naming_node.is_some();
        #[cfg(not(feature = "accessibility"))]
        let scope_claimed = false;
        let group_id = renderer.gesture_group_id_for_identity(effect.gesture_group_identity);
        let captured_env = env.clone();
        let action = Rc::clone(&effect.action);
        let mut layered_action: BoxedAction<()> = Box::new(move |runtime_env: &Environment| {
            let action_env = captured_env.layered_on(runtime_env);
            action.borrow_mut()(&action_env);
        });

        if matches!(effect.gesture, Gesture::Tap(_))
            && let Some(style) = env
                .get::<waterui_backend_core::widget::InteractionStyle>()
                .cloned()
        {
            let interaction_key = InteractionKey::for_rc(&effect.action, 0);
            let (interaction, press_slot, _) = renderer.bind_control_interaction_target(
                interaction_key.clone(),
                bounds,
                env,
                disabled,
            );
            Self::render_gesture_content(renderer, env, content_env, scope_claimed, render_content);
            #[cfg(feature = "accessibility")]
            if let Some(node_id) = claimed_naming_node {
                // The node advertises `Focus`; without this link Tab can land
                // on it semantically while the interaction machinery never
                // sees the key — FOCUSED would never reach
                // `.interaction_state` reports or the focus ring.
                renderer.register_accessibility_focus_link(&interaction_key, node_id);
                renderer.drain_claim_scope(node_id, env);
            }

            let state = renderer.reported_interaction_state(&interaction_key);
            let color_signal = style.state_layer_color.resolve(env);
            let color = renderer.read_signal(&color_signal);
            let interaction =
                local_interaction_state(interaction, renderer.current_hit_transform());
            {
                let theme = renderer.theme();
                let layer_bounds = style.state_layer_bounds(ctx.bounds);
                let radii = *style.state_layer_radii.resolve(state);
                let ring =
                    interaction_focus_ring(renderer, env, layer_bounds, radii, &style, state);
                renderer.draw_context(ctx, |draw| {
                    theme.draw_interaction_state_layer(
                        draw,
                        layer_bounds,
                        radii,
                        color,
                        interaction,
                    );
                    if let Some((ring_bounds, ring_radii, color, width)) = ring {
                        draw.stroke(
                            kurbo::RoundedRect::from_rect(ring_bounds, ring_radii),
                            kurbo::Stroke::new(width),
                            color,
                        );
                    }
                });
            }

            if !disabled {
                renderer.register_interactive_pointer_target_with_keyboard(
                    bounds,
                    press_slot,
                    style.keyboard_focusable,
                    move |_renderer, _point, runtime_env| {
                        layered_action(runtime_env);
                        false
                    },
                );
            }
            return;
        }

        if let Some(target) = effect.gesture_target.take() {
            renderer.register_retained_gesture_target(&target, bounds, group_id);
            effect.gesture_target.set(Some(target));
        } else {
            effect
                .gesture_target
                .set(Some(renderer.register_gesture_target(
                    bounds,
                    group_id,
                    effect.gesture.clone(),
                    layered_action,
                )));
        }
        Self::render_gesture_content(renderer, env, content_env, scope_claimed, render_content);
        #[cfg(feature = "accessibility")]
        if let Some(node_id) = claimed_naming_node {
            renderer.drain_claim_scope(node_id, env);
        }
    }

    fn render_gesture_content(
        renderer: &mut Self,
        env: &Environment,
        content_env: &Environment,
        scope_claimed: bool,
        render_content: impl FnOnce(&mut Self, &Environment),
    ) {
        #[cfg(not(feature = "accessibility"))]
        let _ = (env, content_env, scope_claimed);
        // `ExcludeDescendants` belongs to the element that claims this naming
        // scope — the claim registers its node above and suppresses its own
        // descendants here. An observer that registers no node (a long-press,
        // or a silenced tap) must not consume the flag: doing so suppresses
        // the inner element the flag actually names (water-rs/hydrolysis#266).
        #[cfg(feature = "accessibility")]
        if scope_claimed
            && env
                .get::<AccessibilityChildren>()
                .is_some_and(AccessibilityChildren::excludes_descendants)
        {
            renderer.push_accessibility_suppression();
            render_content(renderer, content_env);
            renderer.pop_accessibility_suppression();
            return;
        }
        render_content(renderer, content_env);
    }

    /// The semantic counterpart of [`Self::apply_gesture_observer`]: the tap's
    /// own accessibility node — role, label, `Click` → [`AccessibilityActionTarget::Activate`],
    /// or `disabled` — with no bounds, no pointer or gesture targets, and no
    /// interaction state layer. Returns the node when the gesture claimed the
    /// naming scope; the caller then walks the content under the shielded
    /// environment and drains the claim's scope onto it.
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_gesture_observer_accessibility(
        renderer: &mut SemanticCore,
        env: &Environment,
        effect: &GestureObserverEffect,
    ) -> Option<AccessibilityNodeId> {
        let disabled = env
            .get::<waterui_core::interaction::Disabled>()
            .is_some_and(|disabled| renderer.read_signal(disabled.signal()));
        if !matches!(effect.gesture, Gesture::Tap(_)) {
            return None;
        }
        if env.get::<AccessibilityRole>().is_some() && !renderer.accessibility_scope_is_claimed(env)
        {
            let mut node =
                AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                    env,
                    AccessibilityNodeRole::Button,
                ));
            if let Some(label) =
                renderer.resolve_accessibility_label(env, effect.default_a11y_label.clone())
            {
                node.set_label(label);
            }
            node.add_action(AccessibilityAction::Focus);
            if renderer.control_selected(env, &InteractionKey::for_rc(&effect.action, 0)) {
                node.set_selected(true);
            }
            let action_target = if disabled {
                node.set_disabled();
                None
            } else {
                node.add_action(AccessibilityAction::Click);
                Some(AccessibilityActionTarget::Activate {
                    action: Self::tap_accessibility_activation(env, &effect.action),
                })
            };
            return renderer.register_accessibility_node_semantic(node, env, action_target);
        }
        if !disabled && let Some(scope) = env.get::<ScopedAccessibilitySemantics>() {
            // Claimed or not, the scope's representative drains the donation
            // when its subtree ends — the same contract the rendered arm of
            // `apply_gesture_observer` holds. The semantic walk has no
            // geometry, so the donation carries no interaction region either.
            scope.delegate_activation(
                Self::tap_accessibility_activation(env, &effect.action),
                None,
            );
        }
        None
    }

    /// Register the hover-enter/move/exit target for `handler`, then render the
    /// given content. Shared by the dispatch handler and the retained `Wrapper`
    /// node. The handler is shared (`Rc<RefCell<OnEvent>>`) so the node can
    /// re-register the same handler every flush; the dispatch handler wraps its
    /// owned value once. The registered closure resolves the action environment
    /// against `env` exactly as before.
    pub(super) fn apply_on_event(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        handler: Rc<RefCell<OnEvent>>,
        render_content: impl FnOnce(&mut Self),
    ) {
        let event = handler.borrow().event();
        let interaction_key = InteractionKey::for_rc(&handler, 0);
        let bounds = ctx.bounds;
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
                        crate::num_cast::f64_as_f32(point.x)
                            - crate::num_cast::f64_as_f32(bounds.x0),
                        crate::num_cast::f64_as_f32(point.y)
                            - crate::num_cast::f64_as_f32(bounds.y0),
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
    /// [`ContextMenuEffect`] by reference, so the menu items are cloned for
    /// registration and the preview/accessory slots travel with the target for
    /// the open presentation to mount. The node's environment travels with the
    /// target so the popup opens inside it (water-rs/hydrolysis#140).
    pub(super) fn apply_context_menu(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        value: &ContextMenuEffect,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = ctx.bounds;
        renderer.register_context_menu_target(
            bounds,
            value.items.clone(),
            env,
            value.dismiss_requests.clone(),
            Rc::clone(&value.preview),
            Rc::clone(&value.accessory),
        );
        render_content(renderer);
    }

    /// Register the anchor's live bounds and the overlay's handles for the
    /// post-flush render pass, then render the anchor content. Shared by the
    /// dispatch handler and the retained `Wrapper` node. The binding is read
    /// through [`HydrolysisRenderer::read_signal`], so a value change
    /// schedules the refresh that opens or closes the overlay; re-registering
    /// every frame is also what lets the render pass follow an anchor that
    /// moved or detect one that left the tree.
    pub(super) fn apply_anchored_overlay(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        value: &AnchoredOverlayEffect,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = renderer.resolve_window_rect(ctx.bounds);
        let presented = renderer.read_signal(&value.is_presented);
        renderer
            .popup_menu
            .anchored_overlays
            .push(RegisteredAnchoredOverlay {
                anchor: bounds,
                placement: value.placement,
                dismissal: value.dismissal,
                presented,
                is_presented: value.is_presented.clone(),
                placed_edge: value.placed_edge.clone(),
                env: env.clone(),
                content: Rc::clone(&value.content),
                marker: Rc::clone(&value.marker),
            });
        render_content(renderer);
    }

    /// Register the draggable hit-target, then render the given content. Shared by
    /// the dispatch handler and the retained `Wrapper` node. The node owns the
    /// [`Draggable`] in an `Rc`, so the registration clones the handle and the
    /// payload reads live at the moment the drag begins.
    pub(super) fn apply_draggable(
        renderer: &mut Self,
        ctx: RenderContext,
        value: &Rc<Draggable>,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = ctx.bounds;
        renderer.register_draggable_target(bounds, Rc::clone(value));
        render_content(renderer);
    }

    /// Register the drop-destination hit-target from pre-wrapped handler handles,
    /// then render the given content. Shared by the dispatch handler and the
    /// retained `Wrapper` node (which holds the handles by value and re-registers
    /// the same `Rc`s every flush).
    pub(super) fn apply_drop_destination(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        handles: &DropDestinationHandles,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = ctx.bounds;
        renderer.register_drop_destination_handles(bounds, handles, env);
        render_content(renderer);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RegularClipShape {
    Rect(kurbo::Rect),
    RoundedRect {
        rect: kurbo::Rect,
        corner_width: f64,
        corner_height: f64,
    },
}

/// The fast rounded-rect/rect clip for a structured shape kind.
///
/// A normalized radius resolves against the shorter side, so corners stay
/// circular and a fully-rounded shape is a stadium rather than an ellipse.
#[expect(
    clippy::float_cmp,
    reason = "the comparison is exact by design — the value originates from a literal fixture, not accumulated arithmetic"
)]
fn kind_clip_shape(kind: ShapeKind, bounds: kurbo::Rect) -> Option<RegularClipShape> {
    let min_side = bounds.width().min(bounds.height()).max(0.0);
    let rounded = |corner: f64| {
        Some(RegularClipShape::RoundedRect {
            rect: bounds,
            corner_width: corner,
            corner_height: corner,
        })
    };
    let uniform = |radius: f32| rounded(f64::from(radius.clamp(0.0, 0.5)) * min_side);
    // A fixed radius is already a length in points; only the
    // half-shorter-side ceiling applies.
    let fixed = |radius: f32| rounded(f64::from(radius.max(0.0)).min(min_side / 2.0));
    match kind {
        ShapeKind::Rect => Some(RegularClipShape::Rect(bounds)),
        ShapeKind::RoundedRect { corner_radius } => uniform(corner_radius),
        ShapeKind::FixedRoundedRect { corner_radius } => fixed(corner_radius),
        ShapeKind::Capsule => uniform(0.5),
        // A circle is *inscribed* in the bounds, so only a square one is a
        // rounded rect: elsewhere `uniform(0.5)` describes a stadium filling
        // the bounds, which is what a capsule is and what a circle is not. The
        // fill path builds a real `kurbo::Circle`, and a clip that disagreed
        // with its own fill is the bug this guard closes.
        ShapeKind::Circle if bounds.width() == bounds.height() => uniform(0.5),
        // An ellipse is not a rounded rect, a non-square circle is not either,
        // and uneven corners need the path mask; all stay on the general route.
        ShapeKind::Circle
        | ShapeKind::Ellipse
        | ShapeKind::UnevenRoundedRect { .. }
        | ShapeKind::FixedUnevenRoundedRect { .. }
        | ShapeKind::CustomPath => None,
    }
}

#[cfg(test)]
mod clip_shape_tests {
    use super::{RegularClipShape, ShapeKind, kind_clip_shape};
    use kurbo::Rect;

    /// A square circle is exactly a rounded rect whose corner is half the
    /// side, so the fast clip is allowed to take it.
    #[test]
    fn a_square_circle_takes_the_rounded_rect_fast_path() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let clip = kind_clip_shape(ShapeKind::Circle, bounds);
        assert!(
            matches!(
                clip,
                Some(RegularClipShape::RoundedRect {
                    corner_width,
                    corner_height,
                    ..
                }) if (corner_width - 50.0).abs() < f64::EPSILON
                    && (corner_height - 50.0).abs() < f64::EPSILON
            ),
            "a square circle should clip as a rounded rect with a half-side corner, got {clip:?}"
        );
    }

    /// On a wider-than-tall rect the same shortcut would describe a stadium,
    /// which is a capsule and not the inscribed circle the fill draws.
    #[test]
    fn a_non_square_circle_does_not_take_the_fast_path() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(
            kind_clip_shape(ShapeKind::Circle, bounds).is_none(),
            "a non-square circle must fall through to the path mask so the clip \
             matches the inscribed circle the fill builds"
        );
    }

    /// A capsule *is* the stadium, on any aspect ratio.
    #[test]
    fn a_capsule_takes_the_fast_path_at_any_aspect_ratio() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(matches!(
            kind_clip_shape(ShapeKind::Capsule, bounds),
            Some(RegularClipShape::RoundedRect { .. })
        ));
    }

    /// A fixed radius is a length in points: 12 stays 12 on a wide bar, and
    /// only the half-shorter-side ceiling cuts it down.
    #[test]
    fn a_fixed_radius_clips_at_its_own_length_up_to_the_ceiling() {
        let wide = Rect::new(0.0, 0.0, 200.0, 50.0);
        assert!(matches!(
            kind_clip_shape(
                ShapeKind::FixedRoundedRect {
                    corner_radius: 12.0
                },
                wide
            ),
            Some(RegularClipShape::RoundedRect {
                corner_width,
                corner_height,
                ..
            }) if (corner_width - 12.0).abs() < f64::EPSILON
                && (corner_height - 12.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            kind_clip_shape(
                ShapeKind::FixedRoundedRect {
                    corner_radius: 40.0
                },
                wide
            ),
            Some(RegularClipShape::RoundedRect {
                corner_width,
                corner_height,
                ..
            }) if (corner_width - 25.0).abs() < f64::EPSILON
                && (corner_height - 25.0).abs() < f64::EPSILON
        ));
    }

    /// Per-corner radii cannot be a uniform `RoundedRect` clip.
    #[test]
    fn a_fixed_uneven_kind_stays_on_the_path_mask() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(
            kind_clip_shape(
                ShapeKind::FixedUnevenRoundedRect {
                    top_left: 0.0,
                    top_right: 16.0,
                    bottom_left: 0.0,
                    bottom_right: 16.0,
                },
                bounds
            )
            .is_none()
        );
    }
}

fn regular_clip_shape(commands: &[PathCommand], bounds: kurbo::Rect) -> Option<RegularClipShape> {
    regular_rect(commands, bounds).or_else(|| regular_rounded_rect(commands, bounds))
}

fn regular_rect(commands: &[PathCommand], bounds: kurbo::Rect) -> Option<RegularClipShape> {
    let [
        PathCommand::MoveTo { x: x0, y: y0 },
        PathCommand::LineTo { x: x1, y: top_y },
        PathCommand::LineTo { x: right_x, y: y1 },
        PathCommand::LineTo {
            x: left_x,
            y: bottom_y,
        },
        PathCommand::Close,
    ] = commands
    else {
        return None;
    };
    if !approx_eq(*y0, *top_y)
        || !approx_eq(*x1, *right_x)
        || !approx_eq(*y1, *bottom_y)
        || !approx_eq(*x0, *left_x)
        || !valid_rect(*x0, *y0, *x1, *y1)
    {
        return None;
    }
    Some(RegularClipShape::Rect(resolve_normalized_rect(
        *x0, *y0, *x1, *y1, bounds,
    )))
}

#[allow(clippy::too_many_lines)]
#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
fn regular_rounded_rect(commands: &[PathCommand], bounds: kurbo::Rect) -> Option<RegularClipShape> {
    let [
        PathCommand::MoveTo { x: start_x, y: y0 },
        PathCommand::LineTo {
            x: top_end_x,
            y: top_y,
        },
        PathCommand::Arc {
            cx: top_right_cx,
            cy: top_right_cy,
            rx,
            ry,
            start: top_right_start,
            sweep: top_right_sweep,
        },
        PathCommand::LineTo {
            x: x1,
            y: right_end_y,
        },
        PathCommand::Arc {
            cx: bottom_right_cx,
            cy: bottom_right_cy,
            rx: bottom_right_rx,
            ry: bottom_right_ry,
            start: bottom_right_start,
            sweep: bottom_right_sweep,
        },
        PathCommand::LineTo {
            x: bottom_end_x,
            y: y1,
        },
        PathCommand::Arc {
            cx: bottom_left_cx,
            cy: bottom_left_cy,
            rx: bottom_left_rx,
            ry: bottom_left_ry,
            start: bottom_left_start,
            sweep: bottom_left_sweep,
        },
        PathCommand::LineTo {
            x: x0,
            y: left_end_y,
        },
        PathCommand::Arc {
            cx: top_left_cx,
            cy: top_left_cy,
            rx: top_left_rx,
            ry: top_left_ry,
            start: top_left_start,
            sweep: top_left_sweep,
        },
        PathCommand::Close,
    ] = commands
    else {
        return None;
    };

    let quarter_turn = core::f32::consts::FRAC_PI_2;
    let uniform_radii = [*bottom_right_rx, *bottom_left_rx, *top_left_rx]
        .into_iter()
        .all(|radius| approx_eq(radius, *rx))
        && [*bottom_right_ry, *bottom_left_ry, *top_left_ry]
            .into_iter()
            .all(|radius| approx_eq(radius, *ry));
    let geometry_matches = approx_eq(*top_y, *y0)
        && approx_eq(*start_x, *x0 + *rx)
        && approx_eq(*top_end_x, *x1 - *rx)
        && approx_eq(*top_right_cx, *x1 - *rx)
        && approx_eq(*top_right_cy, *y0 + *ry)
        && approx_eq(*right_end_y, *y1 - *ry)
        && approx_eq(*bottom_right_cx, *x1 - *rx)
        && approx_eq(*bottom_right_cy, *y1 - *ry)
        && approx_eq(*bottom_end_x, *x0 + *rx)
        && approx_eq(*bottom_left_cx, *x0 + *rx)
        && approx_eq(*bottom_left_cy, *y1 - *ry)
        && approx_eq(*left_end_y, *y0 + *ry)
        && approx_eq(*top_left_cx, *x0 + *rx)
        && approx_eq(*top_left_cy, *y0 + *ry);
    let angles_match = approx_eq(*top_right_start, -quarter_turn)
        && approx_eq(*top_right_sweep, quarter_turn)
        && approx_eq(*bottom_right_start, 0.0)
        && approx_eq(*bottom_right_sweep, quarter_turn)
        && approx_eq(*bottom_left_start, quarter_turn)
        && approx_eq(*bottom_left_sweep, quarter_turn)
        && approx_eq(*top_left_start, core::f32::consts::PI)
        && approx_eq(*top_left_sweep, quarter_turn);
    if !uniform_radii
        || !geometry_matches
        || !angles_match
        || !valid_rect(*x0, *y0, *x1, *y1)
        || !rx.is_finite()
        || !ry.is_finite()
        || *rx < 0.0
        || *ry < 0.0
    {
        return None;
    }

    // A normalized corner radius resolves against the shorter side, so the corner
    // stays circular on a non-square rect. Scaling each axis by its own extent
    // instead turns every rounded-rect *clip* into an ellipse while the identical
    // shape *fills* as a rounded rect, because the fill route (`rounded_rect_path`)
    // already resolves against `min_side`. The two must agree.
    let min_side = bounds.width().min(bounds.height()).max(0.0);
    Some(RegularClipShape::RoundedRect {
        rect: resolve_normalized_rect(*x0, *y0, *x1, *y1, bounds),
        corner_width: f64::from(*rx) * min_side,
        corner_height: f64::from(*ry) * min_side,
    })
}

fn resolve_normalized_rect(x0: f32, y0: f32, x1: f32, y1: f32, bounds: kurbo::Rect) -> kurbo::Rect {
    kurbo::Rect::new(
        f64::from(x0) * bounds.width(),
        f64::from(y0) * bounds.height(),
        f64::from(x1) * bounds.width(),
        f64::from(y1) * bounds.height(),
    )
}

fn valid_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    [x0, y0, x1, y1].into_iter().all(f32::is_finite) && x0 <= x1 && y0 <= y1
}

fn approx_eq(left: f32, right: f32) -> bool {
    (left - right).abs() <= f32::EPSILON * 64.0
}

#[cfg(test)]
mod regular_clip_tests {
    use waterui_shape::{Path, Rectangle, RoundedRectangle, Shape as _, UnevenRoundedRectangle};

    use super::*;

    const BOUNDS: kurbo::Rect = kurbo::Rect::new(0.0, 0.0, 200.0, 100.0);

    #[test]
    fn recognizes_axis_aligned_rectangle() {
        assert_eq!(
            regular_clip_shape(&Rectangle.path(), BOUNDS),
            Some(RegularClipShape::Rect(BOUNDS))
        );
    }

    /// A normalized corner radius resolves against the shorter side, so the
    /// corners stay circular on a non-square rect and a clip matches the fill of
    /// the same shape. Resolving each axis against its own extent produced
    /// elliptical corners — a fully-rounded clip came out as an ellipse instead
    /// of a pill.
    #[test]
    fn uniform_rounded_rectangle_clip_keeps_circular_corners() {
        let Some(RegularClipShape::RoundedRect {
            rect,
            corner_width,
            corner_height,
        }) = regular_clip_shape(&RoundedRectangle::new(0.1).path(), BOUNDS)
        else {
            panic!("uniform rounded rectangle must use the regular clip route");
        };
        let min_side = BOUNDS.width().min(BOUNDS.height());
        assert_eq!(rect, BOUNDS);
        assert!(0.1f64.mul_add(-min_side, corner_width).abs() < 1.0e-5);
        assert!(
            (corner_width - corner_height).abs() < 1.0e-5,
            "a uniform rounded rectangle must clip with circular corners, got \
             {corner_width}x{corner_height} on a {}x{} rect",
            BOUNDS.width(),
            BOUNDS.height()
        );
    }

    /// The fully-rounded case: a clip at the maximum normalized radius is a
    /// stadium whose caps are half the shorter side, not an ellipse.
    #[test]
    fn fully_rounded_clip_is_a_stadium_not_an_ellipse() {
        let Some(RegularClipShape::RoundedRect {
            corner_width,
            corner_height,
            ..
        }) = regular_clip_shape(&RoundedRectangle::new(0.5).path(), BOUNDS)
        else {
            panic!("a fully-rounded rectangle must use the regular clip route");
        };
        let cap = BOUNDS.width().min(BOUNDS.height()) / 2.0;
        assert!((corner_width - cap).abs() < 1.0e-5);
        assert!((corner_height - cap).abs() < 1.0e-5);
    }

    #[test]
    fn leaves_uneven_and_custom_paths_on_the_path_mask_route() {
        assert_eq!(
            regular_clip_shape(
                &UnevenRoundedRectangle::new(0.1, 0.2, 0.3, 0.4).path(),
                BOUNDS,
            ),
            None
        );
        let triangle = Path::new()
            .move_to(0.5, 0.0)
            .line_to(1.0, 1.0)
            .line_to(0.0, 1.0)
            .close();
        let triangle_commands: Vec<_> = triangle.path().collect();
        assert_eq!(regular_clip_shape(&triangle_commands, BOUNDS), None);
    }
}
