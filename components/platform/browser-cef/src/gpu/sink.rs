//! The platform-shared CEF frame sink.
//!
//! Chromium hands each accelerated paint over as a shared image whose
//! contents return to CEF's pool when `on_accelerated_paint` returns — the
//! handle cannot be cached or accessed afterwards, so the sink copies:
//! inside the callback the image is imported transiently and lands once on
//! the GPU in a texture the engine owns — one `copy_texture_to_texture`,
//! or one composite draw while a popup is open. That owned texture
//! presents through [`FrameOutput`] as the view layer's content, and a
//! pooled texture is reused only after the engine has released the frame
//! that showed it. No reference to CEF's shared image outlives the
//! callback.
//!
//! What differs per platform is how the shared image imports, what an
//! engine-owned target is allocated from and how its release is observed —
//! the [`Backend`] implementation; the pool scan, the popup composite and
//! the presentation flow are common.

use std::cell::RefCell;
use std::marker::PhantomData;

use cef::{AcceleratedPaintInfo, PaintElementType, Rect};
use num_traits::ToPrimitive as _;
use waterui_graphics::cherenkov_gpu::interop::ExternalFrame;
use waterui_graphics::gpu::{ExternalFrameSource, ExternalFrameView, FrameOutput};

use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

/// The format a composited target renders to: what Chromium's BGRA/XBGRA
/// shared images decode to, in premultiplied alpha.
const COMPOSITE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// How many owned textures the presentation pool holds before it stops
/// absorbing paints: one on screen, one pending in the engine, one being
/// written, plus headroom so a resize or a slow release does not stall
/// Chromium's compositor.
const POOL_LIMIT: usize = 4;

/// The platform half of a [`SharedSink`]: transient import of a paint's
/// shared image, the engine-owned pool texture it lands in, and the frame
/// a written target presents as.
pub(super) trait Backend: Sized + 'static {
    /// What a pooled target must match for reuse: extent plus the target
    /// class it was allocated as.
    type Key: Copy + PartialEq;
    /// A transient import of CEF's shared image — dropped inside the
    /// paint callback, never retained.
    type Import;
    /// One engine-owned presentation texture the pool recycles after the
    /// engine released the generation last presented from it.
    type Target;

    /// Whether the transient import is a `copy_texture_to_texture`
    /// source; where it is not, every view paint composites through
    /// [`Compositor`].
    const COPIES: bool;

    /// Opens the backend on `output`'s device.
    ///
    /// # Panics
    ///
    /// Panics when the output's device cannot run this backend.
    fn open(output: &FrameOutput) -> Self;

    /// Imports the paint's shared image for the callback's duration.
    /// Whatever the import retains of the resource dies with it.
    ///
    /// # Panics
    ///
    /// Panics when the shared image cannot be imported on the engine's
    /// device.
    fn import(&self, frame: &AcceleratedPaintInfo) -> Self::Import;

    /// The import's texture on the output's device.
    fn texture(import: &Self::Import) -> &wgpu::Texture;

    /// The extent the import presents at: the visible size a padded
    /// shared image is cropped to.
    fn size(import: &Self::Import) -> (u32, u32);

    /// The normalized texel rect the frame occupies in
    /// [`texture`](Self::texture) — full extent where the import cropped
    /// at import, the visible fraction where the shared image is padded.
    fn source_uv(import: &Self::Import) -> [f32; 4];

    /// The import's own target class — what a pooled target written by a
    /// straight copy of it matches on.
    fn key(import: &Self::Import) -> Self::Key;

    /// The key a view target is allocated under: `key` for the copy
    /// path, the composite class's key for a composited write.
    fn view_key(key: Self::Key, composited: bool) -> Self::Key;

    /// Allocates one pooled target under `key`.
    ///
    /// # Panics
    ///
    /// Panics when the allocation fails.
    fn alloc(&self, key: Self::Key) -> Self::Target;

    /// Materializes `target`'s texture for this write — a fresh
    /// generation of the pooled allocation where the platform wraps it
    /// per present.
    ///
    /// # Panics
    ///
    /// Panics when the import fails.
    fn materialize<'a>(&self, target: &'a mut Self::Target) -> &'a wgpu::Texture;

    /// The texture `target` currently holds, if a write already
    /// materialized one.
    fn current(target: &Self::Target) -> Option<&wgpu::Texture>;

    /// Whether the engine released the generation last presented from
    /// `target`, so it may host a new one. The engine's retirement state
    /// is the observation — never a CPU wait.
    fn released(target: &mut Self::Target) -> bool;

    /// The frame a written target presents as.
    ///
    /// # Panics
    ///
    /// Panics when `materialize` did not run first.
    fn frame(&self, target: &mut Self::Target) -> ExternalFrame;
}

