struct QuadUniform {
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> quad: QuadUniform;
@group(0) @binding(1)
var quad_sampler: sampler;
@group(0) @binding(2)
var quad_texture: texture_2d<f32>;
@group(0) @binding(3)
var quad_mask_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) mask_uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let indices = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let corners = array<vec4<f32>, 4>(quad.p0, quad.p1, quad.p2, quad.p3);
    let selected = corners[indices[vertex_index]];
    var out: VertexOutput;
    out.position = vec4<f32>(selected.xy, 0.0, 1.0);
    out.uv = selected.zw;
    out.mask_uv = vec2<f32>((selected.x + 1.0) * 0.5, (1.0 - selected.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(quad_texture, quad_sampler, in.uv);
    let mask = textureSample(quad_mask_texture, quad_sampler, in.mask_uv);
    return vec4<f32>(color.rgb, color.a * mask.a);
}
