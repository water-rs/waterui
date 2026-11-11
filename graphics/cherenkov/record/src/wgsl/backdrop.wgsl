// The backdrop effect contract as declarations: what the backdrop shader
// contract (`cherenkov::BackdropShaderSource`) and the GPU module give a
// registered `backdrop_effect`, for an evaluator that binds the capture
// reads itself — the CPU backend's lane executor and the reference oracle.
// filtrate-core's working-space fragment (`space_to_srgb`,
// `space_from_srgb`, the `SPACE_*` constants) precedes it and the user
// source follows.

const SPACE_LINEAR: u32 = 0u;
const SPACE_SRGB: u32 = 1u;

// One premultiplied pixel moved between spaces, as the GPU module's
// `move_space` does.
fn move_space(px: vec4<f32>, src: u32, dst: u32) -> vec4<f32> {
    if (src == dst) {
        return px;
    }
    if (dst == SPACE_SRGB) {
        return space_to_srgb(px);
    }
    return space_from_srgb(px);
}

// The bound capture, set by the backend for each member: the region's
// texel origin and size on the capture grid, the capture scale, and the
// pyramid's level count.
var<private> backdrop_origin: vec2<f32>;
var<private> backdrop_size: vec2<f32>;
var<private> backdrop_scale: f32;
var<private> backdrop_levels: f32;

struct BackdropPixel {
    p: vec2<f32>,
    sdf: f32,
    normal: vec2<f32>,
    own_sdf: f32,
    size: vec2<f32>,
    scale: f32,
    local: vec2<f32>,
}

struct BackdropField {
    sdf: f32,
    normal: vec2<f32>,
    weight: f32,
}

// The three capture reads. The backend binds every call to them to its
// capture and field; these bodies only declare the signatures and never
// run.
fn backdrop_sample(q: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(0.0);
}

fn backdrop_sample_level(q: vec2<f32>, level: f32) -> vec4<f32> {
    return vec4<f32>(0.0);
}

fn backdrop_field(q: vec2<f32>) -> BackdropField {
    return BackdropField(0.0, vec2<f32>(0.0), 0.0);
}