/// One CEF page's [`ExternalFrameSource`]: installs the page's frame sink
/// on `start`, and on each host frame keeps Chromium's logical viewport in
/// step with the presented extent and asks for the next compositor frame.
pub(super) struct ExternalSource<B: Backend> {
    page: CefPageHandle,
    output: Option<FrameOutput>,
    backend: PhantomData<fn() -> B>,
}

impl<B: Backend> ExternalSource<B> {
    pub(super) const fn new(page: CefPageHandle) -> Self {
        Self {
            page,
            output: None,
            backend: PhantomData,
        }
    }
}

impl<B: Backend> ExternalFrameSource for ExternalSource<B> {
    fn start(&mut self, output: FrameOutput) {
        // `set_frame_sink` re-shows the page, resizes it and invalidates
        // the view — on a restart after device loss that is exactly the
        // repaint onto the new output the issue asks for.
        self.page.set_frame_sink(SharedSink::<B>::new(output.clone()));
        self.output = Some(output);
    }

    /// # Panics
    ///
    /// Panics when the logical viewport does not fit a `u32`.
    fn frame(&mut self) {
        let Some(output) = &self.output else {
            return;
        };
        if output.is_retired() {
            return;
        }
        let (width, height, scale) = output.presented_size();
        if width > 0 && height > 0 {
            let logical_width = (f64::from(width) / f64::from(scale))
                .round()
                .max(1.0)
                .to_u32()
                .expect("CEF logical width exceeds u32");
            let logical_height = (f64::from(height) / f64::from(scale))
                .round()
                .max(1.0)
                .to_u32()
                .expect("CEF logical height exceeds u32");
            self.page.set_viewport(logical_width, logical_height, scale);
        }
        // `external_begin_frame_enabled` is off on macOS — `request_frame`
        // does not exist there; paints arrive on vsync and damage at
        // `windowless_frame_rate` and wake the surface through `present`.
        #[cfg(not(target_os = "macos"))]
        self.page.request_frame();
    }
}

/// The owned state of one [`FrameOutput`]: the texture pool presented
/// frames come from, the open popup's owned texture and rect, and the
/// compositor for while it is open.
struct SinkState<B: Backend> {
    pool: Vec<(B::Key, B::Target)>,
    popup: Option<(B::Key, B::Target)>,
    popup_rect: Option<CefPopupRect>,
    compositor: Option<Compositor>,
    retired: bool,
}

/// Chromium's paint callback: imports each element's shared image
/// transiently, lands it once on the GPU in an owned texture, and
/// presents that.
struct SharedSink<B: Backend> {
    output: FrameOutput,
    backend: B,
    state: RefCell<SinkState<B>>,
}

impl<B: Backend> SharedSink<B> {
    fn new(output: FrameOutput) -> Self {
        let backend = B::open(&output);
        Self {
            output,
            backend,
            state: RefCell::new(SinkState {
                pool: Vec::new(),
                popup: None,
                popup_rect: None,
                compositor: None,
                retired: false,
            }),
        }
    }

