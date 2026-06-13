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

impl Drop for HydrolysisRenderer {
    fn drop(&mut self) {
        self.lifecycle.drop_all_hooks();
    }
}

impl HydrolysisRenderer {
    pub(crate) fn set_window_bounds(&mut self, bounds: vello::kurbo::Rect) {
        self.window_bounds = bounds;
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

    pub(crate) fn table_slot_and_state_mut(
        &mut self,
        index: usize,
    ) -> (&mut crate::renderer::lazy::LazyTableSlot, &mut HydroState) {
        (
            &mut self.lazy.lazy_table_controller.slots[index],
            &mut self.state,
        )
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
        if !self.reuse_scroll_content_caches {
            self.scroll_content_caches.clear();
        }
        self.retained_window_frame = None;
        self.state.measurement.begin_rebuild_frame();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
        self.active_applied_filter_cursor = 0;
        self.effect_runtime_slots.begin_rebuild_frame();
        self.lifecycle.begin_rebuild_frame();
        self.hit_test.begin_rebuild_frame();
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.animation_controller.begin_rebuild_frame();
        self.scroll_controller.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        self.compositor.gpu_surface_cursor = 0;
        self.compositor.render_layers.clear();
        self.compositor.active_scene_layers.clear();
        self.popup_menu.begin_rebuild_frame();
        self.text_editing.text_selection_cursor = 0;
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
    }

    pub(crate) fn set_scroll_content_cache_reuse(&mut self, reuse: bool) {
        self.reuse_scroll_content_caches = reuse;
    }

    pub(crate) fn set_applied_filter_input_cache_reuse(&mut self, reuse: bool) {
        self.reuse_applied_filter_inputs = reuse;
    }

    pub(crate) fn begin_redraw_frame(&mut self) {
        self.state.measurement.reset_counters();
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
    }

    pub fn finish_rebuild_frame(&mut self) {
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis renderer: scene layer stack must be empty at end of rebuild (len={})",
            self.compositor.active_scene_layers.len()
        );
        self.flush_vello_scene_layer();
        self.lifecycle
            .finish_rebuild_frame(&mut self.state, self.reuse_scroll_content_caches);

        if matches!(
            self.text_editing.focused_text_input.get(),
            Some(index) if index >= self.text_editing.text_input_targets.len()
        ) {
            self.set_focused_text_input(None);
        }
        if matches!(
            self.text_editing.active_text_selection_drag,
            Some(index) if index >= self.text_editing.text_input_targets.len()
        ) {
            self.text_editing.active_text_selection_drag = None;
        }

        self.animation_controller
            .finish_rebuild_frame_with_inactive_slot_retention(self.reuse_scroll_content_caches);
        self.scroll_controller.finish_rebuild_frame();
        self.hit_test.finish_rebuild_frame();
        self.lazy.finish_rebuild_frame();
        // Drop retained collection caches whose slot was not rebound this frame
        // (the collection left the tree); reused slots keep their per-item caches.
        let live_collections = self.lazy.live_collection_keys();
        self.collection_caches
            .retain(|key, _| live_collections.contains(key));
        self.navigation.finish_rebuild_frame();
        self.compositor
            .gpu_surface_slots
            .truncate(self.compositor.gpu_surface_cursor);
        self.active_applied_filters
            .truncate(self.active_applied_filter_cursor);
        self.effect_runtime_slots.finish_rebuild_frame();
        self.popup_menu.finish_rebuild_frame();
        self.text_editing
            .text_selection_slots
            .truncate(self.text_editing.text_selection_cursor);
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

    pub fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.state.set_frame_resources(device, queue);
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

    /// The shared frame-trigger handle for closures that outlive a borrow of
    /// the renderer (navigation controllers, GPU-surface invalidators, …).
    pub(crate) fn frame_signals(&self) -> FrameSignals {
        self.signals.clone()
    }

    pub fn request_redraw(&self) {
        self.signals.request_redraw();
    }

    pub fn take_redraw_request(&self) -> bool {
        self.signals.take_redraw_request()
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

    #[must_use]
    pub fn has_retained_window_frame(&self) -> bool {
        self.retained_window_frame.is_some()
    }

    pub fn take_next_frame_rebuild_request(&self) -> bool {
        self.signals.take_next_frame_rebuild_request()
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
