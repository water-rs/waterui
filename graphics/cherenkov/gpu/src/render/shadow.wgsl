struct Kernel {
    size: vec2<u32>,
    count: u32,
    mode: u32,
    taps: array<vec4<f32>>,
}
@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> kernel: Kernel;

@vertex fn vs(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));
    return vec4<f32>(p[vertex],0.,1.);
}
fn texel(p: vec2<i32>) -> vec4<f32> {
    if any(p < vec2<i32>(0)) || any(p >= vec2<i32>(kernel.size)) { return vec4<f32>(0.); }
    return textureLoad(input,p,0);
}
fn sample_at(p: vec2<f32>) -> vec4<f32> {
    let origin = vec2<i32>(floor(p));
    let phase = fract(p);
    return mix(mix(texel(origin),texel(origin+vec2<i32>(1,0)),phase.x),
               mix(texel(origin+vec2<i32>(0,1)),texel(origin+vec2<i32>(1,1)),phase.x),phase.y);
}
@fragment fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
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
