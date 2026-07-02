// Cinematic flame (HDR) with simple film pipeline:
// 1) Render procedural flame to HDR film buffer
// 2) Threshold + downsample to bloom buffer
// 3) Blur bloom (separable)
// 4) Composite + ACES tonemap + vignette + grain

struct Globals {
    time: f32,
    exposure: f32,
    bloom_threshold: f32,
    bloom_intensity: f32,
    edr_gain: f32,
    bloom_radius: f32,
    wind: f32,
    flame_strength: f32,
    resolution: vec2<f32>,
    inv_resolution: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

// Shared texture/sampler set: film + bloom (bloom may be unused in some passes).
@group(1) @binding(0) var t_film: texture_2d<f32>;
@group(1) @binding(1) var t_bloom: texture_2d<f32>;
@group(1) @binding(2) var s_linear: sampler;

struct BlurParams {
    texel_size: vec2<f32>,
    direction: vec2<f32>,
}

@group(2) @binding(0) var<uniform> blur: BlurParams;
@group(2) @binding(1) var t_source: texture_2d<f32>;
@group(2) @binding(2) var s_source: sampler;

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
    var output: VertexOutput;
    output.position = vec4<f32>(pos, 0.0, 1.0);
    output.uv = (pos + 1.0) * 0.5; // (0,0) bottom-left
    return output;
}

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);

    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));

    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.55;
    var s = 0.0;
    for (var i: i32 = 0; i < 6; i = i + 1) {
        s += a * noise(p);
        p = (rot(0.35) * p) * 2.0 + vec2<f32>(17.0, 23.0);
        a *= 0.5;
    }
    return s;
}

fn ridge(n: f32) -> f32 {
    return 1.0 - abs(2.0 * n - 1.0);
}

fn rfbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.60;
    var s = 0.0;
    for (var i: i32 = 0; i < 5; i = i + 1) {
        s += a * ridge(noise(p));
        p = (rot(0.62) * p) * 2.1 + vec2<f32>(9.2, 7.7);
        a *= 0.5;
    }
    return s;
}

fn fire_palette(x: f32) -> vec3<f32> {
    // x: 0..1 (cool -> hot). Return HDR-ish linear RGB.
    let c0 = vec3<f32>(0.02, 0.005, 0.002);  // ember
    let c1 = vec3<f32>(0.85, 0.10, 0.015);  // red
    let c2 = vec3<f32>(1.75, 0.55, 0.08);   // orange
    let c3 = vec3<f32>(2.60, 1.80, 0.55);   // yellow-hot (less white)

    let t1 = smoothstep(0.00, 0.55, x);
    let t2 = smoothstep(0.55, 0.85, x);
    let t3 = smoothstep(0.85, 1.00, x);

    let a = mix(c0, c1, t1);
    let b = mix(c1, c2, t2);
    let c = mix(c2, c3, t3);
    return mix(mix(a, b, t2), c, t3);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn vignette(uv: vec2<f32>, aspect: f32) -> f32 {
    let q = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
    let r = length(q);
    return smoothstep(0.95, 0.30, r);
}

// Render-target textures in wgpu/WebGPU use a top-left origin when sampled.
// Our `uv` is bottom-left origin, so flip Y for all texture sampling to keep
// multi-pass render->sample pipelines consistent (avoids vertical mirroring).
fn tex_uv(uv: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(uv.x, 1.0 - uv.y);
}

fn sample_film(uv: vec2<f32>) -> vec3<f32> {
    return sample_film4(uv).rgb;
}

fn sample_film4(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_film, s_linear, tex_uv(uv));
}

fn sample_bloom(uv: vec2<f32>) -> vec3<f32> {
    return textureSample(t_bloom, s_linear, tex_uv(uv)).rgb;
}

fn sample_source(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_source, s_source, tex_uv(uv));
}

