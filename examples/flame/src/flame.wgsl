// Cinematic Campfire - HDR
// UV: (0,0) bottom-left, (1,1) top-right

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y
    );
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var pos = p;
    for (var i = 0; i < 5; i++) {
        v += a * noise(pos);
        pos *= 2.0;
        a *= 0.5;
    }
    return v;
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let time = uniforms.time;
    let res = uniforms.resolution;

    // Centered x, y from 0 (bottom) to 1 (top)
    let aspect = res.x / res.y;
    var x = (uv.x - 0.5) * aspect;
    let y = uv.y;

    // Turbulence that rises upward
    let n1 = fbm(vec2<f32>(x * 4.0, y * 3.0 - time * 1.5)) - 0.5;
    let n2 = fbm(vec2<f32>(x * 8.0 + 50.0, y * 6.0 - time * 2.5)) - 0.5;

    // Displace x position with turbulence (more at top)
    let turb_strength = y * 0.8;
    x += n1 * 0.15 * turb_strength + n2 * 0.08 * turb_strength;

    // Flame width: wide at bottom (y=0), narrow at top (y=1)
    let base_width = 0.35;
    let tip_width = 0.02;
    let width = mix(base_width, tip_width, pow(y, 0.7));

    // Distance from center
    let d = abs(x) / width;

    // Flame intensity
    var flame = 1.0 - smoothstep(0.0, 1.0, d);

    // Fade out at top
    flame *= 1.0 - smoothstep(0.6, 0.95, y);

    // Add detail
    let detail = fbm(vec2<f32>(x * 12.0, y * 8.0 - time * 3.0));
    flame *= 0.7 + 0.3 * detail;

    flame = pow(clamp(flame, 0.0, 1.0), 1.5);

    // Inner bright core
    let core_width = width * 0.35;
    let core_d = abs(x) / core_width;
    var core = 1.0 - smoothstep(0.0, 1.0, core_d);
    core *= 1.0 - smoothstep(0.4, 0.85, y);
    core = pow(clamp(core, 0.0, 1.0), 2.5);

    // === HDR Colors ===
    var color = vec3<f32>(0.0);

    // Outer red glow
    color += vec3<f32>(0.6, 0.08, 0.0) * smoothstep(0.0, 0.2, flame);

    // Orange layer
    color += vec3<f32>(1.5, 0.35, 0.02) * smoothstep(0.15, 0.45, flame);

    // Yellow-orange
    color += vec3<f32>(2.5, 1.0, 0.1) * smoothstep(0.35, 0.65, flame);

    // Hot yellow
    color += vec3<f32>(4.0, 2.5, 0.5) * smoothstep(0.55, 0.85, flame);

    // Bright white-yellow core (HDR!)
    color += vec3<f32>(10.0, 8.0, 4.0) * core;

    // Flicker
    let flicker = 0.9 + 0.1 * sin(time * 15.0) * sin(time * 11.0 + 2.0);
    color *= flicker;

    // Ambient glow
    let glow = exp(-length(vec2<f32>(x, y - 0.3)) * 2.5) * 0.2;
    color += vec3<f32>(1.2, 0.4, 0.1) * glow;

    // Background
    color = mix(vec3<f32>(0.01, 0.005, 0.015), color, clamp(flame + glow, 0.0, 1.0));

    return vec4<f32>(color, 1.0);
}
