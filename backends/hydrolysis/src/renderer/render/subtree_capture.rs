//! Batched capture of filtered subtrees.
//!
//! An `AppliedFilter` needs its child's pixels as a texture before it can run.
//! Rendering each filtered subtree through its own compositor pass costs a fixed
//! few milliseconds per filter on every backend (39 ms on DX12 / WARP), so a
//! screen of a hundred filters paid a hundred passes per frame. Every filtered
//! subtree in a frame is instead flushed into a slot of a shelf-packed atlas
//! page, each page is rendered with one compositor pass, and the slots are
//! copied into the filters' input textures and filtered under a single command
//! encoder and queue submit.
//!
//! Filters nest: a filter inside a filtered subtree is captured one level
//! deeper. Deeper levels are rendered first, so a filter's output image exists
//! by the time the page holding its parent is rendered.

use super::*;
use core::mem;
use std::time::Instant;

/// Pages are padded up to this granularity so that a layout that jitters by a
/// few pixels between frames keeps hitting the same pooled page texture.
const PAGE_EXTENT_GRANULARITY: u32 = 64;

/// The per-frame atlas state: one level per capture nesting depth plus the
/// page textures kept across frames.
#[derive(Default)]
pub(crate) struct SubtreeCaptures {
    levels: Vec<CaptureLevel>,
    /// Nesting depth of the subtree currently being flushed: `0` for the
    /// window, `n + 1` inside the `n`th nested capture.
    pub(crate) depth: usize,
    /// Page textures released by earlier frames, reused by exact extent.
    texture_pool: Vec<CapturePageTexture>,
    frame_pages: u32,
}

#[derive(Default)]
struct CaptureLevel {
    pages: Vec<CapturePage>,
    pending: Vec<PendingFilter>,
}

/// One atlas page: the scene every slot of this page was flushed into and the
/// shelf-packing cursor that placed them.
#[derive(Default)]
struct CapturePage {
    content: CapturePageContent,
    slots: Vec<CaptureSlot>,
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
    used_width: u32,
}

/// The renderer scene state a page's slots are flushed into, swapped in and
/// out of the renderer around each slot flush and the page render.
#[derive(Default)]
struct CapturePageContent {
    scene: vello::Scene,
    render_layers: Vec<RenderLayer>,
    transient_scene: Option<vello::Scene>,
}

struct CaptureSlot {
    origin: (u32, u32),
    width: u32,
    height: u32,
    /// The filter's input texture the slot is copied into once the page is rendered.
    input: wgpu::Texture,
}

struct PendingFilter {
    runtime: Rc<RefCell<AppliedFilterRuntime>>,
    width: u32,
    height: u32,
}

struct CapturePageTexture {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl CapturePage {
    /// Place a `width` × `height` slot on this page, or `None` when the page is
    /// full at `limit` pixels per side.
    fn place(&mut self, width: u32, height: u32, limit: u32) -> Option<(u32, u32)> {
        if self.cursor_x + width > limit {
            let next_y = self.cursor_y + self.shelf_height;
            if next_y + height > limit {
                return None;
            }
            self.cursor_x = 0;
            self.cursor_y = next_y;
            self.shelf_height = 0;
        }
        if self.cursor_y + height > limit {
            return None;
        }
        let origin = (self.cursor_x, self.cursor_y);
        self.cursor_x += width;
        self.shelf_height = self.shelf_height.max(height);
        self.used_width = self.used_width.max(self.cursor_x);
        Some(origin)
    }

    /// The page texture extent, padded to [`PAGE_EXTENT_GRANULARITY`] and
    /// clamped to `limit`.
    fn extent(&self, limit: u32) -> (u32, u32) {
        let pad = |value: u32| value.div_ceil(PAGE_EXTENT_GRANULARITY) * PAGE_EXTENT_GRANULARITY;
        (
            pad(self.used_width).min(limit),
            pad(self.cursor_y + self.shelf_height).min(limit),
        )
    }
}

impl SubtreeCaptures {
    pub(crate) fn begin_frame(&mut self) {
        assert!(
            self.levels.is_empty(),
            "hydrolysis filter atlas: a previous frame left {} capture level(s) unflushed",
            self.levels.len()
        );
        assert_eq!(
            self.depth, 0,
            "hydrolysis filter atlas: a previous frame left the capture depth unbalanced"
        );
        self.frame_pages = 0;
    }

