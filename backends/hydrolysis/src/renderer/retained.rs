//! The retained scene: replayable dynamic draws (transform/opacity/morph),
//! `Dynamic` node placements and reactive patching, scroll content caches,
//! and window-frame capture/replay.

use super::*;

/// A retained snapshot of the entire window content captured during a structural
/// rebuild. Parametric frames (animation ticks, scroll offset changes) re-render by
/// replaying this subtree — re-sampling animated transforms/morphs and applying current
/// scroll offsets at the new frame instant — instead of re-walking and re-measuring the
/// WaterUI view tree.
///
/// The subtree is captured in real (already-DPI-scaled) coordinates, so it replays
/// under an identity context. Scrolling is subsumed into it via [`DynamicScrollDraw`].
pub(crate) struct RetainedWindowFrame {
    pub(super) subtree: DynamicSubtree,
    /// The static root transform (device scale factor) used for the background fill.
    pub(super) transform: vello::kurbo::Affine,
    pub(super) bounds: vello::kurbo::Rect,
    pub(super) active_layers: Vec<ActiveSceneLayer>,
    pub(super) content_morphs: Vec<DynamicMorphDraw>,
    /// Whether this frame can be re-rendered by pure replay. False when the content
    /// baked an animated non-transform value (e.g. opacity), bound a GPU surface, or
    /// used an applied filter — those cannot be reproduced without a real dispatch, so
    /// such frames fall back to a structural rebuild.
    pub(super) drivable: bool,
}

pub(crate) struct ScrollContentCache {
    pub(super) lazy_viewport: vello::kurbo::Rect,
    pub(super) viewport_dependent: bool,
    pub(super) animation_dependent: bool,
    pub(super) subtree: DynamicSubtree,
    pub(super) active_filters: Vec<ActiveAppliedFilter>,
    pub(super) dynamic_morphs: Vec<DynamicMorphDraw>,
}

#[derive(Clone)]
pub(crate) struct DynamicMorphDraw {
    pub(super) shape: ResolvedMorphShape,
    pub(super) bounds: vello::kurbo::Rect,
    pub(super) transform: vello::kurbo::Affine,
    pub(super) started_at: Instant,
}

pub(crate) struct DynamicTransformScalar {
    value: f32,
    handle: Option<AnimatedScalarHandle>,
}

pub(crate) struct DynamicScaleTransform {
    x: DynamicTransformScalar,
    y: DynamicTransformScalar,
    center: vello::kurbo::Point,
}

pub(crate) struct DynamicRotationTransform {
    angle: DynamicTransformScalar,
    center: vello::kurbo::Point,
}

pub(crate) struct DynamicOffsetTransform {
    x: DynamicTransformScalar,
    y: DynamicTransformScalar,
}

pub(crate) struct DynamicTransformComponents {
    scale: Option<DynamicScaleTransform>,
    rotation: Option<DynamicRotationTransform>,
    offset: Option<DynamicOffsetTransform>,
}

pub(crate) struct DynamicTransformDraw {
    transform: DynamicTransformComponents,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    subtree: DynamicSubtree,
}

impl DynamicTransformScalar {
    pub(super) fn sample(&self, now: Instant) -> f32 {
        self.handle
            .as_ref()
            .map_or(self.value, |handle| handle.sample(now))
    }

    pub(super) fn collect_active_key(&self, keys: &mut BTreeSet<AnimationKey>) {
        if let Some(handle) = &self.handle
            && handle.is_active()
        {
            keys.insert(handle.key());
        }
    }
}

impl DynamicTransformComponents {
    pub(super) fn scale(
        x: DynamicTransformScalar,
        y: DynamicTransformScalar,
        center: vello::kurbo::Point,
    ) -> Self {
        Self {
            scale: Some(DynamicScaleTransform { x, y, center }),
            rotation: None,
            offset: None,
        }
    }

    pub(super) fn rotation(angle: DynamicTransformScalar, center: vello::kurbo::Point) -> Self {
        Self {
            scale: None,
            rotation: Some(DynamicRotationTransform { angle, center }),
            offset: None,
        }
    }

    pub(super) fn offset(x: DynamicTransformScalar, y: DynamicTransformScalar) -> Self {
        Self {
            scale: None,
            rotation: None,
            offset: Some(DynamicOffsetTransform { x, y }),
        }
    }

    pub(super) fn affine(&self, now: Instant) -> vello::kurbo::Affine {
        let active_components = usize::from(self.scale.is_some())
            + usize::from(self.rotation.is_some())
            + usize::from(self.offset.is_some());
        assert!(
            active_components == 1,
            "hydrolysis dynamic transform draw must contain exactly one transform component"
        );
        if let Some(scale) = &self.scale {
            return vello::kurbo::Affine::translate((scale.center.x, scale.center.y))
                * vello::kurbo::Affine::scale_non_uniform(
                    f64::from(scale.x.sample(now)),
                    f64::from(scale.y.sample(now)),
                )
                * vello::kurbo::Affine::translate((-scale.center.x, -scale.center.y));
        }
        if let Some(rotation) = &self.rotation {
            let radians = f64::from(rotation.angle.sample(now)).to_radians();
            return vello::kurbo::Affine::translate((rotation.center.x, rotation.center.y))
                * vello::kurbo::Affine::rotate(radians)
                * vello::kurbo::Affine::translate((-rotation.center.x, -rotation.center.y));
        }
        let offset = self
            .offset
            .as_ref()
            .expect("hydrolysis dynamic transform draw missing offset component");
        vello::kurbo::Affine::translate((
            f64::from(offset.x.sample(now)),
            f64::from(offset.y.sample(now)),
        ))
    }

