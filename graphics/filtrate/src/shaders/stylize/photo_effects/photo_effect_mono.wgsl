// Photo effect: monochrome — the working-space luma.

struct WorkingSpace {
    luma: vec3<f32>,
}

fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    return vec4<f32>(vec3<f32>(dot(color.rgb, space.luma)), color.a);
}
