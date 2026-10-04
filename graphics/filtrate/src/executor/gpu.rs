//! The GPU side of the reference executor: pipelines, bindings,
//! intermediates, and command encoding.

extern crate alloc;

use alloc::{borrow::Cow, string::ToString, vec::Vec};

use cherenkov_shader::{SamplerFilter, SegmentArg, naga::Module};
use filtrate_core::{AuxFormat, AuxImage, Filter, ImageVisitor, ShapeInput, WorkingSpace};

use super::{
    entry::{self, ENTRY_POINT, binding},
    plan::{PassAux, PassPlan, Plan},
    uniforms::UniformBuffers,
};
use crate::{
    aux_image::TextureImage,
    effect::{EffectContext, EffectInput, EffectOutput, EffectRenderError, EffectSetupError},
};

/// The format every intermediate is materialized in: f16, so every
/// materialization point rounds the same way and extended values survive.
pub(super) const INTERMEDIATE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Choices the composer uses when planning the reference passes.
pub(super) struct PlanOptions {
    pub input_filterable: bool,
    pub intermediate_filterable: bool,
    pub fold: bool,
}

/// This encode's uniform buffers, already selected for its frame sequence.
#[derive(Clone, Copy)]
struct PassUniforms<'a> {
    params: Option<&'a wgpu::Buffer>,
    output_scale: Option<&'a wgpu::Buffer>,
}

impl PlanOptions {
    fn compose<F: Filter>(
        &self,
        filter: &F,
        ctx: &EffectContext<'_>,
    ) -> Result<Plan, EffectSetupError> {
        let plan = Plan::new(
            filter,
            self.input_filterable,
            self.intermediate_filterable,
            self.fold,
        )?;
        for (index, pass) in plan.passes.iter().enumerate() {
            let (format, filterable) = if index == 0 {
                (ctx.input_format, self.input_filterable)
            } else {
                (INTERMEDIATE_FORMAT, self.intermediate_filterable)
            };
            if pass.sampler == Some(SamplerFilter::Filtered) && !filterable {
                return Err(EffectSetupError::InputNotFilterable { format });
            }
        }
        Ok(plan)
    }
}

/// The full-screen triangle every pass draws.
const VERTEX_SHADER: &str = include_str!("../shaders/fullscreen.wgsl");

/// One pass's pipeline and uniform block.
#[derive(Debug)]
pub(super) struct GpuPass {
    plan: PassPlan,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The segment's uniform buffers, one per distinct block an encode of
    /// the current frame bound (see [`UniformBuffers`]).
    pub(super) params: UniformBuffers,
    output_scale: Option<UniformBuffers>,
    uses_space: bool,
}

/// An auxiliary image's binding: CPU data uploaded at its native
/// precision, or a caller-provided texture bound in place.
#[derive(Debug)]
enum Image {
    /// The executor's own upload.
    Uploaded {
        view: wgpu::TextureView,
        _texture: wgpu::Texture,
    },
    /// The caller's texture, at its native format.
    Bound {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
    },
}

impl Image {
    /// The view a pass binds.
    const fn view(&self) -> &wgpu::TextureView {
        match self {
            Self::Uploaded { view, .. } | Self::Bound { view, .. } => view,
        }
    }
}

/// The intermediate slots for one input size.
#[derive(Debug)]
pub(super) struct Intermediates {
    pub(super) size: (u32, u32),
    /// The frame sequence that last encoded through these slots; a size
    /// unused for a whole frame is dropped rather than retained.
    pub(super) last_used: u64,
    pub(super) views: Vec<wgpu::TextureView>,
}

/// Bindings every pass may read.
#[derive(Debug)]
struct Shared {
    point_sampler: wgpu::Sampler,
    filtering_sampler: wgpu::Sampler,
    /// The working-space constants.
    space: wgpu::Buffer,
    images: Vec<Image>,
}

/// Everything setup produced for one device and pair of formats.
#[derive(Debug)]
pub(super) struct Gpu {
    pub(super) passes: Vec<GpuPass>,
    /// The intermediate slot pass `n` writes, for every pass but the last,
    /// which writes the output.
    slot_of: Vec<usize>,
    slot_count: usize,
    shared: Shared,
    /// Intermediate slots per input size, capped; a frame can encode one
    /// effect against several sizes (backdrop capture regions).
    pub(super) intermediates: Vec<Intermediates>,
    input_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,
}

