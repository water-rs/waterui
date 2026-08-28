// Barcode GPU renderer for packed module matrices.

struct Uniforms {
    matrix_width: u32,
    matrix_height: u32,
    quiet_zone_x: u32,
    quiet_zone_y: u32,
    // Output frame size in pixels.
    output_width: u32,
    output_height: u32,
    // Fill mode for dark modules: 0 = solid, 1 = linear gradient.
    fill_mode: u32,
    preserve_square_modules: u32,
    // Colors are linear RGBA.
    solid_dark_color: vec4<f32>,
    light_color: vec4<f32>,
    gradient_start_color: vec4<f32>,
    gradient_end_color: vec4<f32>,
    // Normalized fill coordinates in QR space.
    gradient_start_point: vec2<f32>,
    gradient_end_point: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
// Packed QR bitset: bit 0 of word 0 = module (0,0), 1 = dark, 0 = light.
@group(0) @binding(1) var<storage, read> matrix_words: array<u32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen quad using 6 vertices (2 triangles)
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

    if uniforms.fill_mode == 0u {
        return uniforms.solid_dark_color;
    }

    // Linear gradient for dark modules.
    let p = vec2<f32>(fx, fy);
    let gradient_dir = uniforms.gradient_end_point - uniforms.gradient_start_point;
    let gradient_len_sq = max(dot(gradient_dir, gradient_dir), 1e-6);
    let t = clamp(dot(p - uniforms.gradient_start_point, gradient_dir) / gradient_len_sq, 0.0, 1.0);
    return mix(uniforms.gradient_start_color, uniforms.gradient_end_color, t);
}
