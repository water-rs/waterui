//! CEF's Linux GPU path.
//!
//! Chromium hands each accelerated paint over as a DMA-BUF shared image
//! whose contents return to CEF's pool when `on_accelerated_paint`
//! returns — the handle cannot be cached or accessed afterwards, so the
//! sink copies: inside the callback the plane is imported transiently
//! and one `copy_texture_to_texture` (or one composite draw while a
//! popup is open) lands it in a texture the engine owns. That owned
//! texture presents through [`FrameOutput`] as the view layer's content,
//! and a pooled texture is reused only after the engine has released the
//! frame that showed it. No reference to CEF's shared image — no
//! duplicated fd, no imported object — survives the callback.

use std::cell::RefCell;
use std::os::fd::{BorrowedFd, OwnedFd};

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use num_traits::ToPrimitive as _;
use waterui_graphics::cherenkov_gpu::interop::vulkan::{
    self, DmaBuf, DmaBufPlane, FrameSource, QueueFamily, State,
};
use waterui_graphics::cherenkov_gpu::interop::{ExternalFrame, FrameColor, RgbAlpha};
use waterui_graphics::gpu::{ExternalFrameSource, ExternalFrameView, FrameOutput};
use wgpu_external_frame::dma_buf::DmaBufFormat;

use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

/// The format of the owned texture a popup composite renders into: what
/// Chromium's BGRA/XBGRA shared images decode to, in premultiplied alpha.
const COMPOSITE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// How many owned textures the presentation pool holds before it stops
/// absorbing paints: one on screen, one pending in the engine, one being
/// written, plus headroom so a resize or a slow release does not stall
/// Chromium's compositor.
const POOL_LIMIT: usize = 4;

/// One CEF page's [`ExternalFrameSource`]: installs the page's frame sink on
/// `start`, and on each host frame keeps Chromium's logical viewport in step
/// with the presented extent and asks for the next compositor frame.
struct CefExternalSource {
    page: CefPageHandle,
    output: Option<FrameOutput>,
}

impl ExternalFrameSource for CefExternalSource {
    fn start(&mut self, output: FrameOutput) {
        // Native import needs the engine's Vulkan device: this path is
        // DMA-BUF-only, so the host must run on it — a `FrameOutput` on any
        // other backend cannot take these frames.
        let native = vulkan::Device::new(output.shared_device())
            .expect("CEF DMA-BUF import requires WaterUI's Vulkan backend");
        // `set_frame_sink` re-shows the page, resizes it and invalidates the
        // view — on a restart after device loss that is exactly the repaint
        // onto the new output the issue asks for.
        self.page
            .set_frame_sink(LinuxFrameSink::new(output.clone(), native));
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
        self.page.request_frame();
    }
}

/// One engine-owned texture: the allocation's import descriptor — built
/// once by [`vulkan::Device::alloc_dmabuf`] — and the generation the last
/// write left in it.
struct OwnedTarget {
    descriptor: DmaBuf,
    frame: Option<vulkan::Frame>,
}

impl OwnedTarget {
    /// Whether the engine has released the last frame presented from this
    /// allocation, so it may host a new generation. The engine's
    /// retirement state is the observation — never a CPU wait.
    fn released(&self) -> bool {
        self.frame
            .as_ref()
            .is_none_or(|frame| frame.generation.state() == State::Released)
    }
}

/// The owned state of one [`FrameOutput`]: the texture pool presented
/// frames come from, the open popup's owned texture and rect, and the
/// compositor for while it is open.
struct SinkState {
    pool: Vec<OwnedTarget>,
    popup: Option<OwnedTarget>,
    popup_rect: Option<CefPopupRect>,
    compositor: Option<PopupCompositor>,
    retired: bool,
}

/// Chromium's paint callback: imports each element's DMA-BUF transiently,
/// copies it once on the GPU into an owned texture, and presents that.
struct LinuxFrameSink {
    output: FrameOutput,
    native: vulkan::Device,
    state: RefCell<SinkState>,
}

impl LinuxFrameSink {
    const fn new(output: FrameOutput, native: vulkan::Device) -> Self {
        Self {
            output,
            native,
            state: RefCell::new(SinkState {
                pool: Vec::new(),
                popup: None,
                popup_rect: None,
                compositor: None,
                retired: false,
            }),
        }
    }

