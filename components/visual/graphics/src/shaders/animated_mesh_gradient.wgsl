// Animated mesh gradient (GPU renderer)
// Uses uniforms.time, uniforms.resolution, uniforms.speed, uniforms.warp, uniforms.palette

const PI: f32 = 3.14159265359;

// Uniforms (aligned to 16 bytes)
struct Uniforms {
    time: f32,
    speed: f32,
    warp: f32,
    _pad0: f32,
    resolution: vec2<f32>,
    _pad1: vec2<f32>,
    palette: array<vec4<f32>, 16>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );

    let pos = positions[vertex_index];
    var out: VertexOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = (pos + 1.0) * 0.5;
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn grad(p: vec2<f32>) -> vec2<f32> {
    let a = hash21(p) * 2.0 * PI;
    return vec2<f32>(cos(a), sin(a));
}

fn gnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let g00 = grad(i + vec2<f32>(0.0, 0.0));
    let g10 = grad(i + vec2<f32>(1.0, 0.0));
    let g01 = grad(i + vec2<f32>(0.0, 1.0));
    let g11 = grad(i + vec2<f32>(1.0, 1.0));

    let v00 = dot(g00, f - vec2<f32>(0.0, 0.0));
    let v10 = dot(g10, f - vec2<f32>(1.0, 0.0));
    let v01 = dot(g01, f - vec2<f32>(0.0, 1.0));
    let v11 = dot(g11, f - vec2<f32>(1.0, 1.0));

    let x1 = mix(v00, v10, u.x);
    let x2 = mix(v01, v11, u.x);
    let v = mix(x1, x2, u.y);

    return 0.5 + 0.5 * v;
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.55;
    var s = 0.0;
    let m = mat2x2<f32>(1.85, 0.0, 0.0, 1.85);

    for (var i: i32 = 0; i < 4; i = i + 1) {
        s += a * gnoise(p);
        p = m * p + vec2<f32>(0.11, -0.17);
        a *= 0.55;
    }
    return s;
}

fn mesh_color(uv: vec2<f32>) -> vec3<f32> {
    let grid = 4.0;
    let p = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * (grid - 1.0);

    let ix = u32(floor(p.x));
    let y_index = u32(floor(p.y));

    let fx = fract(p.x);
    let fy = fract(p.y);

    let x0 = min(ix, 2u);
    let y0 = min(y_index, 2u);
    let x1 = x0 + 1u;
    let y1 = y0 + 1u;

    let idx00 = y0 * 4u + x0;
    let idx10 = y0 * 4u + x1;
    let idx01 = y1 * 4u + x0;
    let idx11 = y1 * 4u + x1;

    let tx = fx * fx * (3.0 - 2.0 * fx);
    let ty = fy * fy * (3.0 - 2.0 * fy);

    let c00 = uniforms.palette[idx00].rgb;
    let c10 = uniforms.palette[idx10].rgb;
    let c01 = uniforms.palette[idx01].rgb;
    let c11 = uniforms.palette[idx11].rgb;

    let top = mix(c00, c10, tx);
    let bottom = mix(c01, c11, tx);
    return mix(top, bottom, ty);
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let res = max(uniforms.resolution, vec2<f32>(1.0));
    let aspect = res.x / res.y;
    let t = uniforms.time * uniforms.speed;

    var p = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);

    let w1 = fbm(p * 1.8 + vec2<f32>(t * 0.12, -t * 0.10));
    let w2 = fbm(p * 2.1 + vec2<f32>(-t * 0.09, t * 0.11) + vec2<f32>(2.0, -1.6));
    let flow = vec2<f32>(w1 - 0.5, w2 - 0.5);

    let w3 = fbm((p + flow * 0.6) * 3.0 + vec2<f32>(t * 0.18, t * 0.14));
    let w4 = fbm((p - flow * 0.5) * 3.6 + vec2<f32>(-t * 0.15, t * 0.12));
    let flow2 = vec2<f32>(w3 - 0.5, w4 - 0.5);

    let edge = smoothstep(0.0, 0.12, min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)));
    let warped = vec2<f32>(
        uv.x + (flow.x + flow2.x * 0.45) * uniforms.warp * edge,
        uv.y + (flow.y + flow2.y * 0.45) * uniforms.warp * edge,
    );

    var col = mesh_color(warped);

    let vign = smoothstep(1.1, 0.18, length(vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5)));
    col *= 0.96 + 0.04 * vign;

    let px = floor(uv * res);
    let grain = hash21(px + vec2<f32>(t * 6.0, t * 2.0));
    col += (grain - 0.5) * (1.0 / 255.0);

    return vec4<f32>(col, 1.0);
}
