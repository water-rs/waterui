//! A tile epoch is one tracked wgpu pass on the engine's Metal queue.

use std::borrow::Cow;

use cherenkov::FrameStats;
use objc2_metal::{MTLResource, MTLStorageMode};
use rustc_hash::FxHashMap;

use crate::render::composition as report;
use crate::render::lower::{PipelineKind, ShaderVariant, Target};
use crate::render::{Bind1Key, ScratchTarget, SurfaceState, create_target, make_bind1};

/// Attachment slots this executor binds at once, slot zero — the
/// epoch's stored output — included.
///
/// `attachment.metal` declares the closed set this names:
/// `TILE_OUTPUT(D)` for every `D` below `SLOTS`, giving each target its
/// simple, shadow, clear and self-sourced fragment, plus
/// `TILE_COMPOSITE(D,S)` for every other pair. Every target and source
/// slot the planner can assign within [`crate::render::composite::plan::Limits::slots`]
/// therefore has an entry point.
pub const SLOTS: u8 = 4;

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

    /// The bytes of memoryless attachment storage this frame's epochs
    /// bound: what the same attachments would have cost in device memory
    /// had the hardware not kept them in tile memory (#2547). Sizes the
    /// frame did not use are excluded, so it is this frame's live set.
    pub fn memoryless_bytes(&self) -> u64 {
        self.textures
            .iter()
            .filter(|(_, entry)| entry.used)
            .map(|((width, height), entry)| {
                u64::from(*width)
                    * u64::from(*height)
                    * crate::render::texel_bytes(wgpu::TextureFormat::Rgba16Float)
                    * u64::try_from(entry.targets.len()).unwrap_or(u64::MAX)
            })
            .sum()
    }

    /// Every live memoryless attachment's bytes, this frame's and the
    /// sizes a previous frame left allocated.
    ///
    /// This is the steady-state figure `Engine::memory` reports, so
    /// unlike [`Self::memoryless_bytes`] it cannot be scoped to a
    /// frame: between frames nothing is "used", and the textures are
    /// still there.
    pub fn reserved_bytes(&self) -> u64 {
        self.textures
            .iter()
            .map(|((width, height), entry)| {
                u64::from(*width)
                    * u64::from(*height)
                    * crate::render::texel_bytes(wgpu::TextureFormat::Rgba16Float)
                    * u64::try_from(entry.targets.len()).unwrap_or(u64::MAX)
            })
            .sum()
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

/// `engine_tile.metallib` — the short-gradient tile module.
const TILE_METALLIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/engine_tile.metallib"));
/// `engine_tile_long.metallib` — the variant-4 tile module.
const LONG_TILE_METALLIB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/engine_tile_long.metallib"));
/// `engine_tile_image.metallib` — the variant-6 (image source) tile module.
const IMAGE_TILE_METALLIB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/engine_tile_image.metallib"));
/// Solid clipped glyph tile module.
const GLYPH_TILE_METALLIB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/engine_tile_glyph.metallib"));
/// Span composite tile module.
const COMPOSITE_TILE_METALLIB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/engine_tile_composite.metallib"));

/// The passthrough module for one tile family: the declared entries are
/// identical across families — the attachment interfaces wrap whichever
/// `fs_*` bodies the family's preincluded source carried.
fn tile_module(device: &wgpu::Device, metallib: &'static [u8]) -> wgpu::ShaderModule {
    let mut entries: Vec<wgpu::PassthroughShaderEntryPoint<'static>> =
        ["vs_main", "tile_clear_vertex"]
            .into_iter()
            .map(|name| wgpu::PassthroughShaderEntryPoint {
                name: Cow::Borrowed(name),
                workgroup_size: (0, 0, 0),
                workgroup_memory_sizes: std::borrow::Cow::Borrowed(&[]),
            })
            .collect();
    for target in 0..SLOTS {
        let sources = ["simple", "shadow", "clear"]
            .map(String::from)
            .into_iter()
            .chain((0..SLOTS).map(|source| source.to_string()));
        for source in sources {
            entries.push(wgpu::PassthroughShaderEntryPoint {
                name: Cow::Owned(format!("tile_{target}_{source}")),
                workgroup_size: (0, 0, 0),
                workgroup_memory_sizes: std::borrow::Cow::Borrowed(&[]),
            });
        }
    }
    // SAFETY: build.rs validates the shared WGSL and compiles the
    // native interfaces against it. Bindings and colour slots match
    // this executor's fixed layout and pipeline key.
    unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some("tile composition"),
            metallib: Some(Cow::Borrowed(metallib)),
            entry_points: Cow::Owned(entries),
            ..Default::default()
        })
    }
}

