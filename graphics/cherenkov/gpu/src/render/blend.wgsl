// Blend-mode compositing shared by the engine tail (`shader.wgsl`) and the
// projective composite (`projective.wgsl`): W3C Compositing and Blending
// Level 1 on premultiplied pixels. `build.rs` concatenates it after the
// module's own tail; it is not a module on its own.

// The isolated-plane composite: the source texel is already in blend
// space `s`; the backdrop converts in and the blended result converts
// back to the pass's storage space.
fn composite_space(mode: u32, s: u32, cb: vec4<f32>, cs: vec4<f32>) -> vec4<f32> {
    return move_space(blend_color(mode, move_space(cb, globals.space, s), cs), s, globals.space);
}

// W3C Compositing and Blending Level 1, a literal port of
// oracle/src/blend.rs. Premultiplied inputs and output.
//
// Extension over the W3C text (#2908): the four non-separable modes
// (Hue, Saturation, Color, Luminosity) evaluate B on a shared affine
// domain covering both straight colours instead of fixed SDR [0,1], so
// extended-range and negative channels blend through the same formulas
// rather than clamping. See blend_color.

// Non-separable inputs have already been normalized to [0, 1]. Keep
// branch selection and divisors in f32; bounded colour arithmetic uses f16.
fn lum(c: vec3<f32>) -> f32 {
    let h = vec3<f16>(c);
    return f32(0.3h * h.x + 0.59h * h.y + 0.11h * h.z);
}

fn sat(c: vec3<f32>) -> f32 {
    return max(c.x, max(c.y, c.z)) - min(c.x, min(c.y, c.z));
}

fn clip_color(c_in: vec3<f32>) -> vec3<f32> {
    var c = vec3<f16>(c_in);
    let l = lum(c_in);
    let n = min(c_in.x, min(c_in.y, c_in.z));
    let x = max(c_in.x, max(c_in.y, c_in.z));
    if n < 0.0 {
        c = f16(l) + (c - f16(l)) * f16(l / (l - n));
    }
    if x > 1.0 {
        c = f16(l) + (c - f16(l)) * f16((1.0 - l) / (x - l));
    }
    return vec3<f32>(c);
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let d = f16(l) - f16(lum(c));
    return clip_color(vec3<f32>(vec3<f16>(c) + d));
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    var mn = 0;
    var mx = 0;
    for (var i = 1; i < 3; i += 1) {
        if c[i] < c[mn] {
            mn = i;
        }
        if c[i] > c[mx] {
            mx = i;
        }
    }
    let imid = 3 - mn - mx;
    var out = vec3<f16>(0.0h);
    if c[mx] > c[mn] {
        out[imid] = f16((c[imid] - c[mn]) / (c[mx] - c[mn])) * f16(s);
        out[mx] = f16(s);
    }
    return vec3<f32>(out);
}

// The Porter-Duff operators whose transparent source replaces the
// destination rather than leaving it unchanged (codes per blend_code).
fn blend_is_destructive(mode: u32) -> bool {
    return mode == 16u || mode == 17u || mode == 20u || mode == 21u || mode == 22u || mode == 25u;
}

// B(Cb, Cs) for one channel pair, separable modes; non-separable modes and
// Porter-Duff operators are handled in blend_color, not here.
fn blend_channel(mode: u32, cb: f32, cs: f32) -> f32 {
    switch mode {
        case 1u: { return cb * cs; }                                 // Multiply
        case 2u: { return cb + cs - cb * cs; }                       // Screen
        case 3u: {                                                   // Overlay
            if cb <= 0.5 { return 2.0 * cb * cs; }
            return 1.0 - 2.0 * (1.0 - cb) * (1.0 - cs);
        }
        case 4u: { return min(cb, cs); }                             // Darken
        case 5u: { return max(cb, cs); }                             // Lighten
        case 6u: {                                                   // ColorDodge
            if cs >= 1.0 { return 1.0; }
            return min(cb / (1.0 - cs), 1.0);
        }
        case 7u: {                                                   // ColorBurn
            if cs <= 0.0 { return 0.0; }
            return 1.0 - min((1.0 - cb) / cs, 1.0);
        }
        case 8u: {                                                   // HardLight
            if cs <= 0.5 { return 2.0 * cb * cs; }
            return 1.0 - 2.0 * (1.0 - cb) * (1.0 - cs);
        }
        case 9u: {                                                   // SoftLight
            if cs <= 0.5 {
                return cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb);
            }
            var d: f32;
            if cb <= 0.25 {
                d = ((16.0 * cb - 12.0) * cb + 4.0) * cb;
            } else {
                d = sqrt(cb);
            }
            return cb + (2.0 * cs - 1.0) * (d - cb);
        }
        case 10u: { return abs(cb - cs); }                           // Difference
        case 11u: { return cb + cs - 2.0 * cb * cs; }                // Exclusion
        default: { return cs; }
    }
}

