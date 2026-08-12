//! Generic GPU runtime for compiling a typed [`Filter`] graph into an [`Effect`].
//!
//! This module owns filter-stage fusion, deterministic parameter animation,
//! HDR intermediate selection, scratch-texture reuse, and command encoding.
//! It has no `WaterUI` dependency and can process images, decoded video frames,
//! or arbitrary wgpu textures in any Rust application.

extern crate alloc;

pub(crate) mod animation;
mod pass;
mod plan;
mod shader;
mod uniform;

#[cfg(test)]
mod tests;

pub use shader::HdrPolicy;

use alloc::{boxed::Box, vec::Vec};
use core::fmt;

use filtrate_core::{Chain, Filter, MAX_FILTER_PARAMS, ParamArray};

#[cfg(test)]
use crate::effect::EffectFrameTiming;
use crate::effect::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderError,
    EffectRenderResult, EffectSetupError, EffectSetupResult,
};

use animation::ParamAnimator;
use pass::{
    ColorTarget, CompiledPass, CompiledPassKind, PassBindingPlan, PassTextureSource,
    SCRATCH_SLOT_COUNT, find_or_insert_dynamic_bind_group, get_or_create_static_bind_group,
};
use plan::{
    FilterSetupPlan, PlannedPass, PlannedPassKind, collect_filter_stages,
    final_direct_output_pass_index, fuse_stages, plan_runtime_bindings,
};
use shader::{
    SPATIAL_WORKGROUP_X, SPATIAL_WORKGROUP_Y, is_filterable_texture_format, is_hdr_texture_format,
    preferred_scratch_format, scratch_texture_usage, specialize_color_shader,
    specialize_spatial_shader,
};
use uniform::{
    build_color_uniform_data, build_spatial_uniform_data, create_pass_uniform_buffer,
    spatial_source_layout_entry, spatial_target_layout_entry, spatial_uniform_layout_entry,
    upload_uniform_if_changed,
};

// ============================================================================
// Filter trait adapter - converts Filter to Effect with animation support
// ============================================================================

/// Adapter that wraps a `Filter` to implement `Effect` with animation support.
///
/// This bridges the pure-data `Filter` trait from filtrate-core to the
/// GPU-aware `Effect` trait used by the rendering system.
///
/// When filter parameters change with animation metadata, this adapter
/// smoothly interpolates values and signals for continued rendering.
///
/// ## Pipeline Selection
///
/// - **Color-only filters** (`F::COLOR_ONLY = true`): Use fragment shaders for native HDR support.
/// - **Spatial filters** (`F::COLOR_ONLY = false`): Use compute shaders with intermediate texture for HDR.
pub struct FilterAdapter<F: Filter> {
    filter: F,
    /// Reactive-parameter driver: watchers, event channel, per-parameter tracks.
    animator: ParamAnimator,
    passes: Vec<CompiledPass>,
    /// Whether render should use scratch ping-pong textures.
    requires_scratch: bool,
    /// HDR/LDR behavior policy for intermediate passes.
    hdr_policy: HdrPolicy,
    /// Scratch texture format for intermediate passes (SDR/HDR).
    scratch_format: wgpu::TextureFormat,
    /// Texture formats the pipeline was compiled against; render validates
    /// each frame's formats against them and fails fast on mismatch.
    setup_formats: Option<(wgpu::TextureFormat, wgpu::TextureFormat)>,
    /// Sticky setup error: once set, render fails fast.
    setup_error: Option<EffectSetupError>,
    // Shared resources
    sampler: Option<wgpu::Sampler>,
    // Scratch ping-pong textures for multi-pass.
    scratch_textures: [Option<wgpu::Texture>; SCRATCH_SLOT_COUNT],
    scratch_views: [Option<wgpu::TextureView>; SCRATCH_SLOT_COUNT],
    scratch_size: (u32, u32),
    // Final blit when last stage is spatial.
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blit_bind_group: Option<wgpu::BindGroup>,
    blit_source_scratch_slot: Option<usize>,
    #[cfg(test)]
    last_render_used_direct_output: bool,
}

impl<F: Filter> fmt::Debug for FilterAdapter<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterAdapter").finish_non_exhaustive()
    }
}

