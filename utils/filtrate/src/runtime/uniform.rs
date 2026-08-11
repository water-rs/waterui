//! Uniform buffer layout shared by color and spatial passes, plus
//! change-tracked uploads.

use filtrate_core::MAX_FILTER_PARAMS;
use num_traits::ToPrimitive;

/// The uniform block layout shared by color and spatial passes:
/// 8 header words (output/input/original dimensions + padding) followed by
/// the packed parameter array. Color passes only populate the first two
/// header words; the buffer is sized for the larger spatial header so one
/// builder type serves both.
pub(super) const FILTER_UNIFORM_WORDS: usize = 8 + MAX_FILTER_PARAMS;

pub(super) fn create_pass_uniform_buffer(
    device: &wgpu::Device,
    label: &'static str,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (FILTER_UNIFORM_WORDS * core::mem::size_of::<f32>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(super) fn upload_uniform_if_changed(
    queue: &wgpu::Queue,
    uniform_buffer: &wgpu::Buffer,
    last_uniform_data: &mut Option<[f32; FILTER_UNIFORM_WORDS]>,
    uniform_data: &[f32; FILTER_UNIFORM_WORDS],
) {
    let needs_upload = last_uniform_data.as_ref() != Some(uniform_data);
    if needs_upload {
        queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniform_data[..]));
        *last_uniform_data = Some(*uniform_data);
    }
}

pub(super) fn write_uniform_params(
    data: &mut [f32; FILTER_UNIFORM_WORDS],
    offset: usize,
    params: &[f32],
) {
    debug_assert!(
        params.len() <= MAX_FILTER_PARAMS,
        "per-pass parameter slices are bounded by the setup-time budget"
    );
    for (slot, value) in data[offset..].iter_mut().zip(params) {
        *slot = *value;
    }
}

pub(super) fn build_color_uniform_data(
    width: u32,
    height: u32,
    params: &[f32],
) -> [f32; FILTER_UNIFORM_WORDS] {
    let mut data = [0.0f32; FILTER_UNIFORM_WORDS];
    data[0] = u32_to_f32(width);
    data[1] = u32_to_f32(height);
    // The color uniform struct has a 4-word header (dimensions + padding),
    // so its params start at word 4; the buffer keeps the shared sizing and
    // simply leaves the tail unused.
    write_uniform_params(&mut data, 4, params);
    data
}

pub(super) fn build_spatial_uniform_data(
    output_width: u32,
    output_height: u32,
    input_width: u32,
    input_height: u32,
    original_width: u32,
    original_height: u32,
    params: &[f32],
) -> [f32; FILTER_UNIFORM_WORDS] {
    let mut data = [0.0f32; FILTER_UNIFORM_WORDS];
    data[0] = u32_to_f32(output_width);
    data[1] = u32_to_f32(output_height);
    data[2] = u32_to_f32(input_width);
    data[3] = u32_to_f32(input_height);
    data[4] = u32_to_f32(original_width);
    data[5] = u32_to_f32(original_height);
    // The spatial uniform struct has an 8-word header (output/input/original
    // dimensions + padding); params start at word 8.
    write_uniform_params(&mut data, 8, params);
    data
}

pub(super) const fn spatial_source_layout_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(super) const fn spatial_target_layout_entry(
    storage_format: wgpu::TextureFormat,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: storage_format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

pub(super) const fn spatial_uniform_layout_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(super) fn u32_to_f32(value: u32) -> f32 {
    value
        .to_f32()
        .unwrap_or_else(|| panic!("value {value} must fit into f32"))
}
