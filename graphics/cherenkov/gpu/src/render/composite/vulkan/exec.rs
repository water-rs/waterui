//! A tile epoch is one native `vkCmdBeginRendering` on the engine's own
//! wgpu device and queue.
//!
//! The epoch binds the same four attachment slots the Metal executor
//! binds — slot zero the epoch's stored output, slots one to three
//! transient — and records the same operations in the same order, with
//! the same clear, replace and source-over semantics. What differs is
//! only how a draw reaches the slots: Metal's fragment shaders declare
//! them as colour attachments in a raster-order group, and Vulkan's
//! read them as input attachments under
//! `VK_KHR_dynamic_rendering_local_read`, with
//! `VK_EXT_rasterization_order_attachment_access` supplying the
//! same-pixel ordering guarantee. `vkCmdSetRenderingAttachmentLocations`
//! and `vkCmdSetRenderingInputAttachmentIndices` point the one shader
//! output and the two shader reads at the draw's target and source
//! slots, which is what lets two SPIR-V modules cover every pair.
//!
//! wgpu cannot express any of that (`gfx-rs/wgpu#10148`), so the epoch
//! is recorded natively. It is still one command buffer in the frame's
//! own ordered submission: the frame encoder is finished before the
//! epoch and a fresh one opened after it, and the resources the epoch
//! touches are moved into known states with
//! `wgpu::CommandEncoder::transition_resources` on the encoder that
//! precedes it, so wgpu's tracker and the native recording agree on
//! every layout.

use std::ops::Range;

use ash::vk;
use cherenkov::FrameStats;
use rustc_hash::FxHashMap;

use super::mirror::{LOCAL_READ, Mirror, MirrorError, PipeKey, Stamp};
use crate::render::composition as report;
use crate::render::lower::{PipelineKind, ShaderVariant, Target};
use crate::render::{SurfaceState, split_encoder};

/// Borrowed engine resources the epoch's descriptor sets are written
/// from. The attachment images are the mirror's own and never appear
/// in a sampled binding.
pub struct Resources<'a> {
    pub device: &'a wgpu::Device,
    pub globals: &'a wgpu::Buffer,
    pub instances: &'a wgpu::Buffer,
    pub stops: &'a wgpu::Buffer,
    pub image_decode: &'a wgpu::Buffer,
    pub dummy: &'a wgpu::TextureView,
    /// The texture `dummy` views, so it can be transitioned with the
    /// rest of the epoch's sampled resources.
    pub dummy_texture: &'a wgpu::Texture,
    pub images: &'a FxHashMap<u64, crate::render::GpuImage>,
    pub bitmaps: &'a FxHashMap<crate::render::bitmap::BitmapKey, crate::render::GpuBitmap>,
    pub atlas: &'a crate::render::glyph::Atlas,
}

/// Where one epoch's encode records: the frame encoder it is spliced
/// into, the finished buffers of the frame's submission, the frame
/// statistics it counts into, and the attribution collector when a
/// report is being built (#2547).
pub struct Recording<'a> {
    /// The frame encoder. It is finished into `buffers` before the
    /// epoch's own command buffer and replaced with a fresh one, which
    /// the caller goes on recording into.
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The frame's command buffers, in submission order.
    pub buffers: &'a mut Vec<wgpu::CommandBuffer>,
    /// The query set and the two indices bracketing the epoch, when
    /// the engine times the pass. A native epoch is bracketed at
    /// command-buffer granularity: the first write is the last command
    /// of the preceding buffer and the second the first command of the
    /// following one, so the interval covers the epoch and nothing
    /// else of the frame.
    pub timestamps: Option<(&'a wgpu::QuerySet, u32, u32)>,
    /// The frame's statistics.
    pub stats: &'a mut FrameStats,
    /// First-use tile pipeline creations, when a report is collecting.
    pub pipelines: Option<&'a mut Vec<report::PipelineCreated>>,
}

/// One recorded draw, resolved before the native command buffer opens
/// so that nothing borrows the surface inside it.
struct Draw {
    key: PipeKey,
    /// The canonical pass the draw belongs to, for the attribution
    /// report's first-use event.
    pass: usize,
    /// Viewport and scissor in attachment-origin coordinates.
    region: [u32; 4],
    /// Byte offset of the pass's `Globals` window.
    dynamic_offset: u32,
    /// The draw's sampled image, when it has one.
    image: Option<vk::ImageView>,
    /// The draw's clip mask, when it has one.
    mask: Option<vk::ImageView>,
    /// Vertices and instances, matching the Metal executor's draws.
    vertices: Range<u32>,
    instances: Range<u32>,
}

