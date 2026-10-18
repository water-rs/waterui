//! Engine-owned silhouette morphology and Gaussian convolution on the GPU.
use cherenkov::RenderError;
use cherenkov::lowering::shadow as shadow_taps;
use kurbo::Affine;
use wgpu::util::DeviceExt;

/// The local-to-device covariance and spread of a captured silhouette.
#[derive(Clone, Copy)]
pub struct Parameters {
    pub transform: Affine,
    pub sigma: f64,
    pub spread: f64,
}

struct Kernel {
    key: [u64; 8],
    compute: bool,
    horizontal: bool,
    count: u32,
    /// Tap extrema `[min_x, max_x, min_y, max_y]` from the
    /// exact f32 taps, with `(0, 0)` folded in for the erosion
    /// initializer. Bounds the texel offsets the stage can sample.
    offsets: [f32; 4],
    buffer: wgpu::Buffer,
}

/// Fixed pipeline, one reusable intermediate, and at most three last kernels.
/// Bind groups retain replaced buffers until their queued commands complete.
pub struct Blur {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    compute_pipeline: wgpu::ComputePipeline,
    compute_layout: wgpu::BindGroupLayout,
    kernels: [Option<Kernel>; 3],
    temporary: Option<super::ScratchTarget>,
}

impl Blur {
    pub fn new(
        device: &wgpu::Device,
        delivery: super::shaders::ShaderDelivery,
        format: wgpu::TextureFormat,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silhouette blur"),
            entries: &super::layout_entries(super::bindings::SHADOW_GROUP0),
        });
        // Passthrough delivery trusts the fixed module's own bounds: the
        // vertex stage draws 0..3, `sample_at` clamps to the logical source
        // size, which is within the scratch texture, and the
        // uploaded count equals the following tap array's length, capped to
        // the storage binding limit.
        let shader = delivery.shadow_module(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("silhouette blur"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("silhouette blur"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let rgba16_float = match format {
            wgpu::TextureFormat::Rgba16Float => true,
            wgpu::TextureFormat::Rgba8Unorm => false,
            _ => panic!("unsupported silhouette texture format: {format:?}"),
        };
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silhouette shared convolution"),
            entries: &super::layout_entries(if rgba16_float {
                super::bindings::SHADOW_COMPUTE_F16_GROUP0
            } else {
                super::bindings::SHADOW_COMPUTE_U8_GROUP0
            }),
        });
        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("silhouette shared convolution"),
                bind_group_layouts: &[Some(&compute_layout)],
                immediate_size: 0,
            });
        let compute_module = delivery.shadow_compute_module(device, rgba16_float);
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("silhouette shared convolution"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_module,
            entry_point: Some("convolve"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        Self {
            pipeline,
            layout,
            compute_pipeline,
            compute_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("silhouette linear sampling"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            kernels: std::array::from_fn(|_| None),
            temporary: None,
        }
    }

    /// Release retained allocations without recompiling the fixed pipeline.
    pub fn trim(&mut self) {
        self.kernels = std::array::from_fn(|_| None);
        self.temporary = None;
    }

    pub fn gpu_bytes(&self) -> u64 {
        self.kernels
            .iter()
            .flatten()
            .map(|kernel| kernel.buffer.size())
            .sum::<u64>()
            + self.temporary.as_ref().map_or(0, |target| {
                u64::from(target.width)
                    * u64::from(target.height)
                    * if target.texture.format() == wgpu::TextureFormat::Rgba16Float {
                        8
                    } else {
                        4
                    }
            })
    }

    /// Convolves `input`'s capture in place. `support` is the
    /// `(x, y, w, h)` region of the `size` capture that can hold nonzero
    /// pixels; each stage scissors its output to the region its taps can
    /// still reach from it. Returns the number of passes encoded.
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &super::ScratchTarget,
        size: (u32, u32),
        mut support: [u32; 4],
        parameters: Parameters,
    ) -> Result<u32, RenderError> {
        if self
            .temporary
            .as_ref()
            .is_none_or(|t| (t.width, t.height) != size)
        {
            let (texture, view) = super::create_target(
                device,
                "silhouette intermediate",
                size,
                super::TARGET_USAGES | wgpu::TextureUsages::STORAGE_BINDING,
                input.texture.format(),
            );
            self.temporary = Some(super::ScratchTarget {
                texture,
                view,
                width: size.0,
                height: size.1,
            });
        }
        let [a, b, c, d, _, _] = parameters.transform.as_coeffs();
        let mut dispatches = 0;
        for (slot, mode, axis) in [
            (0, if parameters.spread >= 0. { 1 } else { 2 }, [0., 0.]),
            (1, 0, [a, b]),
            (2, 0, [c, d]),
        ] {
            if (slot == 0 && parameters.spread == 0.) || (slot != 0 && parameters.sigma <= 0.) {
                continue;
            }
            let key = [
                u64::from(size.0),
                u64::from(size.1),
                parameters.sigma.to_bits(),
                parameters.spread.to_bits(),
                a.to_bits(),
                b.to_bits(),
                c.to_bits(),
                d.to_bits(),
            ];
            if self.kernels[slot]
                .as_ref()
                .is_none_or(|kernel| kernel.key != key)
            {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "storage limits stay far below 2^53 bytes"
                )]
                let count_limit = device
                    .limits()
                    .max_storage_buffer_binding_size
                    .saturating_sub(16) as f64
                    / 16.0;
                let taps = if slot == 0 {
                    shadow_taps::spread_taps([a, b, c, d], parameters.spread, count_limit)?
                } else {
                    shadow_taps::gaussian_taps(axis, parameters.sigma, count_limit)?
                };
                let taps: Vec<[f32; 4]> = taps.into_iter().map(|[x, y, w]| [x, y, w, 0.]).collect();
                let offsets = tap_bounds(&taps, mode == 2);
                let count = u32::try_from(taps.len()).map_err(|_| {
                    RenderError::Render("shadow kernel exceeds addressable storage".into())
                })?;
                let mut bytes = bytemuck::cast_slice(&[size.0, size.1, count, mode]).to_vec();
                bytes.extend_from_slice(bytemuck::cast_slice(&taps));
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("silhouette kernel"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::STORAGE,
                });
                self.kernels[slot] = Some(Kernel {
                    key,
                    compute: slot != 0 && (axis[0] == 0.0 || axis[1] == 0.0),
                    horizontal: axis[1] == 0.0,
                    count,
                    offsets,
                    buffer,
                });
            }
            if support[2] == 0 || support[3] == 0 {
                // The input is identically transparent, so the output is
                // too — and it is already in place: the cleared capture
                // is never touched. Validation above still ran.
                continue;
            }
            let kernel = self.kernels[slot].as_ref().expect("kernel prepared");
            support = expand(support, kernel.offsets, size);
            self.dispatch(device, encoder, input, size, slot, support);
            dispatches += 1;
        }
        Ok(dispatches)
    }

    fn dispatch(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &super::ScratchTarget,
        size: (u32, u32),
        slot: usize,
        region: [u32; 4],
    ) {
        let kernel = self.kernels[slot].as_ref().expect("kernel prepared");
        let temporary = self.temporary.as_ref().expect("intermediate prepared");
        if kernel.compute {
            let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("silhouette convolution region"),
                contents: bytemuck::cast_slice(&[
                    region[0],
                    region[1],
                    region[2],
                    region[3],
                    kernel.count,
                    u32::from(kernel.horizontal),
                    0,
                    0,
                ]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("silhouette shared convolution"),
                layout: &self.compute_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&input.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: kernel.buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&temporary.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: parameters.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("silhouette shared convolution"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute_pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            let (along, across) = if kernel.horizontal {
                size
            } else {
                (size.1, size.0)
            };
            pass.dispatch_workgroups(along.div_ceil(64), across, 1);
        } else {
            self.dispatch_fragment(device, encoder, input, slot, region);
        }
        encoder.copy_texture_to_texture(
            temporary.texture.as_image_copy(),
            input.texture.as_image_copy(),
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
    }
    fn dispatch_fragment(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &super::ScratchTarget,
        slot: usize,
        region: [u32; 4],
    ) {
        let kernel = self.kernels[slot].as_ref().expect("kernel prepared");
        let temporary = self.temporary.as_ref().expect("intermediate prepared");
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("silhouette blur"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: kernel.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("silhouette convolution"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &temporary.view,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        // The full-target triangle still runs; the scissor drops the
        // fragments whose inputs the kernel's reach cannot make nonzero.
        pass.set_scissor_rect(region[0], region[1], region[2], region[3]);
        pass.draw(0..3, 0..1);
        drop(pass);
    }
}

fn tap_bounds(taps: &[[f32; 4]], erosion: bool) -> [f32; 4] {
    let mut offsets = [
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ];
    for tap in taps {
        offsets[0] = offsets[0].min(tap[0]);
        offsets[1] = offsets[1].max(tap[0]);
        offsets[2] = offsets[2].min(tap[1]);
        offsets[3] = offsets[3].max(tap[1]);
    }
    if erosion {
        // Erosion seeds its result from the unshifted sample.
        offsets[0] = offsets[0].min(0.0);
        offsets[1] = offsets[1].max(0.0);
        offsets[2] = offsets[2].min(0.0);
        offsets[3] = offsets[3].max(0.0);
    }
    offsets
}

/// The output interval of one stage along an axis whose taps span
/// `[min_tap, max_tap]`: output `p` samples `floor(p + t)` and its `+1`
/// bilinear neighbour, so nonzero input in `[lo, hi)` can only leave
/// outputs in `[lo - max_tap - 1, hi - min_tap + 1)` nonzero — clamped to
/// `[0, size)`. `error` covers the GPU's f32 rounding of `p + t` at
/// magnitudes up to `size + |t|`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the interval is floor/ceil-clamped to [0, size] before conversion"
)]
fn axis_support(lo: u32, hi: u32, min_tap: f64, max_tap: f64, size: u32) -> (u32, u32) {
    let error = f64::from(f32::EPSILON) * (f64::from(size) + min_tap.abs().max(max_tap.abs()));
    let out_lo = (f64::from(lo) - max_tap - 1.0 - error)
        .floor()
        .clamp(0.0, f64::from(size));
    let out_hi = (f64::from(hi) - min_tap + 1.0 + error)
        .ceil()
        .clamp(0.0, f64::from(size));
    (out_lo as u32, out_hi as u32)
}

/// The `(x, y, w, h)` region a stage's output can be nonzero in, given
/// the input `support` and the kernel's per-axis tap `offsets`
/// `[min_x, max_x, min_y, max_y]`; `[0, 0, 0, 0]` when no output pixel
/// can reach the input.
fn expand(support: [u32; 4], offsets: [f32; 4], size: (u32, u32)) -> [u32; 4] {
    if support[2] == 0 || support[3] == 0 {
        return [0, 0, 0, 0];
    }
    let (x0, x1) = axis_support(
        support[0],
        support[0] + support[2],
        f64::from(offsets[0]),
        f64::from(offsets[1]),
        size.0,
    );
    let (y0, y1) = axis_support(
        support[1],
        support[1] + support[3],
        f64::from(offsets[2]),
        f64::from(offsets[3]),
        size.1,
    );
    if x1 <= x0 || y1 <= y0 {
        return [0, 0, 0, 0];
    }
    [x0, y0, x1 - x0, y1 - y0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_support_expands_by_the_kernels_reach() {
        // Output p reads floor(p + t) and its +1 neighbour for taps
        // t ∈ [min, max]: x = 5 reaches input 8 at t = 3 (floor(8..9)),
        // x = 23 reaches 20 at t = -3 (floor(19..20) + 1) — and the
        // rounding margin adds one integer pixel on each side.
        assert_eq!(axis_support(10, 20, -2.0, 3.0, 64), (5, 24));
        // The interval clamps to the capture.
        assert_eq!(axis_support(0, 5, -2.0, 3.0, 64), (0, 9));
        assert_eq!(axis_support(10, 64, -2.0, 3.0, 64), (5, 64));
        // Taps that cannot reach the input leave no support.
        assert_eq!(axis_support(10, 20, 100.0, 200.0, 64), (0, 0));
        assert_eq!(axis_support(10, 20, -200.0, -100.0, 64), (64, 64));
    }

    #[test]
    fn expand_applies_each_axis_independently() {
        assert_eq!(expand([10, 10, 0, 10], [-3.0; 4], (64, 64)), [0; 4]);
        assert_eq!(
            expand([10, 10, 10, 10], [-2.0, 3.0, -1.0, 1.0], (64, 64)),
            [5, 7, 19, 16]
        );
        assert_eq!(
            expand([10, 10, 10, 10], [100.0, 200.0, -1.0, 1.0], (64, 64)),
            [0, 0, 0, 0]
        );
    }
}
