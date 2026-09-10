//! Drawing surfaces nested inside a captured view subtree back into it.
//!
//! Android's HWUI records a `SurfaceView` as a cleared hole, so a `GpuSurface`
//! that lives inside an `AppliedFilter` or `ViewEffect` subtree is simply absent
//! from the buffer the subtree was captured into: the filter would receive a
//! transparent rectangle where the surface's picture belongs, and the surface's
//! own layer would keep presenting to the window on its own.
//!
//! So the surface is asked for its frame separately, into a texture of its own
//! ([`super::gpu_surface::render_composite_source`]), and that texture is drawn
//! into the capture at the rectangle the surface occupies — the same thing the
//! Apple path does with its capture compositor, expressed in wgpu.
//!
//! Everything here runs on the queue the capture copy and the filter render run
//! on, in that order, so no extra fence is involved: the buffer copy lands
//! first, the composites follow, and the filter reads the finished capture.

use std::collections::HashMap;

use super::gpu_surface::{WuiGpuSurfaceState, composite_runtime, render_composite_source};

/// Where a nested surface's frame lands in the capture it is drawn into.
#[derive(Clone, Copy, Debug)]
pub struct CompositePlacement {
    /// Left edge in capture pixels, measured from the captured subtree's origin.
    ///
    /// Signed because a surface inside a scrolled container legitimately starts
    /// left of or above the captured content; the shader clips what falls
    /// outside the capture rather than the placement being invalid.
    pub x: i32,
    /// Top edge in capture pixels, measured from the captured subtree's origin.
    pub y: i32,
    /// Width in capture pixels.
    pub width: u32,
    /// Height in capture pixels.
    pub height: u32,
    /// Capture pixels per logical unit, which is the surface's own scale.
    pub scale: f64,
}

/// The pipelines one capture target draws its nested surfaces with.
///
/// Held by the capture target that owns the destination texture, so the shader
/// is compiled once per target format and reused for every nested surface and
/// every frame. Everything is created on first use: a subtree with no nested
/// surface never pays for any of it.
#[derive(Default)]
pub struct CaptureCompositor {
    sampler: Option<wgpu::Sampler>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    uniforms: Option<wgpu::Buffer>,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

/// What one composite draw needs from the cache, borrowed for that draw.
struct CompositeResources<'a> {
    pipeline: &'a wgpu::RenderPipeline,
    bind_group_layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    uniforms: &'a wgpu::Buffer,
}

/// Floats in the `Placement` uniform: a `vec4` rectangle and a `vec2` size,
/// padded out to the 16-byte alignment WGSL gives the struct.
const PLACEMENT_UNIFORM_FIELDS: usize = 8;

/// The same uniform's size in bytes.
const PLACEMENT_UNIFORM_BYTES: usize = PLACEMENT_UNIFORM_FIELDS * size_of::<f32>();

impl CaptureCompositor {
    /// Compiles what a draw into `format` needs, reusing whatever already exists.
    fn resources(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> CompositeResources<'_> {
        let bind_group_layout = self.bind_group_layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Capture Composite Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
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
                            min_binding_size: wgpu::BufferSize::new(PLACEMENT_UNIFORM_BYTES as u64),
                        },
                        count: None,
                    },
                ],
            })
        });
        let sampler = self.sampler.get_or_insert_with(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Capture Composite Sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        });
        let uniforms = self.uniforms.get_or_insert_with(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Capture Composite Placement"),
                size: PLACEMENT_UNIFORM_BYTES as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let pipeline = self
            .pipelines
            .entry(format)
            .or_insert_with(|| create_pipeline(device, bind_group_layout, format));
        CompositeResources {
            pipeline,
            bind_group_layout,
            sampler,
            uniforms,
        }
    }
}

/// Builds the pipeline that draws a composite source into a `format` target.
fn create_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Capture Composite Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("capture_composite.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Capture Composite Pipeline Layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Capture Composite Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // A GPU surface presents premultiplied alpha, and the capture it
                // is drawn into is transparent wherever HWUI punched its hole,
                // so the surface composites over the capture exactly as its own
                // layer would have composited over the window.
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Renders `surface` and draws its frame into `target` at `placement`.
///
/// `scope` names the FFI entry point this was reached through, so a contract
/// violation says which one.
///
/// # Panics
///
/// Panics if `placement` is empty, or if the surface has never been attached
/// and so has no renderer format to be captured in.
pub fn composite_gpu_surface(
    compositor: &mut CaptureCompositor,
    surface: &mut WuiGpuSurfaceState,
    target: &wgpu::Texture,
    placement: CompositePlacement,
    scope: &'static str,
) {
    assert!(
        placement.width > 0 && placement.height > 0,
        "{scope}: a nested surface occupies no pixels at {}x{}",
        placement.width,
        placement.height
    );
    assert!(
        placement.scale.is_finite() && placement.scale > 0.0,
        "{scope}: scale must be a positive, finite device-pixel ratio, got {}",
        placement.scale
    );

    // Both textures live on the one environment-owned runtime the whole view
    // tree shares, so the surface's device is the capture's device. The runtime
    // is cloned out first because rendering the surface borrows its state.
    let runtime = composite_runtime(surface);
    let gpu = runtime.context();

    let source =
        render_composite_source(surface, placement.width, placement.height, placement.scale);
    let source_view = source.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Capture Composite Source View"),
        ..Default::default()
    });

    let resources = compositor.resources(&gpu.device, target.format());
    gpu.queue.write_buffer(
        resources.uniforms,
        0,
        &placement_uniform(placement, target.width(), target.height()),
    );

    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Capture Composite Bind Group"),
        layout: resources.bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(resources.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: resources.uniforms.as_entire_binding(),
            },
        ],
    });

    let target_view = target.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Capture Composite Target View"),
        ..Default::default()
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Capture Composite Encoder"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Capture Composite Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The capture the subtree was drawn into is what everything
                    // outside this surface's rectangle keeps.
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(resources.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
    gpu.queue.submit([encoder.finish()]);
}

/// The `Placement` uniform's bytes, in the layout the shader declares.
fn placement_uniform(
    placement: CompositePlacement,
    target_width: u32,
    target_height: u32,
) -> [u8; PLACEMENT_UNIFORM_BYTES] {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a placement is a view rectangle in pixels, far below f32's exact integer range"
    )]
    let fields: [f32; PLACEMENT_UNIFORM_FIELDS] = [
        placement.x as f32,
        placement.y as f32,
        placement.width as f32,
        placement.height as f32,
        target_width as f32,
        target_height as f32,
        0.0,
        0.0,
    ];
    let mut bytes = [0_u8; PLACEMENT_UNIFORM_BYTES];
    for (index, field) in fields.into_iter().enumerate() {
        let offset = index * size_of::<f32>();
        bytes[offset..offset + size_of::<f32>()].copy_from_slice(&field.to_ne_bytes());
    }
    bytes
}
