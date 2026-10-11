// LUT colour grade: trilinear lookup of the input colour, clamped to
// [0, 1], in a 3D LUT packed as a 2D strip (`aux0`: `lut_size` slices of
// `lut_size` x `lut_size`, blue selecting the slice), mixed in by
// `intensity`.

struct Params {
    intensity: f32,
    lut_size: f32,
}

fn lut_texel(lut: texture_2d<f32>, r: u32, g: u32, b: u32, lut_size: u32) -> vec3<f32> {
    let dims = textureDimensions(lut, 0);
    let x = min(b * lut_size + r, dims.x - 1u);
    let y = min(g, dims.y - 1u);
    return textureLoad(lut, vec2<i32>(i32(x), i32(y)), 0).rgb;
}

fn sample_lut(lut: texture_2d<f32>, color: vec3<f32>, lut_size: u32) -> vec3<f32> {
    let grid = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)) * (f32(lut_size) - 1.0);

    let r0 = u32(floor(grid.r));
    let g0 = u32(floor(grid.g));
    let b0 = u32(floor(grid.b));
    let r1 = min(r0 + 1u, lut_size - 1u);
    let g1 = min(g0 + 1u, lut_size - 1u);
    let b1 = min(b0 + 1u, lut_size - 1u);
    let f = grid - vec3<f32>(f32(r0), f32(g0), f32(b0));

    let c00 = mix(lut_texel(lut, r0, g0, b0, lut_size), lut_texel(lut, r1, g0, b0, lut_size), f.r);
    let c10 = mix(lut_texel(lut, r0, g1, b0, lut_size), lut_texel(lut, r1, g1, b0, lut_size), f.r);
    let c01 = mix(lut_texel(lut, r0, g0, b1, lut_size), lut_texel(lut, r1, g0, b1, lut_size), f.r);
    let c11 = mix(lut_texel(lut, r0, g1, b1, lut_size), lut_texel(lut, r1, g1, b1, lut_size), f.r);
    return mix(mix(c00, c10, f.g), mix(c01, c11, f.g), f.b);
}

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let lut_size = max(u32(round(params.lut_size)), 2u);
    let intensity = clamp(params.intensity, 0.0, 1.0);
    let graded = sample_lut(aux0, base.rgb, lut_size);
    return vec4<f32>(mix(base.rgb, graded, intensity), base.a);
}
