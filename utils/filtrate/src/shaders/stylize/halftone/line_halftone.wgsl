// Line halftone shader
struct Uniforms { output_dimensions: vec2<f32>, input_dimensions: vec2<f32>, params: array<vec4<f32>, 16>, }
@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
fn param(index:u32)->f32{let v=uniforms.params[index/4u];switch index%4u{case 0u:{return v.x;}case 1u:{return v.y;}case 2u:{return v.z;}default:{return v.w;}}}
fn luminance(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126,0.7152,0.0722)); }
@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) { let dims=vec2<u32>(uniforms.output_dimensions); if gid.x>=dims.x||gid.y>=dims.y{return;} let coord=vec2<i32>(gid.xy); let scale=max(param(0u),2.0); let angle=param(1u)*0.017453292519943295; let center=vec2<f32>(param(2u),param(3u)); let uv=(vec2<f32>(gid.xy)+vec2<f32>(0.5))/uniforms.output_dimensions; let input_dims=vec2<i32>(vec2<u32>(uniforms.input_dimensions)); let mapped=clamp(vec2<i32>(uv*uniforms.input_dimensions), vec2<i32>(0), input_dims-vec2<i32>(1)); let base=textureLoad(input_texture,mapped,0); let rel=(uv-center)*uniforms.output_dimensions; let rotated = sin(angle) * rel.x + cos(angle) * rel.y; let stripe = abs(fract(rotated / scale) - 0.5) * 2.0; let threshold = luminance(base.rgb); let mask = select(0.0, 1.0, stripe < threshold); textureStore(output_texture, coord, vec4<f32>(vec3<f32>(mask), base.a)); }
