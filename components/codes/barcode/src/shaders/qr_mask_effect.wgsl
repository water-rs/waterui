// Barcode mask effect shader
// - dark modules: sample input texture
// - light modules + outer area: output configured light color

struct Uniforms {
    matrix_width: u32,
    matrix_height: u32,
    quiet_zone_x: u32,
    quiet_zone_y: u32,
    output_width: u32,
    output_height: u32,
    preserve_square_modules: u32,
    _padding: u32,
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

fn read_dark_module(x: u32, y: u32, matrix_width: u32) -> bool {
    let linear_idx = y * matrix_width + x;
    let word_idx = linear_idx / 32u;
    let bit_idx = linear_idx % 32u;
    let value = matrix_words[word_idx];
    return ((value >> bit_idx) & 1u) == 1u;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let matrix_width = uniforms.matrix_width;
    let matrix_height = uniforms.matrix_height;
    if matrix_width == 0u || matrix_height == 0u ||
        uniforms.output_width == 0u || uniforms.output_height == 0u {
        return uniforms.light_color;
    }

    let width = f32(uniforms.output_width);
    let height = f32(uniforms.output_height);
    let pixel_x = input.uv.x * width;
    let pixel_y = input.uv.y * height;
    let total_width = matrix_width + uniforms.quiet_zone_x * 2u;
    let total_height = matrix_height + uniforms.quiet_zone_y * 2u;

    var fx = clamp(input.uv.x, 0.0, 0.99999994);
    var fy = clamp(input.uv.y, 0.0, 0.99999994);
    if uniforms.preserve_square_modules == 1u {
        let side = min(width, height);
        let offset_x = (width - side) * 0.5;
        let offset_y = (height - side) * 0.5;
        if pixel_x < offset_x || pixel_x >= (offset_x + side) ||
            pixel_y < offset_y || pixel_y >= (offset_y + side) {
            return uniforms.light_color;
        }
        fx = clamp((pixel_x - offset_x) / side, 0.0, 0.99999994);
        fy = clamp((pixel_y - offset_y) / side, 0.0, 0.99999994);
    }

    let module_x = u32(floor(fx * f32(total_width)));
    let module_y = u32(floor(fy * f32(total_height)));

    var is_dark = false;
    if module_x >= uniforms.quiet_zone_x &&
        module_x < uniforms.quiet_zone_x + matrix_width &&
        module_y >= uniforms.quiet_zone_y &&
        module_y < uniforms.quiet_zone_y + matrix_height {
        let barcode_x = module_x - uniforms.quiet_zone_x;
        let barcode_y = module_y - uniforms.quiet_zone_y;
        is_dark = read_dark_module(barcode_x, barcode_y, matrix_width);
    }

    if !is_dark {
        return uniforms.light_color;
    }

    return textureSampleLevel(t_input, s_input, input.uv, 0.0);
}
