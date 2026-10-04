//! Projective layers on the GPU (#84).
//!
//! A projective layer's subtree renders into a retained, layer-local
//! premultiplied linear-P3 RGBA16F texture at the planner's density, gets a
//! full area-average mip chain, and composes into its parent raster with
//! one quad drawn by the projective pipeline (`projective.wgsl`). A warm
//! layer whose content, density and layout are unchanged draws only that
//! quad: no local passes and no mip builds.

use cherenkov::LayerId;
use cherenkov::lowering::projective::{Homography, Limits, LocalImage};
use rustc_hash::{FxHashMap, FxHashSet};

use super::filter::FilterKey;
use super::instance::Stop;
use super::lower::ImageSource;

/// A retained local image: the projective layer and its density bucket,
/// `log2` of the texels per layer unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalKey {
    /// The projective layer.
    pub layer: LayerId,
    /// `log2(density)`.
    pub bucket: u32,
}

/// The density bucket of `density`, a power of two of at least one.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the planner's density is a power of two in 1..=2^20"
)]
pub fn bucket(density: f64) -> u32 {
    density.log2() as u32
}

/// How a planned layer composes into its parent raster this frame.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// The local image it samples.
    pub key: LocalKey,
    /// Parent raster pixels to base texels, front half-space at `w > 0`.
    pub inverse: Homography,
    /// Layer space to parent raster pixels.
    pub to_parent: Homography,
    /// The parent-raster pixels to shade, `[x0, y0, x1, y1)`.
    pub bounds: [u32; 4],
    /// Texels per layer unit.
    pub density: f64,
}

impl Placement {
    /// The placement of `image`, stored under `key`.
    #[must_use]
    pub const fn new(key: LocalKey, image: &LocalImage) -> Self {
        Self {
            key,
            inverse: image.inverse,
            to_parent: image.to_parent,
            bounds: image.bounds,
            density: image.density,
        }
    }

    /// The inverse homography rows for the composite, evaluated relative
    /// to the region origin (so the shader's device offsets stay small and
    /// exact in `f32`) and scaled by a positive power of two that brings
    /// the largest coefficient into `[0.5, 1)` — a positive scale keeps
    /// the front half-space.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "conditioned coefficients are stored as f32 instance data"
    )]
    pub fn record(&self) -> [Stop; 3] {
        let m = self.inverse.0;
        let (ox, oy) = (f64::from(self.bounds[0]), f64::from(self.bounds[1]));
        let rows = m.map(|r| [r[0], r[1], r[2] + r[0].mul_add(ox, r[1] * oy)]);
        let max = rows
            .iter()
            .flatten()
            .fold(0.0_f64, |acc, v| acc.max(v.abs()));
        let scale = if max > 0.0 {
            (-(max.log2().floor() + 1.0)).exp2()
        } else {
            1.0
        };
        rows.map(|r| Stop {
            color: [
                (r[0] * scale) as f32,
                (r[1] * scale) as f32,
                (r[2] * scale) as f32,
                0.0,
            ],
            offset: 0.0,
            pad: [0.0; 3],
        })
    }
}

/// What a retained local image was realized from. The layer's pose,
/// opacity and blend are absent: changing them only moves the sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    /// Layer space to base texels (density and the stable texel-grid
    /// origin).
    local_to_texel: [f64; 6],
    /// The base level size in texels.
    pub size: (u32, u32),
    /// [`cherenkov::SurfaceTree::content_stamp`] of the layer.
    stamp: u64,
    /// The renderer's image-replacement count.
    replacements: u64,
}

impl Key {
    /// The key of `image` at content stamp `stamp` after `replacements`
    /// image replacements.
    #[must_use]
    pub const fn new(image: &LocalImage, stamp: u64, replacements: u64) -> Self {
        Self {
            local_to_texel: image.local_to_texel.as_coeffs(),
            size: image.size,
            stamp,
            replacements,
        }
    }

    /// Whether the image was realized from the layer's content at
    /// `stamp` after `replacements` image replacements. Stamps and the
    /// count only grow, so an image that is not current can never be
    /// composed again.
    #[must_use]
    pub const fn is_current(&self, stamp: u64, replacements: u64) -> bool {
        self.stamp == stamp && self.replacements == replacements
    }
}

/// Renderer-side inputs a realization read that the content stamp does
/// not cover: they decide whether a retained image is still current.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deps {
    /// Filter chains its passes ran (layer filters and backdrop groups).
    pub filters: Vec<FilterKey>,
    /// Textures its instances sampled whose pixels change without a tree
    /// edit: GPU content, external frames and shader paints.
    pub images: Vec<ImageSource>,
    /// The surface's interop generation when it sampled GPU content or an
    /// external frame, bumped whenever either is installed or resized.
    pub interop: Option<u64>,
}

