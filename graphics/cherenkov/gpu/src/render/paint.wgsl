struct Uniforms {
    time: f32,
    _padding: f32,
    resolution: vec2<f32>,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<uniform> params: array<vec4<f32>, 16>;
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-1.0, 1.0), vec2(3.0, 1.0));
    let position = positions[index];
    var output: VertexOutput;
    output.position = vec4(position, 0.0, 1.0);
    output.uv = vec2((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return output;
}