#[derive(Default)]
pub struct Executor {
    /// The short tile module, created on its family's first pipeline.
    module: Option<wgpu::ShaderModule>,
    /// The long-gradient tile module, created on its family's first
    /// pipeline.
    module_long: Option<wgpu::ShaderModule>,
    /// The image-source tile module, created on its family's first
    /// pipeline.
    module_image: Option<wgpu::ShaderModule>,
    /// Solid clipped glyph module.
    module_glyph: Option<wgpu::ShaderModule>,
    /// Span composite module.
    module_composite: Option<wgpu::ShaderModule>,
    pipelines: FxHashMap<Key, wgpu::RenderPipeline>,
}

impl Executor {
    /// The pipeline for `key`, created on first use. `created` is set
    /// when this call built it — the attribution report's first-use
    /// event (#2547); the flag costs one store, so the hot path pays
    /// nothing for it.
    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        key: Key,
        created: &mut bool,
    ) -> &wgpu::RenderPipeline {
        // The module is the family's, the entries are shared: variants 4
        // and 6 draw through the full entry names in their own modules.
        let module = match key.variant {
            4 => self
                .module_long
                .get_or_insert_with(|| tile_module(device, LONG_TILE_METALLIB)),
            6 => self
                .module_image
                .get_or_insert_with(|| tile_module(device, IMAGE_TILE_METALLIB)),
            7 => self
                .module_glyph
                .get_or_insert_with(|| tile_module(device, GLYPH_TILE_METALLIB)),
            8 => self
                .module_composite
                .get_or_insert_with(|| tile_module(device, COMPOSITE_TILE_METALLIB)),
            _ => self
                .module
                .get_or_insert_with(|| tile_module(device, TILE_METALLIB)),
        };
        self.pipelines.entry(key).or_insert_with(|| {
            *created = true;
            let entry = match key.variant {
                0 => format!("tile_{}_simple", key.target),
                1 => format!("tile_{}_shadow", key.target),
                2 | 4 | 5 | 6 | 7 | 8 => format!("tile_{}_{}", key.target, key.source),
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

/// Records a first-use pipeline creation when the attribution report is
/// collecting; with no report this is one `Option` test (#2547).
fn note(
    pipelines: &mut Option<&mut Vec<report::PipelineCreated>>,
    created: bool,
    pass: usize,
    key: Key,
) {
    if !created {
        return;
    }
    if let Some(pipelines) = pipelines {
        pipelines.push(report::PipelineCreated {
            pass,
            target: key.target,
            source: key.source,
            variant: match key.variant {
                0 => report::Variant::Simple,
                1 => report::Variant::Shadow,
                2 | 4 | 6 | 7 | 8 => report::Variant::Source,
                3 => report::Variant::Clear,
                _ => unreachable!("closed tile shader set"),
            },
            replace: key.replace,
            slots: key.slots,
        });
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

/// Where one epoch's encode records: the command encoder it is written
/// to, that pass's timestamp writes, the frame statistics it counts into,
/// and the attribution collector when a report is being built (#2547).
pub struct Recording<'a, 'b> {
    /// The encoder the epoch's single render pass is recorded on.
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// Timestamp writes bracketing the pass, when the engine times it.
    pub timestamps: Option<wgpu::RenderPassTimestampWrites<'b>>,
    /// The frame's statistics.
    pub stats: &'a mut FrameStats,
    /// First-use tile pipeline creations, when a report is collecting.
    pub pipelines: Option<&'a mut Vec<report::PipelineCreated>>,
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
    recording: Recording<'_, '_>,
) {
    let Recording {
        encoder,
        timestamps,
        stats,
        mut pipelines,
    } = recording;
    let epoch = surf
        .composition
        .epoch_at(first_pass)
        .expect("planned tile epoch");
    let output = &surf.composition.images[epoch.output.0];
    let (view, texture) = match output.target {
        Target::Part(part) => surf.part(part),
        Target::Scratch(_) => {
            let slot = output.backing.expect("tile output has backing");
            (&surf.scratch[&slot].view, &surf.scratch[&slot].texture)
        }
        _ => unreachable!("planner restricts persistent tile outputs"),
    };
    let view = view.clone();
    let size = (texture.width(), texture.height());
    let first_output = epoch
        .ops
        .iter()
        .find(|op| op.target == 0)
        .expect("epoch writes its output");
    let partial_clear = surf.repaint_clears.instance(first_output.pass);
    if let Some(instance) = partial_clear {
        let bind = surf
            .binds1
            .entry((None, None, None, None))
            .or_insert_with(|| {
                stats.bind_groups_created += 1;
                make_bind1(
                    resources.device,
                    resources.layout1,
                    resources.dummy,
                    None,
                    None,
                    None,
                    None,
                )
            });
        let key = Key {
            target: 0,
            source: 0,
            variant: 0,
            replace: true,
            slots: 1,
        };
        let mut created = false;
        crate::render::repaint::ClearDraw {
            timestamps: timestamps
                .as_ref()
                .map(|writes| wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: writes.beginning_of_pass_write_index,
                    end_of_pass_write_index: None,
                }),
            pipeline: executor.pipeline(resources.device, resources.layout, key, &mut created),
            bind0: resources.bind0,
            bind1: bind,
            offset: (surf.globals_base + u32::try_from(first_output.pass).expect("pass index"))
                * 256,
            instance: surf.inst_base + instance,
        }
        .encode(
            encoder,
            &view,
            &crate::render::repaint::Scissors::new(
                surf.draw_demand.pass(first_output.pass),
                epoch.region,
            ),
        );
        note(&mut pipelines, created, first_output.pass, key);
        stats.passes += 1;
        stats.draws += u32::try_from(surf.draw_demand.pass(first_output.pass).rectangles().len())
            .expect("damage rectangle count");
    }
    let load = surf.frame.passes[first_output.pass]
        .clear
        .filter(|_| partial_clear.is_none())
        .map_or(wgpu::LoadOp::Load, |c| {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: f64::from(c[0]),
                g: f64::from(c[1]),
                b: f64::from(c[2]),
                a: f64::from(c[3]),
            })
        });
    let timestamps = timestamps.map(|mut writes| {
        if partial_clear.is_some() {
            writes.beginning_of_pass_write_index = None;
        }
        writes
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
    let mut used = [false; SLOTS as usize];
    // The state the pass has already been given. An epoch spans many
    // canonical passes that share a region, a pipeline or a sampled
    // binding, and rebinding what is already bound is the executor's own
    // host cost (#2549).
    let mut viewport: Option<[u32; 4]> = None;
    let mut bound: Option<(Option<crate::render::lower::ImageSource>, Option<u64>)> = None;
    let origin = |region: [u32; 4]| {
        [
            region[0] - epoch.region[0],
            region[1] - epoch.region[1],
            region[2],
            region[3],
        ]
    };
    let place = |pass: &mut wgpu::RenderPass<'_>, viewport: &mut Option<[u32; 4]>, region| {
        let region = origin(region);
        if *viewport == Some(region) {
            return;
        }
        *viewport = Some(region);
        let [x, y, width, height] = region;
        pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(x, y, width, height);
    };
    for op in &epoch.ops {
        tracing::trace!(pass = op.pass, target = op.target, backdrop = ?op.backdrop, "tile operation");
        let canonical = &surf.frame.passes[op.pass];
        if op.begin && used[usize::from(op.target)] {
            // The slot still holds an image this epoch has finished with,
            // over a region this draw need not cover. The planner's clear
            // region is everywhere the opening image is written, which
            // bounds every read of it.
            place(&mut pass, &mut viewport, op.clear_region);
            let key = Key {
                target: op.target,
                source: op.target,
                variant: 3,
                replace: true,
                slots: epoch.slots,
            };
            let mut created = false;
            pass.set_pipeline(executor.pipeline(
                resources.device,
                resources.layout,
                key,
                &mut created,
            ));
            note(&mut pipelines, created, op.pass, key);
            pass.draw(0..3, 0..1);
            stats.draws += 1;
            stats.pipeline_switches += 1;
        }
        used[usize::from(op.target)] = true;
        place(&mut pass, &mut viewport, canonical.region);
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
                // A union member is `PAINT_BACKDROP` sampling its group's
                // capture: a neighbourhood (or effect) read that
                // `ExecutionPlan::epoch` never admits into a tile epoch.
                // `Cell` is a draw module (`DrawRange::module`), never a
                // range's lowered variant.
                ShaderVariant::Cell => unreachable!("no range is lowered with the cell module"),
                ShaderVariant::Union => {
                    unreachable!("a union sample reads its capture; no tile epoch admits it")
                }
                ShaderVariant::LongGradient => 4,
                ShaderVariant::Image => 6,
                ShaderVariant::Glyph => 7,
                ShaderVariant::Composite => 8,
            };
            let key = Key {
                target: op.target,
                source: source.unwrap_or(op.target),
                variant,
                replace: range.pipeline == PipelineKind::Replace,
                slots: epoch.slots,
            };
            let mut created = false;
            pass.set_pipeline(executor.pipeline(
                resources.device,
                resources.layout,
                key,
                &mut created,
            ));
            note(&mut pipelines, created, op.pass, key);
            // A tile epoch's draws sample nothing but their own image and
            // mask, and a run of them shares both — most of an epoch's
            // draws want the group already bound, so neither the key nor
            // the lookup is built for them.
            if !bound
                .as_ref()
                .is_some_and(|(image, mask)| *image == range.image && *mask == range.mask)
            {
                bound = Some((range.image.clone(), range.mask));
                let key: Bind1Key = (None, None, range.image.clone(), range.mask);
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
            }
            let scissors =
                crate::render::repaint::Scissors::new(surf.draw_demand.pass(op.pass), epoch.region)
                    .within(canonical.region);
            scissors.draw(&mut pass, |pass| {
                pass.draw(
                    0..6,
                    surf.inst_base + range.instances.start..surf.inst_base + range.instances.end,
                );
            });
            stats.draws += scissors.draw_count();
            stats.pipeline_switches += 1;
        }
    }
}
