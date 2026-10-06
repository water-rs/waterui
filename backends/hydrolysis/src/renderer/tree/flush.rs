//! Per-frame flush: [`RenderNode::flush`] re-encodes the laid-out subtree
//! into the renderer's scene using the cached placements.

#[cfg(feature = "accessibility")]
use super::layout::kurbo_rect;
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

impl RenderNode {
    /// Re-encode this subtree into the renderer's scene using the cached
    /// placements. Runs every frame.
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    // `_focus_node` is bound only for the accessibility surface-input path; on
    // builds without the feature the bindings stay dormant, which is why they keep
    // the underscore marker.
    #[allow(clippy::used_underscore_binding)]
    pub(crate) fn flush(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
    ) {
        match self {
            Self::Color(node) => {
                renderer.state.counters.recorded_view_contents += 1;
                let color = waterui_graphics::draw::Paint::Solid(renderer.read_signal(&node.color));
                renderer.scene_mut().fill_paint(
                    peniko::Fill::NonZero,
                    ctx.transform,
                    color,
                    &ctx.bounds,
                );
            }
            // §7.1's fill rule, paint side: the extension layout recorded
            // grows the child's paint rect — nothing else moves.
            Self::Fill(node) => {
                let ctx = node
                    .extension
                    .get()
                    .map_or(ctx, |extension| safe_area::fill_paint_ctx(ctx, extension));
                node.child.flush(renderer, ctx, env);
            }
            Self::Text(text) => {
                renderer.state.counters.recorded_view_contents += 1;
                renderer.push_render_owner(&text.accessibility_identity);
                // Read the content/alignment signals through `read_signal` so a change
                // re-subscribes this frame and schedules a window refresh — the same
                // cheap pump every other reactive leaf uses. (A bare reactive `Text`
                // has no surrounding widget to watch it, so the node must do so itself.)
                let styled = renderer.read_signal(&text.content);
                let alignment = renderer.read_signal(&text.alignment);
                TextNode::emit_accessibility(renderer, Some(ctx), &styled, env);
                renderer.pop_render_owner();
                let (state, scene) = renderer.state_and_scene_mut();
                HydrolysisRenderer::render_styled_text_limited(
                    state,
                    scene,
                    ctx,
                    styled,
                    alignment,
                    env,
                    text.line_limit.map_or(TailMark::None, TailMark::Ellipsis),
                );
            }
            Self::Container(container) => {
                renderer.push_render_owner(&container.accessibility_identity);
                // The claim is resolved against the env the flush actually
                // sees: an enclosing claim (e.g. a tap gesture's own node)
                // hands its children a stripped environment where this
                // container no longer carries semantics to claim.
                #[cfg(feature = "accessibility")]
                let container_scope = accessibility_container_child_environment(env).map(|_| {
                    renderer.begin_accessibility_container(
                        transformed_rect(ctx.hit_transform, ctx.bounds),
                        Some(transformed_rect(
                            ctx.hit_transform,
                            kurbo_rect(container.resolved),
                        )),
                        env,
                    )
                });
                #[cfg(feature = "accessibility")]
                let child_env = container.accessibility_child_env.as_ref().unwrap_or(env);
                #[cfg(not(feature = "accessibility"))]
                let child_env = env;
                renderer.pop_render_owner();
                for (child, rect) in container.children.iter().zip(container.placed.iter()) {
                    let child_ctx = ctx.child(
                        kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y()))),
                        kurbo::Rect::new(
                            0.0,
                            0.0,
                            f64::from(rect.width()),
                            f64::from(rect.height()),
                        ),
                    );
                    child.flush(renderer, child_ctx, child_env);
                }
                #[cfg(feature = "accessibility")]
                if let Some(container_scope) = container_scope {
                    renderer.push_accessibility_owner(&container.accessibility_identity);
                    renderer.end_accessibility_container(container_scope);
                    renderer.pop_accessibility_owner();
                }
            }
            Self::Opacity(node) => {
                let alpha = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.value,
                    OPACITY_ANIMATION_KEY,
                );
                renderer.with_clip_rect_scope(
                    alpha,
                    LayerTransforms {
                        paint: ctx.transform,
                        hit: ctx.hit_transform,
                    },
                    ctx.bounds,
                    |renderer| node.child.flush(renderer, ctx, env),
                );
            }
            Self::Scale(node) => {
                let center = anchor_point(ctx.bounds, node.value.anchor);
                let scale_x = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.x,
                    SCALE_X_ANIMATION_KEY,
                );
                let scale_y = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.y,
                    SCALE_Y_ANIMATION_KEY,
                );
                let transform = kurbo::Affine::translate((center.x, center.y))
                    * kurbo::Affine::scale_non_uniform(f64::from(scale_x), f64::from(scale_y))
                    * kurbo::Affine::translate((-center.x, -center.y));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            Self::Rotation(node) => {
                let center = anchor_point(ctx.bounds, node.value.anchor);
                let radians = f64::from(renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.angle,
                    ROTATION_ANIMATION_KEY,
                ))
                .to_radians();
                let transform = kurbo::Affine::translate((center.x, center.y))
                    * kurbo::Affine::rotate(radians)
                    * kurbo::Affine::translate((-center.x, -center.y));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            Self::Offset(node) => {
                let offset_x = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.x,
                    OFFSET_X_ANIMATION_KEY,
                );
                let offset_y = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.y,
                    OFFSET_Y_ANIMATION_KEY,
                );
                let transform =
                    kurbo::Affine::translate((f64::from(offset_x), f64::from(offset_y)));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            Self::Dynamic(node) => {
                if node.apply_pending_mid_pass(renderer) {
                    // The parent placed this host before the pending existed,
                    // so the new child has no placements yet: lay it out inside
                    // the host's assigned rect so it encodes a self-consistent
                    // subtree this frame. `apply_pending_mid_pass` already
                    // marked the host layout-dirty, so the enclosing retained
                    // sub-view re-lays out (re-placing this host) before its
                    // next flush.
                    #[allow(clippy::cast_possible_truncation)]
                    let size = Size::new(ctx.bounds.width() as f32, ctx.bounds.height() as f32);
                    let proposal = ProposalSize::new(Some(size.width), Some(size.height));
                    let safe_area = node.safe_area.as_deref().cloned();
                    node.child
                        .borrow_mut()
                        .layout(renderer, &node.env, safe_area, proposal, size);
                }
                node.child.borrow().flush(renderer, ctx, env);
            }
            Self::Retain(node) => node.child.flush(renderer, ctx, env),
            Self::Env(node) => {
                renderer.register_modal_scope(&node.env);
                node.child.flush(renderer, ctx, &node.env);
            }
            Self::Wrapper(node) => {
                renderer.push_render_owner(&node.accessibility_identity);
                // Each effect re-applies through the shared `apply_*` helper, with
                // a closure that flushes the child node under the wrapper's scoped
                // environment — so reactive descendants reach their own nodes and
                // keep updating, instead of being frozen by a one-shot capture.
                let child_env = &node.env;
                match &node.effect {
                    WrapperEffect::NavigationTransitionSource(id) => {
                        flush_navigation_transition_element(
                            renderer,
                            ctx,
                            child_env,
                            &node.child,
                            true,
                            *id,
                        );
                    }
                    WrapperEffect::NavigationTransitionDestination(id) => {
                        flush_navigation_transition_element(
                            renderer,
                            ctx,
                            child_env,
                            &node.child,
                            false,
                            *id,
                        );
                    }
                    WrapperEffect::Clip(value) => {
                        HydrolysisRenderer::apply_clip_shape(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Border(value) => {
                        HydrolysisRenderer::apply_border(renderer, ctx, child_env, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Shadow(value) => {
                        HydrolysisRenderer::apply_shadow(renderer, ctx, child_env, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Material(level) => {
                        // Everything painted so far is the material's
                        // backdrop: close that segment, present the keyed
                        // member mount, and flush the content above it.
                        // The member's scope is the stack's top — the nearest
                        // enclosing `.material_group()` — or none.
                        renderer.flush_scene_layer();
                        // The resolved scheme keys the backdrop group — a
                        // subtree-installed appearance must not share a
                        // capture. `read_signal` subscribes the flush, so an
                        // appearance flip requests a refresh and re-keys the
                        // member.
                        let scheme =
                            renderer.read_signal(&waterui::theme::current_color_scheme(child_env));
                        renderer
                            .compositor
                            .render_layers
                            .push(RenderLayer::Material(MaterialLayer {
                                key: crate::renderer::retained::RenderKey {
                                    render: node.render_id,
                                    presentation:
                                        crate::renderer::retained::PresentationId::ORDINARY,
                                },
                                scope: renderer.compositor.material_scopes.last().copied(),
                                scheme,
                                level: *level,
                                transform: ctx.transform,
                                bounds: ctx.bounds,
                                active_layers: renderer.compositor.active_scene_layers.clone(),
                            }));
                        node.child.flush(renderer, ctx, child_env);
                    }
                    WrapperEffect::MaterialGroup => {
                        // The node's render identity is the group scope:
                        // push it while the child flushes, then pop, so the
                        // members inside join its shared backdrop group.
                        renderer.compositor.material_scopes.push(node.render_id);
                        node.child.flush(renderer, ctx, child_env);
                        renderer.compositor.material_scopes.pop();
                    }
                    WrapperEffect::PopupMenuSurface => {
                        HydrolysisRenderer::apply_popup_menu_surface(renderer, ctx, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::AnchoredOverlay(value) => {
                        HydrolysisRenderer::apply_anchored_overlay(
                            renderer,
                            ctx,
                            child_env,
                            value,
                            |r| {
                                node.child.flush(r, ctx, child_env);
                            },
                        );
                    }
                    WrapperEffect::LayoutPriority(_) => {
                        // Layout-only: nothing to apply while drawing.
                        node.child.flush(renderer, ctx, child_env);
                    }
                    WrapperEffect::IgnoreSafeArea(_) => {
                        // The mirror of the layout arm: the release layout
                        // computed lands the child where the grown frame
                        // put it — the transform carries the leading/top
                        // overhang so descendants place from the shifted
                        // origin too.
                        node.child.flush(
                            renderer,
                            safe_area::released_ctx(ctx, node.released_offsets.get()),
                            child_env,
                        );
                    }
                    WrapperEffect::Cursor(value) => {
                        HydrolysisRenderer::apply_cursor(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::Draggable(value) => {
                        HydrolysisRenderer::apply_draggable(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::DropDestination(handles) => {
                        HydrolysisRenderer::apply_drop_destination(
                            renderer,
                            ctx,
                            child_env,
                            handles,
                            |r| {
                                node.child.flush(r, ctx, child_env);
                            },
                        );
                    }
                    WrapperEffect::ContextMenu(value) => {
                        HydrolysisRenderer::apply_context_menu(
                            renderer,
                            ctx,
                            child_env,
                            value,
                            |r| {
                                node.child.flush(r, ctx, child_env);
                            },
                        );
                    }
                    WrapperEffect::Hittable(value) => {
                        HydrolysisRenderer::apply_hittable(renderer, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::OnEvent(handler) => {
                        HydrolysisRenderer::apply_on_event(
                            renderer,
                            ctx,
                            child_env,
                            Rc::clone(handler),
                            |r| {
                                node.child.flush(r, ctx, child_env);
                            },
                        );
                    }
                    WrapperEffect::OnKeyPress(handler) => {
                        renderer.push_key_handler_scope(child_env.clone(), Rc::clone(handler));
                        node.child.flush(renderer, ctx, child_env);
                        renderer.pop_key_handler_scope();
                    }
                    WrapperEffect::GestureObserver(effect) => {
                        HydrolysisRenderer::apply_gesture_observer(
                            renderer,
                            ctx,
                            child_env,
                            effect,
                            |r, walk_env| {
                                node.child.flush(r, ctx, walk_env);
                            },
                        );
                    }
                    WrapperEffect::Focused(value) => {
                        HydrolysisRenderer::apply_focused(renderer, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
                    }
                    WrapperEffect::LifeCycle(effect) => {
                        // Flush the child first so reactive animation handles bind
                        // their initial values before an appear callback changes the
                        // target. Consuming the hook makes this a one-time event on
                        // the retained node; disappear still fires from `Drop`.
                        node.child.flush(renderer, ctx, child_env);
                        if let Some(hook) = effect.appear.take() {
                            hook.call();
                        }
                    }
                }
                renderer.pop_render_owner();
            }
            Self::SceneView(node) => {
                renderer.state.counters.recorded_view_contents += 1;
                // The drawing's own name and content, read every flush: content
                // that follows a signal answers with what it currently draws.
                let (content_label, content_value, wants_input) = {
                    let content = node.content.borrow();
                    (
                        content.accessibility_label(),
                        content.accessibility_value(),
                        content.wants_input_events(),
                    )
                };
                renderer.push_render_owner(&node.accessibility_identity);
                #[allow(
                    clippy::let_unit_value,
                    reason = "without the accessibility feature the stub returns (); with it the binding carries the focus id into the input-target registration below"
                )]
                let _focus_node = emit_graphics_image_accessibility(
                    renderer,
                    Some(ctx),
                    env,
                    content_label,
                    content_value,
                    wants_input,
                );
                renderer.pop_render_owner();
                // The content re-records itself onto its keyed layer at
                // `surface.update` every frame (#1324): the flush only
                // presents the layer.
                renderer.flush_scene_layer();
                renderer
                    .compositor
                    .render_layers
                    .push(RenderLayer::SceneContent(SceneContentLayer {
                        key: crate::renderer::retained::RenderKey {
                            render: node.render_id,
                            presentation: crate::renderer::retained::PresentationId::ORDINARY,
                        },
                        content: Rc::clone(&node.content),
                        invalidator: Rc::clone(&node.invalidator),
                        association: Rc::clone(&node.association),
                        transform: ctx.transform,
                        bounds: ctx.bounds,
                        active_layers: renderer.compositor.active_scene_layers.clone(),
                    }));
                // Content that handles its own input receives the pointer,
                // keyboard, IME and scroll events landing on its bounds, through
                // the same routing an interactive `GpuContentView` uses.
                if wants_input {
                    renderer.register_surface_input_target(
                        ctx.bounds,
                        ctx.hit_transform,
                        Rc::clone(&node.content),
                        #[cfg(feature = "accessibility")]
                        _focus_node,
                    );
                }
            }
            Self::GpuContent(node) => {
                renderer.state.counters.recorded_view_contents += 1;
                // The view's own name and content, read every flush — it is
                // re-asked after each frame it produces.
                let (content_label, content_value, wants_input) = {
                    let view = &node.runtime.borrow().view;
                    (
                        view.accessibility_label().map(str::to_owned),
                        view.accessibility_value().map(str::to_owned),
                        view.wants_input_events(),
                    )
                };
                renderer.push_render_owner(&node.accessibility_identity);
                #[allow(
                    clippy::let_unit_value,
                    reason = "without the accessibility feature the stub returns (); with it the binding carries the focus id into node.flush below"
                )]
                let _focus_node = emit_graphics_image_accessibility(
                    renderer,
                    Some(ctx),
                    env,
                    content_label,
                    content_value,
                    wants_input,
                );
                renderer.pop_render_owner();
                renderer.flush_scene_layer();
                renderer
                    .compositor
                    .render_layers
                    .push(RenderLayer::GpuContent(GpuContentLayer {
                        key: crate::renderer::retained::RenderKey {
                            render: node.render_id,
                            presentation: crate::renderer::retained::PresentationId::ORDINARY,
                        },
                        runtime: Rc::clone(&node.runtime),
                        transform: ctx.transform,
                        bounds: ctx.bounds,
                        active_layers: renderer.compositor.active_scene_layers.clone(),
                    }));
                if wants_input {
                    renderer.register_surface_input_target(
                        ctx.bounds,
                        ctx.hit_transform,
                        Rc::clone(&node.runtime),
                        #[cfg(feature = "accessibility")]
                        _focus_node,
                    );
                }
            }
            Self::ExternalFrame(node) => {
                renderer.state.counters.recorded_view_contents += 1;
                let (content_label, content_value) = {
                    let view = &node.runtime.borrow().view;
                    (
                        view.accessibility_label().map(str::to_owned),
                        view.accessibility_value().map(str::to_owned),
                    )
                };
                renderer.push_render_owner(&node.accessibility_identity);
                #[allow(
                    clippy::let_unit_value,
                    reason = "without the accessibility feature the stub returns ()"
                )]
                let _focus_node = emit_graphics_image_accessibility(
                    renderer,
                    Some(ctx),
                    env,
                    content_label,
                    content_value,
                    false,
                );
                renderer.pop_render_owner();
                renderer.flush_scene_layer();
                renderer
                    .compositor
                    .render_layers
                    .push(RenderLayer::ExternalFrame(ExternalFrameLayer {
                        key: crate::renderer::retained::RenderKey {
                            render: node.render_id,
                            presentation: crate::renderer::retained::PresentationId::ORDINARY,
                        },
                        runtime: Rc::clone(&node.runtime),
                        transform: ctx.transform,
                        bounds: ctx.bounds,
                        active_layers: renderer.compositor.active_scene_layers.clone(),
                    }));
            }
            Self::Filtered(node) => {
                // Ancestor clips and opacity belong on the filtered mount
                // itself — the engine's `Filter` covers the mount's whole
                // subtree — so the children's scene segments must not bake
                // them in a second time. Drain the ops above the filter, then
                // unwind the paint stack for the child flush and re-open the
                // scopes for what flushes after.
                renderer.flush_scene_layer();
                let ancestry = core::mem::take(&mut renderer.compositor.active_scene_layers);
                for _ in 0..ancestry.len() {
                    renderer.scene_mut().pop_scope();
                }
                let children_start = renderer.compositor.render_layers.len();
                node.child.flush(renderer, ctx, &node.env);
                renderer.flush_scene_layer();
                let children = renderer.compositor.render_layers.split_off(children_start);
                for layer in &ancestry {
                    layer.push_to_scene(renderer.scene_mut());
                }
                renderer
                    .compositor
                    .active_scene_layers
                    .clone_from(&ancestry);
                renderer
                    .compositor
                    .render_layers
                    .push(RenderLayer::Filtered(FilteredLayer {
                        key: crate::renderer::retained::RenderKey {
                            render: node.render_id,
                            presentation: crate::renderer::retained::PresentationId::ORDINARY,
                        },
                        runtime: Rc::clone(&node.runtime),
                        children,
                        active_layers: ancestry,
                    }));
            }
            Self::Scroll(node) => {
                // §7.1's scroll surface: the viewport is the laid-out frame
                // grown by the extension layout computed — the surface
                // paints through the bands its frame touched, clips its
                // content there, and its own subtree owns the inset.
                let viewport_rect = safe_area::grow_rect(ctx.bounds, node.surface.extension());
                let Some(handle) = node.handle.borrow().clone() else {
                    return;
                };
                // The keyboard-moving clearance runs before the content
                // paints and before metrics are read: while the host's
                // keyboard animation is in flight the offset follows it
                // frame by frame, so this flush paints the field already
                // clear — `begin_flush` first, metrics after (the order
                // `List`/`Table` use).
                let targets_start = node.surface.begin_flush(renderer, &handle);
                let metrics = handle.metrics();
                renderer.with_clip_rect_scope(
                    1.0,
                    LayerTransforms {
                        paint: ctx.transform,
                        hit: ctx.hit_transform,
                    },
                    viewport_rect,
                    |renderer| {
                        let scroll_offset =
                            kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
                        let content_bounds = kurbo::Rect::new(
                            0.0,
                            0.0,
                            f64::from(node.content_size.width),
                            f64::from(node.content_size.height),
                        );
                        let content_ctx = RenderContext::with_transforms(
                            content_bounds,
                            ctx.transform * scroll_offset,
                            ctx.hit_transform * scroll_offset,
                        );
                        // Publish the visible window (in content coordinates) —
                        // the viewport window grown by the same extension —
                        // so a virtualized `LazyStack` child builds the rows
                        // painted inside the extended clip, not just the ones
                        // inside the laid-out frame.
                        let horizontal =
                            node.surface.visible_span(&metrics, ScrollAxis::Horizontal);
                        let vertical = node.surface.visible_span(&metrics, ScrollAxis::Vertical);
                        let lazy_viewport = kurbo::Rect::new(
                            horizontal.start,
                            vertical.start,
                            horizontal.end,
                            vertical.end,
                        );
                        // Registered before the content so the content can be parented
                        // to it: a scroll region owns what it scrolls, and a label on
                        // the scroll view must reach the node carrying the scroll
                        // actions rather than a group beside it.
                        #[cfg(feature = "accessibility")]
                        let scroll_accessibility_node = {
                            renderer.push_accessibility_owner(&node.accessibility_identity);
                            let scroll_accessibility_node =
                                crate::widgets::scroll::register_scroll_accessibility_node(
                                    renderer,
                                    &node.env,
                                    Some(transformed_rect(ctx.hit_transform, viewport_rect)),
                                    &handle,
                                    metrics,
                                    node.axis,
                                );
                            renderer.pop_accessibility_owner();
                            if let Some(scroll_accessibility_node) = scroll_accessibility_node {
                                renderer.push_accessibility_parent(scroll_accessibility_node);
                            }
                            scroll_accessibility_node
                        };
                        // The wheel/trackpad target registers before the content
                        // flushes: dispatch walks the frame's targets newest-first, so
                        // a scroll region nested inside this one — registered by the
                        // child below — wins the delta until it hits its own edge.
                        crate::widgets::scroll::register_scroll_wheel_target(
                            renderer,
                            ctx.hit_transform,
                            viewport_rect,
                            &handle,
                        );
                        renderer.push_lazy_viewport(crate::renderer::lifecycle::LazyViewport {
                            bounds: lazy_viewport,
                            transform: content_ctx.transform,
                        });
                        node.child.flush(renderer, content_ctx, env);
                        renderer.pop_lazy_viewport("hydrolysis render tree ScrollNode");
                        #[cfg(feature = "accessibility")]
                        if scroll_accessibility_node.is_some() {
                            renderer.pop_accessibility_parent();
                        }
                    },
                );
                // The focused-field clearance reads this frame's input
                // targets — the child's flush above just emitted them.
                node.surface.end_flush(renderer, &handle, targets_start);
                // The indicators ride the surface's own frame, not the
                // extended clip: they stay visible at the avoided edge.
                let scroll_ctx =
                    RenderContext::with_transforms(ctx.bounds, ctx.transform, ctx.hit_transform);
                let mut widget_ctx = WidgetRenderContext::new(renderer, scroll_ctx, None);
                crate::widgets::draw_scroll_indicators(
                    &mut widget_ctx,
                    &node.env,
                    ctx.bounds,
                    metrics,
                    node.axis,
                    &handle,
                    node.surface.extension(),
                );
            }
            Self::LazyStack(node) => node.flush(renderer, ctx, env),
            Self::Collection(node) => node.flush(renderer, ctx),
            Self::Widget(node) => {
                renderer.state.counters.recorded_view_contents += 1;
                renderer.push_render_owner(&node.accessibility_identity);
                // Re-render the leaf widget from its retained config so its handler
                // re-reads live signals and re-emits interaction targets + a11y at the
                // current bounds. A leaf render starts a fresh recursion depth.
                renderer.render_depth = 0;
                // The node's stored environment keeps precedence for the keys it
                // scoped at build time; the live flush environment supplies any
                // keys injected since — a compact split's leading reserve is one.
                let merged;
                let env = if node.env.identity() == env.identity() {
                    &node.env
                } else {
                    merged = node.env.layered_on(env);
                    &merged
                };
                let safe_area = node.safe_area.as_deref().cloned();
                Rc::clone(&node.behavior).render(renderer, ctx, env, safe_area);
                renderer.pop_render_owner();
            }
        }
    }
}

impl RenderNode {
    /// Emits this subtree's accessibility nodes for the semantic walk — the
    /// same tree `flush` produces under a `RenderContext`, with no bounds, no
    /// scene writes, no layers, and no pointer/hit targets. This is the
    /// semantic runtime's pump: it walks the retained tree exactly as a flush
    /// does so the emitted structure cannot drift from what a frame would
    /// produce, and every registration goes through the no-bounds semantic
    /// path.
    #[cfg(feature = "accessibility")]
    #[expect(
        clippy::option_if_let_else,
        reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(crate) fn emit_accessibility(&self, renderer: &mut SemanticCore, env: &Environment) {
        match self {
            // A color fill carries no semantics.
            Self::Color(_) => {}
            Self::Text(text) => {
                renderer.push_accessibility_owner(&text.accessibility_identity);
                let styled = renderer.read_signal(&text.content);
                TextNode::emit_accessibility(renderer, None, &styled, env);
                renderer.pop_accessibility_owner();
            }
            Self::Container(container) => {
                renderer.push_accessibility_owner(&container.accessibility_identity);
                // As in `flush`: the claim is resolved on the env this walk
                // actually sees — an enclosing claim strips the naming
                // metadata before it reaches here.
                let container_scope = accessibility_container_child_environment(env)
                    .map(|_| renderer.begin_accessibility_container_semantic(env));
                let child_env = container.accessibility_child_env.as_ref().unwrap_or(env);
                renderer.pop_accessibility_owner();
                for child in &container.children {
                    child.emit_accessibility(renderer, child_env);
                }
                if let Some(container_scope) = container_scope {
                    renderer.push_accessibility_owner(&container.accessibility_identity);
                    renderer.end_accessibility_container(container_scope);
                    renderer.pop_accessibility_owner();
                }
            }
            // Transforms are presentation: the semantic tree keeps the child.
            Self::Opacity(node) => node.child.emit_accessibility(renderer, env),
            Self::Scale(node) => node.child.emit_accessibility(renderer, env),
            Self::Rotation(node) => node.child.emit_accessibility(renderer, env),
            Self::Offset(node) => node.child.emit_accessibility(renderer, env),
            Self::Dynamic(node) => {
                node.apply_pending_mid_pass(renderer);
                node.child.borrow().emit_accessibility(renderer, env);
            }
            Self::Retain(node) => node.child.emit_accessibility(renderer, env),
            Self::Env(node) => {
                renderer.register_modal_scope(&node.env);
                node.child.emit_accessibility(renderer, &node.env);
            }
            Self::Wrapper(node) => {
                renderer.push_accessibility_owner(&node.accessibility_identity);
                let child_env = &node.env;
                match &node.effect {
                    WrapperEffect::GestureObserver(effect) => {
                        let claimed_node = HydrolysisRenderer::emit_gesture_observer_accessibility(
                            renderer, child_env, effect,
                        );
                        // The gesture's node claimed the naming scope: the
                        // content walks under the same shielded environment a
                        // naming container hands its children, so a leaf cannot
                        // repeat the claim's role and label as a second node.
                        let content_env;
                        let walk_env = if claimed_node.is_some() {
                            content_env = accessibility_container_child_environment(child_env)
                                .unwrap_or_else(|| child_env.clone());
                            &content_env
                        } else {
                            child_env
                        };
                        // `ExcludeDescendants` belongs to the element that
                        // claims this naming scope — an observer that registers
                        // no node (a long-press, or a silenced tap) must not
                        // consume the flag, or it suppresses the inner element
                        // the flag names (water-rs/hydrolysis#266).
                        if claimed_node.is_some()
                            && child_env
                                .get::<AccessibilityChildren>()
                                .is_some_and(AccessibilityChildren::excludes_descendants)
                        {
                            renderer.push_accessibility_suppression();
                            node.child.emit_accessibility(renderer, walk_env);
                            renderer.pop_accessibility_suppression();
                        } else {
                            node.child.emit_accessibility(renderer, walk_env);
                        }
                        if let Some(node_id) = claimed_node {
                            renderer.drain_claim_scope(node_id, child_env);
                        }
                    }
                    WrapperEffect::Focused(value) => {
                        HydrolysisRenderer::apply_focused_semantic(renderer, value, |r| {
                            node.child.emit_accessibility(r, child_env);
                        });
                    }
                    WrapperEffect::OnKeyPress(handler) => {
                        renderer.push_key_handler_scope(child_env.clone(), Rc::clone(handler));
                        node.child.emit_accessibility(renderer, child_env);
                        renderer.pop_key_handler_scope();
                    }
                    WrapperEffect::LifeCycle(effect) => {
                        // The semantic pump is this tree's frame: an appear hook
                        // fires on the first walk exactly as on the first flush.
                        node.child.emit_accessibility(renderer, child_env);
                        if let Some(hook) = effect.appear.take() {
                            hook.call();
                        }
                    }
                    _ => node.child.emit_accessibility(renderer, child_env),
                }
                renderer.pop_accessibility_owner();
            }
            Self::SceneView(node) => {
                let (content_label, content_value, wants_input) = {
                    let content = node.content.borrow();
                    (
                        content.accessibility_label(),
                        content.accessibility_value(),
                        content.wants_input_events(),
                    )
                };
                renderer.push_accessibility_owner(&node.accessibility_identity);
                let focus_node = emit_graphics_image_accessibility(
                    renderer,
                    None,
                    env,
                    content_label,
                    content_value,
                    wants_input,
                );
                renderer.pop_accessibility_owner();
                if wants_input {
                    // The semantic walk registers the same input target the
                    // rendered flush would — there is no layout to bound it,
                    // so it is a focus-bookkeeping slot (keyboard traversal,
                    // `.focused`), not a hit rect.
                    renderer.register_surface_input_target(
                        kurbo::Rect::ZERO,
                        kurbo::Affine::IDENTITY,
                        Rc::clone(&node.content),
                        focus_node,
                    );
                }
            }
            Self::GpuContent(node) => {
                let (content_label, content_value, wants_input) = {
                    let view = &node.runtime.borrow().view;
                    (
                        view.accessibility_label().map(str::to_owned),
                        view.accessibility_value().map(str::to_owned),
                        view.wants_input_events(),
                    )
                };
                renderer.push_accessibility_owner(&node.accessibility_identity);
                let focus_node = emit_graphics_image_accessibility(
                    renderer,
                    None,
                    env,
                    content_label,
                    content_value,
                    wants_input,
                );
                renderer.pop_accessibility_owner();
                if wants_input {
                    renderer.register_surface_input_target(
                        kurbo::Rect::ZERO,
                        kurbo::Affine::IDENTITY,
                        Rc::clone(&node.runtime),
                        focus_node,
                    );
                }
            }
            Self::ExternalFrame(node) => {
                let (content_label, content_value) = {
                    let view = &node.runtime.borrow().view;
                    (
                        view.accessibility_label().map(str::to_owned),
                        view.accessibility_value().map(str::to_owned),
                    )
                };
                renderer.push_accessibility_owner(&node.accessibility_identity);
                emit_graphics_image_accessibility(
                    renderer,
                    None,
                    env,
                    content_label,
                    content_value,
                    false,
                );
                renderer.pop_accessibility_owner();
            }
            // The filter is a paint concern: the semantic tree keeps the
            // child exactly as it emits on its own.
            Self::Filtered(node) => {
                node.child.emit_accessibility(renderer, &node.env);
            }
            // The slot fill is a paint marker: the semantic tree keeps the child.
            Self::Fill(node) => node.child.emit_accessibility(renderer, env),
            Self::Scroll(node) => {
                // The semantic scroll domain is unbounded — there is no layout
                // to measure content against — so scroll actions move the
                // bound offset freely and `scroll_y_max` reports infinity.
                let handle = {
                    let mut slot = node.handle.borrow_mut();
                    let handle = if let Some(handle) = slot.as_mut() {
                        handle.rebind(
                            node.axis,
                            0.0,
                            0.0,
                            f64::INFINITY,
                            f64::INFINITY,
                            (0.0, 0.0),
                        )
                    } else {
                        ScrollHandle::new(
                            node.axis,
                            0.0,
                            0.0,
                            f64::INFINITY,
                            f64::INFINITY,
                            node.offset.clone(),
                        )
                    };
                    *slot = Some(handle.clone());
                    handle
                };
                let metrics = handle.metrics();
                if let Some(controller) = &node.controller {
                    let generation = renderer.read_signal(&controller.generation());
                    if generation != node.applied_scroll_generation.get() {
                        let request = renderer.read_signal(&controller.request());
                        // The semantic domain has no frame pump to advance an
                        // animation, so a request lands in place whether or
                        // not it carries one.
                        let _ = handle
                            .scroll_to(f64::from(request.target.x), f64::from(request.target.y));
                        node.applied_scroll_generation.set(generation);
                    }
                }
                renderer.push_accessibility_owner(&node.accessibility_identity);
                let scroll_accessibility_node =
                    crate::widgets::scroll::register_scroll_accessibility_node(
                        renderer, &node.env, None, &handle, metrics, node.axis,
                    );
                renderer.pop_accessibility_owner();
                if let Some(scroll_accessibility_node) = scroll_accessibility_node {
                    renderer.push_accessibility_parent(scroll_accessibility_node);
                }
                node.child.emit_accessibility(renderer, env);
                if scroll_accessibility_node.is_some() {
                    renderer.pop_accessibility_parent();
                }
            }
            Self::LazyStack(node) => node.emit_accessibility(renderer),
            Self::Collection(node) => node.emit_accessibility(renderer),
            Self::Widget(node) => {
                renderer.push_accessibility_owner(&node.accessibility_identity);
                let merged;
                let env = if node.env.identity() == env.identity() {
                    &node.env
                } else {
                    merged = node.env.layered_on(env);
                    &merged
                };
                Rc::clone(&node.behavior).emit_accessibility(renderer, env);
                renderer.pop_accessibility_owner();
            }
        }
    }
}

fn flush_navigation_transition_element(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    env: &Environment,
    child: &RenderNode,
    source: bool,
    id: RawId,
) {
    if !renderer.begin_navigation_element_capture() {
        child.flush(renderer, ctx, env);
        return;
    }
    let layers = renderer.capture_layers(|renderer| child.flush(renderer, ctx, env));
    renderer.finish_navigation_element_capture(
        source,
        id,
        transformed_rect(ctx.transform, ctx.bounds),
        layers,
    );
}
