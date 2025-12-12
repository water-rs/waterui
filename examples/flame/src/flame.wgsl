// Film-level Campfire (HDR)
// Compatible with ShaderSurface prelude:
// - uniforms.time, uniforms.resolution
// - @fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32>
// UV: (0,0) bottom-left, (1,1) top-right

fn hash12(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        hash12(p + 17.0),
        hash12(p + 59.0)
    );
}

fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let a = hash12(i);
    let b = hash12(i + vec2<f32>(1.0, 0.0));
    let c = hash12(i + vec2<f32>(0.0, 1.0));
    let d = hash12(i + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var x = p;
    // 6 octaves for richer “film” detail
    for (var i = 0; i < 6; i = i + 1) {
        v += a * noise2(x);
        x = mat2x2<f32>(1.7, 1.2, -1.2, 1.7) * x + 0.11;
        a *= 0.5;
    }
    return v;
}

// “curl-ish” flow field from fbm gradient
fn flow(p: vec2<f32>) -> vec2<f32> {
    let e = 0.003;
    let n1 = fbm(p + vec2<f32>( e, 0.0));
    let n2 = fbm(p + vec2<f32>(-e, 0.0));
    let n3 = fbm(p + vec2<f32>(0.0,  e));
    let n4 = fbm(p + vec2<f32>(0.0, -e));
    let gx = (n1 - n2) / (2.0 * e);
    let gy = (n3 - n4) / (2.0 * e);
    return normalize(vec2<f32>(gy, -gx) + 1e-5);
}

// blackbody-ish ramp (HDR)
fn fire_color(t: f32) -> vec3<f32> {
    let x = clamp(t, 0.0, 1.0);

    let c0 = vec3<f32>(0.02, 0.015, 0.03); // soot-dark
    let c1 = vec3<f32>(0.70, 0.08, 0.01);  // deep red
    let c2 = vec3<f32>(1.60, 0.35, 0.02);  // orange
    let c3 = vec3<f32>(3.20, 1.20, 0.12);  // yellow-orange (HDR)
    let c4 = vec3<f32>(10.0, 8.0, 4.0);    // white-hot core (HDR)

    let a = smoothstep(0.00, 0.30, x);
    let b = smoothstep(0.18, 0.60, x);
    let c = smoothstep(0.45, 0.90, x);
    let d = smoothstep(0.70, 1.00, x);

    var col = mix(c0, c1, a);
    col = mix(col, c2, b);
    col = mix(col, c3, c);
    col = mix(col, c4, d);
    return col;
}

