struct TileUniforms {
    target_size: vec2<f32>,
    target_origin: vec2<f32>,
    target_extent: vec2<f32>,
    uv_origin: vec2<f32>,
    uv_extent: vec2<f32>,
    padding: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );
    let corner = corners[vertex_index];
    let target_pixel = tile.target_origin + corner * tile.target_extent;
    let clip = vec2<f32>(
        target_pixel.x / tile.target_size.x * 2.0 - 1.0,
        1.0 - target_pixel.y / tile.target_size.y * 2.0,
    );

    var output: VertexOutput;
    output.position = vec4<f32>(clip, 0.0, 1.0);
    output.uv = tile.uv_origin + corner * tile.uv_extent;
    return output;
}

@group(0) @binding(0) var<uniform> tile: TileUniforms;
@group(0) @binding(1) var map_sampler: sampler;
@group(0) @binding(2) var map_texture: texture_2d<f32>;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSampleLevel(map_texture, map_sampler, input.uv, 0.0);
}
