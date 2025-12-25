struct Uniforms {
    color: vec4<f32>,
    size: vec2<f32>,
    _padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    // Convert from pixel coordinates to clip space (-1 to 1)
    let x = (position.x / uniforms.size.x) * 2.0 - 1.0;
    let y = 1.0 - (position.y / uniforms.size.y) * 2.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Output HDR color directly (values > 1.0 are preserved)
    return uniforms.color;
}
