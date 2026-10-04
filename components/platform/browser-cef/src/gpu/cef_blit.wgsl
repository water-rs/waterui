@group(0) @binding(0)
var source_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

struct DrawRect {
    /// The quad's rect in normalized destination space.
    destination: vec4<f32>,
    /// The texel rect's normalized source space — narrower than the whole
    /// plane when the shared image is padded past its visible extent.
    source: vec4<f32>,
}

@group(0) @binding(2)
var<uniform> rect: DrawRect;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let uvs = array(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );
    var output: VertexOutput;
    let uv = uvs[vertex_index];
    let normalized = rect.destination.xy + uv * rect.destination.zw;
    output.position = vec4<f32>(
        normalized.x * 2.0 - 1.0,
        1.0 - normalized.y * 2.0,
        0.0,
        1.0,
    );
    output.uv = rect.source.xy + uv * rect.source.zw;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source_texture, source_sampler, input.uv);
}
