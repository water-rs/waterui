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
    /// Tap extrema `[min_x, max_x, min_y, max_y]` from the
    /// exact f32 taps, with `(0, 0)` folded in for the erosion
    /// initializer. Bounds the texel offsets the stage can sample.
    offsets: [f32; 4],
    buffer: wgpu::Buffer,
}

/// Fixed pipeline, reusable morphology/blur intermediates and a capture pyramid.
/// Bind groups retain replaced buffers until their queued commands complete.
pub struct Blur {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    kernels: [Option<Kernel>; 3],
    temporary: [Option<super::ScratchTarget>; 2],
    levels: Vec<super::ScratchTarget>,
    resampling: Vec<wgpu::Buffer>,
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
        Self {
            pipeline,
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("silhouette linear sampling"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            kernels: std::array::from_fn(|_| None),
            temporary: [None, None],
            levels: Vec::new(),
            resampling: Vec::new(),
        }
    }

    /// Release retained allocations without recompiling the fixed pipeline.
    pub fn trim(&mut self) {
        self.kernels = std::array::from_fn(|_| None);
        self.temporary = [None, None];
        self.levels.clear();
        self.resampling.clear();
    }

    pub fn gpu_bytes(&self) -> u64 {
        self.kernels
            .iter()
            .flatten()
            .map(|kernel| kernel.buffer.size())
            .sum::<u64>()
            + self
                .temporary
                .iter()
                .flatten()
                .chain(&self.levels)
                .map(super::ScratchTarget::bytes)
                .sum::<u64>()
            + self.resampling.iter().map(wgpu::Buffer::size).sum::<u64>()
    }

