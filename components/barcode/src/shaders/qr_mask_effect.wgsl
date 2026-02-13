// QR mask effect shader
// - dark modules: sample input texture
// - light modules + outer area: output configured light color

struct Uniforms {
    matrix_dim: u32,
    quiet_zone: u32,
    output_width: u32,
    output_height: u32,
    light_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> matrix_words: array<u32>;
@group(0) @binding(2) var t_input: texture_2d<f32>;
@group(0) @binding(3) var s_input: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );

    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = uvs[vertex_index];
    return output;
}

fn read_dark_module(x: u32, y: u32, matrix_dim: u32) -> bool {
    let linear_idx = y * matrix_dim + x;
    let word_idx = linear_idx / 32u;
    let bit_idx = linear_idx % 32u;
    let value = matrix_words[word_idx];
    return ((value >> bit_idx) & 1u) == 1u;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let matrix_dim = uniforms.matrix_dim;
    if matrix_dim == 0u || uniforms.output_width == 0u || uniforms.output_height == 0u {
        return uniforms.light_color;
    }

    let quiet_zone = uniforms.quiet_zone;
    let total_dim = matrix_dim + quiet_zone * 2u;
    if total_dim == 0u {
        return uniforms.light_color;
    }

    let width = f32(uniforms.output_width);
    let height = f32(uniforms.output_height);
    let side = min(width, height);
    let offset_x = (width - side) * 0.5;
    let offset_y = (height - side) * 0.5;

    let pixel_x = input.uv.x * width;
    let pixel_y = input.uv.y * height;

    if pixel_x < offset_x || pixel_x >= (offset_x + side) ||
        pixel_y < offset_y || pixel_y >= (offset_y + side) {
        return uniforms.light_color;
    }

    let local_x = pixel_x - offset_x;
    let local_y = pixel_y - offset_y;

    let fx = clamp(local_x / side, 0.0, 0.99999994);
    let fy = clamp(local_y / side, 0.0, 0.99999994);
    let module_x = u32(floor(fx * f32(total_dim)));
    let module_y = u32(floor(fy * f32(total_dim)));

    var is_dark = false;
    if module_x >= quiet_zone && module_x < quiet_zone + matrix_dim &&
        module_y >= quiet_zone && module_y < quiet_zone + matrix_dim {
        let qr_x = module_x - quiet_zone;
        let qr_y = module_y - quiet_zone;
        is_dark = read_dark_module(qr_x, qr_y, matrix_dim);
    }

    if !is_dark {
        return uniforms.light_color;
    }

    return textureSampleLevel(t_input, s_input, input.uv, 0.0);
}

