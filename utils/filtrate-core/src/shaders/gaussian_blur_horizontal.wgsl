// Separable horizontal gaussian blur pass.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

const WORKGROUP_X: u32 = 8u;
const WORKGROUP_Y: u32 = 8u;
const SEGMENT_WIDTH: i32 = i32(WORKGROUP_X * 2u + 1u);
const TILE_WIDTH: u32 = WORKGROUP_X * 3u;

var<workgroup> tile_rows: array<array<vec4<f32>, TILE_WIDTH>, WORKGROUP_Y>;

fn param(index: u32) -> f32 {
    let vec_idx = index / 4u;
    let component = index % 4u;
    let v = uniforms.params[vec_idx];
    switch component {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

fn gaussian_weight(offset: i32, sigma: f32) -> f32 {
    let distance = f32(offset);
    return exp(-(distance * distance) / (2.0 * sigma * sigma));
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    let in_bounds = global_id.x < dims.x && global_id.y < dims.y;

    let input_dims_u = vec2<u32>(uniforms.input_dimensions);
    let input_dims_i = vec2<i32>(input_dims_u);
    let output_coord = vec2<i32>(global_id.xy);

    let sigma = max(param(0u), 0.001);
    let radius = max(i32(ceil(sigma * 3.0)), 0);
    if radius == 0 {
        if in_bounds {
            let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
                / uniforms.output_dimensions;
            let center = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims_i - vec2<i32>(1));
            textureStore(output_texture, output_coord, textureLoad(input_texture, center, 0));
        }
        return;
    }

    if all(input_dims_u == dims) {
        let output_x = i32(global_id.x);
        let output_y = i32(global_id.y);
        let group_origin_x = output_x - i32(local_id.x);

        var sum = vec4<f32>(0.0);
        var weight_total = 0.0;
        var segment_start = -radius;

        loop {
            if segment_start > radius {
                break;
            }

            let segment_end = min(segment_start + SEGMENT_WIDTH - 1, radius);
            let segment_len = segment_end - segment_start + 1;
            let shared_width = i32(WORKGROUP_X) + segment_len - 1;

            var load_x = i32(local_id.x);
            loop {
                if load_x >= shared_width {
                    break;
                }
                let sample_x = clamp(group_origin_x + segment_start + load_x, 0, input_dims_i.x - 1);
                tile_rows[local_id.y][u32(load_x)] = textureLoad(input_texture, vec2<i32>(sample_x, output_y), 0);
                load_x += i32(WORKGROUP_X);
            }

            workgroupBarrier();

            let base = i32(local_id.x);
            var offset = 0;
            loop {
                if offset >= segment_len {
                    break;
                }
                let sample_offset = segment_start + offset;
                let weight = gaussian_weight(sample_offset, sigma);
                sum += tile_rows[local_id.y][u32(base + offset)] * weight;
                weight_total += weight;
                offset += 1;
            }

            workgroupBarrier();
            segment_start = segment_end + 1;
        }

        if in_bounds {
            textureStore(output_texture, output_coord, sum / max(weight_total, 0.0001));
        }
        return;
    }

    if !in_bounds {
        return;
    }

    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions / uniforms.output_dimensions;
    let center = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims_i - vec2<i32>(1));

    var sum = vec4<f32>(0.0);
    var weight_total = 0.0;
    for (var x = -radius; x <= radius; x++) {
        let sample_coord = clamp(center + vec2<i32>(x, 0), vec2<i32>(0), input_dims_i - vec2<i32>(1));
        let weight = gaussian_weight(x, sigma);
        sum += textureLoad(input_texture, sample_coord, 0) * weight;
        weight_total += weight;
    }

    textureStore(output_texture, output_coord, sum / max(weight_total, 0.0001));
}