    /// Copies the view's transient import into a pooled owned texture —
    /// compositing the open popup over it in the same pass — and presents
    /// the result.
    ///
    /// # Panics
    ///
    /// Panics when the pool allocation or import fails, and when either
    /// texture is not an RGB image.
    fn present_view(&self, state: &mut SinkState, source: &vulkan::Frame, fourcc: u32) {
        let source_texture = source
            .generation
            .rgb_wrap
            .as_ref()
            .expect("a CEF view DMA-BUF imports as an RGB frame");
        let size = source.size();
        let popup = match (state.popup_rect, &state.popup) {
            (Some(rect), Some(target)) => target.frame.as_ref().map(|frame| {
                (
                    rect,
                    frame
                        .generation
                        .rgb_wrap
                        .as_ref()
                        .expect("a CEF popup DMA-BUF imports as an RGB frame"),
                )
            }),
            _ => None,
        };
        // A composited target renders BGRA through the blit pipeline and
        // ends the pass in COLOR_ATTACHMENT_OPTIMAL; a plain copy keeps the
        // source's own format and ends in TRANSFER_DST_OPTIMAL.
        let (fourcc, layout) = if popup.is_some() {
            (
                DmaBufFormat::Bgra8.fourcc(),
                vulkan::LAYOUT_COLOR_ATTACHMENT,
            )
        } else {
            (fourcc, vulkan::LAYOUT_TRANSFER_DST)
        };
        let index = state
            .pool
            .iter()
            .position(|target| {
                target.descriptor.size == size
                    && target.descriptor.fourcc == fourcc
                    && target.released()
            })
            .or_else(|| {
                if state.pool.len() >= POOL_LIMIT {
                    return None;
                }
                let descriptor = self
                    .native
                    .alloc_dmabuf(
                        size,
                        fourcc,
                        layout,
                        FrameColor::SRGB,
                        RgbAlpha::Premultiplied,
                    )
                    .expect("CEF pool texture allocation failed");
                state.pool.push(OwnedTarget {
                    descriptor,
                    frame: None,
                });
                Some(state.pool.len() - 1)
            });
        let Some(index) = index else {
            // The engine holds every pooled texture longer than Chromium's
            // frame interval — transient backpressure, not an error; the
            // next paint presents normally.
            tracing::warn!(
                size = ?size,
                "CEF frame pool is saturated; dropping this paint"
            );
            return;
        };
        let target = self
            .native
            .import(FrameSource::DmaBuf(Box::new(
                state.pool[index]
                    .descriptor
                    .reopen()
                    .expect("pool DMA-BUF duplication failed"),
            )))
            .expect("CEF pool texture import failed");
        let target_texture = target
            .generation
            .rgb_wrap
            .as_ref()
            .expect("a pool DMA-BUF imports as an RGB frame");
        if let Some((rect, popup_texture)) = popup {
            let compositor = state
                .compositor
                .get_or_insert_with(|| PopupCompositor::new(&self.output));
            let (_, _, scale) = self.output.presented_size();
            compositor.composite(
                self.output.device(),
                self.output.queue(),
                source_texture,
                popup_texture,
                target_texture,
                size,
                rect,
                f64::from(scale),
            );
        } else {
            copy_plane(
                self.output.device(),
                self.output.queue(),
                source_texture,
                target_texture,
                size,
            );
        }
        let frame = ExternalFrame::native(target.clone())
            .expect("a pool texture frame is a valid external frame");
        state.pool[index].frame = Some(target);
        // `RetiredOutput` is the documented stop signal: the host is gone,
        // so this sink stops importing and presenting until `start` hands
        // the page a fresh output.
        if self.output.present(frame).is_err() {
            state.retired = true;
        }
    }