    pub(super) fn collect_active_scalar_keys(&self, keys: &mut BTreeSet<AnimationKey>) {
        if let Some(scale) = &self.scale {
            scale.x.collect_active_key(keys);
            scale.y.collect_active_key(keys);
        }
        if let Some(rotation) = &self.rotation {
            rotation.angle.collect_active_key(keys);
        }
        if let Some(offset) = &self.offset {
            offset.x.collect_active_key(keys);
            offset.y.collect_active_key(keys);
        }
    }
}

/// A replayable opacity layer captured during a dynamic subtree capture. Its alpha is
/// re-sampled at replay time so animated opacity re-renders without re-dispatching the
/// wrapped content, the opacity counterpart of [`DynamicTransformDraw`].
pub(crate) struct DynamicOpacityDraw {
    alpha: DynamicTransformScalar,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    subtree: DynamicSubtree,
}

/// A placement of a `Dynamic` node within a captured subtree. The node's content is
/// not baked into the parent scene; instead it is composited from the node's own
/// retained `cached_subtree` (keyed by `identity` in `lifecycle.dynamic_nodes`) at
/// replay time. This is what makes fine-grained reactive patching possible: when one
/// `Dynamic` node's content changes, only that node is re-dispatched and the window is
/// re-composited from the unchanged placements of every other node.
pub(crate) struct DynamicNodeDraw {
    identity: usize,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
}

/// A placement of a scroll view within a captured subtree. Its content is captured once
/// (offset-independently) into `scroll_content_caches[cache_key]`; the current scroll
/// offset is applied at replay, so scrolling re-composites the window frame without
/// re-dispatching the view tree. This subsumes the former standalone retained-scroll
/// fast-path into the single window-frame retention path. Lazy (viewport-dependent)
/// content that scrolls beyond its captured window escalates to a structural rebuild.
pub(crate) struct DynamicScrollDraw {
    pub(super) handle: crate::scroll::ScrollHandle,
    pub(super) cache_key: usize,
    pub(super) axis: ScrollAxis,
    pub(super) viewport: vello::kurbo::Rect,
    pub(super) content_width: f64,
    pub(super) content_height: f64,
    pub(super) base_transform: vello::kurbo::Affine,
    pub(super) base_hit_transform: vello::kurbo::Affine,
    pub(super) content_morphs: Vec<DynamicMorphDraw>,
    pub(super) needs_viewport_clip: bool,
    pub(super) env: Environment,
}

impl DynamicScrollDraw {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        handle: crate::scroll::ScrollHandle,
        cache_key: usize,
        axis: ScrollAxis,
        viewport: vello::kurbo::Rect,
        content_width: f64,
        content_height: f64,
        base_transform: vello::kurbo::Affine,
        base_hit_transform: vello::kurbo::Affine,
        content_morphs: Vec<DynamicMorphDraw>,
        needs_viewport_clip: bool,
        env: Environment,
    ) -> Self {
        Self {
            handle,
            cache_key,
            axis,
            viewport,
            content_width,
            content_height,
            base_transform,
            base_hit_transform,
            content_morphs,
            needs_viewport_clip,
            env,
        }
    }
}

pub(crate) struct ScrollContentRender {
    pub(crate) dynamic_morphs: Vec<DynamicMorphDraw>,
}

pub(crate) fn affine_near(left: vello::kurbo::Affine, right: vello::kurbo::Affine) -> bool {
    left.as_coeffs()
        .iter()
        .zip(right.as_coeffs())
        .all(|(left, right)| (*left - right).abs() <= 0.001)
}

pub(crate) fn rect_near(left: vello::kurbo::Rect, right: vello::kurbo::Rect) -> bool {
    (left.x0 - right.x0).abs() <= 0.001
        && (left.y0 - right.y0).abs() <= 0.001
        && (left.x1 - right.x1).abs() <= 0.001
        && (left.y1 - right.y1).abs() <= 0.001
}

