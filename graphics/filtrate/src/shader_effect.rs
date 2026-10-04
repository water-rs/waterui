//! Application-supplied WGSL post-processing.
//!
//! [`ShaderEffect`] runs one fragment shader the application writes over the
//! effect's input texture — the captured content of whatever the host applies
//! it to. It is the effect a terminal's custom shader, a CRT or scanline pass,
//! or an animated distortion is built from: the shader samples the input, reads
//! the frame time, the resolution and its own parameters, and writes the
//! output pixel.
//!
//! # Shader contract
//!
//! The application supplies a WGSL module that defines a fragment entry point
//! named `main`. The effect appends a prelude to that module, so the module can
//! use these declarations without writing them:
//!
//! ```wgsl
//! struct ShaderEffectUniforms {
//!     resolution: vec2<f32>,       // output size in physical pixels
//!     input_resolution: vec2<f32>, // input size in physical pixels
//!     time: f32,                   // seconds on the host frame timeline
//!     time_delta: f32,             // seconds since the previous frame
//!     frame: u32,                  // host frame sequence, wrapping at 2^32
//!     param_count: u32,            // parameters added to the effect
//!     params: array<vec4<f32>, 4>, // the parameters, four per vector
//! }
//! @group(0) @binding(0) var input_texture: texture_2d<f32>;
//! @group(0) @binding(1) var input_sampler: sampler; // bilinear, clamp to edge
//! @group(0) @binding(2) var<uniform> uniforms: ShaderEffectUniforms;
//! fn effect_param(index: u32) -> f32;
//!
//! struct VertexOutput {
//!     @builtin(position) position: vec4<f32>, // output pixel, top-left origin
//!     @location(0) uv: vec2<f32>,            // (0, 0) top-left, (1, 1) bottom-right
//! }
//! ```
//!
//! Those names, and the vertex entry point `shader_effect_vs`, are reserved.
//! `uv` addresses the input texture directly, so
//! `textureSample(input_texture, input_sampler, in.uv)` reads the input under
//! the fragment even when the input and output sizes differ. The input
//! carries premultiplied alpha in the working space, and the output is
//! expected to as well (see [`crate::effect`]).
//!
//! ```rust
//! use filtrate::ShaderEffect;
//!
//! // Darkens every other pair of rows, with a faint flicker over time.
//! let scanlines = ShaderEffect::new(
//!     r"
//!     @fragment
//!     fn main(in: VertexOutput) -> @location(0) vec4<f32> {
//!         let color = textureSample(input_texture, input_sampler, in.uv);
//!         let row = u32(in.position.y);
//!         let line = select(1.0, 0.0, (row / 2u) % 2u == 1u);
//!         let flicker = 0.97 + 0.03 * sin(uniforms.time * 50.0);
//!         let shade = mix(1.0, line, effect_param(0u)) * flicker;
//!         return vec4<f32>(color.rgb * shade, color.a);
//!     }
//!     ",
//! )
//! .expect("the scanline shader is valid WGSL")
//! .param(0.35)
//! .animated();
//! assert_eq!(scanlines.param_count(), 1);
//! ```
//!
//! # Errors surface at construction
//!
//! [`ShaderEffect::new`] parses and validates the module on the CPU and
//! returns a [`ShaderEffectError`] carrying the full diagnostic when the WGSL
//! is malformed, fails validation, or lacks the `main` fragment entry point.
//! Line and column numbers in the diagnostic count in the application's own
//! source. Applications that load shaders at run time (from a user's
//! configuration file, say) handle the error where they read the file; no
//! invalid shader reaches a host. Validation uses the capabilities of a core
//! WebGPU device, so a shader that validates runs on every device.
//!
//! What depends on the device is checked by [`Effect::setup`]: a shader that
//! reads `input_sampler` needs an input format the device can filter, and
//! setup fails with [`EffectSetupError::InputNotFilterable`] otherwise.
//!
//! # Redraw
//!
//! A shader that reads `uniforms.time` changes with every frame, and says so
//! with [`ShaderEffect::animated`]: the effect then asks its host for another
//! frame after each one it renders. A shader that does not animate is rendered
//! only when its input or one of its parameters changes. Reactive parameters
//! ([`ShaderEffect::watch_param`]) wake the host through the effect's redraw
//! callback and animate along their interpolators exactly like the
//! parameters of a filter run by the [`Executor`](crate::Executor).
//!
//! # Threading and encoding
//!
//! The effect is `Send` on native targets: the validated module and the
//! parameter tracks move to whichever thread owns the GPU, and reactive
//! subscriptions stay with the caller (see [`ShaderEffect::watch_param`]).
//! A subscription may outlive the effect; once the effect is dropped, the
//! changes it reports are discarded.
//! [`Effect::encode_render`] records into the host's shared command encoder;
//! one effect encoded several times before the encoder is submitted keeps
//! each encode's uniforms in a buffer of its own. Bind groups are cached
//! per input view and uniform buffer, so a steady animated effect allocates
//! nothing per frame.

