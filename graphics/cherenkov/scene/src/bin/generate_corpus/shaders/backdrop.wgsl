// The corpus backdrop shader: a lens that pulls the capture inward along
// the field's normal near the member's edge (depth params[1].x, strength
// params[1].y), mixed with a pyramid read whose level ramps to params[1].z
// at the edge, tinted by params[0] fading down the member's own height,
// plus a rim of colour params[2].rgb at gain params[2].a.
fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
    let lens = params[1];
    let t = clamp(1.0 + px.sdf / lens.x, 0.0, 1.0);
    let lensed = backdrop_sample(px.p - px.normal * (lens.y * t * t));
    let blurred = backdrop_sample_level(px.p, lens.z * t);
    let c = mix(lensed, blurred, 0.5);
    let down = clamp((px.local.y - params[3].y) / px.size.y, 0.0, 1.0);
    let tint = mix(params[0].rgb, vec3<f32>(1.0), down);
    let rim = params[2].rgb * (params[2].a * t * t * t * c.a);
    return vec4<f32>(c.rgb * tint + rim, c.a);
}
