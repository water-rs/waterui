//! Source headroom of immutable premultiplied linear-P3 captures.

use cherenkov::RenderError;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLBuffer, MTLDevice, MTLResourceOptions};

#[derive(Debug)]
pub struct Reducer {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl Reducer {
    pub fn new(device: &wgpu::Device, delivery: super::shaders::ShaderDelivery) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("capture headroom"),
            entries: &super::layout_entries(super::bindings::HEADROOM_GROUP0),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("capture headroom"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = super::projective::quad_pipeline(
            device,
            &pipeline_layout,
            &delivery.headroom_program(device),
            wgpu::TextureFormat::R32Float,
            None,
        );
        Self { layout, pipeline }
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        size: (u32, u32),
    ) -> Result<Readback, RenderError> {
        let mut source = source.clone();
        let (mut width, mut height) = size;
        let staging = Readback::new(device)?;
        loop {
            width = width.div_ceil(16);
            height = height.div_ceil(16);
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("capture headroom level"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("capture headroom"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source),
                }],
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("capture headroom"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            if width == 1 && height == 1 {
                encoder.copy_texture_to_buffer(
                    target.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &staging.gpu,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: None,
                            rows_per_image: None,
                        },
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                return Ok(staging);
            }
            source = view;
        }
    }
}

/// Shared Metal memory read only after the capture's completion event.
#[derive(Debug)]
pub struct Readback {
    gpu: wgpu::Buffer,
    raw: Retained<ProtocolObject<dyn MTLBuffer>>,
}

impl Readback {
    pub fn bytes(&self) -> u64 {
        self.gpu.size()
    }

    fn new(device: &wgpu::Device) -> Result<Self, RenderError> {
        // SAFETY: this module is the native Metal capture path.
        let metal = unsafe { device.as_hal::<wgpu::hal::metal::Api>() }.expect("Metal device");
        let raw = metal
            .raw_device()
            .newBufferWithLength_options(4, MTLResourceOptions::StorageModeShared)
            .ok_or_else(|| RenderError::Render("Metal refused capture headroom buffer".into()))?;
        // SAFETY: a new four-byte shared buffer is CPU-owned until submission;
        // initialize it before importing into wgpu, whose import requires it.
        unsafe { raw.contents().cast::<f32>().as_ptr().write(1.0) };
        // SAFETY: the retained buffer belongs to this device, has four initialized
        // bytes and shared storage. Its GPU use is copy destination only.
        let gpu = unsafe {
            device.create_buffer_from_hal::<wgpu::hal::metal::Api>(
                wgpu::hal::metal::Device::buffer_from_raw(raw.clone(), 4),
                &wgpu::BufferDescriptor {
                    label: Some("capture headroom"),
                    size: 4,
                    usage: wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            )
        };
        Ok(Self { gpu, raw })
    }

    /// The caller must have observed completion of the GPU copy, and no later
    /// GPU use may write this immutable capture's buffer.
    pub unsafe fn read(self) -> f32 {
        // SAFETY: the caller's completion event orders the GPU copy before this
        // CPU read of coherent shared memory on the supported ARM64 Apple devices.
        unsafe { self.raw.contents().cast::<f32>().as_ptr().read() }
    }
}