impl Gpu {
    /// Composes `filter` and builds its pipelines.
    #[expect(
        clippy::future_not_send,
        reason = "setup borrows the filter and the device-bound context on the GPU host thread"
    )]
    pub(super) async fn new<F: Filter>(
        filter: &F,
        ctx: &EffectContext<'_>,
        resizes_output: bool,
    ) -> Result<Self, EffectSetupError> {
        let features = ctx.device.features();
        Self::with_options(
            filter,
            ctx,
            PlanOptions {
                input_filterable: filterable(ctx.input_format, features),
                intermediate_filterable: filterable(INTERMEDIATE_FORMAT, features),
                fold: false,
            },
            resizes_output,
        )
        .await
    }

    /// `new` with the plan options given rather than queried — tests force
    /// the manual path or pick the folded segments through it.
    #[expect(
        clippy::future_not_send,
        reason = "setup borrows the filter and the device-bound context on the GPU host thread"
    )]
    pub(super) async fn with_options<F: Filter>(
        filter: &F,
        ctx: &EffectContext<'_>,
        options: PlanOptions,
        resizes_output: bool,
    ) -> Result<Self, EffectSetupError> {
        let plan = options.compose(filter, ctx)?;
        // Slots first: a single-pass chain never materializes an
        // intermediate and must not probe for one.
        let (slot_of, slot_count) = assign_slots(&plan.passes);
        if slot_count > 0 {
            probe_intermediate_format(ctx.device).await?;
        }

        let vertex = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("filtrate full-screen triangle"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(VERTEX_SHADER)),
            });
        let last = plan.passes.len() - 1;
        let mut passes = Vec::with_capacity(plan.passes.len());
        for (index, pass) in plan.passes.into_iter().enumerate() {
            let resizes_output = resizes_output && index == last;
            let module = entry::pass_module(
                &plan.module,
                plan.capabilities,
                index,
                &pass,
                resizes_output,
            )?;
            let format = if index == last {
                ctx.output_format
            } else {
                INTERMEDIATE_FORMAT
            };
            passes.push(
                GpuPass::new(
                    ctx.device,
                    &vertex,
                    module,
                    pass,
                    index,
                    format,
                    resizes_output,
                )
                .await?,
            );
        }

        let space = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filtrate working space"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let luma = WorkingSpace::LINEAR_DISPLAY_P3.luma;
        ctx.queue.write_buffer(
            &space,
            0,
            bytemuck::cast_slice(&[luma[0], luma[1], luma[2], 0.0]),
        );

        let mut uploader = Uploader {
            ctx,
            images: (0..F::IMAGES).map(|_| None).collect(),
        };
        filter.visit_images(&mut uploader);
        let images: Vec<Image> = uploader
            .images
            .into_iter()
            .enumerate()
            .map(|(index, image)| {
                image.unwrap_or_else(|| {
                    panic!(
                        "the filter declares {} images but never visited image {index}",
                        F::IMAGES
                    )
                })
            })
            .collect();

        check_bound_textures(&passes, &images, ctx.device)?;

        Ok(Self {
            passes,
            slot_of,
            slot_count,
            shared: Shared {
                point_sampler: sampler(ctx.device, wgpu::FilterMode::Nearest),
                filtering_sampler: sampler(ctx.device, wgpu::FilterMode::Linear),
                space,
                images,
            },
            intermediates: Vec::new(),
            input_format: ctx.input_format,
            output_format: ctx.output_format,
        })
    }

    /// The stage ranges each pass's segment covers — for tests asserting a
    /// folded segment was selected.
    #[cfg(test)]
    pub(super) fn segment_stages(&self) -> Vec<core::ops::Range<usize>> {
        self.passes
            .iter()
            .map(|pass| pass.plan.segment.stages.clone())
            .collect()
    }

    /// Encodes every pass, reading the parameters from `values`.
    pub(super) fn encode(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
        values: &[f32],
    ) -> Result<(), EffectRenderError> {
        if input.format != self.input_format || output.format != self.output_format {
            return Err(EffectRenderError::FormatMismatch {
                input: input.format,
                output: output.format,
                setup_input: self.input_format,
                setup_output: self.output_format,
            });
        }
        let size = (input.width, input.height);
        for pass in &self.passes {
            if let Some(shape) = pass.plan.shape {
                shape_view(input, shape)?;
            }
        }
        let sequence = input.timing.sequence();
        // A used size is live this frame: mark it before stale sizes are
        // dropped so steady state reuses its slots.
        if let Some(index) = self
            .intermediates
            .iter()
            .position(|intermediates| intermediates.size == size)
        {
            self.intermediates[index].last_used = sequence;
        }
        // Keep every size this or the previous frame used: a frame that
        // encodes several sizes would otherwise drop the sizes encoded
        // before this call and reallocate them on the next frame.
        self.intermediates
            .retain(|intermediates| intermediates.last_used.saturating_add(1) >= sequence);
        let intermediates_index = self
            .intermediates
            .iter()
            .position(|intermediates| intermediates.size == size)
            .unwrap_or_else(|| {
                const MAX_INTERMEDIATE_SETS: usize = 4;
                if self.intermediates.len() == MAX_INTERMEDIATE_SETS {
                    self.intermediates.remove(0);
                }
                self.intermediates.push(Intermediates::new(
                    input.device,
                    size,
                    self.slot_count,
                    sequence,
                ));
                self.intermediates.len() - 1
            });

        let slots = &self.intermediates[intermediates_index].views;
        let slot_of = &self.slot_of;
        let last = self.passes.len() - 1;
        for (index, pass) in self.passes.iter_mut().enumerate() {
            let output_scale = pass.output_scale.as_mut().map(|buffers| {
                buffers.select(
                    input.device,
                    input.queue,
                    scale_words(size, (output.width, output.height)),
                    sequence,
                )
            });
            let params = pass
                .params_index(input.device, input.queue, values, size, sequence)
                .map(|index| pass.params.buffer(index));

            let output_scale = output_scale.map(|index| {
                pass.output_scale
                    .as_ref()
                    .expect("a resized pass owns scale buffers")
                    .buffer(index)
            });
            let bind_group = pass.bind_group(
                index,
                input,
                &self.shared,
                slots,
                slot_of,
                PassUniforms {
                    params,
                    output_scale,
                },
            )?;

            let target = if index == last {
                &output.view
            } else {
                &slots[slot_of[index]]
            };
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("filtrate pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
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
            render_pass.set_pipeline(&pass.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }
        Ok(())
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "GPU texture dimensions fit f32 exactly"
)]
fn scale_words(input: (u32, u32), output: (u32, u32)) -> Vec<u32> {
    alloc::vec![
        (input.0 as f32 / output.0 as f32).to_bits(),
        (input.1 as f32 / output.1 as f32).to_bits(),
    ]
}

impl GpuPass {
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "the wasm32 WebGPU device is !Send; the executor's futures run on the page's event loop"
        )
    )]
    async fn new(
        device: &wgpu::Device,
        vertex: &wgpu::ShaderModule,
        module: Module,
        plan: PassPlan,
        index: usize,
        format: wgpu::TextureFormat,
        resizes_output: bool,
    ) -> Result<Self, EffectSetupError> {
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filtrate pass layout"),
            entries: &layout_entries(&plan, resizes_output),
        });
        let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filtrate pass"),
            source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filtrate pass pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("filtrate pass pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex,
                entry_point: Some("main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some(ENTRY_POINT),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = error_scope.pop().await {
            let message = error.to_string();
            tracing::error!("[filtrate] pass {index} pipeline validation failed: {message}");
            return Err(EffectSetupError::PipelineValidation {
                pass: index,
                message,
            });
        }

        let uses_space = plan.segment.args.contains(&SegmentArg::WorkingSpace);
        Ok(Self {
            plan,
            pipeline,
            layout,
            params: UniformBuffers::new("filtrate pass parameters"),
            output_scale: resizes_output.then(|| UniformBuffers::new("filtrate output scale")),
            uses_space,
        })
    }

    /// Binds the pass's inputs for one frame. `params` is the uniform
    /// buffer [`Self::params_index`] selected for this encode.
    fn bind_group(
        &self,
        index: usize,
        input: &EffectInput<'_>,
        shared: &Shared,
        slots: &[wgpu::TextureView],
        slot_of: &[usize],
        uniforms: PassUniforms<'_>,
    ) -> Result<wgpu::BindGroup, EffectRenderError> {
        let mut entries = alloc::vec![wgpu::BindGroupEntry {
            binding: binding::INPUT,
            resource: wgpu::BindingResource::TextureView(input_view(index, input, slots, slot_of)),
        }];
        if let Some(buffer) = uniforms.output_scale {
            entries.push(wgpu::BindGroupEntry {
                binding: entry::scale_binding(&self.plan),
                resource: buffer.as_entire_binding(),
            });
        }
        if let Some(filter) = self.plan.sampler {
            entries.push(wgpu::BindGroupEntry {
                binding: binding::SAMPLER,
                resource: wgpu::BindingResource::Sampler(match filter {
                    SamplerFilter::Point => &shared.point_sampler,
                    SamplerFilter::Filtered => &shared.filtering_sampler,
                }),
            });
        }
        if let Some(buffer) = uniforms.params {
            entries.push(wgpu::BindGroupEntry {
                binding: binding::PARAMS,
                resource: buffer.as_entire_binding(),
            });
        }
        if self.uses_space {
            entries.push(wgpu::BindGroupEntry {
                binding: binding::SPACE,
                resource: shared.space.as_entire_binding(),
            });
        }
        if let Some(shape) = self.plan.shape {
            entries.push(wgpu::BindGroupEntry {
                binding: binding::SHAPE,
                resource: wgpu::BindingResource::TextureView(shape_view(input, shape)?),
            });
        }
        for (n, aux) in (0u32..).zip(&self.plan.aux) {
            let view = match *aux {
                PassAux::Image(image) | PassAux::Texture(image) => shared.images[image].view(),
                PassAux::PassInput(pass) => input_view(pass, input, slots, slot_of),
            };
            entries.push(wgpu::BindGroupEntry {
                binding: binding::AUX + n,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        Ok(input.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filtrate pass bindings"),
            layout: &self.layout,
            entries: &entries,
        }))
    }

    /// The uniform buffer holding this encode's `values` and `size` — the
    /// `size` member of a spatial segment's block — as an index into
    /// [`Self::params`]. `None` when the segment has no uniform block.
    #[expect(
        clippy::cast_precision_loss,
        reason = "image extents fit f32 exactly (well below 2^24 pixels)"
    )]
    fn params_index(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        values: &[f32],
        size: (u32, u32),
        sequence: u64,
    ) -> Option<usize> {
        if self.plan.segment.uniform.size == 0 {
            return None;
        }
        let mut words = alloc::vec![0u32; self.plan.segment.uniform.size as usize / 4];
        if let Some(offset) = self.plan.segment.uniform.input_size {
            let first = offset as usize / 4;
            words[first] = (size.0 as f32).to_bits();
            words[first + 1] = (size.1 as f32).to_bits();
        }
        for slot in &self.plan.uniforms {
            let first = slot.offset as usize / 4;
            for component in 0..slot.components {
                words[first + component] = values[slot.param + component].to_bits();
            }
        }
        Some(self.params.select(device, queue, words, sequence))
    }
}