extern crate alloc;

use alloc::{borrow::Cow, string::String};
use core::fmt;

use cherenkov_shader::naga;
use filtrate_core::{FilterParam, WatchGuard};

use crate::effect::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderError,
    EffectRenderResult, EffectSetupError, EffectSetupResult,
};
use crate::executor::{animation::ParamAnimator, filterable, sampler, uniforms::UniformBuffers};

/// Maximum number of parameters one [`ShaderEffect`] carries.
pub const SHADER_EFFECT_MAX_PARAMS: usize = 16;

const PARAM_VEC4S: usize = SHADER_EFFECT_MAX_PARAMS / 4;

/// The fragment entry point the application's module defines.
const FRAGMENT_ENTRY_POINT: &str = "main";
/// The vertex entry point the prelude defines.
const VERTEX_ENTRY_POINT: &str = "shader_effect_vs";
/// The prelude's sampler binding.
const INPUT_SAMPLER: &str = "input_sampler";

const PRELUDE: &str = include_str!("shaders/shader_effect_prelude.wgsl");

/// Why an application's WGSL cannot become a [`ShaderEffect`].
///
/// Each message carries naga's rendered diagnostic, with line and column
/// numbers counted in the application's own source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShaderEffectError {
    /// The source is not well-formed WGSL.
    #[error("shader effect WGSL failed to parse:\n{0}")]
    Parse(String),
    /// The source parsed but is not a valid WGSL module.
    #[error("shader effect WGSL failed validation:\n{0}")]
    Validation(String),
    /// The module declares no `@fragment fn main`.
    #[error("shader effect WGSL declares no `@fragment fn main` entry point")]
    MissingEntryPoint,
}

/// `ShaderEffectUniforms` in the prelude, byte for byte.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderEffectUniforms {
    resolution: [f32; 2],
    input_resolution: [f32; 2],
    time: f32,
    time_delta: f32,
    frame: u32,
    param_count: u32,
    params: [[f32; 4]; PARAM_VEC4S],
}

/// The uniform block's size in 32-bit words.
const UNIFORM_WORDS: usize = core::mem::size_of::<ShaderEffectUniforms>() / 4;

/// GPU objects built by [`Effect::setup`].
#[derive(Debug)]
struct Resources {
    input_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: UniformBuffers,
    /// Bind groups of recent encodes, keyed by everything they bind that
    /// varies: the input view and the uniform buffer.
    bind_groups: Vec<CachedBindGroup>,
}

/// A bind group and the resources that key it.
#[derive(Debug)]
struct CachedBindGroup {
    input: wgpu::TextureView,
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
    /// The frame sequence that last bound it.
    last_used: u64,
}