    /// Copies the popup's transient import into its dedicated owned
    /// texture, which lives for as long as the popup is open and feeds the
    /// composite in `present_view`.
    ///
    /// # Panics
    ///
    /// Panics when the allocation or import fails, and when the texture is
    /// not an RGB image.
    fn write_popup(&self, state: &mut SinkState, source: &vulkan::Frame, fourcc: u32) {
        let source_texture = source
            .generation
            .rgb_wrap
            .as_ref()
            .expect("a CEF popup DMA-BUF imports as an RGB frame");
        let size = source.size();
        let rebuild = match &state.popup {
            Some(target) => target.descriptor.size != size || target.descriptor.fourcc != fourcc,
            None => true,
        };
        if rebuild {
            let descriptor = self
                .native
                .alloc_dmabuf(
                    size,
                    fourcc,
                    vulkan::LAYOUT_TRANSFER_DST,
                    FrameColor::SRGB,
                    RgbAlpha::Premultiplied,
                )
                .expect("CEF popup texture allocation failed");
            state.popup = Some(OwnedTarget {
                descriptor,
                frame: None,
            });
        }
        let target = state
            .popup
            .as_mut()
            .expect("the popup target was ensured above");
        let texture = self
            .native
            .import(FrameSource::DmaBuf(Box::new(
                target
                    .descriptor
                    .reopen()
                    .expect("popup DMA-BUF duplication failed"),
            )))
            .expect("CEF popup texture import failed");
        copy_plane(
            self.output.device(),
            self.output.queue(),
            source_texture,
            texture
                .generation
                .rgb_wrap
                .as_ref()
                .expect("a pool DMA-BUF imports as an RGB frame"),
            size,
        );
        // Replacing the slot drops the previous popup generation; its
        // image destruction is queued behind this copy on the engine's
        // ordered queue.
        target.frame = Some(texture);
    }
}