/// The bind group layout a pass's segment needs.
fn layout_entries(plan: &PassPlan, resizes_output: bool) -> Vec<wgpu::BindGroupLayoutEntry> {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    let texture = |filterable| wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let filtering = plan.sampler == Some(SamplerFilter::Filtered);
    let mut entries = alloc::vec![entry(binding::INPUT, texture(filtering))];
    if resizes_output {
        entries.push(entry(entry::scale_binding(plan), uniform));
    }
    for arg in &plan.segment.args {
        match *arg {
            SegmentArg::InputSampler => entries.push(entry(
                binding::SAMPLER,
                wgpu::BindingType::Sampler(if filtering {
                    wgpu::SamplerBindingType::Filtering
                } else {
                    wgpu::SamplerBindingType::NonFiltering
                }),
            )),
            SegmentArg::Params => entries.push(entry(binding::PARAMS, uniform)),
            SegmentArg::WorkingSpace => entries.push(entry(binding::SPACE, uniform)),
            SegmentArg::Shape => entries.push(entry(binding::SHAPE, texture(false))),
            SegmentArg::Aux(n) => entries.push(entry(binding::AUX + n, texture(false))),
            SegmentArg::Color | SegmentArg::Input | SegmentArg::Uv => {}
        }
    }
    entries
}