impl Resources {
    /// The bind group of `input` and the uniform buffer `uniforms` indexes,
    /// for an encode of frame `sequence`. A group bound in this or the
    /// previous frame is reused; older groups are dropped, so a retired
    /// input texture is not kept alive.
    fn bind_group(
        &mut self,
        device: &wgpu::Device,
        input: &wgpu::TextureView,
        uniforms: usize,
        sequence: u64,
    ) -> &wgpu::BindGroup {
        self.bind_groups
            .retain(|cached| cached.last_used.saturating_add(1) >= sequence);
        let buffer = self.uniforms.buffer(uniforms);
        let index = if let Some(index) = self
            .bind_groups
            .iter()
            .position(|cached| cached.input == *input && cached.uniforms == *buffer)
        {
            self.bind_groups[index].last_used = sequence;
            index
        } else {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shader effect bindings"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            });
            self.bind_groups.push(CachedBindGroup {
                input: input.clone(),
                uniforms: buffer.clone(),
                group,
                last_used: sequence,
            });
            self.bind_groups.len() - 1
        };
        &self.bind_groups[index].group
    }
}

/// Where the effect is in its setup.
#[derive(Debug)]
enum Setup {
    /// [`Effect::setup`] has not run.
    Pending,
    /// The pipeline is built.
    Ready(Resources),
    /// Setup failed; the error is sticky and rendering fails fast.
    Failed(EffectSetupError),
}

/// An application-supplied WGSL fragment shader run over the effect input.
///
/// See the [module documentation](self) for the shader contract.
pub struct ShaderEffect {
    /// The application's module with the prelude appended, validated.
    module: naga::Module,
    /// Whether `main` reads `input_sampler`, which then needs a filterable
    /// input format.
    samples_input: bool,
    /// Parameter tracks and the channel reactive parameters feed. It holds no
    /// subscription, which keeps the effect `Send`.
    animator: ParamAnimator,
    animated: bool,
    setup: Setup,
}

impl fmt::Debug for ShaderEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShaderEffect")
            .field("params", &self.animator.param_count())
            .field("animated", &self.animated)
            .field("setup", &self.setup)
            .finish_non_exhaustive()
    }
}

impl ShaderEffect {
    /// Validates `source` and builds an effect that runs its `main` fragment
    /// entry point over the input.
    ///
    /// # Errors
    ///
    /// Returns [`ShaderEffectError`] when `source` does not parse, does not
    /// validate together with the prelude, or declares no `@fragment fn main`.
    pub fn new(source: impl AsRef<str>) -> Result<Self, ShaderEffectError> {
        let source = source.as_ref();
        let mut full = String::with_capacity(source.len() + PRELUDE.len() + 1);
        full.push_str(source);
        full.push('\n');
        full.push_str(PRELUDE);
        let (module, info) = parse_and_validate(&full)?;
        let main = module
            .entry_points
            .iter()
            .position(|entry_point| {
                entry_point.name == FRAGMENT_ENTRY_POINT
                    && entry_point.stage == naga::ShaderStage::Fragment
            })
            .ok_or(ShaderEffectError::MissingEntryPoint)?;
        let main_uses = info.get_entry_point(main);
        let samples_input = module.global_variables.iter().any(|(handle, global)| {
            global.name.as_deref() == Some(INPUT_SAMPLER) && !main_uses[handle].is_empty()
        });
        // No parameter is visited here, so there are no guards to keep;
        // `watch_param` hands each later subscription to the caller.
        let (animator, _) = ParamAnimator::new(Vec::new(), |_| {});
        Ok(Self {
            module,
            samples_input,
            animator,
            animated: false,
            setup: Setup::Pending,
        })
    }

    /// Adds a constant parameter, readable in the shader as `effect_param(n)`
    /// where `n` counts the parameters added before it.
    ///
    /// # Panics
    ///
    /// Panics when the effect already carries [`SHADER_EFFECT_MAX_PARAMS`]
    /// parameters.
    #[must_use]
    pub fn param(mut self, value: f32) -> Self {
        self.push_param(value);
        self
    }

