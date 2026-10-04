// The ShaderEffect prelude, appended after the application's WGSL module.
// WGSL resolves module-scope declarations regardless of order, so the
// application's `main` can use everything declared here, and diagnostics keep
// the application's own line numbers.

struct ShaderEffectUniforms {
    // Output size in physical pixels.
    resolution: vec2<f32>,
    // Input (captured content) size in physical pixels.
    input_resolution: vec2<f32>,
    // Seconds on the host-selected frame timeline.
    time: f32,
    // Seconds since the previous frame.
    time_delta: f32,
    // The host's frame sequence number, wrapping at 2^32.
    frame: u32,
    // Number of application parameters bound to this effect.
    param_count: u32,
    // Application parameters, four per vector; read them with `effect_param`.
    params: array<vec4<f32>, 4>,
}

@group(0) @binding(0)
var input_texture: texture_2d<f32>;
@group(0) @binding(1)
var input_sampler: sampler;
@group(0) @binding(2)
var<uniform> uniforms: ShaderEffectUniforms;

// The application parameter at `index`, in the order the parameters were added.
fn effect_param(index: u32) -> f32 {
    return uniforms.params[index / 4u][index % 4u];
}

struct VertexOutput {
    // Fragment position in output pixels, origin at the top-left.
    @builtin(position) position: vec4<f32>,
    // Normalized input coordinates, (0, 0) at the top-left.
    @location(0) uv: vec2<f32>,
}

// One triangle covering the whole target.
@vertex
fn shader_effect_vs(@builtin(vertex_index) index: u32) -> VertexOutput {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    var output: VertexOutput;
    output.position = vec4<f32>(x, y, 0.0, 1.0);
    output.uv = vec2<f32>(x + 1.0, 1.0 - y) * 0.5;
    return output;
}
