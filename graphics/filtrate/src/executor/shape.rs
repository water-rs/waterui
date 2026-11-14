//! The signed distance field of the input's shape, for stages that read
//! [`ShapeInput::Sdf`](filtrate_core::ShapeInput::Sdf), as GPU passes.
//!
//! The field is built by jump flooding on [`crate::shape_field`]'s
//! schedule: a seed pass, one step pass per jump, and a resolve pass. Two
//! `Rgba16Float` targets per input size ping-pong between the passes, and
//! the resolve pass writes the field into the one the last step read.

extern crate alloc;

use alloc::{borrow::Cow, borrow::ToOwned, vec::Vec};

use super::gpu::{INTERMEDIATE_FORMAT, build_pipeline, draw, intermediate_texture, layout_entry};
use crate::{
    effect::{EffectContext, EffectSetupError},
    pipeline_cache::{PassKey, PassPipeline},
    shape_field::jumps,
};

const SEED: &str = include_str!("../shaders/shape/seed.wgsl");
const STEP: &str = include_str!("../shaders/shape/step.wgsl");
const RESOLVE: &str = include_str!("../shaders/shape/resolve.wgsl");

/// The pipelines that build the field.
#[derive(Debug)]
pub(super) struct ShapeField {
    seed: PassPipeline,
    step: PassPipeline,
    resolve: PassPipeline,
}

/// One input size's field targets and the bindings that do not change
/// between frames.
#[derive(Debug)]
pub(super) struct FieldTargets {
    views: [wgpu::TextureView; 2],
    /// Step `n` reads `views[n % 2]` and writes the other.
    steps: Vec<wgpu::BindGroup>,
    resolve: wgpu::BindGroup,
}

impl ShapeField {
    #[expect(
        clippy::future_not_send,
        reason = "setup borrows the device-bound context on the GPU host thread"
    )]
    pub(super) async fn new(ctx: &EffectContext<'_>) -> Result<Self, EffectSetupError> {
        let texture = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let (seed, step, resolve) = futures_util::future::try_join3(
            pipeline(ctx, "seed", SEED, alloc::vec![layout_entry(0, texture)]),
            pipeline(
                ctx,
                "step",
                STEP,
                alloc::vec![layout_entry(0, texture), layout_entry(1, uniform)],
            ),
            pipeline(
                ctx,
                "resolve",
                RESOLVE,
                alloc::vec![layout_entry(0, texture)],
            ),
        )
        .await?;
        Ok(Self {
            seed,
            step,
            resolve,
        })
    }

    /// Allocates the targets for `size` and binds the passes that read only
    /// them.
    pub(super) fn targets(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: (u32, u32),
    ) -> FieldTargets {
        let views = [(); 2].map(|()| {
            intermediate_texture(device, size).create_view(&wgpu::TextureViewDescriptor::default())
        });
        let steps = jumps(size)
            .into_iter()
            .enumerate()
            .map(|(index, jump)| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("filtrate shape field jump"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&[jump, 0, 0, 0]));
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("filtrate shape field step bindings"),
                    layout: &self.step.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&views[index % 2]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buffer.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect::<Vec<_>>();
        let resolve = bind_texture(device, &self.resolve, &views[steps.len() % 2]);
        FieldTargets {
            views,
            steps,
            resolve,
        }
    }

    /// Encodes the field of `coverage`'s alpha into `targets`.
    pub(super) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        coverage: &wgpu::TextureView,
        targets: &FieldTargets,
    ) {
        let seed = bind_texture(device, &self.seed, coverage);
        draw(encoder, &self.seed.pipeline, &seed, &targets.views[0]);
        for (index, step) in targets.steps.iter().enumerate() {
            draw(
                encoder,
                &self.step.pipeline,
                step,
                &targets.views[(index + 1) % 2],
            );
        }
        draw(
            encoder,
            &self.resolve.pipeline,
            &targets.resolve,
            targets.field(),
        );
    }
}

/// Binds `view` as the single texture of a seed or resolve pass.
fn bind_texture(
    device: &wgpu::Device,
    pass: &PassPipeline,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("filtrate shape field bindings"),
        layout: &pass.layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(view),
        }],
    })
}

impl FieldTargets {
    /// The resolved field: the target the last step read.
    pub(super) const fn field(&self) -> &wgpu::TextureView {
        &self.views[(self.steps.len() + 1) % 2]
    }

    /// How many `INTERMEDIATE_FORMAT` textures the targets hold.
    pub(super) const fn texture_count(&self) -> usize {
        self.views.len()
    }
}

#[expect(
    clippy::future_not_send,
    reason = "setup borrows the device-bound context on the GPU host thread"
)]
async fn pipeline(
    ctx: &EffectContext<'_>,
    pass: &'static str,
    source: &'static str,
    layout: Vec<wgpu::BindGroupLayoutEntry>,
) -> Result<PassPipeline, EffectSetupError> {
    let key = PassKey {
        wgsl: source.to_owned(),
        layout,
        resizes_output: false,
        format: INTERMEDIATE_FORMAT,
    };
    ctx.pipelines
        .pass(key, |key| {
            build_pipeline(ctx, wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)), key)
        })
        .await
        .map_err(|message| {
            ctx.pipelines.forget_failed();
            tracing::error!("[filtrate] shape field {pass} pipeline validation failed: {message}");
            EffectSetupError::ShapeFieldValidation { pass, message }
        })
}
