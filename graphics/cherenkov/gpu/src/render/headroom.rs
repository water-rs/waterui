//! Source headroom of premultiplied images stored in linear P3 or in
//! encoded sRGB.

use cherenkov::RenderError;
use wgpu::hal::Device as _;

#[cfg(target_vendor = "apple")]
type NativeApi = wgpu::hal::metal::Api;
#[cfg(target_os = "android")]
type NativeApi = wgpu::hal::vulkan::Api;

#[derive(Debug)]
pub struct Reducer {
    layout: wgpu::BindGroupLayout,
    /// The reduction over working-space texels: every level after the
    /// first, and the first of a linear capture.
    pipeline: wgpu::RenderPipeline,
    /// The first reduction of a capture stored in encoded sRGB, which
    /// decodes each texel to the working space before taking its peak.
    decode: wgpu::RenderPipeline,
}

impl Reducer {
    pub fn new(
        device: &wgpu::Device,
        delivery: super::shaders::ShaderDelivery,
        cache: &super::PipelineCache,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("capture headroom"),
            entries: &super::layout_entries(super::bindings::HEADROOM_GROUP0),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("capture headroom"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let [linear, decode] = delivery.headroom_programs(device).map(|program| {
            super::projective::quad_pipeline(
                device,
                &pipeline_layout,
                &program,
                wgpu::TextureFormat::R32Float,
                None,
                cache,
            )
        });
        Self {
            layout,
            pipeline: linear,
            decode,
        }
    }

    /// Reduces `source`, which stores `space`, to its peak straight
    /// working-space component.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        (source, space): (&wgpu::TextureView, cherenkov::BlendSpace),
        size: (u32, u32),
    ) -> Result<Readback, RenderError> {
        let mut source = source.clone();
        let (mut width, mut height) = size;
        let staging = Readback::new(device)?;
        let mut pipeline = match space {
            cherenkov::BlendSpace::Linear => &self.pipeline,
            cherenkov::BlendSpace::SrgbEncoded => &self.decode,
        };
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
                pass.set_pipeline(pipeline);
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
            // Every later level holds working-space peaks.
            pipeline = &self.pipeline;
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

#[cfg(test)]
mod tests {
    use cherenkov::{BlendSpace, Draw, Engine, FrameTime, WorkingColor, kurbo::Rect};

    use super::Reducer;
    use crate::{
        Gpu, GpuConfig,
        interop::{SharedDevice, TextureTarget, shader_delivery},
    };

    /// The straight peak the test content reaches: its HDR rectangle's red
    /// channel, in the working space.
    const PEAK: f32 = 3.0;

    /// The measured headroom of the same content is the same on a surface
    /// storing the working space and on one storing encoded sRGB, and only
    /// the decoding reduction recovers it from the encoded store.
    #[test]
    fn headroom_is_independent_of_the_storage_space() -> Result<(), Box<dyn std::error::Error>> {
        let shared = SharedDevice::create(&GpuConfig::default())?;
        let device = shared.device.clone();
        let queue = shared.queue.clone();
        let delivery = shader_delivery(shared.adapter.get_info().backend, &device)?;
        let engine = Engine::<Gpu>::new(GpuConfig {
            device: Some(shared),
            ..GpuConfig::default()
        })?;
        let size = (96, 40);
        let mut textures = Vec::new();
        let mut surfaces = Vec::new();
        for space in [BlendSpace::Linear, BlendSpace::SrgbEncoded] {
            let (target, receiver) = TextureTarget::new(size);
            let surface = engine.surface(target, || {})?;
            surface.storage_space(space);
            surface.clear_color(WorkingColor::new([0.1, 0.1, 0.1, 1.0]));
            surface.update(|tx| {
                tx[surface.root()].content(surface.record(|r| {
                    r.fill(
                        Rect::new(4.0, 4.0, 36.0, 36.0),
                        WorkingColor::new([PEAK, 0.5, 0.25, 1.0]),
                    );
                    r.fill(
                        Rect::new(48.0, 4.0, 92.0, 36.0),
                        WorkingColor::new([0.2, 0.4, 0.8, 0.5]),
                    );
                }));
            });
            textures.push((receiver, space));
            surfaces.push(surface);
        }
        engine.render(FrameTime::now())?;
        let reducer = Reducer::new(&device, delivery, &super::super::PipelineCache::default());
        let measure = |texture: &wgpu::Texture,
                       space: BlendSpace|
         -> Result<f32, Box<dyn std::error::Error>> {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("headroom test"),
            });
            let readback = reducer.encode(&device, &mut encoder, (&view, space), size)?;
            let submission = queue.submit([encoder.finish()]);
            device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })?;
            // SAFETY: the wait above completed the submission that wrote
            // the readback, and nothing writes it again.
            Ok(unsafe { readback.read() })
        };
        let mut rendered = Vec::new();
        for (receiver, space) in &textures {
            rendered.push((receiver.try_recv()?, *space));
        }
        let [(linear, _), (encoded, encoded_space)] = &rendered[..] else {
            unreachable!("two surfaces were rendered");
        };
        let linear_peak = measure(linear, BlendSpace::Linear)?;
        let encoded_peak = measure(encoded, *encoded_space)?;
        // The encoded store holds half-float encoded values; the decode
        // recovers the peak to within that store's precision.
        assert!((linear_peak - PEAK).abs() < 1e-3, "linear {linear_peak}");
        assert!(
            (encoded_peak - linear_peak).abs() < 1e-2,
            "linear {linear_peak}, encoded {encoded_peak}"
        );
        let undecoded = measure(encoded, BlendSpace::Linear)?;
        assert!(
            (undecoded - linear_peak).abs() > 0.5,
            "the encoded store read as linear reports {undecoded}"
        );
        drop(surfaces);
        Ok(())
    }
}
