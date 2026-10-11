// Presents retained premultiplied linear Display P3 on a host attachment.
// sRGB output premultiplies after the transfer function, including when
// hardware applies that transfer. Linear P3 output preserves extended values.
//
// Out-of-gamut linear destination components are gamut-mapped — preserving
// hue instead of clipping each channel (#96). The map is Ottosson's analytic
// OKLab clip, chosen over the CSS Color 4 binary search for its fixed
// per-pixel cost (the search measured ~2.7x slower on lavapipe while the
// analytic clip stayed under one ΔE_OK JND of it on the corpus sweep).
// #98 reuses the same family for Display P3 destinations with cusp
// coefficients fitted against the f64 P3 boundary (oracle/src/gamut.rs).

struct Present {
    // 0: sRGB SDR, hardware transfer; 1: sRGB SDR, shader transfer;
    // 2: extended linear Display P3;
    // 3: Display P3 SDR, shader transfer; 4: Display P3 SDR, hardware;
    // 5: extended linear sRGB (scRGB); 6: encoded extended sRGB;
    // 7: encoded extended Display P3; 8: BT.2100 PQ; 9: BT.2100 HLG.
    encode: u32,
    // 0: opaque (alpha forced to 1), 1: premultiplied in the output space,
    // 2: straight alpha.
    alpha: u32,
    // The effective tone-map headroom (#97/#98): the display's headroom
    // already clamped by the CPU side to what the destination carries —
    // 1 for SDR spaces, 10000/203 for PQ, 1000/203 for HLG.
    headroom: f32,
    _pad: u32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> present: Present;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    // One triangle covering the clip-space square.
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    var out: Vertex;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

fn srgb_encode(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 { return c / 12.92; }
    return pow((c + 0.055) / 1.055, 2.4);
}

// The sRGB OETF continued beyond [0, 1] with odd symmetry through the
// origin — the wire format of the ExtendedSrgb and ExtendedDisplayP3
// colour spaces (wgpu's HDR surface example, IEC 61966-2-2 nonlinear).
// Negative (out-of-gamut) and above-one (HDR) components are encoded,
// never clamped.
fn srgb_encode_extended(c: f32) -> f32 {
    let a = abs(c);
    let e = select(1.055 * pow(a, 1.0 / 2.4) - 0.055, a * 12.92, a <= 0.0031308);
    return sign(c) * e;
}

// SMPTE ST 2084 (PQ) OETF; input is luminance normalized to 10000 nits.
// Below-black channels — content outside BT.2020 — clamp to zero: the
// signal cannot carry them.
fn pq_encode(c: f32) -> f32 {
    let y = pow(max(c, 0.0), 0.1593017578125);
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), 78.84375);
}

// BT.2100 HLG OETF; input is scene-referred signal normalized to the
// 1000-nit nominal peak.
fn hlg_encode_channel(c: f32) -> f32 {
    let y = max(c, 0.0);
    let lo = sqrt(3.0 * y);
    let hi = 0.17883277 * log(12.0 * y - 0.28466892) + 0.55991073;
    return select(hi, lo, y <= 1.0 / 12.0);
}

// The BT.2100 reference OOTF is defined on display light; the working
// space is display-referred, so the inverse converts back to the scene
// signal the OETF expects. Nominal peak 1000 nits, system gamma 1.2 —
// the contract shared with oracle/src/present.rs (#98).
fn hlg_inverse_ootf(rgb: vec3<f32>) -> vec3<f32> {
    let y = dot(rgb, vec3<f32>(0.2627, 0.6780, 0.0593));
    if y <= 0.0 {
        return vec3<f32>(0.0);
    }
    return rgb * pow(y, 1.0 / 1.2 - 1.0);
}

fn hlg_encode(rgb: vec3<f32>) -> vec3<f32> {
    let scene = hlg_inverse_ootf(rgb);
    return vec3<f32>(
        hlg_encode_channel(scene.r),
        hlg_encode_channel(scene.g),
        hlg_encode_channel(scene.b),
    );
}

// The SDR-white-to-nits calibration the absolute and relative HDR
// encodings share: working 1.0 presents as 203 nits (BT.2408 reference
// white) — the same reference cherenkov's PQ/HLG decode side uses.
const REFERENCE_WHITE_NITS: f32 = 203.0;