impl HydrolysisRenderer {
    pub(super) fn resolve_animated_scalar_with_discriminator<S>(
        &mut self,
        signal: &S,
        discriminator: usize,
    ) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return signal.get();
        };
        self.mark_scroll_content_animation_dependent();
        let now = self.frame_instant;
        let key = AnimationKey::scalar_with_discriminator(identity, discriminator);
        let handle = self
            .animation_controller
            .bind_scalar(key, signal.get(), now);
        let watcher_handle = handle.clone();
        let signals = self.signals.clone();
        let guard = signal.watch(move |update| {
            watcher_handle.apply_update_from_context(update, signals.frame_clock());
            signals.request_redraw();
        });
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        handle.sample(now)
    }

    pub(super) fn dynamic_transform_scalar_with_discriminator<S>(
        &mut self,
        signal: &S,
        discriminator: usize,
    ) -> DynamicTransformScalar
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return DynamicTransformScalar {
                value: signal.get(),
                handle: None,
            };
        };
        let now = self.frame_instant;
        let key = AnimationKey::scalar_with_discriminator(identity, discriminator);
        let handle = self
            .animation_controller
            .bind_scalar(key, signal.get(), now);
        let watcher_handle = handle.clone();
        let signals = self.signals.clone();
        let guard = signal.watch(move |update| {
            watcher_handle.apply_update_from_context(update, signals.frame_clock());
            signals.request_redraw();
        });
        let value = handle.sample(now);
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        DynamicTransformScalar {
            value,
            handle: Some(handle),
        }
    }

    pub(super) fn capture_dynamic_transform(
        &mut self,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
        transform: DynamicTransformComponents,
    ) {
        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.dynamic_transform_draws.push(DynamicTransformDraw {
            transform,
            base_transform: ctx.transform,
            base_hit_transform: ctx.hit_transform,
            bounds: ctx.bounds,
            subtree,
        });
    }

    pub(super) fn capture_dynamic_opacity(
        &mut self,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
        alpha: DynamicTransformScalar,
    ) {
        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.dynamic_opacity_draws.push(DynamicOpacityDraw {
            alpha,
            base_transform: ctx.transform,
            base_hit_transform: ctx.hit_transform,
            bounds: ctx.bounds,
            subtree,
        });
    }

    pub(crate) fn sample_morph_progress(
        &mut self,
        animation: waterui_shape::MorphAnimation,
    ) -> f32 {
        if animation.duration.is_zero() {
            return 1.0;
        }
        let key = AnimationKey::renderer_local_repeating(self.render_depth);
        let elapsed = self.animation_controller.bind_timeline_phase(
            key,
            animation.duration,
            animation.repeat,
            self.frame_instant,
        );
        let raw = elapsed.as_secs_f32() / animation.duration.as_secs_f32();
        let cycle = if animation.repeat {
            let base = raw.fract();
            assert!(
                raw.is_finite() && raw >= 0.0,
                "morph animation cycle index must be finite and non-negative"
            );
            let index = raw.floor() as u64;
            if animation.autoreverse && index % 2 == 1 {
                1.0 - base
            } else {
                base
            }
        } else {
            raw.clamp(0.0, 1.0)
        };
        animation.easing.ease(cycle).clamp(0.0, 1.0)
    }

    pub(super) fn sample_morph_draw_progress(&self, draw: &DynamicMorphDraw) -> f32 {
        let animation = draw.shape.animation;
        if animation.duration.is_zero() {
            return 1.0;
        }
        let elapsed = self
            .frame_instant
            .saturating_duration_since(draw.started_at);
        let raw = elapsed.as_secs_f32() / animation.duration.as_secs_f32();
        let cycle = if animation.repeat {
            let base = raw.fract();
            assert!(
                raw.is_finite() && raw >= 0.0,
                "morph animation cycle index must be finite and non-negative"
            );
            let index = raw.floor() as u64;
            if animation.autoreverse && index % 2 == 1 {
                1.0 - base
            } else {
                base
            }
        } else {
            raw.clamp(0.0, 1.0)
        };
        animation.easing.ease(cycle).clamp(0.0, 1.0)
    }

    pub(super) fn dynamic_morph_is_active(&self, draw: &DynamicMorphDraw) -> bool {
        let animation = draw.shape.animation;
        animation.repeat
            || self
                .frame_instant
                .saturating_duration_since(draw.started_at)
                < animation.duration
    }

    pub(super) fn draw_dynamic_morphs(
        &mut self,
        morphs: &[DynamicMorphDraw],
        parent_transform: vello::kurbo::Affine,
    ) {
        for morph in morphs {
            let progress = self.sample_morph_draw_progress(morph);
            let path = resolved_morph_shape_to_path(&morph.shape, progress, morph.bounds);
            let fill = resolved_color_to_peniko(morph.shape.fill);
            self.scene.fill(
                vello::peniko::Fill::NonZero,
                parent_transform * morph.transform,
                fill,
                None,
                &path,
            );
        }
    }

    pub(super) fn draw_dynamic_transforms(
        &mut self,
        parent_ctx: RenderContext,
        transforms: &[DynamicTransformDraw],
    ) {
        for draw in transforms {
            let dynamic_transform = draw.transform.affine(self.frame_instant);
            let ctx = RenderContext::with_transforms(
                draw.bounds,
                parent_ctx.transform * draw.base_transform * dynamic_transform,
                parent_ctx.hit_transform * draw.base_hit_transform * dynamic_transform,
            );
            self.replay_dynamic_subtree(ctx, &draw.subtree);
        }
    }

    pub(super) fn draw_dynamic_opacities(
        &mut self,
        parent_ctx: RenderContext,
        opacities: &[DynamicOpacityDraw],
    ) {
        for draw in opacities {
            let alpha = draw.alpha.sample(self.frame_instant).clamp(0.0, 1.0);
            let transform = parent_ctx.transform * draw.base_transform;
            let hit_transform = parent_ctx.hit_transform * draw.base_hit_transform;
            self.push_layer_rect(alpha, transform, draw.bounds);
            let previous_opacity = self.hit_test.hit_test_opacity;
            self.hit_test.hit_test_opacity = previous_opacity * alpha;
            let ctx = RenderContext::with_transforms(draw.bounds, transform, hit_transform);
            self.replay_dynamic_subtree(ctx, &draw.subtree);
            self.hit_test.hit_test_opacity = previous_opacity;
            self.pop_layer();
        }
    }

    /// Composites each placed `Dynamic` node from its retained `cached_subtree`. The
    /// subtree is taken out, replayed, and returned, so a content change to one node
    /// (which only refreshes that node's `cached_subtree`) is picked up here without
    /// touching any other node's placement.
    pub(super) fn replay_dynamic_node_placements(
        &mut self,
        parent_ctx: RenderContext,
        placements: &[DynamicNodeDraw],
    ) {
        for placement in placements {
            let Some(subtree) = self
                .lifecycle
                .dynamic_nodes
                .get_mut(&placement.identity)
                .and_then(|node| node.cached_subtree.take())
            else {
                continue;
            };
            let ctx = RenderContext::with_transforms(
                placement.bounds,
                parent_ctx.transform * placement.base_transform,
                parent_ctx.hit_transform * placement.base_hit_transform,
            );
            self.replay_dynamic_subtree(ctx, &subtree);
            self.lifecycle
                .dynamic_nodes
                .get_mut(&placement.identity)
                .expect("hydrolysis dynamic node missing after placement replay")
                .cached_subtree = Some(subtree);
        }
    }

    /// Collects the animation keys of every active replayable scalar (transform and
    /// opacity) reachable from `subtree`, recursing through nested dynamic draws and
    /// through placed `Dynamic` nodes (whose content lives in their `cached_subtree`).
    pub(super) fn collect_subtree_active_scalar_keys(
        &self,
        subtree: &DynamicSubtree,
        keys: &mut BTreeSet<AnimationKey>,
    ) {
        for transform in &subtree.dynamic_transforms {
            transform.transform.collect_active_scalar_keys(keys);
            self.collect_subtree_active_scalar_keys(&transform.subtree, keys);
        }
        for opacity in &subtree.dynamic_opacities {
            opacity.alpha.collect_active_key(keys);
            self.collect_subtree_active_scalar_keys(&opacity.subtree, keys);
        }
        for placement in &subtree.dynamic_node_draws {
            if let Some(cached) = self
                .lifecycle
                .dynamic_nodes
                .get(&placement.identity)
                .and_then(|node| node.cached_subtree.as_ref())
            {
                self.collect_subtree_active_scalar_keys(cached, keys);
            }
        }
        for scroll in &subtree.dynamic_scroll_draws {
            if let Some(cache) = self.scroll_content_caches.get(&scroll.cache_key) {
                self.collect_subtree_active_scalar_keys(&cache.subtree, keys);
            }
        }
    }

    /// Re-dispatches a `Dynamic` node's content into its `cached_subtree`, refreshing the
    /// intrinsic/proposal dimension caches. If the content's intrinsic size changed, the
    /// surrounding layout must reflow, so this escalates to a full structural rebuild.
    pub(super) fn capture_dynamic_node_content(
        &mut self,
        identity: usize,
        content: AnyView,
        ctx: RenderContext,
        env: &Environment,
    ) {
        let content = normalize_layout_view(content, env);
        let dimensions = measure_view_dimensions(&content, &mut self.state, env);
        let proposal = ProposalSize::new(
            Some(ctx.bounds.width() as f32),
            Some(ctx.bounds.height() as f32),
        );
        let proposal_dimensions =
            measure_view_dimensions_with_proposal(&content, proposal, &mut self.state, env);
        let previous_dimensions = self.state.measurement.dynamic_intrinsic(identity);
        self.state.measurement.store_dynamic_dimensions(
            identity,
            ProposalSize::UNSPECIFIED,
            dimensions.clone(),
        );
        self.state
            .measurement
            .store_dynamic_dimensions(identity, proposal, proposal_dimensions);
        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.lifecycle
            .dynamic_nodes
            .get_mut(&identity)
            .expect("hydrolysis dynamic node missing after connect")
            .cached_subtree = Some(subtree);
        if previous_dimensions.is_some() && previous_dimensions.as_ref() != Some(&dimensions) {
            self.request_rebuild();
        }
    }

    /// Re-dispatches every dirty `Dynamic` node in isolation, refreshing only those
    /// nodes' cached subtrees. Returns `false` if any patch reflowed layout (escalating
    /// to a structural rebuild), in which case the caller must rebuild instead of
    /// compositing a patched frame.
    pub(super) fn patch_dirty_dynamic_nodes(&mut self) -> bool {
        let dirty = self.signals.take_dirty_dynamic_nodes();
        for identity in dirty {
            let Some((pending_view, ctx, env)) = self
                .lifecycle
                .dynamic_nodes
                .get(&identity)
                .and_then(|node| {
                    Some((
                        Rc::clone(&node.pending_view),
                        node.dispatch_ctx?,
                        node.dispatch_env.clone()?,
                    ))
                })
            else {
                continue;
            };
            let Some(content) = pending_view.borrow_mut().take() else {
                continue;
            };
            // Re-dispatch under a retained capture so nested dynamic draws and Dynamic
            // node placements inside the patched content are captured, not baked.
            self.dynamic_transform_capture_depth = self
                .dynamic_transform_capture_depth
                .checked_add(1)
                .expect("hydrolysis reactive patch transform capture depth overflow");
            self.dynamic_morph_capture_depth = self
                .dynamic_morph_capture_depth
                .checked_add(1)
                .expect("hydrolysis reactive patch morph capture depth overflow");
            self.scroll_content_capture_depth = self
                .scroll_content_capture_depth
                .checked_add(1)
                .expect("hydrolysis reactive patch scroll capture depth overflow");
            self.capture_dynamic_node_content(identity, content, ctx, &env);
            self.scroll_content_capture_depth = self
                .scroll_content_capture_depth
                .checked_sub(1)
                .expect("hydrolysis reactive patch scroll capture depth underflow");
            self.dynamic_morph_capture_depth = self
                .dynamic_morph_capture_depth
                .checked_sub(1)
                .expect("hydrolysis reactive patch morph capture depth underflow");
            self.dynamic_transform_capture_depth = self
                .dynamic_transform_capture_depth
                .checked_sub(1)
                .expect("hydrolysis reactive patch transform capture depth underflow");
            if self.signals.has_rebuild_request() {
                return false;
            }
        }
        true
    }

    pub(crate) fn render_dynamic(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        dynamic: Native<Dynamic>,
        env: &Environment,
    ) {
        let dynamic = dynamic.into_inner();
        let identity = dynamic.identity();
        renderer
            .lifecycle
            .dynamic_identities_current_frame
            .push(identity);
        let pending_view = {
            if let Some(node) = renderer.lifecycle.dynamic_nodes.get(&identity) {
                Rc::clone(&node.pending_view)
            } else {
                let pending_view = Rc::new(RefCell::new(None::<AnyView>));
                let render_generation = Rc::new(Cell::new(0));
                dynamic.connect_with_pending_view(Rc::clone(&pending_view), {
                    let pending_view = Rc::clone(&pending_view);
                    let signals = renderer.signals.clone();
                    let render_generation = Rc::clone(&render_generation);
                    move |update| {
                        let is_initial_content = update
                            .metadata()
                            .try_get::<DynamicInitialContent>()
                            .is_some();
                        if is_initial_content
                            && signals
                                .initial_dynamic_content_already_rendered(render_generation.get())
                        {
                            return;
                        }
                        *pending_view.borrow_mut() = Some(update.into_value());
                        // A real content change is a fine-grained reactive update: mark
                        // this node dirty so it can be re-dispatched in isolation. If the
                        // re-dispatch reflows layout, render_dynamic escalates to a full
                        // rebuild itself.
                        if !is_initial_content {
                            signals.mark_dynamic_dirty(identity, render_generation.get());
                        }
                    }
                });
                renderer.lifecycle.dynamic_nodes.insert(
                    identity,
                    DynamicNode {
                        pending_view: Rc::clone(&pending_view),
                        cached_subtree: None,
                        render_generation,
                        dispatch_ctx: None,
                        dispatch_env: None,
                    },
                );
                pending_view
            }
        };
        let current_generation = renderer.signals.rebuild_generation();
        renderer
            .lifecycle
            .dynamic_nodes
            .get(&identity)
            .expect("hydrolysis dynamic node missing before render")
            .render_generation
            .set(current_generation);

        let update = pending_view.borrow_mut().take();
        if let Some(content) = update {
            renderer.capture_dynamic_node_content(identity, content, ctx, env);
        }
        if renderer
            .lifecycle
            .dynamic_nodes
            .get(&identity)
            .is_some_and(|node| node.cached_subtree.is_none())
        {
            let local_ctx = ctx.with_identity_transforms(ctx.bounds);
            let subtree = Self::render_dynamic_subtree_with_local_interactions(
                renderer,
                ctx,
                local_ctx,
                env,
                AnyView::new(()),
            );
            renderer
                .lifecycle
                .dynamic_nodes
                .get_mut(&identity)
                .expect("hydrolysis dynamic node missing after empty subtree initialization")
                .cached_subtree = Some(subtree);
        }

        // Remember where and with what environment this node was dispatched, so a later
        // content change can re-dispatch just this node in isolation (reactive patch).
        if let Some(node) = renderer.lifecycle.dynamic_nodes.get_mut(&identity) {
            node.dispatch_ctx = Some(ctx);
            node.dispatch_env = Some(env.clone());
        }

        // Inside a retained capture, record a placement instead of baking the node's
        // content into the parent scene. The content stays in `cached_subtree` and is
        // composited at replay, so a later content change to this node can be patched
        // in isolation without re-walking the rest of the window.
        if renderer.dynamic_transform_capture_depth > 0 {
            renderer.dynamic_node_draws.push(DynamicNodeDraw {
                identity,
                base_transform: ctx.transform,
                base_hit_transform: ctx.hit_transform,
                bounds: ctx.bounds,
            });
            return;
        }

        let subtree = renderer
            .lifecycle
            .dynamic_nodes
            .get_mut(&identity)
            .and_then(|node| node.cached_subtree.take())
            .expect("hydrolysis Dynamic must provide an initial view before dispatch");
        renderer.replay_dynamic_subtree(ctx, &subtree);
        renderer
            .lifecycle
            .dynamic_nodes
            .get_mut(&identity)
            .expect("hydrolysis dynamic node missing after replay")
            .cached_subtree = Some(subtree);
    }

    pub(crate) fn invalidate_retained_scroll_content(&mut self) {
        self.scroll_content_caches.clear();
        self.retained_window_frame = None;
    }

    /// Whether the retained window frame has any active (repeating or in-flight) dynamic
    /// morph — at the window root or inside a scroll draw — so the runner keeps issuing
    /// parametric refreshes to advance the morph animation.
    pub(crate) fn window_dynamic_morphs_active(&self) -> bool {
        let Some(frame) = &self.retained_window_frame else {
            return false;
        };
        if frame
            .content_morphs
            .iter()
            .any(|draw| self.dynamic_morph_is_active(draw))
        {
            return true;
        }
        self.subtree_scroll_morphs_active(&frame.subtree)
    }

    pub(super) fn subtree_scroll_morphs_active(&self, subtree: &DynamicSubtree) -> bool {
        subtree.dynamic_scroll_draws.iter().any(|draw| {
            draw.content_morphs
                .iter()
                .any(|morph| self.dynamic_morph_is_active(morph))
        })
    }

    pub(crate) fn active_scene_layers_snapshot(&self) -> Vec<ActiveSceneLayer> {
        self.compositor.active_scene_layers.clone()
    }

    pub(crate) fn scene_is_empty(&self) -> bool {
        self.scene.encoding().is_empty()
    }

    pub(crate) fn viewport_matches_window_bounds(&self, viewport: vello::kurbo::Rect) -> bool {
        (viewport.x0 - self.window_bounds.x0).abs() <= f64::EPSILON
            && (viewport.y0 - self.window_bounds.y0).abs() <= f64::EPSILON
            && (viewport.x1 - self.window_bounds.x1).abs() <= f64::EPSILON
            && (viewport.y1 - self.window_bounds.y1).abs() <= f64::EPSILON
    }

    pub(crate) fn push_dynamic_scroll_draw(&mut self, draw: DynamicScrollDraw) {
        self.dynamic_scroll_draws.push(draw);
    }

    /// Composites each scroll view from its retained offset-independent content cache,
    /// applying the current scroll offset, viewport clip, content morphs, scroll target,
    /// accessibility node, and indicators. This is the per-frame body of the former
    /// `refresh_retained_scroll_scene`, generalized to run inside the window-frame replay
    /// for any number of (possibly nested) scroll views.
    pub(super) fn replay_dynamic_scroll_draws(
        &mut self,
        parent_ctx: RenderContext,
        draws: &[DynamicScrollDraw],
    ) {
        for draw in draws {
            let transform = parent_ctx.transform * draw.base_transform;
            let hit_transform = parent_ctx.hit_transform * draw.base_hit_transform;
            if draw.needs_viewport_clip {
                self.record_clip_layer_push();
                self.scene.push_layer(
                    vello::peniko::Fill::NonZero,
                    vello::peniko::BlendMode::default(),
                    1.0,
                    transform,
                    &draw.viewport,
                );
                self.compositor.active_scene_layers.push(ActiveSceneLayer {
                    alpha: 1.0,
                    transform,
                    shape: LayerShape::Rect(draw.viewport),
                });
            }
            let metrics = draw.handle.metrics();
            let scroll_content_transform =
                vello::kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
            let content_transform = transform * scroll_content_transform;
            let content_hit_transform = hit_transform * scroll_content_transform;
            let content_bounds =
                vello::kurbo::Rect::new(0.0, 0.0, draw.content_width, draw.content_height);
            let content_ctx = RenderContext::with_transforms(
                content_bounds,
                content_transform,
                content_hit_transform,
            );
            if let Some(cache) = self.scroll_content_caches.remove(&draw.cache_key) {
                self.replay_dynamic_subtree(content_ctx, &cache.subtree);
                self.scroll_content_caches.insert(draw.cache_key, cache);
            }
            self.draw_dynamic_morphs(&draw.content_morphs, content_transform);
            if draw.needs_viewport_clip {
                self.pop_layer();
            }
            let target_handle = draw.handle.clone();
            self.register_scroll_target(
                transformed_rect(hit_transform, draw.viewport),
                move |dx, dy, is_line_delta| {
                    target_handle.apply_scroll_delta(dx, dy, is_line_delta)
                },
            );
            crate::widgets::scroll::register_scroll_accessibility_node(
                self,
                &draw.env,
                transformed_rect(hit_transform, draw.viewport),
                &draw.handle,
                metrics,
                draw.axis,
            );
            let scroll_ctx =
                RenderContext::with_transforms(draw.viewport, transform, hit_transform);
            let mut widget_ctx = WidgetRenderContext::new(self, scroll_ctx);
            crate::widgets::draw_scroll_indicators(
                &mut widget_ctx,
                &draw.env,
                draw.viewport,
                metrics,
                draw.axis,
            );
        }
    }

    /// Whether every scroll view reachable from the retained window frame can be
    /// re-composited from its cached content at the current scroll offset. Lazy
    /// (viewport-dependent) content that scrolled beyond its captured window returns
    /// false, forcing a structural rebuild that re-materializes the visible items.
    pub(crate) fn window_scroll_draws_reusable(&self) -> bool {
        match &self.retained_window_frame {
            Some(frame) => self.subtree_scroll_draws_reusable(&frame.subtree),
            None => true,
        }
    }

    pub(super) fn subtree_scroll_draws_reusable(&self, subtree: &DynamicSubtree) -> bool {
        for draw in &subtree.dynamic_scroll_draws {
            let metrics = draw.handle.metrics();
            let lazy_viewport = vello::kurbo::Rect::new(
                metrics.offset_x,
                metrics.offset_y,
                metrics.offset_x + draw.viewport.width(),
                metrics.offset_y + draw.viewport.height(),
            );
            let Some(cache) = self.scroll_content_caches.get(&draw.cache_key) else {
                return false;
            };
            if !self.can_reuse_scroll_content_cache(cache, lazy_viewport)
                || !self.subtree_scroll_draws_reusable(&cache.subtree)
            {
                return false;
            }
        }
        for placement in &subtree.dynamic_node_draws {
            if let Some(cached) = self
                .lifecycle
                .dynamic_nodes
                .get(&placement.identity)
                .and_then(|node| node.cached_subtree.as_ref())
                && !self.subtree_scroll_draws_reusable(cached)
            {
                return false;
            }
        }
        for transform in &subtree.dynamic_transforms {
            if !self.subtree_scroll_draws_reusable(&transform.subtree) {
                return false;
            }
        }
        for opacity in &subtree.dynamic_opacities {
            if !self.subtree_scroll_draws_reusable(&opacity.subtree) {
                return false;
            }
        }
        true
    }

    pub(super) fn mark_scroll_content_viewport_dependent(&mut self) {
        if self.scroll_content_capture_depth > 0 {
            self.scroll_content_viewport_dependent = true;
        }
    }

    pub(super) fn mark_scroll_content_animation_dependent(&mut self) {
        if self.scroll_content_capture_depth > 0 {
            self.scroll_content_animation_dependent = true;
        }
    }

    pub(super) fn can_reuse_scroll_content_cache(
        &self,
        cache: &ScrollContentCache,
        lazy_viewport: vello::kurbo::Rect,
    ) -> bool {
        let viewport_reusable =
            !cache.viewport_dependent || rect_near(cache.lazy_viewport, lazy_viewport);
        let animation_reusable = !cache.animation_dependent || !self.animations_active();
        viewport_reusable && animation_reusable
    }

    pub(crate) fn render_scroll_content(
        &mut self,
        cache_key: usize,
        lazy_viewport: vello::kurbo::Rect,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> ScrollContentRender {
        if self.reuse_scroll_content_caches
            && let Some(cache) = self.scroll_content_caches.remove(&cache_key)
        {
            if self.can_reuse_scroll_content_cache(&cache, lazy_viewport) {
                // Capture-only: the content is composited by the scroll draw at replay,
                // not baked into the parent scene here. Re-register applied filters since
                // no dispatch happened to advance them this frame.
                let dynamic_morphs = cache.dynamic_morphs.clone();
                for active_filter in cache.active_filters.iter().cloned() {
                    self.remember_active_applied_filter_entry(active_filter);
                }
                self.scroll_content_caches.insert(cache_key, cache);
                return ScrollContentRender { dynamic_morphs };
            }
            self.scroll_content_caches.insert(cache_key, cache);
        }

        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let active_filter_start = self.active_applied_filter_cursor;
        let previous_morphs = core::mem::take(&mut self.dynamic_morph_draws);
        let previous_scroll_content_viewport_dependent = self.scroll_content_viewport_dependent;
        let previous_scroll_content_animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = false;
        self.scroll_content_animation_dependent = false;
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_add(1)
            .expect("hydrolysis scroll content capture depth overflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_add(1)
            .expect("hydrolysis dynamic morph capture depth overflow");
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_add(1)
            .expect("hydrolysis dynamic transform capture depth overflow");
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_sub(1)
            .expect("hydrolysis dynamic transform capture depth underflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_sub(1)
            .expect("hydrolysis dynamic morph capture depth underflow");
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_sub(1)
            .expect("hydrolysis scroll content capture depth underflow");
        let viewport_dependent = self.scroll_content_viewport_dependent;
        let animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = previous_scroll_content_viewport_dependent;
        self.scroll_content_animation_dependent = previous_scroll_content_animation_dependent;
        let dynamic_morphs = core::mem::replace(&mut self.dynamic_morph_draws, previous_morphs);
        // Capture-only: do not bake content into the parent scene; the scroll draw
        // composites it from the cache at replay, applying the current scroll offset.
        let active_filters = self.active_applied_filters
            [active_filter_start..self.active_applied_filter_cursor]
            .to_vec();
        self.scroll_content_caches.insert(
            cache_key,
            ScrollContentCache {
                lazy_viewport,
                viewport_dependent,
                animation_dependent,
                subtree,
                active_filters,
                dynamic_morphs: dynamic_morphs.clone(),
            },
        );
        ScrollContentRender { dynamic_morphs }
    }

    /// Dispatches the whole window content while capturing it as a retained,
    /// replayable [`DynamicSubtree`], then renders this frame by replaying that
    /// capture. Animated transforms and morphs are captured as replayable dynamic
    /// draws (not baked), so later animation-only frames can refresh via
    /// [`Self::refresh_window_frame`] without re-walking or re-measuring the view tree.
    ///
    /// The subtree is captured in real (DPI-scaled) coordinates so it replays under an
    /// identity context; this keeps any nested scroll retention working in real space.
    pub fn capture_window_scene<V: View>(
        &mut self,
        view: V,
        env: &Environment,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) {
        self.retained_window_frame = None;
        #[cfg(feature = "accessibility")]
        {
            self.accessibility.root_bounds = transformed_rect(hit_transform, bounds);
        }
        let local_env = self.lifecycle.install_local_state_env(env);
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        self.render_depth = 0;

        let gpu_surface_cursor_start = self.compositor.gpu_surface_cursor;
        let active_filter_start = self.active_applied_filter_cursor;
        let previous_morphs = core::mem::take(&mut self.dynamic_morph_draws);
        let previous_viewport_dependent = self.scroll_content_viewport_dependent;
        let previous_animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = false;
        self.scroll_content_animation_dependent = false;
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_add(1)
            .expect("hydrolysis window scene capture depth overflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_add(1)
            .expect("hydrolysis window morph capture depth overflow");
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_add(1)
            .expect("hydrolysis window transform capture depth overflow");
        let subtree = Self::render_dynamic_subtree(self, ctx, &local_env, AnyView::new(view));
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_sub(1)
            .expect("hydrolysis window transform capture depth underflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_sub(1)
            .expect("hydrolysis window morph capture depth underflow");
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_sub(1)
            .expect("hydrolysis window scene capture depth underflow");
        let animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = previous_viewport_dependent;
        self.scroll_content_animation_dependent = previous_animation_dependent;
        let content_morphs = core::mem::replace(&mut self.dynamic_morph_draws, previous_morphs);

        let used_gpu_surface = self.compositor.gpu_surface_cursor != gpu_surface_cursor_start;
        let used_applied_filter = self.active_applied_filter_cursor != active_filter_start;
        let drivable = !animation_dependent && !used_gpu_surface && !used_applied_filter;
        let active_layers = self.active_scene_layers_snapshot();

        // Replay immediately so this structural frame renders pixels identical to a
        // direct dispatch. Captured in real coordinates, so replay uses identity.
        let replay_ctx = RenderContext::with_transforms(
            bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        self.replay_dynamic_subtree(replay_ctx, &subtree);
        self.draw_dynamic_morphs(&content_morphs, vello::kurbo::Affine::IDENTITY);

        self.retained_window_frame = Some(RetainedWindowFrame {
            subtree,
            transform,
            bounds,
            active_layers,
            content_morphs,
            drivable,
        });
    }

    /// Whether the retained window frame can re-render active animations by pure
    /// replay this frame (no structural rebuild). Mirrors
    /// [`Self::retained_scroll_can_drive_active_animations`] but for non-scroll roots.
    pub(crate) fn retained_window_can_drive_active_animations(&self) -> bool {
        let Some(frame) = &self.retained_window_frame else {
            return false;
        };
        if !frame.drivable {
            return false;
        }
        if self.navigation.slots.iter().any(|slot| {
            slot.transition
                .as_ref()
                .is_some_and(|state| state.is_active(self.frame_instant))
        }) {
            return false;
        }
        if self.animation_controller.has_active_radio_indicator() {
            return false;
        }
        // All animations driving this frame must be captured as replayable dynamic
        // draws (transform/opacity; renderer-local interaction scalars replay too).
        // Any active top-level scalar that is not captured would render stale.
        let mut retained_scalar_keys = BTreeSet::new();
        self.collect_subtree_active_scalar_keys(&frame.subtree, &mut retained_scalar_keys);
        let active_scalar_keys: BTreeSet<_> = self
            .animation_controller
            .active_scalar_keys()
            .into_iter()
            .filter(|key| !key.is_renderer_local_scalar())
            .collect();
        active_scalar_keys
            .iter()
            .all(|key| retained_scalar_keys.contains(key))
    }

    /// Re-renders the retained window frame by replaying its captured subtree at the
    /// current frame instant — re-sampling animated transforms and morphs — without
    /// re-dispatching or re-measuring. Returns `false` when the frame cannot be driven
    /// by replay, in which case the caller must fall back to a structural rebuild.
    pub(crate) fn refresh_window_frame(&mut self, env: &Environment) -> bool {
        if self.retained_window_frame.is_none() {
            return false;
        }
        // Apply any pending fine-grained reactive patches before compositing. If a patch
        // reflowed layout it escalates to a full rebuild, so bail to the rebuild path.
        if !self.patch_dirty_dynamic_nodes() {
            return false;
        }
        // A scroll that moved a lazy (viewport-dependent) list beyond its captured window
        // cannot be re-composited from the cache; escalate to a rebuild that re-materializes.
        if !self.window_scroll_draws_reusable() {
            return false;
        }
        let Some(frame) = self.retained_window_frame.take() else {
            return false;
        };
        // A frame that baked an animated non-transform value can only be replayed safely
        // while no animation is active; otherwise the baked value would be stale.
        if !frame.drivable && self.animations_active() {
            self.retained_window_frame = Some(frame);
            return false;
        }
        self.reset_scene();
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
        let background_color =
            resolved_color_to_peniko(Color::new(theme::color::Background).resolve(env).get());
        self.scene.fill(
            vello::peniko::Fill::NonZero,
            frame.transform,
            background_color,
            None,
            &self.window_bounds,
        );
        for layer in &frame.active_layers {
            layer.push_to_scene(&mut self.scene);
            self.compositor.active_scene_layers.push(layer.clone());
        }
        let replay_ctx = RenderContext::with_transforms(
            frame.bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        self.replay_dynamic_subtree(replay_ctx, &frame.subtree);
        self.draw_dynamic_morphs(&frame.content_morphs, vello::kurbo::Affine::IDENTITY);
        while !self.compositor.active_scene_layers.is_empty() {
            self.pop_layer();
        }
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
        self.flush_vello_scene_layer();
        self.retained_window_frame = Some(frame);
        true
    }
}
