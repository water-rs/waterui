//! Source headroom of premultiplied linear-P3 images.

use cherenkov::RenderError;
use wgpu::hal::Device as _;

#[cfg(target_vendor = "apple")]
type NativeApi = wgpu::hal::metal::Api;
#[cfg(target_os = "android")]
type NativeApi = wgpu::hal::vulkan::Api;

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

/// Persistent native mapping read only after the capture's completion event.
#[derive(Debug)]
pub struct Readback {
    gpu: wgpu::Buffer,
    device: wgpu::Device,
    mapping: wgpu::hal::BufferMapping,
}

impl Readback {
    pub fn bytes(&self) -> u64 {
        self.gpu.size()
    }

    fn new(device: &wgpu::Device) -> Result<Self, RenderError> {
        let gpu = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture headroom"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mapping = {
            // SAFETY: the buffer and device share this native backend. HAL permits
            // persistent mappings during GPU access; no CPU read occurs until the
            // capture completion event, and no map_async operation uses this buffer.
            unsafe {
                let native = device.as_hal::<NativeApi>().expect("native device");
                let buffer = gpu.as_hal::<NativeApi>().expect("native buffer");
                native.map_buffer(&buffer, 0..4)
            }
            .map_err(|error| RenderError::Render(format!("capture headroom mapping: {error}")))?
        };
        Ok(Self {
            gpu,
            device: device.clone(),
            mapping,
        })
    }

    /// The caller must have observed completion of the GPU copy, and no later
    /// GPU use may write this immutable capture's buffer.
    pub unsafe fn read(self) -> f32 {
        if !self.mapping.is_coherent {
            // SAFETY: the caller's completion event orders the GPU write before
            // invalidation and the CPU read. The mapping covers the whole buffer.
            unsafe {
                let device = self.device.as_hal::<NativeApi>().expect("native device");
                let buffer = self.gpu.as_hal::<NativeApi>().expect("native buffer");
                device.invalidate_mapped_ranges(&buffer, std::iter::once(0..4));
            }
        }
        // SAFETY: the mapped buffer is aligned for f32 and contains the completed
        // four-byte GPU result. Drop removes the mapping only after this read.
        unsafe { self.mapping.ptr.cast::<f32>().as_ptr().read() }
    }
}

impl Drop for Readback {
    fn drop(&mut self) {
        // SAFETY: this object's unique persistent mapping stays live until Drop;
        // wgpu retains any submitted buffer use independently of this CPU mapping.
        unsafe {
            let device = self.device.as_hal::<NativeApi>().expect("native device");
            let buffer = self.gpu.as_hal::<NativeApi>().expect("native buffer");
            device.unmap_buffer(&buffer);
        }
    }
}