fn present_color(rgb: vec3<f32>, alpha: f32) -> vec4<f32> {
    if present.alpha == 1u {
        return vec4<f32>(rgb * alpha, alpha);
    }
    return vec4<f32>(rgb, alpha);
}

// ---- Gamut mapping (linear destination primaries in [0,1] bounds) ------
//
// `dest` selects the destination gamut: 0 = linear sRGB, 1 = linear
// Display P3. The machinery is identical; the LMS conversion matrices,
// channel weights and cusp fits are destination-specific.

// Signed cube root — the OKLab LMS nonlinearity for possibly-negative
// components an out-of-gamut colour produces.
fn gamut_cbrt(x: f32) -> f32 {
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}

fn srgb_to_oklab(rgb: vec3<f32>) -> vec3<f32> {
    let l_ = gamut_cbrt(0.4122214708 * rgb.r + 0.5363325363 * rgb.g + 0.0514459929 * rgb.b);
    let m_ = gamut_cbrt(0.2119034982 * rgb.r + 0.6806995451 * rgb.g + 0.1073969566 * rgb.b);
    let s_ = gamut_cbrt(0.0883024619 * rgb.r + 0.2817188376 * rgb.g + 0.6299787005 * rgb.b);
    return vec3<f32>(
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    );
}

fn p3_to_oklab(rgb: vec3<f32>) -> vec3<f32> {
    // XYZ_TO_LMS · P3_TO_XYZ — the sRGB path's structure, P3 primaries.
    let l_ = gamut_cbrt(0.4813272912 * rgb.r + 0.4620679116 * rgb.g + 0.0564956028 * rgb.b);
    let m_ = gamut_cbrt(0.2288381014 * rgb.r + 0.6532343997 * rgb.g + 0.1179544132 * rgb.b);
    let s_ = gamut_cbrt(0.0839860178 * rgb.r + 0.2242727893 * rgb.g + 0.6922208389 * rgb.b);
    return vec3<f32>(
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    );
}

fn oklab_to_srgb(lab: vec3<f32>) -> vec3<f32> {
    let l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    let m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    let s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    return vec3<f32>(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    );
}

fn oklab_to_p3(lab: vec3<f32>) -> vec3<f32> {
    let l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    let m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    let s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    // LMS_TO_P3 = inverse(P3_TO_LMS) — rows are the channel weights.
    return vec3<f32>(
        3.1281105298 * l - 2.2570750194 * m + 0.1293047887 * s,
        -1.0911281617 * l + 2.4132667631 * m - 0.3221681711 * s,
        -0.0260136496 * l - 0.5080276491 * m + 1.5333166823 * s,
    );
}

fn gamut_to_oklab(dest: u32, rgb: vec3<f32>) -> vec3<f32> {
    if dest == 1u {
        return p3_to_oklab(rgb);
    }
    return srgb_to_oklab(rgb);
}

fn gamut_from_oklab(dest: u32, lab: vec3<f32>) -> vec3<f32> {
    if dest == 1u {
        return oklab_to_p3(lab);
    }
    return oklab_to_srgb(lab);
}

// The destination's `channel` row of LMS -> RGB (the `w` vector the
// boundary surfaces are evaluated against).
fn gamut_channel_weights(dest: u32, channel: u32) -> vec3<f32> {
    if dest == 1u {
        if channel == 0u { return vec3<f32>(3.1281105298, -2.2570750194, 0.1293047887); }
        if channel == 1u { return vec3<f32>(-1.0911281617, 2.4132667631, -0.3221681711); }
        return vec3<f32>(-0.0260136496, -0.5080276491, 1.5333166823);
    }
    if channel == 0u { return vec3<f32>(4.0767416621, -3.3077115913, 0.2309699292); }
    if channel == 1u { return vec3<f32>(-1.2684380046, 2.6097574011, -0.3413193965); }
    return vec3<f32>(-0.0041960863, -0.7034186147, 1.7076147010);
}

