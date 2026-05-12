// Kaleidoscope shader
struct Uniforms { output_dimensions: vec2<f32>, input_dimensions: vec2<f32>, params: array<vec4<f32>, 16>, }
@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
fn param(index:u32)->f32{let v=uniforms.params[index/4u];switch index%4u{case 0u:{return v.x;}case 1u:{return v.y;}case 2u:{return v.z;}default:{return v.w;}}}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) { let dims=vec2<u32>(uniforms.output_dimensions); if gid.x>=dims.x||gid.y>=dims.y{return;} let coord=vec2<i32>(gid.xy); let segments=max(param(0u), 2.0); let rotation=param(1u)*0.017453292519943295; let center=vec2<f32>(param(2u), param(3u)); let uv=(vec2<f32>(gid.xy)+vec2<f32>(0.5))/uniforms.output_dimensions; let delta=uv-center; let radius=length(delta); var angle=atan2(delta.y, delta.x) - rotation; let slice = 6.28318530718 / segments; angle = abs(fract(angle / slice) - 0.5) * slice; let sample_uv = center + vec2<f32>(cos(angle + rotation), sin(angle + rotation)) * radius; let input_dims=vec2<i32>(vec2<u32>(uniforms.input_dimensions)); let mapped=clamp(vec2<i32>(sample_uv*uniforms.input_dimensions), vec2<i32>(0), input_dims-vec2<i32>(1)); textureStore(output_texture, coord, textureLoad(input_texture, mapped, 0)); }
