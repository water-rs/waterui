//! WGSL shader sources for the particle system.

/// Compute shader for particle simulation.
pub const COMPUTE_SHADER: &str = r#"
struct Particle {
    pos: vec2<f32>,
    vel: vec2<f32>,
    life: f32,
    max_life: f32,
    size: f32,
    _pad: f32,
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
    stretch_factor: f32,
    softness: f32,
    life_range: vec2<f32>,
    speed_range: vec2<f32>,
    angle_range: vec2<f32>,
    size_range: vec2<f32>,
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    shape: u32,
    _pad: vec3<u32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;

// PCG random number generator
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

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= uniforms.max_particles) {
        return;
    }

    var p = particles[index];
    var seed = uniforms.seed + index;

    if (p.life > 0.0) {
        // Apply forces
        let drag = 1.0; // TODO: add drag to uniforms
        p.vel = p.vel * drag;
        p.vel += (uniforms.gravity + uniforms.wind) * uniforms.dt;
        
        // Turbulence (random horizontal jitter)
        if (uniforms.turbulence > 0.0) {
            let jitter = (rand(&seed) - 0.5) * uniforms.turbulence * uniforms.dt;
            p.vel.x += jitter;
        }
        
        // Update position
        p.pos += p.vel * uniforms.dt;
        
        // Decrease life
        p.life -= uniforms.dt;
    } else {
        // Dead particle: maybe respawn
        let spawn_chance = uniforms.emit_rate * uniforms.dt / f32(uniforms.max_particles);
        
        // Always spawn until we hit steady state? No, random chance is correct for uniform rate.
        // But to fill initial buffer faster, logic might differ. For now standard.
        
        if (rand(&seed) < spawn_chance) {
            // Respawn at emitter
            let offset_x = (rand(&seed) - 0.5) * uniforms.emitter_size.x;
            let offset_y = (rand(&seed) - 0.5) * uniforms.emitter_size.y;
            p.pos = uniforms.emitter_pos + vec2<f32>(offset_x, offset_y);
            
            // Random velocity
            let speed = mix_f32(uniforms.speed_range.x, uniforms.speed_range.y, rand(&seed));
            let angle = mix_f32(uniforms.angle_range.x, uniforms.angle_range.y, rand(&seed));
            
            p.vel = vec2<f32>(cos(angle), sin(angle)) * speed;
            
            // Life & Size
            p.life = mix_f32(uniforms.life_range.x, uniforms.life_range.y, rand(&seed));
            p.max_life = p.life;
            p.size = mix_f32(uniforms.size_range.x, uniforms.size_range.y, rand(&seed));
            
            // Color is calculated in render shader usually, but we can store per-particle color data if we wanted.
            // For now, we just zero it or store something useful.
            p.color = vec4<f32>(1.0); 
        }
    }

    particles[index] = p;
}
"#;

/// Render shader for particle visualization.
pub const RENDER_SHADER: &str = r#"
struct Particle {
    pos: vec2<f32>,
    vel: vec2<f32>,
    life: f32,
    max_life: f32,
    size: f32,
    _pad: f32,
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
    stretch_factor: f32,
    softness: f32,
    life_range: vec2<f32>,
    speed_range: vec2<f32>,
    angle_range: vec2<f32>,
    size_range: vec2<f32>,
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    shape: u32,
    _pad: vec3<u32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) life_ratio: f32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> particles: array<Particle>;

// Quad vertices for instancing
const QUAD_VERTICES: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 1.0, -1.0),
    vec2<f32>(-1.0,  1.0),
    vec2<f32>(-1.0,  1.0),
    vec2<f32>( 1.0, -1.0),
    vec2<f32>( 1.0,  1.0),
);

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let p = particles[instance_index];
    
    var out: VertexOutput;
    
    // Discard dead particles by moving offscreen
    if (p.life <= 0.0) {
        out.position = vec4<f32>(-10.0, -10.0, 0.0, 1.0);
        return out;
    }
    
    let quad_pos = QUAD_VERTICES[vertex_index];
    var world_pos = vec2<f32>(0.0);
    
    // Velocity Stretching
    if (uniforms.stretch_factor > 0.0) {
        let speed = length(p.vel);
        if (speed > 0.0001) {
            let dir = p.vel / speed;
            let perp = vec2<f32>(-dir.y, dir.x);
            
            // Stretch along directory based on speed
            let stretch_amount = 1.0 + speed * uniforms.stretch_factor * 10.0;
            
            // Width is perp, Length is dir
            // quad.x controls width, quad.y controls length
            let width = p.size;
            let length = p.size * stretch_amount;
            
            world_pos = p.pos + (perp * quad_pos.x * width) + (dir * quad_pos.y * length);
        } else {
            world_pos = p.pos + quad_pos * p.size;
        }
    } else {
        world_pos = p.pos + quad_pos * p.size;
    }
    
    // Convert from [0,1] to clip space [-1,1]
    let clip_pos = world_pos * 2.0 - 1.0;
    out.position = vec4<f32>(clip_pos.x, -clip_pos.y, 0.0, 1.0);
    
    out.uv = (quad_pos + 1.0) * 0.5;
    
    // Color Interpolation
    let ratio = p.life / p.max_life;
    out.color = mix(uniforms.color_end, uniforms.color_start, ratio);
    
    out.life_ratio = ratio;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Procedural circle / soft particle
    // UV is 0..1
    
    let center = vec2<f32>(0.5, 0.5);
    var dist = 0.0;

    if (uniforms.shape == 1u) {
        // Rect/Box SDF
        // uv is 0..1, center is 0.5
        // distance to edge of 0.5 box
        let d = abs(in.uv - center);
        dist = max(d.x, d.y);
    } else {
        // Circle SDF
        dist = distance(in.uv, center);
    }
    
    let edge = 0.5;
    // Ensure smooth_width is at least a tiny bit to avoid aliasing even at 0.0
    let smooth_width = max(0.01, uniforms.softness * 0.5); 
    
    let alpha = 1.0 - smoothstep(edge - smooth_width, edge, dist);
    
    let final_alpha = in.color.a * alpha;
    
    // Premultiply Alpha: RGB * Alpha
    // This supports both Additive (One, One) and Alpha (One, OneMinusSrcAlpha) blending
    return vec4<f32>(in.color.rgb * final_alpha, final_alpha);
}
"#;
