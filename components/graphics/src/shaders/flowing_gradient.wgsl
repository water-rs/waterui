// Flowing animated gradient (ShaderSurface-compatible)
// Uses uniforms.time and uniforms.resolution

const PI: f32 = 3.14159265359;

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
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
    let m = rot(0.63) * mat2x2<f32>(1.85, 0.0, 0.0, 1.85);

    for (var i: i32 = 0; i < 4; i = i + 1) {
        s += a * gnoise(p);
        p = m * p + vec2<f32>(0.11, -0.17);
        a *= 0.55;
    }
    return s;
}

fn palette(t: f32) -> vec3<f32> {
    let c0 = vec3<f32>(0.05, 0.08, 0.18);
    let c1 = vec3<f32>(0.12, 0.30, 0.52);
    let c2 = vec3<f32>(0.45, 0.28, 0.60);
    let c3 = vec3<f32>(0.95, 0.96, 1.00);

    let t1 = smoothstep(0.0, 0.5, t);
    let t2 = smoothstep(0.35, 0.85, t);
    let t3 = smoothstep(0.7, 1.0, t);

    var col = mix(c0, c1, t1);
    col = mix(col, c2, t2);
    col = mix(col, c3, t3);
    return col;
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let res = max(uniforms.resolution, vec2<f32>(1.0));
    let aspect = res.x / res.y;
    var p = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);

    let t = uniforms.time * 0.08;
    let drift = vec2<f32>(0.08 * sin(t * 1.1), 0.06 * cos(t * 0.9));
    p = rot(0.15 * sin(t * 0.6)) * (p + drift);

    let w1 = fbm(p * 1.6 + vec2<f32>(t * 0.35, -t * 0.28));
    let w2 = fbm(p * 2.2 + vec2<f32>(-t * 0.22, t * 0.31) + vec2<f32>(2.3, -1.7));
    let flow = vec2<f32>(w1 - 0.5, w2 - 0.5);

    let n = fbm(p * 3.1 + flow * 1.7 + vec2<f32>(t * 0.12, t * 0.16));
    let m = fbm(p * 1.9 - flow * 1.1 + vec2<f32>(-t * 0.08, t * 0.10));

    let v = clamp(0.65 * n + 0.35 * m, 0.0, 1.0);
    var col = palette(v);

    let sheen = smoothstep(0.55, 0.9, n);
    col += vec3<f32>(0.18, 0.20, 0.22) * sheen * 0.6;

    let vign = smoothstep(1.15, 0.2, length(p));
    col *= 0.85 + 0.15 * vign;

    return vec4<f32>(col, 1.0);
}
