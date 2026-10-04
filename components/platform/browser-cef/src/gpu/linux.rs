//! CEF's Linux GPU path.
//!
//! Chromium hands each accelerated paint over as a DMA-BUF shared image.
//! The sink imports the plane in place on the engine's Vulkan device —
//! nothing copies it — and publishes the frame through [`FrameOutput`],
//! where it becomes the content of the view's own engine layer. While a
//! popup widget is open the two planes still arrive separately, so the sink
//! draws both into one owned texture and presents that instead; the steady
//! state stays zero-copy.

use std::cell::RefCell;
use std::os::fd::{BorrowedFd, OwnedFd};

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use num_traits::ToPrimitive as _;
use waterui_graphics::cherenkov_gpu::interop::vulkan::{
    self, DmaBuf, DmaBufPlane, FrameSource, QueueFamily,
};
use waterui_graphics::cherenkov_gpu::interop::{ExternalFrame, FrameColor, RgbAlpha};
use waterui_graphics::gpu::{ExternalFrameSource, ExternalFrameView, FrameOutput};
use wgpu_external_frame::dma_buf::DmaBufFormat;

use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

/// The format of the owned texture a popup composite renders into: what
/// Chromium's BGRA/XBGRA shared images decode to, in premultiplied alpha.
const COMPOSITE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

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

/// The frames one [`FrameOutput`] owns: the newest imported plane per paint
/// element, the popup's target rect, and the compositor for when it is open.
struct SinkState {
    view: Option<vulkan::Frame>,
    popup: Option<vulkan::Frame>,
    popup_rect: Option<CefPopupRect>,
    compositor: Option<PopupCompositor>,
    retired: bool,
}

/// Chromium's paint callback: imports each element's DMA-BUF onto the output's
/// device and presents the page's current frame.
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
                view: None,
                popup: None,
                popup_rect: None,
                compositor: None,
                retired: false,
            }),
        }
    }

    /// Publishes the newest view frame, compositing the popup over it while
    /// one is open.
    fn present(&self, state: &mut SinkState) {
        let Some(view) = &state.view else {
            return;
        };
        let frame = match (state.popup_rect, state.popup.as_ref()) {
            (Some(rect), Some(popup)) => {
                let compositor = state
                    .compositor
                    .get_or_insert_with(|| PopupCompositor::new(&self.output));
                let (_, _, scale) = self.output.presented_size();
                let texture = compositor.composite(
                    self.output.device(),
                    self.output.queue(),
                    view,
                    popup,
                    rect,
                    f64::from(scale),
                );
                ExternalFrame::rgb(texture, RgbAlpha::Premultiplied, FrameColor::SRGB)
                    .expect("the composited browser frame is a valid external frame")
            }
            _ => ExternalFrame::native(view.clone())
                .expect("an imported CEF view frame is a valid external frame"),
        };
        // `RetiredOutput` is the documented stop signal: the host is gone,
        // so this sink stops importing and presenting until `start` hands
        // the page a fresh output.
        if self.output.present(frame).is_err() {
            state.retired = true;
        }
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
        let imported = self
            .native
            .import(FrameSource::DmaBuf(Box::new(dmabuf_of(frame))))
            .expect("CEF DMA-BUF import failed");
        let mut state = self.state.borrow_mut();
        match element {
            PaintElementType::VIEW => {
                state.view = Some(imported);
            }
            PaintElementType::POPUP => {
                state.popup = Some(imported);
            }
            element => panic!("CEF returned unsupported paint element {element:?}"),
        }
        self.present(&mut state);
    }

    fn set_popup_rect(&self, rect: Option<CefPopupRect>) {
        let mut state = self.state.borrow_mut();
        state.popup_rect = rect;
        if rect.is_none() {
            state.popup = None;
        }
        self.present(&mut state);
    }
}