    /// Adds a reactive parameter, readable in the shader as `effect_param(n)`
    /// where `n` counts the parameters added before it.
    ///
    /// The effect starts from `param`'s current value; each change `param`
    /// reports re-renders the effect and follows the animation attached to
    /// the change. The returned guard is the subscription: the caller keeps
    /// it for as long as `param` should drive the effect. Holding it outside
    /// the effect keeps [`ShaderEffect`] `Send` while a reactive frontend's
    /// subscription, which is commonly not `Send`, stays on the frontend's
    /// thread.
    ///
    /// The guard may outlive the effect. A host drops the effect on its own
    /// schedule — often on another thread, when the content it filters goes
    /// away — so the order in which the effect and the guard are dropped is
    /// not the caller's to control. Once the effect is gone, a change `param`
    /// reports is discarded: there is nothing left to render it, and the
    /// host is not woken.
    ///
    /// # Panics
    ///
    /// Panics when the effect already carries [`SHADER_EFFECT_MAX_PARAMS`]
    /// parameters.
    #[must_use]
    pub fn watch_param<P: FilterParam + ?Sized>(mut self, param: &P) -> (Self, WatchGuard) {
        let index = self.push_param(param.snapshot());
        let guard = self.animator.sender().watch(index, param);
        (self, guard)
    }

    /// Marks the shader as time-driven: after every frame it renders, the
    /// effect asks its host for another one.
    #[must_use]
    pub const fn animated(mut self) -> Self {
        self.animated = true;
        self
    }

    /// The number of parameters added so far.
    #[must_use]
    pub const fn param_count(&self) -> usize {
        self.animator.param_count()
    }

    /// The cached bind groups — for tests asserting reuse across frames.
    #[cfg(test)]
    pub(crate) fn cached_bind_groups(&self) -> Vec<wgpu::BindGroup> {
        match &self.setup {
            Setup::Ready(resources) => resources
                .bind_groups
                .iter()
                .map(|cached| cached.group.clone())
                .collect(),
            Setup::Pending | Setup::Failed(_) => Vec::new(),
        }
    }

    fn push_param(&mut self, initial: f32) -> usize {
        assert!(
            self.animator.param_count() < SHADER_EFFECT_MAX_PARAMS,
            "a ShaderEffect carries at most {SHADER_EFFECT_MAX_PARAMS} parameters"
        );
        self.animator.push_param(initial)
    }
}

/// Builds the pipeline running `module` for `ctx`'s device and formats.
/// `samples_input` says whether `main` reads `input_sampler`.
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "the wasm32 WebGPU device is !Send; effect futures run on the page's event loop"
    )
)]
async fn build(
    module: &naga::Module,
    samples_input: bool,
    ctx: &EffectContext<'_>,
) -> Result<Resources, EffectSetupError> {
    let input_filterable = filterable(ctx.input_format, ctx.device.features());
    if samples_input && !input_filterable {
        return Err(EffectSetupError::InputNotFilterable {
            format: ctx.input_format,
        });
    }

    let error_scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let layout = ctx
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader effect layout"),
            entries: &layout_entries(input_filterable),
        });
    let shader = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader effect"),
            source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
        });
    let pipeline_layout = ctx
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader effect pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let pipeline = ctx
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader effect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(VERTEX_ENTRY_POINT),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(FRAGMENT_ENTRY_POINT),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.output_format,
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
        return Err(EffectSetupError::PipelineValidation {
            pass: 0,
            message: error.to_string(),
        });
    }

    Ok(Resources {
        input_format: ctx.input_format,
        output_format: ctx.output_format,
        pipeline,
        layout,
        sampler: sampler(
            ctx.device,
            if input_filterable {
                wgpu::FilterMode::Linear
            } else {
                wgpu::FilterMode::Nearest
            },
        ),
        uniforms: UniformBuffers::new("shader effect uniforms"),
        bind_groups: Vec::new(),
    })
}

/// The uniform block for one encode, as the words its buffer holds.
fn uniform_words(
    animator: &ParamAnimator,
    input: &EffectInput<'_>,
    output: &EffectOutput<'_>,
) -> [u32; UNIFORM_WORDS] {
    let mut params = [[0.0; 4]; PARAM_VEC4S];
    for (index, value) in animator.current_values().iter().enumerate() {
        params[index / 4][index % 4] = *value;
    }
    bytemuck::cast(ShaderEffectUniforms {
        resolution: [u32_to_f32(output.width), u32_to_f32(output.height)],
        input_resolution: [u32_to_f32(input.width), u32_to_f32(input.height)],
        time: input.timing.presentation_time().as_secs_f32(),
        time_delta: input.timing.delta().as_secs_f32(),
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the WGSL frame counter is the host sequence wrapping at 2^32"
        )]
        frame: input.timing.sequence() as u32,
        param_count: u32::try_from(animator.param_count())
            .expect("the parameter count is bounded by SHADER_EFFECT_MAX_PARAMS"),
        params,
    })
}