#[allow(private_bounds)]
impl<F: Filter> FilterAdapter<F> {
    /// Create a new filter adapter.
    #[must_use]
    pub fn new(filter: F) -> Self {
        let param_count = <F::Params as ParamArray>::LEN;
        let mut target_params = alloc::vec![0.0; param_count];
        filter.params().write_to(&mut target_params);
        let animator = ParamAnimator::new(target_params, |installer| {
            filter.visit_signals(installer);
        });
        Self {
            filter,
            animator,
            passes: Vec::new(),
            requires_scratch: false,
            hdr_policy: HdrPolicy::default(),
            scratch_format: wgpu::TextureFormat::Rgba8Unorm,
            setup_formats: None,
            setup_error: None,
            sampler: None,
            scratch_textures: [const { None }; SCRATCH_SLOT_COUNT],
            scratch_views: [const { None }; SCRATCH_SLOT_COUNT],
            scratch_size: (0, 0),
            blit_pipeline: None,
            blit_bind_group_layout: None,
            blit_bind_group: None,
            blit_source_scratch_slot: None,
            #[cfg(test)]
            last_render_used_direct_output: false,
        }
    }

    /// Borrows the typed filter graph wrapped by this runtime adapter.
    #[must_use]
    pub const fn filter(&self) -> &F {
        &self.filter
    }

    /// Chain another filter onto this adapter.
    ///
    /// Returns a new `FilterAdapter` wrapping a `Chain` of both filters.
    /// Consecutive color-only filters will be fused into a single GPU pass.
    #[must_use]
    pub fn then<F2: Filter>(self, filter: F2) -> FilterAdapter<Chain<F, F2>> {
        let redraw_callback = self.animator.redraw_callback();
        let mut next = FilterAdapter::new(Chain {
            first: self.filter,
            second: filter,
        });
        next.hdr_policy = self.hdr_policy;
        if let Some(redraw_callback) = redraw_callback {
            next.install_redraw_callback(redraw_callback);
        }
        next
    }

    /// Set HDR behavior policy for this filter chain.
    #[must_use]
    pub const fn hdr_policy(mut self, policy: HdrPolicy) -> Self {
        self.hdr_policy = policy;
        self
    }

