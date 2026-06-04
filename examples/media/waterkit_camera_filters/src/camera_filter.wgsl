struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(2.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var out: VertexOut;
    out.position = vec4<f32>(positions[index], 0.0, 1.0);
    out.uv = uvs[index];
    return out;
}

struct FilterUniforms {
    tone: vec4<f32>,
    extras: vec4<f32>,
};

@group(0) @binding(0)
var input_tex: texture_2d<f32>;

@group(0) @binding(1)
var input_sampler: sampler;

@group(0) @binding(2)
var<uniform> filters: FilterUniforms;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sampled = textureSample(input_tex, input_sampler, in.uv);

    var rgb = sampled.rgb;
    let luma = dot(rgb, vec3<f32>(0.299, 0.587, 0.114));

    rgb = mix(vec3<f32>(luma), rgb, filters.tone.y);
    rgb = (rgb - vec3<f32>(0.5)) * filters.tone.z + vec3<f32>(0.5);
    rgb = rgb + vec3<f32>(filters.tone.x);
    rgb = rgb * vec3<f32>(
        1.0 + filters.tone.w * 0.12,
        1.0,
        1.0 - filters.tone.w * 0.12,
    );

    let dist = distance(in.uv, vec2<f32>(0.5, 0.5));
    let vignette = 1.0 - smoothstep(0.35, 0.75, dist) * filters.extras.x;
    rgb = rgb * vignette;

    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), sampled.a);
}