// The five polynomial coefficients estimating `channel`'s saturation
// root — sRGB's are ok_color.h's; P3's are fitted against the f64
// boundary over the sector where the channel binds (#98).
fn gamut_channel_coeffs(dest: u32, channel: u32) -> array<f32, 5> {
    if dest == 1u {
        if channel == 0u {
            return array<f32, 5>(1.4912985, 2.1212300, 0.7562167, 0.8755941, 0.6931256);
        }
        if channel == 1u {
            return array<f32, 5>(0.7727826, -0.4533926, 0.1141164, 0.1369652, -0.1703990);
        }
        return array<f32, 5>(1.5199945, -0.0309198, -1.2823272, -0.5567115, 0.0250234);
    }
    if channel == 0u {
        return array<f32, 5>(1.19086277, 1.76576728, 0.59662641, 0.75515197, 0.56771245);
    }
    if channel == 1u {
        return array<f32, 5>(0.73956515, -0.45954404, 0.08285427, 0.12541070, 0.14503204);
    }
    return array<f32, 5>(1.35733652, -0.00915799, -1.15130210, -0.50559606, 0.00692167);
}

fn in_gamut(rgb: vec3<f32>) -> bool {
    return all(rgb >= vec3<f32>(0.0)) && all(rgb <= vec3<f32>(1.0));
}

// ---- Tone mapping to the display headroom (#97) ------------------------

// The shoulder: identity on [0, 1], then highlights compress smoothly
// towards the headroom h — the extended-Reinhard/EDR knee at 1,
// asymptote h, so highlight ordering survives for every finite input
// (#97; the BT.2390-adapted EETF plateaued at 3h-2 and lost it).
// oracle/src/tone.rs is the f64 reference.
fn tone_shoulder(x: f32, h: f32) -> f32 {
    if !(x > 1.0) {
        return x;
    }
    let d = max(h - 1.0, 0.0);
    let t = x - 1.0;
    return 1.0 + d * (1.0 - d / (t + d));
}

// One pixel's tone map: scale every channel by the shoulder's value at
// the maximum channel — a per-pixel scalar, so hue and saturation are
// preserved. max <= 1 returns the input unchanged.
fn tone_map(rgb: vec3<f32>, headroom: f32) -> vec3<f32> {
    let m = max(rgb.r, max(rgb.g, rgb.b));
    if !(m > 1.0) {
        return rgb;
    }
    return rgb * (tone_shoulder(m, headroom) / m);
}

// The analytic OKLab clip — Ottosson's cusp-triangle boundary
// with one Halley refinement, projecting towards the lightness axis.

// The cubic and its two derivatives of a destination channel along the
// L=1 saturation ray, evaluated at s.
fn gamut_sat_eval(s: f32, w: vec3<f32>, kl: vec3<f32>) -> vec3<f32> {
    let l_ = 1.0 + s * kl.x;
    let m_ = 1.0 + s * kl.y;
    let s_ = 1.0 + s * kl.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let sc = s_ * s_ * s_;
    let l_ds = 3.0 * kl.x * l_ * l_;
    let m_ds = 3.0 * kl.y * m_ * m_;
    let s_ds = 3.0 * kl.z * s_ * s_;
    let l_ds2 = 6.0 * kl.x * kl.x * l_;
    let m_ds2 = 6.0 * kl.y * kl.y * m_;
    let s_ds2 = 6.0 * kl.z * kl.z * s_;
    return vec3<f32>(
        w.x * l + w.y * m + w.z * sc,
        w.x * l_ds + w.y * m_ds + w.z * s_ds,
        w.x * l_ds2 + w.y * m_ds2 + w.z * s_ds2,
    );
}

// One channel of ok_color.h's compute_max_saturation: the polynomial
// estimate plus one Halley step, returning (root, |residual at root|).
fn gamut_sat_candidate(k: array<f32, 5>, w: vec3<f32>, kl: vec3<f32>, a: f32, b: f32) -> vec2<f32> {
    let est = k[0] + k[1] * a + k[2] * b + k[3] * a * a + k[4] * a * b;
    let e = gamut_sat_eval(est, w, kl);
    let den = e.y * e.y - 0.5 * e.x * e.z;
    if den == 0.0 {
        return vec2<f32>(-1.0, 1.0);
    }
    let root = est - e.x * e.y / den;
    return vec2<f32>(root, abs(gamut_sat_eval(root, w, kl).x));
}

