// Sepia: mixes colour toward the classic sepia tone matrix.

struct Params {
    intensity: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let sepia = vec3<f32>(
        dot(color.rgb, vec3<f32>(0.393, 0.769, 0.189)),
        dot(color.rgb, vec3<f32>(0.349, 0.686, 0.168)),
        dot(color.rgb, vec3<f32>(0.272, 0.534, 0.131)),
    );
    return vec4<f32>(mix(color.rgb, sepia, params.intensity), color.a);
}