impl Deps {
    /// What the realization lowered into `frame` from filter entry
    /// `filters` and pass `passes` on reads, on a surface at interop
    /// generation `interop`.
    #[must_use]
    pub fn of(frame: &super::lower::Frame, filters: usize, passes: usize, interop: u64) -> Self {
        let mut deps = Self::default();
        for (_, key) in &frame.filters[filters..] {
            if !deps.filters.contains(key) {
                deps.filters.push(*key);
            }
        }
        for image in frame.passes[passes..]
            .iter()
            .flat_map(|pass| &pass.ranges)
            .filter_map(|range| range.image.as_ref())
        {
            let interop_source = matches!(image, ImageSource::Content(_));
            if interop_source {
                deps.interop = Some(interop);
            }
            if (interop_source || matches!(image, ImageSource::Shader(_)))
                && !deps.images.contains(image)
            {
                deps.images.push(image.clone());
            }
        }
        deps
    }
}

/// A local image this frame renders: allocated before encoding, and
/// current once the frame's encode succeeded.
#[derive(Clone, Debug)]
pub struct Realize {
    /// Where it is stored.
    pub key: LocalKey,
    /// What it is realized from.
    pub cache: Key,
    /// Every level's size, base first.
    pub levels: Vec<(u32, u32)>,
    /// What its rendering read.
    pub deps: Deps,
}

/// What one surface's lowering needs to plan and reuse local images.
pub struct Inputs {
    /// The admitted image dimension and bytes.
    pub limits: Limits,
    /// The renderer's image-replacement count.
    pub replacements: u64,
    /// The surface's interop generation.
    pub interop: u64,
    /// Retained images whose renderer-side inputs changed since they were
    /// realized.
    pub stale: FxHashSet<LocalKey>,
}

/// One retained local image and its mip chain.
pub struct Entry {
    /// Its density bucket.
    pub bucket: u32,
    /// What the texture holds; `None` until a realization completed.
    pub key: Option<Key>,
    /// The inputs its realization read.
    pub deps: Deps,
    /// The RGBA16F texture with every level.
    pub texture: wgpu::Texture,
    /// A view of every level, sampled by the composite.
    pub view: wgpu::TextureView,
    /// One view per level: level 0 is the render target, every level is
    /// a mip source or destination.
    pub levels: Vec<wgpu::TextureView>,
    /// Bind groups reading level `i` for building level `i + 1`.
    pub mips: Vec<wgpu::BindGroup>,
    /// Composite bind groups by `(blend backdrop slot, mask texture)`,
    /// valid under `binds_stamp`.
    pub binds: FxHashMap<(Option<usize>, Option<u64>), wgpu::BindGroup>,
    /// The `(surface bind generation, mask texture generation)` `binds`
    /// were built under.
    pub binds_stamp: (u64, u64),
    /// The renderer frame count that last composed it.
    pub last_used: u64,
}

impl Entry {
    /// Bytes of the texture: `8 · Σ w·h` over its levels.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        let size = self.texture.size();
        cherenkov::lowering::projective::mip_levels((size.width, size.height))
            .iter()
            .map(|&(w, h)| 8 * u64::from(w) * u64::from(h))
            .sum()
    }
}

