// The corpus shader paint: three premultiplied corner colours blended
// across `uv` (params 0-2) under a soft ring that darkens and fades the
// paint between radii params[3].x and params[3].y to alpha params[3].z.
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let ring = params[3];
    let distance = length(uv - vec2<f32>(0.5, 0.5));
    let t = smoothstep(ring.x, ring.y, distance);
    let base = mix(mix(params[0], params[1], uv.x), params[2], uv.y * uv.y);
    let alpha = mix(1.0, ring.z, t);
    return vec4<f32>(base.rgb * (1.0 - 0.5 * t) * alpha, alpha);
}
