//! Shader source specialization: token substitution, preambles, and
//! format capability helpers shared by the runtime's pipeline builders.

use filtrate_core::MAX_FILTER_PARAM_VEC4S;

extern crate alloc;

pub(super) const SPATIAL_OUTPUT_FORMAT_TOKEN: &str = "OUTPUT_STORAGE_FORMAT";
/// Token in both shader preambles for the parameter array row count, so the
/// WGSL declaration can never drift from `filtrate_core::MAX_FILTER_PARAMS`.
pub(super) const PARAM_VEC4S_TOKEN: &str = "PARAM_VEC4S";
/// Token in the color preamble for the display-range clamp bound
/// (`const COLOR_CLAMP_MAX`) used by LDR-styled fragments (photo-effect
/// presets). Substituted with 1.0 for LDR targets and the f16 maximum for
/// HDR targets so those fragments never crush an HDR chain's highlights.
pub(super) const COLOR_CLAMP_MAX_TOKEN: &str = "CLAMP_MAX_BOUND";
pub(super) const F16_MAX_WGSL: &str = "65504.0";

/// Workgroup shape shared by every spatial compute shader. The WGSL sources
/// reference the `WORKGROUP_X` / `WORKGROUP_Y` tokens, which
/// [`specialize_spatial_shader`] substitutes with these values so shader
/// declarations and `dispatch_workgroups` can never drift apart.
/// 256 threads benchmarks at the memory-bandwidth floor on tiler GPUs.
pub(super) const SPATIAL_WORKGROUP_X: u32 = 16;
pub(super) const SPATIAL_WORKGROUP_Y: u32 = 16;
pub(super) const SPATIAL_WORKGROUP_X_TOKEN: &str = "WORKGROUP_X";
pub(super) const SPATIAL_WORKGROUP_Y_TOKEN: &str = "WORKGROUP_Y";

/// Policy controlling whether intermediate (scratch) textures use HDR
/// formats. Chosen per adapter via [`super::FilterAdapter::require_hdr`] /
/// [`super::FilterAdapter::force_ldr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HdrPolicy {
    /// Require HDR-capable intermediate pipeline; fail setup if unavailable.
    RequireHdr,
    /// Prefer HDR intermediates and automatically downgrade to LDR when unsupported.
    #[default]
    PreferHdr,
    /// Force LDR intermediates even on HDR-capable devices.
    ForceLdr,
}

pub(super) const fn is_hdr_texture_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    )
}

pub(super) const fn preferred_scratch_format(
    input_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    if is_hdr_texture_format(input_format) || is_hdr_texture_format(output_format) {
        wgpu::TextureFormat::Rgba16Float
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

pub(super) fn scratch_texture_usage() -> wgpu::TextureUsages {
    wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::STORAGE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT
}

pub(super) const fn storage_format_to_wgsl(
    format: wgpu::TextureFormat,
) -> Result<&'static str, &'static str> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Ok("rgba8unorm"),
        wgpu::TextureFormat::Rgba16Float => Ok("rgba16float"),
        wgpu::TextureFormat::Rgba32Float => Ok("rgba32float"),
        _ => Err("unsupported storage texture format for spatial filter"),
    }
}

/// Whether `format` supports hardware linear filtering without extra device
/// features (float32 filtering is feature-gated in WebGPU; float16 and unorm
/// formats are filterable in core).
pub(super) const fn is_filterable_texture_format(format: wgpu::TextureFormat) -> bool {
    !matches!(format, wgpu::TextureFormat::Rgba32Float)
}

pub(super) const SPATIAL_PREAMBLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/shaders/shared/spatial_preamble.wgsl"
));

pub(super) fn specialize_spatial_shader(
    shader_source: &str,
    storage_format: wgpu::TextureFormat,
) -> Result<alloc::string::String, &'static str> {
    let storage_ty = storage_format_to_wgsl(storage_format)?;
    let mut combined =
        alloc::string::String::with_capacity(SPATIAL_PREAMBLE.len() + shader_source.len() + 1);
    combined.push_str(SPATIAL_PREAMBLE);
    combined.push('\n');
    combined.push_str(shader_source);
    Ok(combined
        .replace(SPATIAL_OUTPUT_FORMAT_TOKEN, storage_ty)
        .replace(SPATIAL_WORKGROUP_X_TOKEN, &SPATIAL_WORKGROUP_X.to_string())
        .replace(SPATIAL_WORKGROUP_Y_TOKEN, &SPATIAL_WORKGROUP_Y.to_string())
        .replace(PARAM_VEC4S_TOKEN, &MAX_FILTER_PARAM_VEC4S.to_string()))
}

/// Assembles the fused color shader (preamble + fragments + postamble) and
/// substitutes the token contract for the pass's target format.
pub(super) fn specialize_color_shader(
    fragments: &str,
    target_format: wgpu::TextureFormat,
) -> alloc::string::String {
    let preamble = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/shared/fragment_preamble.wgsl"
    ));
    let postamble = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/shared/fragment_postamble.wgsl"
    ));
    let mut shader_source =
        alloc::string::String::with_capacity(preamble.len() + fragments.len() + postamble.len());
    shader_source.push_str(preamble);
    shader_source.push_str(fragments);
    shader_source.push_str(postamble);
    let clamp_max = if is_hdr_texture_format(target_format) {
        F16_MAX_WGSL
    } else {
        "1.0"
    };
    shader_source
        .replace(PARAM_VEC4S_TOKEN, &MAX_FILTER_PARAM_VEC4S.to_string())
        .replace(COLOR_CLAMP_MAX_TOKEN, clamp_max)
}