impl AcceleratedFrameSink for LinuxFrameSink {
    /// # Panics
    ///
    /// Panics when the frame's DMA-BUF cannot be imported on the engine's
    /// device, and on a paint element this sink does not draw.
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
        // the GPU copy it feeds is the only thing that keeps the pixels.
        let imported = self
            .native
            .import(FrameSource::DmaBuf(Box::new(dmabuf_of(frame))))
            .expect("CEF DMA-BUF import failed");
        let fourcc = format_of(frame).fourcc();
        let mut state = self.state.borrow_mut();
        match element {
            PaintElementType::VIEW => self.present_view(&mut state, &imported, fourcc),
            PaintElementType::POPUP => self.write_popup(&mut state, &imported, fourcc),
            element => panic!("CEF returned unsupported paint element {element:?}"),
        }
        // `imported` drops here, inside the callback: the imported image's
        // destruction is queued behind the copy on the engine's ordered
        // queue, and no reference to CEF's shared handle — no duplicated
        // fd, no import — outlives it.
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

/// The DMA-BUF format a CEF accelerated paint carries.
///
/// # Panics
///
/// Panics on a color type outside the ones Chromium's Linux shared images
/// use.
fn format_of(frame: &AcceleratedPaintInfo) -> DmaBufFormat {
    if frame.format == ColorType::BGRA_8888 {
        DmaBufFormat::Bgra8
    } else if frame.format == ColorType::RGBA_8888 {
        DmaBufFormat::Rgba8
    } else {
        panic!("CEF returned unsupported Linux accelerated color format")
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

/// Builds the transient [`DmaBuf`] descriptor for one accelerated paint —
/// imported and dropped inside the callback, never retained.
///
/// # Panics
///
/// Panics when the paint is not a single packed DMA-BUF plane, or its
/// geometry does not fit a `u32`.
fn dmabuf_of(frame: &AcceleratedPaintInfo) -> DmaBuf {
    assert_eq!(
        frame.plane_count, 1,
        "CEF Linux accelerated paint must provide one packed DMA-BUF plane"
    );
    let plane = &frame.planes[0];
    assert!(plane.fd >= 0, "CEF DMA-BUF file descriptor is invalid");
    // SAFETY: `borrow_raw` requires the descriptor to be open and to stay
    // open for the borrow's lifetime. CEF owns this descriptor and keeps it
    // valid for the duration of the `on_accelerated_paint` callback this
    // runs inside, which is exactly the scope of `borrowed`; it is asserted
    // non-negative just above. The borrow is only used to duplicate the
    // descriptor into an `OwnedFd`, so nothing outlives the callback and
    // CEF's own close is unaffected.
    let borrowed = unsafe { BorrowedFd::borrow_raw(plane.fd) };
    let fd: OwnedFd = borrowed
        .try_clone_to_owned()
        .expect("failed to duplicate CEF DMA-BUF file descriptor");
    let coded = &frame.extra.coded_size;
    let coded_width = u32::try_from(coded.width).expect("CEF DMA-BUF width must be positive");
    let coded_height = u32::try_from(coded.height).expect("CEF DMA-BUF height must be positive");
    // Only the region that holds the page: Chromium may allocate the shared
    // image at a coded size with alignment padding, and copying that edge
    // to edge drew the gutter.
    let visible = &frame.extra.visible_rect;
    let size = match (u32::try_from(visible.width), u32::try_from(visible.height)) {
        (Ok(visible_width), Ok(visible_height))
            if visible_width <= coded_width && visible_height <= coded_height =>
        {
            (visible_width, visible_height)
        }
        _ => (coded_width, coded_height),
    };
    DmaBuf {
        fourcc: format_of(frame).fourcc(),
        modifier: frame.modifier,
        size,
        planes: vec![DmaBufPlane {
            memory: 0,
            offset: u32::try_from(plane.offset).expect("CEF DMA-BUF offset exceeds u32"),
            stride: plane.stride,
        }],
        memory: vec![fd],
        // A shared image whose producer was never a Vulkan queue sits in
        // `VK_IMAGE_LAYOUT_GENERAL`, which is a legal copy-source layout —
        // the import declares `COPY_SRC` up front so the copy below records
        // no transition and CEF's buffer state is left untouched.
        layout: vulkan::LAYOUT_GENERAL,
        producer_family: QueueFamily::External,
        // CEF completes the shared image's writes before
        // `on_accelerated_paint` runs and the buffer returns to CEF's pool
        // when the callback returns, so the frame carries neither a wait
        // nor a release sync.
        sync: None,
        release: None,
        color: FrameColor::SRGB,
        alpha: RgbAlpha::Premultiplied,
        usage: vulkan::DmaBufUsage::CopySource,
    }
}

/// Draws the view plane and an open popup plane into one owned texture.
///
/// An [`ExternalFrame`] describes one image, and the popup is a second
/// plane: compositing them here is the only way the pair presents as the
/// layer's content. The shader is the same `cef_blit` the macOS and Windows
/// presenters draw with; the render target is the pooled texture the frame
/// presents, so there is no second pass into a producer buffer.
struct PopupCompositor {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    view_rect_buffer: wgpu::Buffer,
    popup_rect_buffer: wgpu::Buffer,
}

impl PopupCompositor {
    fn new(output: &FrameOutput) -> Self {
        let device = output.device();
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
                        min_binding_size: std::num::NonZeroU64::new(16),
                    },
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
    fn composite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::Texture,
        popup: &wgpu::Texture,
        target: &wgpu::Texture,
        size: (u32, u32),
        rect: CefPopupRect,
        scale: f64,
    ) {
        let (width, height) = size;
        write_rect(queue, &self.view_rect_buffer, [0.0, 0.0, 1.0, 1.0]);
        // The popup's rect arrives in view-logical points; `scale` is the
        // device-pixel ratio the layer presents at.
        write_rect(
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
        );
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
            pass.set_bind_group(
                0,
                &self.bind_group(device, popup, &self.popup_rect_buffer),
                &[],
            );
            pass.draw(0..6, 0..1);
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
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn write_rect(queue: &wgpu::Queue, buffer: &wgpu::Buffer, rect: [f32; 4]) {
    let mut bytes = [0; 16];
    for (source, destination) in rect.into_iter().zip(bytes.as_chunks_mut::<4>().0) {
        destination.copy_from_slice(&source.to_ne_bytes());
    }
    queue.write_buffer(buffer, 0, &bytes);
}

/// Creates the GPU view for one visible CEF page on Linux: an
/// [`ExternalFrameView`] whose source imports the page's shared DMA-BUF
/// frames on the layer's own device.
pub(super) fn gpu_view(page: CefPageHandle) -> ExternalFrameView {
    // No pump here. Chromium's message loop belongs to
    // `CefRuntime::start_message_pump`, which Chromium itself paces; running
    // `do_message_loop_work` inside the frame callback put whatever the
    // browser had queued — parsing, script, compositing — on the main thread
    // inside one frame's budget, which is what tripped the stall probe every
    // few seconds on an idle page. The source's tick installs the sink,
    // keeps the viewport in step and requests the next compositor frame,
    // nothing else.
    ExternalFrameView::new(CefExternalSource { page, output: None })
}