    /// Require HDR intermediates; setup fails if unsupported.
    #[must_use]
    pub const fn require_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::RequireHdr)
    }

    /// Prefer HDR intermediates with automatic LDR fallback.
    #[must_use]
    pub const fn prefer_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::PreferHdr)
    }

    /// Force LDR intermediates for maximum compatibility.
    #[must_use]
    pub const fn force_ldr(self) -> Self {
        self.hdr_policy(HdrPolicy::ForceLdr)
    }

    fn install_redraw_callback(&self, callback: EffectRedrawCallback) {
        self.animator.install_redraw_callback(callback);
    }

    fn set_setup_error(&mut self, err: &EffectSetupError) {
        if self.setup_error.is_none() {
            self.setup_error = Some(err.clone());
            tracing::error!("[Filter] setup failed fast: {err}");
        }
    }

    #[cfg(test)]
    const fn has_setup_error(&self) -> bool {
        self.setup_error.is_some()
    }

    #[cfg(test)]
    const fn last_render_used_direct_output(&self) -> bool {
        self.last_render_used_direct_output
    }

    #[cfg(test)]
    const fn allocated_scratch_slots(&self) -> [bool; SCRATCH_SLOT_COUNT] {
        [
            self.scratch_views[0].is_some(),
            self.scratch_views[1].is_some(),
            self.scratch_views[2].is_some(),
        ]
    }

    fn required_scratch_slots_for_frame(
        &self,
        direct_output_pass_index: Option<usize>,
    ) -> [bool; SCRATCH_SLOT_COUNT] {
        let mut required = [false; SCRATCH_SLOT_COUNT];

        for (pass_index, pass) in self.passes.iter().enumerate() {
            match pass.binding_plan {
                PassBindingPlan::Color { source, target } => {
                    if let PassTextureSource::Scratch(slot) = source {
                        required[slot] = true;
                    }
                    if let ColorTarget::Scratch(slot) = target {
                        required[slot] = true;
                    }
                }
                PassBindingPlan::Spatial {
                    source,
                    target_scratch,
                    original,
                } => {
                    if let PassTextureSource::Scratch(slot) = source {
                        required[slot] = true;
                    }
                    if let Some(PassTextureSource::Scratch(slot)) = original {
                        required[slot] = true;
                    }
                    if direct_output_pass_index != Some(pass_index) {
                        required[target_scratch] = true;
                    }
                }
            }
        }

        if direct_output_pass_index.is_none()
            && let Some(blit_slot) = self.blit_source_scratch_slot
        {
            required[blit_slot] = true;
        }

        required
    }

    fn ensure_scratch_textures(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        required_slots: [bool; SCRATCH_SLOT_COUNT],
    ) {
        if !self.requires_scratch {
            return;
        }

        let size_changed = self.scratch_size != (width, height);
        let mut bindings_invalidated = false;

        for (slot, required) in required_slots.iter().copied().enumerate() {
            if !required {
                if self.scratch_textures[slot].is_some() || self.scratch_views[slot].is_some() {
                    self.scratch_textures[slot] = None;
                    self.scratch_views[slot] = None;
                    bindings_invalidated = true;
                }
                continue;
            }

            let missing =
                self.scratch_textures[slot].is_none() || self.scratch_views[slot].is_none();
            if size_changed || missing {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("filter scratch texture"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.scratch_format,
                    usage: scratch_texture_usage(),
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.scratch_textures[slot] = Some(texture);
                self.scratch_views[slot] = Some(view);
                bindings_invalidated = true;
            }
        }

        if bindings_invalidated {
            for pass in &mut self.passes {
                pass.cached_bind_group = None;
                pass.dynamic_bind_groups.clear();
            }
            self.blit_bind_group = None;
        }

        self.scratch_size = if required_slots.iter().any(|required| *required) {
            (width, height)
        } else {
            (0, 0)
        };
    }

    #[expect(
        clippy::too_many_lines,
        clippy::future_not_send,
        reason = "filter pass compilation keeps one ordered device-bound resource graph on the GPU host thread"
    )]
    async fn build_compiled_passes(
        &mut self,
        ctx: &EffectContext<'_>,
        planned: &[PlannedPass],
        scratch_format: wgpu::TextureFormat,
    ) -> Result<(), EffectSetupError> {
        self.passes.clear();
        self.blit_bind_group = None;
        let (binding_plans, blit_source_scratch_slot) = plan_runtime_bindings(planned)?;
        self.blit_source_scratch_slot = blit_source_scratch_slot;
        let final_direct_output_pass = final_direct_output_pass_index(planned, ctx.output_format);

        if self.requires_scratch {
            let error_scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let probe = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("filter scratch format probe"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: scratch_format,
                usage: scratch_texture_usage(),
                view_formats: &[],
            });
            let _ = probe.create_view(&wgpu::TextureViewDescriptor::default());
            if error_scope.pop().await.is_some() {
                return Err(EffectSetupError::ScratchFormatUnsupported {
                    format: scratch_format,
                });
            }
        }

        for (pass_index, (pass, binding_plan)) in planned.iter().zip(binding_plans).enumerate() {
            match &pass.kind {
                PlannedPassKind::Color { fragments } => {
                    let target_format = match binding_plan {
                        PassBindingPlan::Color {
                            target: ColorTarget::Output,
                            ..
                        } => ctx.output_format,
                        PassBindingPlan::Color {
                            target: ColorTarget::Scratch(_),
                            ..
                        } => scratch_format,
                        PassBindingPlan::Spatial { .. } => {
                            return Err(EffectSetupError::PlannerInvariant(
                                "runtime planner produced invalid color binding plan",
                            ));
                        }
                    };
                    let error_scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let (pipeline, bind_group_layout) =
                        Self::create_color_pipeline(ctx, fragments, target_format);
                    if let Some(err) = error_scope.pop().await {
                        let message = alloc::format!("{err}");
                        tracing::error!("[Filter] color pipeline validation error: {message}");
                        return Err(EffectSetupError::PipelineValidation {
                            stage: "color",
                            message,
                        });
                    }
                    self.passes.push(CompiledPass {
                        kind: CompiledPassKind::Color {
                            pipeline,
                            bind_group_layout,
                        },
                        param_offset: pass.param_offset,
                        param_count: pass.param_count,
                        binding_plan,
                        uniform_buffer: create_pass_uniform_buffer(
                            ctx.device,
                            "filter color uniform buffer",
                        ),
                        last_uniform_data: None,
                        cached_bind_group: None,
                        dynamic_bind_groups: Vec::new(),
                    });
                }
                PlannedPassKind::Spatial {
                    shader,
                    original_input,
                } => {
                    if !matches!(binding_plan, PassBindingPlan::Spatial { .. }) {
                        return Err(EffectSetupError::PlannerInvariant(
                            "runtime planner produced invalid spatial binding plan",
                        ));
                    }
                    let error_scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let (pipeline, bind_group_layout) =
                        Self::create_spatial_pipeline(ctx, shader, scratch_format, *original_input)
                            .map_err(EffectSetupError::PlannerInvariant)?;
                    if let Some(err) = error_scope.pop().await {
                        let message = alloc::format!("{err}");
                        tracing::error!("[Filter] spatial pipeline validation error: {message}");
                        return Err(EffectSetupError::PipelineValidation {
                            stage: "spatial",
                            message,
                        });
                    }

                    // The final spatial pass additionally compiles a
                    // direct-output specialization when the output format
                    // supports storage binding, so render can skip the blit.
                    let direct_output = if final_direct_output_pass == Some(pass_index) {
                        let error_scope =
                            ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                        let compiled =
                            Self::create_spatial_pipeline(ctx, shader, ctx.output_format, false);
                        let validation_error = error_scope.pop().await;
                        if let (Ok(pair), None) = (compiled, validation_error) {
                            Some(pair)
                        } else {
                            tracing::debug!(
                                "[Filter] final spatial direct-output path unavailable for output format {:?}",
                                ctx.output_format
                            );
                            None
                        }
                    } else {
                        None
                    };

                    self.passes.push(CompiledPass {
                        kind: CompiledPassKind::Spatial {
                            pipeline,
                            bind_group_layout,
                            original_input: *original_input,
                            direct_output,
                        },
                        param_offset: pass.param_offset,
                        param_count: pass.param_count,
                        binding_plan,
                        uniform_buffer: create_pass_uniform_buffer(
                            ctx.device,
                            "filter spatial uniform buffer",
                        ),
                        last_uniform_data: None,
                        cached_bind_group: None,
                        dynamic_bind_groups: Vec::new(),
                    });
                }
            }
        }

        if self.passes.is_empty() {
            return Err(EffectSetupError::EmptyGraph);
        }

        if self.blit_source_scratch_slot.is_some() {
            let error_scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let (blit_pipeline, blit_bind_group_layout) = Self::create_blit_pipeline(ctx);
            if let Some(err) = error_scope.pop().await {
                let message = alloc::format!("{err}");
                tracing::error!("[Filter] blit pipeline validation error: {message}");
                return Err(EffectSetupError::PipelineValidation {
                    stage: "blit",
                    message,
                });
            }
            self.blit_pipeline = Some(blit_pipeline);
            self.blit_bind_group_layout = Some(blit_bind_group_layout);
        } else {
            self.blit_pipeline = None;
            self.blit_bind_group_layout = None;
            self.blit_bind_group = None;
        }

        Ok(())
    }

    fn plan_setup(&mut self, ctx: &EffectContext<'_>) -> Result<FilterSetupPlan, EffectSetupError> {
        self.animator.ensure_redraw_callback();
        if <F::Params as ParamArray>::LEN > MAX_FILTER_PARAMS {
            return Err(EffectSetupError::TooManyParams {
                declared: <F::Params as ParamArray>::LEN,
                limit: MAX_FILTER_PARAMS,
            });
        }

        // Linear filtering when every sampled format supports it (color
        // passes rescale input -> output through the fullscreen quad, so
        // point sampling would stair-step); nearest only where a float32
        // input format forces NonFiltering. Scratch candidates (Rgba8Unorm /
        // Rgba16Float) are always filterable.
        let filter_mode = if is_filterable_texture_format(ctx.input_format) {
            wgpu::FilterMode::Linear
        } else {
            wgpu::FilterMode::Nearest
        };
        self.sampler = Some(ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter_mode,
            min_filter: filter_mode,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        }));

        let planned = fuse_stages(&collect_filter_stages(&self.filter))?;
        self.requires_scratch = planned.len() > 1
            || matches!(
                planned.last().map(|pass| &pass.kind),
                Some(PlannedPassKind::Spatial { .. })
            );
        let needs_original_input_scratch = planned.iter().any(|pass| {
            matches!(
                pass.kind,
                PlannedPassKind::Spatial {
                    original_input: true,
                    ..
                }
            )
        });
        let scratch_candidates = if self.requires_scratch {
            match self.hdr_policy {
                HdrPolicy::ForceLdr => alloc::vec![wgpu::TextureFormat::Rgba8Unorm],
                HdrPolicy::RequireHdr => alloc::vec![wgpu::TextureFormat::Rgba16Float],
                HdrPolicy::PreferHdr if needs_original_input_scratch => {
                    // Bloom/gloom accumulate unclamped highlight energy, so
                    // HDR scratch is strongly preferred — but PreferHdr
                    // documents an automatic LDR downgrade, so honor it
                    // (the energy clamps, the effect degrades gracefully).
                    alloc::vec![
                        wgpu::TextureFormat::Rgba16Float,
                        wgpu::TextureFormat::Rgba8Unorm,
                    ]
                }
                HdrPolicy::PreferHdr => {
                    let preferred = preferred_scratch_format(ctx.input_format, ctx.output_format);
                    if is_hdr_texture_format(preferred) {
                        alloc::vec![preferred, wgpu::TextureFormat::Rgba8Unorm]
                    } else {
                        alloc::vec![preferred, wgpu::TextureFormat::Rgba16Float]
                    }
                }
            }
        } else {
            alloc::vec![wgpu::TextureFormat::Rgba8Unorm]
        };
        Ok(FilterSetupPlan {
            passes: planned,
            scratch_formats: scratch_candidates,
        })
    }

    fn clear_compiled_passes(&mut self) {
        self.passes.clear();
        self.blit_pipeline = None;
        self.blit_bind_group_layout = None;
        self.blit_bind_group = None;
        self.blit_source_scratch_slot = None;
    }

    #[expect(
        clippy::future_not_send,
        reason = "filter pipeline compilation awaits device validation on the GPU host thread"
    )]
    async fn compile_setup_plan(
        &mut self,
        ctx: &EffectContext<'_>,
        planned: &[PlannedPass],
        scratch_candidates: &[wgpu::TextureFormat],
    ) -> Result<(), EffectSetupError> {
        let mut last_error = None;
        for (index, candidate) in scratch_candidates.iter().copied().enumerate() {
            match self.build_compiled_passes(ctx, planned, candidate).await {
                Ok(()) => {
                    self.scratch_format = candidate;
                    if index > 0 && self.hdr_policy == HdrPolicy::PreferHdr {
                        tracing::warn!(
                            "[Filter] preferred scratch format unavailable, falling back to {candidate:?}"
                        );
                    }
                    return Ok(());
                }
                Err(error) => {
                    last_error = Some(error);
                    self.clear_compiled_passes();
                }
            }
        }

        Err(match (self.hdr_policy, last_error) {
            (HdrPolicy::RequireHdr, Some(error)) => {
                EffectSetupError::HdrRequiredUnavailable(Box::new(error))
            }
            (_, Some(error)) => error,
            (_, None) => EffectSetupError::EmptyGraph,
        })
    }
}