// Max saturation S = C/L for the normalized hue direction (a, b).
// ok_color.h picks one channel's surface by a fitted hue partition; that
// partition is a ~1e-6-sensitive coin flip at the sRGB vertex hues across
// f32/f64, so all three channel surfaces are solved instead and the
// smallest converged root wins — one Halley step each, deterministic.
fn gamut_max_saturation(dest: u32, a: f32, b: f32) -> f32 {
    let kl = vec3<f32>(
        0.3963377774 * a + 0.2158037573 * b,
        -0.1055613458 * a - 0.0638541728 * b,
        -0.0894841775 * a - 1.2914855480 * b,
    );
    var s = 3.4e38;
    var est_s = 3.4e38;
    for (var i = 0u; i < 3u; i++) {
        let k = gamut_channel_coeffs(dest, i);
        let w = gamut_channel_weights(dest, i);
        let est = k[0] + k[1] * a + k[2] * b + k[3] * a * a + k[4] * a * b;
        if est > 0.0 {
            est_s = min(est_s, est);
        }
        let cand = gamut_sat_candidate(k, w, kl, a, b);
        if cand.x > 0.0 && cand.y < 0.05 {
            s = min(s, cand.x);
        }
    }
    if s < 1.0e38 {
        return s;
    }
    // No step converged: fall back to the smallest positive estimate.
    return max(est_s, 0.0);
}

// The destination cusp (L, C) of the hue slice — ok_color.h's find_cusp.
fn gamut_cusp(dest: u32, a: f32, b: f32) -> vec2<f32> {
    let s = gamut_max_saturation(dest, a, b);
    let rgb = gamut_from_oklab(dest, vec3<f32>(1.0, s * a, s * b));
    let l_cusp = gamut_cbrt(1.0 / max(rgb.r, max(rgb.g, rgb.b)));
    return vec2<f32>(l_cusp, l_cusp * s);
}

// Intersects the segment (l0,0) -> (l1,c1) with the gamut boundary in the
// hue slice — ok_color.h's find_gamut_intersection.
fn gamut_intersection(dest: u32, a: f32, b: f32, l1: f32, c1: f32, l0: f32, cusp: vec2<f32>) -> f32 {
    var t: f32;
    if (l1 - l0) * cusp.y - (cusp.x - l0) * c1 <= 0.0 {
        t = cusp.y * l0 / (c1 * cusp.x + cusp.y * (l0 - l1));
    } else {
        t = cusp.y * (l0 - 1.0) / (c1 * (cusp.x - 1.0) + cusp.y * (l0 - l1));
        let dl = l1 - l0;
        let dc = c1;
        let k_l = 0.3963377774 * a + 0.2158037573 * b;
        let k_m = -0.1055613458 * a - 0.0638541728 * b;
        let k_s = -0.0894841775 * a - 1.2914855480 * b;
        let l_dt = dl + dc * k_l;
        let m_dt = dl + dc * k_m;
        let s_dt = dl + dc * k_s;
        let l_at = l0 * (1.0 - t) + t * l1;
        let c_at = t * c1;
        let l_ = l_at + c_at * k_l;
        let m_ = l_at + c_at * k_m;
        let s_ = l_at + c_at * k_s;
        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;
        let ldt = 3.0 * l_dt * l_ * l_;
        let mdt = 3.0 * m_dt * m_ * m_;
        let sdt = 3.0 * s_dt * s_ * s_;
        let ldt2 = 6.0 * l_dt * l_dt * l_;
        let mdt2 = 6.0 * m_dt * m_dt * m_;
        let sdt2 = 6.0 * s_dt * s_dt * s_;
        let r = wdot(gamut_channel_weights(dest, 0u), l, m, s) - 1.0;
        let r1 = wdot(gamut_channel_weights(dest, 0u), ldt, mdt, sdt);
        let r2 = wdot(gamut_channel_weights(dest, 0u), ldt2, mdt2, sdt2);
        let g = wdot(gamut_channel_weights(dest, 1u), l, m, s) - 1.0;
        let g1 = wdot(gamut_channel_weights(dest, 1u), ldt, mdt, sdt);
        let g2 = wdot(gamut_channel_weights(dest, 1u), ldt2, mdt2, sdt2);
        let bch = wdot(gamut_channel_weights(dest, 2u), l, m, s) - 1.0;
        let b1 = wdot(gamut_channel_weights(dest, 2u), ldt, mdt, sdt);
        let b2 = wdot(gamut_channel_weights(dest, 2u), ldt2, mdt2, sdt2);
        let u_r = r1 / (r1 * r1 - 0.5 * r * r2);
        let u_g = g1 / (g1 * g1 - 0.5 * g * g2);
        let u_b = b1 / (b1 * b1 - 0.5 * bch * b2);
        var t_r = select(3.4e38, -r * u_r, u_r >= 0.0);
        var t_g = select(3.4e38, -g * u_g, u_g >= 0.0);
        var t_b = select(3.4e38, -bch * u_b, u_b >= 0.0);
        t += min(t_r, min(t_g, t_b));
    }
    return t;
}

