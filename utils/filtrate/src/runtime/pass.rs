//! Compiled pass state: pipelines, binding plans, and the bind-group
//! caches keyed on rotating texture views.

extern crate alloc;

use alloc::vec::Vec;

use super::uniform::FILTER_UNIFORM_WORDS;

/// How many (source, target, original) view combinations each pass caches
/// bind groups for. Swapchains rotate 2-3 backbuffer views, so four slots
/// keep every combination warm across a full swapchain cycle.
pub(super) const DYNAMIC_BIND_GROUP_CACHE_CAPACITY: usize = 4;

pub(super) enum CompiledPassKind {
    Color {
        pipeline: wgpu::RenderPipeline,
        bind_group_layout: wgpu::BindGroupLayout,
    },
    Spatial {
        pipeline: wgpu::ComputePipeline,
        bind_group_layout: wgpu::BindGroupLayout,
        original_input: bool,
        /// Present only on the final spatial pass when the output format
        /// supports storage binding: a second pipeline specialization that
        /// writes the output texture directly, skipping the final blit.
        direct_output: Option<(wgpu::ComputePipeline, wgpu::BindGroupLayout)>,
    },
}

/// Number of scratch ping-pong slots. Two suffice for plain chains; a
/// `spatial_shader_with_original` pass can need a third because its target
/// must alias neither its source nor the retained original.
pub(super) const SCRATCH_SLOT_COUNT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PassTextureSource {
    Input,
    Scratch(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ColorTarget {
    Output,
    Scratch(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PassBindingPlan {
    Color {
        source: PassTextureSource,
        target: ColorTarget,
    },
    Spatial {
        source: PassTextureSource,
        target_scratch: usize,
        /// For `spatial_shader_with_original` passes: the texture that fed
        /// this filter's first stage (the source of the preceding pass, or
        /// the pipeline input when there is no preceding pass).
        original: Option<PassTextureSource>,
    },
}

pub(super) struct CompiledPass {
    pub(super) kind: CompiledPassKind,
    pub(super) param_offset: usize,
    pub(super) param_count: usize,
    pub(super) binding_plan: PassBindingPlan,
    pub(super) uniform_buffer: wgpu::Buffer,
    pub(super) last_uniform_data: Option<[f32; FILTER_UNIFORM_WORDS]>,
    pub(super) cached_bind_group: Option<wgpu::BindGroup>,
    /// Small keyed cache over (source, target, original) view identities.
    /// Sized so a rotating swapchain's 2-3 backbuffer views all stay warm.
    pub(super) dynamic_bind_groups: Vec<CachedDynamicBindGroup>,
}

pub(super) struct CachedDynamicBindGroup {
    pub(super) source_view: wgpu::TextureView,
    pub(super) target_view: Option<wgpu::TextureView>,
    pub(super) original_view: Option<wgpu::TextureView>,
    pub(super) bind_group: wgpu::BindGroup,
}

impl CachedDynamicBindGroup {
    fn matches(
        &self,
        source_view: &wgpu::TextureView,
        target_view: Option<&wgpu::TextureView>,
        original_view: Option<&wgpu::TextureView>,
    ) -> bool {
        self.source_view == *source_view
            && self.target_view.as_ref() == target_view
            && self.original_view.as_ref() == original_view
    }
}

/// Finds a cached bind group matching the given `(source, target, original)`
/// view identities, or creates and caches one. The cache is bounded (oldest
/// entry evicted) so rotating swapchain views cycle through warm entries
/// instead of thrashing.
pub(super) fn find_or_insert_dynamic_bind_group<'c>(
    cache: &'c mut Vec<CachedDynamicBindGroup>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &'static str,
    views: (
        &wgpu::TextureView,
        Option<&wgpu::TextureView>,
        Option<&wgpu::TextureView>,
    ),
    entries: &[wgpu::BindGroupEntry],
) -> &'c wgpu::BindGroup {
    let (source_view, target_view, original_view) = views;
    if let Some(index) = cache
        .iter()
        .position(|cached| cached.matches(source_view, target_view, original_view))
    {
        return &cache[index].bind_group;
    }
    if cache.len() >= DYNAMIC_BIND_GROUP_CACHE_CAPACITY {
        cache.remove(0);
    }
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries,
    });
    cache.push(CachedDynamicBindGroup {
        source_view: source_view.clone(),
        target_view: target_view.cloned(),
        original_view: original_view.cloned(),
        bind_group,
    });
    &cache
        .last()
        .expect("cache cannot be empty immediately after push")
        .bind_group
}

/// Returns the cached static bind group, creating it on first use.
pub(super) fn get_or_create_static_bind_group<'c>(
    slot: &'c mut Option<wgpu::BindGroup>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &'static str,
    entries: &[wgpu::BindGroupEntry],
) -> &'c wgpu::BindGroup {
    slot.get_or_insert_with(|| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries,
        })
    })
}
