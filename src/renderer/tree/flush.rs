//! Per-frame flush: [`RenderNode::flush`] re-encodes the laid-out subtree
//! into the renderer's scene using the cached placements.

use super::*;

pub(crate) struct ChildTextureTarget<'a> {
    pub(crate) texture: &'a wgpu::Texture,
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl RenderNode {
    /// Re-encode this subtree into the renderer's scene using the cached
    /// placements. Runs every frame.
    pub(crate) fn flush(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
    ) {
        match self {
            RenderNode::Color(color) => {
                let color = resolved_color_to_peniko(renderer.read_signal(&color.color));
                renderer.scene_mut().fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    color,
                    None,
                    &ctx.bounds,
                );
            }
            RenderNode::Text(text) => {
                renderer.push_render_owner(&text.accessibility_identity);
                // Read the content/alignment signals through `read_signal` so a change
                // re-subscribes this frame and schedules a window refresh — the same
                // cheap pump every other reactive leaf uses. (A bare reactive `Text`
                // has no surrounding widget to watch it, so the node must do so itself.)
                let styled = renderer.read_signal(&text.content);
                let alignment = renderer.read_signal(&text.alignment);
                text.emit_accessibility(renderer, Some(ctx), &styled, env);
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
            RenderNode::Container(container) => {
                renderer.push_render_owner(&container.accessibility_identity);
                // The claim is resolved against the env the flush actually
                // sees: an enclosing claim (e.g. a tap gesture's own node)
                // hands its children a stripped environment where this
                // container no longer carries semantics to claim.
                #[cfg(feature = "accessibility")]
                let container_scope = accessibility_container_child_environment(env).map(|_| {
                    renderer.begin_accessibility_container(
                        transformed_rect(ctx.hit_transform, ctx.bounds),
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
                        vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y()))),
                        vello::kurbo::Rect::new(
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
            RenderNode::Opacity(node) => {
                let alpha = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.value,
                    OPACITY_ANIMATION_KEY,
                );
                renderer.push_layer_rect(
                    alpha,
                    LayerTransforms {
                        paint: ctx.transform,
                        hit: ctx.hit_transform,
                    },
                    ctx.bounds,
                );
                node.child.flush(renderer, ctx, env);
                renderer.pop_layer();
            }
            RenderNode::Scale(node) => {
                let center = anchor_point(ctx.bounds, node.value.anchor);
                let scale_x = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.x,
                    SCALE_X_ANIMATION_KEY,
                );
                let scale_y = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.y,
                    SCALE_Y_ANIMATION_KEY,
                );
                let transform = vello::kurbo::Affine::translate((center.x, center.y))
                    * vello::kurbo::Affine::scale_non_uniform(
                        f64::from(scale_x),
                        f64::from(scale_y),
                    )
                    * vello::kurbo::Affine::translate((-center.x, -center.y));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            RenderNode::Rotation(node) => {
                let center = anchor_point(ctx.bounds, node.value.anchor);
                let radians = f64::from(renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.angle,
                    ROTATION_ANIMATION_KEY,
                ))
                .to_radians();
                let transform = vello::kurbo::Affine::translate((center.x, center.y))
                    * vello::kurbo::Affine::rotate(radians)
                    * vello::kurbo::Affine::translate((-center.x, -center.y));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            RenderNode::Offset(node) => {
                let offset_x = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.x,
                    OFFSET_X_ANIMATION_KEY,
                );
                let offset_y = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.y,
                    OFFSET_Y_ANIMATION_KEY,
                );
                let transform =
                    vello::kurbo::Affine::translate((f64::from(offset_x), f64::from(offset_y)));
                node.child
                    .flush(renderer, ctx.child(transform, ctx.bounds), env);
            }
            RenderNode::Dynamic(node) => {
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
                    node.child
                        .borrow_mut()
                        .layout(renderer, &node.env, proposal, size);
                }
                node.child.borrow().flush(renderer, ctx, env);
            }
            RenderNode::Retain(node) => node.child.flush(renderer, ctx, env),
            RenderNode::Env(node) => {
                renderer.register_modal_scope(&node.env);
                node.child.flush(renderer, ctx, &node.env);
            }
            RenderNode::Wrapper(node) => {
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
            RenderNode::SceneView(node) => {
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
                let mut scene = vello::Scene::new();
                // Scope `scene2d` so its `&mut scene` borrow ends before `&scene` is
                // appended below. `CheckedScene2D` validates every image brush at
                // this ingest boundary — malformed `ImageData` rejected here would
                // otherwise only fail inside wgpu's `write_texture`.
                let needs_next = {
                    let mut scene2d = VelloScene2D::new(&mut scene);
                    let mut scene2d = crate::renderer::CheckedScene2D::new(&mut scene2d);
                    #[allow(clippy::cast_possible_truncation)]
                    node.content.borrow_mut().build_scene(
                        &mut scene2d,
                        ctx.bounds.width() as f32,
                        ctx.bounds.height() as f32,
                    )
                };
                crate::engine::vello_backend::append_scene(
                    renderer.scene_mut(),
                    &scene,
                    Some(
                        ctx.transform
                            * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0)),
                    ),
                );
                if needs_next {
                    renderer.request_refresh();
                }
                // Content that handles its own input receives the pointer,
                // keyboard, IME and scroll events landing on its bounds, through
                // the same routing an interactive `GpuSurface` uses.
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
            RenderNode::GpuSurface(node) => {
                // The surface view's own name and content, read every flush —
                // it is re-asked after each frame it draws.
                let (content_label, content_value, wants_input) = {
                    let runtime = node.runtime.borrow();
                    (
                        runtime.accessibility_label(),
                        runtime.accessibility_value(),
                        runtime.wants_input_events(),
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
                node.flush(
                    renderer,
                    ctx,
                    #[cfg(feature = "accessibility")]
                    _focus_node,
                );
            }
            RenderNode::ViewEffect(node) => node.flush(renderer, ctx),
            RenderNode::AppliedFilter(node) => node.flush(renderer, ctx),
            RenderNode::Scroll(node) => {
                let Some(handle) = node.handle.borrow().clone() else {
                    return;
                };
                let metrics = handle.metrics();
                let viewport_rect = vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(node.viewport.width),
                    f64::from(node.viewport.height),
                );
                renderer.push_layer_rect(
                    1.0,
                    LayerTransforms {
                        paint: ctx.transform,
                        hit: ctx.hit_transform,
                    },
                    viewport_rect,
                );
                let scroll_offset =
                    vello::kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
                let content_bounds = vello::kurbo::Rect::new(
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
                // Publish the visible window (in content coordinates) so a
                // virtualized `LazyStack` child only builds the rows on screen.
                let lazy_viewport = vello::kurbo::Rect::new(
                    metrics.offset_x,
                    metrics.offset_y,
                    metrics.offset_x + f64::from(node.viewport.width),
                    metrics.offset_y + f64::from(node.viewport.height),
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
                renderer.pop_layer();
                let scroll_ctx =
                    RenderContext::with_transforms(viewport_rect, ctx.transform, ctx.hit_transform);
                let mut widget_ctx = WidgetRenderContext::new(renderer, scroll_ctx);
                crate::widgets::draw_scroll_indicators(
                    &mut widget_ctx,
                    &node.env,
                    viewport_rect,
                    metrics,
                    node.axis,
                    &handle,
                );
            }
            RenderNode::LazyStack(node) => node.flush(renderer, ctx, env),
            RenderNode::Collection(node) => node.flush(renderer, ctx),
            RenderNode::Widget(node) => {
                renderer.push_render_owner(&node.accessibility_identity);
                // Re-render the leaf widget from its retained config so its handler
                // re-reads live signals and re-emits interaction targets + a11y at the
                // current bounds. A leaf render starts a fresh recursion depth.
                renderer.render_depth = 0;
                Rc::clone(&node.behavior).render(renderer, ctx, &node.env);
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
    pub(crate) fn emit_accessibility(&self, renderer: &mut SemanticCore, env: &Environment) {
        match self {
            // A color fill carries no semantics.
            RenderNode::Color(_) => {}
            RenderNode::Text(text) => {
                renderer.push_accessibility_owner(&text.accessibility_identity);
                let styled = renderer.read_signal(&text.content);
                text.emit_accessibility(renderer, None, &styled, env);
                renderer.pop_accessibility_owner();
            }
            RenderNode::Container(container) => {
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
            RenderNode::Opacity(node) => node.child.emit_accessibility(renderer, env),
            RenderNode::Scale(node) => node.child.emit_accessibility(renderer, env),
            RenderNode::Rotation(node) => node.child.emit_accessibility(renderer, env),
            RenderNode::Offset(node) => node.child.emit_accessibility(renderer, env),
            RenderNode::Dynamic(node) => {
                node.apply_pending_mid_pass(renderer);
                node.child.borrow().emit_accessibility(renderer, env);
            }
            RenderNode::Retain(node) => node.child.emit_accessibility(renderer, env),
            RenderNode::Env(node) => {
                renderer.register_modal_scope(&node.env);
                node.child.emit_accessibility(renderer, &node.env);
            }
            RenderNode::Wrapper(node) => {
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
            RenderNode::SceneView(node) => {
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
                        vello::kurbo::Rect::ZERO,
                        vello::kurbo::Affine::IDENTITY,
                        Rc::clone(&node.content),
                        focus_node,
                    );
                }
            }
            RenderNode::GpuSurface(node) => {
                let (content_label, content_value, wants_input) = {
                    let runtime = node.runtime.borrow();
                    (
                        runtime.accessibility_label(),
                        runtime.accessibility_value(),
                        runtime.wants_input_events(),
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
                        vello::kurbo::Rect::ZERO,
                        vello::kurbo::Affine::IDENTITY,
                        Rc::clone(&node.runtime),
                        focus_node,
                    );
                }
            }
            RenderNode::ViewEffect(node) => {
                node.child.borrow().emit_accessibility(renderer, &node.env);
            }
            RenderNode::AppliedFilter(node) => {
                node.child.emit_accessibility(renderer, &node.env);
            }
            RenderNode::Scroll(node) => {
                // The semantic scroll domain is unbounded — there is no layout
                // to measure content against — so scroll actions move the
                // bound offset freely and `scroll_y_max` reports infinity.
                let handle = {
                    let mut slot = node.handle.borrow_mut();
                    let handle = if let Some(handle) = slot.as_mut() {
                        handle.rebind(node.axis, 0.0, 0.0, f64::INFINITY, f64::INFINITY)
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
                        let target = renderer.read_signal(&controller.target());
                        let _ = handle.scroll_to(f64::from(target.x), f64::from(target.y));
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
            RenderNode::LazyStack(node) => node.emit_accessibility(renderer),
            RenderNode::Collection(node) => node.emit_accessibility(renderer),
            RenderNode::Widget(node) => {
                renderer.push_accessibility_owner(&node.accessibility_identity);
                Rc::clone(&node.behavior).emit_accessibility(renderer, &node.env);
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
    let mut scene = vello::Scene::new();
    core::mem::swap(renderer.scene_mut(), &mut scene);
    child.flush(renderer, ctx, env);
    core::mem::swap(renderer.scene_mut(), &mut scene);
    renderer.finish_navigation_element_capture(
        source,
        id,
        transformed_rect(ctx.transform, ctx.bounds),
        scene,
    );
}

impl HydrolysisRenderer {
    /// Render an already-laid-out child into an effect input texture in local
    /// coordinates. The complete painter stream is isolated, including embedded
    /// GPU surfaces, rather than capturing only the Vello scene.
    pub(crate) fn render_child_node_to_texture(
        &mut self,
        child: &RenderNode,
        ctx: RenderContext,
        env: &Environment,
        target: ChildTextureTarget<'_>,
    ) {
        let adapter = self.state().frame_adapter().clone();
        let (device, queue) = {
            let (device, queue) = self.state().frame_resources();
            (device.clone(), queue.clone())
        };
        let device_loss = self.state().frame_device_loss().clone();
        let parent_scene = core::mem::take(&mut self.scene);
        let parent_render_layers = core::mem::take(&mut self.compositor.render_layers);
        let parent_active_layers = core::mem::take(&mut self.compositor.active_scene_layers);
        let parent_transient_scene = self.transient_scene.take();
        // The captured subtree is flushed under identity transforms into a
        // pixel-sized texture, so its viewport is that texture and its root
        // transform is the identity — not the window's.
        let parent_window_bounds = self.window_bounds;
        let parent_window_root_transform = self.window_root_transform;
        self.set_window_viewport(
            vello::kurbo::Rect::new(0.0, 0.0, f64::from(target.width), f64::from(target.height)),
            vello::kurbo::Affine::IDENTITY,
        );

        let local_ctx = ctx.with_identity_transforms(vello::kurbo::Rect::new(
            0.0,
            0.0,
            f64::from(target.width),
            f64::from(target.height),
        ));
        // Filters inside this subtree are captured one level deeper and flushed
        // here, so their outputs exist before the subtree itself is rendered.
        let depth = self.subtree_captures.depth;
        self.subtree_captures.depth = depth + 1;
        child.flush(self, local_ctx, env);
        self.subtree_captures.depth = depth;
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis GPU subtree capture left an unclosed scene layer"
        );
        self.flush_subtree_captures(depth + 1);
        self.render_scene_to_texture(HydrolysisRenderTarget {
            adapter: &adapter,
            device: &device,
            queue: &queue,
            device_loss,
            texture: Some(target.texture),
            view: target.view,
            format: target.format,
            width: target.width,
            height: target.height,
            base_color: vello::peniko::Color::TRANSPARENT,
        });
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis GPU subtree compositor restored an active scene layer"
        );

        self.scene = parent_scene;
        self.compositor.render_layers = parent_render_layers;
        self.compositor.active_scene_layers = parent_active_layers;
        self.transient_scene = parent_transient_scene;
        self.set_window_viewport(parent_window_bounds, parent_window_root_transform);
    }
}