    /// Pages rendered so far this frame.
    #[cfg(test)]
    pub(crate) fn frame_pages(&self) -> u32 {
        self.frame_pages
    }

    fn allocate(
        &mut self,
        depth: usize,
        width: u32,
        height: u32,
        limit: u32,
        input: wgpu::Texture,
    ) -> (usize, (u32, u32)) {
        assert!(
            width <= limit && height <= limit,
            "hydrolysis filter atlas: a {width}x{height} filtered subtree exceeds the \
             device's {limit}px texture limit"
        );
        if self.levels.len() <= depth {
            self.levels.resize_with(depth + 1, CaptureLevel::default);
        }
        let pages = &mut self.levels[depth].pages;
        let placed = pages
            .last_mut()
            .and_then(|page| page.place(width, height, limit));
        let (index, origin) = match placed {
            Some(origin) => (pages.len() - 1, origin),
            None => {
                let mut page = CapturePage::default();
                let origin = page
                    .place(width, height, limit)
                    .expect("hydrolysis filter atlas: a fresh page must fit one slot");
                pages.push(page);
                (pages.len() - 1, origin)
            }
        };
        pages[index].slots.push(CaptureSlot {
            origin,
            width,
            height,
            input,
        });
        (index, origin)
    }

    fn acquire_page_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> CapturePageTexture {
        if let Some(index) = self
            .texture_pool
            .iter()
            .position(|page| page.width == width && page.height == height)
        {
            return self.texture_pool.swap_remove(index);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_filter_atlas_page"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        CapturePageTexture {
            width,
            height,
            texture,
            view,
        }
    }
}

impl HydrolysisRenderer {
    /// Flush `child` into an atlas slot at the current capture depth and queue
    /// `runtime` to be filtered from that slot when the level is flushed.
    ///
    /// The child is flushed in slot-local coordinates: its viewport is the slot
    /// and its root transform places the slot on the page. Hit testing keeps the
    /// parent's transform so controls inside a filtered subtree stay clickable
    /// where they are drawn.
    pub(crate) fn capture_child_into_atlas(
        &mut self,
        child: &RenderNode,
        ctx: RenderContext,
        env: &Environment,
        runtime: &Rc<RefCell<AppliedFilterRuntime>>,
        width: u32,
        height: u32,
    ) {
        let device = self.state().frame_resources().0.clone();
        let limit = device.limits().max_texture_dimension_2d;
        let input = runtime
            .borrow_mut()
            .input_texture(&device, width, height)
            .0
            .clone();
        let depth = self.subtree_captures.depth;
        let (page_index, origin) = self
            .subtree_captures
            .allocate(depth, width, height, limit, input);

        let page_content =
            mem::take(&mut self.subtree_captures.levels[depth].pages[page_index].content);
        let parent_content = self.swap_capture_page_content(page_content);
        let parent_active_layers = mem::take(&mut self.compositor.active_scene_layers);
        let parent_window_bounds = self.window_bounds;
        let parent_window_root_transform = self.window_root_transform;

        let local_bounds = vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let slot_transform =
            vello::kurbo::Affine::translate((f64::from(origin.0), f64::from(origin.1)));
        self.set_window_viewport(local_bounds, slot_transform);
        let local_ctx = RenderContext::with_transforms(
            local_bounds,
            slot_transform,
            ctx.hit_transform * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0)),
        );

