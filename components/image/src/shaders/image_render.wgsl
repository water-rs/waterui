// Simple full-screen quad shader for rendering a texture

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );

    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = uvs[vertex_index];
    return output;
}

@group(0) @binding(0) var image_texture: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image_texture, image_sampler, input.uv);
}

fn tone_map_reinhard(color: vec3<f32>) -> vec3<f32> {
    let safe = max(color, vec3<f32>(0.0));
    return safe / (safe + vec3<f32>(1.0));
}

@fragment
fn fs_main_tonemap(input: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(image_texture, image_sampler, input.uv);
    let mapped = tone_map_reinhard(sample.rgb);
    let alpha = clamp(sample.a, 0.0, 1.0);
    return vec4<f32>(mapped, alpha);
}
