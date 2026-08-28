@group(0) @binding(0)
var source_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

struct DestinationRect {
    origin_size: vec4<f32>,
}

@group(0) @binding(2)
var<uniform> destination_rect: DestinationRect;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let uvs = array(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );
    var output: VertexOutput;
    let uv = uvs[vertex_index];
    let normalized = destination_rect.origin_size.xy + uv * destination_rect.origin_size.zw;
    output.position = vec4<f32>(
        normalized.x * 2.0 - 1.0,
        1.0 - normalized.y * 2.0,
        0.0,
        1.0,
    );
    output.uv = uv;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source_texture, source_sampler, input.uv);
}
