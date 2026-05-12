// Art Starfield (single-pass, ShaderSurface-compatible)
// Uses: uniforms.time, uniforms.resolution

const PI: f32 = 3.14159265359;

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn hash21(p: vec2<f32>) -> f32 {
    // stable-ish hash
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn hash23(p: vec2<f32>) -> vec3<f32> {
    let n = hash21(p);
    return vec3<f32>(
        n,
        hash21(p + 19.19),
        hash21(p + 73.73)
    );
}

// Perlin-ish gradient noise (0..1)
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
    let v  = mix(x1, x2, u.y);

    return 0.5 + 0.5 * v; // -> 0..1
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.55;
    var s = 0.0;
    let m = rot(0.63) * mat2x2<f32>(1.85, 0.0, 0.0, 1.85);

    for (var i: i32 = 0; i < 6; i = i + 1) {
        s += a * gnoise(p);
        p = m * p + vec2<f32>(0.11, -0.17);
        a *= 0.52;
    }
    return s;
}

// Layered stars in grid space with neighbor sampling to avoid cell seams
fn star_layer(p: vec2<f32>, t: f32, scale: f32, density: f32, size: f32, boost: f32) -> vec3<f32> {
    let gp = p * scale;
    let cell = floor(gp);
    var col = vec3<f32>(0.0);

    for (var j: i32 = -1; j <= 1; j = j + 1) {
        for (var i: i32 = -1; i <= 1; i = i + 1) {
            let c = cell + vec2<f32>(f32(i), f32(j));
            let r = hash23(c);

            // keep some cells as stars
            if (r.z < density) {
                // star position inside cell
                let sp = (c + r.xy) - gp;
                let d = length(sp);

                // core + halo (Gaussian-ish)
                let core = exp(-d * d / (size * size));
                let halo = exp(-d * d / ((size * 4.0) * (size * 4.0)));

                // twinkle per star
                let tw = 0.70 + 0.30 * sin(t * (2.0 + r.x * 8.0) + r.y * 6.28318);

                // temperature tint
                let cool = vec3<f32>(0.75, 0.88, 1.12);
                let warm = vec3<f32>(1.10, 0.95, 0.80);
                let tint = mix(cool, warm, r.y);

                // bright stars: add soft diffraction spikes
                let bright = smoothstep(density * 0.6, 0.0, r.z); // rarer -> brighter
                let ax = abs(sp.x);
                let ay = abs(sp.y);
                let spike = exp(-ax * 8.0) + exp(-ay * 8.0);

                col += tint * (core * (1.2 + 2.5 * bright) + halo * 0.35 + spike * 0.10 * bright) * tw * boost;
            }
        }
    }

    return col;
}

// ACES-ish tonemap
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
    return smoothstep(1.05, 0.25, r);
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let t = uniforms.time;
    let res = max(uniforms.resolution, vec2<f32>(1.0));
    let aspect = res.x / res.y;

    // aspect-correct space, centered
    var p = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);

    // subtle camera drift + very slow rotation (cinematic)
    let drift = vec2<f32>(0.02 * sin(t * 0.05), 0.015 * cos(t * 0.04));
    p = rot(0.04 * sin(t * 0.02)) * (p + drift);

    // --- background gradient (deep navy -> violet) ---
    let grad = smoothstep(-0.55, 0.55, p.y);
    var col = mix(vec3<f32>(0.004, 0.006, 0.012), vec3<f32>(0.020, 0.012, 0.030), grad);

    // --- milky way band + domain-warp nebula ---
    let band_dir = normalize(vec2<f32>(0.33, 0.94));
    let band = dot(p, band_dir);
    let band_mask = exp(-abs(band) * 2.6);

    // domain warp for wispy structure
    let w1 = fbm(p * 1.8 + vec2<f32>(0.0,  t * 0.02));
    let w2 = fbm(p * 1.8 + vec2<f32>(11.7, -t * 0.018));
    let warp = vec2<f32>(w1 - 0.5, w2 - 0.5);

    let n  = fbm(p * 2.4 + warp * 1.6);
    let n2 = fbm(p * 5.0 + warp * 2.4 + vec2<f32>(2.0, 0.0));

    let neb = pow(clamp(n, 0.0, 1.0), 1.7) * band_mask;
    let filaments = pow(clamp(n2, 0.0, 1.0), 3.2) * band_mask;

    // nebula palette (blue/purple with slight warm highlights)
    let neb_cool = vec3<f32>(0.09, 0.13, 0.30);
    let neb_purp = vec3<f32>(0.20, 0.10, 0.32);
    let neb_warm = vec3<f32>(0.30, 0.22, 0.18);

    col += (neb_cool * 1.2 + neb_purp * 0.9) * neb * 1.6;
    col += neb_purp * filaments * 1.2;
    col += neb_warm * filaments * 0.25; // subtle warm dust

    // dust specks (sparkly, inside band)
    let dust = smoothstep(0.82, 0.98, fbm(p * 14.0 + warp * 3.0 + vec2<f32>(t * 0.03, 0.0)));
    col += vec3<f32>(0.18, 0.16, 0.20) * dust * band_mask * 0.65;

    // --- stars: 3 layers (depth) ---
    // Use p-space scaling (not res-based) to reduce “screen-size dependent” weirdness
    col += star_layer(p, t, 220.0, 0.040, 0.020, 0.9);  // tiny many
    col += star_layer(p, t * 0.85, 120.0, 0.028, 0.030, 1.2); // medium
    col += star_layer(p, t * 0.60,  55.0, 0.020, 0.040, 1.9); // big few

    // --- finishing ---
    col *= 0.55 + 0.45 * vignette(uv, aspect);

    // tiny grain/dither to avoid banding
    let px = floor(uv * res);
    let g = hash21(px + vec2<f32>(t * 60.0, t * 13.0));
    col += (g - 0.5) * (1.0 / 255.0) * 2.0;

    // tone map
    col = aces(col * 1.15);

    return vec4<f32>(col, 1.0);
}
