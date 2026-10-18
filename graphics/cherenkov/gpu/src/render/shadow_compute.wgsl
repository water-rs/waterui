struct Kernel {
    size: vec2<u32>,
    count: u32,
    mode: u32,
    taps: array<vec4<f32>>,
}
struct Region {
    origin: vec2<u32>,
    size: vec2<u32>,
    count: u32,
    horizontal: u32,
    padding: vec2<u32>,
}
@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> kernel: Kernel;
// build.rs specializes this storage type through Naga for both target formats.
@group(0) @binding(2) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> region: Region;

// 64 output pixels plus the halo of one 32-tap chunk.
var<workgroup> tile: array<vec4<f32>, 95>;

fn texel(p: vec2<i32>) -> vec4<f32> {
    if any(p < vec2<i32>(0)) || any(p >= vec2<i32>(kernel.size)) {
        return vec4<f32>(0.);
    }
    return textureLoad(input, p, 0);
}

@compute @workgroup_size(64)
fn convolve(@builtin(workgroup_id) group: vec3<u32>,
            @builtin(local_invocation_index) lane: u32) {
    let horizontal = region.horizontal != 0u;
    let axis = select(vec2<i32>(0, 1), vec2<i32>(1, 0), horizontal);
    let origin = select(group.yx * vec2<u32>(1, 64),
                        group.xy * vec2<u32>(64, 1), horizontal);
    let p = origin + vec2<u32>(axis) * lane;
    let end = region.origin + region.size;
    let across = select(origin.x, origin.y, horizontal);
    let across_start = select(region.origin.x, region.origin.y, horizontal);
    let across_end = select(end.x, end.y, horizontal);
    let along = select(origin.y, origin.x, horizontal);
    let along_start = select(region.origin.y, region.origin.x, horizontal);
    let along_end = select(end.y, end.x, horizontal);
    // This condition is uniform across the workgroup. Clear untouched
    // regions in this same dispatch, including a recycled intermediate.
    if across < across_start || across >= across_end
        || along + 64u <= along_start || along >= along_end {
        if all(p < kernel.size) { textureStore(output, p, vec4<f32>(0.)); }
        return;
    }
    let in_region = all(p >= region.origin) && all(p < end) && all(p < kernel.size);
    var result = vec4<f32>(0.);
    for (var first = 0u; first < region.count; first += 32u) {
        let count = min(32u, region.count - first);
        let a = kernel.taps[first].xy;
        let b = kernel.taps[first + count - 1u].xy;
        let offset = i32(select(min(a.y, b.y), min(a.x, b.x), horizontal));
        for (var i = lane; i < 64u + count - 1u; i += 64u) {
            tile[i] = texel(vec2<i32>(origin) + axis * (offset + i32(i)));
        }
        workgroupBarrier();
        if in_region {
            for (var i = 0u; i < count; i++) {
                let tap = kernel.taps[first + i];
                let distance = i32(select(tap.y, tap.x, horizontal)) - offset;
                result += tile[lane + u32(distance)] * tap.z;
            }
        }
        workgroupBarrier();
    }
    if all(p < kernel.size) { textureStore(output, p, result); }
}