// Filmic-ish compression (keeps HDR feel but avoids hard clip on SDR targets)
fn compress_hdr(c: vec3<f32>) -> vec3<f32> {
    // smooth rolloff
    return c / (vec3<f32>(1.0) + c * 0.12);
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let time = uniforms.time;
    let res = max(uniforms.resolution, vec2<f32>(1.0));

    let aspect = res.x / res.y;

    // Flame space: x centered with aspect, y bottom->top
    var x = (uv.x - 0.5) * aspect;
    let y = uv.y;

    // --- Heat haze (refraction-like) ---
    // stronger above flame base
    let haze_n = fbm(vec2<f32>(x * 2.2, y * 1.7 - time * 0.7));
    let haze = (haze_n - 0.5) * (0.012 + 0.028 * smoothstep(0.15, 1.0, y));
    x += haze;

    // --- Turbulent flow advection (cinema tearing) ---
    let fp = vec2<f32>(x * 1.6, y * 1.2 - time * 0.35);
    let f = flow(fp);
    x += f.x * (0.06 + 0.14 * y);
    // add subtle lateral “lick” sway
    x += 0.03 * sin(time * 1.3 + y * 7.0);

    // Flame width profile: wider base, narrow tip
    let base_width = 0.36;
    let tip_width  = 0.018;
    let width = mix(base_width, tip_width, pow(y, 0.72));

    // Core width profile
    let core_width = width * 0.33;

    // --- Multi-scale flame structure ---
    // large billow + high freq streaks
    let n_big = fbm(vec2<f32>(x * 3.2, y * 2.6 - time * 1.55)) - 0.5;
    let n_mid = fbm(vec2<f32>(x * 6.8 + 30.0, y * 5.2 - time * 2.45)) - 0.5;
    let n_hi  = fbm(vec2<f32>(x * 13.5 + 80.0, y * 10.0 - time * 4.10));

    // turbulence displacement grows with height
    let turb = y * 0.95;
    x += n_big * 0.12 * turb + n_mid * 0.07 * turb;

    // signed distance-ish from center
    let d_body = abs(x) / (width + 1e-4);
    let d_core = abs(x) / (core_width + 1e-4);

    // base flame mask
    var flame = 1.0 - smoothstep(0.0, 1.0, d_body);
    flame *= 1.0 - smoothstep(0.62, 0.97, y); // fade at top

    // tongue tearing / breakup
    let detail = smoothstep(0.20, 0.95, n_hi);
    flame *= 0.70 + 0.30 * detail;

    // sharpen a bit (more photographic “contrast”)
    flame = pow(clamp(flame, 0.0, 1.0), 1.65);

    // inner core (hot)
    var core = 1.0 - smoothstep(0.0, 1.0, d_core);
    core *= 1.0 - smoothstep(0.42, 0.88, y);
    core = pow(clamp(core, 0.0, 1.0), 2.9);

    // temperature proxy
    let heat = clamp(flame * 1.05 + core * 1.05, 0.0, 1.0);

    // --- Color (HDR) ---
    var color = vec3<f32>(0.0);
    color += fire_color(heat) * (0.08 + 1.25 * flame + 2.20 * core);

    // subtle blue-ish base (fuel-rich look)
    let blue_base = smoothstep(0.55, 0.05, y) * smoothstep(0.15, 0.65, core);
    color += vec3<f32>(0.10, 0.20, 0.55) * blue_base * 0.65;

    // --- Smoke / soot (soft, rising) ---
    let smoke_n = fbm(vec2<f32>((uv.x - 0.5) * aspect * 1.2, y * 1.0 - time * 0.22));
    let smoke = smoothstep(0.55, 0.95, smoke_n) * smoothstep(0.15, 1.0, y) * 0.25;
    color = mix(color, color + vec3<f32>(0.10, 0.12, 0.14), smoke * 0.35);

    // --- Embers (cinematic sparks) ---
    var sparks = vec3<f32>(0.0);
    for (var k = 0; k < 7; k = k + 1) {
        let fk = f32(k);
        // grid in flame space
        let pp = vec2<f32>(
            (uv.x - 0.5) * aspect * 26.0 + fk * 7.3,
            uv.y * 36.0 - time * (3.0 + fk * 0.35)
        );
        let cell = floor(pp);
        let rnd = hash22(cell + fk * 19.7);

        // local particle pos
        let local = fract(pp) - 0.5;
        let dist = length(local);

        // visibility depends on flame presence
        let s = smoothstep(0.11, 0.0, dist)
              * smoothstep(0.05, 0.85, flame)
              * (0.55 + 0.45 * sin(time * 11.0 + rnd.x * 6.283));

        sparks += vec3<f32>(1.2, 0.85, 0.30) * s * 0.45;
    }
    color += sparks;

    // --- Glow (single-pass “bloom-ish”) ---
    // ambient halo around flame body
    let glow = exp(-length(vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.28)) * 2.6) * 0.35;
    color += vec3<f32>(1.3, 0.45, 0.10) * glow;

    // Background
    let bg = vec3<f32>(0.01, 0.005, 0.015);
    let mixv = clamp(flame + glow * 0.75, 0.0, 1.0);
    color = mix(bg, color, mixv);

    // Vignette
    let r = length(vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5));
    let vign = smoothstep(0.95, 0.25, r);
    color *= 0.65 + 0.35 * vign;

    // Film grain
    let grain = (hash12(uv * res + vec2<f32>(time * 60.0, time * 17.0)) - 0.5) * 0.04;
    color += vec3<f32>(grain);

    // HDR rolloff (safe across SDR/HDR targets)
    color = compress_hdr(color);

    return vec4<f32>(color, 1.0);
}
