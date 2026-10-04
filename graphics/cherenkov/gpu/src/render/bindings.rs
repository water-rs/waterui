//! Engine bind group layouts as data, shared between the crate and
//! `build.rs`.
//!
//! The runtime maps these tables into `wgpu::BindGroupLayoutEntry`s; the
//! build script maps them into the resource tables naga needs to emit
//! matching MSL argument slots and SPIR-V descriptor indices. Keeping the
//! layouts in one table means a precompiled shader's slot assignments
//! cannot drift from the layout the device builds at runtime.
//!
//! The assignment rules below restate wgpu-hal 29: every stage counts
//! buffers, textures and samplers independently through the groups in
//! declaration order (`metal/device.rs` `create_pipeline_layout`); a stage
//! that can see a storage buffer reserves a trailing runtime-array-sizes
//! buffer, and the vertex stage always reserves one for vertex pulling.
//! On Vulkan a WGSL `@binding` maps to its entry's ordinal position within
//! the group (`vulkan/device.rs` `create_bind_group_layout`).

/// Vertex stage bit in [`Entry::stages`].
pub const VERTEX: u8 = 1;
/// Fragment stage bit in [`Entry::stages`].
pub const FRAGMENT: u8 = 2;
/// Compute stage bit in [`Entry::stages`].
///
/// Used by `build.rs`; the crate itself has no compute pipelines.
#[allow(dead_code)]
pub const COMPUTE: u8 = 4;

/// Resource kind of one binding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A uniform buffer.
    Uniform,
    /// A read-only storage buffer.
    StorageRead,
    /// A sampled `texture_2d<f32>`.
    Texture,
    /// A sampled `texture_2d<u32>` (external frame planes are code-bearing
    /// uint textures the shader decodes itself).
    TextureUint,
    /// A filtering sampler.
    Sampler,
}

/// One bind group layout entry, backend-agnostic.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The WGSL `@binding` number within its group.
    pub binding: u32,
    /// Stage visibility: `VERTEX | FRAGMENT`.
    pub stages: u8,
    /// The resource kind.
    pub kind: Kind,
    /// `has_dynamic_offset` for buffers; always false for samplers/textures.
    ///
    /// Read by the crate's `layout_entries`, not by `build.rs`.
    #[allow(dead_code)]
    pub dynamic_offset: bool,
    /// `min_binding_size` for buffers; 0 for none.
    ///
    /// Read by the crate's `layout_entries`, not by `build.rs`.
    #[allow(dead_code)]
    pub min_size: u64,
}

impl Entry {
    const fn uniform(binding: u32, stages: u8, dynamic_offset: bool, min_size: u64) -> Self {
        Self {
            binding,
            stages,
            kind: Kind::Uniform,
            dynamic_offset,
            min_size,
        }
    }

    const fn storage_read(binding: u32, stages: u8) -> Self {
        Self {
            binding,
            stages,
            kind: Kind::StorageRead,
            dynamic_offset: false,
            min_size: 0,
        }
    }

    const fn texture(binding: u32) -> Self {
        Self {
            binding,
            stages: FRAGMENT,
            kind: Kind::Texture,
            dynamic_offset: false,
            min_size: 0,
        }
    }

    const fn texture_uint(binding: u32) -> Self {
        Self {
            binding,
            stages: FRAGMENT,
            kind: Kind::TextureUint,
            dynamic_offset: false,
            min_size: 0,
        }
    }

    const fn sampler(binding: u32) -> Self {
        Self {
            binding,
            stages: FRAGMENT,
            kind: Kind::Sampler,
            dynamic_offset: false,
            min_size: 0,
        }
    }
}

/// Group 0 of the engine pipelines (`shader.wgsl`): the per-pass globals
/// window, the instance and gradient-stop buffers, and the glyph atlas.
pub const ENGINE_GROUP0: &[Entry] = &[
    // One 32-byte Globals window; the dynamic offset selects the pass's slot.
    Entry::uniform(0, VERTEX | FRAGMENT, true, 32),
    Entry::storage_read(1, VERTEX | FRAGMENT),
    Entry::storage_read(2, FRAGMENT),
    Entry::texture(3),
];