/// Records the planned tile epoch that starts at `first_pass`.
///
/// # Errors
/// [`MirrorError`] when a descriptor set or a pipeline cannot be
/// produced, or when the device reports a failure.
///
/// # Panics
/// When `first_pass` does not open a planned epoch, when the epoch's
/// output is not a persistent target, or when a draw samples a
/// capture — none of which the planner can produce.
#[expect(
    clippy::too_many_lines,
    reason = "one native rendering for one planned tile epoch"
)]
pub fn encode(
    mirror: &mut Mirror,
    resources: &Resources<'_>,
    surf: &SurfaceState,
    images_gen: u64,
    first_pass: usize,
    recording: Recording<'_>,
) -> Result<(), MirrorError> {
    let Recording {
        encoder,
        buffers,
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
        Target::Scratch(depth) => (&surf.scratch[&depth].view, &surf.scratch[&depth].texture),
        _ => unreachable!("planner restricts persistent tile outputs"),
    };
    let size = (texture.width(), texture.height());
    let first_output = epoch
        .ops
        .iter()
        .find(|op| op.target == 0)
        .expect("epoch writes its output");
    let clear = surf.frame.passes[first_output.pass].clear;

    let stamp = Stamp {
        bind_gen: surf.bind_gen,
        images_gen,
        mask_gen: resources.atlas.mask_texture_generation(),
    };
    let dummy = crate::render::raw_vk_view(resources.dummy);
    let origin = |region: [u32; 4]| {
        [
            region[0] - epoch.region[0],
            region[1] - epoch.region[1],
            region[2],
            region[3],
        ]
    };

    // Resolve every draw, its pipeline key and its sampled views here,
    // where the surface may be borrowed. The native recording below
    // borrows only the mirror.
    let mut draws = Vec::new();
    let mut used = [false; super::SLOTS as usize];
    for op in &epoch.ops {
        tracing::trace!(pass = op.pass, target = op.target, backdrop = ?op.backdrop, "tile operation");
        let canonical = &surf.frame.passes[op.pass];
        let dynamic_offset = (surf.globals_base
            + u32::try_from(op.pass).expect("a frame's passes fit in u32"))
            * 256;
        if op.begin && used[usize::from(op.target)] {
            // The slot still holds an image this epoch has finished
            // with, over a region this draw need not cover. The
            // planner's clear region is everywhere the opening image is
            // written, which bounds every read of it.
            draws.push(Draw {
                key: PipeKey {
                    target: op.target,
                    source: op.target,
                    variant: 3,
                    replace: true,
                    slots: epoch.slots,
                },
                pass: op.pass,
                region: origin(op.clear_region),
                dynamic_offset,
                image: None,
                mask: None,
                vertices: 0..3,
                instances: 0..1,
            });
        }
        used[usize::from(op.target)] = true;
        for (range, source) in canonical.ranges.iter().zip(&op.sources) {
            let variant = match range.variant {
                ShaderVariant::Simple => 0,
                ShaderVariant::Shadow => 1,
                ShaderVariant::Full => 2,
                // A union member is `PAINT_BACKDROP` sampling its
                // group's capture: a neighbourhood read that
                // `ExecutionPlan::epoch` never admits into an epoch.
                ShaderVariant::Union => {
                    unreachable!("a union sample reads its capture; no tile epoch admits it")
                }
            };
            draws.push(Draw {
                key: PipeKey {
                    target: op.target,
                    source: source.unwrap_or(op.target),
                    variant,
                    replace: range.pipeline == PipelineKind::Replace,
                    slots: epoch.slots,
                },
                pass: op.pass,
                region: origin(canonical.region),
                dynamic_offset,
                image: range.image.as_ref().map(|image| {
                    crate::render::raw_vk_view(crate::render::image_view(
                        image,
                        resources.images,
                        resources.bitmaps,
                        &surf.shader_textures,
                    ))
                }),
                mask: range.mask.map(|key| {
                    crate::render::raw_vk_view(
                        resources
                            .atlas
                            .mask_texture_view(key)
                            .expect("prepared mask"),
                    )
                }),
                vertices: 0..6,
                instances: surf.inst_base + range.instances.start
                    ..surf.inst_base + range.instances.end,
            });
        }
    }

    let engine_set = mirror.engine_set(
        stamp,
        crate::render::raw_vk_buffer(resources.globals),
        crate::render::raw_vk_buffer(resources.instances),
        crate::render::raw_vk_buffer(resources.stops),
        crate::render::raw_vk_view(resources.atlas.view()),
        crate::render::raw_vk_buffer(resources.image_decode),
    )?;
    let slots = mirror.slots(size, epoch.slots - 1)?;
    let mut views = [vk::ImageView::null(); super::SLOTS as usize];
    let mut images = [vk::Image::null(); super::SLOTS as usize];
    for (slot, transient) in slots.iter().enumerate() {
        views[slot + 1] = transient.view();
        images[slot + 1] = transient.image();
    }
    views[0] = crate::render::raw_vk_view(view);
    images[0] = crate::render::raw_vk_image(texture);

    // The engine's own resources must be in the states the native
    // recording assumes before its command buffer runs. wgpu's tracker
    // is the authority on them, so the transition is requested from
    // wgpu on the encoder that precedes the epoch, not barriered
    // behind its back.
    let mut textures = vec![wgpu::TextureTransition {
        texture,
        selector: None,
        state: wgpu::TextureUses::COLOR_TARGET,
    }];
    textures.extend(sampled_textures(resources, surf, epoch).map(|texture| {
        wgpu::TextureTransition {
            texture,
            selector: None,
            state: wgpu::TextureUses::RESOURCE,
        }
    }));
    encoder.transition_resources(std::iter::empty(), textures.into_iter());
    if let Some((query_set, begin, _)) = timestamps {
        encoder.write_timestamp(query_set, begin);
    }
    buffers.push(split_encoder(encoder, resources.device));

    let mut op = resources
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tile composition"),
        });
    let slot_count = usize::from(epoch.slots);
    let result = {
        // SAFETY: `op` is a live Vulkan encoder that has recorded
        // nothing, so `hal`'s command buffer is recording and outside
        // any rendering; every handle `record` is given is live for
        // the call.
        unsafe {
            op.as_hal_mut::<wgpu::hal::vulkan::Api, _, _>(|hal| {
                let hal = hal.expect("vulkan encoder");
                let cb = hal.raw_handle();
                record(
                    mirror,
                    cb,
                    &Epoch {
                        views: &views[..slot_count],
                        images: &images[..slot_count],
                        extent: size,
                        clear,
                        engine_set,
                        dummy,
                        stamp,
                    },
                    &draws,
                    stats,
                    &mut pipelines,
                )
            })
        }
    };
    buffers.push(op.finish());
    if let Some((query_set, _, end)) = timestamps {
        encoder.write_timestamp(query_set, end);
    }
    result
}