    /// Writes the view's transient import into a pooled owned texture —
    /// compositing the open popup over it in the same pass — and presents
    /// the result.
    ///
    /// # Panics
    ///
    /// Panics when the pool allocation or import fails.
    fn present_view(&self, state: &mut SinkState<B>, imported: &B::Import) {
        let composited = !B::COPIES || state.popup.is_some();
        let key = B::view_key(B::key(imported), composited);
        let index = state
            .pool
            .iter_mut()
            .position(|(k, target)| *k == key && B::released(target))
            .or_else(|| {
                if state.pool.len() >= POOL_LIMIT {
                    return None;
                }
                state.pool.push((key, self.backend.alloc(key)));
                Some(state.pool.len() - 1)
            });
        let Some(index) = index else {
            // The engine holds every pooled texture longer than Chromium's
            // frame interval — transient backpressure, not an error; the
            // next paint presents normally.
            tracing::warn!(
                size = ?B::size(imported),
                "CEF frame pool is saturated; dropping this paint"
            );
            return;
        };
        let target = &mut state.pool[index].1;
        let target_texture = self.backend.materialize(target);
        if composited {
            let popup = match (state.popup_rect, &mut state.popup) {
                (Some(rect), Some((_, target))) => {
                    B::current(target).map(|texture| (rect, texture))
                }
                _ => None,
            };
            let compositor = state
                .compositor
                .get_or_insert_with(|| Compositor::new(self.output.device()));
            let (_, _, scale) = self.output.presented_size();
            compositor.composite(
                self.output.device(),
                self.output.queue(),
                B::texture(imported),
                B::source_uv(imported),
                popup,
                target_texture,
                B::size(imported),
                f64::from(scale),
            );
        } else {
            copy_plane(
                self.output.device(),
                self.output.queue(),
                B::texture(imported),
                target_texture,
                B::size(imported),
            );
        }
        let frame = self.backend.frame(target);
        // `RetiredOutput` is the documented stop signal: the host is gone,
        // so this sink stops importing and presenting until `start` hands
        // the page a fresh output.
        if self.output.present(frame).is_err() {
            state.retired = true;
        }
    }

    /// Writes the popup's transient import into its dedicated owned
    /// texture, which lives for as long as the popup is open and feeds
    /// the composite in `present_view`.
    ///
    /// # Panics
    ///
    /// Panics when the allocation or import fails.
    fn write_popup(&self, state: &mut SinkState<B>, imported: &B::Import) {
        // The popup's owned texture is the copy path's target class —
        // on a composite-only backend it is the compositor's target.
        let key = B::view_key(B::key(imported), false);
        if state.popup.as_ref().map(|(key, _)| *key) != Some(key) {
            state.popup = Some((key, self.backend.alloc(key)));
        }
        let target = &mut state
            .popup
            .as_mut()
            .expect("the popup target was ensured above")
            .1;
        let target_texture = self.backend.materialize(target);
        if B::COPIES {
            copy_plane(
                self.output.device(),
                self.output.queue(),
                B::texture(imported),
                target_texture,
                B::size(imported),
            );
        } else {
            state
                .compositor
                .get_or_insert_with(|| Compositor::new(self.output.device()))
                .composite(
                    self.output.device(),
                    self.output.queue(),
                    B::texture(imported),
                    B::source_uv(imported),
                    None,
                    target_texture,
                    B::size(imported),
                    1.0,
                );
        }
    }
}

impl<B: Backend> AcceleratedFrameSink for SharedSink<B> {
    fn import(
        &self,
        element: PaintElementType,
        _dirty_rects: &[Rect],
        frame: &AcceleratedPaintInfo,
    ) {
        if self.state.borrow().retired || self.output.is_retired() {
            return;
        }
        // Transient: `imported` is dropped at the end of this callback —
        // the GPU write it feeds is the only thing that keeps the pixels.
        let imported = self.backend.import(frame);
        let mut state = self.state.borrow_mut();
        match element {
            PaintElementType::VIEW => self.present_view(&mut state, &imported),
            PaintElementType::POPUP => self.write_popup(&mut state, &imported),
            element => panic!("CEF returned unsupported paint element {element:?}"),
        }
        // `imported` drops here, inside the callback: the imported image's
        // destruction is queued behind the write on the engine's ordered
        // queue, and no reference to CEF's shared handle outlives it.
    }

    fn set_popup_rect(&self, rect: Option<CefPopupRect>) {
        let mut state = self.state.borrow_mut();
        state.popup_rect = rect;
        if rect.is_none() {
            state.popup = None;
        }
        // No re-present is possible here: the shared images were transient,
        // so nothing remains to composite from. A closing popup uncovers the
        // page and a moving popup moves over it — Chromium repaints both,
        // and the next view frame presents.
    }
}

/// Copies `source` into `target` on `queue` — the single GPU copy the CEF
/// contract requires inside the paint callback. Submissions on the
/// output's queue serialize with the engine's own, so the texture is
/// written before the frame that presents it is consumed.
fn copy_plane(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Texture,
    target: &wgpu::Texture,
    size: (u32, u32),
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("waterui_cef_frame_copy"),
    });
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: source,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
}