fn wdot(w: vec3<f32>, l: f32, m: f32, s: f32) -> f32 {
    return w.x * l + w.y * m + w.z * s;
}

fn gamut_map(dest: u32, rgb: vec3<f32>) -> vec3<f32> {
    // In-gamut colours keep the pre-#96 bits: map and clamp agree exactly.
    if in_gamut(rgb) {
        return rgb;
    }
    let lab = gamut_to_oklab(dest, rgb);
    // Local-MINDE (ΔE_OK JND = 0.02): when the plain channel-clip is
    // already within a JND of the colour, keep the clip's bytes — the
    // pre-#96 output. A colour a few ULPs out of gamut (an in-sRGB value
    // round-tripped through the P3 matrices) then never enters the
    // projection where the cusp fit is weakest.
    let clipped = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if distance(gamut_to_oklab(dest, clipped), lab) < 0.02 {
        return clipped;
    }
    let l = lab.x;
    let chroma = length(lab.yz);
    if chroma <= 0.00001 {
        return clamp(gamut_from_oklab(dest, vec3<f32>(clamp(l, 0.0, 1.0), 0.0, 0.0)),
                     vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let c = max(0.00001, chroma);
    let a_ = lab.y / c;
    let b_ = lab.z / c;
    let cusp = gamut_cusp(dest, a_, b_);
    // gamut_clip_adaptive_L0_L_cusp (alpha = 0.05): the anchor blends from
    // the colour's lightness towards the cusp's as the colour recedes.
    let ld = l - cusp.x;
    let k = 2.0 * select(cusp.x, 1.0 - cusp.x, ld > 0.0);
    let e1 = 0.5 * k + abs(ld) + 0.05 * c / k;
    let l0 = cusp.x + 0.5 * sign(ld) * (e1 - sqrt(e1 * e1 - 2.0 * k * abs(ld)));
    let t = gamut_intersection(dest, a_, b_, l, c, l0, cusp);
    let l_out = l0 * (1.0 - t) + t * l;
    let c_out = t * c;
    return clamp(gamut_from_oklab(dest, vec3<f32>(l_out, c_out * a_, c_out * b_)),
                 vec3<f32>(0.0), vec3<f32>(1.0));
}

// Linear Display P3 → linear sRGB (Bradford-adapted, D65).
fn p3_to_linear_srgb(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        1.2249402 * rgb.r - 0.2249402 * rgb.g,
        -0.04205695 * rgb.r + 1.0420569 * rgb.g,
        -0.01963755 * rgb.r - 0.07863605 * rgb.g + 1.0982736 * rgb.b,
    );
}

// Linear Display P3 → linear BT.2020 (D65) — inverse(REC2020_TO_XYZ) ·
// P3_TO_XYZ, the same constants as oracle/src/color.rs.
fn p3_to_bt2020(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.7538330343 * rgb.r + 0.1985973692 * rgb.g + 0.0475695965 * rgb.b,
        0.0457438491 * rgb.r + 0.9417772194 * rgb.g + 0.0124789312 * rgb.b,
        -0.0012103404 * rgb.r + 0.0176017173 * rgb.g + 0.9836086230 * rgb.b,
    );
}

fn srgb_encode3(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(srgb_encode(rgb.r), srgb_encode(rgb.g), srgb_encode(rgb.b));
}

