// Shaderloom reflection fixture covering resources shared across render stages.
@group(0) @binding(0) var<uniform> scale: vec4<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index) - 1);
    return vec4<f32>(x * scale.x, 0.0, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return scale;
}
