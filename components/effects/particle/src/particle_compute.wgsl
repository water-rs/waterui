struct Particle {
    pos: vec2<f32>,
    vel: vec2<f32>,
    life: f32,
    max_life: f32,
    size: f32,
    rotation: f32,
    rot_speed: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    color: vec4<f32>,
}

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

struct CircleObstacle {
    center: vec2<f32>,
    radius: f32,
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

@group(0) @binding(0) var<storage, read> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> particle_source: array<Particle>;
@group(0) @binding(2) var<storage, read_write> particle_target: array<Particle>;
@group(0) @binding(3) var<storage, read> circle_obstacles: array<CircleObstacle>;
@group(0) @binding(4) var<storage, read_write> cell_heads: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> particle_links: array<u32>;

const INVALID_INDEX: u32 = 0xffffffffu;
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

fn grid_cell_index(position: vec2<f32>) -> u32 {
    let clamped = clamp(position, vec2<f32>(0.0, 0.0), vec2<f32>(0.99999994, 0.99999994));
    let cell = vec2<u32>(
        u32(clamped.x * f32(uniforms.interaction.grid_width)),
        u32(clamped.y * f32(uniforms.interaction.grid_height)),
    );
    return cell.y * uniforms.interaction.grid_width + cell.x;
}

fn interaction_radius(a: Particle, b: Particle) -> f32 {
    return uniforms.interaction.radius + a.size + b.size;
}

fn apply_particle_neighbor_interaction(index: u32, particle: ptr<function, Particle>) {
    if (uniforms.interaction.enabled == 0u) {
        return;
    }

    let cell = vec2<i32>(
        i32(grid_cell_index((*particle).pos) % uniforms.interaction.grid_width),
        i32(grid_cell_index((*particle).pos) / uniforms.interaction.grid_width),
    );

    for (var oy = -1; oy <= 1; oy += 1) {
        let neighbor_y = cell.y + oy;
        if (neighbor_y < 0 || neighbor_y >= i32(uniforms.interaction.grid_height)) {
            continue;
        }

        for (var ox = -1; ox <= 1; ox += 1) {
            let neighbor_x = cell.x + ox;
            if (neighbor_x < 0 || neighbor_x >= i32(uniforms.interaction.grid_width)) {
                continue;
            }

            let neighbor_cell_index = u32(neighbor_y) * uniforms.interaction.grid_width + u32(neighbor_x);
            var neighbor_index = atomicLoad(&cell_heads[neighbor_cell_index]);
            loop {
                if (neighbor_index == INVALID_INDEX) {
                    break;
                }

                if (neighbor_index != index) {
                    let neighbor = particle_source[neighbor_index];
                    if (neighbor.life > 0.0 && neighbor.max_life > 0.0) {
                        let delta = (*particle).pos - neighbor.pos;
                        let radius = interaction_radius((*particle), neighbor);
                        let distance_sq = dot(delta, delta);
                        if (distance_sq < radius * radius) {
                            let distance = sqrt(distance_sq);
                            var normal = vec2<f32>(0.0, -1.0);
                            if (distance > 0.000001) {
                                normal = delta / distance;
                            } else {
                                let hashed = pcg_hash(index ^ neighbor_index);
                                let angle = f32(hashed & 1023u) / 1023.0 * TAU;
                                normal = vec2<f32>(cos(angle), sin(angle));
                            }

                            let overlap = radius - distance;
                            let relative_speed = dot((*particle).vel - neighbor.vel, normal);
                            let impulse = overlap * uniforms.interaction.strength;
                            (*particle).vel += normal * impulse * uniforms.dt;
                            if (relative_speed < 0.0) {
                                (*particle).vel -= normal * relative_speed * 0.5;
                            }
                        }
                    }
                }

                neighbor_index = particle_links[neighbor_index];
            }
        }
    }
}

fn apply_bounds_collision(particle: ptr<function, Particle>) {
    if (uniforms.collision.enabled == 0u) {
        return;
    }

    let extent = (*particle).size;
    let min_bound = uniforms.collision.bounds.xy + vec2<f32>(extent, extent);
    let max_bound = uniforms.collision.bounds.zw - vec2<f32>(extent, extent);

    if ((*particle).pos.x < min_bound.x) {
        (*particle).pos.x = min_bound.x;
        (*particle).vel.x = abs((*particle).vel.x) * uniforms.collision.restitution;
        (*particle).vel.y *= uniforms.collision.surface_friction;
    } else if ((*particle).pos.x > max_bound.x) {
        (*particle).pos.x = max_bound.x;
        (*particle).vel.x = -abs((*particle).vel.x) * uniforms.collision.restitution;
        (*particle).vel.y *= uniforms.collision.surface_friction;
    }

    if ((*particle).pos.y < min_bound.y) {
        (*particle).pos.y = min_bound.y;
        (*particle).vel.y = abs((*particle).vel.y) * uniforms.collision.restitution;
        (*particle).vel.x *= uniforms.collision.surface_friction;
    } else if ((*particle).pos.y > max_bound.y) {
        (*particle).pos.y = max_bound.y;
        (*particle).vel.y = -abs((*particle).vel.y) * uniforms.collision.restitution;
        (*particle).vel.x *= uniforms.collision.surface_friction;
    }
}

fn apply_circle_obstacle_collision(
    particle: ptr<function, Particle>,
    obstacle: CircleObstacle,
) {
    let combined_radius = obstacle.radius + (*particle).size;
    let to_particle = (*particle).pos - obstacle.center;
    let distance_sq = dot(to_particle, to_particle);
    if (distance_sq >= combined_radius * combined_radius) {
        return;
    }

    let speed = length((*particle).vel);
    var normal = vec2<f32>(0.0, -1.0);
    if (distance_sq > 0.000001) {
        normal = normalize(to_particle);
    } else if (speed > 0.0001) {
        normal = normalize(-(*particle).vel);
    }

    (*particle).pos = obstacle.center + normal * combined_radius;

    let normal_velocity = dot((*particle).vel, normal);
    if (normal_velocity >= 0.0) {
        return;
    }

    let tangent_velocity = (*particle).vel - normal * normal_velocity;
    (*particle).vel = tangent_velocity * uniforms.collision.surface_friction
        - normal * normal_velocity * uniforms.collision.restitution;
}

fn apply_circle_obstacle_collisions(particle: ptr<function, Particle>) {
    for (var i = 0u; i < uniforms.collision.circle_obstacle_count; i += 1u) {
        apply_circle_obstacle_collision(particle, circle_obstacles[i]);
    }
}

@compute @workgroup_size(64)
fn clear_grid(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    let cell_count = uniforms.interaction.grid_width * uniforms.interaction.grid_height;
    if (index >= cell_count) {
        return;
    }
    atomicStore(&cell_heads[index], INVALID_INDEX);
}

@compute @workgroup_size(64)
fn build_grid(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= uniforms.max_particles) {
        return;
    }

    let particle = particle_source[index];
    particle_links[index] = INVALID_INDEX;
    if (particle.life <= 0.0 || particle.max_life <= 0.0) {
        return;
    }

    let cell_index = grid_cell_index(particle.pos);
    let previous = atomicExchange(&cell_heads[cell_index], index);
    particle_links[index] = previous;
}

@compute @workgroup_size(64)
fn simulate_particles(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= uniforms.max_particles) {
        return;
    }

    var p = particle_source[index];
    var seed = uniforms.seed + index;

    if (p.life > 0.0) {
        let drag_factor = pow(uniforms.drag, uniforms.dt * 60.0);
        p.vel = p.vel * drag_factor;
        p.vel += (uniforms.gravity + uniforms.wind) * uniforms.dt;

        if (uniforms.turbulence > 0.0) {
            let jitter = (rand(&seed) - 0.5) * uniforms.turbulence * uniforms.dt;
            p.vel.x += jitter;
        }

        apply_particle_neighbor_interaction(index, &p);
        p.pos += p.vel * uniforms.dt;
        apply_bounds_collision(&p);
        apply_circle_obstacle_collisions(&p);
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

    particle_target[index] = p;
}
