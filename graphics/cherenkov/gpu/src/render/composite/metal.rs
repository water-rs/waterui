//! A tile epoch is one tracked wgpu pass on the engine's Metal queue.

use std::borrow::Cow;

use cherenkov::FrameStats;
use objc2_metal::{MTLResource, MTLStorageMode};
use rustc_hash::FxHashMap;

use crate::render::lower::{PipelineKind, ShaderVariant, Target};
use crate::render::{Bind1Key, ScratchTarget, SurfaceState, create_target, make_bind1};

#[derive(Default)]
pub struct Attachments {
    textures: FxHashMap<(u32, u32), AttachmentSet>,
}

#[derive(Default)]
struct AttachmentSet {
    targets: Vec<ScratchTarget>,
    used: bool,
}

impl Attachments {
    fn get(&mut self, device: &wgpu::Device, size: (u32, u32), count: u8) -> &[ScratchTarget] {
        let entry = self.textures.entry(size).or_default();
        entry.used = true;
        let textures = &mut entry.targets;
        while textures.len() < usize::from(count) {
            let (texture, view) = create_target(
                device,
                "tile intermediate",
                size,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TRANSIENT_ATTACHMENT,
                wgpu::TextureFormat::Rgba16Float,
            );
            // SAFETY: the guard retains the texture and only reads its storage mode.
            let native = unsafe { texture.as_hal::<wgpu::hal::metal::Api>() }
                .expect("Metal attachment executor requires a Metal texture");
            assert_eq!(
                native.raw_handle().storageMode(),
                MTLStorageMode::Memoryless,
                "the Apple tile executor requires memoryless attachment storage"
            );
            drop(native);
            textures.push(ScratchTarget {
                texture,
                view,
                width: size.0,
                height: size.1,
            });
        }
        &textures[..usize::from(count)]
    }

    pub fn clear(&mut self) {
        self.textures.clear();
    }

    /// Animated bounds must not retain every historical attachment size.
    pub fn finish_frame(&mut self) {
        self.textures
            .retain(|_, entry| std::mem::take(&mut entry.used));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    target: u8,
    source: u8,
    variant: u8,
    replace: bool,
    slots: u8,
}

#[derive(Default)]
pub struct Executor {
    module: Option<wgpu::ShaderModule>,
    pipelines: FxHashMap<Key, wgpu::RenderPipeline>,
}

impl Executor {
    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        key: Key,
    ) -> &wgpu::RenderPipeline {
        let module = self.module.get_or_insert_with(|| {
            let mut entries: Vec<wgpu::PassthroughShaderEntryPoint<'static>> =
                ["vs_main", "tile_clear_vertex"]
                    .into_iter()
                    .map(|name| wgpu::PassthroughShaderEntryPoint {
                        name: Cow::Borrowed(name),
                        workgroup_size: (0, 0, 0),
                    })
                    .collect();
            for target in 0..4 {
                for source in ["simple", "shadow", "clear", "0", "1", "2", "3"] {
                    entries.push(wgpu::PassthroughShaderEntryPoint {
                        name: Cow::Owned(format!("tile_{target}_{source}")),
                        workgroup_size: (0, 0, 0),
                    });
                }
            }
            // SAFETY: build.rs validates the shared WGSL and compiles the
            // native interfaces against it. Bindings and colour slots match
            // this executor's fixed layout and pipeline key.
            unsafe {
                device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                    label: Some("tile composition"),
                    metallib: Some(Cow::Borrowed(include_bytes!(concat!(
                        env!("OUT_DIR"),
                        "/engine_tile.metallib"
                    )))),
                    entry_points: Cow::Owned(entries),
                    ..Default::default()
                })
            }
        });
        self.pipelines.entry(key).or_insert_with(|| {
            let entry = match key.variant {
                0 => format!("tile_{}_simple", key.target),
                1 => format!("tile_{}_shadow", key.target),
                2 => format!("tile_{}_{}", key.target, key.source),
                3 => format!("tile_{}_clear", key.target),
                _ => unreachable!("closed tile shader set"),
            };
            let targets: Vec<_> = (0..key.slots)
                .map(|slot| {
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: (!key.replace && slot == key.target)
                            .then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: if slot == key.target {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::empty()
                        },
                    })
                })
                .collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&entry),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(if key.variant == 3 {
                        "tile_clear_vertex"
                    } else {
                        "vs_main"
                    }),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(&entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &targets,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        })
    }
}

/// Borrowed resources shared with the materialized encoder; attachment
/// textures are never installed in sampled bind groups.
pub struct Resources<'a> {
    pub device: &'a wgpu::Device,
    pub layout: &'a wgpu::PipelineLayout,
    pub layout1: &'a wgpu::BindGroupLayout,
    pub dummy: &'a wgpu::TextureView,
    pub bind0: &'a wgpu::BindGroup,
    pub images: &'a FxHashMap<u64, crate::render::GpuImage>,
    pub bitmaps: &'a FxHashMap<crate::render::bitmap::BitmapKey, crate::render::GpuBitmap>,
    pub atlas: &'a crate::render::glyph::Atlas,
}

