struct Particle {
    pos: vec2<f32>,
    vel: vec2<f32>,
    life: f32,
    max_life: f32,
    size: f32,
    rotation: f32,
    rot_speed: f32,
    color: vec4<f32>,
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

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;

const TAU: f32 = 6.283185307179586;

fn pcg_hash(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg_hash(*seed);
    return f32(*seed) / 4294967295.0;
}

fn mix_f32(a: f32, b: f32, t: f32) -> f32 {
    return a * (1.0 - t) + b * t;
}

fn sample_emitter_offset(seed: ptr<function, u32>) -> vec2<f32> {
    if (uniforms.emitter_size.x == 0.0 && uniforms.emitter_size.y == 0.0) {
        return vec2<f32>(0.0, 0.0);
    }

    if (uniforms.emitter_size.y >= 0.0) {
        return vec2<f32>(
            (rand(seed) - 0.5) * uniforms.emitter_size.x,
            (rand(seed) - 0.5) * uniforms.emitter_size.y,
        );
    }

    let angle = rand(seed) * TAU;
    let radius = sqrt(rand(seed)) * uniforms.emitter_size.x;
    return vec2<f32>(cos(angle), sin(angle)) * radius;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= uniforms.max_particles) {
        return;
    }

    var p = particles[index];
    var seed = uniforms.seed + index;

    if (p.life > 0.0) {
        let drag_factor = pow(uniforms.drag, uniforms.dt * 60.0);
        p.vel = p.vel * drag_factor;
        p.vel += (uniforms.gravity + uniforms.wind) * uniforms.dt;

        if (uniforms.turbulence > 0.0) {
            let jitter = (rand(&seed) - 0.5) * uniforms.turbulence * uniforms.dt;
            p.vel.x += jitter;
        }

        p.pos += p.vel * uniforms.dt;
        p.rotation += p.rot_speed * uniforms.dt;
        p.life -= uniforms.dt;
    } else {
        let spawn_chance = uniforms.emit_rate * uniforms.dt / f32(uniforms.max_particles);

        if (rand(&seed) < spawn_chance) {
            p.pos = uniforms.emitter_pos + sample_emitter_offset(&seed);

            let speed = mix_f32(uniforms.speed_range.x, uniforms.speed_range.y, rand(&seed));
            let angle = mix_f32(uniforms.angle_range.x, uniforms.angle_range.y, rand(&seed));
            p.vel = vec2<f32>(cos(angle), sin(angle)) * speed;

            p.life = mix_f32(uniforms.life_range.x, uniforms.life_range.y, rand(&seed));
            p.max_life = p.life;
            p.size = mix_f32(uniforms.size_range.x, uniforms.size_range.y, rand(&seed));
            p.rotation = rand(&seed) * TAU;
            p.rot_speed = mix_f32(uniforms.spin_range.x, uniforms.spin_range.y, rand(&seed));
            p.color = vec4<f32>(1.0);
        }
    }

    particles[index] = p;
}
