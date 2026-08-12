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
                // Read the content/alignment signals through `read_signal` so a change
                // re-subscribes this frame and schedules a window refresh — the same
                // cheap pump every other reactive leaf uses. (A bare reactive `Text`
                // has no surrounding widget to watch it, so the node must do so itself.)
                let styled = renderer.read_signal(&text.content);
                let alignment = renderer.read_signal(&text.alignment);
                text.emit_accessibility(renderer, ctx, &styled, env);
                let (state, scene) = renderer.state_and_scene_mut();
                HydrolysisRenderer::render_styled_text(state, scene, ctx, styled, alignment, env);
            }
            RenderNode::Container(container) => {
                #[cfg(feature = "accessibility")]
                let group_scope = container.accessibility_child_env.as_ref().map(|_| {
                    renderer.begin_accessibility_group(
                        transformed_rect(ctx.hit_transform, ctx.bounds),
                        env,
                    )
                });
                #[cfg(feature = "accessibility")]
                let child_env = container.accessibility_child_env.as_ref().unwrap_or(env);
                #[cfg(not(feature = "accessibility"))]
                let child_env = env;
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
                if let Some(group_scope) = group_scope {
                    renderer.end_accessibility_group(group_scope);
                }
            }
            RenderNode::Opacity(node) => {
                let alpha = renderer.resolve_animated_scalar_with_discriminator(
                    &node.value.value,
                    OPACITY_ANIMATION_KEY,
                );
                renderer.push_layer_rect(alpha, ctx.transform, ctx.bounds);
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
            RenderNode::Dynamic(node) => node.child.flush(renderer, ctx, env),
            RenderNode::Retain(node) => node.child.flush(renderer, ctx, env),
            RenderNode::Env(node) => node.child.flush(renderer, ctx, &node.env),
            RenderNode::Wrapper(node) => {
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
                        HydrolysisRenderer::apply_context_menu(renderer, ctx, value, |r| {
                            node.child.flush(r, ctx, child_env);
                        });
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
                    WrapperEffect::GestureObserver(effect) => {
                        HydrolysisRenderer::apply_gesture_observer(
                            renderer,
                            ctx,
                            child_env,
                            effect,
                            |r| {
                                node.child.flush(r, ctx, child_env);
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
            }
            RenderNode::SceneView(node) => {
                emit_graphics_image_accessibility(renderer, ctx, env);
                let mut scene = vello::Scene::new();
                // Scope `scene2d` so its `&mut scene` borrow ends before `&scene` is
                // appended below.
                let needs_next = {
                    let mut scene2d = VelloScene2D::new(&mut scene);
                    #[allow(clippy::cast_possible_truncation)]
                    node.content.borrow_mut().build_scene(
                        &mut scene2d,
                        ctx.bounds.width() as f32,
                        ctx.bounds.height() as f32,
                    )
                };
                renderer.scene_mut().append(
                    &scene,
                    Some(
                        ctx.transform
                            * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0)),
                    ),
                );
                if needs_next {
                    renderer.request_refresh();
                }
            }
            RenderNode::GpuSurface(node) => {
                emit_graphics_image_accessibility(renderer, ctx, env);
                node.flush(renderer, ctx);
            }
            RenderNode::ViewEffect(node) => node.flush(renderer, ctx),
            RenderNode::AppliedFilter(node) => node.flush(renderer, ctx),
            RenderNode::Scroll(node) => {
                let Some(handle) = node.handle.clone() else {
                    return;
                };
                let metrics = handle.metrics();
                let viewport_rect = vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(node.viewport.width),
                    f64::from(node.viewport.height),
                );
                renderer.push_layer_rect(1.0, ctx.transform, viewport_rect);
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
                renderer.push_lazy_viewport(lazy_viewport);
                node.child.flush(renderer, content_ctx, env);
                renderer.pop_lazy_viewport("hydrolysis render tree ScrollNode");
                renderer.pop_layer();
                let target_handle = handle.clone();
                renderer.register_scroll_target(
                    transformed_rect(ctx.hit_transform, viewport_rect),
                    handle.clone(),
                    move |dx, dy, is_line_delta| {
                        target_handle.apply_scroll_delta(dx, dy, is_line_delta)
                    },
                );
                #[cfg(feature = "accessibility")]
                crate::widgets::scroll::register_scroll_accessibility_node(
                    renderer,
                    &node.env,
                    transformed_rect(ctx.hit_transform, viewport_rect),
                    &handle,
                    metrics,
                    node.axis,
                );
                let scroll_ctx =
                    RenderContext::with_transforms(viewport_rect, ctx.transform, ctx.hit_transform);
                let mut widget_ctx = WidgetRenderContext::new(renderer, scroll_ctx);
                crate::widgets::draw_scroll_indicators(
                    &mut widget_ctx,
                    &node.env,
                    viewport_rect,
                    metrics,
                    node.axis,
                );
            }
            RenderNode::LazyStack(node) => node.flush(renderer, ctx, env),
            RenderNode::Collection(node) => node.flush(renderer, ctx),
            RenderNode::Widget(node) => {
                // Re-render the leaf widget from its retained config so its handler
                // re-reads live signals and re-emits interaction targets + a11y at the
                // current bounds. A leaf render starts a fresh recursion depth.
                renderer.render_depth = 0;
                Rc::clone(&node.behavior).render(renderer, ctx, &node.env);
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
        let parent_scene = core::mem::take(&mut self.scene);
        let parent_render_layers = core::mem::take(&mut self.compositor.render_layers);
        let parent_active_layers = core::mem::take(&mut self.compositor.active_scene_layers);
        let parent_transient_scene = self.transient_scene.take();
        let parent_window_bounds = core::mem::replace(
            &mut self.window_bounds,
            vello::kurbo::Rect::new(0.0, 0.0, f64::from(target.width), f64::from(target.height)),
        );

        let local_ctx = ctx.with_identity_transforms(vello::kurbo::Rect::new(
            0.0,
            0.0,
            f64::from(target.width),
            f64::from(target.height),
        ));
        child.flush(self, local_ctx, env);
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis GPU subtree capture left an unclosed scene layer"
        );
        self.render_scene_to_texture(HydrolysisRenderTarget {
            adapter: &adapter,
            device: &device,
            queue: &queue,
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
        self.window_bounds = parent_window_bounds;
    }
}
