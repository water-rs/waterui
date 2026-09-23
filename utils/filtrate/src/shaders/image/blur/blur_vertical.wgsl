// Separable vertical blur pass.
// Reads the horizontal-pass texture and writes final blurred result.

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
const SEGMENT_HEIGHT: i32 = i32(WORKGROUP_Y * 2u + 1u);
const TILE_HEIGHT: u32 = WORKGROUP_Y * 3u;

var<workgroup> tile_cols: array<array<vec4<f32>, TILE_HEIGHT>, WORKGROUP_X>;

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

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    let is_active = global_id.x < dims.x && global_id.y < dims.y;

    let input_dims_u = vec2<u32>(uniforms.input_dimensions);
    let input_dims_i = vec2<i32>(input_dims_u);
    let output_coord = vec2<i32>(global_id.xy);

    let radius = max(i32(round(param(0u))), 0);
    if radius == 0 {
        if is_active {
            let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
                / uniforms.output_dimensions;
            let center = clamp(
                vec2<i32>(mapped),
                vec2<i32>(0),
                input_dims_i - vec2<i32>(1),
            );
            textureStore(output_texture, output_coord, textureLoad(input_texture, center, 0));
        }
        return;
    }

    if all(input_dims_u == dims) {
        let output_x = i32(global_id.x);
        let output_y = i32(global_id.y);
        let group_origin_y = output_y - i32(local_id.y);

        var sum = vec4<f32>(0.0);
        var count = 0.0;
        var segment_start = -radius;

        loop {
            if segment_start > radius {
                break;
            }

            let segment_end = min(segment_start + SEGMENT_HEIGHT - 1, radius);
            let segment_len = segment_end - segment_start + 1;
            let shared_height = i32(WORKGROUP_Y) + segment_len - 1;

            var load_y = i32(local_id.y);
            loop {
                if load_y >= shared_height {
                    break;
                }
                let sample_y = clamp(
                    group_origin_y + segment_start + load_y,
                    0,
                    input_dims_i.y - 1,
                );
                tile_cols[local_id.x][u32(load_y)] =
                    textureLoad(input_texture, vec2<i32>(output_x, sample_y), 0);
                load_y += i32(WORKGROUP_Y);
            }

            workgroupBarrier();

            let base = i32(local_id.y);
            var offset = 0;
            loop {
                if offset >= segment_len {
                    break;
                }
                sum += tile_cols[local_id.x][u32(base + offset)];
                offset += 1;
            }
            count += f32(segment_len);

            workgroupBarrier();
            segment_start = segment_end + 1;
        }

        if is_active {
            textureStore(output_texture, output_coord, sum / count);
        }
        return;
    }

    if !is_active {
        return;
    }

    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center = clamp(
        vec2<i32>(mapped),
        vec2<i32>(0),
        input_dims_i - vec2<i32>(1),
    );

    var sum = vec4<f32>(0.0);
    var count = 0.0;
    for (var y = -radius; y <= radius; y++) {
        let sample_coord = clamp(
            center + vec2<i32>(0, y),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        );
        sum += textureLoad(input_texture, sample_coord, 0);
        count += 1.0;
    }

    textureStore(output_texture, output_coord, sum / count);
}
