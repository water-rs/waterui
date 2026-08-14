//! Frame lifecycle: scene reset, rebuild/redraw frame boundaries, layer
//! stack management, frame triggers, and per-frame statistics.

use super::*;

pub(crate) fn duration_micros_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Whether a scene encodes any visible content.
///
/// `Encoding::is_empty` only checks the path stream; glyph runs are deferred
/// resources that resolve to paths at render time, so a scene containing only
/// text would otherwise read as empty and be dropped by the compositor.
pub(crate) fn scene_has_content(scene: &vello::Scene) -> bool {
    let encoding = scene.encoding();
    !encoding.is_empty() || !encoding.resources.glyph_runs.is_empty()
}

pub(crate) fn color_to_wgpu(color: vello::peniko::Color) -> wgpu::Color {
    let linear = ResolvedColor::from_srgb(Srgb::new(
        color.components[0],
        color.components[1],
        color.components[2],
    ))
    .linear_with_headroom();
    wgpu::Color {
        r: f64::from(linear[0]),
        g: f64::from(linear[1]),
        b: f64::from(linear[2]),
        a: f64::from(color.components[3]),
    }
}

impl HydrolysisRenderer {
    pub(crate) fn set_window_bounds(&mut self, bounds: vello::kurbo::Rect) {
        self.window_bounds = bounds;
    }

    /// Whether the persistent render tree has been built. The view tree's `body()`
    /// is dispatched recursively exactly once — on the first frame, when this is
    /// `false`. Afterwards every change (reactive value, structural patch, scroll,
    /// resize, interaction) is reflected by refreshing this retained tree, so the
    /// runner routes any later rebuild request through the refresh pump instead of
    /// re-running `build_content`.
    #[must_use]
    pub fn has_render_tree(&self) -> bool {
        self.render_tree.is_some()
    }

    #[must_use]
    pub fn state(&self) -> &HydroState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut HydroState {
        &mut self.state
    }

    pub(crate) fn state_and_scene_mut(&mut self) -> (&mut HydroState, &mut vello::Scene) {
        (&mut self.state, &mut self.scene)
    }

    #[must_use]
    pub fn scene(&self) -> &vello::Scene {
        &self.scene
    }

    pub fn reset_scene(&mut self) {
        for image in self.compositor.active_filter_images.drain(..) {
            self.vello_renderer.unregister_texture(image);
        }
        self.hit_test.reset_scene();
        self.gesture_engine.clear_targets();
        self.text_editing.text_input_targets.clear();
        self.scene.reset();
        self.compositor.render_layers.clear();
        self.compositor.active_scene_layers.clear();
        self.state.measurement.reset_counters();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
        #[cfg(feature = "accessibility")]
        self.accessibility.reset_scene();
    }

    pub fn begin_rebuild_frame(&mut self) {
        // A full rebuild re-dispatches every Dynamic node, so any pending isolated
        // reactive patch is subsumed by it.
        self.signals.begin_rebuild();
        self.state.measurement.begin_frame();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
        self.lifecycle.begin_rebuild_frame();
        self.hit_test.begin_rebuild_frame();
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.animation_controller.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        self.compositor.render_layers.clear();
        self.compositor.active_scene_layers.clear();
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
    }

    /// Drop cached `Dynamic` measurements whose node has left the retained tree.
    ///
    /// Shared by the two frames that can remove nodes: the one-time build and any
    /// refresh that applied a structural patch.
    pub(crate) fn prune_dynamic_measurements(&mut self, live: &FxHashSet<usize>) {
        self.state
            .measurement
            .retain_dynamic_identities(|identity| live.contains(&identity));
    }

    pub(crate) fn begin_redraw_frame(&mut self) {
        // Clear the per-frame `stable_ptr`-keyed view-dimension cache, not just the
        // counters: the refresh path now runs full layout every frame, so it measures
        // `RetainedSubview`/widget content through that cache. Its keys are view heap
        // addresses, unique only within a frame (a freed view's address is reused next
        // frame), so a stale entry would otherwise be read as a different view's size.
        // The persistent, content-keyed text-shaping cache is untouched and keeps full
        // layout cheap.
        self.state.measurement.begin_frame();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
    }

    /// Drop text-input focus / the selection drag when the target they name is
    /// no longer emitted. Targets are pure-emission (rebuilt in flush order every
    /// frame), so after any flush — rebuild or refresh — a previously focused
    /// field may be gone. Both are held by stable identity, so this only asks
    /// whether that identity still resolves; it can never mistake a different
    /// field that moved into the old position for the focused one. Shared by both
    /// frame paths.
    pub(crate) fn validate_focused_text_input_after_flush(&mut self) {
        let modal_active = self.hit_test.modal_interaction.is_some();
        let focus_is_live = !self.text_editing.has_focus()
            || self
                .text_editing
                .focused_target()
                .is_some_and(|target| !modal_active || target.modal);
        if !focus_is_live {
            self.set_focused_text_input_key(None);
        }
        let keyboard_focus_is_live =
            self.hit_test.keyboard_focus.as_ref().is_none_or(|focused| {
                self.hit_test.pointer_targets.iter().any(|target| {
                    (!modal_active || target.modal)
                        && target
                            .press_slot
                            .as_ref()
                            .is_some_and(|slot| &slot.key == focused)
                }) || self.text_editing.text_input_targets.iter().any(|target| {
                    (!modal_active || target.modal) && &target.interaction_key == focused
                })
            });
        if !keyboard_focus_is_live {
            self.set_keyboard_focus(None, false);
        }
        let selection_drag_is_live = self
            .text_editing
            .active_text_selection_drag
            .as_ref()
            .is_none_or(|key| self.text_editing.index_of(key).is_some());
        if !selection_drag_is_live {
            self.text_editing.active_text_selection_drag = None;
        }
    }