/// Builds the [`DmaBuf`] descriptor for one accelerated paint.
///
/// # Panics
///
/// Panics when the paint is not a single packed DMA-BUF plane, its color
/// format is not one Chromium's Linux shared images use, or its geometry
/// does not fit a `u32`.
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
    // image at a coded size with alignment padding, and presenting that edge
    // to edge stretched the page and drew the gutter.
    let visible = &frame.extra.visible_rect;
    let size = match (u32::try_from(visible.width), u32::try_from(visible.height)) {
        (Ok(visible_width), Ok(visible_height))
            if visible_width <= coded_width && visible_height <= coded_height =>
        {
            (visible_width, visible_height)
        }
        _ => (coded_width, coded_height),
    };
    let format = if frame.format == ColorType::BGRA_8888 {
        DmaBufFormat::Bgra8
    } else if frame.format == ColorType::RGBA_8888 {
        DmaBufFormat::Rgba8
    } else {
        panic!("CEF returned unsupported Linux accelerated color format")
    };
    DmaBuf {
        fourcc: format.fourcc(),
        modifier: frame.modifier,
        size,
        planes: vec![DmaBufPlane {
            memory: 0,
            offset: u32::try_from(plane.offset).expect("CEF DMA-BUF offset exceeds u32"),
            stride: plane.stride,
        }],
        memory: vec![fd],
        // A shared image whose producer was never a Vulkan queue sits in
        // `VK_IMAGE_LAYOUT_GENERAL`; that is also the layout the engine's
        // release barrier must hand back.
        layout: vulkan::LAYOUT_GENERAL,
        producer_family: QueueFamily::External,
        // CEF completes the shared image's writes before
        // `on_accelerated_paint` runs and never wants the buffer back, so
        // the frame carries neither a wait nor a release sync.
        sync: None,
        release: None,
        color: FrameColor::SRGB,
        alpha: RgbAlpha::Premultiplied,
    }
}

/// Draws the view plane and an open popup plane into one owned texture.
///
/// An [`ExternalFrame`] describes one image, and the popup is a second
/// plane: compositing them here is the only way the pair presents as the
/// layer's content. The shader is the same `cef_blit` the macOS and Windows
/// presenters draw with.
struct PopupCompositor {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    view_rect_buffer: wgpu::Buffer,
    popup_rect_buffer: wgpu::Buffer,
    target: Option<wgpu::Texture>,
    target_view: Option<wgpu::TextureView>,
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
            target: None,
            target_view: None,
        }
    }

    /// Renders `view` under `popup` into an owned texture at the view frame's
    /// extent and returns it.
    ///
    /// # Panics
    ///
    /// Panics when either frame is not an RGB image (a dmabuf that imported
    /// as planar YUV carries no texture to sample).
    fn composite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &vulkan::Frame,
        popup: &vulkan::Frame,
        rect: CefPopupRect,
        scale: f64,
    ) -> wgpu::Texture {
        let (width, height) = view.size();
        if !matches!(self.target, Some(ref t) if t.width() == width && t.height() == height) {
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("waterui_cef_composite"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COMPOSITE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.target_view = Some(target.create_view(&wgpu::TextureViewDescriptor::default()));
            self.target = Some(target);
        }
        let view_texture = view
            .generation
            .rgb_wrap
            .as_ref()
            .expect("a CEF view DMA-BUF imports as an RGB frame");
        let popup_texture = popup
            .generation
            .rgb_wrap
            .as_ref()
            .expect("a CEF popup DMA-BUF imports as an RGB frame");
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
        let target_view = self
            .target_view
            .as_ref()
            .expect("the composite target was created above");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui_cef_composite"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("waterui_cef_composite_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
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
                &self.bind_group(device, view_texture, &self.view_rect_buffer),
                &[],
            );
            pass.draw(0..6, 0..1);
            pass.set_bind_group(
                0,
                &self.bind_group(device, popup_texture, &self.popup_rect_buffer),
                &[],
            );
            pass.draw(0..6, 0..1);
        }
        queue.submit([encoder.finish()]);
        self.target
            .clone()
            .expect("the composite target was created above")
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