impl Entry {
    /// A cleared entry for bucket `bucket` holding a new `levels`-level
    /// RGBA16F texture, its views and its mip bind groups.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a mip chain has at most 32 levels"
    )]
    pub fn new(
        device: &wgpu::Device,
        pipelines: &Pipelines,
        bucket: u32,
        levels: &[(u32, u32)],
    ) -> Self {
        let (w, h) = levels[0];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("projective image"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            // A capture inside the local image copies out of it; a
            // blended composite inside it copies it aside.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let views: Vec<wgpu::TextureView> = (0..levels.len() as u32)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("projective level"),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..wgpu::TextureViewDescriptor::default()
                })
            })
            .collect();
        let mips = views[..views.len() - 1]
            .iter()
            .map(|previous| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("projective mip"),
                    layout: &pipelines.mip_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(previous),
                    }],
                })
            })
            .collect();
        Self {
            bucket,
            key: None,
            deps: Deps::default(),
            texture,
            view,
            levels: views,
            mips,
            binds: FxHashMap::default(),
            binds_stamp: (u64::MAX, u64::MAX),
            last_used: 0,
        }
    }

    /// Whether the texture's base level is `size`.
    #[must_use]
    pub fn fits(&self, size: (u32, u32)) -> bool {
        let s = self.texture.size();
        (s.width, s.height) == size
    }

    /// The composite bind group reading this image with the blend backdrop
    /// `(slot, view)` and the mask texture `(key, view)`, rebuilt when the
    /// `(surface bind generation, mask texture generation)` stamp moved.
    pub fn bind(
        &mut self,
        device: &wgpu::Device,
        pipelines: &Pipelines,
        (slot, backdrop): (Option<usize>, Option<&wgpu::TextureView>),
        (mask_key, mask): (Option<u64>, Option<&wgpu::TextureView>),
        stamp: (u64, u64),
        dummy: &wgpu::TextureView,
    ) -> &wgpu::BindGroup {
        if self.binds_stamp != stamp {
            self.binds.clear();
            self.binds_stamp = stamp;
        }
        let view = &self.view;
        self.binds.entry((slot, mask_key)).or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("projective composite"),
                layout: &pipelines.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(backdrop.unwrap_or(dummy)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(mask.unwrap_or(dummy)),
                    },
                ],
            })
        })
    }

    /// Encodes one pass per mip level, each the area average of the level
    /// above it.
    pub fn build_mips(&self, encoder: &mut wgpu::CommandEncoder, pipelines: &Pipelines) {
        for (bind, level) in self.mips.iter().zip(&self.levels[1..]) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("projective mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: level,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipelines.mip);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// The projective pipelines and their layouts, built on the first frame
/// that composes a projective layer.
pub struct Pipelines {
    /// The composite's group-1 layout.
    pub layout: wgpu::BindGroupLayout,
    /// `[format index][replace]` composite pipelines: format 0 = surface
    /// (and local images), 1 = scratch; source-over or replace.
    pub composite: [[wgpu::RenderPipeline; 2]; 2],
    /// Linear/linear/linear clamp-to-edge sampling without hardware
    /// anisotropy: the shader integrates the anisotropic footprint.
    pub sampler: wgpu::Sampler,
    /// The mip level pipeline's layout.
    pub mip_layout: wgpu::BindGroupLayout,
    /// The mip level pipeline.
    pub mip: wgpu::RenderPipeline,
}

impl Pipelines {
    /// Builds the composite pipelines for the surface format and
    /// `scratch_format` over the engine's group-0 `layout0`, and the mip
    /// pipeline.
    pub fn new(
        device: &wgpu::Device,
        delivery: super::shaders::ShaderDelivery,
        layout0: &wgpu::BindGroupLayout,
        scratch_format: wgpu::TextureFormat,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cherenkov projective 1"),
            entries: &super::layout_entries(super::bindings::PROJECTIVE_GROUP1),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cherenkov projective"),
            bind_group_layouts: &[Some(layout0), Some(&layout)],
            immediate_size: 0,
        });
        let module = delivery.projective_module(device);
        let composite = [super::TARGET_FORMAT, scratch_format].map(|format| {
            [false, true].map(|replace| {
                let component = wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: if replace {
                        wgpu::BlendFactor::Zero
                    } else {
                        wgpu::BlendFactor::OneMinusSrcAlpha
                    },
                    operation: wgpu::BlendOperation::Add,
                };
                quad_pipeline(
                    device,
                    &pipeline_layout,
                    &module,
                    "fs_projective",
                    format,
                    Some(wgpu::BlendState {
                        color: component,
                        alpha: component,
                    }),
                )
            })
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("projective image"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 1,
            ..wgpu::SamplerDescriptor::default()
        });
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cherenkov mip"),
            entries: &super::layout_entries(super::bindings::MIP_GROUP0),
        });
        let mip_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cherenkov mip"),
            bind_group_layouts: &[Some(&mip_layout)],
            immediate_size: 0,
        });
        let mip = quad_pipeline(
            device,
            &mip_pipeline_layout,
            &delivery.mip_module(device),
            "fs_main",
            wgpu::TextureFormat::Rgba16Float,
            None,
        );
        Self {
            layout,
            composite,
            sampler,
            mip_layout,
            mip,
        }
    }
}

/// A triangle-list pipeline over `vs_main` and `fragment` of `module`,
/// without vertex buffers, writing `format` through `blend`.
fn quad_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    fragment: &'static str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov::kurbo::Affine;

    #[test]
    fn records_evaluate_relative_to_the_region_origin_with_a_positive_scale() {
        let inverse = Homography([[2.0, 0.5, -30.0], [0.25, 3.0, 12.0], [0.001, 0.0, 1.5]]);
        let placement = Placement {
            key: LocalKey {
                layer: LayerId::new(1),
                bucket: 0,
            },
            inverse,
            to_parent: Homography::affine(Affine::IDENTITY),
            bounds: [40, 24, 90, 60],
            density: 1.0,
        };
        let rows = placement.record();
        let (x, y) = (57.5, 31.5);
        let want = inverse.map(x, y);
        let d = [x - 40.0, y - 24.0];
        let got: Vec<f64> = rows
            .iter()
            .map(|r| {
                f64::from(r.color[2])
                    + f64::from(r.color[0]).mul_add(d[0], f64::from(r.color[1]) * d[1])
            })
            .collect();
        // Same projective point, positive homogeneous scale.
        let s = got[2] / want[2];
        assert!(s > 0.0);
        for (g, w) in got.iter().zip(want) {
            assert!(
                (g - w * s).abs() < 1e-5 * (w * s).abs().max(1.0),
                "{g} vs {}",
                w * s
            );
        }
        let max = rows
            .iter()
            .flat_map(|r| &r.color[..3])
            .fold(0.0_f32, |m, v| m.max(v.abs()));
        assert!((0.5..1.0).contains(&max), "{max}");
    }
}