    pub fn finish_rebuild_frame(&mut self) {
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis renderer: scene layer stack must be empty at end of rebuild (len={})",
            self.compositor.active_scene_layers.len()
        );
        self.flush_vello_scene_layer();
        self.lifecycle.finish_rebuild_frame();
        // Prune the measure-path `Dynamic` dimension cache down to the identities
        // still present in the retained render tree. The cache is read by
        // `measure_dynamic` when a `Dynamic` leaf is measured after its content was
        // handed to a `DynamicHostNode`; the live `DynamicHostNode`s in `render_tree`
        // are exactly the alive identities now that the dispatch path is gone.
        let live_dynamics = self
            .render_tree
            .as_ref()
            .map(RenderNode::collect_dynamic_identities)
            .unwrap_or_default();
        self.prune_dynamic_measurements(&live_dynamics);

        self.validate_focused_text_input_after_flush();

        self.animation_controller
            .finish_rebuild_frame_with_inactive_slot_retention(false);
        self.hit_test.finish_rebuild_frame();
        self.navigation.finish_rebuild_frame();
        self.signals.finish_rebuild();
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
    }

    pub fn scene_mut(&mut self) -> &mut vello::Scene {
        &mut self.scene
    }

    pub(crate) fn draw_context(&mut self, ctx: RenderContext) -> VelloDrawContext<'_> {
        VelloDrawContext::with_root_transform(&mut self.scene, ctx.transform)
    }

    pub fn vello_renderer(&mut self) -> &mut vello::Renderer {
        &mut self.vello_renderer
    }

    pub fn set_frame_resources(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        self.state.set_frame_resources(adapter, device, queue);
    }

    pub fn clear_frame_resources(&mut self) {
        self.state.clear_frame_resources();
    }

    pub(crate) fn push_layer_rect(
        &mut self,
        alpha: f32,
        transform: vello::kurbo::Affine,
        rect: vello::kurbo::Rect,
    ) {
        self.record_clip_layer_push();
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            transform,
            &rect,
        );
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform,
            shape: LayerShape::Rect(rect),
        });
    }

    pub(super) fn push_layer_path(
        &mut self,
        alpha: f32,
        transform: vello::kurbo::Affine,
        path: vello::kurbo::BezPath,
    ) {
        self.record_clip_layer_push();
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            transform,
            &path,
        );
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform,
            shape: LayerShape::Path(path),
        });
    }

    pub(super) fn push_layer_rounded_rect(
        &mut self,
        alpha: f32,
        transform: vello::kurbo::Affine,
        path: vello::kurbo::BezPath,
        rect: vello::kurbo::Rect,
        corner_width: f64,
        corner_height: f64,
    ) {
        self.record_clip_layer_push();
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            transform,
            &path,
        );
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform,
            shape: LayerShape::RoundedRect {
                path,
                rect,
                corner_width,
                corner_height,
            },
        });
    }

    pub(crate) fn pop_layer(&mut self) {
        self.scene.pop_layer();
        self.compositor
            .active_scene_layers
            .pop()
            .expect("hydrolysis renderer: pop_layer underflow");
    }

    pub(super) fn record_clip_layer_push(&mut self) {
        self.frame_clip_layers = self
            .frame_clip_layers
            .checked_add(1)
            .expect("hydrolysis frame clip layer counter overflow");
        let depth = u32::try_from(self.compositor.active_scene_layers.len() + 1)
            .expect("hydrolysis active scene layer depth exceeds u32");
        self.frame_max_clip_depth = self.frame_max_clip_depth.max(depth);
    }

    pub(super) fn flush_vello_scene_layer(&mut self) {
        assert!(
            (self.scene.encoding().n_open_clips as usize)
                == self.compositor.active_scene_layers.len(),
            "hydrolysis renderer: scene clip count {} does not match tracked scene layers {}",
            self.scene.encoding().n_open_clips,
            self.compositor.active_scene_layers.len()
        );

        for _ in 0..self.compositor.active_scene_layers.len() {
            self.scene.pop_layer();
        }

        if !scene_has_content(&self.scene) {
            for layer in &self.compositor.active_scene_layers {
                layer.push_to_scene(&mut self.scene);
            }
            return;
        }
        let scene = core::mem::take(&mut self.scene);
        self.compositor
            .render_layers
            .push(RenderLayer::Vello(scene));

        for layer in &self.compositor.active_scene_layers {
            layer.push_to_scene(&mut self.scene);
        }
    }

    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn record_native_view_layer(
        &mut self,
        view: objc2::rc::Retained<objc2_web_kit::WKWebView>,
        transform: vello::kurbo::Affine,
        bounds: vello::kurbo::Rect,
    ) {
        self.flush_vello_scene_layer();
        self.compositor
            .render_layers
            .push(RenderLayer::NativeView(NativeViewLayer {
                view,
                transform,
                bounds,
                active_layers: self.compositor.active_scene_layers.clone(),
            }));
    }

    /// The shared frame-trigger handle for closures that outlive a borrow of
    /// the renderer (navigation controllers, GPU-surface invalidators, …).
    pub(crate) fn frame_signals(&self) -> FrameSignals {
        self.signals.clone()
    }

    pub(crate) fn set_host_redraw_handle(&mut self, handle: RedrawHandle) {
        self.host_redraw_handle = Some(handle);
    }

    pub fn request_redraw(&self) {
        self.signals.request_redraw();
    }

    pub(crate) fn request_refresh(&self) {
        self.signals.request_refresh();
    }

    pub fn take_redraw_request(&self) -> bool {
        self.signals.take_redraw_request()
    }

    /// A transform-level visual value outside the reactive graph moved (a
    /// scroll offset, a scrollbar drag): the retained tree must be re-encoded
    /// at the placements the last layout computed.
    pub fn request_reencode(&self) {
        self.signals.request_reencode();
    }

    pub fn take_reencode_request(&self) -> bool {
        self.signals.take_reencode_request()
    }

    pub fn request_rebuild(&self) {
        self.signals.request_rebuild();
    }

    #[must_use]
    pub fn has_rebuild_request(&self) -> bool {
        self.signals.has_rebuild_request()
    }

    pub fn request_next_frame_rebuild(&self) {
        self.signals.request_next_frame_rebuild();
    }

    pub fn take_rebuild_request(&self) -> bool {
        self.signals.take_rebuild_request()
    }

    #[must_use]
    pub fn has_patch_request(&self) -> bool {
        self.signals.has_patch_request()
    }

    pub fn take_patch_request(&self) -> bool {
        self.signals.take_patch_request()
    }

    pub fn take_next_frame_rebuild_request(&self) -> bool {
        self.signals.take_next_frame_rebuild_request()
    }

    /// Whether the renderer has scheduled work that will still change layout,
    /// semantics, or reactive state on a future frame: pending patches or
    /// rebuilds, active animations, armed gesture deadlines, or gliding smooth
    /// scrolls.
    ///
    /// Visual-only redraw requests (caret blink, the visible-window present
    /// cadence) are deliberately excluded: they repaint pixels without moving
    /// semantic state, and a focused text caret blinks forever — including it
    /// would make an app with a focused field never count as settled.
    #[must_use]
    pub fn has_scheduled_semantic_work(&self) -> bool {
        self.signals.has_patch_request()
            || self.signals.has_reencode_request()
            || self.signals.has_rebuild_request()
            || self.signals.has_next_frame_rebuild_request()
            || self.animations_active()
            || self.next_gesture_deadline().is_some()
            || self.has_gliding_smooth_scrolls()
    }

    pub(crate) fn measurement_cache_stats(&self) -> (u32, u32) {
        self.state.measurement.stats()
    }

    pub(crate) fn render_layer_stats(&self) -> (u32, u32, u32) {
        let scene_layers = u32::try_from(self.compositor.render_layers.len())
            .expect("hydrolysis render layer count exceeds u32");
        let vello_scene_layers = u32::try_from(
            self.compositor
                .render_layers
                .iter()
                .filter(|layer| matches!(layer, RenderLayer::Vello(_)))
                .count(),
        )
        .expect("hydrolysis Vello scene layer count exceeds u32");
        let direct_gpu_surfaces = u32::try_from(
            self.compositor
                .render_layers
                .iter()
                .filter(|layer| {
                    matches!(
                        layer,
                        RenderLayer::GpuSurface(GpuSurfaceLayer {
                            direct_to_target: true,
                            ..
                        })
                    )
                })
                .count(),
        )
        .expect("hydrolysis direct GpuSurface count exceeds u32");
        let gpu_surface_layers = scene_layers
            .checked_sub(vello_scene_layers)
            .and_then(|count| count.checked_sub(direct_gpu_surfaces))
            .expect("hydrolysis render layer count accounting underflow");
        let composited_scene_layers = scene_layers
            .checked_sub(direct_gpu_surfaces)
            .expect("hydrolysis render layer count accounting underflow");
        (
            composited_scene_layers,
            vello_scene_layers,
            gpu_surface_layers,
        )
    }

    pub(crate) fn clip_layer_stats(&self) -> (u32, u32) {
        (self.frame_clip_layers, self.frame_max_clip_depth)
    }
}