/// Draws the view plane and an open popup plane into one owned texture.
///
/// An [`ExternalFrame`] describes one image, and the popup is a second
/// plane: compositing them here is the only way the pair presents as the
/// layer's content. The shader is the same `cef_blit` the Windows
/// presenter draws with; the render target is the pooled texture the
/// frame presents, so there is no second pass into a producer buffer.
struct Compositor {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    view_rect_buffer: wgpu::Buffer,
    popup_rect_buffer: wgpu::Buffer,
}

impl Compositor {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("cef_blit.wgsl"));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("waterui_cef_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(32),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("waterui_cef_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("waterui_cef_composite_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COMPOSITE_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("waterui_cef_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            bind_group_layout,
            sampler,
            view_rect_buffer: rect_buffer(device, "waterui_cef_view_rect"),
            popup_rect_buffer: rect_buffer(device, "waterui_cef_popup_rect"),
        }
    }

    /// Renders `view` under `popup` into `target` at the view frame's
    /// extent.
    ///
    /// `view_uv` is the normalized texel rect the frame occupies in
    /// `view` — the visible fraction of a padded shared image. `popup`,
    /// when the popup is open, is its owned texture and its rect in
    /// view-logical points; `scale` is the device-pixel ratio the layer
    /// presents at.
    fn composite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::Texture,
        view_uv: [f32; 4],
        popup: Option<(CefPopupRect, &wgpu::Texture)>,
        target: &wgpu::Texture,
        size: (u32, u32),
        scale: f64,
    ) {
        let (width, height) = size;
        write_draw(queue, &self.view_rect_buffer, [0.0, 0.0, 1.0, 1.0], view_uv);
        if let Some((rect, _)) = popup {
            // The popup's rect arrives in view-logical points; `scale` is
            // the device-pixel ratio the layer presents at.
            write_draw(
                queue,
                &self.popup_rect_buffer,
                [
                    (f64::from(rect.x) * scale / f64::from(width))
                        .to_f32()
                        .expect("CEF popup x exceeds f32"),
                    (f64::from(rect.y) * scale / f64::from(height))
                        .to_f32()
                        .expect("CEF popup y exceeds f32"),
                    (f64::from(rect.width) * scale / f64::from(width))
                        .to_f32()
                        .expect("CEF popup width exceeds f32"),
                    (f64::from(rect.height) * scale / f64::from(height))
                        .to_f32()
                        .expect("CEF popup height exceeds f32"),
                ],
                [0.0, 0.0, 1.0, 1.0],
            );
        }
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui_cef_composite"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("waterui_cef_composite_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(
                0,
                &self.bind_group(device, view, &self.view_rect_buffer),
                &[],
            );
            pass.draw(0..6, 0..1);
            if let Some((_, popup)) = popup {
                pass.set_bind_group(
                    0,
                    &self.bind_group(device, popup, &self.popup_rect_buffer),
                    &[],
                );
                pass.draw(0..6, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        source: &wgpu::Texture,
        rect: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let view = source.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("waterui_cef_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: rect.as_entire_binding(),
                },
            ],
        })
    }
}

fn rect_buffer(device: &wgpu::Device, label: &'static str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Writes one draw's destination rect (normalized target space) and
/// source rect (normalized texture space) as the `DrawRect` uniform
/// `cef_blit` reads.
fn write_draw(queue: &wgpu::Queue, buffer: &wgpu::Buffer, destination: [f32; 4], source: [f32; 4]) {
    let mut bytes = [0; 32];
    for (value, chunk) in destination
        .into_iter()
        .chain(source)
        .zip(bytes.as_chunks_mut::<4>().0)
    {
        chunk.copy_from_slice(&value.to_ne_bytes());
    }
    queue.write_buffer(buffer, 0, &bytes);
}

/// Creates the GPU view for one visible CEF page: an
/// [`ExternalFrameView`] whose source imports the page's shared frames
/// on the layer's own device.
pub(super) fn external_view<B: Backend>(page: CefPageHandle) -> ExternalFrameView {
    // No pump here. Chromium's message loop belongs to
    // `CefRuntime::start_message_pump`, which Chromium itself paces; running
    // `do_message_loop_work` inside the frame callback put whatever the
    // browser had queued — parsing, script, compositing — on the main thread
    // inside one frame's budget, which is what tripped the stall probe every
    // few seconds on an idle page. The source's tick installs the sink,
    // keeps the viewport in step and requests the next compositor frame,
    // nothing else.
    ExternalFrameView::new(ExternalSource::<B>::new(page))
}