/// Group 1 of the engine pipelines: the textures a composite or paint
/// samples — composite source, blend backdrop, image paint, clip mask.
pub const ENGINE_GROUP1: &[Entry] = &[
    Entry::texture(0),
    Entry::texture(1),
    Entry::texture(2),
    Entry::texture(3),
];

/// The present pipeline's single group (`present.wgsl`): source texture,
/// nearest sampler, encode/alpha uniforms.
pub const PRESENT_GROUP0: &[Entry] = &[
    Entry::texture(0),
    Entry::sampler(1),
    Entry::uniform(2, FRAGMENT, false, 0),
];

/// `min_size` of the external-frame params uniform
/// (`render::external::Params`, 192 bytes). Duplicated here because
/// `build.rs` shares this table and cannot see the render module.
pub const EXTERNAL_PARAMS_SIZE: u64 = 192;

/// Group 1 of the external-frame pipelines (`external.wgsl`): the frame's
/// planes, the clip mask texture, and the per-frame params.
///
/// YUV frames bind their luma and chroma planes at 0–1 and the f32 dummy at
/// 2; RGB frames bind the uint dummy at 0–1 and their plane at 2.
pub const EXTERNAL_GROUP1: &[Entry] = &[
    Entry::texture_uint(0),
    Entry::texture_uint(1),
    Entry::texture(2),
    Entry::texture(3),
    Entry::uniform(4, FRAGMENT, false, EXTERNAL_PARAMS_SIZE),
];

/// Group 1 of the projective composite pipelines (`projective.wgsl`): the
/// layer's local image with its mips, its filtering sampler, the blend
/// backdrop and the clip mask texture.
pub const PROJECTIVE_GROUP1: &[Entry] = &[
    Entry::texture(0),
    Entry::sampler(1),
    Entry::texture(2),
    Entry::texture(3),
];

/// The mip pipeline's single group (`mip.wgsl`): the previous level.
pub const MIP_GROUP0: &[Entry] = &[Entry::texture(0)];

/// The engine pipelines' two groups, in declaration order.
///
/// Used by `build.rs`; the crate addresses the groups directly.
#[allow(dead_code)]
pub const ENGINE_GROUPS: &[&[Entry]] = &[ENGINE_GROUP0, ENGINE_GROUP1];
/// The present pipeline's group list.
///
/// Used by `build.rs`; the crate addresses the group directly.
#[allow(dead_code)]
pub const PRESENT_GROUPS: &[&[Entry]] = &[PRESENT_GROUP0];

/// The external-frame pipelines' two groups, in declaration order: the
/// shared engine group 0 followed by `EXTERNAL_GROUP1`.
///
/// Used by `build.rs`; the crate addresses the groups directly.
#[allow(dead_code)]
pub const EXTERNAL_GROUPS: &[&[Entry]] = &[ENGINE_GROUP0, EXTERNAL_GROUP1];

/// The projective composite pipelines' two groups, in declaration order:
/// the shared engine group 0 followed by `PROJECTIVE_GROUP1`.
///
/// Used by `build.rs`; the crate addresses the groups directly.
#[allow(dead_code)]
pub const PROJECTIVE_GROUPS: &[&[Entry]] = &[ENGINE_GROUP0, PROJECTIVE_GROUP1];

/// The mip pipeline's group list.
///
/// Used by `build.rs`; the crate addresses the group directly.
#[allow(dead_code)]
pub const MIP_GROUPS: &[&[Entry]] = &[MIP_GROUP0];