// Porter-Duff: co = αs·Fa·Cs + αb·Fb·Cb in premultiplied form.
fn porter_duff(fa: f32, fb: f32, cb: vec4<f32>, cs: vec4<f32>) -> vec4<f32> {
    return fa * cs + fb * cb;
}

// blend(mode, cb, cs): premultiplied backdrop and source, composited
// premultiplied output — oracle blend().
fn blend_color(mode: u32, cb: vec4<f32>, cs: vec4<f32>) -> vec4<f32> {
    let ab = cb.a;
    let as_ = cs.a;
    switch mode {
        case 16u: { return vec4<f32>(0.0); }                              // Clear
        case 17u: { return cs; }                                          // Src
        case 18u: { return cb; }                                          // Dst
        case 19u: { return porter_duff(1.0 - ab, 1.0, cb, cs); }           // DestOver
        case 20u: { return porter_duff(ab, 0.0, cb, cs); }                 // SrcIn
        case 21u: { return porter_duff(0.0, as_, cb, cs); }                // DestIn
        case 22u: { return porter_duff(1.0 - ab, 0.0, cb, cs); }           // SrcOut
        case 23u: { return porter_duff(0.0, 1.0 - as_, cb, cs); }          // DestOut
        case 24u: { return porter_duff(ab, 1.0 - as_, cb, cs); }           // SrcAtop
        case 25u: { return porter_duff(1.0 - ab, as_, cb, cs); }           // DestAtop
        case 26u: { return porter_duff(1.0 - ab, 1.0 - as_, cb, cs); }     // Xor
        case 27u: { return vec4<f32>(cs.rgb + cb.rgb, min(as_ + ab, 1.0)); } // PlusLighter
        default: {}
    }
    if as_ == 0.0 {
        return cb;
    }
    var ub = vec3<f32>(0.0);
    if ab > 0.0 {
        ub = cb.rgb / ab;
    }
    let us = cs.rgb / as_;
    // #2908: the non-separable modes share one affine domain covering
    // both straight colours — lo = min(0, all six channels), hi = max(1,
    // all six), so the domain always contains [0,1]. When the domain is
    // extended, both colours normalize through scale = max(-lo, hi) as
    // (c/scale - lo/scale) / (hi/scale - lo/scale) — the same affine map
    // as (c - lo)/(hi - lo) with every intermediate bounded in magnitude
    // by 2, so even ±f32::MAX channels stay finite where the direct
    // width hi - lo would overflow. nb/ns are the colours the SDR formulas below see;
    // a pure-SDR pair hits the identity domain and keeps the exact
    // original arithmetic. Since lum and sat are affine-covariant the
    // modes keep their semantics; the blend result maps back through
    // lo·(1-B) + hi·B after the switch (the convex combination
    // lo + (hi-lo)·B written with opposite-sign terms so no intermediate
    // can overflow), and the source-over tail still uses the
    // unnormalized us.
    var nb = ub;
    var ns = us;
    var lo = 0.0;
    var hi = 1.0;
    var normalized = false;
    switch mode {
        case 12u, 13u, 14u, 15u: {
            for (var i = 0; i < 3; i += 1) {
                lo = min(lo, min(ub[i], us[i]));
                hi = max(hi, max(ub[i], us[i]));
            }
            if lo < 0.0 || hi > 1.0 {
                let scale = max(-lo, hi);
                let low_scaled = lo / scale;
                let width_scaled = hi / scale - low_scaled;
                nb = (ub / scale - vec3<f32>(low_scaled)) / width_scaled;
                ns = (us / scale - vec3<f32>(low_scaled)) / width_scaled;
                normalized = true;
            }
        }
        default: {}
    }
    var b: vec3<f32>;
    switch mode {
        case 12u: { b = set_lum(set_sat(ns, sat(nb)), lum(nb)); }          // Hue
        case 13u: { b = set_lum(set_sat(nb, sat(ns)), lum(nb)); }          // Saturation
        case 14u: { b = set_lum(ns, lum(nb)); }                            // Color
        case 15u: { b = set_lum(nb, lum(ns)); }                            // Luminosity
        default: {
            b = vec3<f32>(
                blend_channel(mode, ub.x, us.x),
                blend_channel(mode, ub.y, us.y),
                blend_channel(mode, ub.z, us.z),
            );
        }
    }
    if normalized {
        b = fma(vec3<f32>(hi), b, lo * (vec3<f32>(1.0) - b));
    }
    // Cr = (1-αb)·Cs + αb·B(Cb,Cs); premultiplied: αs·Cr + (1-αs)·Cb.
    var out: vec4<f32>;
    for (var i = 0; i < 3; i += 1) {
        let cr = (1.0 - ab) * us[i] + ab * b[i];
        out[i] = as_ * cr + (1.0 - as_) * cb[i];
    }
    out.a = as_ + ab * (1.0 - as_);
    return out;
}