/// The epoch-wide state one native recording needs.
struct Epoch<'a> {
    views: &'a [vk::ImageView],
    images: &'a [vk::Image],
    extent: (u32, u32),
    clear: Option<[f32; 4]>,
    engine_set: vk::DescriptorSet,
    dummy: vk::ImageView,
    stamp: Stamp,
}

/// Records the epoch into `cb`.
///
/// # Safety
/// `cb` must be a recording command buffer on the mirror's device with
/// no rendering open, and every handle in `epoch` and `draws` must be
/// live.
#[expect(
    clippy::cast_precision_loss,
    reason = "attachment coordinates are pixel counts and fit exactly in f32"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one rendering scope: its barriers, its attachments and its draws"
)]
unsafe fn record(
    mirror: &mut Mirror,
    cb: vk::CommandBuffer,
    epoch: &Epoch<'_>,
    draws: &[Draw],
    stats: &mut FrameStats,
    pipelines: &mut Option<&mut Vec<report::PipelineCreated>>,
) -> Result<(), MirrorError> {
    let device = mirror.device().clone();
    let layout = mirror.pipeline_layout();
    // Slot zero arrives in `COLOR_ATTACHMENT_OPTIMAL`, where the
    // preceding encoder's `transition_resources` put it; the transient
    // slots carry nothing worth preserving, so they come from
    // `UNDEFINED`. Both end in the layout that is simultaneously a
    // colour attachment and a local-read input attachment.
    let barriers: Vec<vk::ImageMemoryBarrier<'_>> = epoch
        .images
        .iter()
        .enumerate()
        .map(|(slot, image)| {
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::COLOR_ATTACHMENT_READ
                        | vk::AccessFlags::INPUT_ATTACHMENT_READ,
                )
                .old_layout(if slot == 0 {
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
                } else {
                    vk::ImageLayout::UNDEFINED
                })
                .new_layout(LOCAL_READ)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(*image)
                .subresource_range(COLOR)
        })
        .collect();
    // SAFETY: `cb` is recording outside a rendering and every barrier
    // names an image live for this call.
    unsafe {
        device.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                | vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &barriers,
        );
    }

    let attachments: Vec<vk::RenderingAttachmentInfo<'_>> = epoch
        .views
        .iter()
        .enumerate()
        .map(|(slot, view)| {
            let info = vk::RenderingAttachmentInfo::default()
                .image_view(*view)
                .image_layout(LOCAL_READ);
            if slot == 0 {
                // The epoch's output is stored, and loaded or cleared
                // exactly as the canonical pass that opens it asks.
                let info = info.store_op(vk::AttachmentStoreOp::STORE);
                epoch.clear.map_or_else(
                    || info.load_op(vk::AttachmentLoadOp::LOAD),
                    |c| {
                        info.load_op(vk::AttachmentLoadOp::CLEAR)
                            .clear_value(vk::ClearValue {
                                color: vk::ClearColorValue { float32: c },
                            })
                    },
                )
            } else {
                // A transient slot opens transparent and is never
                // stored: its contents exist only for this rendering.
                info.load_op(vk::AttachmentLoadOp::CLEAR)
                    .clear_value(vk::ClearValue {
                        color: vk::ClearColorValue {
                            float32: [0.0, 0.0, 0.0, 0.0],
                        },
                    })
                    .store_op(vk::AttachmentStoreOp::DONT_CARE)
            }
        })
        .collect();
    let area = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D {
            width: epoch.extent.0,
            height: epoch.extent.1,
        },
    };
    // SAFETY: `cb` is recording with no rendering open and the
    // attachments are live views of images in the declared layout.
    unsafe {
        mirror.begin_rendering(
            cb,
            &vk::RenderingInfo::default()
                .render_area(area)
                .layer_count(1)
                .color_attachments(&attachments),
        );
    }

    // The state the rendering has already been given. An epoch spans
    // many canonical passes that share a region, a pipeline or a
    // sampled binding, and rebinding what is already bound is the
    // executor's own host cost (#2549).
    let mut viewport: Option<[u32; 4]> = None;
    let mut bound_sampled: Option<vk::DescriptorSet> = None;
    let mut bound_input: Option<vk::DescriptorSet> = None;
    let mut bound_offset: Option<u32> = None;
    let mut remapped: Option<(u8, u8)> = None;
    let mut result = Ok(());
    for draw in draws {
        let mut created = false;
        let pipeline = match mirror.pipeline(draw.key, &mut created) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        note(pipelines, created, draw.pass, draw.key);
        let sampled = match mirror.sampled_set(epoch.stamp, epoch.dummy, draw.image, draw.mask) {
            Ok(set) => set,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        let input = match mirror.input_set(
            epoch.stamp,
            epoch.views[usize::from(draw.key.target)],
            epoch.views[usize::from(draw.key.source)],
        ) {
            Ok(set) => set,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        if remapped != Some((draw.key.target, draw.key.source)) {
            remapped = Some((draw.key.target, draw.key.source));
            // SAFETY: `cb` is recording inside the rendering begun
            // above, whose attachment count is `draw.key.slots`.
            unsafe { mirror.remap(cb, draw.key.slots, draw.key.target, draw.key.source) };
        }
        if viewport != Some(draw.region) {
            viewport = Some(draw.region);
            let [x, y, width, height] = draw.region;
            // wgpu's clip convention needs the Y-flip viewport: every
            // wgpu pass of the frame is issued `y + h / -h` by
            // wgpu-hal, and the epoch draws the same instances.
            // SAFETY: `cb` is recording and both states are dynamic in
            // every tile pipeline.
            unsafe {
                device.cmd_set_viewport(
                    cb,
                    0,
                    &[vk::Viewport {
                        x: x as f32,
                        y: (y + height) as f32,
                        width: width as f32,
                        height: -(height as f32),
                        min_depth: 0.0,
                        max_depth: 1.0,
                    }],
                );
                device.cmd_set_scissor(
                    cb,
                    0,
                    &[vk::Rect2D {
                        offset: vk::Offset2D {
                            x: i32::try_from(x).unwrap_or(i32::MAX),
                            y: i32::try_from(y).unwrap_or(i32::MAX),
                        },
                        extent: vk::Extent2D { width, height },
                    }],
                );
            }
        }
        // SAFETY: `cb` is recording inside the rendering; `pipeline`
        // and the sets were produced by the mirror for this layout.
        unsafe {
            device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, pipeline);
            if bound_offset != Some(draw.dynamic_offset) {
                bound_offset = Some(draw.dynamic_offset);
                device.cmd_bind_descriptor_sets(
                    cb,
                    vk::PipelineBindPoint::GRAPHICS,
                    layout,
                    0,
                    &[epoch.engine_set],
                    &[draw.dynamic_offset],
                );
            }
            if bound_sampled != Some(sampled) {
                bound_sampled = Some(sampled);
                device.cmd_bind_descriptor_sets(
                    cb,
                    vk::PipelineBindPoint::GRAPHICS,
                    layout,
                    1,
                    &[sampled],
                    &[],
                );
            }
            if bound_input != Some(input) {
                bound_input = Some(input);
                device.cmd_bind_descriptor_sets(
                    cb,
                    vk::PipelineBindPoint::GRAPHICS,
                    layout,
                    2,
                    &[input],
                    &[],
                );
            }
            device.cmd_draw(
                cb,
                draw.vertices.end - draw.vertices.start,
                draw.instances.end - draw.instances.start,
                draw.vertices.start,
                draw.instances.start,
            );
        }
        stats.draws += 1;
        stats.pipeline_switches += 1;
    }

    // SAFETY: `cb` is recording inside the rendering begun above.
    unsafe { mirror.end_rendering(cb) };
    // Slot zero goes back to the layout wgpu's tracker believes it is
    // in, so the frame's remaining passes need no knowledge of the
    // epoch. The transient slots are the mirror's own and are left in
    // the local-read layout the next epoch re-establishes from
    // `UNDEFINED`.
    let restore = vk::ImageMemoryBarrier::default()
        .src_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::INPUT_ATTACHMENT_READ,
        )
        .dst_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::COLOR_ATTACHMENT_READ,
        )
        .old_layout(LOCAL_READ)
        .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(epoch.images[0])
        .subresource_range(COLOR);
    // SAFETY: `cb` is recording outside the rendering, and the barrier
    // names the output image live for this call.
    unsafe {
        device.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                | vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            std::slice::from_ref(&restore),
        );
    }
    result
}

