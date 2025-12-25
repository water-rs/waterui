//! Filter pipeline implementation.

use std::sync::Arc;

use wgpu::util::DeviceExt;

use crate::{Filter, uniform::FilterUniform};

/// Pipeline for applying GPU filters to textures.
///
/// `FilterPipeline` manages the wgpu resources needed to apply filters
/// to any texture. It can be created once and reused for multiple filter
/// operations.
///
/// # Example
///
/// ```ignore
/// let pipeline = FilterPipeline::new(&device, &queue);
///
/// // Apply filters
/// pipeline.apply(&input, &output, &[
///     Filter::Blur { radius: 5.0 },
///     Filter::Brightness { amount: 0.1 },
/// ]);
/// ```
pub struct FilterPipeline {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    compute_pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl core::fmt::Debug for FilterPipeline {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilterPipeline").finish_non_exhaustive()
    }
}

impl FilterPipeline {
    /// Create a new filter pipeline.
    ///
    /// # Arguments
    ///
    /// * `device` - The wgpu device for creating GPU resources
    /// * `queue` - The wgpu queue for submitting commands
    #[must_use]
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        // Load shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filtrate shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/filters.wgsl").into()),
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filtrate bind group layout"),
            entries: &[
                // Input texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Output texture (storage)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                // Uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filtrate pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create compute pipeline
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("filtrate compute pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filtrate sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            device,
            queue,
            compute_pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// Apply filters to an input texture, writing the result to an output texture.
    ///
    /// Filters are applied in order. The input and output textures must have
    /// the same dimensions and format (Rgba8Unorm).
    ///
    /// # Arguments
    ///
    /// * `input` - Source texture to read from
    /// * `output` - Destination texture to write to
    /// * `filters` - Slice of filters to apply in sequence
    pub fn apply(&self, input: &wgpu::Texture, output: &wgpu::Texture, filters: &[Filter]) {
        if filters.is_empty() {
            // No filters, just copy
            self.copy_texture(input, output);
            return;
        }

        let width = input.width();
        let height = input.height();

        // For single filter, apply directly
        if filters.len() == 1 {
            self.apply_single_filter(input, output, &filters[0], width, height);
            return;
        }

        // For multiple filters, we need intermediate textures
        // Create two scratch textures for ping-pong
        let scratch_a = self.create_scratch_texture(width, height);
        let scratch_b = self.create_scratch_texture(width, height);

        let mut current_input = input;
        let mut current_output = &scratch_a;

        for (i, filter) in filters.iter().enumerate() {
            let is_last = i == filters.len() - 1;

            if is_last {
                // Last filter writes to final output
                self.apply_single_filter(current_input, output, filter, width, height);
            } else {
                // Intermediate filter
                self.apply_single_filter(current_input, current_output, filter, width, height);

                // Swap for next iteration
                if current_output as *const _ == &scratch_a as *const _ {
                    current_input = &scratch_a;
                    current_output = &scratch_b;
                } else {
                    current_input = &scratch_b;
                    current_output = &scratch_a;
                }
            }
        }
    }

    /// Apply filters in-place using an internal scratch texture.
    ///
    /// The result is written back to the same texture.
    ///
    /// # Arguments
    ///
    /// * `texture` - Texture to apply filters to (in-place)
    /// * `filters` - Slice of filters to apply in sequence
    pub fn apply_in_place(&self, texture: &wgpu::Texture, filters: &[Filter]) {
        if filters.is_empty() {
            return;
        }

        let width = texture.width();
        let height = texture.height();
        let scratch = self.create_scratch_texture(width, height);

        // Apply to scratch
        self.apply(texture, &scratch, filters);

        // Copy back
        self.copy_texture(&scratch, texture);
    }

    fn apply_single_filter(
        &self,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        filter: &Filter,
        width: u32,
        height: u32,
    ) {
        // Create uniform buffer
        let uniform = FilterUniform::from_filter(filter, width, height);
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filtrate uniform buffer"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Create texture views
        let input_view = input.create_view(&wgpu::TextureViewDescriptor::default());
        let output_view = output.create_view(&wgpu::TextureViewDescriptor::default());

        // Create bind group
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filtrate bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        // Create and submit command buffer
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filtrate encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("filtrate compute pass"),
                timestamp_writes: None,
            });

            compute_pass.set_pipeline(&self.compute_pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);

            // Dispatch workgroups (8x8 threads per workgroup)
            let workgroups_x = (width + 7) / 8;
            let workgroups_y = (height + 7) / 8;
            compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        }

        self.queue.submit([encoder.finish()]);
    }

    fn create_scratch_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filtrate scratch texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn copy_texture(&self, src: &wgpu::Texture, dst: &wgpu::Texture) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filtrate copy encoder"),
            });

        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: dst,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: src.width(),
                height: src.height(),
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit([encoder.finish()]);
    }
}