/// Group 1 of the Vulkan native external-frame module: `EXTERNAL_GROUP1`
/// plus the designated texture/sampler pair at bindings 5–6 that the
/// `external_native` build lowering merges into the combined sampled
/// image at binding 5 (the `VkSamplerYcbcrConversion` sampler).
///
/// Used by `build.rs` only — the Vulkan descriptor layout is declared in
/// `render::external::vulkan` because an immutable sampler cannot be
/// expressed in this table.
#[allow(dead_code)]
pub const NATIVE_EXTERNAL_GROUP1: &[Entry] = &[
    Entry::texture_uint(0),
    Entry::texture_uint(1),
    Entry::texture(2),
    Entry::texture(3),
    Entry::uniform(4, FRAGMENT, false, EXTERNAL_PARAMS_SIZE),
    Entry::texture(5),
    Entry::sampler(6),
];

/// The Vulkan native external-frame module's groups.
///
/// Used by `build.rs` only.
#[allow(dead_code)]
pub const NATIVE_EXTERNAL_GROUPS: &[&[Entry]] = &[ENGINE_GROUP0, NATIVE_EXTERNAL_GROUP1];

/// One resource's Metal argument slots.
///
/// Used by `build.rs` to emit matching MSL.
#[allow(dead_code)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Target {
    /// `[[buffer(n)]]` slot.
    pub buffer: Option<u8>,
    /// `[[texture(n)]]` slot.
    pub texture: Option<u8>,
    /// `[[sampler(n)]]` slot.
    pub sampler: Option<u8>,
    /// Whether the binding writes its buffer (read-only storage: false).
    pub mutable: bool,
}

/// A stage's complete Metal resource assignment for a pipeline layout.
///
/// Used by `build.rs` to emit matching MSL.
#[allow(dead_code)]
#[derive(Default)]
pub struct StagePlan {
    /// `(group, binding, slots)` in layout order.
    pub targets: Vec<(u32, u32, Target)>,
    /// Slot of the `_mslBufferSizes` argument, when the stage needs or
    /// reserves one.
    pub sizes_buffer: Option<u8>,
}

/// wgpu-hal Metal's slot assignment for `groups` seen by `stage`
/// (`VERTEX`/`FRAGMENT`/`COMPUTE`): each resource kind counts separately
/// through the groups in order; storage buffers force a trailing sizes
/// buffer; the vertex stage always reserves one for vertex pulling.
///
/// Used by `build.rs` to emit matching MSL.
#[allow(dead_code)]
pub fn metal_plan(groups: &[&[Entry]], stage: u8) -> StagePlan {
    let (mut buffers, mut textures, mut samplers) = (0u8, 0u8, 0u8);
    let mut plan = StagePlan::default();
    let mut needs_sizes = false;
    for (group, entries) in groups.iter().enumerate() {
        for entry in *entries {
            if entry.kind == Kind::StorageRead {
                needs_sizes |= entry.stages & stage != 0;
            }
            if entry.stages & stage == 0 {
                continue;
            }
            let mut target = Target::default();
            match entry.kind {
                Kind::Uniform | Kind::StorageRead => {
                    target.buffer = Some(buffers);
                    buffers += 1;
                }
                Kind::Texture | Kind::TextureUint => {
                    target.texture = Some(textures);
                    textures += 1;
                }
                Kind::Sampler => {
                    target.sampler = Some(samplers);
                    samplers += 1;
                }
            }
            plan.targets.push((
                u32::try_from(group).expect("few groups"),
                entry.binding,
                target,
            ));
        }
    }
    if needs_sizes || stage == VERTEX {
        plan.sizes_buffer = Some(buffers);
    }
    plan
}

/// wgpu-hal Vulkan's binding rule: a WGSL `@binding` maps to its entry's
/// ordinal position within the group.
///
/// Used by `build.rs` to emit matching SPIR-V.
#[allow(dead_code)]
pub fn vulkan_slot(group: &[Entry], binding: u32) -> u32 {
    group
        .iter()
        .position(|e| e.binding == binding)
        .unwrap_or_else(|| panic!("no binding {binding} in the layout table"))
        .try_into()
        .expect("layout fits u32")
}
