// Sharpen shader - standalone pass (spatial filter, cannot fuse)

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
const TILE_WIDTH: u32 = WORKGROUP_X + 2u;
const TILE_HEIGHT: u32 = WORKGROUP_Y + 2u;

var<workgroup> tile: array<array<vec4<f32>, TILE_WIDTH>, TILE_HEIGHT>;

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
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let input_dims_u = vec2<u32>(uniforms.input_dimensions);
    let input_dims_i = vec2<i32>(input_dims_u);
    let coord = vec2<i32>(global_id.xy);
    let amount = param(0u);

    if input_dims_u == dims {
        let output_x = i32(global_id.x);
        let output_y = i32(global_id.y);
        let tile_origin_x = output_x - i32(local_id.x) - 1;
        let tile_origin_y = output_y - i32(local_id.y) - 1;

        var load_y = i32(local_id.y);
        loop {
            if load_y >= i32(TILE_HEIGHT) {
                break;
            }

            var load_x = i32(local_id.x);
            loop {
                if load_x >= i32(TILE_WIDTH) {
                    break;
                }
                let sample_coord = vec2<i32>(
                    clamp(tile_origin_x + load_x, 0, input_dims_i.x - 1),
                    clamp(tile_origin_y + load_y, 0, input_dims_i.y - 1),
                );
                tile[u32(load_y)][u32(load_x)] = textureLoad(input_texture, sample_coord, 0);
                load_x += i32(WORKGROUP_X);
            }

            load_y += i32(WORKGROUP_Y);
        }

        workgroupBarrier();

        let local_x = local_id.x + 1u;
        let local_y = local_id.y + 1u;

        let center = tile[local_y][local_x];
        let top = tile[local_y - 1u][local_x];
        let bottom = tile[local_y + 1u][local_x];
        let left = tile[local_y][local_x - 1u];
        let right = tile[local_y][local_x + 1u];

        let laplacian = center * 4.0 - top - bottom - left - right;
        let result = center + laplacian * amount;
        textureStore(output_texture, coord, vec4<f32>(result.rgb, center.a));
        return;
    }

    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center_coord = clamp(
        vec2<i32>(mapped),
        vec2<i32>(0),
        input_dims_i - vec2<i32>(1),
    );
    let center = textureLoad(input_texture, center_coord, 0);
    let top = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(0, -1),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );
    let bottom = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(0, 1),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );
    let left = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(-1, 0),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );
    let right = textureLoad(
        input_texture,
        clamp(
            center_coord + vec2<i32>(1, 0),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        ),
        0,
    );

    // Laplacian kernel
    let laplacian = center * 4.0 - top - bottom - left - right;

    // Add sharpened detail
    let result = center + laplacian * amount;
    let color = vec4<f32>(result.rgb, center.a);

    textureStore(output_texture, coord, color);
}
