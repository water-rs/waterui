// The content-effect shader contract as declarations: what
// `cherenkov::ContentShaderSource` and the GPU module give a registered
// `content_effect`, for validation and for an evaluator that binds
// `content_sample` itself (the CPU backend's lane executor).
// filtrate-core's working-space fragment (`space_to_srgb`,
// `space_from_srgb`, the `SPACE_*` constants) precedes it and the user
// source follows.

const SPACE_LINEAR: u32 = 0u;
const SPACE_SRGB: u32 = 1u;

// One premultiplied pixel moved between spaces, matching the GPU module.
fn move_space(px: vec4<f32>, src: u32, dst: u32) -> vec4<f32> {
    if (src == dst) {
        return px;
    }
    if (dst == SPACE_SRGB) {
        return space_to_srgb(px);
    }
    return space_from_srgb(px);
}

struct ContentPixel {
    p: vec2<f32>,
    sdf: f32,
    normal: vec2<f32>,
    size: vec2<f32>,
    scale: f32,
    local: vec2<f32>,
}

// Bilinear read of the layer's isolated content at device point `q`,
// returning premultiplied extended linear Display P3 and transparent
// outside the isolated content bounds. The backend binds every call to its
// isolated content image; this body only declares the signature.
fn content_sample(q: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(0.0);
}
