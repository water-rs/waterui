// The projective composite (#84): one quad over a projective layer's
// destination region samples the layer's completed local image through
// the inverse homography, reconstructs it with the bounded 16-tap
// anisotropic trilinear filter, and composites the result under the
// ancestor clip with the layer's opacity and blend mode.
//
// `shared.wgsl` and `blend.wgsl` precede this file in the build. The
// instance is an ordinary `KIND_SPAN` quad:
//
// - `bounds` is the device region to shade;
// - `grad.xy` is the region's integer origin: the point the inverse
//   homography is evaluated relative to, and the blend backdrop's texel
//   origin;
// - `params.y` is the layer opacity;
// - the clip fields are the ancestor clip, or for a destructive blend the
//   operator domain (the ancestor clip intersected with the projected
//   layer clip);
// - `meta_.z` indexes three stop-buffer rows holding the inverse
//   homography, destination offsets to base texels, row by row in `.xyz`;
// - `meta_.w` bits 16-23 carry the blend code (0 = source-over).

@group(1) @binding(0) var image: texture_2d<f32>;
// Linear magnification, minification and mip filtering, clamp to edge,
// no hardware anisotropy.
@group(1) @binding(1) var image_sampler: sampler;
// The blend backdrop: a copy of the target over the instance's region.
@group(1) @binding(2) var backdrop: texture_2d<f32>;
// Binding 3, the clip mask texture, is declared in `shared.wgsl`.

// The largest tap count of the anisotropic filter.
const MAX_TAPS: f32 = 16.0;
// How far the anisotropy ratio may exceed an integer and still take that
// many taps (`TAP_SLACK` in `lowering::projective`): rounding must not turn
// an isotropic footprint into two taps.
const TAP_SLACK: f32 = 1.0 / 256.0;

// The weight of a bilinear footprint at level coordinate `u` (texel
// centres at n + 0.5) that falls on texels inside `0..n`.
fn in_domain(u: f32, n: f32) -> f32 {
    let x = u - 0.5;
    let i0 = floor(x);
    let f = x - i0;
    var w = 0.0;
    if i0 >= 0.0 && i0 < n {
        w += 1.0 - f;
    }
    if i0 + 1.0 >= 0.0 && i0 + 1.0 < n {
        w += f;
    }
    return w;
}

// The bilinear sample of mip `level` at base-texel point `at`: texels
// outside the level are transparent. The clamp-to-edge sample repeats the
// edge texel for an outside tap, so scaling it by the inside weight of
// each axis gives exactly the sum over inside taps.
fn level_sample(at: vec2<f32>, base: vec2<f32>, level: u32) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(image, level));
    let u = at * size / base;
    let w = in_domain(u.x, size.x) * in_domain(u.y, size.y);
    return textureSampleLevel(image, image_sampler, at / base, f32(level)) * w;
}

// The anisotropic reconstruction at device pixel centre `pixel`,
// premultiplied; transparent without a front-facing preimage.
fn projected(i: u32, pixel: vec2<f32>) -> vec4<f32> {
    let first = instances[i].meta_.z;
    let r0 = stops[first].color.xyz;
    let r1 = stops[first + 1u].color.xyz;
    let r2 = stops[first + 2u].color.xyz;
    let d = vec3<f32>(pixel - instances[i].grad.xy, 1.0);
    let q = vec3<f32>(dot(r0, d), dot(r1, d), dot(r2, d));
    // The front half-space is exactly w > 0; nothing is clamped.
    if !(q.z > 0.0) {
        return vec4<f32>(0.0);
    }
    let center = q.xy / q.z;
    // Jacobian of the division with respect to the device point.
    let w2 = q.z * q.z;
    let j = vec4<f32>(
        (r0.x * q.z - q.x * r2.x) / w2,
        (r0.y * q.z - q.x * r2.y) / w2,
        (r1.x * q.z - q.y * r2.x) / w2,
        (r1.y * q.z - q.y * r2.y) / w2,
    );
    // Singular values a >= b of J and the major axis e in texel space.
    let xx = j.x * j.x + j.y * j.y;
    let xy = j.x * j.z + j.y * j.w;
    let yy = j.z * j.z + j.w * j.w;
    let root = length(vec2<f32>(0.5 * (xx - yy), xy));
    let mean = 0.5 * (xx + yy);
    let a = sqrt(mean + root);
    let b = sqrt(max(mean - root, 0.0));
    // An isotropic footprint has no major axis and takes one tap; its
    // direction is irrelevant but must stay finite.
    let angle = select(0.5 * atan2(2.0 * xy, xx - yy), 0.0, root == 0.0);
    let e = vec2<f32>(cos(angle), sin(angle));
    let b_eff = max(max(1.0, b), a / MAX_TAPS);
    let taps = clamp(ceil(a / b_eff - TAP_SLACK), 1.0, MAX_TAPS);
    let step = e * (a / taps);
    let top = f32(textureNumLevels(image) - 1u);
    let lod = clamp(log2(b_eff), 0.0, top);
    let fine = floor(lod);
    let blend = lod - fine;
    let coarse = min(fine + 1.0, top);
    let base = vec2<f32>(textureDimensions(image, 0u));
    let n = u32(taps);
    var acc = vec4<f32>(0.0);
    for (var t = 0u; t < n; t += 1u) {
        // Centres of `taps` equal parts of the major axis.
        let at = center + step * (f32(t) + 0.5 - 0.5 * taps);
        let near = level_sample(at, base, u32(fine));
        if blend > 0.0 && coarse != fine {
            acc += mix(near, level_sample(at, base, u32(coarse)), blend);
        } else {
            acc += near;
        }
    }
    return acc / taps;
}

@fragment
fn fs_projective(in: VsOut) -> @location(0) vec4<f32> {
    let i = in.instance;
    let sample = projected(i, in.device);
    let inside = clamp(clip_mask_coverage(in), 0.0, 1.0);
    let mode = (in.meta_.w >> 16u) & 0xffu;
    if mode == 0u {
        return move_space(sample * (inside * in.params.y), SPACE_LINEAR, globals.space);
    }
    // A blended composite reads the target's prior contents from the
    // backdrop copy of the region and writes the result verbatim.
    let cb = textureLoad(backdrop, vec2<i32>(floor(in.device - instances[i].grad.xy)), 0);
    if blend_is_destructive(mode) {
        // The operator applies over its whole domain: a transparent
        // source still replaces the destination there.
        return mix(cb, composite_space(mode, SPACE_LINEAR, cb, sample * in.params.y), inside);
    }
    return composite_space(mode, SPACE_LINEAR, cb, sample * (inside * in.params.y));
}