@fragment
fn fs_flame(input: VertexOutput) -> @location(0) vec4<f32> {
    let t = globals.time;
    let res = max(globals.resolution, vec2<f32>(1.0));
    let aspect = res.x / res.y;

    // Aspect-correct flame space; base anchored at bottom.
    // `uv` is bottom-left origin; keep p.y in 0..1 so the tip never hard-clips.
    var p = vec2<f32>((input.uv.x - 0.5) * aspect, input.uv.y);
    p.x *= 1.15;

    // Scale Y so the flame fades out *before* the top edge.
    let y = max(p.y, 0.0) * 1.25;
    let y01 = clamp(y, 0.0, 1.0);
    let wind = globals.wind;

    // Bend + sway (stronger towards the top), plus a gusty drift.
    let sway = 0.10 * sin(t * 0.90 + y01 * 2.4) + 0.06 * sin(t * 1.70 + y01 * 4.8);
    let gust = (fbm(vec2<f32>(y01 * 1.20, t * 0.25)) - 0.5) * 0.12;
    let center = (sway + gust) * (0.20 + 0.80 * y01) + wind * y01 * y01;

    // Flow field for turbulence (rising motion).
    let rise = t * 2.2;
    let q = vec2<f32>((p.x - center) * 2.4, y * 3.8 - rise);

    let n = fbm(q + vec2<f32>(0.0, t * 0.25));
    let r = rfbm(q * 1.6 + vec2<f32>(2.3, -t * 0.7));
    let tongue_n = rfbm(vec2<f32>((p.x - center) * 4.2, y * 7.5 - t * 4.5));
    var tongues = smoothstep(0.62, 1.10, tongue_n);
    tongues = tongues * tongues;

    // Width profile: wide base -> thin tip.
    let base_w = 0.13;
    let tip_w = 0.0035;
    let w = mix(base_w, tip_w, pow(y01, 1.65));
    let wv = w * (0.72 + 0.40 * n) * (0.92 + 0.35 * tongues);

    // Lateral turbulence (adds tongues and breaks symmetry).
    let x_turb =
        (fbm(q * 2.3 + vec2<f32>(12.0, t * 1.6)) - 0.5) * 0.09 * (0.15 + 0.85 * y01);
    let d = abs((p.x - center) + x_turb);

    // Main body mask with soft edge.
    var mask = 1.0 - smoothstep(wv * 0.85, wv, d);

    // Add flame tongues (mostly in the upper half).
    let tongue_halo = 1.0 - smoothstep(wv * 0.45, wv * 2.4, d);
    mask = clamp(mask + tongues * tongue_halo * (0.05 + 0.35 * y01), 0.0, 1.0);

    // Streaky breakup.
    let breakup = smoothstep(0.15, 0.90, r);
    mask *= mix(0.30, 1.0, breakup);

    // Fade-in at the base (avoid hard clip on the bottom edge).
    mask *= smoothstep(0.00, 0.03, y01);

    // Soft tip fade (prevents the "black bar" truncation).
    let tip_fade = 1.0 - smoothstep(0.95, 1.30, y + (n - 0.5) * 0.12);
    mask *= clamp(tip_fade, 0.0, 1.0);

    // Halo glow around the flame.
    let halo = (1.0 - smoothstep(wv * 1.0, wv * 2.8, d)) * (0.10 + 0.22 * y01);

    // Core is hottest near the centerline.
    let core = exp(-d * d / (wv * wv * 0.06 + 1e-4));

    // Flicker
    let flicker = 0.86 + 0.14 * sin(t * 10.0 + (n + tongues) * 6.28318);
    let body = mask * flicker;

    // Temperature: hot core + hot base, cooler top/edges.
    let heat = clamp(core * 0.90 + (1.0 - y01) * 0.32 + tongues * 0.06, 0.0, 1.0);
    let hot = pow(heat, 2.2);

    // HDR emission (scaled so bloom can do the heavy lifting).
    let strength = globals.flame_strength;
    let outer_col = vec3<f32>(1.60, 0.34, 0.03);
    let inner_col = vec3<f32>(2.80, 1.90, 0.55);
    let mixv = clamp(pow(core, 1.6) * 0.92 + hot * 0.08, 0.0, 1.0);
    let base_col = mix(outer_col, inner_col, mixv);
    let emit = (0.18 + 7.0 * pow(hot, 1.5)) * strength;
    var col = base_col * body * emit;
    col += outer_col * halo * (0.20 + 0.90 * hot) * strength;

    // Slight soot/dimming near edges in the upper flame.
    let soot = smoothstep(0.25, 0.95, y01) * smoothstep(wv * 0.35, wv * 2.2, d);
    col *= 1.0 - 0.45 * soot;

    // Background (subtle warm base).
    var bg = vec3<f32>(0.0015, 0.0018, 0.0025);
    bg += vec3<f32>(0.010, 0.004, 0.002) * exp(-y01 * 4.0);

    // Bloom mask in alpha: keep bloom mostly on the hot core (prevents "smoky" glow).
    let bloom_mask = clamp(pow(core, 1.25) * mask * (0.35 + 0.65 * hot) * 1.25, 0.0, 1.0);
    return vec4<f32>(bg + col, bloom_mask);
}