    /// Convolves `input`'s capture in place. `support` is the
    /// `(x, y, w, h)` region of the `size` capture that can hold nonzero
    /// pixels; each stage scissors its output to the region its taps can
    /// still reach from it. Returns the number of passes encoded.
    fn convolve(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &super::ScratchTarget,
        mut support: [u32; 4],
        parameters: Parameters,
        stages: std::ops::Range<usize>,
    ) -> Result<(u32, [u32; 4]), RenderError> {
        let size = (input.width, input.height);
        let intermediate = usize::from(stages.start != 0);
        if self.temporary[intermediate]
            .as_ref()
            .is_none_or(|t| (t.width, t.height) != size)
        {
            let (texture, view) = super::create_target(
                device,
                "silhouette intermediate",
                size,
                super::TARGET_USAGES,
                input.texture.format(),
            );
            self.temporary[intermediate] = Some(super::ScratchTarget {
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
            if !stages.contains(&slot)
                || (slot == 0 && parameters.spread == 0.)
                || (slot != 0 && parameters.sigma <= 0.)
            {
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
                    let mut taps = shadow_taps::gaussian_taps(axis, parameters.sigma, count_limit)?;
                    // Adjacent unit taps on a device axis share a linear
                    // interpolation interval. One weighted sample evaluates
                    // both; oblique taps cross different texel cells.
                    if axis[0] == 0.0 || axis[1] == 0.0 {
                        pair_linear_taps(&mut taps);
                    }
                    taps
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
        Ok((dispatches, support))
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
        let temporary = self.temporary[usize::from(slot != 0)]
            .as_ref()
            .expect("intermediate prepared");
        self.draw(device, encoder, input, temporary, &kernel.buffer, region);
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
    fn draw(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &super::ScratchTarget,
        output: &super::ScratchTarget,
        kernel: &wgpu::Buffer,
        region: [u32; 4],
    ) {
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
                    resource: kernel.as_entire_binding(),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        // The full-target triangle still runs; the scissor drops the
        // fragments whose inputs the kernel's reach cannot make nonzero.
        pass.set_scissor_rect(region[0], region[1], region[2], region[3]);
        pass.draw(0..3, 0..1);
    }

    fn resample(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &super::ScratchTarget,
        output: &super::ScratchTarget,
        scale: f32,
    ) {
        let mut bytes = bytemuck::cast_slice(&[input.width, input.height, 1, 3]).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&[scale, scale, 0.0, 0.0]));
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("silhouette resampling"),
            contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.draw(
            device,
            encoder,
            input,
            output,
            &buffer,
            [0, 0, output.width, output.height],
        );
        self.resampling.push(buffer);
    }

    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &super::ScratchTarget,
        size: (u32, u32),
        mut support: [u32; 4],
        parameters: Parameters,
    ) -> Result<u32, RenderError> {
        self.resampling.clear();
        let mut source = input.clone();
        source.width = size.0;
        source.height = size.1;
        let full_target = source.clone();
        let mut passes = 0;
        if parameters.spread == 0.0 {
            self.temporary[0] = None;
        } else {
            (passes, support) =
                self.convolve(device, encoder, &full_target, support, parameters, 0..1)?;
        }
        let (factor, scaled) = blur_resolution(parameters, size)?;
        let mut extent = size;
        let mut reconstruction_scale = 1.0;
        let levels = factor.ilog2() as usize;
        self.levels.truncate(levels);
        for level in 0..levels {
            extent = (extent.0.div_ceil(2), extent.1.div_ceil(2));
            if self
                .levels
                .get(level)
                .is_none_or(|target| (target.width, target.height) != extent)
            {
                let (texture, view) = super::create_target(
                    device,
                    "silhouette pyramid",
                    extent,
                    super::TARGET_USAGES,
                    input.texture.format(),
                );
                let target = super::ScratchTarget {
                    texture,
                    view,
                    width: extent.0,
                    height: extent.1,
                };
                if level == self.levels.len() {
                    self.levels.push(target);
                } else {
                    self.levels[level] = target;
                }
            }
            let destination = self.levels[level].clone();
            self.resample(device, encoder, &source, &destination, 2.0);
            source = destination;
            passes += 1;
            reconstruction_scale *= 0.5;
        }
        let end = [
            (support[0] + support[2]).div_ceil(factor),
            (support[1] + support[3]).div_ceil(factor),
        ];
        support = [
            support[0] / factor,
            support[1] / factor,
            end[0] - support[0] / factor,
            end[1] - support[1] / factor,
        ];
        passes += self
            .convolve(device, encoder, &source, support, scaled, 1..3)?
            .0;
        if factor != 1 {
            self.resample(device, encoder, &source, &full_target, reconstruction_scale);
            passes += 1;
        }
        Ok(passes)
    }
}

/// Keep the Gaussian's half-Nyquist response below f32 precision. The
/// box pyramid and bilinear reconstruction contribute (factor² - 1)/4
/// variance in each axis; remove it before factoring the remaining covariance.
fn blur_resolution(
    parameters: Parameters,
    size: (u32, u32),
) -> Result<(u32, Parameters), RenderError> {
    let [a, b, c, d, _, _] = parameters.transform.as_coeffs();
    if !parameters.sigma.is_finite() || ![a, b, c, d].into_iter().all(f64::is_finite) {
        return Err(RenderError::Render("non-finite shadow covariance".into()));
    }
    if parameters.sigma <= 0.0 {
        return Ok((1, parameters));
    }
    let xx = a.mul_add(a, c * c);
    let xy = a.mul_add(b, c * d);
    let yy = b.mul_add(b, d * d);
    let largest = ((xx + yy + (xx - yy).hypot(2.0 * xy)) * 0.5).sqrt();
    let smallest_sigma = parameters.sigma * a.mul_add(d, -b * c).abs() / largest;
    let minimum_sigma = (-2.0 * f64::from(f32::EPSILON).ln()).sqrt() * 2.0 / std::f64::consts::PI;
    let mut factor = 1u32;
    while factor <= size.0.max(size.1) / 2
        && smallest_sigma >= minimum_sigma * f64::from(factor) * 2.0
    {
        factor *= 2;
    }
    if factor == 1 {
        return Ok((1, parameters));
    }
    let scale = f64::from(factor);
    let variance = scale.mul_add(scale, -1.0) * 0.25;
    let sigma2 = parameters.sigma * parameters.sigma;
    let horizontal = sigma2.mul_add(xx, -variance).sqrt();
    let skew = sigma2 * xy / horizontal;
    let vertical = skew.mul_add(-skew, sigma2.mul_add(yy, -variance)).sqrt();
    Ok((
        factor,
        Parameters {
            transform: Affine::new([
                horizontal / scale,
                skew / scale,
                0.0,
                vertical / scale,
                0.0,
                0.0,
            ]),
            sigma: 1.0,
            spread: 0.0,
        },
    ))
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

/// Combine adjacent, axis-aligned unit Gaussian taps into linear samples.
#[expect(clippy::cast_possible_truncation, reason = "GPU taps use f32 storage")]
fn pair_linear_taps(taps: &mut Vec<[f32; 3]>) {
    let count = taps.len();
    for output in 0..count / 2 {
        let first = taps[output * 2].map(f64::from);
        let second = taps[output * 2 + 1].map(f64::from);
        let weight = first[2] + second[2];
        taps[output] = [
            (second[0].mul_add(second[2], first[0] * first[2]) / weight) as f32,
            (second[1].mul_add(second[2], first[1] * first[2]) / weight) as f32,
            weight as f32,
        ];
    }
    if !count.is_multiple_of(2) {
        taps[count / 2] = taps[count - 1];
    }
    taps.truncate(count.div_ceil(2));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_taps_preserve_gaussian_mass_and_interpolation() {
        for axis in [[1., 0.], [-1., 0.], [0., 2.], [0., -2.]] {
            for sigma in [0., 0.1, 1., 30.] {
                let original = shadow_taps::gaussian_taps(axis, sigma, 4096.).unwrap();
                let mut paired = original.clone();
                pair_linear_taps(&mut paired);
                assert_eq!(paired.len(), original.len().div_ceil(2));
                for (source, tap) in original.chunks(2).zip(&paired) {
                    let mass: f32 = source.iter().map(|tap| tap[2]).sum();
                    assert!((tap[2] - mass).abs() < 1e-7);
                    if let [first, second] = source {
                        let dimension = usize::from(axis[0] == 0.);
                        let phase = (tap[dimension] - first[dimension])
                            / (second[dimension] - first[dimension]);
                        assert!(tap[2].mul_add(phase, -second[2]).abs() < 1e-6);
                        assert!((0.0..=1.0).contains(&phase));
                    }
                }
            }
        }
    }

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