/// The one colour mip and layer every attachment carries.
const COLOR: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// Every wgpu texture the epoch's draws sample, so the encoder before
/// the epoch can put each in `SHADER_READ_ONLY_OPTIMAL`.
fn sampled_textures<'a>(
    resources: &'a Resources<'a>,
    surf: &'a SurfaceState,
    epoch: &'a crate::render::composite::plan::TileEpoch,
) -> impl Iterator<Item = &'a wgpu::Texture> {
    std::iter::once(resources.atlas.texture())
        .chain(std::iter::once(resources.dummy_texture))
        .chain(epoch.ops.iter().flat_map(move |op| {
            surf.frame.passes[op.pass]
                .ranges
                .iter()
                .flat_map(move |range| {
                    range
                        .image
                        .as_ref()
                        .map(|image| {
                            crate::render::image_texture(
                                image,
                                resources.images,
                                resources.bitmaps,
                                &surf.shader_textures,
                            )
                        })
                        .into_iter()
                        .chain(
                            range
                                .mask
                                .and_then(|key| resources.atlas.mask_texture_resource(key)),
                        )
                })
        }))
}

/// Records a first-use pipeline creation when the attribution report is
/// collecting; with no report this is one `Option` test (#2547).
fn note(
    pipelines: &mut Option<&mut Vec<report::PipelineCreated>>,
    created: bool,
    pass: usize,
    key: PipeKey,
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
                2 => report::Variant::Source,
                3 => report::Variant::Clear,
                _ => unreachable!("closed tile shader set"),
            },
            replace: key.replace,
            slots: key.slots,
        });
    }
}