fn srgb_encode_extended3(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_encode_extended(rgb.r),
        srgb_encode_extended(rgb.g),
        srgb_encode_extended(rgb.b),
    );
}

// Presents an already-encoded colour, undoing the hardware transfer when
// the destination format applies one: the stored bytes then match what a
// non-sRGB-format write would produce.
fn present_encoded(encoded: vec3<f32>, alpha: f32) -> vec4<f32> {
    let result = present_color(encoded, alpha);
    return vec4<f32>(
        srgb_decode(result.r),
        srgb_decode(result.g),
        srgb_decode(result.b),
        result.a,
    );
}

@fragment
fn fs_main(in: Vertex) -> @location(0) vec4<f32> {
    var p3 = textureSample(source, source_sampler, in.uv);
    var alpha = 1.0;
    if present.alpha != 0u {
        alpha = p3.a;
        if p3.a > 0.0 {
            p3 = vec4<f32>(p3.rgb / p3.a, p3.a);
        }
    }
    switch present.encode {
        case 2u: {
            // Extended linear P3 output rolls off to the host's headroom.
            return present_color(tone_map(p3.rgb, present.headroom), alpha);
        }
        case 3u, 4u: {
            // Display-P3 SDR: tone-map to SDR, map to the P3 boundary,
            // sRGB transfer (Display P3's transfer is sRGB's).
            let toned = tone_map(p3.rgb, min(present.headroom, 1.0));
            let mapped = gamut_map(1u, toned);
            let encoded = srgb_encode3(mapped);
            if present.encode == 3u {
                return present_color(encoded, alpha);
            }
            return present_encoded(encoded, alpha);
        }
        case 5u: {
            // Extended linear sRGB (scRGB): destination primaries,
            // linear transfer, extended range — no [0,1] gamut clip:
            // negative and above-one components carry wide gamut.
            let toned = tone_map(p3.rgb, present.headroom);
            return present_color(p3_to_linear_srgb(toned), alpha);
        }
        case 6u: {
            // Encoded extended sRGB: signed extended transfer.
            let toned = tone_map(p3.rgb, present.headroom);
            return present_color(srgb_encode_extended3(p3_to_linear_srgb(toned)), alpha);
        }
        case 7u: {
            // Encoded extended Display P3: signed extended transfer on
            // the working primaries — not a raw linear attachment.
            let toned = tone_map(p3.rgb, present.headroom);
            return present_color(srgb_encode_extended3(toned), alpha);
        }
        case 8u: {
            // BT.2100 PQ: BT.2020 primaries, SDR white at 203 nits of
            // the 10000-nit PQ range, ST 2084 encode. wgpu supplies no
            // mastering metadata — this is not an authored HDR10 stream.
            let toned = tone_map(p3.rgb, present.headroom);
            let nits = p3_to_bt2020(toned) * (REFERENCE_WHITE_NITS / 10000.0);
            return present_color(
                vec3<f32>(pq_encode(nits.r), pq_encode(nits.g), pq_encode(nits.b)),
                alpha,
            );
        }
        case 9u: {
            // BT.2100 HLG: BT.2020 primaries at the 1000-nit nominal
            // peak, inverse OOTF (system gamma 1.2), then the OETF.
            let toned = tone_map(p3.rgb, present.headroom);
            let display = p3_to_bt2020(toned) * (REFERENCE_WHITE_NITS / 1000.0);
            return present_color(hlg_encode(display), alpha);
        }
        default: {
            // sRGB SDR. Highlights roll off in the working space — an
            // sRGB-intensity P3 colour never takes the shoulder — then
            // the matrix, then the gamut map (#97).
            let p3_toned = tone_map(p3.rgb, min(present.headroom, 1.0));
            let rgb = p3_to_linear_srgb(p3_toned);
            let clamped = gamut_map(0u, rgb);
            let encoded = srgb_encode3(clamped);
            if present.encode == 1u {
                return present_color(encoded, alpha);
            }
            // Hardware applies the transfer after this shader. Undo the
            // transfer of the encoded-domain premultiplied result so
            // stored bytes match Unorm.
            return present_encoded(encoded, alpha);
        }
    }
}