/// The texture pass `pass` reads as its input.
fn input_view<'a>(
    pass: usize,
    input: &'a EffectInput<'_>,
    slots: &'a [wgpu::TextureView],
    slot_of: &[usize],
) -> &'a wgpu::TextureView {
    pass.checked_sub(1)
        .map_or(&input.view, |previous| &slots[slot_of[previous]])
}

fn shape_view<'a>(
    input: &'a EffectInput<'_>,
    shape: ShapeInput,
) -> Result<&'a wgpu::TextureView, EffectRenderError> {
    match shape {
        ShapeInput::Sdf => input.shape.sdf.as_ref(),
        ShapeInput::Mask => input.shape.mask.as_ref(),
    }
    .ok_or(EffectRenderError::MissingShape(shape))
}

/// Assigns every intermediate (the output of every pass but the last) a
/// slot, reusing a slot once every pass that reads its occupant has run.
/// Returns each intermediate's slot and the slot count.
fn assign_slots(passes: &[PassPlan]) -> (Vec<usize>, usize) {
    let intermediates = passes.len() - 1;
    // The last pass reading each intermediate: the next pass, or a later
    // pass binding its consumer's input as an auxiliary image.
    let mut last_read: Vec<usize> = (1..=intermediates).collect();
    for (pass, plan) in passes.iter().enumerate() {
        for aux in &plan.aux {
            if let PassAux::PassInput(consumer) = *aux
                && let Some(intermediate) = consumer.checked_sub(1)
            {
                last_read[intermediate] = last_read[intermediate].max(pass);
            }
        }
    }
    let mut busy_until: Vec<usize> = Vec::new();
    let slot_of = (0..intermediates)
        .map(|intermediate| {
            // Pass `intermediate` writes it; a slot is free once its
            // occupant's last reader ran before that pass.
            let slot = busy_until
                .iter()
                .position(|&until| until < intermediate)
                .unwrap_or_else(|| {
                    busy_until.push(0);
                    busy_until.len() - 1
                });
            busy_until[slot] = last_read[intermediate];
            slot
        })
        .collect();
    (slot_of, busy_until.len())
}

