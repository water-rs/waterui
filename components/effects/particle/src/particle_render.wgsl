struct InteractionUniforms {
    enabled: u32,
    grid_width: u32,
    grid_height: u32,
    radius: f32,
    strength: f32,
}

struct CollisionUniforms {
    enabled: u32,
    restitution: f32,
    surface_friction: f32,
    circle_obstacle_count: u32,
    bounds: vec4<f32>,
}

struct Uniforms {
    time: f32,
    dt: f32,
    seed: u32,
    max_particles: u32,
    gravity: vec2<f32>,
    wind: vec2<f32>,
    emitter_pos: vec2<f32>,
    emitter_size: vec2<f32>,
    emit_rate: f32,
    turbulence: f32,
    drag: f32,
    stretch_factor: f32,
    softness: f32,
    interaction: InteractionUniforms,
    collision: CollisionUniforms,
    life_range: vec2<f32>,
    speed_range: vec2<f32>,
    angle_range: vec2<f32>,
    size_range: vec2<f32>,
    spin_range: vec2<f32>,
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    shape: u32,
    viewport_width: u32,
    viewport_height: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> uniforms: Uniforms;

const QUAD_VERTICES: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
);

fn pcg_hash(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn mix_f32(a: f32, b: f32, t: f32) -> f32 {
    return a * (1.0 - t) + b * t;
}

fn aspect_correct_offset(offset: vec2<f32>) -> vec2<f32> {
    let vp_width = f32(uniforms.viewport_width);
    let vp_height = f32(uniforms.viewport_height);
    if (vp_width > 0.0 && vp_height > 0.0) {
        let aspect = vp_width / vp_height;
        if (aspect > 1.0) {
            return vec2<f32>(offset.x / aspect, offset.y);
        }
        return vec2<f32>(offset.x, offset.y * aspect);
    }

    return offset;
}

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) vel: vec2<f32>,
    @location(2) life: f32,
    @location(3) max_life: f32,
    @location(4) size: f32,
    @location(5) rotation: f32,
    @location(6) rot_speed: f32,
    @location(7) color: vec4<f32>,
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;
    if (life <= 0.0 || max_life <= 0.0) {
        out.position = vec4<f32>(-10.0, -10.0, 0.0, 1.0);
        return out;
    }

    var quad_pos = QUAD_VERTICES[vertex_index];
    if (uniforms.shape == 1u) {
        let tumble_seed = pcg_hash(instance_index * 12345u);
        let tumble_speed = mix_f32(1.0, 3.0, f32(tumble_seed % 100u) / 100.0);
        let tumble_phase = f32(pcg_hash(instance_index * 54321u) % 628u) / 100.0;
        let tumble = cos(uniforms.time * tumble_speed + tumble_phase);
        quad_pos.x = quad_pos.x * 0.4 * abs(tumble);
    }

    let c = cos(rotation);
    let s = sin(rotation);
    let rotated_pos = vec2<f32>(
        quad_pos.x * c - quad_pos.y * s,
        quad_pos.x * s + quad_pos.y * c,
    );

    var local_offset = rotated_pos * size;
    if (uniforms.stretch_factor > 0.0) {
        let speed = length(vel);
        if (speed > 0.0001) {
            let dir = vel / speed;
            let perp = vec2<f32>(-dir.y, dir.x);
            let stretch_amount = 1.0 + speed * uniforms.stretch_factor * 10.0;
            let width = size;
            let length_value = size * stretch_amount;
            local_offset = (perp * quad_pos.x * width) + (dir * quad_pos.y * length_value);
        }
    }

    let world_pos = pos + aspect_correct_offset(local_offset);
    let clip_pos = world_pos * 2.0 - 1.0;
    out.position = vec4<f32>(clip_pos.x, -clip_pos.y, 0.0, 1.0);
    out.uv = (quad_pos + 1.0) * 0.5;
    let ratio = life / max_life;
    out.color = mix(uniforms.color_end, uniforms.color_start, ratio);
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    var dist = 0.0;

    if (uniforms.shape == 1u) {
        let d = abs(input.uv - center);
        dist = max(d.x, d.y);
    } else {
        dist = distance(input.uv, center);
    }

    let edge = 0.5;
    let smooth_width = max(0.01, uniforms.softness * 0.5);
    let alpha = 1.0 - smoothstep(edge - smooth_width, edge, dist);
    let final_alpha = input.color.a * alpha;
    return vec4<f32>(input.color.rgb * final_alpha, final_alpha);
}
