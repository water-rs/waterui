struct GeneratorUniforms {
    resolution: vec2<f32>,
    kind: u32,
    flag: u32,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    params_a: vec4<f32>,
    uint_params: vec4<u32>,
}

@group(0) @binding(0)
var<uniform> uniforms: GeneratorUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );
    let position = positions[vertex_index];
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = (position + 1.0) * 0.5;
    return output;
}

fn checkerboard(uv: vec2<f32>) -> vec3<f32> {
    let cell_size = vec2<f32>(uniforms.uint_params.xy);
    let offset = vec2<f32>(uniforms.uint_params.zw);
    let pixel = uv * uniforms.resolution + offset;
    let cell = floor(pixel / cell_size);
    let parity = i32(cell.x + cell.y) % 2;
    return select(uniforms.color_b.rgb, uniforms.color_a.rgb, parity == 0);
}

fn stripes(uv: vec2<f32>) -> vec3<f32> {
    let pixel = uv * uniforms.resolution;
    let stripe_width = f32(uniforms.uint_params.x);
    let offset = f32(uniforms.uint_params.y);
    let axis = select(pixel.x, pixel.y, uniforms.flag != 0u) + offset;
    let stripe = i32(floor(axis / stripe_width)) % 2;
    return select(uniforms.color_b.rgb, uniforms.color_a.rgb, stripe == 0);
}

fn dot_grid(uv: vec2<f32>) -> vec3<f32> {
    let pixel = uv * uniforms.resolution;
    let spacing = f32(uniforms.uint_params.x);
    let radius = uniforms.params_a.x;
    let centered = fract((pixel + spacing * 0.5) / spacing) - vec2<f32>(0.5, 0.5);
    let distance_to_center = length(centered * spacing);
    let aa = max(fwidth(distance_to_center), 0.75);
    let coverage = 1.0 - smoothstep(
        radius - aa,
        radius + aa,
        distance_to_center,
    );
    return mix(uniforms.color_b.rgb, uniforms.color_a.rgb, coverage);
}

fn noise(uv: vec2<f32>) -> vec3<f32> {
    let pixel = floor(uv * uniforms.resolution);
    let seed = uniforms.params_a.x;
    let seeded = pixel + vec2<f32>(seed, seed * 1.37);
    let dot_product = dot(seeded, vec2<f32>(127.1, 311.7));
    let value = fract(sin(dot_product) * 43758.5453123);
    return vec3<f32>(value);
}

fn linear_gradient(uv: vec2<f32>) -> vec3<f32> {
    let start_point = uniforms.params_a.xy;
    let direction = uniforms.params_a.zw - start_point;
    let length_squared = max(dot(direction, direction), 0.0001);
    let t = clamp(dot(uv - start_point, direction) / length_squared, 0.0, 1.0);
    return mix(uniforms.color_a.rgb, uniforms.color_b.rgb, t);
}

fn radial_gradient(uv: vec2<f32>) -> vec3<f32> {
    let radius = max(uniforms.params_a.z, 0.0001);
    let t = clamp(distance(uv, uniforms.params_a.xy) / radius, 0.0, 1.0);
    return mix(uniforms.color_a.rgb, uniforms.color_b.rgb, t);
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    var color: vec3<f32>;
    switch uniforms.kind {
        case 0u: {
            color = checkerboard(uv);
        }
        case 1u: {
            color = stripes(uv);
        }
        case 2u: {
            color = dot_grid(uv);
        }
        case 3u: {
            color = noise(uv);
        }
        case 4u: {
            color = linear_gradient(uv);
        }
        case 5u: {
            color = radial_gradient(uv);
        }
        default: {
            discard;
        }
    }
    return vec4<f32>(color, 1.0);
}