impl Intermediates {
    fn new(device: &wgpu::Device, size: (u32, u32), count: usize, last_used: u64) -> Self {
        Self {
            size,
            last_used,
            views: (0..count)
                .map(|_| {
                    intermediate_texture(device, size)
                        .create_view(&wgpu::TextureViewDescriptor::default())
                })
                .collect(),
        }
    }
}

fn intermediate_texture(device: &wgpu::Device, (width, height): (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filtrate intermediate"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: INTERMEDIATE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// Validates the bound textures against their stages' `texture_2d<f32>`
/// declarations: a `Texture` auxiliary requires one, and every bound
/// texture must be a float-sampleable 2D texture.
fn check_bound_textures(
    passes: &[GpuPass],
    images: &[Image],
    device: &wgpu::Device,
) -> Result<(), EffectSetupError> {
    for pass in passes {
        for (n, aux) in (0u32..).zip(&pass.plan.aux) {
            let (image, requires_texture) = match *aux {
                PassAux::Image(image) => (image, false),
                PassAux::Texture(image) => (image, true),
                PassAux::PassInput(_) => continue,
            };
            match &images[image] {
                Image::Bound { texture, .. } => {
                    let format = texture.format();
                    let sample_type =
                        format.sample_type(Some(wgpu::TextureAspect::All), Some(device.features()));
                    let float = matches!(
                        sample_type,
                        Some(
                            wgpu::TextureSampleType::Float { .. } | wgpu::TextureSampleType::Depth
                        )
                    );
                    if texture.dimension() != wgpu::TextureDimension::D2 || !float {
                        return Err(EffectSetupError::StageMismatch {
                            stage: pass.plan.name,
                            reason: format!(
                                "aux{n} is a `texture_2d<f32>`, but image {image} is a \
                                 {format:?} {:?} texture",
                                texture.dimension()
                            ),
                        });
                    }
                }
                Image::Uploaded { .. } if requires_texture => {
                    return Err(EffectSetupError::StageMismatch {
                        stage: pass.plan.name,
                        reason: format!(
                            "declares aux{n} a GPU texture, but image {image} is CPU data"
                        ),
                    });
                }
                Image::Uploaded { .. } => {}
            }
        }
    }
    Ok(())
}

/// Whether `format` supports a filtering sampler under `features`.
pub fn filterable(format: wgpu::TextureFormat, features: wgpu::Features) -> bool {
    format
        .guaranteed_format_features(features)
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
}

/// Fails setup when the device cannot render to the intermediates.
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "the wasm32 WebGPU device is !Send; the executor's futures run on the page's event loop"
    )
)]
async fn probe_intermediate_format(device: &wgpu::Device) -> Result<(), EffectSetupError> {
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    drop(intermediate_texture(device, (1, 1)));
    error_scope.pop().await.map_or(Ok(()), |error| {
        Err(EffectSetupError::IntermediateFormatUnsupported {
            message: error.to_string(),
        })
    })
}