/// The bind group layout of the prelude's bindings. An unfilterable input
/// binds a non-filtering sampler, which only a shader that never reads
/// `input_sampler` is set up with.
fn layout_entries(input_filterable: bool) -> [wgpu::BindGroupLayoutEntry; 3] {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    [
        entry(
            0,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float {
                    filterable: input_filterable,
                },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        ),
        entry(
            1,
            wgpu::BindingType::Sampler(if input_filterable {
                wgpu::SamplerBindingType::Filtering
            } else {
                wgpu::SamplerBindingType::NonFiltering
            }),
        ),
        entry(
            2,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(
                    core::mem::size_of::<ShaderEffectUniforms>() as u64,
                ),
            },
        ),
    ]
}

/// Parses and validates a complete effect module with the capabilities of a
/// core WebGPU device, rendering naga's diagnostic against `source` on
/// failure.
fn parse_and_validate(
    source: &str,
) -> Result<(naga::Module, naga::valid::ModuleInfo), ShaderEffectError> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| ShaderEffectError::Parse(error.emit_to_string(source)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|error| ShaderEffectError::Validation(error.emit_to_string(source)))?;
    Ok((module, info))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "texture dimensions are far below f32's exact integer range"
)]
const fn u32_to_f32(value: u32) -> f32 {
    value as f32
}

