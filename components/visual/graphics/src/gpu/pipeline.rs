//! Pipeline-assembly helpers shared by every `GpuView` in the workspace.
//!
//! Shaderloom reflects a shader's bind groups as a `Vec`, but almost every
//! `WaterUI` shader declares exactly one. Without these helpers each renderer
//! repeats the same reflect / assert / pop dance before it can build a pipeline
//! layout, and each one spells the failure differently.

use alloc::vec::Vec;

use shaderloom::{CompiledShader, CompiledShaderModule};

/// Reflects the single bind group layout `shader` declares.
///
/// `label` names the shader in the panic message; pass what a reader would need
/// to find the offending WGSL, e.g. `"morph shape shader"`.
///
/// # Panics
///
/// Panics when the shader declares a number of bind groups other than one.
#[must_use]
pub fn single_bind_group_layout(
    shader: &CompiledShader,
    device: &wgpu::Device,
    label: &str,
) -> wgpu::BindGroupLayout {
    let mut layouts: Vec<wgpu::BindGroupLayout> = shader.create_bind_group_layouts(device);
    assert_eq!(
        layouts.len(),
        1,
        "{label} must declare exactly one bind group"
    );
    layouts
        .pop()
        .expect("one bind group layout was asserted just above")
}

/// Creates the vertex and fragment modules for a render pipeline together with
/// the single bind group layout the shader declares.
///
/// This is [`single_bind_group_layout`] paired with
/// [`CompiledShader::create_render_stages`], which is how every single-bind-group
/// render pipeline in the workspace starts.
///
/// # Panics
///
/// Panics when an entry point is missing, or when the shader declares a number
/// of bind groups other than one.
#[must_use]
pub fn single_bind_group_render_stages(
    shader: &CompiledShader,
    device: &wgpu::Device,
    label: &str,
    vertex_entry_point: &str,
    fragment_entry_point: &str,
) -> (
    CompiledShaderModule,
    CompiledShaderModule,
    wgpu::BindGroupLayout,
) {
    let (vertex, fragment) =
        shader.create_render_stages(device, vertex_entry_point, fragment_entry_point);
    let layout = single_bind_group_layout(shader, device, label);
    (vertex, fragment, layout)
}
