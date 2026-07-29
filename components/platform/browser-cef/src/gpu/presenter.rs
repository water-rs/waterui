use std::cell::RefCell;
use std::rc::Rc;

use cef::PaintElementType;
use num_traits::ToPrimitive as _;
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame};

use crate::CefPopupRect;

pub(super) struct OwnedFrameMailbox {
    view_frame: RefCell<Option<wgpu::Texture>>,
    popup_frame: RefCell<Option<wgpu::Texture>>,
    popup_rect: RefCell<Option<CefPopupRect>>,
    waker: RefCell<Option<Rc<dyn Fn()>>>,
}

impl OwnedFrameMailbox {
    pub(super) fn new() -> Self {
        Self {
            view_frame: RefCell::new(None),
            popup_frame: RefCell::new(None),
            popup_rect: RefCell::new(None),
            waker: RefCell::new(None),
        }
    }

    pub(super) fn set_waker(&self, waker: Rc<dyn Fn()>) {
        self.waker.replace(Some(waker));
    }

    pub(super) fn publish(&self, element: PaintElementType, frame: wgpu::Texture) {
        match element {
            PaintElementType::VIEW => {
                self.view_frame.replace(Some(frame));
            }
            PaintElementType::POPUP => {
                self.popup_frame.replace(Some(frame));
            }
            element => panic!("CEF returned unsupported paint element {element:?}"),
        }
        self.wake();
    }

    pub(super) fn set_popup_rect(&self, rect: Option<CefPopupRect>) {
        self.popup_rect.replace(rect);
        if rect.is_none() {
            self.popup_frame.borrow_mut().take();
        }
        self.wake();
    }

    fn wake(&self) {
        if let Some(waker) = self.waker.borrow().as_ref() {
            waker();
        }
    }

    pub(super) fn take_view(&self) -> Option<wgpu::Texture> {
        self.view_frame.borrow_mut().take()
    }

    pub(super) fn take_popup(&self) -> Option<wgpu::Texture> {
        self.popup_frame.borrow_mut().take()
    }

    pub(super) fn popup_rect(&self) -> Option<CefPopupRect> {
        *self.popup_rect.borrow()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn copy_source_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Texture,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    let destination = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("waterui_cef_owned_frame"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("waterui_cef_frame_copy"),
    });
    encoder.copy_texture_to_texture(source.as_image_copy(), destination.as_image_copy(), size);
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .expect("CEF accelerated frame GPU copy failed");
    destination
}

pub(super) struct TexturePresenter {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
    source: Option<SourceTexture>,
    popup: Option<SourceTexture>,
    popup_rect: Option<CefPopupRect>,
    view_rect_buffer: wgpu::Buffer,
    popup_rect_buffer: wgpu::Buffer,
}

struct SourceTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl TexturePresenter {
    pub(super) fn new(context: &GpuContext<'_>) -> Self {
        let shader = context
            .device
            .create_shader_module(wgpu::include_wgsl!("cef_blit.wgsl"));
        let bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("waterui_cef_bind_group_layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                    ],
                });
        let pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("waterui_cef_pipeline_layout"),
                    bind_group_layouts: &[Some(&bind_group_layout)],
                    immediate_size: 0,
                });
        let pipeline = context
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("waterui_cef_pipeline"),
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
                        format: context.surface_format,
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
        let sampler = context.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("waterui_cef_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            bind_group_layout,
            sampler,
            target_format: context.surface_format,
            source: None,
            popup: None,
            popup_rect: None,
            view_rect_buffer: create_rect_buffer(context.device, "waterui_cef_view_rect"),
            popup_rect_buffer: create_rect_buffer(context.device, "waterui_cef_popup_rect"),
        }
    }

    pub(super) fn set_source(&mut self, texture: wgpu::Texture) {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.source = Some(SourceTexture {
            _texture: texture,
            view,
        });
    }

    pub(super) fn set_popup_source(&mut self, texture: wgpu::Texture) {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.popup = Some(SourceTexture {
            _texture: texture,
            view,
        });
    }

    pub(super) fn set_popup_rect(&mut self, rect: Option<CefPopupRect>) {
        self.popup_rect = rect;
        if rect.is_none() {
            self.popup = None;
        }
    }

    pub(super) fn render(&self, frame: &mut GpuFrame<'_>, scale: f64) -> bool {
        assert_eq!(
            frame.format, self.target_format,
            "CEF target format changed after setup"
        );
        let Some(source) = self.source.as_ref() else {
            clear_target(frame);
            frame.request_redraw();
            return false;
        };
        write_rect(frame.queue, &self.view_rect_buffer, [0.0, 0.0, 1.0, 1.0]);
        let bind_group = self.create_bind_group(frame.device, source, &self.view_rect_buffer);
        let popup_bind_group = self
            .popup
            .as_ref()
            .zip(self.popup_rect)
            .map(|(popup, rect)| {
                let width = f64::from(frame.width);
                let height = f64::from(frame.height);
                write_rect(
                    frame.queue,
                    &self.popup_rect_buffer,
                    [
                        (f64::from(rect.x) * scale / width)
                            .to_f32()
                            .expect("CEF popup x exceeds f32"),
                        (f64::from(rect.y) * scale / height)
                            .to_f32()
                            .expect("CEF popup y exceeds f32"),
                        (f64::from(rect.width) * scale / width)
                            .to_f32()
                            .expect("CEF popup width exceeds f32"),
                        (f64::from(rect.height) * scale / height)
                            .to_f32()
                            .expect("CEF popup height exceeds f32"),
                    ],
                );
                self.create_bind_group(frame.device, popup, &self.popup_rect_buffer)
            });
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("waterui_cef_encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("waterui_cef_blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..6, 0..1);
            if let Some(popup_bind_group) = popup_bind_group.as_ref() {
                pass.set_bind_group(0, popup_bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        frame.queue.submit([encoder.finish()]);
        true
    }

    fn create_bind_group(
        &self,
        device: &wgpu::Device,
        source: &SourceTexture,
        rect: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
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
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: rect.as_entire_binding(),
                },
            ],
        })
    }
}

fn create_rect_buffer(device: &wgpu::Device, label: &'static str) -> wgpu::Buffer {
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

fn clear_target(frame: &GpuFrame<'_>) {
    let mut encoder = frame
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui_cef_empty_encoder"),
        });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("waterui_cef_empty"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame.view,
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
    }
    frame.queue.submit([encoder.finish()]);
}