@fragment
fn fs_downsample(input: VertexOutput) -> @location(0) vec4<f32> {
    // 2x2 box filter + soft threshold
    let uv = input.uv;
    let texel = globals.inv_resolution;

    let f0 = sample_film4(uv + vec2<f32>(-0.5 * texel.x, -0.5 * texel.y));
    let f1 = sample_film4(uv + vec2<f32>( 0.5 * texel.x, -0.5 * texel.y));
    let f2 = sample_film4(uv + vec2<f32>(-0.5 * texel.x,  0.5 * texel.y));
    let f3 = sample_film4(uv + vec2<f32>( 0.5 * texel.x,  0.5 * texel.y));

    let col = (f0.rgb + f1.rgb + f2.rgb + f3.rgb) * 0.25;
    let m = (f0.a + f1.a + f2.a + f3.a) * 0.25;
    let lum = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));

    // Soft-knee bloom extraction (keeps bloom mostly on highlights).
    let thr = globals.bloom_threshold;
    let knee = thr * 0.55;
    let soft = clamp((lum - thr + knee) / (2.0 * knee), 0.0, 1.0);
    let contrib = max(lum - thr, 0.0) + soft * soft * knee;
    let scale = contrib / max(lum, 1e-4);
    let weight = m * m;
    return vec4<f32>(col * scale * weight, 1.0);
}

@fragment
fn fs_blur(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;
    let off = blur.texel_size * blur.direction * globals.bloom_radius;

    // 5-tap Gaussian-ish blur (separable)
    var c = sample_source(uv) * 0.227027;
    c += sample_source(uv + off * 1.384615) * 0.316216;
    c += sample_source(uv - off * 1.384615) * 0.316216;
    c += sample_source(uv + off * 3.230769) * 0.070270;
    c += sample_source(uv - off * 3.230769) * 0.070270;

    return c;
}

@fragment
fn fs_final(input: VertexOutput) -> @location(0) vec4<f32> {
    let t = globals.time;
    let res = max(globals.resolution, vec2<f32>(1.0));
    let aspect = res.x / res.y;

    let film = sample_film(input.uv);
    let bloom = sample_bloom(input.uv);

    var col = film + bloom * globals.bloom_intensity;
    col *= globals.exposure;

    // subtle vignette before tonemap
    col *= 0.55 + 0.45 * vignette(input.uv, aspect);

    // Tonemap to displayable range, then push into extended range on HDR surfaces.
    col = aces(col);
    col *= globals.edr_gain;

    // film grain (tiny, post-tonemap) to avoid banding
    let px = floor(input.uv * res);
    let g = hash21(px + vec2<f32>(t * 60.0, t * 13.0));
    col += (g - 0.5) * (1.0 / 255.0) * 6.0;

    return vec4<f32>(col, 1.0);
}
