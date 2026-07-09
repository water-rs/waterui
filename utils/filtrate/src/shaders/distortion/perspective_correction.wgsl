// Perspective correction shader
struct Uniforms { output_dimensions: vec2<f32>, input_dimensions: vec2<f32>, params: array<vec4<f32>, 16>, }
@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
fn param(index:u32)->f32{let v=uniforms.params[index/4u];switch index%4u{case 0u:{return v.x;}case 1u:{return v.y;}case 2u:{return v.z;}default:{return v.w;}}}
fn bilerp(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>, uv: vec2<f32>) -> vec2<f32> { return mix(mix(a,b,uv.x), mix(d,c,uv.x), uv.y); }
@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) { let dims=vec2<u32>(uniforms.output_dimensions); if gid.x>=dims.x||gid.y>=dims.y{return;} let coord=vec2<i32>(gid.xy); let uv=(vec2<f32>(gid.xy)+vec2<f32>(0.5))/uniforms.output_dimensions; let tl=vec2<f32>(param(0u),param(1u)); let tr=vec2<f32>(param(2u),param(3u)); let br=vec2<f32>(param(4u),param(5u)); let bl=vec2<f32>(param(6u),param(7u)); let sample_uv = bilerp(tl,tr,br,bl,uv); let input_dims=vec2<i32>(vec2<u32>(uniforms.input_dimensions)); let mapped=clamp(vec2<i32>(sample_uv*uniforms.input_dimensions), vec2<i32>(0), input_dims-vec2<i32>(1)); textureStore(output_texture, coord, textureLoad(input_texture, mapped, 0)); }