#[expect(
    clippy::too_many_lines,
    reason = "one tracked render pass for one planned tile epoch"
)]
#[expect(
    clippy::cast_precision_loss,
    reason = "texture coordinates fit exactly in f32"
)]
pub fn encode(
    executor: &mut Executor,
    resources: &Resources<'_>,
    surf: &mut SurfaceState,
    first_pass: usize,
    encoder: &mut wgpu::CommandEncoder,
    timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    stats: &mut FrameStats,
) {
    let epoch = surf
        .composition
        .epoch_at(first_pass)
        .expect("planned tile epoch");
    let output = &surf.composition.images[epoch.output.0];
    let (view, texture) = match output.target {
        Target::Part(part) => surf.part(part),
        Target::Scratch(depth) => (&surf.scratch[&depth].view, &surf.scratch[&depth].texture),
        _ => unreachable!("planner restricts persistent tile outputs"),
    };
    let view = view.clone();
    let size = (texture.width(), texture.height());
    let first_output = epoch
        .ops
        .iter()
        .find(|op| op.target == 0)
        .expect("epoch writes its output");
    let load = surf.frame.passes[first_output.pass]
        .clear
        .map_or(wgpu::LoadOp::Load, |c| {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: f64::from(c[0]),
                g: f64::from(c[1]),
                b: f64::from(c[2]),
                a: f64::from(c[3]),
            })
        });
    let temporaries = surf
        .tile_targets
        .get(resources.device, size, epoch.slots - 1);
    let mut attachments = [None, None, None, None];
    attachments[0] = Some(wgpu::RenderPassColorAttachment {
        view: &view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    });
    for (slot, target) in temporaries.iter().enumerate() {
        attachments[slot + 1] = Some(wgpu::RenderPassColorAttachment {
            view: &target.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Discard,
            },
        });
    }
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("tile composition"),
        color_attachments: &attachments[..usize::from(epoch.slots)],
        depth_stencil_attachment: None,
        timestamp_writes: timestamps,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let mut used = [false; 4];
    for op in &epoch.ops {
        tracing::trace!(pass = op.pass, target = op.target, backdrop = ?op.backdrop, "tile operation");
        let canonical = &surf.frame.passes[op.pass];
        let [x, y, width, height] = canonical.region;
        let x = x - epoch.region[0];
        let y = y - epoch.region[1];
        pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(x, y, width, height);
        if op.begin && used[usize::from(op.target)] {
            pass.set_pipeline(executor.pipeline(
                resources.device,
                resources.layout,
                Key {
                    target: op.target,
                    source: op.target,
                    variant: 3,
                    replace: true,
                    slots: epoch.slots,
                },
            ));
            pass.draw(0..3, 0..1);
            stats.draws += 1;
            stats.pipeline_switches += 1;
        }
        used[usize::from(op.target)] = true;
        pass.set_bind_group(
            0,
            resources.bind0,
            &[(surf.globals_base + u32::try_from(op.pass).unwrap()) * 256],
        );
        for (range, source) in canonical.ranges.iter().zip(&op.sources) {
            let variant = match range.variant {
                ShaderVariant::Simple => 0,
                ShaderVariant::Shadow => 1,
                ShaderVariant::Full => 2,
            };
            pass.set_pipeline(executor.pipeline(
                resources.device,
                resources.layout,
                Key {
                    target: op.target,
                    source: source.unwrap_or(op.target),
                    variant,
                    replace: range.pipeline == PipelineKind::Replace,
                    slots: epoch.slots,
                },
            ));
            let key: Bind1Key = (None, false, range.image.clone(), range.mask);
            let bind = surf.binds1.entry(key).or_insert_with(|| {
                stats.bind_groups_created += 1;
                make_bind1(
                    resources.device,
                    resources.layout1,
                    resources.dummy,
                    None,
                    None,
                    range.image.as_ref().map(|image| {
                        crate::render::image_view(
                            image,
                            resources.images,
                            resources.bitmaps,
                            &surf.shader_textures,
                        )
                    }),
                    range.mask.map(|key| {
                        resources
                            .atlas
                            .mask_texture_view(key)
                            .expect("prepared mask")
                    }),
                )
            });
            pass.set_bind_group(1, &*bind, &[]);
            pass.draw(
                0..6,
                surf.inst_base + range.instances.start..surf.inst_base + range.instances.end,
            );
            stats.draws += 1;
            stats.pipeline_switches += 1;
        }
    }
}
