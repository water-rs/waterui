struct Kernel {
    size: vec2<u32>,
    count: u32,
    mode: u32,
    taps: array<vec4<f32>>,
}
@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> kernel: Kernel;
@group(0) @binding(2) var linear_sampler: sampler;

@vertex fn vs(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));
    return vec4<f32>(p[vertex],0.,1.);
}
fn sample_at(p: vec2<f32>) -> vec4<f32> {
    // Clamp to the logical capture, which can be smaller than the texture.
    // Fade the edge texels to zero over one pixel to reproduce a transparent
    // border without requiring the optional clamp-to-border sampler feature.
    let size = vec2<f32>(kernel.size);
    let edge = clamp(p + vec2<f32>(1.), vec2<f32>(0.), vec2<f32>(1.))
             * clamp(size - p, vec2<f32>(0.), vec2<f32>(1.));
    let uv = (clamp(p, vec2<f32>(0.), size - vec2<f32>(1.)) + vec2<f32>(0.5))
           / vec2<f32>(textureDimensions(input));
    return textureSampleLevel(input, linear_sampler, uv, 0.) * (edge.x * edge.y);
}
@fragment fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    if kernel.mode == 3u {
        return sample_at(position.xy * kernel.taps[0].xy - vec2<f32>(0.5));
    }
    let p = position.xy-vec2<f32>(0.5);
    var result = vec4<f32>(0.);
    if kernel.mode == 2u { result = sample_at(p); }
    for(var i=0u; i<kernel.count; i++) {
        let tap = kernel.taps[i];
        let value = sample_at(p+tap.xy);
        if kernel.mode == 0u { result += value*tap.z; }
        else if kernel.mode == 1u { if value.a > result.a { result=value; } }
        else if value.a < result.a { result=value; }
    }
    return result;
}