        self.subtree_captures.depth = depth + 1;
        // The slot clip keeps a child that paints outside its bounds (a shadow,
        // an overflowing transform) from bleeding into its neighbours' slots.
        self.push_layer_rect(1.0, slot_transform, local_bounds);
        child.flush(self, local_ctx, env);
        self.pop_layer();
        self.subtree_captures.depth = depth;
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis filter atlas capture left an unclosed scene layer"
        );

        self.subtree_captures.levels[depth].pages[page_index].content =
            self.swap_capture_page_content(parent_content);
        self.compositor.active_scene_layers = parent_active_layers;
        self.set_window_viewport(parent_window_bounds, parent_window_root_transform);
        self.subtree_captures.levels[depth]
            .pending
            .push(PendingFilter {
                runtime: Rc::clone(runtime),
                width,
                height,
            });
    }

    /// Render every atlas page at capture depth `from_depth` and deeper, copy
    /// the slots into their filters' input textures, and run the filters.
    /// Deeper levels go first so their outputs exist when the level above is
    /// rendered.
    ///
    /// Called once the subtree that owns those levels has been flushed: the
    /// window flush calls it with depth `0`, and a nested texture capture with
    /// the depth of its own child.
    pub(crate) fn flush_subtree_captures(&mut self, from_depth: usize) {
        if self.subtree_captures.levels.len() <= from_depth {
            return;
        }
        let levels = self.subtree_captures.levels.split_off(from_depth);
        let adapter = self.state().frame_adapter().clone();
        let (device, queue) = {
            let (device, queue) = self.state().frame_resources();
            (device.clone(), queue.clone())
        };
        let limit = device.limits().max_texture_dimension_2d;
        for level in levels.into_iter().rev() {
            let capture_started_at = Instant::now();
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis filter atlas encoder"),
            });
            // Page textures are handed back to the pool only after the submit
            // that copies out of them: releasing one earlier would let the next
            // page of this frame render over slots not yet copied.
            let mut used_textures = Vec::with_capacity(level.pages.len());
            for page in level.pages {
                let (page_width, page_height) = page.extent(limit);
                let texture =
                    self.subtree_captures
                        .acquire_page_texture(&device, page_width, page_height);
                self.render_capture_page(page.content, &adapter, &device, &queue, &texture);
                for slot in &page.slots {
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: slot.origin.0,
                                y: slot.origin.1,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &slot.input,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width: slot.width,
                            height: slot.height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                used_textures.push(texture);
                self.subtree_captures.frame_pages = self
                    .subtree_captures
                    .frame_pages
                    .checked_add(1)
                    .expect("hydrolysis filter atlas page counter overflow");
            }
            self.frame_applied_filter_capture += capture_started_at.elapsed();

            let effect_started_at = Instant::now();
            for pending in level.pending {
                let (_image, needs_redraw) = pending.runtime.borrow_mut().encode_output(
                    &device,
                    &queue,
                    &mut self.vello_renderer,
                    pending.width,
                    pending.height,
                    &mut encoder,
                );
                self.frame_applied_filter_count = self
                    .frame_applied_filter_count
                    .checked_add(1)
                    .expect("hydrolysis applied filter counter overflow");
                if needs_redraw {
                    self.request_redraw();
                }
            }
            queue.submit([encoder.finish()]);
            self.frame_applied_filter_effect += effect_started_at.elapsed();
            self.subtree_captures.texture_pool.extend(used_textures);
        }
    }

    /// Composite one page's scene into its page texture with the renderer's
    /// scene state swapped out for the duration.
    fn render_capture_page(
        &mut self,
        content: CapturePageContent,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &CapturePageTexture,
    ) {
        let parent_content = self.swap_capture_page_content(content);
        let parent_active_layers = mem::take(&mut self.compositor.active_scene_layers);
        let parent_window_bounds = self.window_bounds;
        let parent_window_root_transform = self.window_root_transform;
        self.set_window_viewport(
            vello::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(texture.width),
                f64::from(texture.height),
            ),
            vello::kurbo::Affine::IDENTITY,
        );
        self.render_scene_to_texture(HydrolysisRenderTarget {
            adapter,
            device,
            queue,
            texture: Some(&texture.texture),
            view: &texture.view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: texture.width,
            height: texture.height,
            base_color: vello::peniko::Color::TRANSPARENT,
        });
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis filter atlas compositor restored an active scene layer"
        );
        // The compositor consumed the page content; what it leaves behind is
        // empty and dropped.
        let _ = self.swap_capture_page_content(parent_content);
        self.compositor.active_scene_layers = parent_active_layers;
        self.set_window_viewport(parent_window_bounds, parent_window_root_transform);
    }

    /// Install `content` as the renderer's current scene state and return the
    /// state it replaced.
    fn swap_capture_page_content(&mut self, content: CapturePageContent) -> CapturePageContent {
        CapturePageContent {
            scene: mem::replace(&mut self.scene, content.scene),
            render_layers: mem::replace(&mut self.compositor.render_layers, content.render_layers),
            transient_scene: mem::replace(&mut self.transient_scene, content.transient_scene),
        }
    }
}