impl Effect for ShaderEffect {
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        self.animator.install_redraw_callback(callback);
    }

    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "the wasm32 WebGPU device is !Send; effect futures run on the page's event loop"
        )
    )]
    async fn setup(&mut self, ctx: &EffectContext<'_>) -> EffectSetupResult {
        match build(&self.module, self.samples_input, ctx).await {
            Ok(resources) => {
                self.setup = Setup::Ready(resources);
                self.animator.ensure_redraw_callback();
                self.animator.apply_targets_to_current();
                Ok(())
            }
            Err(error) => {
                tracing::error!("[filtrate] shader effect setup failed: {error}");
                self.setup = Setup::Failed(error.clone());
                Err(error)
            }
        }
    }

    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        let pipeline = match &mut self.setup {
            Setup::Pending => return Err(EffectRenderError::NotSetUp),
            Setup::Failed(error) => return Err(EffectRenderError::SetupFailed(error.clone())),
            Setup::Ready(pipeline) => pipeline,
        };
        if input.format != pipeline.input_format || output.format != pipeline.output_format {
            return Err(EffectRenderError::FormatMismatch {
                input: input.format,
                output: output.format,
                setup_input: pipeline.input_format,
                setup_output: pipeline.output_format,
            });
        }
        let parameters_animating = self.animator.update(input.timing.delta());
        let sequence = input.timing.sequence();
        let uniforms = pipeline.uniforms.select(
            input.device,
            input.queue,
            uniform_words(&self.animator, input, output).to_vec(),
            sequence,
        );
        // A handle clone: the pass below also borrows the pipeline.
        let bind_group = pipeline
            .bind_group(input.device, &input.view, uniforms, sequence)
            .clone();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shader effect pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
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
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.animator.mark_rendered();
        Ok(self.animated || parameters_animating)
    }

    fn redraw_hint(&self) -> bool {
        self.animated || self.animator.redraw_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS_THROUGH: &str = "
        @fragment
        fn main(in: VertexOutput) -> @location(0) vec4<f32> {
            return textureSample(input_texture, input_sampler, in.uv);
        }
    ";

    /// The Rust uniform block must match the prelude's struct byte for byte,
    /// or every field after the first mismatch reads garbage on the GPU:
    /// same members, in the same order, at the same offsets, same size.
    #[test]
    fn uniform_block_matches_the_prelude_layout() {
        use core::mem::{offset_of, size_of};

        let (module, _) =
            parse_and_validate(PRELUDE).expect("the prelude is valid WGSL on its own");
        let ty = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("ShaderEffectUniforms"))
            .map(|(_, ty)| ty)
            .expect("the prelude declares ShaderEffectUniforms");
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("ShaderEffectUniforms is a struct, got {:?}", ty.inner);
        };
        let wgsl: Vec<(&str, u32)> = members
            .iter()
            .map(|member| {
                (
                    member.name.as_deref().expect("prelude members are named"),
                    member.offset,
                )
            })
            .collect();
        let offset = |offset: usize| u32::try_from(offset).expect("offsets fit u32");
        let rust = [
            (
                "resolution",
                offset(offset_of!(ShaderEffectUniforms, resolution)),
            ),
            (
                "input_resolution",
                offset(offset_of!(ShaderEffectUniforms, input_resolution)),
            ),
            ("time", offset(offset_of!(ShaderEffectUniforms, time))),
            (
                "time_delta",
                offset(offset_of!(ShaderEffectUniforms, time_delta)),
            ),
            ("frame", offset(offset_of!(ShaderEffectUniforms, frame))),
            (
                "param_count",
                offset(offset_of!(ShaderEffectUniforms, param_count)),
            ),
            ("params", offset(offset_of!(ShaderEffectUniforms, params))),
        ];
        assert_eq!(wgsl, rust, "members and offsets");
        assert_eq!(
            *span as usize,
            size_of::<ShaderEffectUniforms>(),
            "struct size"
        );
    }

    /// Hosts move effects to the thread that owns the GPU.
    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn a_shader_effect_is_send() {
        const fn assert_send<T: Send>() {}
        assert_send::<ShaderEffect>();
    }

    #[test]
    fn a_valid_module_builds() {
        let effect = ShaderEffect::new(PASS_THROUGH).expect("pass-through shader is valid");
        assert!(effect.samples_input, "the pass-through reads the sampler");
    }

    #[test]
    fn a_module_that_only_loads_texels_does_not_sample() {
        let effect = ShaderEffect::new(
            "@fragment
            fn main(in: VertexOutput) -> @location(0) vec4<f32> {
                return textureLoad(input_texture, vec2<i32>(in.position.xy), 0);
            }",
        )
        .expect("a textureLoad shader is valid");
        assert!(!effect.samples_input);
    }

    #[test]
    fn malformed_wgsl_is_a_parse_error_on_the_application_line() {
        let error = ShaderEffect::new("@fragment\nfn main( -> {}").expect_err("malformed WGSL");
        let ShaderEffectError::Parse(message) = error else {
            panic!("expected a parse error, got {error:?}");
        };
        assert!(
            message.contains(":2:"),
            "the diagnostic points at the application's line 2: {message}"
        );
    }

    #[test]
    fn an_ill_typed_module_is_rejected() {
        let error = ShaderEffect::new(
            "@fragment
            fn main(in: VertexOutput) -> @location(0) vec4<f32> {
                return textureSample(input_texture, input_sampler, 1.0);
            }",
        )
        .expect_err("sampling with a scalar coordinate is invalid");
        assert!(
            matches!(
                error,
                ShaderEffectError::Parse(_) | ShaderEffectError::Validation(_)
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn a_module_without_main_is_rejected() {
        let error = ShaderEffect::new(
            "@fragment
            fn shade(in: VertexOutput) -> @location(0) vec4<f32> {
                return vec4<f32>(in.uv, 0.0, 1.0);
            }",
        )
        .expect_err("no main entry point");
        assert_eq!(error, ShaderEffectError::MissingEntryPoint);
    }

    #[test]
    #[should_panic(expected = "at most 16 parameters")]
    fn parameters_are_bounded_by_the_uniform_block() {
        let mut effect = ShaderEffect::new(PASS_THROUGH).expect("valid");
        for value in 0..=SHADER_EFFECT_MAX_PARAMS {
            #[expect(clippy::cast_precision_loss, reason = "small test indices")]
            let value = value as f32;
            effect = effect.param(value);
        }
    }
}
