// QR Code GPU Renderer
// Renders QR code directly from matrix texture using fragment shader

struct Uniforms {
    // QR matrix dimension (e.g., 21 for version 1, 25 for version 2, etc.)
    matrix_dim: u32,
    // Output texture size in pixels
    output_size: u32,
    // Padding for alignment
    _padding: vec2<u32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
// QR matrix stored as a texture (R channel: 1.0 = dark, 0.0 = light)
@group(0) @binding(1) var matrix_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen quad using 6 vertices (2 triangles)
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );

    let pos = positions[vertex_index];
    var output: VertexOutput;
    output.position = vec4<f32>(pos, 0.0, 1.0);
    // UV: (0,0) at top-left, (1,1) at bottom-right (standard texture coords)
    output.uv = vec2<f32>((pos.x + 1.0) * 0.5, (1.0 - pos.y) * 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let matrix_dim = uniforms.matrix_dim;

    // Add quiet zone (4 modules on each side as per QR spec)
    let quiet_zone = 4u;
    let total_dim = matrix_dim + quiet_zone * 2u;

    // Scale factor from UV (0-1) to module coordinates
    let module_x = u32(input.uv.x * f32(total_dim));
    let module_y = u32(input.uv.y * f32(total_dim));

    // Check if in quiet zone
    var is_dark = false;
    if module_x >= quiet_zone && module_x < quiet_zone + matrix_dim &&
       module_y >= quiet_zone && module_y < quiet_zone + matrix_dim {
        // Inside the QR code area - load from texture
        let qr_x = module_x - quiet_zone;
        let qr_y = module_y - quiet_zone;
        let texel = textureLoad(matrix_texture, vec2<i32>(i32(qr_x), i32(qr_y)), 0);
        is_dark = texel.r > 0.5;
    }
    // Quiet zone stays white (is_dark = false)

    // Output color: dark = black, light = white
    return select(vec4<f32>(1.0, 1.0, 1.0, 1.0), vec4<f32>(0.0, 0.0, 0.0, 1.0), is_dark);
}