impl<F: Filter> Effect for FilterAdapter<F> {
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        self.install_redraw_callback(callback);
    }

    #[expect(
        clippy::future_not_send,
        reason = "filter setup owns device-bound pipelines and runs on the GPU host thread"
    )]
    async fn setup(&mut self, ctx: &EffectContext<'_>) -> EffectSetupResult {
        let plan = match self.plan_setup(ctx) {
            Ok(plan) => plan,
            Err(error) => {
                self.set_setup_error(&error);
                return Err(error);
            }
        };
        if let Err(error) = self
            .compile_setup_plan(ctx, &plan.passes, &plan.scratch_formats)
            .await
        {
            self.set_setup_error(&error);
            return Err(error);
        }
        self.setup_formats = Some((ctx.input_format, ctx.output_format));
        self.animator.apply_targets_to_current();
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        #[cfg(test)]
        {
            self.last_render_used_direct_output = false;
        }
        if let Some(err) = &self.setup_error {
            return Err(EffectRenderError::SetupFailed(err.clone()));
        }
        if self.passes.is_empty() {
            return Err(EffectRenderError::MissingResource(
                "filter render called before a compiled pass graph exists",
            ));
        }
        let Some((setup_input, setup_output)) = self.setup_formats else {
            return Err(EffectRenderError::MissingResource(
                "filter render called before setup recorded texture formats",
            ));
        };
        if input.format != setup_input || output.format != setup_output {
            return Err(EffectRenderError::FormatMismatch {
                input: input.format,
                output: output.format,
                setup_input,
                setup_output,
            });
        }

        // Direct output is usable this frame only when the output texture
        // actually carries STORAGE_BINDING (swapchain usage can differ from
        // the setup-time assumption).
        let output_supports_storage = output
            .texture
            .usage()
            .contains(wgpu::TextureUsages::STORAGE_BINDING);
        let direct_output_pass_index = if output_supports_storage {
            self.passes.iter().position(|pass| {
                matches!(
                    &pass.kind,
                    CompiledPassKind::Spatial {
                        direct_output: Some(_),
                        ..
                    }
                )
            })
        } else {
            None
        };

        if self.requires_scratch {
            let required_scratch_slots =
                self.required_scratch_slots_for_frame(direct_output_pass_index);
            self.ensure_scratch_textures(
                input.device,
                output.width,
                output.height,
                required_scratch_slots,
            );
            for (slot, required) in required_scratch_slots.into_iter().enumerate() {
                if required && self.scratch_views[slot].is_none() {
                    return Err(EffectRenderError::MissingResource(
                        "required scratch texture view was not allocated",
                    ));
                }
            }
        }

        let needs_redraw = self.animator.update(input.timing.delta());
        let current_values = self.animator.current_values();
        if current_values.is_empty() && <F::Params as ParamArray>::LEN > 0 {
            return Err(EffectRenderError::MissingResource(
                "filter render missing current parameter values",
            ));
        }

        let Some(sampler) = &self.sampler else {
            return Err(EffectRenderError::MissingResource(
                "filter sampler missing after setup",
            ));
        };

        let mut used_direct_spatial_output = false;
        let mut source_width = input.width;
        let mut source_height = input.height;
        for (pass_index, pass) in self.passes.iter_mut().enumerate() {
            let param_start = pass.param_offset;
            let param_end = param_start + pass.param_count;
            let params = &current_values[param_start..param_end];

            match (&pass.kind, pass.binding_plan) {
                (
                    CompiledPassKind::Color {
                        pipeline,
                        bind_group_layout,
                    },
                    PassBindingPlan::Color { source, target },
                ) => {
                    let source_view: &wgpu::TextureView = match source {
                        PassTextureSource::Input => &input.view,
                        PassTextureSource::Scratch(slot) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return Err(EffectRenderError::MissingResource(
                                    "color pass source scratch view missing",
                                ));
                            };
                            view
                        }
                    };
                    let target_view: &wgpu::TextureView = match target {
                        ColorTarget::Output => &output.view,
                        ColorTarget::Scratch(slot) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return Err(EffectRenderError::MissingResource(
                                    "color pass target scratch view missing",
                                ));
                            };
                            view
                        }
                    };

                    let uniform_data =
                        build_color_uniform_data(source_width, source_height, params);
                    upload_uniform_if_changed(
                        input.queue,
                        &pass.uniform_buffer,
                        &mut pass.last_uniform_data,
                        &uniform_data,
                    );

                    let entries = [
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: pass.uniform_buffer.as_entire_binding(),
                        },
                    ];
                    // The pipeline-input view can rotate (hosts recreate it);
                    // the output-target variant of a color pass keys on the
                    // source only, since color bind groups never bind the
                    // target. Scratch-to-scratch passes use the static slot.
                    let bind_group = if matches!(source, PassTextureSource::Input) {
                        find_or_insert_dynamic_bind_group(
                            &mut pass.dynamic_bind_groups,
                            input.device,
                            bind_group_layout,
                            "filter color dynamic bind group",
                            (source_view, None, None),
                            &entries,
                        )
                    } else {
                        get_or_create_static_bind_group(
                            &mut pass.cached_bind_group,
                            input.device,
                            bind_group_layout,
                            "filter color static bind group",
                            &entries,
                        )
                    };

                    {
                        let mut render_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("filter color pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: target_view,
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
                        render_pass.set_pipeline(pipeline);
                        render_pass.set_bind_group(0, bind_group, &[]);
                        render_pass.draw(0..6, 0..1);
                    }

                    if matches!(target, ColorTarget::Scratch(_)) {
                        source_width = output.width;
                        source_height = output.height;
                    }
                }
                (
                    CompiledPassKind::Spatial {
                        pipeline,
                        bind_group_layout,
                        original_input,
                        direct_output,
                    },
                    PassBindingPlan::Spatial {
                        source,
                        target_scratch,
                        original,
                    },
                ) => {
                    let source_view: &wgpu::TextureView = match source {
                        PassTextureSource::Input => &input.view,
                        PassTextureSource::Scratch(slot) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return Err(EffectRenderError::MissingResource(
                                    "spatial pass source scratch view missing",
                                ));
                            };
                            view
                        }
                    };
                    debug_assert_eq!(
                        original.is_some(),
                        *original_input,
                        "binding plan original must mirror the compiled pass layout"
                    );
                    // The original is the texture that fed this filter's
                    // first stage (planned per-pass); scratch originals are
                    // output-sized.
                    let (original_view, original_width, original_height) = match original {
                        Some(PassTextureSource::Input) => {
                            (Some(&input.view), input.width, input.height)
                        }
                        Some(PassTextureSource::Scratch(slot)) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return Err(EffectRenderError::MissingResource(
                                    "spatial pass original scratch view missing",
                                ));
                            };
                            (Some(view), output.width, output.height)
                        }
                        None => (None, 0, 0),
                    };

                    let mut writes_output_directly = false;
                    let (target_view, dispatch_pipeline, dispatch_bind_group_layout): (
                        &wgpu::TextureView,
                        &wgpu::ComputePipeline,
                        &wgpu::BindGroupLayout,
                    ) = if direct_output_pass_index == Some(pass_index)
                        && let Some((direct_pipeline, direct_layout)) = direct_output
                    {
                        writes_output_directly = true;
                        (&output.view, direct_pipeline, direct_layout)
                    } else {
                        let Some(target_view) = self.scratch_views[target_scratch].as_ref() else {
                            return Err(EffectRenderError::MissingResource(
                                "spatial pass target scratch view missing",
                            ));
                        };
                        (target_view, pipeline, bind_group_layout)
                    };
                    let target_width = output.width;
                    let target_height = output.height;

                    let uniform_data = build_spatial_uniform_data(
                        target_width,
                        target_height,
                        source_width,
                        source_height,
                        original_width,
                        original_height,
                        params,
                    );
                    upload_uniform_if_changed(
                        input.queue,
                        &pass.uniform_buffer,
                        &mut pass.last_uniform_data,
                        &uniform_data,
                    );

                    let mut entries = alloc::vec![
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(target_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: pass.uniform_buffer.as_entire_binding(),
                        },
                    ];
                    if let Some(original_view) = original_view {
                        entries.push(wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(original_view),
                        });
                    }
                    let binds_rotating_view = writes_output_directly
                        || matches!(source, PassTextureSource::Input)
                        || matches!(original, Some(PassTextureSource::Input));
                    let bind_group = if binds_rotating_view {
                        find_or_insert_dynamic_bind_group(
                            &mut pass.dynamic_bind_groups,
                            input.device,
                            dispatch_bind_group_layout,
                            "filter spatial dynamic bind group",
                            (source_view, Some(target_view), original_view),
                            &entries,
                        )
                    } else {
                        get_or_create_static_bind_group(
                            &mut pass.cached_bind_group,
                            input.device,
                            dispatch_bind_group_layout,
                            "filter spatial static bind group",
                            &entries,
                        )
                    };

                    {
                        let mut compute_pass =
                            encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("filter spatial pass"),
                                timestamp_writes: None,
                            });
                        compute_pass.set_pipeline(dispatch_pipeline);
                        compute_pass.set_bind_group(0, bind_group, &[]);
                        let workgroups_x = target_width.div_ceil(SPATIAL_WORKGROUP_X);
                        let workgroups_y = target_height.div_ceil(SPATIAL_WORKGROUP_Y);
                        compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
                    }

                    if writes_output_directly {
                        used_direct_spatial_output = true;
                    }

                    source_width = target_width;
                    source_height = target_height;
                }
                _ => {
                    return Err(EffectRenderError::MissingResource(
                        "compiled filter pass kind does not match its runtime binding plan",
                    ));
                }
            }
        }

        if !used_direct_spatial_output && let Some(blit_source_slot) = self.blit_source_scratch_slot
        {
            let Some(blit_pipeline) = &self.blit_pipeline else {
                return Err(EffectRenderError::MissingResource(
                    "final blit pipeline missing after setup",
                ));
            };
            let Some(blit_bind_group_layout) = &self.blit_bind_group_layout else {
                return Err(EffectRenderError::MissingResource(
                    "final blit bind group layout missing after setup",
                ));
            };
            let Some(blit_source_view) = self.scratch_views[blit_source_slot].as_ref() else {
                return Err(EffectRenderError::MissingResource(
                    "final blit source scratch view missing",
                ));
            };

            if self.blit_bind_group.is_none() {
                self.blit_bind_group =
                    Some(input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("filter final blit bind group"),
                        layout: blit_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(blit_source_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                        ],
                    }));
            }
            let Some(blit_bind_group) = self.blit_bind_group.as_ref() else {
                return Err(EffectRenderError::MissingResource(
                    "final blit bind group missing after creation",
                ));
            };

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("filter final blit pass"),
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
                render_pass.set_pipeline(blit_pipeline);
                render_pass.set_bind_group(0, blit_bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        self.animator.mark_rendered();
        #[cfg(test)]
        {
            self.last_render_used_direct_output = used_direct_spatial_output;
        }
        Ok(needs_redraw)
    }

    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        self.filter.output_size(input_width, input_height)
    }

    fn redraw_hint(&self) -> bool {
        self.animator.redraw_hint()
    }
}
#[allow(private_bounds)]
impl<F: Filter> FilterAdapter<F> {
    fn create_color_pipeline(
        ctx: &EffectContext,
        fragments: &str,
        target_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let shader_source = specialize_color_shader(fragments, target_format);
        let shader =
            ctx.shader_cache
                .module(ctx.device, Some("filter color shader"), &shader_source);

        let filterable = is_filterable_texture_format(ctx.input_format);
        let sampler_binding = if filterable {
            wgpu::SamplerBindingType::Filtering
        } else {
            wgpu::SamplerBindingType::NonFiltering
        };
        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter color bind group layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Sampler(sampler_binding),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("filter color pipeline layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("filter color pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
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

        (pipeline, bind_group_layout)
    }

    fn create_spatial_pipeline(
        ctx: &EffectContext,
        shader_source: &str,
        storage_format: wgpu::TextureFormat,
        original_input: bool,
    ) -> Result<(wgpu::ComputePipeline, wgpu::BindGroupLayout), &'static str> {
        let shader_source = specialize_spatial_shader(shader_source, storage_format)?;
        let shader =
            ctx.shader_cache
                .module(ctx.device, Some("filter spatial shader"), &shader_source);

        let original_entry = wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let entries = if original_input {
            alloc::vec![
                spatial_source_layout_entry(),
                spatial_target_layout_entry(storage_format),
                spatial_uniform_layout_entry(),
                original_entry,
            ]
        } else {
            alloc::vec![
                spatial_source_layout_entry(),
                spatial_target_layout_entry(storage_format),
                spatial_uniform_layout_entry(),
            ]
        };

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter spatial bind group layout"),
                    entries: &entries,
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("filter spatial pipeline layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("filter spatial pipeline"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        Ok((pipeline, bind_group_layout))
    }

    fn create_blit_pipeline(ctx: &EffectContext) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let (vertex_shader, fragment_shader) =
            crate::compiled_shaders::BLIT.create_render_stages(ctx.device, "vs_main", "fs_main");

        // The blit samples scratch (always filterable), but shares the
        // adapter-wide sampler, whose binding type follows the input format.
        let filterable = is_filterable_texture_format(ctx.input_format);
        let sampler_binding = if filterable {
            wgpu::SamplerBindingType::Filtering
        } else {
            wgpu::SamplerBindingType::NonFiltering
        };
        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter blit bind group layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(sampler_binding),
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("filter blit pipeline layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("filter blit pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: vertex_shader.module(),
                    entry_point: Some(vertex_shader.entry_point()),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: fragment_shader.module(),
                    entry_point: Some(fragment_shader.entry_point()),
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

        (pipeline, bind_group_layout)
    }
}