/// A clamp-to-edge sampler of the input with `filter` for magnification and
/// minification.
pub fn sampler(device: &wgpu::Device, filter: wgpu::FilterMode) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("filtrate input sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}

/// Uploads the filter's auxiliary images as it visits them.
struct Uploader<'a, 'c> {
    ctx: &'a EffectContext<'c>,
    images: Vec<Option<Image>>,
}

impl ImageVisitor for Uploader<'_, '_> {
    fn visit<I: AuxImage + ?Sized>(&mut self, index: usize, image: &I) {
        assert!(
            index < self.images.len(),
            "the filter visited image {index} but declares {} images",
            self.images.len()
        );
        assert!(
            self.images[index].is_none(),
            "the filter visited image {index} twice"
        );
        // A caller-provided texture binds in place — no upload.
        if let Some(image) = image
            .as_any()
            .and_then(|any| any.downcast_ref::<TextureImage>())
        {
            let texture = image.texture();
            self.images[index] = Some(Image::Bound {
                view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
                texture: texture.clone(),
            });
            return;
        }

        let data = image
            .data()
            .unwrap_or_else(|| panic!("aux image {index} is neither CPU data nor a GPU texture"));
        let format = match data.format {
            AuxFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            AuxFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
            AuxFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        };
        let texel_size = u32::try_from(data.format.texel_size()).expect("a texel size fits in u32");
        let size = wgpu::Extent3d {
            width: image.width(),
            height: image.height(),
            depth_or_array_layers: 1,
        };
        let texture = self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filtrate auxiliary image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data.bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width() * texel_size),
                rows_per_image: Some(image.height()),
            },
            size,
        );
        self.images[index] = Some(Image::Uploaded {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            _texture: texture,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_reused_once_their_readers_ran() {
        use cherenkov_shader::{Segment, UniformLayout};

        let pass = |aux: Vec<PassAux>| PassPlan {
            segment: Segment {
                function: "segment".to_owned(),
                stages: 0..1,
                variants: Vec::new(),
                args: Vec::new(),
                uniform: UniformLayout::default(),
            },
            name: "stage",
            sampler: None,
            shape: None,
            aux,
            uniforms: Vec::new(),
        };
        // A plain chain ping-pongs between two slots.
        let (slots, count) = assign_slots(&[
            pass(Vec::new()),
            pass(Vec::new()),
            pass(Vec::new()),
            pass(Vec::new()),
        ]);
        assert_eq!((slots, count), (alloc::vec![0, 1, 0], 2));
        // Pass 3 reads pass 2's input (intermediate 1) as an auxiliary image,
        // so intermediate 1 stays live through pass 3, and pass 3's own
        // output needs a third slot.
        let (slots, count) = assign_slots(&[
            pass(Vec::new()),
            pass(Vec::new()),
            pass(Vec::new()),
            pass(alloc::vec![PassAux::PassInput(2)]),
            pass(Vec::new()),
        ]);
        assert_eq!((slots, count), (alloc::vec![0, 1, 0, 2], 3));
    }
}
