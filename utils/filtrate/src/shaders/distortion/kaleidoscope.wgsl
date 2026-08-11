// Kaleidoscope: folds the image into `segments` mirrored angular slices
// around a center point.
//
// Parameters: segments, rotation (degrees), center.x, center.y (uv space).
// Angles are computed in isotropic space so slices keep equal angular width
// on non-square targets.

const TAU: f32 = 6.283185307179586;

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = output_uv(gid.xy);
    let segments = max(param(0u), 2.0);
    let rotation = param(1u) * DEGREES_TO_RADIANS;
    let center = vec2<f32>(param(2u), param(3u));

    let center_iso = to_isotropic(center);
    let delta = to_isotropic(uv) - center_iso;
    let r = length(delta);
    var angle = atan2(delta.y, delta.x) - rotation;
    let slice = TAU / segments;
    angle = abs(fract(angle / slice) - 0.5) * slice;
    let folded = vec2<f32>(cos(angle + rotation), sin(angle + rotation)) * r;
    let sample_uv = from_isotropic(center_iso + folded);
    textureStore(output_texture, vec2<i32>(gid.xy), sample_input_bilinear(sample_uv));
}
